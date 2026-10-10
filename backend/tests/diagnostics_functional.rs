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
const PDB_DUPLICATE_PLAYLIST_ENTRIES_FIX_ID: &str = "repair_pdb_duplicate_playlist_entries";
const PDB_SENTINEL_U5_FIX_ID: &str = "repair_pdb_sentinel_u5_on_data_pages";
const PDB_WRONG_PAGE_FLAGS_FIX_ID: &str = "repair_pdb_wrong_page_flags";
const PDB_ZERO_TRANRF_FIX_ID: &str = "repair_pdb_zero_tranrf_on_track_pages";
const PDB_WRONG_TRACK_U5_FIX_ID: &str = "repair_pdb_wrong_track_u5_num_rl";
const PDB_WRONG_HISTORY_SHAPE_FIX_ID: &str = "repair_pdb_wrong_history_page_shape";
const PDB_WRONG_PLAYLIST_TREE_SHAPE_FIX_ID: &str = "repair_pdb_wrong_playlist_tree_shape";
/// A flat waveform with the same level on every band.
fn flat_waveform(len: usize, level: u8) -> backend::service::anlz::WaveformData {
    let band = vec![level.min(127); len];
    backend::service::anlz::WaveformData {
        peaks: vec![level; len],
        bands: vec![3; len],
        low_energy: band.clone(),
        mid_energy: band.clone(),
        high_energy: band.clone(),
        low_energy_full: band.clone(),
        mid_energy_full: band.clone(),
        high_energy_full: band,
        peak_level: 1.0,
    }
}

fn vendor_db_dir(usb_root: &Path) -> std::path::PathBuf {
    usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)
}

/// Locate the first non-sentinel, non-empty data page of `table_type` in a
/// real exported PDB. Mirrors the page-header scan every `detect_pdb_*`
/// function in `service::repair` performs: idx@+4, table_type@+8,
/// flags@+0x1b (0x64 = sentinel/index page, skipped).
fn find_pdb_data_page(bytes: &[u8], page_size: usize, table_type: u32) -> Option<usize> {
    let total = bytes.len() / page_size;
    (1..total).find(|&i| {
        let off = i * page_size;
        let idx = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
        if idx == 0 {
            return false;
        }
        let flags = bytes[off + 0x1b];
        if flags == 0x64 {
            return false;
        }
        let tt = u32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap());
        tt == table_type
    })
}

fn read_pdb_page_size(bytes: &[u8]) -> usize {
    u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize
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

    // Both tracks were copied from the same audio fixture, so they share
    // identical embedded ID3 title/artist tags and file size. Give each a
    // distinct title so they don't collide under export's content-fingerprint
    // (size + title + artist) match for tracks already on the USB — this
    // fixture wants two genuinely independent exported tracks, not one track
    // exported twice under different filenames.
    {
        let db_path = data_dir.join("backend.db");
        let conn = rusqlite::Connection::open(&db_path).expect("open backend db");
        for (idx, track_id) in track_ids.iter().enumerate() {
            conn.execute(
                "UPDATE tracks SET title = ?1 WHERE id = ?2",
                rusqlite::params![format!("Fixture No Art {idx}"), track_id],
            )
            .expect("seed distinct track title");
        }
    }

    let playlist_names = ["Repair Target".to_string(), "Repair Control".to_string()];
    for (playlist_name, track_id) in playlist_names.iter().zip(track_ids) {
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
fn diagnostics_and_repair_pdb_header_compatibility_ignores_backup_snapshot_drift() {
    // Regression test: a previous version of this check compared the current
    // value against the most recent backup snapshot and flagged "drift" when
    // they differed. Since the writer always emits 5 on a fresh export while
    // this repair (when it existed) patched to 1, that comparison oscillated
    // forever across repeated apply/export cycles — the exact anti-pattern
    // `docs/PDB.md` and this repair are meant to avoid. Both 1 and 5 are
    // confirmed validator-accepted, so a differing backup snapshot must not
    // be treated as an issue, and the current value must not be "corrected"
    // to match it.
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
        matches!(header_check.status, backend::models::DiagStatus::Pass),
        "known-compatible value differing from a backup snapshot must not warn: {header_check:?}"
    );

    let preview = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: false,
        selected_fix_ids: Vec::new(),
    });
    assert!(preview.ok, "repair preview failed: {preview:?}");
    let preview_data = preview.data.expect("preview data");
    assert!(
        !preview_data
            .proposed_fixes
            .iter()
            .any(|fix| fix.id == PDB_HEADER_COMPATIBILITY_FIX_ID),
        "no repair should be proposed for a known-compatible value: {:?}",
        preview_data.proposed_fixes
    );
    assert_eq!(read_pdb_header_compatibility_value(&pdb_path), 5);
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
fn strict_repair_carries_the_edb_rating_and_colour_into_the_pdb() {
    let (_root, backend, usb, playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");

    // rekordbox-style values in the eDB only (as an older app version left
    // them: its PDB rows had no rating or colour), and a playlist that needs
    // the strict repair: its eDB membership is gone, the track row stays.
    let conn = open_edb(&vendor_db);
    let playlist_id: i64 = conn
        .query_row(
            "SELECT playlist_id FROM playlist WHERE name = ?1 ORDER BY playlist_id ASC LIMIT 1",
            [&playlist_name],
            |row| row.get(0),
        )
        .expect("playlist id");
    conn.execute("UPDATE content SET rating = 3, color_id = 5", [])
        .expect("set eDB rating and colour");
    conn.execute(
        "DELETE FROM playlist_content WHERE playlist_id = ?1",
        [playlist_id],
    )
    .expect("delete playlist_content rows");
    drop(conn);
    let pdb_rating_colour = || {
        let parsed = backend::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb");
        assert_eq!(parsed.tracks.len(), 1);
        (parsed.tracks[0].rating, parsed.tracks[0].color_id)
    };
    assert_eq!(pdb_rating_colour(), (0, 0));

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
        "strict repair should run: {repair_data:?}"
    );

    assert_eq!(pdb_rating_colour(), (3, 5));
    let conn = open_edb(&vendor_db);
    let edb: (i64, i64) = conn
        .query_row("SELECT rating, color_id FROM content", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .expect("eDB rating and colour");
    assert_eq!(edb, (3, 5), "the eDB keeps its values");
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

// ── End-to-end PDB byte-corruption repair wiring ──────────────────────────
//
// These exercise the orchestrator wiring in
// `repair_usb_diagnostics_with_progress` (proposal + apply/skip branches
// per fix id) that the byte-level unit tests in `service::repair::tests`
// don't reach, by corrupting a single targeted field on a real page of a
// freshly exported PDB (same technique as `thin_first_pdb_track_row_fields`
// / `mutate_first_pdb_analysis_path` above) and driving the fix through
// `BackendCommands::repair_usb_diagnostics`.

fn read_write_pdb(pdb_path: &Path, mutate: impl FnOnce(&mut Vec<u8>, usize)) {
    let mut bytes = fs::read(pdb_path).expect("read export pdb");
    let page_size = read_pdb_page_size(&bytes);
    mutate(&mut bytes, page_size);
    fs::write(pdb_path, bytes).expect("write export pdb");
}

fn assert_fix_proposed(backend: &BackendCommands, usb: &Path, fix_id: &str) {
    let preview = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: false,
        selected_fix_ids: vec![],
    });
    assert!(preview.ok, "repair preview failed: {preview:?}");
    let data = preview.data.expect("preview data");
    assert!(
        data.proposed_fixes.iter().any(|f| f.id == fix_id),
        "expected fix '{fix_id}' to be proposed: {:#?}",
        data.proposed_fixes
    );
}

fn assert_fix_not_proposed(backend: &BackendCommands, usb: &Path, fix_id: &str) {
    let preview = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: false,
        selected_fix_ids: vec![],
    });
    assert!(preview.ok, "repair preview failed: {preview:?}");
    let data = preview.data.expect("preview data");
    assert!(
        !data.proposed_fixes.iter().any(|f| f.id == fix_id),
        "expected fix '{fix_id}' to no longer be proposed: {:#?}",
        data.proposed_fixes
    );
}

fn apply_fix_and_assert_applied(backend: &BackendCommands, usb: &Path, fix_id: &str) {
    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec![fix_id.to_string()],
    });
    assert!(repair.ok, "repair apply failed: {repair:?}");
    let data = repair.data.expect("repair data");
    assert!(
        data.applied_fixes.iter().any(|m| !m.is_empty()) && !data.applied_fixes.is_empty(),
        "expected '{fix_id}' to be applied: {data:#?}"
    );
    assert!(
        data.failed_fixes.is_empty(),
        "unexpected repair failures: {:#?}",
        data.failed_fixes
    );
}

#[test]
fn repair_pdb_sentinel_u5_detects_and_fixes_via_orchestrator() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    read_write_pdb(&pdb_path, |bytes, page_size| {
        let page = find_pdb_data_page(bytes, page_size, 0).expect("tt=0 data page");
        let off = page * page_size;
        bytes[off + 0x20..off + 0x22].copy_from_slice(&0x1FFFu16.to_le_bytes());
        bytes[off + 0x22..off + 0x24].copy_from_slice(&0x1FFFu16.to_le_bytes());
    });

    assert_fix_proposed(&backend, &usb, PDB_SENTINEL_U5_FIX_ID);
    apply_fix_and_assert_applied(&backend, &usb, PDB_SENTINEL_U5_FIX_ID);
    assert_no_pdb_structural_repairs(&backend, &usb);
}

#[test]
fn repair_pdb_duplicate_playlist_entries_detects_and_fixes_via_orchestrator() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    read_write_pdb(&pdb_path, |bytes, page_size| {
        let page = find_pdb_data_page(bytes, page_size, 8).expect("tt=8 playlist_entries page");
        let off = page * page_size;
        let page_end = off + page_size;
        let heap_start = off + 0x28;

        let packed = u32::from(bytes[off + 0x18])
            | (u32::from(bytes[off + 0x19]) << 8)
            | (u32::from(bytes[off + 0x1a]) << 16);
        let n = (packed & 0x1FFF) as usize; // active row count
        let used_s = u16::from_le_bytes(bytes[off + 0x1e..off + 0x20].try_into().unwrap()) as usize;

        // Resolve row 0's real heap offset via its offset-table entry (don't
        // assume it's 0) -- mirrors the read loop in
        // remove_duplicate_playlist_entries_inplace exactly.
        let rowpf_off = page_end - 4;
        let row0_heap_off =
            u16::from_le_bytes(bytes[rowpf_off - 2..rowpf_off].try_into().unwrap()) as usize;
        let row0 = heap_start + row0_heap_off;
        let orig_entry_index = u32::from_le_bytes(bytes[row0..row0 + 4].try_into().unwrap());
        let track_id = u32::from_le_bytes(bytes[row0 + 4..row0 + 8].try_into().unwrap());
        let playlist_id = u32::from_le_bytes(bytes[row0 + 8..row0 + 12].try_into().unwrap());

        // Append a duplicate of row 0 under a higher entry_index -- same
        // shape as the real-world stale-tail defect fixed in 0.1.27.
        let new_row = heap_start + used_s;
        bytes[new_row..new_row + 4].copy_from_slice(&(orig_entry_index + 1000).to_le_bytes());
        bytes[new_row + 4..new_row + 8].copy_from_slice(&track_id.to_le_bytes());
        bytes[new_row + 8..new_row + 12].copy_from_slice(&playlist_id.to_le_bytes());

        let new_n = (n + 1) as u32;
        let new_packed = (new_n & 0x1FFF) | ((new_n & 0x7FF) << 13);
        bytes[off + 0x18] = (new_packed & 0xFF) as u8;
        bytes[off + 0x19] = ((new_packed >> 8) & 0xFF) as u8;
        bytes[off + 0x1a] = ((new_packed >> 16) & 0xFF) as u8;
        bytes[off + 0x1e..off + 0x20].copy_from_slice(&((used_s + 12) as u16).to_le_bytes());

        let mut rowpf = u16::from_le_bytes(bytes[rowpf_off..rowpf_off + 2].try_into().unwrap());
        rowpf |= 1u16 << n;
        bytes[rowpf_off..rowpf_off + 2].copy_from_slice(&rowpf.to_le_bytes());

        let new_off_pos = rowpf_off - 2 * (n + 1);
        bytes[new_off_pos..new_off_pos + 2].copy_from_slice(&(used_s as u16).to_le_bytes());
    });

    assert_fix_proposed(&backend, &usb, PDB_DUPLICATE_PLAYLIST_ENTRIES_FIX_ID);
    apply_fix_and_assert_applied(&backend, &usb, PDB_DUPLICATE_PLAYLIST_ENTRIES_FIX_ID);
    assert_fix_not_proposed(&backend, &usb, PDB_DUPLICATE_PLAYLIST_ENTRIES_FIX_ID);
}

#[test]
fn repair_pdb_wrong_page_flags_detects_and_fixes_via_orchestrator() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    read_write_pdb(&pdb_path, |bytes, page_size| {
        let page = find_pdb_data_page(bytes, page_size, 0).expect("tt=0 data page");
        bytes[page * page_size + 0x1b] = 0x11; // neither 0x24 nor 0x34
    });

    assert_fix_proposed(&backend, &usb, PDB_WRONG_PAGE_FLAGS_FIX_ID);
    apply_fix_and_assert_applied(&backend, &usb, PDB_WRONG_PAGE_FLAGS_FIX_ID);
    // Not `assert_no_pdb_structural_repairs`: the correction target for tt=0
    // is 0x34 (active), which legitimately differs from this fixture's
    // originally-exported sealed (0x24) page and exposes an unrelated,
    // genuinely separate stale-sentinel-B-tree condition. Only assert the
    // fix under test actually resolved.
    assert_fix_not_proposed(&backend, &usb, PDB_WRONG_PAGE_FLAGS_FIX_ID);
}

#[test]
fn repair_pdb_zero_tranrf_detects_and_fixes_via_orchestrator() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    read_write_pdb(&pdb_path, |bytes, page_size| {
        let page = find_pdb_data_page(bytes, page_size, 0).expect("tt=0 data page");
        let off = page * page_size;
        let rowpf_off = off + page_size - 4;
        let tranrf_off = off + page_size - 2;
        let rowpf = u16::from_le_bytes(bytes[rowpf_off..rowpf_off + 2].try_into().unwrap());
        assert_ne!(
            rowpf, 0,
            "expected the real track row to be active (rowpf != 0)"
        );
        bytes[tranrf_off..tranrf_off + 2].copy_from_slice(&0u16.to_le_bytes());
    });

    assert_fix_proposed(&backend, &usb, PDB_ZERO_TRANRF_FIX_ID);
    apply_fix_and_assert_applied(&backend, &usb, PDB_ZERO_TRANRF_FIX_ID);
    assert_no_pdb_structural_repairs(&backend, &usb);
}

#[test]
fn repair_pdb_wrong_history_page_shape_detects_and_fixes_via_orchestrator() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    read_write_pdb(&pdb_path, |bytes, page_size| {
        let page = find_pdb_data_page(bytes, page_size, 17).expect("tt=17 data page");
        let off = page * page_size;
        let nrs = bytes[off + 0x18];
        assert!(nrs > 1, "need nrs > 1 to construct the old-bug pattern");
        bytes[off + 0x20..off + 0x22].copy_from_slice(&1u16.to_le_bytes());
        bytes[off + 0x22..off + 0x24].copy_from_slice(&(nrs as u16 - 1).to_le_bytes());
    });

    assert_fix_proposed(&backend, &usb, PDB_WRONG_HISTORY_SHAPE_FIX_ID);
    apply_fix_and_assert_applied(&backend, &usb, PDB_WRONG_HISTORY_SHAPE_FIX_ID);
    assert_no_pdb_structural_repairs(&backend, &usb);
}

#[test]
fn repair_pdb_wrong_track_u5_detects_and_fixes_via_orchestrator() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    read_write_pdb(&pdb_path, |bytes, page_size| {
        let page = find_pdb_data_page(bytes, page_size, 0).expect("tt=0 data page");
        let off = page * page_size;
        bytes[off + 0x1b] = 0x34; // active
        bytes[off + 0x20..off + 0x22].copy_from_slice(&99u16.to_le_bytes());
        bytes[off + 0x22..off + 0x24].copy_from_slice(&99u16.to_le_bytes());
    });

    assert_fix_proposed(&backend, &usb, PDB_WRONG_TRACK_U5_FIX_ID);
    apply_fix_and_assert_applied(&backend, &usb, PDB_WRONG_TRACK_U5_FIX_ID);
    // Same rationale as the wrong_page_flags test above: forcing this page
    // active (0x34) exposes an unrelated stale-sentinel-B-tree condition.
    assert_fix_not_proposed(&backend, &usb, PDB_WRONG_TRACK_U5_FIX_ID);
}

#[test]
fn repair_pdb_wrong_playlist_tree_shape_detects_and_fixes_via_orchestrator() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    read_write_pdb(&pdb_path, |bytes, page_size| {
        let page = find_pdb_data_page(bytes, page_size, 7).expect("tt=7 data page");
        let off = page * page_size;
        bytes[off + 0x22..off + 0x24].copy_from_slice(&5u16.to_le_bytes()); // num_rl != 0
    });

    assert_fix_proposed(&backend, &usb, PDB_WRONG_PLAYLIST_TREE_SHAPE_FIX_ID);
    apply_fix_and_assert_applied(&backend, &usb, PDB_WRONG_PLAYLIST_TREE_SHAPE_FIX_ID);
    assert_no_pdb_structural_repairs(&backend, &usb);
}

#[test]
fn repair_pdb_fix_not_in_selected_ids_is_skipped_not_applied() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    read_write_pdb(&pdb_path, |bytes, page_size| {
        let page = find_pdb_data_page(bytes, page_size, 0).expect("tt=0 data page");
        bytes[page * page_size + 0x1b] = 0x11;
    });
    assert_fix_proposed(&backend, &usb, PDB_WRONG_PAGE_FLAGS_FIX_ID);

    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });
    assert!(repair.ok, "repair apply failed: {repair:?}");
    let data = repair.data.expect("repair data");
    assert!(
        data.skipped_fixes
            .iter()
            .any(|m| m.contains("Repair PDB Data Page Flags") && m.contains("not selected")),
        "expected the unselected fix to be reported as skipped: {:#?}",
        data.skipped_fixes
    );
    assert!(
        !data.applied_fixes.iter().any(|m| m.contains("Page Flags")),
        "unselected fix must not have been applied: {:#?}",
        data.applied_fixes
    );

    // The corruption must still be present since the fix wasn't applied.
    assert_fix_proposed(&backend, &usb, PDB_WRONG_PAGE_FLAGS_FIX_ID);
}

// ── fix_empty_analysis_files ──────────────────────────────────────────────

fn find_exported_anlz_dat(usb: &Path) -> PathBuf {
    walkdir::WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join("USBANLZ"))
        .into_iter()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name() == "ANLZ0000.DAT")
        .map(|e| e.path().to_path_buf())
        .expect("expected an exported ANLZ0000.DAT bundle")
}

#[test]
fn repair_fix_empty_analysis_files_regenerates_from_source_audio() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();

    let dat_path = find_exported_anlz_dat(&usb);
    let ext_path = dat_path.with_extension("EXT");
    let twoex_path = dat_path.with_extension("2EX");
    assert!(
        fs::metadata(&dat_path).expect("dat metadata").len() > 0,
        "fixture bundle should start non-empty"
    );

    // Corrupt: truncate the whole bundle to empty files.
    fs::write(&dat_path, []).expect("truncate DAT");
    fs::write(&ext_path, []).expect("truncate EXT");
    fs::write(&twoex_path, []).expect("truncate 2EX");

    assert_fix_proposed(&backend, &usb, "fix_empty_analysis_files");

    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["fix_empty_analysis_files".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let data = repair.data.expect("repair data");
    // scan_anlz_warnings reports each bundle member (.DAT/.EXT/.2EX)
    // separately, so a single regenerated bundle counts as 3 "fixed".
    assert!(
        data.applied_fixes
            .iter()
            .any(|m| m.contains("Fix Empty Analysis Files") && m.contains("fixed 3")),
        "expected the empty bundle to be reported fixed: {:#?}",
        data.applied_fixes
    );

    let regenerated_len = fs::metadata(&dat_path)
        .expect("dat metadata after repair")
        .len();
    assert!(
        regenerated_len > 0,
        "DAT bundle should be regenerated with real waveform content"
    );
    assert_fix_not_proposed(&backend, &usb, "fix_empty_analysis_files");
}

#[test]
fn repair_fix_empty_analysis_files_skips_unmapped_stray_bundle() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();

    // An empty analysis bundle with no corresponding PDB/eDB track mapping
    // (a stray leftover, not referenced by anything) can't be regenerated —
    // there's no source audio to derive it from.
    let stray_dir = usb
        .join(USB_VENDOR_ROOT_DIR)
        .join("USBANLZ")
        .join("P0AA")
        .join("DEADBEEF");
    fs::create_dir_all(&stray_dir).expect("create stray analysis dir");
    fs::write(stray_dir.join("ANLZ0000.DAT"), []).expect("write empty stray DAT");
    fs::write(stray_dir.join("ANLZ0000.EXT"), []).expect("write empty stray EXT");
    fs::write(stray_dir.join("ANLZ0000.2EX"), []).expect("write empty stray 2EX");

    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["fix_empty_analysis_files".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let data = repair.data.expect("repair data");
    // Nothing was fixed (no mapping to regenerate from), so this lands in
    // `skipped_fixes` with the generic "nothing to apply" message rather
    // than `applied_fixes` — see `repair_usb_diagnostics_with_progress`'s
    // fixed/failed/else dispatch for "fix_empty_analysis_files".
    assert!(
        data.skipped_fixes
            .iter()
            .any(|m| m.contains("Fix Empty Analysis Files") && m.contains("nothing to apply")),
        "expected the stray bundle to be reported as skipped: {:#?}",
        data.skipped_fixes
    );
    assert!(
        data.warnings
            .iter()
            .any(|w| w.message.contains("source audio mapping not found")),
        "expected a skip-reason warning for the unmapped stray bundle: {:#?}",
        data.warnings
    );
}

// ── fix_bpm_key_mismatch ────────────────────────────────────────────────────

fn bpm_key_check_status(backend: &BackendCommands, usb: &Path) -> backend::models::DiagStatus {
    let diagnostics = backend.run_usb_diagnostics(RunUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(diagnostics.ok, "diagnostics failed: {diagnostics:?}");
    diagnostics
        .data
        .expect("diagnostics data")
        .analysis_integrity
        .checks
        .into_iter()
        .find(|c| c.label == "BPM/key consistency")
        .expect("BPM/key consistency check")
        .status
}

#[test]
fn repair_fix_bpm_key_mismatch_aligns_anlz_and_edb_to_pdb_in_one_pass() {
    use backend::service::anlz::{
        build_anlz_dat_file, build_anlz_ext_file, read_beatgrid_tempo_from_anlz,
    };

    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    // A normal export must never look mismatched.
    assert_fix_not_proposed(&backend, &usb, "fix_bpm_key_mismatch");

    let parsed = parse_pdb(&vendor_db_dir(&usb).join("export.pdb")).expect("parse pdb");
    let track = parsed
        .tracks
        .iter()
        .find(|t| t.tempo_x100 > 0)
        .expect("a PDB track with a tempo");
    let pdb_tempo = track.tempo_x100;
    let wrong_bpm = if pdb_tempo == 9_000 { 91.0 } else { 90.0 };
    let pdb_key = parsed
        .keys
        .get(&track.key_id)
        .cloned()
        .expect("fixture track has a PDB key");

    // Replace the fixture's placeholder bundle with a real one whose beat
    // grid is at a wrong tempo, and skew the eDB BPM and key.
    let dat_path = find_exported_anlz_dat(&usb);
    let waveform = flat_waveform(400, 128);
    let track_path = track.track_file_path.as_str();
    fs::write(
        &dat_path,
        build_anlz_dat_file(&waveform, track_path, Some(wrong_bpm), 200_000, None, &[]),
    )
    .expect("write DAT");
    fs::write(
        dat_path.with_extension("EXT"),
        build_anlz_ext_file(&waveform, track_path, Some(wrong_bpm), 200_000, None, &[]),
    )
    .expect("write EXT");
    // The automatic diagnosis is database-only: a beat grid alone never
    // trips it (it doesn't read the analysis files) ...
    assert!(matches!(
        bpm_key_check_status(&backend, &usb),
        backend::models::DiagStatus::Pass
    ));
    let edb_path = vendor_db_dir(&usb).join("exportLibrary.db");
    open_edb(&edb_path)
        .execute(
            "UPDATE content SET bpmx100 = ?1 WHERE path = ?2",
            rusqlite::params![12_345, track.track_file_path],
        )
        .expect("skew eDB bpm");
    open_edb(&edb_path)
        .execute(
            "UPDATE content SET key_id = NULL WHERE path = ?1",
            [&track.track_file_path],
        )
        .expect("skew eDB key");
    // ... but a PDB/eDB disagreement does.
    assert!(matches!(
        bpm_key_check_status(&backend, &usb),
        backend::models::DiagStatus::Warn
    ));

    assert_fix_proposed(&backend, &usb, "fix_bpm_key_mismatch");
    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["fix_bpm_key_mismatch".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let data = repair.data.expect("repair data");
    assert!(
        data.applied_fixes
            .iter()
            .any(|m| m.contains("Fix BPM/Key Mismatch") && m.contains("updated 1 eDB row")),
        "expected the mismatch to be fixed: {:#?}",
        data.applied_fixes
    );

    for path in [dat_path.clone(), dat_path.with_extension("EXT")] {
        let tempo = read_beatgrid_tempo_from_anlz(&fs::read(&path).expect("read bundle"));
        assert_eq!(tempo.map(u32::from), Some(pdb_tempo), "{}", path.display());
    }
    let (edb_bpm, edb_key): (i64, String) = open_edb(&edb_path)
        .query_row(
            r#"SELECT c.bpmx100, k.name FROM content c JOIN "key" k ON k.key_id = c.key_id
               WHERE c.path = ?1"#,
            [&track.track_file_path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("eDB bpm/key");
    assert_eq!(edb_bpm, i64::from(pdb_tempo));
    assert!(
        edb_key.eq_ignore_ascii_case(&pdb_key),
        "eDB key {edb_key} != PDB key {pdb_key}"
    );
    // One pass is enough: nothing left to propose.
    assert_fix_not_proposed(&backend, &usb, "fix_bpm_key_mismatch");
}

// ── fix_beat_grid_header ────────────────────────────────────────────────────

#[test]
fn repair_fix_beat_grid_rewrites_pre_0_3_7_grids_once() {
    use backend::service::anlz::{build_anlz_dat_file, build_anlz_ext_file};

    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let parsed = parse_pdb(&vendor_db_dir(&usb).join("export.pdb")).expect("parse pdb");
    let track_path = parsed.tracks[0].track_file_path.clone();

    let dat_path = find_exported_anlz_dat(&usb);
    let ext_path = dat_path.with_extension("EXT");
    let waveform = flat_waveform(400, 128);
    let dat = build_anlz_dat_file(&waveform, &track_path, Some(120.0), 200_000, None, &[]);
    let ext = build_anlz_ext_file(&waveform, &track_path, Some(120.0), 200_000, None, &[]);

    // The same bundle as a pre-0.3.7 app wrote it: the PQTZ value two bytes
    // early, and a PQT2 without its checksum or sub-millisecond body.
    let pqtz_at = dat.windows(4).position(|w| w == b"PQTZ").expect("PQTZ");
    let mut old_dat = dat.clone();
    old_dat[pqtz_at + 12..pqtz_at + 20].copy_from_slice(&[0, 0, 0, 8, 0, 0, 0, 0]);
    let pqt2_at = ext.windows(4).position(|w| w == b"PQT2").expect("PQT2");
    let mut old_ext = ext.clone();
    old_ext[pqt2_at + 44..pqt2_at + 48].fill(0);
    let tag_len = u32::from_be_bytes(old_ext[pqt2_at + 8..pqt2_at + 12].try_into().unwrap());
    for (i, at) in (pqt2_at + 56..pqt2_at + tag_len as usize)
        .step_by(2)
        .enumerate()
    {
        old_ext[at..at + 2].copy_from_slice(&[(i % 4) as u8, 0]);
    }
    fs::write(&dat_path, &old_dat).expect("write DAT");
    fs::write(&ext_path, &old_ext).expect("write EXT");

    assert_fix_proposed(&backend, &usb, "fix_beat_grid_header");
    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["fix_beat_grid_header".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let data = repair.data.expect("repair data");
    assert!(
        data.applied_fixes
            .iter()
            .any(|m| m == "Fix Beat Grid: fixed 1 bundle(s)"),
        "expected the grid to be rewritten: {:#?} {:#?}",
        data.applied_fixes,
        data.warnings
    );
    // The outcome is in the Event Log too, not only in the repair dialog.
    assert!(
        data.warnings.iter().any(|w| w.code == "usb.repair.applied"
            && w.level == "info"
            && w.message == "repair applied: Fix Beat Grid: fixed 1 bundle(s)"),
        "expected the applied fix in the log: {:#?}",
        data.warnings
    );
    assert_eq!(fs::read(&dat_path).expect("read DAT"), dat);
    assert_eq!(fs::read(&ext_path).expect("read EXT"), ext);
    assert_fix_not_proposed(&backend, &usb, "fix_beat_grid_header");
}

fn analysis_check(backend: &BackendCommands, usb: &Path, label: &str) -> Option<(String, String)> {
    let diagnostics = backend.run_usb_diagnostics(RunUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(diagnostics.ok, "diagnostics failed: {diagnostics:?}");
    diagnostics
        .data
        .expect("diagnostics data")
        .analysis_integrity
        .checks
        .into_iter()
        .find(|c| c.label == label)
        .map(|c| (format!("{:?}", c.status), c.detail))
}

#[test]
fn diagnostics_flag_outdated_beat_grids_only_on_tracks_last_exported_by_older_versions() {
    use backend::service::anlz::build_anlz_dat_file;

    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let parsed = parse_pdb(&vendor_db_dir(&usb).join("export.pdb")).expect("parse pdb");
    let track_path = parsed.tracks[0].track_file_path.clone();
    let dat_path = find_exported_anlz_dat(&usb);
    let dat = build_anlz_dat_file(
        &flat_waveform(400, 128),
        &track_path,
        Some(120.0),
        200_000,
        None,
        &[],
    );
    let pqtz_at = dat.windows(4).position(|w| w == b"PQTZ").expect("PQTZ");
    let mut old_dat = dat.clone();
    old_dat[pqtz_at + 12..pqtz_at + 20].copy_from_slice(&[0, 0, 0, 8, 0, 0, 0, 0]);
    fs::write(&dat_path, &old_dat).expect("write DAT");

    // Exported by this version: the log says so, and the DAT isn't even read.
    let log_path = usb
        .join(".dj-usb-tkit")
        .join("dj_usb_tkit_export_log.v1.json");
    let log = fs::read_to_string(&log_path).expect("export log");
    assert!(log.contains("\"appVersion\""), "{log}");
    let (status, detail) = analysis_check(&backend, &usb, "Beat grid format").expect("check");
    assert_eq!(status, "Pass");
    assert!(detail.contains("current format"), "{detail}");

    // The same export as a pre-0.3.7 app logged it: no appVersion.
    let mut value: serde_json::Value = serde_json::from_str(&log).expect("parse log");
    for record in value["records"].as_array_mut().expect("records") {
        record.as_object_mut().expect("record").remove("appVersion");
    }
    fs::write(&log_path, serde_json::to_string(&value).unwrap()).expect("write log");
    let (status, detail) = analysis_check(&backend, &usb, "Beat grid format").expect("check");
    assert_eq!(status, "Warn");
    assert!(
        detail.starts_with("1 track(s) have a beat grid in an outdated format")
            && detail.contains(&track_path),
        "{detail}"
    );

    // Once fixed, the old export record alone isn't enough to flag it.
    fs::write(&dat_path, &dat).expect("write DAT");
    let (status, _) = analysis_check(&backend, &usb, "Beat grid format").expect("check");
    assert_eq!(status, "Pass");

    // No export log: nothing this app exported, still a line.
    fs::remove_file(&log_path).expect("remove log");
    let (status, detail) = analysis_check(&backend, &usb, "Beat grid format").expect("check");
    assert_eq!(status, "Pass");
    assert_eq!(detail, "no tracks on this USB were exported by this app");

    // An unreadable log: every track's .DAT is checked instead.
    fs::write(&log_path, "not json").expect("write log");
    let (status, _) = analysis_check(&backend, &usb, "Beat grid format").expect("check");
    assert_eq!(status, "Pass");
    fs::write(&dat_path, &old_dat).expect("write DAT");
    let (status, detail) = analysis_check(&backend, &usb, "Beat grid format").expect("check");
    assert_eq!(status, "Warn");
    assert!(detail.contains(&track_path), "{detail}");
}

// ── add_missing_mp3_seek_data / add_missing_flac_seek_data ───────────────────────────────────────────────────

#[test]
fn repair_add_missing_mp3_seek_data_fills_an_empty_pvbr_once() {
    use backend::service::anlz::{build_anlz_dat_file, build_anlz_ext_file};

    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let parsed = parse_pdb(&vendor_db_dir(&usb).join("export.pdb")).expect("parse pdb");
    let track_path = parsed.tracks[0].track_file_path.clone();
    assert!(track_path.ends_with(".mp3"), "{track_path}");

    // A bundle generated by the app: PVBR present but empty.
    let dat_path = find_exported_anlz_dat(&usb);
    let waveform = flat_waveform(400, 128);
    let dat = build_anlz_dat_file(&waveform, &track_path, Some(120.0), 200_000, None, &[]);
    let ext = build_anlz_ext_file(&waveform, &track_path, Some(120.0), 200_000, None, &[]);
    fs::write(&dat_path, &dat).expect("write DAT");
    fs::write(dat_path.with_extension("EXT"), &ext).expect("write EXT");

    assert_fix_proposed(&backend, &usb, "add_missing_mp3_seek_data");
    // The stick has no FLAC, so the FLAC fix has nothing to offer.
    assert_fix_not_proposed(&backend, &usb, "add_missing_flac_seek_data");
    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["add_missing_mp3_seek_data".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let data = repair.data.expect("repair data");
    assert!(
        data.applied_fixes
            .iter()
            .any(|m| m == "Add Missing MP3 Seek Data: fixed 1, skipped 0"),
        "expected the seek data to be added: {:#?} {:#?}",
        data.applied_fixes,
        data.warnings
    );

    let patched = fs::read(&dat_path).expect("read DAT");
    assert_eq!(patched.len(), dat.len(), "PVBR is filled in place");
    let pvbr_at = patched
        .windows(4)
        .position(|w| w == b"PVBR")
        .expect("PVBR chunk");
    let total = u32::from_be_bytes(
        patched[pvbr_at + 16 + 1600..pvbr_at + 16 + 1604]
            .try_into()
            .unwrap(),
    );
    assert!(total > 0 && total % 1152 == 0, "PVBR total {total}");
    let mut without_pvbr = patched.clone();
    without_pvbr[pvbr_at + 16..pvbr_at + 16 + 1604].fill(0);
    assert_eq!(without_pvbr, dat, "only the PVBR payload changed");
    assert_eq!(
        fs::read(dat_path.with_extension("EXT")).expect("read EXT"),
        ext,
        "an MP3's .EXT is left alone"
    );
    // One pass is enough: nothing left to propose.
    assert_fix_not_proposed(&backend, &usb, "add_missing_mp3_seek_data");
}

// ── remove_missing_audio_references ───────────────────────────────────────

fn find_contents_audio_file(usb: &Path) -> PathBuf {
    walkdir::WalkDir::new(usb.join("Contents"))
        .into_iter()
        .filter_map(|e| e.ok())
        .find(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .expect("expected exported audio file under Contents")
}

#[test]
fn repair_remove_missing_audio_references_deletes_dangling_track_refs() {
    // Two tracks are needed: deleting the *only* audio file on the USB would
    // empty Contents/ entirely, which the detector treats as a DB-only
    // snapshot (can't tell what's "missing" vs. never-copied) rather than
    // evidence of a dangling reference — see `any_audio_on_usb` in
    // `repair_usb_diagnostics_with_progress`. Keeping a second track's audio
    // file in place is what makes the deleted one look genuinely missing.
    let (_root, backend, usb, _target_playlist, _control_playlist) =
        setup_two_playlist_strict_parity_fixture();

    let audio_files: Vec<PathBuf> = walkdir::WalkDir::new(usb.join("Contents"))
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .collect();
    assert_eq!(audio_files.len(), 2, "expected two exported audio files");
    let missing_file = &audio_files[0];

    let track_id = {
        let conn = open_edb(&vendor_db_dir(&usb).join("exportLibrary.db"));
        let file_name = missing_file
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        conn.query_row(
            "SELECT content_id FROM content WHERE path LIKE ?1",
            [format!("%{file_name}")],
            |r| r.get::<_, i64>(0),
        )
        .expect("content row for the file about to go missing")
    };

    fs::remove_file(missing_file).expect("delete one referenced audio file");

    let preview = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: false,
        selected_fix_ids: vec![],
    });
    assert!(preview.ok, "preview failed: {preview:?}");
    let preview_data = preview.data.expect("preview data");
    assert!(
        preview_data
            .detected_issues
            .iter()
            .any(|i| i.contains("missing audio")),
        "expected missing-audio issue detected: {:#?}",
        preview_data.detected_issues
    );
    let proposal = preview_data
        .proposed_fixes
        .iter()
        .find(|f| f.id == "remove_missing_audio_references")
        .expect("remove_missing_audio_references should be proposed");
    assert!(
        proposal.supported,
        "fix should be auto-supported: {proposal:?}"
    );

    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["remove_missing_audio_references".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let data = repair.data.expect("repair data");
    assert!(
        data.applied_fixes
            .iter()
            .any(|m| m.contains("Remove Missing Audio References")),
        "expected the fix to be applied: {:#?}",
        data.applied_fixes
    );

    let conn = open_edb(&vendor_db_dir(&usb).join("exportLibrary.db"));
    let remaining_content: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM content WHERE content_id = ?1",
            [track_id],
            |r| r.get(0),
        )
        .expect("count content");
    assert_eq!(remaining_content, 0, "content row should be removed");
    let remaining_links: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM playlist_content WHERE content_id = ?1",
            [track_id],
            |r| r.get(0),
        )
        .expect("count playlist_content");
    assert_eq!(
        remaining_links, 0,
        "playlist_content link should be removed"
    );

    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    let parsed = parse_pdb(&pdb_path).expect("parse pdb after repair");
    assert!(
        parsed
            .playlist_entries
            .iter()
            .all(|e| e.track_id != track_id as u32),
        "PDB playlist entry for the missing track should be removed: {:?}",
        parsed.playlist_entries
    );
}

#[test]
fn repair_relinks_a_renamed_audio_file_instead_of_removing_it() {
    let (_root, backend, usb, playlist_name) = setup_clean_strict_parity_fixture();

    // The exported file was renamed and moved: its DB reference is now
    // missing and the file itself unindexed. They are the same track.
    let original = find_contents_audio_file(&usb);
    let moved_dir = usb.join("Contents").join("Renamed Folder");
    fs::create_dir_all(&moved_dir).expect("create moved dir");
    fs::rename(&original, moved_dir.join("Renamed Track.mp3")).expect("rename audio");
    let new_rel = "/Contents/Renamed Folder/Renamed Track.mp3";

    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    let edb_path = vendor_db_dir(&usb).join("exportLibrary.db");
    let track_id_before = parse_pdb(&pdb_path).expect("parse pdb").tracks[0].id;
    let content_id_before: i64 = open_edb(&edb_path)
        .query_row("SELECT content_id FROM content LIMIT 1", [], |r| r.get(0))
        .expect("content row");

    let preview = backend
        .repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
            apply: false,
            selected_fix_ids: vec![],
        })
        .data
        .expect("preview data");
    let ids: Vec<&str> = preview
        .proposed_fixes
        .iter()
        .map(|f| f.id.as_str())
        .collect();
    assert!(ids.contains(&"relink_moved_audio"), "{ids:?}");
    assert!(!ids.contains(&"add_unindexed_audio_playlist"), "{ids:?}");
    assert!(!ids.contains(&"remove_missing_audio_references"), "{ids:?}");

    // Default selection: every supported fix.
    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec![],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let data = repair.data.expect("repair data");
    assert!(
        data.applied_fixes
            .iter()
            .any(|m| m == "Relink Moved Audio Files: relinked 1 track(s)"),
        "applied {:#?} failed {:#?}",
        data.applied_fixes,
        data.failed_fixes
    );
    assert!(data.failed_fixes.is_empty(), "{:#?}", data.failed_fixes);

    // Same PDB row and eDB row, new path, still in the original playlist.
    let parsed = parse_pdb(&pdb_path).expect("parse pdb");
    let track = parsed
        .tracks
        .iter()
        .find(|t| t.id == track_id_before)
        .expect("track row kept");
    assert_eq!(track.track_file_path, new_rel);
    let playlist = parsed
        .playlist_tree
        .iter()
        .find(|p| p.name == playlist_name)
        .expect("original playlist");
    assert!(
        parsed
            .playlist_entries
            .iter()
            .any(|e| e.playlist_id == playlist.id && e.track_id == track_id_before),
        "track must stay in its playlist"
    );
    assert!(!parsed.playlist_tree.iter().any(|p| p.name == "Unindexed"));
    let edb_path_after: String = open_edb(&edb_path)
        .query_row(
            "SELECT path FROM content WHERE content_id = ?1",
            [content_id_before],
            |r| r.get(0),
        )
        .expect("content row kept");
    assert_eq!(edb_path_after, new_rel);

    // One pass: nothing left to relink, add or remove.
    let after = backend
        .repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
            apply: false,
            selected_fix_ids: vec![],
        })
        .data
        .expect("preview data");
    assert!(
        !after.proposed_fixes.iter().any(|f| matches!(
            f.id.as_str(),
            "relink_moved_audio"
                | "add_unindexed_audio_playlist"
                | "remove_missing_audio_references"
        )),
        "{:#?}",
        after.proposed_fixes
    );
}

#[test]
fn repair_removes_missing_and_adds_unrelated_unindexed_audio_in_one_pass() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();

    // The exported file is gone, and a different track (other size, tags
    // and name) sits unindexed on the USB: no pairing, so the reference is
    // removed and the new file added.
    fs::remove_file(find_contents_audio_file(&usb)).expect("delete referenced audio file");
    let stray_dir = usb
        .join("Contents")
        .join("Stray Artist")
        .join("Stray Album");
    fs::create_dir_all(&stray_dir).expect("create stray contents dir");
    copy_audio_fixture(
        &stray_dir,
        "embedded/track_embedded.mp3",
        "99 Unrelated.mp3",
    );

    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec![],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let data = repair.data.expect("repair data");
    assert!(data.failed_fixes.is_empty(), "{:#?}", data.failed_fixes);
    assert!(
        data.applied_fixes
            .iter()
            .any(|m| m.starts_with("Add Unindexed Audio to a Playlist: added 1 track(s)")),
        "{:#?}",
        data.applied_fixes
    );
    assert!(
        data.applied_fixes
            .iter()
            .any(|m| m.starts_with("Remove Missing Audio References: removed")),
        "{:#?}",
        data.applied_fixes
    );
    assert!(
        !data.applied_fixes.iter().any(|m| m.starts_with("Relink")),
        "{:#?}",
        data.applied_fixes
    );

    let after = backend
        .repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
            apply: false,
            selected_fix_ids: vec![],
        })
        .data
        .expect("preview data");
    assert!(
        !after.proposed_fixes.iter().any(|f| matches!(
            f.id.as_str(),
            "relink_moved_audio"
                | "add_unindexed_audio_playlist"
                | "remove_missing_audio_references"
        )),
        "{:#?}",
        after.proposed_fixes
    );
}

// ── Remaining repair_usb_diagnostics_with_progress orchestrator branches ──

#[test]
fn repair_detects_unindexed_audio_under_contents() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();

    let stray_dir = usb
        .join("Contents")
        .join("Stray Artist")
        .join("Stray Album");
    fs::create_dir_all(&stray_dir).expect("create stray contents dir");
    copy_audio_fixture(
        &stray_dir,
        "noart/track_no_art.mp3",
        "99 Unindexed Stray.mp3",
    );

    let preview = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: false,
        selected_fix_ids: vec![],
    });
    assert!(preview.ok, "preview failed: {preview:?}");
    let data = preview.data.expect("preview data");

    assert!(
        data.detected_issues
            .iter()
            .any(|i| i.contains("missing from the canonical-path indexed set")),
        "expected unindexed-audio issue detected: {:#?}",
        data.detected_issues
    );
    assert!(
        data.proposed_fixes.iter().any(|f| {
            f.id == "add_unindexed_audio_playlist"
                && f.supported
                && !f.destructive
                && f.description.contains("1 audio file(s)")
                && f.description.contains("\"Unindexed\"")
        }),
        "expected the unindexed-audio playlist fix: {:#?}",
        data.proposed_fixes
    );
}

#[test]
fn repair_adds_unindexed_audio_to_usb_playlist_in_one_pass() {
    let (_root, backend, usb, playlist_name) = setup_clean_strict_parity_fixture();

    let stray_dir = usb
        .join("Contents")
        .join("Stray Artist")
        .join("Stray Album");
    fs::create_dir_all(&stray_dir).expect("create stray contents dir");
    copy_audio_fixture(
        &stray_dir,
        "noart/track_no_art.mp3",
        "99 Unindexed Stray.mp3",
    );

    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["add_unindexed_audio_playlist".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let data = repair.data.expect("repair data");
    assert!(
        data.applied_fixes
            .iter()
            .any(|m| m.contains("added 1 track(s)")
                && m.contains("\"Unindexed\"")
                && m.contains("1 without analysis files")),
        "expected the fix to report the added track: applied {:#?} failed {:#?}",
        data.applied_fixes,
        data.failed_fixes
    );
    assert!(data.failed_fixes.is_empty(), "{:#?}", data.failed_fixes);

    // Both databases now carry the playlist with the stray file; the
    // existing playlist is untouched.
    let conn = open_edb(&vendor_db_dir(&usb).join("exportLibrary.db"));
    let edb_paths: Vec<String> = conn
        .prepare(
            "SELECT c.path FROM playlist p
             JOIN playlist_content pc ON pc.playlist_id = p.playlist_id
             JOIN content c ON c.content_id = pc.content_id
             WHERE p.name = 'Unindexed'",
        )
        .expect("prepare")
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert_eq!(
        edb_paths,
        vec!["/Contents/Stray Artist/Stray Album/99 Unindexed Stray.mp3".to_string()]
    );
    let parsed = parse_pdb(&vendor_db_dir(&usb).join("export.pdb")).expect("parse pdb");
    let names: Vec<&str> = parsed
        .playlist_tree
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert!(names.contains(&"Unindexed"), "PDB playlists: {names:?}");
    assert!(
        names.contains(&playlist_name.as_str()),
        "PDB playlists: {names:?}"
    );
    assert!(
        parsed
            .tracks
            .iter()
            .any(|t| t.track_file_path.ends_with("/99 Unindexed Stray.mp3")),
        "stray file should have a PDB track row"
    );

    // One pass is enough: a fresh preview no longer sees unindexed audio.
    let after = backend
        .repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
            apply: false,
            selected_fix_ids: vec![],
        })
        .data
        .expect("preview data");
    assert!(
        !after
            .proposed_fixes
            .iter()
            .any(|f| f.id == "add_unindexed_audio_playlist"),
        "unindexed audio still reported: {:#?}",
        after.detected_issues
    );
}

#[test]
fn repair_unindexed_audio_playlist_reuses_the_files_analysis_bundle() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();

    // Simulate an exported track whose DB rows were lost: an audio file
    // under a new path, with an analysis bundle (126.50 BPM grid) where
    // export puts the bundle for that path.
    let rel = "/Contents/Lost Artist/Lost Album/01 Lost Track.mp3";
    let lost_dir = usb.join("Contents/Lost Artist/Lost Album");
    fs::create_dir_all(&lost_dir).expect("create lost dir");
    copy_audio_fixture(&lost_dir, "noart/track_no_art.mp3", "01 Lost Track.mp3");
    let (dat, ext, twoex) = backend::service::anlz::canonical_analysis_bundle_paths(&usb, rel);
    fs::create_dir_all(dat.parent().unwrap()).expect("create anlz dir");
    backend::service::anlz::write_generated_anlz_bundle_with_first_beat(
        &backend::service::anlz::WaveformData::empty(),
        &backend::service::anlz::AnlzBundlePaths {
            dat_path: dat.clone(),
            ext_path: ext,
            twoex_path: twoex,
        },
        rel,
        Some(126.5),
        180_000,
        Some(250),
        &[],
    )
    .expect("write analysis bundle");
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");

    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["add_unindexed_audio_playlist".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let data = repair.data.expect("repair data");
    assert!(
        data.applied_fixes
            .iter()
            .any(|m| m.contains("added 1 track(s)") && !m.contains("without analysis")),
        "applied {:#?} failed {:#?}",
        data.applied_fixes,
        data.failed_fixes
    );

    let parsed = parse_pdb(&pdb_path).expect("parse pdb");
    let added = parsed
        .tracks
        .iter()
        .find(|t| t.track_file_path == rel)
        .expect("lost track indexed in PDB");
    let expected_anlz = format!("/{}", dat.strip_prefix(&usb).unwrap().to_string_lossy());
    assert_eq!(added.anlz_path, expected_anlz);
    assert_eq!(added.tempo_x100, 12650);
}

#[test]
fn repair_reports_missing_audio_scan_skipped_when_contents_absent() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();

    fs::remove_dir_all(usb.join("Contents")).expect("remove Contents directory");

    let preview = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: false,
        selected_fix_ids: vec![],
    });
    assert!(preview.ok, "preview failed: {preview:?}");
    let data = preview.data.expect("preview data");
    assert!(
        data.warnings.iter().any(|w| w
            .message
            .contains("missing-audio scan skipped: Contents directory is absent or empty")),
        "expected the DB-only-snapshot warning: {:#?}",
        data.warnings
    );
}

#[test]
fn repair_reports_parity_preview_unavailable_when_edb_unreadable() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();

    // Truncate the eDB to a few garbage bytes so it can't be opened as SQLite
    // at all — the parity report must fail gracefully with a warning rather
    // than aborting the whole repair preview.
    let edb_path = vendor_db_dir(&usb).join("exportLibrary.db");
    fs::write(&edb_path, b"not a sqlite database").expect("corrupt eDB");

    let preview = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: false,
        selected_fix_ids: vec![],
    });
    assert!(
        preview.ok,
        "repair preview must still succeed (graceful degradation): {preview:?}"
    );
    let data = preview.data.expect("preview data");
    assert!(
        data.warnings
            .iter()
            .any(|w| w.message.contains("parity preview unavailable")),
        "expected a parity-unavailable warning: {:#?}",
        data.warnings
    );
}

#[test]
fn repair_apply_reports_failure_when_pdb_is_read_only() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    read_write_pdb(&pdb_path, |bytes, page_size| {
        let page = find_pdb_data_page(bytes, page_size, 0).expect("tt=0 data page");
        bytes[page * page_size + 0x1b] = 0x11; // invalid flags
    });
    assert_fix_proposed(&backend, &usb, PDB_WRONG_PAGE_FLAGS_FIX_ID);

    let mut perms = fs::metadata(&pdb_path).expect("pdb metadata").permissions();
    perms.set_readonly(true);
    fs::set_permissions(&pdb_path, perms).expect("make pdb read-only");

    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec![PDB_WRONG_PAGE_FLAGS_FIX_ID.to_string()],
    });

    // Restore write permission before any assertion can early-return, so the
    // TempDir's own cleanup never trips over a read-only file.
    let mut perms = fs::metadata(&pdb_path).expect("pdb metadata").permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(&pdb_path, perms).expect("restore pdb write permission");

    assert!(
        repair.ok,
        "repair call itself should still return ok: {repair:?}"
    );
    let data = repair.data.expect("repair data");
    assert!(
        data.failed_fixes
            .iter()
            .any(|m| m.contains("Repair PDB Data Page Flags")),
        "expected the write failure to be reported in failed_fixes: {:#?}",
        data.failed_fixes
    );
}

// ── apply_strict_parity_upgrade edge cases ────────────────────────────────

#[test]
fn strict_repair_picks_up_pdb_only_playlist_with_no_edb_counterpart() {
    let (_root, backend, usb, target_playlist, control_playlist) =
        setup_two_playlist_strict_parity_fixture();

    // Delete the eDB-side playlist row (and its membership) for the control
    // playlist, leaving only the PDB playlist_tree/playlist_entries rows —
    // the `(None, Some(pdb))` arm in apply_strict_parity_upgrade's playlist
    // identity match.
    {
        let conn = open_edb(&vendor_db_dir(&usb).join("exportLibrary.db"));
        let playlist_id: i64 = conn
            .query_row(
                "SELECT playlist_id FROM playlist WHERE name = ?1",
                [&control_playlist],
                |r| r.get(0),
            )
            .expect("control playlist id");
        conn.execute(
            "DELETE FROM playlist_content WHERE playlist_id = ?1",
            [playlist_id],
        )
        .expect("delete control playlist_content");
        conn.execute("DELETE FROM playlist WHERE playlist_id = ?1", [playlist_id])
            .expect("delete control playlist row");
    }

    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });
    assert!(repair.ok, "repair failed: {repair:?}");
    let data = repair.data.expect("repair data");
    assert!(
        data.applied_fixes
            .iter()
            .any(|m| m.contains("Upgrade Export Data To Strict Parity")),
        "expected strict parity upgrade to apply: {:#?}",
        data.applied_fixes
    );

    // The PDB-only playlist must have been written back into eDB.
    let conn = open_edb(&vendor_db_dir(&usb).join("exportLibrary.db"));
    let restored: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM playlist WHERE name = ?1",
            [&control_playlist],
            |r| r.get(0),
        )
        .expect("count restored control playlist");
    assert_eq!(
        restored, 1,
        "PDB-only playlist should be propagated back into eDB"
    );

    let _ = target_playlist;
}

#[test]
fn strict_repair_skips_orphaned_playlist_tree_entry_without_erroring() {
    let (_root, backend, usb, _playlist_name) = setup_clean_strict_parity_fixture();
    let pdb_path = vendor_db_dir(&usb).join("export.pdb");

    // Change the exported track's own id in its PDB row, leaving the
    // playlist_entries row (which still references the old id) dangling —
    // `pdb_track_by_id.get(&entry.track_id)` must not find it and the entry
    // must be skipped rather than panicking.
    read_write_pdb(&pdb_path, |bytes, page_size| {
        let page = find_pdb_data_page(bytes, page_size, 0).expect("tt=0 data page");
        let id_off = page * page_size + 0x28 + 72; // heap_start + TOMBSTONE_ID_TABLES[tt=0] offset
        let old_id = u32::from_le_bytes(bytes[id_off..id_off + 4].try_into().unwrap());
        let new_id = old_id + 1000;
        bytes[id_off..id_off + 4].copy_from_slice(&new_id.to_le_bytes());
    });

    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });
    assert!(
        repair.ok,
        "repair must complete without erroring on a dangling playlist_tree entry: {repair:?}"
    );
    let data = repair.data.expect("repair data");
    assert!(
        data.failed_fixes.is_empty(),
        "orphaned entry must be skipped, not reported as a failure: {:#?}",
        data.failed_fixes
    );
}

#[test]
fn strict_repair_reports_failure_when_pdb_rewrite_cannot_be_written() {
    let (_root, backend, usb, playlist_name) = setup_clean_strict_parity_fixture();

    // Force a genuine strict-parity discrepancy so the upgrade actually has
    // a playlist to rewrite (a byte-clean export has nothing to do).
    {
        let conn = open_edb(&vendor_db_dir(&usb).join("exportLibrary.db"));
        let playlist_id: i64 = conn
            .query_row(
                "SELECT playlist_id FROM playlist WHERE name = ?1",
                [&playlist_name],
                |r| r.get(0),
            )
            .expect("playlist id");
        let content_id: i64 = conn
            .query_row(
                "SELECT content_id FROM playlist_content WHERE playlist_id = ?1 LIMIT 1",
                [playlist_id],
                |r| r.get(0),
            )
            .expect("content id");
        conn.execute(
            "DELETE FROM playlist_content WHERE playlist_id = ?1 AND content_id = ?2",
            rusqlite::params![playlist_id, content_id],
        )
        .expect("delete content entry to force strict-parity drift");
    }
    assert_fix_proposed(&backend, &usb, "upgrade_export_data_to_strict_parity");

    let pdb_path = vendor_db_dir(&usb).join("export.pdb");
    let mut perms = fs::metadata(&pdb_path).expect("pdb metadata").permissions();
    perms.set_readonly(true);
    fs::set_permissions(&pdb_path, perms).expect("make pdb read-only");

    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });

    let mut perms = fs::metadata(&pdb_path).expect("pdb metadata").permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(&pdb_path, perms).expect("restore pdb write permission");

    assert!(
        repair.ok,
        "repair call itself should still return ok: {repair:?}"
    );
    let data = repair.data.expect("repair data");
    assert!(
        data.failed_fixes
            .iter()
            .any(|m| m.contains("Upgrade Export Data To Strict Parity")),
        "expected the PDB write failure to be reported: {:#?}",
        data.failed_fixes
    );
}
