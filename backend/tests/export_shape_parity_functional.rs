use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use backend::commands::BackendCommands;
use backend::models::InitializeUsbRequest;
use backend::service::usb_vendor_compat::DEFAULT_USB_EDB_KEY;
use backend::shape_compare::compare_usb_shape;
use rusqlite::Connection;
use tempfile::tempdir;

const USB_VENDOR_ROOT_DIR: &str = "PIONEER";
const USB_VENDOR_DB_DIR: &str = "rekordbox";

#[derive(Debug, Clone, PartialEq, Eq)]
struct PdbTablePointer {
    table_type: u32,
    first_page: u32,
    last_page: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShapeSnapshot {
    edb_schema: BTreeMap<String, Vec<String>>,
    pdb_len_page: u32,
    pdb_num_tables: u32,
    pdb_table_pointers: Vec<PdbTablePointer>,
}

fn vendor_db_dir(root: &Path) -> PathBuf {
    root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)
}

fn edb_path(root: &Path) -> PathBuf {
    vendor_db_dir(root).join("exportLibrary.db")
}

fn pdb_path(root: &Path) -> PathBuf {
    vendor_db_dir(root).join("export.pdb")
}

fn open_edb(path: &Path) -> Connection {
    let conn = Connection::open(path).expect("open eDB");
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

fn load_edb_schema(path: &Path) -> BTreeMap<String, Vec<String>> {
    let conn = open_edb(path);
    let mut schema = BTreeMap::<String, Vec<String>>::new();
    let mut tables_stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .expect("prepare table list");
    let table_rows = tables_stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query table list");
    for table in table_rows {
        let table = table.expect("table name");
        let mut cols_stmt = conn
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .expect("prepare table_info");
        let cols = cols_stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query table_info")
            .map(|row| row.expect("column name"))
            .collect::<Vec<_>>();
        schema.insert(table, cols);
    }
    schema
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let a = *bytes.get(offset)?;
    let b = *bytes.get(offset + 1)?;
    let c = *bytes.get(offset + 2)?;
    let d = *bytes.get(offset + 3)?;
    Some(u32::from_le_bytes([a, b, c, d]))
}

fn load_pdb_table_pointers(path: &Path) -> (u32, u32, Vec<PdbTablePointer>) {
    let bytes = fs::read(path).expect("read PDB");
    let len_page = read_u32_le(&bytes, 4).expect("len_page");
    let num_tables = read_u32_le(&bytes, 8).expect("num_tables");
    let mut pointers = Vec::<PdbTablePointer>::with_capacity(num_tables as usize);
    let mut cursor = 28usize;
    for _ in 0..num_tables {
        let table_type = read_u32_le(&bytes, cursor).expect("table_type");
        let first_page = read_u32_le(&bytes, cursor + 8).expect("first_page");
        let last_page = read_u32_le(&bytes, cursor + 12).expect("last_page");
        pointers.push(PdbTablePointer {
            table_type,
            first_page,
            last_page,
        });
        cursor += 16;
    }
    (len_page, num_tables, pointers)
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    let a = *bytes.get(offset)?;
    let b = *bytes.get(offset + 1)?;
    Some(u16::from_le_bytes([a, b]))
}

fn mutate_first_sentinel_marker(pdb_file: &Path) -> bool {
    let mut bytes = fs::read(pdb_file).expect("read PDB for sentinel mutation");
    let len_page = read_u32_le(&bytes, 4).unwrap_or(4096) as usize;
    if len_page == 0 || bytes.len() < len_page {
        return false;
    }
    let total_pages = bytes.len() / len_page;
    for page_idx in 1..total_pages {
        let page_off = page_idx * len_page;
        let packed_lo = bytes.get(page_off + 0x18).copied().unwrap_or(0) as u32;
        let packed_mid = bytes.get(page_off + 0x19).copied().unwrap_or(0) as u32;
        let packed_hi = bytes.get(page_off + 0x1a).copied().unwrap_or(0) as u32;
        let packed = packed_lo | (packed_mid << 8) | (packed_hi << 16);
        let num_rl = packed & 0x1FFF;
        if num_rl == 8191 {
            let mutated = packed & !0x1FFF; // set num_rl -> 0, keep other packed bits
            bytes[page_off + 0x18] = (mutated & 0xFF) as u8;
            bytes[page_off + 0x19] = ((mutated >> 8) & 0xFF) as u8;
            bytes[page_off + 0x1a] = ((mutated >> 16) & 0xFF) as u8;
            fs::write(pdb_file, bytes).expect("write mutated PDB");
            return true;
        }
        let page_flags = bytes.get(page_off + 0x1b).copied().unwrap_or(0);
        if page_flags == 0x64 {
            bytes[page_off + 0x1b] = 0x00;
            fs::write(pdb_file, bytes).expect("write mutated PDB");
            return true;
        }
    }
    false
}

/// Count data rows across all pages belonging to a given table type.
fn count_pdb_table_rows(pdb_bytes: &[u8], target_table_type: u32) -> usize {
    let len_page = read_u32_le(pdb_bytes, 4).unwrap_or(4096) as usize;
    if len_page == 0 || pdb_bytes.len() < len_page {
        return 0;
    }
    let total_pages = pdb_bytes.len() / len_page;
    let mut row_count = 0usize;
    // Skip page 0 (file header)
    for page_idx in 1..total_pages {
        let page_off = page_idx * len_page;
        let tt = read_u32_le(pdb_bytes, page_off + 0x08).unwrap_or(u32::MAX);
        if tt != target_table_type {
            continue;
        }
        let page_flags = pdb_bytes.get(page_off + 0x1b).copied().unwrap_or(0);
        // Skip sentinel/index pages (0x64)
        if page_flags == 0x64 {
            continue;
        }
        let used_size = read_u16_le(pdb_bytes, page_off + 0x1e).unwrap_or(0) as usize;
        if used_size == 0 {
            continue;
        }
        // num_row_slots from packed bytes at 0x18-0x1a (bits 0-12)
        let packed_lo = pdb_bytes.get(page_off + 0x18).copied().unwrap_or(0) as u32;
        let packed_mid = pdb_bytes.get(page_off + 0x19).copied().unwrap_or(0) as u32;
        let packed_hi = pdb_bytes.get(page_off + 0x1a).copied().unwrap_or(0) as u32;
        let packed = packed_lo | (packed_mid << 8) | (packed_hi << 16);
        let num_row_slots = (packed & 0x1FFF) as usize;
        row_count += num_row_slots;
    }
    row_count
}

fn fixture_audio_track() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("audio")
        .join("formats")
        .join("track_format_wav.wav")
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
    let conn = Connection::open(&db_path).expect("open backend db");
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

fn snapshot_usb_shape(root: &Path) -> ShapeSnapshot {
    let edb_schema = load_edb_schema(&edb_path(root));
    let (pdb_len_page, pdb_num_tables, pdb_table_pointers) =
        load_pdb_table_pointers(&pdb_path(root));
    ShapeSnapshot {
        edb_schema,
        pdb_len_page,
        pdb_num_tables,
        pdb_table_pointers,
    }
}

#[test]
fn strict_shape_compare_detects_match_and_mismatch_on_generated_usb_roots() {
    let temp = tempdir().expect("temp dir");
    let expected_root = temp.path().join("usb_expected");
    let actual_root = temp.path().join("usb_actual");
    fs::create_dir_all(&expected_root).expect("create expected root");
    fs::create_dir_all(&actual_root).expect("create actual root");

    let data_dir = temp.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let fixture_track = fixture_audio_track();
    assert!(
        fixture_track.is_file(),
        "fixture audio track missing: {}",
        fixture_track.display()
    );
    let media_dir = temp.path().join("media");
    fs::create_dir_all(&media_dir).expect("create media dir");
    fs::copy(&fixture_track, media_dir.join("shape_compare_track.mp3"))
        .expect("copy fixture audio");

    let expected_init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: expected_root.to_string_lossy().to_string(),
    });
    assert!(
        expected_init.ok,
        "initialize expected failed: {expected_init:?}"
    );

    let actual_init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: actual_root.to_string_lossy().to_string(),
    });
    assert!(actual_init.ok, "initialize actual failed: {actual_init:?}");

    let scan = backend.scan_library(backend::models::ScanLibraryRequest {
        source_roots: vec![media_dir.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let search = backend.search_tracks(backend::models::SearchTracksRequest {
        query: String::new(),
        limit: 100,
        cursor: None,
    });
    let tracks = search.data.expect("search data").items;
    assert!(
        !tracks.is_empty(),
        "no tracks found after fixture scan/analyze"
    );
    let track_ids = tracks
        .iter()
        .map(|track| track.id.clone())
        .collect::<Vec<_>>();
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let created = backend.create_playlist(backend::models::CreatePlaylistRequest {
        name: "ShapeCompareExport".to_string(),
    });
    assert!(created.ok, "create playlist failed: {created:?}");
    let playlist_id = created.data.unwrap().playlist_id;

    let added = backend.add_tracks_to_playlist(backend::models::AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: tracks.iter().map(|t| t.id.clone()).collect(),
        dedupe: backend::models::DedupeMode::Skip,
    });
    assert!(added.ok, "add tracks failed: {added:?}");

    let export_expected = backend.export_to_usb(backend::models::ExportToUsbRequest {
        usb_root: Some(expected_root.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(backend::models::ExportToUsbOptions {
            include_artwork: false,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(
        export_expected.ok,
        "export expected failed: {export_expected:?}"
    );

    let export_actual = backend.export_to_usb(backend::models::ExportToUsbRequest {
        usb_root: Some(actual_root.to_string_lossy().to_string()),
        playlist_id,
        options: Some(backend::models::ExportToUsbOptions {
            include_artwork: false,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export_actual.ok, "export actual failed: {export_actual:?}");

    let matching_diff = compare_usb_shape(&expected_root, &actual_root);
    assert!(
        matching_diff.strict_match,
        "fixture-exported USB roots should match: {matching_diff:#?}"
    );

    // Force structural mismatch on actual eDB.
    let conn = open_edb(&edb_path(&actual_root));
    conn.execute_batch("CREATE TABLE __shape_mismatch_probe(id INTEGER);")
        .expect("create probe table");

    let mismatching_diff = compare_usb_shape(&expected_root, &actual_root);
    assert!(
        !mismatching_diff.strict_match,
        "shape compare must fail after schema divergence: {mismatching_diff:#?}"
    );
}

#[test]
fn strict_shape_compare_detects_sentinel_mutation_case() {
    let temp = tempdir().expect("temp dir");
    let expected_root = temp.path().join("usb_expected");
    let actual_root = temp.path().join("usb_actual");
    fs::create_dir_all(&expected_root).expect("create expected root");
    fs::create_dir_all(&actual_root).expect("create actual root");

    let data_dir = temp.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let expected_init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: expected_root.to_string_lossy().to_string(),
    });
    assert!(
        expected_init.ok,
        "initialize expected failed: {expected_init:?}"
    );
    let actual_init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: actual_root.to_string_lossy().to_string(),
    });
    assert!(actual_init.ok, "initialize actual failed: {actual_init:?}");

    let baseline = compare_usb_shape(&expected_root, &actual_root);
    assert!(
        baseline.strict_match,
        "baseline initialized roots must strictly match: {baseline:#?}"
    );

    let mutated = mutate_first_sentinel_marker(&pdb_path(&actual_root));
    assert!(
        mutated,
        "no sentinel marker (num_rl=8191 or page_flags=0x64) found to mutate"
    );

    let after = compare_usb_shape(&expected_root, &actual_root);
    assert!(
        !after.strict_match,
        "strict shape compare must fail after sentinel mutation: {after:#?}"
    );
    assert!(
        !after.pdb_table_shape_diffs.is_empty(),
        "sentinel mutation should surface as table shape diff"
    );
}

#[test]
fn initialize_usb_from_scratch_produces_valid_fullshape() {
    let temp = tempdir().expect("temp dir");
    let usb_root = temp.path().join("usb");
    fs::create_dir_all(&usb_root).expect("create usb root");
    let data_dir = temp.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize_usb failed: {init:?}");

    let actual = snapshot_usb_shape(&usb_root);

    // PDB: 20 tables, 4096-byte pages
    assert_eq!(actual.pdb_len_page, 4096, "unexpected PDB page size");
    assert_eq!(actual.pdb_num_tables, 20, "expected full 20-table PDB map");
    let table_ids = actual
        .pdb_table_pointers
        .iter()
        .map(|p| p.table_type)
        .collect::<BTreeSet<_>>();
    let expected_ids = (0u32..=19u32).collect::<BTreeSet<_>>();
    assert_eq!(table_ids, expected_ids, "unexpected PDB table id family");

    // eDB: full schema with all required tables
    let required_tables: BTreeSet<&str> = [
        "album",
        "artist",
        "category",
        "color",
        "content",
        "cue",
        "genre",
        "history",
        "history_content",
        "hotCueBankList",
        "hotCueBankList_cue",
        "image",
        "key",
        "label",
        "menuItem",
        "myTag",
        "myTag_content",
        "playlist",
        "playlist_content",
        "property",
        "recommendedLike",
        "sort",
    ]
    .into_iter()
    .collect();
    let actual_tables: BTreeSet<&str> = actual.edb_schema.keys().map(|s| s.as_str()).collect();
    assert_eq!(actual_tables, required_tables, "eDB table set mismatch");

    // Per-table row count checks
    let pdb_bytes = fs::read(pdb_path(&usb_root)).expect("read PDB");
    assert_eq!(
        count_pdb_table_rows(&pdb_bytes, 6),
        8,
        "initialized PDB: colors table should have 8 rows"
    );
    assert_eq!(
        count_pdb_table_rows(&pdb_bytes, 16),
        27,
        "initialized PDB: columns table should have all 27 player browse categories"
    );
    // Core content tables should be empty after initialization (no tracks/playlists/artwork yet).
    // History seed tables (17/18/19) are intentionally non-zero in the baseline shape.
    for tt in [0u32, 1, 2, 3, 4, 5, 7, 8, 13] {
        assert_eq!(
            count_pdb_table_rows(&pdb_bytes, tt),
            0,
            "initialized PDB: table_type={tt} should have 0 rows"
        );
    }
    assert_eq!(
        count_pdb_table_rows(&pdb_bytes, 17),
        22,
        "initialized PDB: table_type=17 should carry baseline seed rows"
    );
    assert_eq!(
        count_pdb_table_rows(&pdb_bytes, 18),
        17,
        "initialized PDB: table_type=18 should carry baseline seed rows"
    );
    assert_eq!(
        count_pdb_table_rows(&pdb_bytes, 19),
        1,
        "initialized PDB: table_type=19 should carry baseline seed row"
    );
}

#[test]
fn export_preserves_full_20_table_family() {
    // Initialize USB, scan+analyze one track, export — verify PDB still has 20 tables
    let temp = tempdir().expect("temp dir");
    let usb_root = temp.path().join("usb");
    fs::create_dir_all(&usb_root).expect("create usb root");
    let data_dir = temp.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    // Initialize
    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize_usb failed: {init:?}");

    // Seed fixture audio file
    let media_dir = temp.path().join("media");
    fs::create_dir_all(&media_dir).expect("create media dir");
    let fixture_track = fixture_audio_track();
    assert!(
        fixture_track.is_file(),
        "fixture audio track missing: {}",
        fixture_track.display()
    );
    fs::copy(&fixture_track, media_dir.join("track_fixture.mp3")).expect("copy fixture audio");

    // Scan + analyze
    let scan = backend.scan_library(backend::models::ScanLibraryRequest {
        source_roots: vec![media_dir.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let search = backend.search_tracks(backend::models::SearchTracksRequest {
        query: String::new(),
        limit: 100,
        cursor: None,
    });
    let tracks = search.data.expect("search data").items;
    assert!(!tracks.is_empty(), "no tracks found");

    let track_ids = tracks
        .iter()
        .map(|track| track.id.clone())
        .collect::<Vec<_>>();
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    // Create playlist and add track
    let created = backend.create_playlist(backend::models::CreatePlaylistRequest {
        name: "ShapeTest".to_string(),
    });
    assert!(created.ok, "create playlist failed: {created:?}");
    let playlist_id = created.data.unwrap().playlist_id;

    let added = backend.add_tracks_to_playlist(backend::models::AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: tracks.iter().map(|t| t.id.clone()).collect(),
        dedupe: backend::models::DedupeMode::Skip,
    });
    assert!(added.ok, "add tracks failed: {added:?}");

    // Export
    let exported = backend.export_to_usb(backend::models::ExportToUsbRequest {
        usb_root: Some(usb_root.to_string_lossy().to_string()),
        playlist_id,
        options: Some(backend::models::ExportToUsbOptions {
            include_artwork: false,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");

    // Verify PDB shape after export
    let actual = snapshot_usb_shape(&usb_root);
    assert_eq!(
        actual.pdb_num_tables, 20,
        "PDB must retain 20 tables after export"
    );
    let table_ids = actual
        .pdb_table_pointers
        .iter()
        .map(|p| p.table_type)
        .collect::<BTreeSet<_>>();
    let expected_ids = (0u32..=19u32).collect::<BTreeSet<_>>();
    assert_eq!(
        table_ids, expected_ids,
        "PDB table id family changed after export"
    );

    // Verify key table row counts
    let pdb_bytes = fs::read(pdb_path(&usb_root)).expect("read PDB");
    assert!(
        count_pdb_table_rows(&pdb_bytes, 0) >= 1,
        "exported PDB: tracks table should have rows"
    );
    assert!(
        count_pdb_table_rows(&pdb_bytes, 2) >= 1,
        "exported PDB: artists table should have rows"
    );
    assert_eq!(
        count_pdb_table_rows(&pdb_bytes, 6),
        8,
        "exported PDB: colors table should have 8 rows"
    );
    assert_eq!(
        count_pdb_table_rows(&pdb_bytes, 16),
        27,
        "exported PDB: columns table should have all 27 player browse categories"
    );
    assert!(
        count_pdb_table_rows(&pdb_bytes, 7) >= 1,
        "exported PDB: playlist_tree should have rows"
    );
    assert!(
        count_pdb_table_rows(&pdb_bytes, 8) >= 1,
        "exported PDB: playlist_entries should have rows"
    );
}
