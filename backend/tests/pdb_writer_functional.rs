use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::Path;

use backend::commands::BackendCommands;
use backend::models::{
    AddTracksToPlaylistRequest, CreatePlaylistRequest, DedupeMode, ExportToUsbOptions,
    ExportToUsbRequest, InitializeUsbRequest, RemoveUsbPlaylistRequest, ScanLibraryRequest,
    SearchTracksRequest,
};
use backend::pdb_reader::parse_pdb;
use backend::service::usb_vendor_compat::DEFAULT_USB_EDB_KEY;
use tempfile::tempdir;

const USB_VENDOR_ROOT_DIR: &str = "PIONEER";
const USB_VENDOR_DB_DIR: &str = "rekordbox";

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
    }
    conn
}

fn fixture_audio_path(relative: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("audio")
        .join(relative)
}

fn copy_audio_fixture(media_dir: &Path, fixture_relative: &str, target_name: &str) {
    let fixture = fixture_audio_path(fixture_relative);
    fs::copy(&fixture, media_dir.join(target_name)).expect("copy audio fixture");
}

fn seed_test_analysis_bundle(data_dir: &Path, stem: &str) -> std::path::PathBuf {
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
                 tonality = '8A',
                 duration_ms = 180000,
                 waveform_peaks_path = ?1
             WHERE id = ?2",
            rusqlite::params![bundle.to_string_lossy().to_string(), track_id],
        )
        .expect("seed analyzed track fields");
    }
}

fn vendor_db_dir(root: &Path) -> std::path::PathBuf {
    root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)
}

fn pdb_path(usb_root: &Path) -> std::path::PathBuf {
    vendor_db_dir(usb_root).join("export.pdb")
}

fn edb_path(usb_root: &Path) -> std::path::PathBuf {
    vendor_db_dir(usb_root).join("exportLibrary.db")
}

/// Validate PDB header invariants:
/// - 20 tables, 4096-byte pages
/// - next_unused > max(empty_candidate)
/// - table pointer IDs cover 0..=19
fn validate_pdb_header(usb_root: &Path) {
    let bytes = fs::read(pdb_path(usb_root)).expect("read PDB");
    assert!(bytes.len() >= 4096, "PDB too small");
    let len_page = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    assert_eq!(len_page, 4096, "unexpected page size");
    let num_tables = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    assert_eq!(num_tables, 20, "expected 20-table PDB");
    let next_unused = u32::from_le_bytes(bytes[0x0c..0x10].try_into().unwrap());

    let mut table_ids = BTreeSet::new();
    let mut cursor = 28usize;
    for _ in 0..num_tables {
        let table_type = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
        let empty_candidate = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap());
        table_ids.insert(table_type);
        assert!(
            next_unused >= empty_candidate,
            "next_unused ({next_unused}) < empty_candidate ({empty_candidate}) for table {table_type}"
        );
        cursor += 16;
    }
    let expected_ids: BTreeSet<u32> = (0..=19).collect();
    assert_eq!(table_ids, expected_ids, "unexpected PDB table id set");
}

/// Validate PDB/eDB playlist ID and count parity.
fn validate_pdb_edb_parity(usb_root: &Path, expected_playlists: usize, expected_tracks: usize) {
    let parsed = parse_pdb(&pdb_path(usb_root)).expect("parse PDB");
    let pdb_playlist_count = parsed
        .playlist_tree
        .iter()
        .filter(|p| !p.row_is_folder)
        .count();
    assert_eq!(
        pdb_playlist_count, expected_playlists,
        "PDB playlist count mismatch"
    );
    assert_eq!(
        parsed.tracks.len(),
        expected_tracks,
        "PDB track count mismatch"
    );

    // eDB parity
    let conn = open_edb(&edb_path(usb_root));
    let edb_playlist_count: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM playlist WHERE attribute = 0",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        edb_playlist_count as usize, expected_playlists,
        "eDB playlist count mismatch"
    );

    // Verify playlist IDs match between PDB and eDB
    for pl in parsed.playlist_tree.iter().filter(|p| !p.row_is_folder) {
        let edb_id: Option<i64> = conn
            .query_row(
                "SELECT playlist_id FROM playlist WHERE name = ?1 AND attribute = 0 LIMIT 1",
                [&pl.name],
                |r| r.get(0),
            )
            .ok();
        assert_eq!(
            edb_id,
            Some(pl.id as i64),
            "playlist '{}' PDB id ({}) != eDB id ({:?})",
            pl.name,
            pl.id,
            edb_id
        );
    }
}

/// Setup: create backend, initialize USB, scan 4 tracks, seed as analyzed.
/// Returns (root, backend, usb_root, track_ids sorted by title).
fn setup_multi_track_fixture() -> (
    tempfile::TempDir,
    BackendCommands,
    std::path::PathBuf,
    Vec<String>,
) {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    copy_audio_fixture(&media, "embedded/track_embedded.mp3", "01 Track A.mp3");
    copy_audio_fixture(&media, "folder/track_folder.jpg.mp3", "02 Track B.mp3");
    copy_audio_fixture(&media, "noart/track_no_art.mp3", "03 Track C.mp3");
    copy_audio_fixture(
        &media,
        "parent/child/track_parent_folder.jpg.mp3",
        "04 Track D.mp3",
    );

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize_usb failed: {init:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let mut tracks = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    assert_eq!(tracks.len(), 4, "expected 4 scanned tracks");

    // Sort by title for deterministic ordering
    tracks.sort_by(|a, b| a.title.cmp(&b.title));
    let track_ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    (root, backend, usb, track_ids)
}

fn export_playlist(
    backend: &BackendCommands,
    usb: &Path,
    name: &str,
    track_ids: &[String],
) -> String {
    let created = backend.create_playlist(CreatePlaylistRequest {
        name: name.to_string(),
    });
    assert!(created.ok, "create_playlist failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: track_ids.to_vec(),
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add_tracks failed: {added:?}");

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
    assert!(export.ok, "export failed: {export:?}");
    playlist_id
}

#[test]
fn pdb_export_three_playlists_remove_middle_validates_structure() {
    let (_root, backend, usb, track_ids) = setup_multi_track_fixture();

    // Phase 1: Export 3 playlists
    // Playlist A: tracks 0, 1
    export_playlist(
        &backend,
        &usb,
        "Playlist A",
        &[track_ids[0].clone(), track_ids[1].clone()],
    );
    validate_pdb_header(&usb);
    validate_pdb_edb_parity(&usb, 1, 2);

    // Capture track IDs for stability check
    let parsed_after_a = parse_pdb(&pdb_path(&usb)).expect("parse after A");
    let track_a_ids: HashSet<u32> = parsed_after_a.tracks.iter().map(|t| t.id).collect();

    // Playlist B: tracks 1, 2 (track 1 shared with A)
    export_playlist(
        &backend,
        &usb,
        "Playlist B",
        &[track_ids[1].clone(), track_ids[2].clone()],
    );
    validate_pdb_header(&usb);
    validate_pdb_edb_parity(&usb, 2, 3);

    // Verify track ID stability — tracks from playlist A should keep their IDs
    let parsed_after_b = parse_pdb(&pdb_path(&usb)).expect("parse after B");
    let track_b_ids: HashSet<u32> = parsed_after_b.tracks.iter().map(|t| t.id).collect();
    assert!(
        track_a_ids.is_subset(&track_b_ids),
        "track IDs from playlist A should be preserved after adding B"
    );

    // Playlist C: tracks 2, 3 (track 2 shared with B)
    export_playlist(
        &backend,
        &usb,
        "Playlist C",
        &[track_ids[2].clone(), track_ids[3].clone()],
    );
    validate_pdb_header(&usb);
    validate_pdb_edb_parity(&usb, 3, 4);

    // Verify all previous track IDs preserved
    let parsed_after_c = parse_pdb(&pdb_path(&usb)).expect("parse after C");
    let track_c_ids: HashSet<u32> = parsed_after_c.tracks.iter().map(|t| t.id).collect();
    assert!(
        track_b_ids.is_subset(&track_c_ids),
        "track IDs from playlists A+B should be preserved after adding C"
    );

    // Verify entry counts: A=2, B=2, C=2 → 6 total entries
    assert_eq!(
        parsed_after_c.playlist_entries.len(),
        6,
        "expected 6 total playlist entries"
    );

    // Phase 2: Remove playlist B
    // Track 1 is shared (in A) → preserved
    // Track 2 is shared (in C) → preserved
    let removed = backend.remove_usb_playlist(RemoveUsbPlaylistRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: None,
        playlist_name: "Playlist B".to_string(),
    });
    assert!(removed.ok, "remove Playlist B failed: {removed:?}");

    validate_pdb_header(&usb);
    validate_pdb_edb_parity(&usb, 2, 4);

    let parsed_after_remove_b = parse_pdb(&pdb_path(&usb)).expect("parse after remove B");
    let remaining_names: BTreeSet<&str> = parsed_after_remove_b
        .playlist_tree
        .iter()
        .filter(|p| !p.row_is_folder)
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(
        remaining_names,
        ["Playlist A", "Playlist C"]
            .into_iter()
            .collect::<BTreeSet<_>>(),
        "only A and C should remain"
    );

    // All 4 tracks preserved (all shared)
    assert_eq!(
        parsed_after_remove_b.tracks.len(),
        4,
        "all tracks should be preserved (all are shared)"
    );

    // Track IDs unchanged
    let track_ids_after_remove: HashSet<u32> =
        parsed_after_remove_b.tracks.iter().map(|t| t.id).collect();
    assert_eq!(
        track_ids_after_remove, track_c_ids,
        "track IDs should be unchanged after removing B"
    );

    // Entries: A=2, C=2 → 4 total
    assert_eq!(
        parsed_after_remove_b.playlist_entries.len(),
        4,
        "expected 4 entries after removing B"
    );

    // Phase 3: Remove playlist A
    // Track 0 is exclusive → removed
    // Track 1 is exclusive (was shared with B, but B is gone) → removed
    let removed = backend.remove_usb_playlist(RemoveUsbPlaylistRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: None,
        playlist_name: "Playlist A".to_string(),
    });
    assert!(removed.ok, "remove Playlist A failed: {removed:?}");

    validate_pdb_header(&usb);
    validate_pdb_edb_parity(&usb, 1, 2);

    let parsed_final = parse_pdb(&pdb_path(&usb)).expect("parse final");
    let final_names: Vec<&str> = parsed_final
        .playlist_tree
        .iter()
        .filter(|p| !p.row_is_folder)
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(final_names, vec!["Playlist C"], "only C should remain");

    // 2 tracks: track 2 and track 3 (from playlist C)
    assert_eq!(
        parsed_final.tracks.len(),
        2,
        "expected 2 tracks for playlist C"
    );

    // Entries: C=2
    assert_eq!(
        parsed_final.playlist_entries.len(),
        2,
        "expected 2 entries for playlist C"
    );
}

#[test]
fn pdb_re_export_same_playlist_preserves_track_ids() {
    let (_root, backend, usb, track_ids) = setup_multi_track_fixture();

    // Export with 2 tracks
    let playlist_id = export_playlist(
        &backend,
        &usb,
        "Stable IDs",
        &[track_ids[0].clone(), track_ids[1].clone()],
    );
    let parsed1 = parse_pdb(&pdb_path(&usb)).expect("parse 1");
    let ids1: Vec<u32> = parsed1.tracks.iter().map(|t| t.id).collect();
    assert_eq!(ids1.len(), 2);

    // Re-export same playlist adding a third track
    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_ids[2].clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add track failed");

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
    assert!(export.ok, "re-export failed");

    let parsed2 = parse_pdb(&pdb_path(&usb)).expect("parse 2");
    let ids2: HashSet<u32> = parsed2.tracks.iter().map(|t| t.id).collect();

    // Original 2 track IDs must still be present
    for id in &ids1 {
        assert!(
            ids2.contains(id),
            "original track id {id} lost after re-export"
        );
    }
    assert_eq!(parsed2.tracks.len(), 3, "expected 3 tracks after re-export");
    validate_pdb_header(&usb);
}
