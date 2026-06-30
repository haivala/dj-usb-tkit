use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use backend::commands::BackendCommands;
use backend::models::{
    AddTracksToPlaylistRequest, CreatePlaylistRequest, DedupeMode, ExportToUsbOptions,
    ExportToUsbRequest, FetchUsbPlaylistsRequest, InitializeUsbRequest, RemoveUsbPlaylistRequest,
    RepairUsbDiagnosticsRequest, RunUsbDiagnosticsRequest, RunUsbParityReportRequest,
    ScanLibraryRequest, SearchTracksRequest, UsbParityPlaylistDetail,
};
use backend::pdb_reader::parse_pdb;
use tempfile::{TempDir, tempdir};

const USB_VENDOR_ROOT_DIR: &str = "PIONEER";
const USB_VENDOR_DB_DIR: &str = "rekordbox";
const PDB_HEADER_COMPATIBILITY_FIX_ID: &str = "repair_pdb_header_compatibility_field";

fn vendor_db_dir(usb_root: &Path) -> std::path::PathBuf {
    usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)
}

use backend::service::usb_vendor_compat::DEFAULT_USB_EDB_KEY;

/// Open an eDB, trying plain SQLite first, then SQLCipher.
fn open_edb(path: &Path) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open(path).expect("open eDB");
    let has_schema = conn
        .query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type IN ('table','view')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0);
    if has_schema == 0 {
        conn.execute_batch(&format!("PRAGMA key='{DEFAULT_USB_EDB_KEY}';"))
            .expect("apply SQLCipher key");
        let unlocked = conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master WHERE type IN ('table','view')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);
        assert!(
            unlocked > 0,
            "failed to unlock SQLCipher DB at {}",
            path.display()
        );
    }
    conn
}

fn seed_test_analysis_bundle(data_dir: &Path, stem: &str) -> PathBuf {
    let dir = data_dir.join("analysis").join("waveforms");
    fs::create_dir_all(&dir).expect("create test analysis dir");
    let dat = dir.join(format!("{stem}.DAT"));
    fs::write(&dat, b"test-dat").expect("write test DAT");
    fs::write(dir.join(format!("{stem}.EXT")), b"test-ext").expect("write test EXT");
    fs::write(dir.join(format!("{stem}.2EX")), b"test-2ex").expect("write test 2EX");
    dat
}

fn seed_tracks_as_analyzed(data_dir: &Path, track_ids: &[String]) {
    let db_path = data_dir.join("backend.db");
    let conn = rusqlite::Connection::open(&db_path).expect("open backend db");
    for (idx, track_id) in track_ids.iter().enumerate() {
        let bundle = seed_test_analysis_bundle(data_dir, &format!("waveform-{idx}"));
        conn.execute(
            "UPDATE tracks
             SET bpm = 120.0,
                 duration_ms = 180000,
                 track_number = ?1,
                 waveform_peaks_path = ?2
             WHERE id = ?3",
            rusqlite::params![
                (idx as u32) + 1,
                bundle.to_string_lossy().to_string(),
                track_id
            ],
        )
        .expect("seed analyzed track fields");
    }
}

fn seed_track_artwork_path(data_dir: &Path, track_id: &str, artwork_path: &Path) {
    let db_path = data_dir.join("backend.db");
    let conn = rusqlite::Connection::open(&db_path).expect("open backend db");
    conn.execute(
        "UPDATE tracks SET artwork_path = ?1 WHERE id = ?2",
        rusqlite::params![artwork_path.to_string_lossy().as_ref(), track_id],
    )
    .expect("seed track artwork path");
}

fn fixture_audio_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("audio")
        .join(relative)
}

fn copy_audio_fixture(media_dir: &Path, fixture_relative: &str, target_name: &str) {
    let fixture = fixture_audio_path(fixture_relative);
    fs::copy(&fixture, media_dir.join(target_name)).expect("copy audio fixture");
}

fn read_pdb_header_compatibility_value(pdb_path: &Path) -> u32 {
    let bytes = fs::read(pdb_path).expect("read export pdb");
    u32::from_le_bytes(bytes[0x10..0x14].try_into().expect("header bytes"))
}

fn write_pdb_header_compatibility_value(pdb_path: &Path, value: u32) {
    let mut bytes = fs::read(pdb_path).expect("read export pdb");
    bytes[0x10..0x14].copy_from_slice(&value.to_le_bytes());
    fs::write(pdb_path, bytes).expect("write export pdb");
}

fn create_previous_pdb_snapshot_with_header(
    usb_root: &Path,
    source_pdb: &Path,
    file_name: &str,
    value: u32,
) -> PathBuf {
    let previous_dir = vendor_db_dir(usb_root).join("backups");
    fs::create_dir_all(&previous_dir).expect("create previous PDB dir");
    let previous_pdb = previous_dir.join(file_name);
    fs::copy(source_pdb, &previous_pdb).expect("copy previous PDB snapshot");
    write_pdb_header_compatibility_value(&previous_pdb, value);
    previous_pdb
}

fn assert_no_pdb_structural_repairs(backend: &BackendCommands, usb: &Path) {
    let preview = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: false,
        selected_fix_ids: vec![],
    });
    assert!(preview.ok, "repair preview failed: {preview:?}");
    let structural: Vec<String> = preview
        .data
        .expect("preview data")
        .proposed_fixes
        .iter()
        .filter(|f| f.id.starts_with("repair_pdb_") && f.supported)
        .map(|f| format!("{} ({})", f.id, f.title))
        .collect();
    assert!(
        structural.is_empty(),
        "structural PDB repairs proposed after export:\n{structural:#?}"
    );
}

fn assert_pdb_crossrefs_clean(usb: &Path) {
    let pdb = vendor_db_dir(usb).join("export.pdb");
    let parsed = parse_pdb(&pdb).expect("parse PDB");

    let track_ids: HashSet<u32> = parsed.tracks.iter().map(|t| t.id).collect();
    let playlist_ids: HashSet<u32> = parsed.playlist_tree.iter().map(|p| p.id).collect();

    let mut errors: Vec<String> = vec![];

    for t in &parsed.tracks {
        if t.artist_id != 0 && !parsed.artists.contains_key(&t.artist_id) {
            errors.push(format!(
                "track {} artist_id={} not in artists",
                t.id, t.artist_id
            ));
        }
        if t.album_id != 0 && !parsed.albums.contains_key(&t.album_id) {
            errors.push(format!(
                "track {} album_id={} not in albums",
                t.id, t.album_id
            ));
        }
        if t.artwork_id != 0 && !parsed.artworks.contains_key(&t.artwork_id) {
            errors.push(format!(
                "track {} artwork_id={} not in artworks",
                t.id, t.artwork_id
            ));
        }
        if t.key_id != 0 && !parsed.keys.contains_key(&t.key_id) {
            errors.push(format!("track {} key_id={} not in keys", t.id, t.key_id));
        }
    }
    for e in &parsed.playlist_entries {
        if !track_ids.contains(&e.track_id) {
            errors.push(format!(
                "playlist_entry track_id={} not in tracks",
                e.track_id
            ));
        }
        if !playlist_ids.contains(&e.playlist_id) {
            errors.push(format!(
                "playlist_entry playlist_id={} not in playlist_tree",
                e.playlist_id
            ));
        }
    }
    for p in &parsed.playlist_tree {
        if p.parent_id != 0 && !playlist_ids.contains(&p.parent_id) {
            errors.push(format!(
                "playlist_tree id={} parent_id={} not in playlist_tree",
                p.id, p.parent_id
            ));
        }
    }

    let mut seen: HashMap<u32, usize> = HashMap::new();
    for t in &parsed.tracks {
        *seen.entry(t.id).or_insert(0) += 1;
    }
    for (id, count) in &seen {
        if *count > 1 {
            errors.push(format!("duplicate track_id={id} appears {count} times"));
        }
    }

    assert!(
        errors.is_empty(),
        "PDB cross-reference errors:\n{}",
        errors.join("\n")
    );
}

fn thin_first_pdb_track_row_fields(pdb_path: &Path, clear_key_id: bool, clear_duration: bool) {
    let mut bytes = fs::read(pdb_path).expect("read export pdb");
    let len_page = u32::from_le_bytes(bytes[4..8].try_into().expect("len_page bytes")) as usize;
    let mut mutated = false;
    for page_idx in 1..(bytes.len() / len_page) {
        let start = page_idx * len_page;
        if start + 128 > bytes.len() {
            break;
        }
        let page_index =
            u32::from_le_bytes(bytes[start + 4..start + 8].try_into().expect("page index"));
        let table_type =
            u32::from_le_bytes(bytes[start + 8..start + 12].try_into().expect("table type"));
        let used_s =
            u16::from_le_bytes(bytes[start + 30..start + 32].try_into().expect("used_s")) as usize;
        if page_index == 0 || table_type != 0 || used_s == 0 {
            continue;
        }
        let row_start = start + 40;
        if clear_key_id {
            bytes[row_start + 32..row_start + 36].copy_from_slice(&0u32.to_le_bytes());
        }
        if clear_duration {
            bytes[row_start + 84..row_start + 86].copy_from_slice(&0u16.to_le_bytes());
        }
        mutated = true;
        break;
    }
    assert!(mutated, "expected to mutate one PDB track row");
    fs::write(pdb_path, bytes).expect("write thinned export pdb");
}

fn mutate_first_pdb_analysis_path(pdb_path: &Path) {
    let mut bytes = fs::read(pdb_path).expect("read export pdb");
    let len_page = u32::from_le_bytes(bytes[4..8].try_into().expect("len_page bytes")) as usize;
    let mut mutated = false;
    for page_idx in 1..(bytes.len() / len_page) {
        let start = page_idx * len_page;
        if start + 180 > bytes.len() {
            break;
        }
        let page_index =
            u32::from_le_bytes(bytes[start + 4..start + 8].try_into().expect("page index"));
        let table_type =
            u32::from_le_bytes(bytes[start + 8..start + 12].try_into().expect("table type"));
        let used_s =
            u16::from_le_bytes(bytes[start + 30..start + 32].try_into().expect("used_s")) as usize;
        if page_index == 0 || table_type != 0 || used_s == 0 {
            continue;
        }

        let row_start = start + 40;
        let anlz_start = u16::from_le_bytes(
            bytes[row_start + 94 + 14 * 2..row_start + 94 + 14 * 2 + 2]
                .try_into()
                .expect("anlz start"),
        ) as usize;
        let anlz_end = u16::from_le_bytes(
            bytes[row_start + 94 + 15 * 2..row_start + 94 + 15 * 2 + 2]
                .try_into()
                .expect("anlz end"),
        ) as usize;
        let anlz_body_start = row_start + anlz_start + 1;
        let anlz_body_end = row_start + anlz_end;

        let current_anlz =
            String::from_utf8(bytes[anlz_body_start..anlz_body_end].to_vec()).expect("anlz utf8");
        let replacement_anlz = current_anlz.replace("ANLZ0000.DAT", "ANLZ9999.DAT");
        assert_ne!(
            replacement_anlz, current_anlz,
            "expected analysis mutation target"
        );
        assert_eq!(replacement_anlz.len(), current_anlz.len());

        bytes[anlz_body_start..anlz_body_end].copy_from_slice(replacement_anlz.as_bytes());
        mutated = true;
        break;
    }
    assert!(mutated, "expected to mutate one PDB track analysis path");
    fs::write(pdb_path, bytes).expect("write analysis-path-mutated export pdb");
}

fn mutate_first_pdb_artist_id(pdb_path: &Path, artist_id: u32) {
    let mut bytes = fs::read(pdb_path).expect("read export pdb");
    let len_page = u32::from_le_bytes(bytes[4..8].try_into().expect("len_page bytes")) as usize;
    let mut mutated = false;
    for page_idx in 1..(bytes.len() / len_page) {
        let start = page_idx * len_page;
        if start + 128 > bytes.len() {
            break;
        }
        let page_index =
            u32::from_le_bytes(bytes[start + 4..start + 8].try_into().expect("page index"));
        let table_type =
            u32::from_le_bytes(bytes[start + 8..start + 12].try_into().expect("table type"));
        let used_s =
            u16::from_le_bytes(bytes[start + 30..start + 32].try_into().expect("used_s")) as usize;
        if page_index == 0 || table_type != 0 || used_s == 0 {
            continue;
        }
        let row_start = start + 40;
        bytes[row_start + 68..row_start + 72].copy_from_slice(&artist_id.to_le_bytes());
        mutated = true;
        break;
    }
    assert!(mutated, "expected to mutate one PDB track artist id");
    fs::write(pdb_path, bytes).expect("write artist-id-mutated export pdb");
}

fn mutate_first_pdb_playlist_tree_row_to_folder_with_id(pdb_path: &Path, folder_id: u32) {
    let mut bytes = fs::read(pdb_path).expect("read export pdb");
    let len_page = u32::from_le_bytes(bytes[4..8].try_into().expect("len_page bytes")) as usize;
    let mut mutated = false;
    for page_idx in 1..(bytes.len() / len_page) {
        let start = page_idx * len_page;
        if start + 80 > bytes.len() {
            break;
        }
        let page_index =
            u32::from_le_bytes(bytes[start + 4..start + 8].try_into().expect("page index"));
        let table_type =
            u32::from_le_bytes(bytes[start + 8..start + 12].try_into().expect("table type"));
        let used_s =
            u16::from_le_bytes(bytes[start + 30..start + 32].try_into().expect("used_s")) as usize;
        if page_index == 0 || table_type != 7 || used_s == 0 {
            continue;
        }

        let row_start = start + 40;
        bytes[row_start + 12..row_start + 16].copy_from_slice(&folder_id.to_le_bytes());
        bytes[row_start + 16..row_start + 20].copy_from_slice(&1u32.to_le_bytes());
        mutated = true;
        break;
    }
    assert!(mutated, "expected to mutate one PDB playlist-tree row");
    fs::write(pdb_path, bytes).expect("write folder-mutated export pdb");
}

fn setup_clean_strict_parity_fixture_with_local_key(
    local_key: Option<&str>,
) -> (TempDir, BackendCommands, PathBuf, String) {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");
    copy_audio_fixture(
        &media,
        "noart/track_no_art.mp3",
        "01 Fixture Artist - Clean Parity.mp3",
    );

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track_ids = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .map(|t| t.id)
        .collect::<Vec<_>>();
    assert_eq!(track_ids.len(), 1);
    seed_tracks_as_analyzed(&data_dir, &track_ids);
    if let Some(key_name) = local_key {
        let db_path = data_dir.join("backend.db");
        let conn = rusqlite::Connection::open(&db_path).expect("open backend db");
        conn.execute(
            "UPDATE tracks SET tonality = ?1 WHERE id = ?2",
            rusqlite::params![key_name, track_ids[0]],
        )
        .expect("seed local key");
    }

    let playlist_name = "Clean Parity".to_string();
    let created = backend.create_playlist(CreatePlaylistRequest {
        name: playlist_name.clone(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;
    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids,
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add failed: {added:?}");
    assert_eq!(added.data.expect("add data").added, 1);

    let export = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export.ok, "export failed: {export:?}");

    (root, backend, usb, playlist_name)
}

fn setup_clean_strict_parity_fixture() -> (TempDir, BackendCommands, PathBuf, String) {
    setup_clean_strict_parity_fixture_with_local_key(None)
}

fn setup_clean_strict_parity_fixture_with_artwork() -> (TempDir, BackendCommands, PathBuf, String) {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let fixture_dir = fixture_audio_path("folder");
    for entry in fs::read_dir(&fixture_dir).expect("read fixture dir") {
        let entry = entry.expect("dir entry");
        fs::copy(entry.path(), media.join(entry.file_name())).expect("copy fixture");
    }

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track_ids = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .map(|t| t.id)
        .collect::<Vec<_>>();
    assert_eq!(track_ids.len(), 1);
    seed_tracks_as_analyzed(&data_dir, &track_ids);
    let cover_path = media.join("cover.jpg");
    assert!(
        cover_path.is_file(),
        "expected fixture cover at {}",
        cover_path.display()
    );
    seed_track_artwork_path(&data_dir, &track_ids[0], &cover_path);

    let playlist_name = "Clean Parity Artwork".to_string();
    let created = backend.create_playlist(CreatePlaylistRequest {
        name: playlist_name.clone(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;
    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids,
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add failed: {added:?}");
    assert_eq!(added.data.expect("add data").added, 1);

    let export = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: true,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export.ok, "export failed: {export:?}");

    (root, backend, usb, playlist_name)
}

#[test]
fn repair_usb_diagnostics_with_progress_missing_root_returns_api_error() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let missing_usb = root.path().join("missing-usb");

    let mut progress_calls = 0usize;
    let response = backend.repair_usb_diagnostics_with_progress(
        RepairUsbDiagnosticsRequest {
            usb_root: Some(missing_usb.to_string_lossy().to_string()),
            apply: false,
            selected_fix_ids: Vec::new(),
        },
        |_, _, _| {
            progress_calls += 1;
        },
    );

    assert!(!response.ok, "missing root should fail: {response:?}");
    let error = response.error.expect("missing-root error payload");
    assert!(
        !error.message.trim().is_empty(),
        "expected non-empty error message"
    );
    let _ = progress_calls;
}

fn setup_two_playlist_strict_parity_fixture() -> (TempDir, BackendCommands, PathBuf, String, String)
{
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");
    copy_audio_fixture(
        &media,
        "noart/track_no_art.mp3",
        "01 Fixture Artist - Repair A.mp3",
    );
    copy_audio_fixture(
        &media,
        "noart/track_no_art.mp3",
        "02 Fixture Artist - Repair B.mp3",
    );

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let tracks = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    assert_eq!(tracks.len(), 2);
    let track_ids = tracks
        .iter()
        .map(|track| track.id.clone())
        .collect::<Vec<_>>();
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let playlist_names = ["Repair Target".to_string(), "Repair Control".to_string()];
    for (playlist_name, track_id) in playlist_names.iter().zip(track_ids.into_iter()) {
        let created = backend.create_playlist(CreatePlaylistRequest {
            name: playlist_name.clone(),
        });
        assert!(created.ok, "create failed: {created:?}");
        let playlist_id = created.data.expect("playlist data").playlist_id;
        let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist_id.clone(),
            track_ids: vec![track_id],
            dedupe: DedupeMode::Skip,
        });
        assert!(added.ok, "add failed: {added:?}");
        assert_eq!(added.data.expect("add data").added, 1);

        let export = backend.export_to_usb(ExportToUsbRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
            playlist_id,
            options: Some(ExportToUsbOptions {
                include_artwork: false,
                include_analysis: true,
                prune_stale: false,
                ..Default::default()
            }),
        });
        assert!(export.ok, "export failed: {export:?}");
    }

    (
        root,
        backend,
        usb,
        playlist_names[0].clone(),
        playlist_names[1].clone(),
    )
}

fn parity_detail_for_playlist(
    backend: &BackendCommands,
    usb_root: &Path,
    playlist_name: &str,
) -> UsbParityPlaylistDetail {
    let parity = backend.run_usb_parity_report(RunUsbParityReportRequest {
        usb_root: Some(usb_root.to_string_lossy().to_string()),
    });
    assert!(parity.ok, "parity failed: {parity:?}");
    parity
        .data
        .expect("parity data")
        .playlist_details
        .into_iter()
        .find(|detail| detail.name == playlist_name)
        .unwrap_or_else(|| panic!("playlist not found in parity report: {playlist_name}"))
}

fn edb_playlist_member_count(usb_root: &Path, playlist_name: &str) -> i64 {
    let vendor_db = vendor_db_dir(usb_root).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    conn.query_row(
        "SELECT COUNT(*)
         FROM playlist_content pc
         JOIN playlist p ON p.playlist_id = pc.playlist_id
         WHERE p.name = ?1",
        [playlist_name],
        |row| row.get(0),
    )
    .expect("playlist member count")
}

fn edb_artwork_fk_column(conn: &rusqlite::Connection) -> &'static str {
    let mut stmt = conn
        .prepare("PRAGMA table_info(content)")
        .expect("prepare table_info");
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query table_info")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect table_info");
    if columns.iter().any(|name| name == "imageFilePath_id") {
        "imageFilePath_id"
    } else {
        "image_id"
    }
}

fn first_playlist_edb_artwork(usb_root: &Path, playlist_name: &str) -> (i64, i64, String) {
    let vendor_db = vendor_db_dir(usb_root).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    let artwork_fk = edb_artwork_fk_column(&conn);
    let sql = format!(
        "SELECT c.content_id, c.{artwork_fk}, i.path
         FROM playlist p
         JOIN playlist_content pc ON pc.playlist_id = p.playlist_id
         JOIN content c ON c.content_id = pc.content_id
         JOIN image i ON i.image_id = c.{artwork_fk}
         WHERE p.name = ?1
         ORDER BY pc.sequenceNo ASC, pc.content_id ASC
         LIMIT 1"
    );
    conn.query_row(&sql, [&playlist_name], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })
    .expect("first playlist eDB artwork")
}

fn first_playlist_pdb_artwork(usb_root: &Path, playlist_name: &str) -> (u32, u32, String) {
    let parsed = backend::pdb_reader::parse_pdb(&vendor_db_dir(usb_root).join("export.pdb"))
        .expect("parse pdb for artwork");
    let playlist_id = parsed
        .playlist_tree
        .iter()
        .find(|row| !row.row_is_folder && row.name == playlist_name)
        .map(|row| row.id)
        .expect("playlist row");
    let track_id = parsed
        .playlist_entries
        .iter()
        .filter(|entry| entry.playlist_id == playlist_id)
        .min_by_key(|entry| (entry.entry_index, entry.track_id))
        .map(|entry| entry.track_id)
        .expect("playlist track");
    let track = parsed
        .tracks
        .iter()
        .find(|track| track.id == track_id)
        .expect("pdb track");
    let artwork_path = parsed
        .artworks
        .get(&track.artwork_id)
        .cloned()
        .expect("pdb artwork path");
    (track.id, track.artwork_id, artwork_path)
}

fn assert_same_parity_detail(actual: &UsbParityPlaylistDetail, expected: &UsbParityPlaylistDetail) {
    assert_eq!(actual.name, expected.name);
    assert_eq!(actual.pdb_tracks, expected.pdb_tracks);
    assert_eq!(actual.edb_tracks, expected.edb_tracks);
    assert_eq!(actual.matched_tracks, expected.matched_tracks);
    assert_eq!(actual.only_in_pdb, expected.only_in_pdb);
    assert_eq!(actual.only_in_edb, expected.only_in_edb);
    assert_eq!(actual.order_mismatch, expected.order_mismatch);
    assert_eq!(actual.path_mismatch_tracks, expected.path_mismatch_tracks);
    assert_eq!(
        actual.dictionary_id_issue_tracks, expected.dictionary_id_issue_tracks,
        "dictionary id mismatch\nactual: {actual:?}\nexpected: {expected:?}"
    );
    assert_eq!(actual.playlist_id_match, expected.playlist_id_match);
    assert_eq!(actual.sort_order_match, expected.sort_order_match);
    assert_eq!(actual.parent_match, expected.parent_match);
    assert_eq!(actual.pdb_playlist_id, expected.pdb_playlist_id);
    assert_eq!(actual.edb_playlist_id, expected.edb_playlist_id);
    assert_eq!(actual.pdb_sort_order, expected.pdb_sort_order);
    assert_eq!(actual.edb_sort_order, expected.edb_sort_order);
    assert_eq!(actual.pdb_duplicate_entries, expected.pdb_duplicate_entries);
    assert_eq!(
        actual.edb_missing_core_metadata,
        expected.edb_missing_core_metadata
    );
    assert_eq!(
        actual.pdb_missing_core_metadata,
        expected.pdb_missing_core_metadata
    );
    assert_eq!(
        actual.artwork_mismatch_tracks,
        expected.artwork_mismatch_tracks
    );
    assert_eq!(actual.sample_only_in_pdb, expected.sample_only_in_pdb);
    assert_eq!(actual.sample_only_in_edb, expected.sample_only_in_edb);
    assert_eq!(
        actual.sample_metadata_mismatches,
        expected.sample_metadata_mismatches
    );
    assert_eq!(
        std::mem::discriminant(&actual.status),
        std::mem::discriminant(&expected.status)
    );
}

#[test]
fn diagnostics_parity_and_import_counts_stay_consistent_after_repeated_exports() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");
    copy_audio_fixture(&media, "noart/track_no_art.mp3", "Artist - One.mp3");
    copy_audio_fixture(&media, "embedded/track_embedded.mp3", "Artist - Two.mp3");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track_ids = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .map(|t| t.id)
        .collect::<Vec<_>>();
    assert_eq!(track_ids.len(), 2);
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let playlist_name = "Diag Stable";
    let created = backend.create_playlist(CreatePlaylistRequest {
        name: playlist_name.to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;
    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids,
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add failed: {added:?}");
    assert_eq!(added.data.expect("add data").added, 2);

    let options = Some(ExportToUsbOptions {
        include_artwork: false,
        include_analysis: true,
        prune_stale: false,
        ..Default::default()
    });
    let export_one = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: options.clone(),
    });
    assert!(export_one.ok, "first export failed: {export_one:?}");
    let export_two = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options,
    });
    assert!(export_two.ok, "second export failed: {export_two:?}");

    let imported = backend.fetch_usb_playlists(FetchUsbPlaylistsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(imported.ok, "fetch usb playlists failed: {imported:?}");
    let imported_data = imported.data.expect("imported data");
    let imported_playlist = imported_data
        .items
        .iter()
        .find(|p| p.name == playlist_name)
        .unwrap_or_else(|| {
            panic!(
                "playlist not found in import view: {:?}",
                imported_data.items
            )
        });
    let imported_count = imported_playlist.track_count;
    assert_eq!(imported_count, 2, "expected imported track count 2");

    let diagnostics = backend.run_usb_diagnostics(RunUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(diagnostics.ok, "diagnostics failed: {diagnostics:?}");
    let diagnostics_data = diagnostics.data.expect("diagnostics data");
    let diag_playlist = diagnostics_data
        .playlist_details
        .iter()
        .find(|d| d.name == playlist_name)
        .unwrap_or_else(|| {
            panic!(
                "playlist not found in diagnostics view: {:?}",
                diagnostics_data.playlist_details
            )
        });

    let parity = backend.run_usb_parity_report(RunUsbParityReportRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(parity.ok, "parity failed: {parity:?}");
    let parity_data = parity.data.expect("parity data");
    let parity_playlist = parity_data
        .playlist_details
        .iter()
        .find(|d| d.name == playlist_name)
        .unwrap_or_else(|| {
            panic!(
                "playlist not found in parity view: {:?}",
                parity_data.playlist_details
            )
        });

    assert_eq!(diag_playlist.total_entries, imported_count);
    assert_eq!(diag_playlist.resolved_entries, imported_count);
    assert_eq!(diag_playlist.pdb_entries, imported_count);
    assert_eq!(diag_playlist.edb_entries, imported_count);
    assert_eq!(diag_playlist.matched_entries, imported_count);

    assert_eq!(parity_playlist.pdb_tracks, imported_count);
    assert_eq!(parity_playlist.edb_tracks, imported_count);
    assert_eq!(parity_playlist.matched_tracks, imported_count);
    assert_eq!(parity_playlist.only_in_pdb, 0);
    assert_eq!(parity_playlist.only_in_edb, 0);
    assert!(!parity_playlist.order_mismatch);
    assert_eq!(parity_playlist.pdb_duplicate_entries, 0);
    let _ = parity_playlist.pdb_missing_core_metadata;
    let _ = parity_playlist.edb_missing_core_metadata;
    let _ = parity_playlist.artwork_mismatch_tracks;
}

#[test]
fn clean_export_fixture_reaches_full_strict_parity_pass() {
    let (_root, backend, usb, playlist_name) = setup_clean_strict_parity_fixture();

    let parity = backend.run_usb_parity_report(RunUsbParityReportRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(parity.ok, "parity failed: {parity:?}");
    let parity_data = parity.data.expect("parity data");

    let playlist = parity_data
        .playlist_details
        .iter()
        .find(|d| d.name == playlist_name)
        .expect("playlist detail");
    assert!(
        playlist.playlist_id_match,
        "playlist ids should match: {playlist:?}"
    );
    assert!(
        playlist.sort_order_match,
        "sort order should match: {playlist:?}"
    );

    let identity_check = parity_data
        .checks
        .iter()
        .find(|c| c.label == "Playlist identity parity")
        .expect("playlist identity parity check");
    assert!(
        matches!(identity_check.status, backend::models::DiagStatus::Pass),
        "identity parity should pass once eDB playlist ids are carried through: {:?}",
        identity_check
    );

    let ordering_check = parity_data
        .checks
        .iter()
        .find(|c| c.label == "Playlist ordering parity")
        .expect("playlist ordering parity check");
    assert!(
        matches!(ordering_check.status, backend::models::DiagStatus::Pass),
        "ordering parity should pass once eDB sort order is carried through: {:?}",
        ordering_check
    );

    let expected_pass_checks = [
        "Overall player parity status",
        "Playlist identity parity",
        "Playlist membership parity",
        "Playlist ordering parity",
        "Duplicate PDB entries",
        "PDB metadata completeness",
        "Media and analysis path parity",
        "Artwork presence parity",
        "PDB dictionary id resolution",
    ];
    for label in expected_pass_checks {
        let check = parity_data
            .checks
            .iter()
            .find(|c| c.label == label)
            .unwrap_or_else(|| panic!("missing parity check: {label}"));
        assert!(
            matches!(check.status, backend::models::DiagStatus::Pass),
            "strict clean fixture check should pass for '{label}': {:?}",
            check
        );
    }

    assert!(
        matches!(
            parity_data.overall_status,
            backend::models::DiagStatus::Pass
        ),
        "clean fixture should now achieve full strict parity pass: {:?}",
        parity_data.checks
    );

    assert!(
        matches!(playlist.status, backend::models::DiagStatus::Pass),
        "clean fixture playlist row should pass strict parity: {playlist:?}"
    );
    assert_eq!(playlist.only_in_pdb, 0);
    assert_eq!(playlist.only_in_edb, 0);
    assert!(
        !playlist.order_mismatch,
        "playlist order should match: {playlist:?}"
    );
    assert_eq!(playlist.pdb_duplicate_entries, 0);
    assert_eq!(playlist.pdb_missing_core_metadata, 0);
    assert_eq!(playlist.edb_missing_core_metadata, 0);
    assert_eq!(playlist.path_mismatch_tracks, 0);
    assert_eq!(playlist.dictionary_id_issue_tracks, 0);
    assert_eq!(playlist.artwork_mismatch_tracks, 0);
    assert!(
        playlist.sample_only_in_pdb.is_empty(),
        "unexpected only-in-PDB samples: {playlist:?}"
    );
    assert!(
        playlist.sample_only_in_edb.is_empty(),
        "unexpected only-in-eDB samples: {playlist:?}"
    );
    assert!(
        playlist.sample_metadata_mismatches.is_empty(),
        "unexpected metadata mismatch samples: {playlist:?}"
    );

    let required_section = parity_data
        .checks
        .iter()
        .find(|c| c.label == "Parity-report section (required)")
        .expect("required parity-report summary");
    assert!(
        required_section.detail.contains("See parity summary rows"),
        "required parity summary should point to structured summary rows: {:?}",
        required_section
    );
    for label in [
        "PDB metadata gaps",
        "Path mismatches",
        "Unresolved PDB dictionary ids",
    ] {
        let row = parity_data
            .summary_rows
            .iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| panic!("missing summary row: {label}"));
        assert_eq!(
            row.count, 0,
            "strict fixture summary row should be clean: {row:?}"
        );
    }
}

#[test]
fn diagnostics_and_repair_pdb_header_compatibility_without_previous_snapshot() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    let _ = fs::remove_dir_all(vendor_db_dir(&usb).join("backups"));
    write_pdb_header_compatibility_value(&pdb_path, 7);

    let diagnostics = backend.run_usb_diagnostics(RunUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(diagnostics.ok, "diagnostics failed: {diagnostics:?}");
    let diagnostics_data = diagnostics.data.expect("diagnostics data");
    let header_check = diagnostics_data
        .pdb_integrity
        .checks
        .iter()
        .find(|c| c.label == "PDB header compatibility")
        .expect("header compatibility check");
    assert!(
        matches!(header_check.status, backend::models::DiagStatus::Warn),
        "unexpected header check: {header_check:?}"
    );
    assert!(
        header_check.detail.contains("known-compatible"),
        "unexpected header detail: {header_check:?}"
    );

    let preview = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: false,
        selected_fix_ids: Vec::new(),
    });
    assert!(preview.ok, "repair preview failed: {preview:?}");
    let preview_data = preview.data.expect("preview data");
    let proposal = preview_data
        .proposed_fixes
        .iter()
        .find(|fix| fix.id == PDB_HEADER_COMPATIBILITY_FIX_ID)
        .expect("header repair proposal");
    assert!(
        proposal.supported,
        "proposal should be supported: {proposal:?}"
    );
    assert!(
        proposal
            .description
            .contains("built-in compatibility value"),
        "proposal should not require a previous snapshot: {proposal:?}"
    );

    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec![PDB_HEADER_COMPATIBILITY_FIX_ID.to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let repair_data = repair.data.expect("repair data");
    assert!(
        repair_data
            .applied_fixes
            .iter()
            .any(|line| line.contains("Repair PDB Header Compatibility Field")),
        "repair should apply: {repair_data:?}"
    );
    assert_eq!(read_pdb_header_compatibility_value(&pdb_path), 5);
}

#[test]
fn diagnostics_and_repair_pdb_header_compatibility_use_previous_snapshot_when_present() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    create_previous_pdb_snapshot_with_header(&usb, &pdb_path, "export_2099-01-01_00-00-00.pdb", 1);
    write_pdb_header_compatibility_value(&pdb_path, 5);

    let diagnostics = backend.run_usb_diagnostics(RunUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(diagnostics.ok, "diagnostics failed: {diagnostics:?}");
    let diagnostics_data = diagnostics.data.expect("diagnostics data");
    let header_check = diagnostics_data
        .pdb_integrity
        .checks
        .iter()
        .find(|c| c.label == "PDB header compatibility")
        .expect("header compatibility check");
    assert!(
        matches!(header_check.status, backend::models::DiagStatus::Warn),
        "unexpected header check: {header_check:?}"
    );
    assert!(
        header_check.detail.contains("previous local PDB snapshot"),
        "unexpected header detail: {header_check:?}"
    );

    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec![PDB_HEADER_COMPATIBILITY_FIX_ID.to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    assert_eq!(read_pdb_header_compatibility_value(&pdb_path), 1);
}

#[test]
fn strict_repair_preview_and_apply_repopulate_edb_for_exported_playlist_gap_case() {
    let (_root, backend, usb, playlist_name) = setup_clean_strict_parity_fixture();

    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    let primary_playlist_id: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [&playlist_name],
            |row| row.get(0),
        )
        .expect("primary playlist id");
    let content_two: i64 = conn
        .query_row(
            "SELECT content_id
             FROM playlist_content
             WHERE playlist_id = ?1
             ORDER BY sequenceNo ASC, content_id ASC
             LIMIT 1",
            [primary_playlist_id],
            |row| row.get(0),
        )
        .expect("playlist content row");
    conn.execute(
        "DELETE FROM playlist_content WHERE playlist_id = ?1 AND content_id = ?2",
        rusqlite::params![primary_playlist_id, content_two],
    )
    .expect("remove playlist entry from primary row");
    drop(conn);

    let parity_before = backend.run_usb_parity_report(RunUsbParityReportRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(parity_before.ok, "parity before failed: {parity_before:?}");
    let before_playlist = parity_before
        .data
        .expect("parity before data")
        .playlist_details
        .into_iter()
        .find(|d| d.name == playlist_name)
        .expect("playlist in parity before");
    assert_eq!(before_playlist.pdb_tracks, 1);
    assert_eq!(before_playlist.edb_tracks, 0);
    assert_eq!(before_playlist.only_in_pdb, 1);

    let repair = backend.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let repair_data = repair.data.expect("repair data");
    assert!(
        repair_data
            .applied_fixes
            .iter()
            .any(|line| line.contains("merged 1 playlist(s)")),
        "strict repair should use the PDB-primary repopulation path: {repair_data:?}"
    );

    assert!(
        repair_data.failed_fixes.is_empty(),
        "strict repair should not report failed fixes: {repair_data:?}"
    );

    let parity_after = backend.run_usb_parity_report(RunUsbParityReportRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(parity_after.ok, "parity after failed: {parity_after:?}");
    let after_playlist = parity_after
        .data
        .expect("parity after data")
        .playlist_details
        .into_iter()
        .find(|d| d.name == playlist_name)
        .expect("playlist in parity after");
    assert_eq!(after_playlist.pdb_tracks, 1);
    assert_eq!(after_playlist.edb_tracks, 1);
    assert_eq!(after_playlist.only_in_pdb, 0);
    assert_eq!(after_playlist.only_in_edb, 0);
    assert!(!after_playlist.order_mismatch);
    assert!(after_playlist.playlist_id_match);
    assert!(after_playlist.sort_order_match);
}

#[test]
fn strict_repair_leaves_unrelated_playlists_unchanged() {
    let (_root, backend, usb, target_playlist, control_playlist) =
        setup_two_playlist_strict_parity_fixture();

    let initial_repair =
        backend.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
            apply: true,
            selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
        });
    assert!(
        initial_repair.ok,
        "initial repair failed: {initial_repair:?}"
    );

    let control_before = parity_detail_for_playlist(&backend, &usb, &control_playlist);
    let control_members_before = edb_playlist_member_count(&usb, &control_playlist);
    assert!(
        matches!(control_before.status, backend::models::DiagStatus::Pass),
        "control playlist should be strict-clean after initial repair: {control_before:?}"
    );

    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    let target_playlist_id: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [&target_playlist],
            |row| row.get(0),
        )
        .expect("target playlist id");
    let target_content_id: i64 = conn
        .query_row(
            "SELECT content_id
             FROM playlist_content
             WHERE playlist_id = ?1
             ORDER BY sequenceNo ASC, content_id ASC
             LIMIT 1",
            [target_playlist_id],
            |row| row.get(0),
        )
        .expect("target playlist content row");
    conn.execute(
        "DELETE FROM playlist_content WHERE playlist_id = ?1 AND content_id = ?2",
        rusqlite::params![target_playlist_id, target_content_id],
    )
    .expect("remove target playlist entry from eDB");
    drop(conn);

    let repair = backend.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let repair_data = repair.data.expect("repair data");
    assert!(
        repair_data
            .applied_fixes
            .iter()
            .any(|line| line.contains("Upgrade Export Data To Strict Parity")),
        "strict repair should apply to the target playlist: {repair_data:?}"
    );

    let target_after = parity_detail_for_playlist(&backend, &usb, &target_playlist);
    assert!(
        matches!(target_after.status, backend::models::DiagStatus::Pass),
        "target playlist should be restored to strict pass: {target_after:?}"
    );

    let control_after = parity_detail_for_playlist(&backend, &usb, &control_playlist);
    let control_members_after = edb_playlist_member_count(&usb, &control_playlist);
    assert!(control_after.sort_order_match);
    assert_eq!(control_after.pdb_sort_order, control_after.edb_sort_order);
    assert_eq!(
        control_after.pdb_sort_order, control_before.pdb_sort_order,
        "strict repair should preserve PDB playlist sorting for unrelated playlists"
    );

    assert_same_parity_detail(&control_after, &control_before);
    assert_eq!(control_members_after, control_members_before);
}

#[test]
fn strict_repair_syncs_all_edb_sort_orders_to_pdb() {
    // Verifies that after strict parity repair, eDB sequenceNos match PDB sort_orders
    // even when eDB sequenceNos start at arbitrary values far from PDB values.
    let (_root, backend, usb, target_playlist, control_playlist) =
        setup_two_playlist_strict_parity_fixture();

    // Corrupt eDB: set both playlists to arbitrary sequenceNos unrelated to PDB values.
    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    conn.execute(
        "UPDATE playlist SET sequenceNo = 50 WHERE name = ?1",
        [&target_playlist],
    )
    .expect("corrupt target sequenceNo");
    conn.execute(
        "UPDATE playlist SET sequenceNo = 100 WHERE name = ?1",
        [&control_playlist],
    )
    .expect("corrupt control sequenceNo");

    // Also remove a content entry from target to trigger strict parity detection.
    let target_id: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 LIMIT 1",
            [&target_playlist],
            |r| r.get(0),
        )
        .expect("target id");
    let content_id: i64 = conn
        .query_row(
            "SELECT content_id FROM playlist_content WHERE playlist_id = ?1 LIMIT 1",
            [target_id],
            |r| r.get(0),
        )
        .expect("content id");
    conn.execute(
        "DELETE FROM playlist_content WHERE playlist_id = ?1 AND content_id = ?2",
        rusqlite::params![target_id, content_id],
    )
    .expect("delete content entry");
    drop(conn);

    let repair = backend.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");

    let target_after = parity_detail_for_playlist(&backend, &usb, &target_playlist);
    let control_after = parity_detail_for_playlist(&backend, &usb, &control_playlist);

    assert!(
        matches!(target_after.status, backend::models::DiagStatus::Pass),
        "target should be strict pass after repair: {target_after:?}"
    );
    assert_eq!(
        target_after.pdb_sort_order, target_after.edb_sort_order,
        "target eDB sequenceNo must match PDB sort_order after repair"
    );
    assert_eq!(
        control_after.pdb_sort_order, control_after.edb_sort_order,
        "control eDB sequenceNo must match PDB sort_order after repair (was diverged)"
    );
}

#[test]
fn strict_repair_is_idempotent_after_successful_upgrade() {
    let (_root, backend, usb, playlist_name) = setup_clean_strict_parity_fixture();

    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    let playlist_id: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [&playlist_name],
            |row| row.get(0),
        )
        .expect("playlist id");
    let content_id: i64 = conn
        .query_row(
            "SELECT content_id
             FROM playlist_content
             WHERE playlist_id = ?1
             ORDER BY sequenceNo ASC, content_id ASC
             LIMIT 1",
            [playlist_id],
            |row| row.get(0),
        )
        .expect("playlist content row");
    conn.execute(
        "DELETE FROM playlist_content WHERE playlist_id = ?1 AND content_id = ?2",
        rusqlite::params![playlist_id, content_id],
    )
    .expect("remove playlist entry from eDB");
    drop(conn);

    let first_repair =
        backend.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
            apply: true,
            selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
        });
    assert!(first_repair.ok, "first repair failed: {first_repair:?}");
    let first_repair_data = first_repair.data.expect("first repair data");
    assert!(
        first_repair_data
            .applied_fixes
            .iter()
            .any(|line| line.contains("Upgrade Export Data To Strict Parity")),
        "first strict repair should apply: {first_repair_data:?}"
    );

    let second_repair =
        backend.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
            apply: true,
            selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
        });
    assert!(second_repair.ok, "second repair failed: {second_repair:?}");
    let second_repair_data = second_repair.data.expect("second repair data");
    assert!(
        !second_repair_data
            .applied_fixes
            .iter()
            .any(|line| line.contains("Upgrade Export Data To Strict Parity")),
        "second strict repair run should be a no-op after convergence: {second_repair_data:?}"
    );
    assert!(
        second_repair_data
            .skipped_fixes
            .iter()
            .any(|line| line.contains("Upgrade Export Data To Strict Parity: nothing to apply")),
        "second strict repair run should explicitly report nothing to apply: {second_repair_data:?}"
    );
    assert!(
        second_repair_data.failed_fixes.is_empty(),
        "second strict repair run should not fail: {second_repair_data:?}"
    );

    let playlist_after = parity_detail_for_playlist(&backend, &usb, &playlist_name);
    assert!(
        matches!(playlist_after.status, backend::models::DiagStatus::Pass),
        "playlist should remain strict-clean after idempotent rerun: {playlist_after:?}"
    );
}

#[test]
fn strict_repair_pdb_primary_repopulates_thin_edb_and_restores_strict_parity() {
    let (_root, backend, usb, playlist_name) = setup_clean_strict_parity_fixture();

    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    let playlist_id: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [&playlist_name],
            |row| row.get(0),
        )
        .expect("playlist id");
    let content_ids = {
        let mut stmt = conn
            .prepare(
                "SELECT content_id
                 FROM playlist_content
                 WHERE playlist_id = ?1
                 ORDER BY sequenceNo ASC, content_id ASC",
            )
            .expect("prepare content ids");
        let rows = stmt
            .query_map([playlist_id], |row| row.get::<_, i64>(0))
            .expect("query content ids");
        rows.collect::<Result<Vec<_>, _>>()
            .expect("collect content ids")
    };
    assert_eq!(
        content_ids.len(),
        1,
        "fixture should have one exported track"
    );
    conn.execute(
        "DELETE FROM playlist_content WHERE playlist_id = ?1",
        [playlist_id],
    )
    .expect("delete playlist_content rows");
    for content_id in &content_ids {
        conn.execute("DELETE FROM content WHERE content_id = ?1", [content_id])
            .expect("delete content row");
    }
    drop(conn);

    let parity_before = parity_detail_for_playlist(&backend, &usb, &playlist_name);
    assert_eq!(parity_before.pdb_tracks, 1);
    assert_eq!(parity_before.edb_tracks, 0);
    assert_eq!(parity_before.only_in_pdb, 1);
    assert_eq!(parity_before.only_in_edb, 0);
    assert!(
        !matches!(parity_before.status, backend::models::DiagStatus::Pass),
        "playlist should no longer be strict-clean once eDB is thinned: {parity_before:?}"
    );

    let repair = backend.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let repair_data = repair.data.expect("repair data");
    assert!(
        repair_data
            .applied_fixes
            .iter()
            .any(|line| line.contains("merged 1 playlist(s)")),
        "strict repair should take the PDB-primary repopulation path: {repair_data:?}"
    );
    assert!(
        repair_data.failed_fixes.is_empty(),
        "strict repair should not report failures in the PDB-primary path: {repair_data:?}"
    );

    let parity_after = parity_detail_for_playlist(&backend, &usb, &playlist_name);
    assert!(
        matches!(parity_after.status, backend::models::DiagStatus::Pass),
        "PDB-primary strict repair should restore full strict parity: {parity_after:?}"
    );
    assert_eq!(parity_after.pdb_tracks, 1);
    assert_eq!(parity_after.edb_tracks, 1);
    assert_eq!(parity_after.only_in_pdb, 0);
    assert_eq!(parity_after.only_in_edb, 0);
    assert!(parity_after.playlist_id_match);
    assert!(parity_after.sort_order_match);
    assert_eq!(parity_after.pdb_missing_core_metadata, 0);
    assert_eq!(parity_after.edb_missing_core_metadata, 0);
    assert_eq!(edb_playlist_member_count(&usb, &playlist_name), 1);
}

#[test]
fn strict_repair_avoids_playlist_id_collisions_with_existing_pdb_folders() {
    let (_root, backend, usb, _target_playlist, control_playlist) =
        setup_two_playlist_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let collision_id = 9000i64;

    mutate_first_pdb_playlist_tree_row_to_folder_with_id(&pdb_path, collision_id as u32);

    let conn = open_edb(&vendor_db);
    let control_playlist_id: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [&control_playlist],
            |row| row.get(0),
        )
        .expect("control playlist id");
    conn.execute(
        "UPDATE playlist_content SET playlist_id = ?1 WHERE playlist_id = ?2",
        rusqlite::params![collision_id, control_playlist_id],
    )
    .expect("move playlist_content rows to collision id");
    conn.execute(
        "UPDATE playlist SET playlist_id = ?1 WHERE playlist_id = ?2",
        rusqlite::params![collision_id, control_playlist_id],
    )
    .expect("update playlist id to collision id");
    drop(conn);

    let repair = backend.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let repair_data = repair.data.expect("repair data");
    assert!(
        repair_data
            .applied_fixes
            .iter()
            .any(|line| line.contains("Upgrade Export Data To Strict Parity")),
        "strict repair should apply in folder-id-collision case: {repair_data:?}"
    );

    let parsed = backend::pdb_reader::parse_pdb(&pdb_path).expect("parse repaired pdb");
    let folder_ids = parsed
        .playlist_tree
        .iter()
        .filter(|row| row.row_is_folder)
        .map(|row| row.id)
        .collect::<Vec<_>>();
    let leaf_rows = parsed
        .playlist_tree
        .iter()
        .filter(|row| !row.row_is_folder)
        .collect::<Vec<_>>();
    let unique_ids = parsed
        .playlist_tree
        .iter()
        .map(|row| row.id)
        .collect::<std::collections::HashSet<_>>();

    assert!(
        folder_ids.contains(&(collision_id as u32)),
        "folder row should keep the collision id in test setup: {:?}",
        parsed.playlist_tree
    );
    assert_eq!(
        unique_ids.len(),
        parsed.playlist_tree.len(),
        "playlist tree IDs must be globally unique after strict repair: {:?}",
        parsed.playlist_tree
    );

    let control_leaf = leaf_rows
        .into_iter()
        .find(|row| row.name == control_playlist)
        .expect("control playlist leaf row");
    assert_ne!(
        control_leaf.id, collision_id as u32,
        "leaf playlist ID must be remapped away from folder ID collision"
    );
}

#[test]
fn strict_repair_preserves_existing_pdb_playlist_ids_for_matched_playlists() {
    let (_root, backend, usb, playlist_a, playlist_b) = setup_two_playlist_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");

    let parsed_before = backend::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb before");
    let expected_id_a = parsed_before
        .playlist_tree
        .iter()
        .find(|row| !row.row_is_folder && row.name == playlist_a)
        .map(|row| row.id)
        .expect("playlist a id before");
    let expected_id_b = parsed_before
        .playlist_tree
        .iter()
        .find(|row| !row.row_is_folder && row.name == playlist_b)
        .map(|row| row.id)
        .expect("playlist b id before");

    let conn = open_edb(&vendor_db);
    let old_a: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [&playlist_a],
            |row| row.get(0),
        )
        .expect("playlist a id in edb");
    let old_b: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [&playlist_b],
            |row| row.get(0),
        )
        .expect("playlist b id in edb");
    // Force eDB IDs away from existing PDB IDs to ensure repair chooses the PDB IDs.
    let new_a = old_a + 5000;
    let new_b = old_b + 6000;
    conn.execute(
        "UPDATE playlist_content SET playlist_id = ?1 WHERE playlist_id = ?2",
        rusqlite::params![new_a, old_a],
    )
    .expect("move playlist_content for a");
    conn.execute(
        "UPDATE playlist_content SET playlist_id = ?1 WHERE playlist_id = ?2",
        rusqlite::params![new_b, old_b],
    )
    .expect("move playlist_content for b");
    conn.execute(
        "UPDATE playlist SET playlist_id = ?1 WHERE playlist_id = ?2",
        rusqlite::params![new_a, old_a],
    )
    .expect("update playlist id for a");
    conn.execute(
        "UPDATE playlist SET playlist_id = ?1 WHERE playlist_id = ?2",
        rusqlite::params![new_b, old_b],
    )
    .expect("update playlist id for b");
    drop(conn);

    let repair = backend.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");

    let parsed_after = backend::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb after");
    let actual_id_a = parsed_after
        .playlist_tree
        .iter()
        .find(|row| !row.row_is_folder && row.name == playlist_a)
        .map(|row| row.id)
        .expect("playlist a id after");
    let actual_id_b = parsed_after
        .playlist_tree
        .iter()
        .find(|row| !row.row_is_folder && row.name == playlist_b)
        .map(|row| row.id)
        .expect("playlist b id after");
    assert_eq!(
        actual_id_a, expected_id_a,
        "matched playlist A should keep existing PDB ID"
    );
    assert_eq!(
        actual_id_b, expected_id_b,
        "matched playlist B should keep existing PDB ID"
    );

    let conn = open_edb(&vendor_db);
    let edb_id_a: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [&playlist_a],
            |row| row.get(0),
        )
        .expect("playlist a id in edb after");
    let edb_id_b: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [&playlist_b],
            |row| row.get(0),
        )
        .expect("playlist b id in edb after");
    assert_eq!(
        edb_id_a as u32, expected_id_a,
        "eDB A should mirror preserved PDB ID"
    );
    assert_eq!(
        edb_id_b as u32, expected_id_b,
        "eDB B should mirror preserved PDB ID"
    );
}

#[test]
fn operational_diagnostics_do_not_walk_usbanlz_files() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();

    // A filesystem-heavy ANLZ scan would report this unreferenced empty bundle
    // member. Operational diagnostics must stay DB-only and ignore it; explicit
    // repair/parity tooling owns expensive filesystem scans.
    let stray_dir = usb
        .join(USB_VENDOR_ROOT_DIR)
        .join("USBANLZ")
        .join("P0AA")
        .join("DEADBEEF");
    fs::create_dir_all(&stray_dir).expect("create stray analysis dir");
    fs::write(stray_dir.join("ANLZ0000.DAT"), []).expect("write empty stray analysis file");

    let diagnostics = backend.run_usb_diagnostics(RunUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(diagnostics.ok, "diagnostics failed: {diagnostics:?}");
    let data = diagnostics.data.expect("diagnostics data");

    assert!(
        data.warnings
            .iter()
            .all(|warning| !warning.message.contains("analysis file appears empty")),
        "operational diagnostics should not scan raw USBANLZ files: {:?}",
        data.warnings
    );
    assert!(
        data.analysis_integrity
            .checks
            .iter()
            .any(|check| check.label == "PDB analysis refs"),
        "diagnostics should report DB-only PDB analysis refs: {:?}",
        data.analysis_integrity
    );
    assert!(
        data.analysis_integrity
            .checks
            .iter()
            .any(|check| check.label == "eDB analysis refs"),
        "diagnostics should report DB-only eDB analysis refs: {:?}",
        data.analysis_integrity
    );
    for forbidden_label in [
        "USBANLZ directory",
        "Analysis files",
        "Empty files",
        "Unreadable files",
        "Track analysis refs",
    ] {
        assert!(
            data.analysis_integrity
                .checks
                .iter()
                .all(|check| check.label != forbidden_label),
            "operational diagnostics should not expose filesystem ANLZ check '{forbidden_label}': {:?}",
            data.analysis_integrity
        );
    }
}

#[test]
fn strict_repair_chooses_richer_side_when_membership_matches_but_metadata_differs() {
    // PDB-primary metadata-only case: eDB membership stays intact, but required eDB metadata is thinned.
    let (_root_pdb, backend_pdb, usb_pdb, playlist_pdb) = setup_clean_strict_parity_fixture();
    let vendor_db = vendor_db_dir(&usb_pdb).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    let content_id: i64 = conn
        .query_row(
            "SELECT pc.content_id
             FROM playlist p
             JOIN playlist_content pc ON pc.playlist_id = p.playlist_id
             WHERE p.name = ?1
             ORDER BY pc.sequenceNo ASC, pc.content_id ASC
             LIMIT 1",
            [&playlist_pdb],
            |row| row.get(0),
        )
        .expect("content id for pdb-primary metadata-only case");
    conn.execute(
        "UPDATE content
         SET album_id = NULL,
             key_id = NULL,
             image_id = NULL,
             bpmx100 = NULL,
             length = NULL,
             analysisDataFilePath = NULL
         WHERE content_id = ?1",
        [content_id],
    )
    .expect("thin eDB metadata");
    drop(conn);

    let before_pdb = parity_detail_for_playlist(&backend_pdb, &usb_pdb, &playlist_pdb);
    assert_eq!(before_pdb.only_in_pdb, 0);
    assert_eq!(before_pdb.only_in_edb, 0);
    assert!(
        before_pdb.edb_missing_core_metadata > 0,
        "eDB metadata thinning should be visible in parity: {before_pdb:?}"
    );

    let repair_pdb =
        backend_pdb.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
            usb_root: Some(usb_pdb.to_string_lossy().to_string()),
            apply: true,
            selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
        });
    assert!(
        repair_pdb.ok,
        "PDB-primary metadata-only repair failed: {repair_pdb:?}"
    );
    let repair_pdb_data = repair_pdb
        .data
        .expect("PDB-primary metadata-only repair data");
    assert!(
        repair_pdb_data
            .applied_fixes
            .iter()
            .any(|line| line.contains("merged 1 playlist(s)")),
        "expected PDB-primary repair for metadata-only mismatch: {repair_pdb_data:?}"
    );
    let after_pdb = parity_detail_for_playlist(&backend_pdb, &usb_pdb, &playlist_pdb);
    assert!(
        matches!(after_pdb.status, backend::models::DiagStatus::Pass),
        "PDB-primary metadata-only repair should restore strict parity: {after_pdb:?}"
    );

    // eDB-primary metadata-only case: PDB membership stays intact, but required PDB metadata is thinned.
    let (_root_edb, backend_edb, usb_edb, playlist_edb) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb_edb).join("export.pdb");
    thin_first_pdb_track_row_fields(&pdb_path, true, true);

    let before_edb = parity_detail_for_playlist(&backend_edb, &usb_edb, &playlist_edb);
    assert_eq!(before_edb.only_in_pdb, 0);
    assert_eq!(before_edb.only_in_edb, 0);
    assert!(
        before_edb.pdb_missing_core_metadata > 0,
        "PDB metadata thinning should be visible in parity: {before_edb:?}"
    );

    let repair_edb =
        backend_edb.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
            usb_root: Some(usb_edb.to_string_lossy().to_string()),
            apply: true,
            selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
        });
    assert!(
        repair_edb.ok,
        "eDB-primary metadata-only repair failed: {repair_edb:?}"
    );
    let repair_edb_data = repair_edb
        .data
        .expect("eDB-primary metadata-only repair data");
    assert!(
        repair_edb_data
            .applied_fixes
            .iter()
            .any(|line| line.contains("merged 1 playlist(s)")),
        "expected eDB-primary repair for metadata-only mismatch: {repair_edb_data:?}"
    );
    let after_edb = parity_detail_for_playlist(&backend_edb, &usb_edb, &playlist_edb);
    assert!(
        matches!(after_edb.status, backend::models::DiagStatus::Pass),
        "eDB-primary metadata-only repair should restore strict parity: {after_edb:?}"
    );
}

#[test]
fn strict_repair_copies_exact_source_artwork_paths_in_both_directions() {
    // PDB-primary case: thin eDB artwork linkage and verify strict repair restores
    // the exact PDB artwork path instead of deriving a new one.
    let (_root_pdb, backend_pdb, usb_pdb, playlist_pdb) =
        setup_clean_strict_parity_fixture_with_artwork();
    let (_pdb_track_id, _pdb_artwork_id, expected_pdb_artwork_path) =
        first_playlist_pdb_artwork(&usb_pdb, &playlist_pdb);
    let vendor_db_pdb = vendor_db_dir(&usb_pdb).join("exportLibrary.db");
    let conn_pdb = open_edb(&vendor_db_pdb);
    let artwork_fk = edb_artwork_fk_column(&conn_pdb);
    let (content_id_pdb, _existing_image_id, _existing_path) =
        first_playlist_edb_artwork(&usb_pdb, &playlist_pdb);
    let thin_sql = format!(
        "UPDATE content
         SET album_id = NULL,
             key_id = NULL,
             {artwork_fk} = NULL,
             bpmx100 = NULL,
             length = NULL,
             analysisDataFilePath = NULL
         WHERE content_id = ?1"
    );
    conn_pdb
        .execute(&thin_sql, [content_id_pdb])
        .expect("thin eDB metadata incl artwork");
    drop(conn_pdb);

    let repair_pdb =
        backend_pdb.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
            usb_root: Some(usb_pdb.to_string_lossy().to_string()),
            apply: true,
            selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
        });
    assert!(
        repair_pdb.ok,
        "PDB-primary artwork repair failed: {repair_pdb:?}"
    );
    let (_content_id_after, _image_id_after, repaired_edb_artwork_path) =
        first_playlist_edb_artwork(&usb_pdb, &playlist_pdb);
    assert_eq!(
        repaired_edb_artwork_path, expected_pdb_artwork_path,
        "PDB-primary strict repair must copy the exact PDB artwork path into eDB"
    );

    // eDB-primary case: mutate eDB image path in-place to a same-length alternative and
    // verify strict repair patches the PDB artwork dictionary row to that exact path.
    let (_root_edb, backend_edb, usb_edb, playlist_edb) =
        setup_clean_strict_parity_fixture_with_artwork();
    let vendor_db_edb = vendor_db_dir(&usb_edb).join("exportLibrary.db");
    let conn_edb = open_edb(&vendor_db_edb);
    let (_content_id_edb, image_id_edb, original_edb_artwork_path) =
        first_playlist_edb_artwork(&usb_edb, &playlist_edb);
    let replacement_edb_artwork_path = if original_edb_artwork_path.contains("/a") {
        original_edb_artwork_path.replacen("/a", "/b", 1)
    } else {
        original_edb_artwork_path.replacen("/b", "/a", 1)
    };
    assert_ne!(
        replacement_edb_artwork_path, original_edb_artwork_path,
        "expected artwork path mutation target"
    );
    assert_eq!(
        replacement_edb_artwork_path.len(),
        original_edb_artwork_path.len(),
        "PDB artwork patch helper currently requires same-length replacements"
    );
    conn_edb
        .execute(
            "UPDATE image SET path = ?1 WHERE image_id = ?2",
            rusqlite::params![replacement_edb_artwork_path, image_id_edb],
        )
        .expect("mutate eDB image path");
    drop(conn_edb);

    let pdb_path = vendor_db_dir(&usb_edb).join("export.pdb");
    thin_first_pdb_track_row_fields(&pdb_path, true, true);
    let repair_edb =
        backend_edb.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
            usb_root: Some(usb_edb.to_string_lossy().to_string()),
            apply: true,
            selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
        });
    assert!(
        repair_edb.ok,
        "eDB-primary artwork repair failed: {repair_edb:?}"
    );
    let (_track_id_after, _artwork_id_after, repaired_pdb_artwork_path) =
        first_playlist_pdb_artwork(&usb_edb, &playlist_edb);
    assert_eq!(
        repaired_pdb_artwork_path, replacement_edb_artwork_path,
        "eDB-primary strict repair must copy the exact eDB artwork path into PDB"
    );
}

#[test]
fn strict_repair_restores_path_only_pdb_mismatch_from_edb() {
    let (_root, backend, usb, playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    mutate_first_pdb_analysis_path(&pdb_path);

    let before = parity_detail_for_playlist(&backend, &usb, &playlist_name);
    assert_eq!(
        before.path_mismatch_tracks, 1,
        "path-only mutation should surface exactly one path mismatch: {before:?}"
    );
    assert_eq!(
        before.matched_tracks, 1,
        "analysis-path-only mutation should preserve track identity matching: {before:?}"
    );
    assert_eq!(
        before.only_in_pdb, 0,
        "analysis-path-only case should not create only-in-PDB drift: {before:?}"
    );
    assert_eq!(
        before.only_in_edb, 0,
        "analysis-path-only case should not create only-in-eDB drift: {before:?}"
    );
    assert_eq!(
        before.pdb_missing_core_metadata, 0,
        "path-only case should not rely on metadata gaps: {before:?}"
    );
    assert_eq!(
        before.dictionary_id_issue_tracks, 0,
        "path-only case should not rely on dictionary-id issues: {before:?}"
    );
    assert!(
        matches!(before.status, backend::models::DiagStatus::Fail),
        "path-only mismatch should fail strict parity before repair: {before:?}"
    );

    let repair = backend.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });
    assert!(repair.ok, "path-only repair failed: {repair:?}");
    let repair_data = repair.data.expect("path-only repair data");
    assert!(
        repair_data
            .applied_fixes
            .iter()
            .any(|line| line.contains("merged 1 playlist(s)")),
        "path-only case should rewrite PDB from richer eDB data: {repair_data:?}"
    );

    let after = parity_detail_for_playlist(&backend, &usb, &playlist_name);
    assert_eq!(
        after.path_mismatch_tracks, 0,
        "repair should clear path mismatches: {after:?}"
    );
    assert!(
        matches!(after.status, backend::models::DiagStatus::Pass),
        "path-only strict repair should restore strict parity pass: {after:?}"
    );
}

#[test]
fn strict_repair_retains_small_and_medium_artwork_variants_together() {
    let (_root, backend, usb, playlist_name) = setup_clean_strict_parity_fixture_with_artwork();
    let (_track_id_before, _artwork_id_before, small_artwork_path) =
        first_playlist_pdb_artwork(&usb, &playlist_name);
    let medium_artwork_path = small_artwork_path.replacen(".jpg", "_m.jpg", 1);
    let small_abs = usb.join(small_artwork_path.trim_start_matches('/'));
    let medium_abs = usb.join(medium_artwork_path.trim_start_matches('/'));
    assert!(small_abs.is_file(), "small artwork missing before repair");
    assert!(medium_abs.is_file(), "medium artwork missing before repair");

    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    let playlist_id: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [&playlist_name],
            |row| row.get(0),
        )
        .expect("playlist id");
    let content_id: i64 = conn
        .query_row(
            "SELECT content_id FROM playlist_content WHERE playlist_id = ?1 LIMIT 1",
            [playlist_id],
            |row| row.get(0),
        )
        .expect("content id");
    conn.execute(
        "DELETE FROM playlist_content WHERE playlist_id = ?1 AND content_id = ?2",
        rusqlite::params![playlist_id, content_id],
    )
    .expect("delete playlist content");
    drop(conn);

    let repair = backend.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");

    assert!(small_abs.is_file(), "small artwork missing after repair");
    assert!(medium_abs.is_file(), "medium artwork missing after repair");
}

#[test]
fn strict_repair_restores_unresolved_pdb_dictionary_ids_from_edb() {
    let (_root, backend, usb, playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    mutate_first_pdb_artist_id(&pdb_path, 999);

    let before = parity_detail_for_playlist(&backend, &usb, &playlist_name);
    assert_eq!(
        before.only_in_pdb, 0,
        "dictionary-id case should preserve membership: {before:?}"
    );
    assert_eq!(
        before.only_in_edb, 0,
        "dictionary-id case should preserve membership: {before:?}"
    );
    assert_eq!(
        before.path_mismatch_tracks, 0,
        "dictionary-id case should not rely on path mismatches: {before:?}"
    );
    assert!(
        before.dictionary_id_issue_tracks > 0,
        "broken artist dictionary id should be visible in strict parity: {before:?}"
    );
    assert!(
        before
            .sample_metadata_mismatches
            .iter()
            .any(|m| m.contains("artistDictId")),
        "expected explicit artist dictionary mismatch evidence: {before:?}"
    );
    assert!(
        matches!(before.status, backend::models::DiagStatus::Fail),
        "unresolved PDB dictionary ids should fail strict parity before repair: {before:?}"
    );

    let repair = backend.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });
    assert!(repair.ok, "dictionary-id repair failed: {repair:?}");
    let repair_data = repair.data.expect("dictionary-id repair data");
    assert!(
        repair_data
            .applied_fixes
            .iter()
            .any(|line| line.contains("merged 1 playlist(s)")),
        "dictionary-id case should rewrite PDB from richer eDB data: {repair_data:?}"
    );

    let after = parity_detail_for_playlist(&backend, &usb, &playlist_name);
    assert_eq!(
        after.dictionary_id_issue_tracks, 0,
        "strict repair should clear unresolved PDB dictionary ids: {after:?}"
    );
    assert!(
        matches!(after.status, backend::models::DiagStatus::Pass),
        "dictionary-id-focused strict repair should restore strict parity pass: {after:?}"
    );
}

#[test]
fn strict_repair_preserves_sharp_keys_from_edb() {
    let (_root, backend, usb, playlist_name) =
        setup_clean_strict_parity_fixture_with_local_key(Some("C#"));
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    thin_first_pdb_track_row_fields(&pdb_path, true, false);

    let repair = backend.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });
    assert!(repair.ok, "sharp-key repair failed: {repair:?}");

    let parsed = backend::pdb_reader::parse_pdb(&pdb_path).expect("parse repaired pdb");
    let playlist_id = parsed
        .playlist_tree
        .iter()
        .find(|row| !row.row_is_folder && row.name == playlist_name)
        .map(|row| row.id)
        .expect("playlist row");
    let track_id = parsed
        .playlist_entries
        .iter()
        .find(|entry| entry.playlist_id == playlist_id)
        .map(|entry| entry.track_id)
        .expect("playlist track");
    let track = parsed
        .tracks
        .iter()
        .find(|track| track.id == track_id)
        .expect("repaired pdb track");
    assert!(
        track.key_id > 0,
        "strict repair must restore a non-zero PDB key id for sharp keys"
    );
    assert_eq!(
        parsed.keys.get(&track.key_id).map(String::as_str),
        Some("C#"),
        "strict repair must preserve sharp keys exactly"
    );
}

#[test]
fn strict_repair_source_selection_matrix_uses_supported_deltas() {
    // eDB-primary metadata-only mismatch.
    let (_root_edb, backend_edb, usb_edb, playlist_edb) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb_edb).join("export.pdb");
    thin_first_pdb_track_row_fields(&pdb_path, true, true);
    let repair_edb =
        backend_edb.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
            usb_root: Some(usb_edb.to_string_lossy().to_string()),
            apply: true,
            selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
        });
    assert!(
        repair_edb.ok,
        "matrix eDB-primary repair failed: {repair_edb:?}"
    );
    let repair_edb_data = repair_edb.data.expect("matrix eDB-primary repair data");
    assert!(
        repair_edb_data
            .applied_fixes
            .iter()
            .any(|line| line.contains("merged 1 playlist(s)")),
        "matrix eDB-primary case should choose eDB as richer source: {repair_edb_data:?}"
    );
    assert!(
        matches!(
            parity_detail_for_playlist(&backend_edb, &usb_edb, &playlist_edb).status,
            backend::models::DiagStatus::Pass
        ),
        "matrix eDB-primary case should end strict-clean"
    );

    // PDB-primary metadata-only mismatch.
    let (_root_pdb, backend_pdb, usb_pdb, playlist_pdb) = setup_clean_strict_parity_fixture();
    let vendor_db = vendor_db_dir(&usb_pdb).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    let content_id: i64 = conn
        .query_row(
            "SELECT pc.content_id
             FROM playlist p
             JOIN playlist_content pc ON pc.playlist_id = p.playlist_id
             WHERE p.name = ?1
             ORDER BY pc.sequenceNo ASC, pc.content_id ASC
             LIMIT 1",
            [&playlist_pdb],
            |row| row.get(0),
        )
        .expect("content id for matrix pdb-primary");
    conn.execute(
        "UPDATE content
         SET album_id = NULL,
             key_id = NULL,
             image_id = NULL,
             bpmx100 = NULL,
             length = NULL,
             analysisDataFilePath = NULL
         WHERE content_id = ?1",
        [content_id],
    )
    .expect("thin eDB metadata for matrix pdb-primary");
    drop(conn);
    let repair_pdb =
        backend_pdb.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
            usb_root: Some(usb_pdb.to_string_lossy().to_string()),
            apply: true,
            selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
        });
    assert!(
        repair_pdb.ok,
        "matrix PDB-primary repair failed: {repair_pdb:?}"
    );
    let repair_pdb_data = repair_pdb.data.expect("matrix PDB-primary repair data");
    assert!(
        repair_pdb_data
            .applied_fixes
            .iter()
            .any(|line| line.contains("merged 1 playlist(s)")),
        "matrix PDB-primary case should choose PDB as richer source: {repair_pdb_data:?}"
    );
    assert!(
        matches!(
            parity_detail_for_playlist(&backend_pdb, &usb_pdb, &playlist_pdb).status,
            backend::models::DiagStatus::Pass
        ),
        "matrix PDB-primary case should end strict-clean"
    );

    // Neither side sufficient.
    let (_root_none, backend_none, usb_none, playlist_none) = setup_clean_strict_parity_fixture();
    let vendor_db = vendor_db_dir(&usb_none).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    let playlist_id: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [&playlist_none],
            |row| row.get(0),
        )
        .expect("playlist id for matrix neither");
    conn.execute(
        "DELETE FROM playlist_content WHERE playlist_id = ?1",
        [playlist_id],
    )
    .expect("delete playlist_content rows for matrix neither");
    conn.execute("DELETE FROM content", [])
        .expect("delete content rows for matrix neither");
    drop(conn);
    let parsed_before =
        backend::pdb_reader::parse_pdb(&vendor_db_dir(&usb_none).join("export.pdb"))
            .expect("parse pdb before matrix neither");
    let target_playlist_id = parsed_before
        .playlist_tree
        .iter()
        .find(|row| !row.row_is_folder && row.name == playlist_none)
        .map(|row| row.id)
        .expect("playlist id before matrix neither");
    let removed = backend_none.remove_usb_playlist(RemoveUsbPlaylistRequest {
        usb_root: Some(usb_none.to_string_lossy().to_string()),
        playlist_id: Some(format!("usb-pl-{target_playlist_id}")),
        playlist_name: playlist_none.clone(),
    });
    assert!(
        removed.ok,
        "remove usb playlist for matrix neither failed: {removed:?}"
    );
    let repair_none =
        backend_none.repair_usb_diagnostics(backend::models::RepairUsbDiagnosticsRequest {
            usb_root: Some(usb_none.to_string_lossy().to_string()),
            apply: true,
            selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
        });
    assert!(
        repair_none.ok,
        "matrix neither repair failed: {repair_none:?}"
    );
    let repair_none_data = repair_none.data.expect("matrix neither repair data");
    assert!(
        repair_none_data.applied_fixes.is_empty(),
        "matrix neither case should not apply repair: {repair_none_data:?}"
    );
    // When no playlists exist on either side, parity has nothing to fail,
    // so the merge fix is not proposed at all ("not selected" skip).
    assert!(
        repair_none_data
            .skipped_fixes
            .iter()
            .any(|line| line.contains("Upgrade Export Data To Strict Parity")),
        "matrix neither case should skip strict repair: {repair_none_data:?}"
    );
}

#[test]
fn parity_report_overall_status_and_required_summary_use_strict_player_wording() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");
    copy_audio_fixture(&media, "noart/track_no_art.mp3", "Artist - One.mp3");
    copy_audio_fixture(&media, "embedded/track_embedded.mp3", "Artist - Two.mp3");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track_ids = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .map(|t| t.id)
        .collect::<Vec<_>>();
    assert_eq!(track_ids.len(), 2);
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let playlist_name = "Strict Summary";
    let created = backend.create_playlist(CreatePlaylistRequest {
        name: playlist_name.to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;
    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: track_ids.clone(),
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add failed: {added:?}");
    assert_eq!(added.data.expect("add data").added, 2);

    let export = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export.ok, "export failed: {export:?}");
    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    let primary_playlist_id: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [playlist_name],
            |row| row.get(0),
        )
        .expect("playlist id");
    let content_two: i64 = conn
        .query_row(
            "SELECT content_id FROM playlist_content WHERE playlist_id = ?1 ORDER BY sequenceNo DESC LIMIT 1",
            [primary_playlist_id],
            |row| row.get(0),
        )
        .expect("second content id");
    conn.execute(
        "DELETE FROM playlist_content WHERE playlist_id = ?1 AND content_id = ?2",
        rusqlite::params![primary_playlist_id, content_two],
    )
    .expect("remove one playlist member from eDB");
    drop(conn);

    let parity = backend.run_usb_parity_report(RunUsbParityReportRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(parity.ok, "parity failed: {parity:?}");
    let parity_data = parity.data.expect("parity data");

    let overall_check = parity_data
        .checks
        .iter()
        .find(|c| c.label == "Overall player parity status")
        .expect("overall player parity status check");
    assert!(
        overall_check.detail.contains("playlists checked:")
            && overall_check.detail.contains("failing playlists:"),
        "overall strict parity summary should include playlist-level totals: {:?}",
        overall_check
    );

    let required_section = parity_data
        .checks
        .iter()
        .find(|c| c.label == "Parity-report section (required)")
        .expect("required parity-report summary");
    assert!(
        required_section.detail.contains("See parity summary rows"),
        "required section should point to structured summary rows: {:?}",
        required_section
    );
    assert!(
        !required_section.detail.contains("playlist-only-in-PDB")
            && !required_section.detail.contains("playlist-only-in-eDB"),
        "required section should not use outdated playlist-only wording: {:?}",
        required_section
    );
    let membership_only_in_pdb = parity_data
        .summary_rows
        .iter()
        .find(|row| row.label == "Membership only-in-PDB")
        .expect("membership only-in-PDB summary row");
    assert_eq!(membership_only_in_pdb.count, 1);
    let membership_only_in_edb = parity_data
        .summary_rows
        .iter()
        .find(|row| row.label == "Membership only-in-eDB")
        .expect("membership only-in-eDB summary row");
    assert_eq!(membership_only_in_edb.count, 0);
}

#[test]
fn parity_report_fails_when_membership_exists_only_in_pdb() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");
    copy_audio_fixture(&media, "noart/track_no_art.mp3", "Artist - One.mp3");
    copy_audio_fixture(&media, "embedded/track_embedded.mp3", "Artist - Two.mp3");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track_ids = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .map(|t| t.id)
        .collect::<Vec<_>>();
    assert_eq!(track_ids.len(), 2);
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let playlist_name = "Only In eDB";
    let created = backend.create_playlist(CreatePlaylistRequest {
        name: playlist_name.to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;
    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: track_ids.clone(),
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add failed: {added:?}");
    assert_eq!(added.data.expect("add data").added, 2);

    let export = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export.ok, "export failed: {export:?}");

    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    let primary_playlist_id: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [playlist_name],
            |row| row.get(0),
        )
        .expect("playlist id");
    let content_two: i64 = conn
        .query_row(
            "SELECT content_id FROM playlist_content WHERE playlist_id = ?1 ORDER BY sequenceNo DESC LIMIT 1",
            [primary_playlist_id],
            |row| row.get(0),
        )
        .expect("content id");
    conn.execute(
        "DELETE FROM playlist_content WHERE playlist_id = ?1 AND content_id = ?2",
        rusqlite::params![primary_playlist_id, content_two],
    )
    .expect("remove one playlist member from eDB");
    drop(conn);

    let parity = backend.run_usb_parity_report(RunUsbParityReportRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(parity.ok, "parity failed: {parity:?}");
    let parity_data = parity.data.expect("parity data");

    let playlist = parity_data
        .playlist_details
        .iter()
        .find(|d| d.name == playlist_name)
        .expect("playlist detail");
    assert_eq!(playlist.only_in_edb, 0);
    assert_eq!(playlist.only_in_pdb, 1);
    assert!(
        matches!(playlist.status, backend::models::DiagStatus::Fail),
        "membership present only in PDB should fail strict parity: {:?}",
        playlist
    );

    let membership_check = parity_data
        .checks
        .iter()
        .find(|c| c.label == "Playlist membership parity")
        .expect("membership parity check");
    assert!(
        matches!(membership_check.status, backend::models::DiagStatus::Fail),
        "membership-only-in-PDB should fail strict membership parity: {:?}",
        membership_check
    );
    assert!(
        membership_check.detail.contains("only-in-PDB=1"),
        "membership check should surface the observed only-in-PDB count: {:?}",
        membership_check
    );
}

#[test]
fn playlist_resolution_stays_passable_for_partial_cross_source_match() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");
    copy_audio_fixture(
        &media,
        "noart/track_no_art.mp3",
        "Fixture Artist - Resolution Full.mp3",
    );
    copy_audio_fixture(
        &media,
        "embedded/track_embedded.mp3",
        "Fixture Artist - Resolution Partial One.mp3",
    );
    copy_audio_fixture(
        &media,
        "folder/track_folder.jpg.mp3",
        "Fixture Artist - Resolution Partial Two.mp3",
    );

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let scanned_tracks = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .map(|t| (t.file_path.clone(), t.id))
        .collect::<Vec<_>>();
    assert_eq!(scanned_tracks.len(), 3);
    let track_ids = scanned_tracks
        .iter()
        .map(|(_, id)| id.clone())
        .collect::<Vec<_>>();
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let full_playlist_name = "Resolution Full";
    let full_created = backend.create_playlist(CreatePlaylistRequest {
        name: full_playlist_name.to_string(),
    });
    assert!(full_created.ok, "create failed: {full_created:?}");
    let full_playlist_id = full_created.data.expect("playlist data").playlist_id;
    let full_track_id = scanned_tracks
        .iter()
        .find(|(file_path, _)| file_path.contains("Resolution Full.mp3"))
        .map(|(_, id)| id.clone())
        .expect("full playlist fixture track");
    let full_added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: full_playlist_id.clone(),
        track_ids: vec![full_track_id],
        dedupe: DedupeMode::Skip,
    });
    assert!(full_added.ok, "add failed: {full_added:?}");
    assert_eq!(full_added.data.expect("add data").added, 1);

    let playlist_name = "Resolution Partial";
    let created = backend.create_playlist(CreatePlaylistRequest {
        name: playlist_name.to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;
    let partial_track_ids = scanned_tracks
        .iter()
        .filter(|(file_path, _)| file_path.contains("Resolution Partial"))
        .map(|(_, id)| id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        partial_track_ids.len(),
        2,
        "expected two partial fixture tracks"
    );
    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: partial_track_ids,
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add failed: {added:?}");
    assert_eq!(added.data.expect("add data").added, 2);

    let full_export = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: full_playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(full_export.ok, "full export failed: {full_export:?}");

    let export = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export.ok, "export failed: {export:?}");

    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    let primary_playlist_id: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [playlist_name],
            |row| row.get(0),
        )
        .expect("playlist id");
    let content_two: i64 = conn
        .query_row(
            "SELECT content_id FROM playlist_content WHERE playlist_id = ?1 ORDER BY sequenceNo DESC LIMIT 1",
            [primary_playlist_id],
            |row| row.get(0),
        )
        .expect("content id");
    conn.execute(
        "DELETE FROM playlist_content WHERE playlist_id = ?1 AND content_id = ?2",
        rusqlite::params![primary_playlist_id, content_two],
    )
    .expect("remove one playlist member from eDB");
    drop(conn);

    let diagnostics = backend.run_usb_diagnostics(RunUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(diagnostics.ok, "diagnostics failed: {diagnostics:?}");
    let diagnostics_data = diagnostics.data.expect("diagnostics data");

    let playlist = diagnostics_data
        .playlist_details
        .iter()
        .find(|d| d.name == playlist_name)
        .expect("playlist detail");
    assert_eq!(playlist.total_entries, 2);
    assert_eq!(playlist.resolved_entries, 2);
    assert_eq!(playlist.pdb_entries, 2);
    assert_eq!(playlist.edb_entries, 1);
    assert_eq!(playlist.matched_entries, 1);
    assert!(
        matches!(playlist.status, backend::models::DiagStatus::Pass),
        "partial cross-source matching should stay operationally passable when playlist entries still resolve: {:?}",
        playlist
    );
    let full_playlist = diagnostics_data
        .playlist_details
        .iter()
        .find(|d| d.name == full_playlist_name)
        .expect("full playlist detail");
    assert_eq!(full_playlist.total_entries, 1);
    assert_eq!(full_playlist.resolved_entries, 1);
    assert_eq!(full_playlist.pdb_entries, 1);
    assert_eq!(full_playlist.edb_entries, 1);
    assert_eq!(full_playlist.matched_entries, 1);
    assert!(
        matches!(full_playlist.status, backend::models::DiagStatus::Pass),
        "control playlist should stay fully resolved: {:?}",
        full_playlist
    );
}

#[test]
fn playlist_resolution_warns_for_partially_resolved_playlist() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");
    copy_audio_fixture(
        &media,
        "formats/track_format_flac.flac",
        "Artist - One.flac",
    );
    copy_audio_fixture(&media, "formats/track_format_wav.wav", "Artist - Two.wav");
    copy_audio_fixture(&media, "formats/track_format_aif.aif", "Artist - Three.aif");
    copy_audio_fixture(&media, "noart/track_no_art.mp3", "Artist - Four.mp3");
    copy_audio_fixture(&media, "embedded/track_embedded.mp3", "Artist - Five.mp3");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track_ids = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .map(|t| t.id)
        .collect::<Vec<_>>();
    assert_eq!(track_ids.len(), 5);
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let playlist_name = "Partial Resolution";
    let created = backend.create_playlist(CreatePlaylistRequest {
        name: playlist_name.to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;
    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: track_ids.clone(),
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add failed: {added:?}");
    assert_eq!(added.data.expect("add data").added, 5);

    let export = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export.ok, "export failed: {export:?}");
    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    let primary_playlist_id: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [playlist_name],
            |row| row.get(0),
        )
        .expect("playlist id");
    let missing_content_id: i64 = conn
        .query_row(
            "SELECT content_id FROM playlist_content WHERE playlist_id = ?1 ORDER BY sequenceNo DESC LIMIT 1",
            [primary_playlist_id],
            |row| row.get(0),
        )
        .expect("missing content id");
    conn.execute(
        "DELETE FROM content WHERE content_id = ?1",
        [missing_content_id],
    )
    .expect("remove one referenced content row");
    drop(conn);

    let diagnostics = backend.run_usb_diagnostics(RunUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(diagnostics.ok, "diagnostics failed: {diagnostics:?}");
    let diagnostics_data = diagnostics.data.expect("diagnostics data");

    let playlist = diagnostics_data
        .playlist_details
        .iter()
        .find(|d| d.name == playlist_name)
        .expect("playlist detail");
    assert_eq!(playlist.total_entries, 5);
    assert_eq!(playlist.resolved_entries, 5);
    assert_eq!(playlist.pdb_entries, 5);
    assert_eq!(playlist.edb_entries, 4);
    assert_eq!(playlist.matched_entries, 4);
    assert!(
        matches!(playlist.status, backend::models::DiagStatus::Pass),
        "playlist resolution should stay operationally passable even when cross-source matching is partial: {:?}",
        playlist
    );
    assert!(
        (playlist.resolution_rate - 1.0).abs() < 0.0001,
        "expected resolution rate of 1.0 for fully resolved playlist entries: {:?}",
        playlist
    );
    assert!(
        (playlist.pdb_match_rate - 0.8).abs() < 0.0001,
        "expected PDB match rate of 0.8 for 4/5 matched cross-source entries: {:?}",
        playlist
    );
    assert!(
        (playlist.edb_match_rate - 1.0).abs() < 0.0001,
        "expected eDB match rate of 1.0 when all eDB entries are matched: {:?}",
        playlist
    );

    let overall_resolution = diagnostics_data
        .playlist_resolution
        .checks
        .iter()
        .find(|c| c.label == "Overall resolution")
        .expect("overall resolution check");
    assert!(
        overall_resolution
            .detail
            .contains("5/5 entries resolve (100.0%)"),
        "overall resolution check should match the observed operational coverage: {:?}",
        overall_resolution
    );

    let overlap_check = diagnostics_data
        .playlist_resolution
        .checks
        .iter()
        .find(|c| c.label == "PDB vs eDB key overlap (informational)")
        .expect("overlap check");
    assert!(
        overlap_check.detail.contains("matched 4 track keys")
            && overlap_check.detail.contains("PDB 80.0% (4/5)")
            && overlap_check.detail.contains("DB 80.0% (4/5)"),
        "cross-source overlap check should explain partial matching without degrading operational resolution: {:?}",
        overlap_check
    );
}

#[test]
fn parity_report_flags_player_quality_metadata_gaps_and_required_section() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");
    copy_audio_fixture(&media, "noart/track_no_art.mp3", "Artist - One.mp3");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track_ids = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .map(|t| t.id)
        .collect::<Vec<_>>();
    assert_eq!(track_ids.len(), 1);
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let playlist_name = "Player Quality";
    let created = backend.create_playlist(CreatePlaylistRequest {
        name: playlist_name.to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;
    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids,
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add failed: {added:?}");
    assert_eq!(added.data.expect("add data").added, 1);

    let export = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export.ok, "export failed: {export:?}");

    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    let playlist_row: (i64, i64) = conn
        .query_row(
            "SELECT p.playlist_id, pc.content_id
             FROM playlist p
             JOIN playlist_content pc ON pc.playlist_id = p.playlist_id
             WHERE p.name = ?1
             ORDER BY p.playlist_id ASC, pc.sequenceNo ASC
             LIMIT 1",
            [playlist_name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("playlist/content row");
    conn.execute(
        "UPDATE content
         SET album_id = 1,
             key_id = 1,
             image_id = 1,
             bpmx100 = 12345,
             length = 245
         WHERE content_id = ?1",
        [playlist_row.1],
    )
    .expect("enrich export db content");
    drop(conn);

    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    let pdb_bytes = fs::read(&pdb_path).expect("read export pdb");
    assert!(
        !pdb_bytes.is_empty(),
        "PDB should not be empty after export"
    );
    let last_byte = pdb_bytes
        .last()
        .copied()
        .expect("PDB should have at least one byte");
    fs::write(&pdb_path, {
        let mut mutated = pdb_bytes.clone();
        let idx = mutated.len() - 1;
        mutated[idx] = last_byte.wrapping_add(1);
        mutated
    })
    .expect("mutate export pdb");

    let parity = backend.run_usb_parity_report(RunUsbParityReportRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(parity.ok, "parity failed: {parity:?}");
    let parity_data = parity.data.expect("parity data");

    let overall_check = parity_data
        .checks
        .iter()
        .find(|c| c.label == "Overall player parity status")
        .expect("overall player parity status check");
    assert!(
        overall_check.detail.contains("playlists checked:")
            && overall_check.detail.contains("failing playlists:"),
        "overall strict parity summary should include playlist-level totals: {:?}",
        overall_check
    );

    let required_section = parity_data
        .checks
        .iter()
        .find(|c| c.label == "Parity-report section (required)")
        .expect("required parity-report summary");
    assert!(
        required_section.detail.contains("See parity summary rows"),
        "required section should point to structured summary rows: {:?}",
        required_section
    );
    let metadata_gaps = parity_data
        .summary_rows
        .iter()
        .find(|row| row.label == "PDB metadata gaps")
        .expect("PDB metadata gaps summary row");
    assert!(
        metadata_gaps.count >= 1,
        "expected strict metadata gap count: {metadata_gaps:?}"
    );
    let membership_only_in_pdb = parity_data
        .summary_rows
        .iter()
        .find(|row| row.label == "Membership only-in-PDB")
        .expect("membership only-in-PDB summary row");
    assert_eq!(membership_only_in_pdb.count, 0);

    let metadata_check = parity_data
        .checks
        .iter()
        .find(|c| c.label == "PDB metadata completeness")
        .expect("PDB metadata completeness check");
    assert!(
        !matches!(metadata_check.status, backend::models::DiagStatus::Pass),
        "mutated PDB should no longer be player-quality pass: {:?}",
        metadata_check
    );

    let playlist = parity_data
        .playlist_details
        .iter()
        .find(|d| d.name == playlist_name)
        .expect("playlist detail");
    assert!(
        playlist.pdb_missing_core_metadata > 0
            || playlist.artwork_mismatch_tracks > 0
            || !playlist.sample_metadata_mismatches.is_empty(),
        "playlist should expose metadata/content quality issues: {:?}",
        playlist
    );
}

#[test]
fn multi_track_export_produces_structurally_clean_pdb() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    // 12 unique-path copies of the same fixture so the PDB track table spans
    // multiple data pages, exercising sentinel and page-chain logic.
    for i in 0..12usize {
        copy_audio_fixture(
            &media,
            "noart/track_no_art.mp3",
            &format!("Fixture Artist - Track {i:02}.mp3"),
        );
    }

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb: {initialized:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan: {scan:?}");

    let all_ids: Vec<String> = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .iter()
        .map(|t| t.id.clone())
        .collect();
    assert_eq!(all_ids.len(), 12, "expected 12 scanned tracks");
    seed_tracks_as_analyzed(&data_dir, &all_ids);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Structural Check".to_string(),
    });
    assert!(created.ok, "create playlist: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: all_ids,
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add tracks: {added:?}");

    let export = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export.ok, "first export: {export:?}");

    assert_no_pdb_structural_repairs(&backend, &usb);
    assert_pdb_crossrefs_clean(&usb);

    // Second export (additive path): same playlist, verify structural integrity
    // is preserved after the additive writer runs again.
    let export2 = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export2.ok, "second export: {export2:?}");

    assert_no_pdb_structural_repairs(&backend, &usb);
    assert_pdb_crossrefs_clean(&usb);
}
