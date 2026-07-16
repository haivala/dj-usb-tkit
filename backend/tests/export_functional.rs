use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use backend::commands::BackendCommands;
use backend::error::ErrorCode;
use backend::models::{
    AddTracksToPlaylistRequest, CreatePlaylistRequest, DedupeMode, ExportToUsbOptions,
    ExportToUsbRequest, FetchUsbPlaylistsRequest, GetPlaylistTracksRequest, InitializeUsbRequest,
    MaterializeSourceTrackRequest, RemoveTracksFromPlaylistRequest, ScanLibraryRequest,
    SearchTracksRequest,
};
use backend::pdb_reader::parse_pdb;
use backend::service::usb_vendor_compat::DEFAULT_USB_EDB_KEY;
use tempfile::tempdir;
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq)]
struct UsbMenuSnapshot {
    edb_tables: BTreeMap<String, Vec<Vec<String>>>,
    pdb_menu_pages: BTreeMap<u32, Vec<Vec<u8>>>,
}

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

const USB_VENDOR_ROOT_DIR: &str = "PIONEER";
const USB_VENDOR_DB_DIR: &str = "rekordbox";
const USB_ARTWORK_DIR: &str = "Artwork";
const USB_ANALYSIS_DIR: &str = "USBANLZ";

fn test_usb_analysis_hash(track_path: &str) -> u32 {
    let mut hash = 0u32;
    for code_unit in track_path.encode_utf16() {
        let value = code_unit as u32;
        hash = 37_813u32
            .wrapping_mul(23_497u32.wrapping_mul(hash).wrapping_add(value))
            .wrapping_add(value);
    }
    let reduced = ((0xA7C5_075Bu64 * hash as u64) >> 49) as u32;
    hash.wrapping_sub(0x30D43u32.wrapping_mul(reduced))
}

fn test_usb_analysis_bucket(hash: u32) -> u16 {
    let mut bucket = (hash & 0x1) as u16;
    bucket |= ((hash >> 1) & 0x2) as u16;
    bucket |= ((hash >> 4) & 0x4) as u16;
    bucket |= ((hash >> 4) & 0x8) as u16;
    bucket |= ((hash >> 5) & 0x10) as u16;
    bucket |= ((hash >> 8) & 0x20) as u16;
    bucket |= ((hash >> 10) & 0x40) as u16;
    bucket
}

fn canonical_usb_analysis_bundle_paths(
    usb_root: &Path,
    track_path: &str,
) -> [std::path::PathBuf; 3] {
    let hash = test_usb_analysis_hash(track_path);
    let bucket = test_usb_analysis_bucket(hash);
    let root = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_ANALYSIS_DIR)
        .join(format!("P{bucket:03X}"))
        .join(format!("{hash:08X}"));
    [
        root.join("ANLZ0000.DAT"),
        root.join("ANLZ0000.EXT"),
        root.join("ANLZ0000.2EX"),
    ]
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
    let safe_stem = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let dat = dir.join(format!("{safe_stem}.DAT"));
    let ext = dir.join(format!("{safe_stem}.EXT"));
    let twoex = dir.join(format!("{safe_stem}.2EX"));
    fs::write(&dat, b"test-dat").expect("write test DAT");
    fs::write(&ext, b"test-ext").expect("write test EXT");
    fs::write(&twoex, b"test-2ex").expect("write test 2EX");
    dat
}

fn cover_fixture_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("audio")
        .join("folder")
        .join("cover.jpg")
}

fn seed_tracks_as_analyzed(data_dir: &Path, track_ids: &[String]) {
    let db_path = data_dir.join("backend.db");
    let conn = rusqlite::Connection::open(&db_path).expect("open backend db");
    for (idx, track_id) in track_ids.iter().enumerate() {
        let fake_waveform = seed_test_analysis_bundle(data_dir, &format!("test-waveform-{idx}"));
        conn.execute(
            "UPDATE tracks
             SET bpm = 120.0,
                 tonality = '8A',
                 duration_ms = 180000,
                 waveform_peaks_path = ?1
             WHERE id = ?2",
            rusqlite::params![fake_waveform.to_string_lossy().to_string(), track_id],
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

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let a = *bytes.get(offset)?;
    let b = *bytes.get(offset + 1)?;
    let c = *bytes.get(offset + 2)?;
    let d = *bytes.get(offset + 3)?;
    Some(u32::from_le_bytes([a, b, c, d]))
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    let a = *bytes.get(offset)?;
    let b = *bytes.get(offset + 1)?;
    Some(u16::from_le_bytes([a, b]))
}

fn write_u16_le(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn pdb_table_ptr(bytes: &[u8], table_type: u32) -> Option<(u32, u32, u32)> {
    let table_count = read_u32_le(bytes, 0x08)? as usize;
    for idx in 0..table_count {
        let off = 0x1c + idx * 16;
        if read_u32_le(bytes, off)? == table_type {
            return Some((
                read_u32_le(bytes, off + 4)?,
                read_u32_le(bytes, off + 8)?,
                read_u32_le(bytes, off + 12)?,
            ));
        }
    }
    None
}

fn load_query_rows(conn: &rusqlite::Connection, sql: &str) -> Vec<Vec<String>> {
    let mut stmt = conn.prepare(sql).expect("prepare menu snapshot query");
    let column_count = stmt.column_count();
    stmt.query_map([], |row| {
        let mut values = Vec::with_capacity(column_count);
        for idx in 0..column_count {
            let value = row.get_ref(idx)?;
            let rendered = match value {
                rusqlite::types::ValueRef::Null => "<NULL>".to_string(),
                rusqlite::types::ValueRef::Integer(v) => v.to_string(),
                rusqlite::types::ValueRef::Real(v) => v.to_string(),
                rusqlite::types::ValueRef::Text(v) => String::from_utf8_lossy(v).into_owned(),
                rusqlite::types::ValueRef::Blob(v) => format!("{v:02x?}"),
            };
            values.push(rendered);
        }
        Ok(values)
    })
    .expect("query menu snapshot")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect menu snapshot")
}

fn load_pdb_table_pages(usb_root: &Path, table_types: &[u32]) -> BTreeMap<u32, Vec<Vec<u8>>> {
    let pdb_file = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let bytes = fs::read(&pdb_file).expect("read PDB for menu snapshot");
    let page_size = read_u32_le(&bytes, 4).expect("PDB page size") as usize;
    assert!(page_size > 0, "PDB page size must be non-zero");

    let mut pages = BTreeMap::<u32, Vec<Vec<u8>>>::new();
    for page in bytes.chunks_exact(page_size).skip(1) {
        let Some(table_type) = read_u32_le(page, 0x08) else {
            continue;
        };
        if table_types.contains(&table_type) {
            pages.entry(table_type).or_default().push(page.to_vec());
        }
    }
    pages
}

fn snapshot_usb_menu(usb_root: &Path) -> UsbMenuSnapshot {
    let conn = open_edb(
        &usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db"),
    );
    let mut edb_tables = BTreeMap::new();
    edb_tables.insert(
        "menuItem".to_string(),
        load_query_rows(
            &conn,
            "SELECT menuItem_id, kind, name FROM menuItem ORDER BY menuItem_id",
        ),
    );
    edb_tables.insert(
        "category".to_string(),
        load_query_rows(
            &conn,
            "SELECT category_id, menuItem_id, sequenceNo, isVisible FROM category ORDER BY category_id",
        ),
    );
    edb_tables.insert(
        "sort".to_string(),
        load_query_rows(
            &conn,
            "SELECT sort_id, menuItem_id, sequenceNo, isVisible, isSelectedAsSubColumn FROM sort ORDER BY sort_id",
        ),
    );

    UsbMenuSnapshot {
        edb_tables,
        // PDB t16=columns, t17=category, t18=sort. CDJs consume these for the
        // browse menu, so a playlist export must not rewrite them.
        pdb_menu_pages: load_pdb_table_pages(usb_root, &[16, 17, 18]),
    }
}

fn first_byte_difference(left: &[u8], right: &[u8]) -> Option<(usize, Option<u8>, Option<u8>)> {
    let max_len = left.len().max(right.len());
    for idx in 0..max_len {
        let a = left.get(idx).copied();
        let b = right.get(idx).copied();
        if a != b {
            return Some((idx, a, b));
        }
    }
    None
}

fn assert_usb_menu_unchanged(before: &UsbMenuSnapshot, after: &UsbMenuSnapshot, context: &str) {
    assert_eq!(
        after.edb_tables, before.edb_tables,
        "{context}: playlist export must not change eDB menuItem/category/sort rows"
    );

    assert_eq!(
        after.pdb_menu_pages.keys().collect::<Vec<_>>(),
        before.pdb_menu_pages.keys().collect::<Vec<_>>(),
        "{context}: playlist export changed the set of PDB menu tables"
    );
    for (table_type, before_pages) in &before.pdb_menu_pages {
        let after_pages = after
            .pdb_menu_pages
            .get(table_type)
            .expect("after snapshot table exists");
        assert_eq!(
            after_pages.len(),
            before_pages.len(),
            "{context}: PDB menu table t{table_type} page count changed"
        );
        for (page_idx, (before_page, after_page)) in
            before_pages.iter().zip(after_pages.iter()).enumerate()
        {
            if let Some((byte_idx, before_byte, after_byte)) =
                first_byte_difference(before_page, after_page)
            {
                panic!(
                    "{context}: playlist export changed PDB menu table t{table_type} page {page_idx} byte {byte_idx}: before={before_byte:?} after={after_byte:?}"
                );
            }
        }
    }
}

#[test]
fn export_to_usb_rejects_empty_or_whitespace_playlist_id() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb dir");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let empty = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: String::new(),
        options: None,
    });
    assert!(!empty.ok, "expected empty playlistId export to fail");
    let empty_error = empty.error.expect("empty export error");
    assert!(matches!(empty_error.code, ErrorCode::ValidationError));

    let whitespace = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: "   \n\t ".to_string(),
        options: None,
    });
    assert!(
        !whitespace.ok,
        "expected whitespace-only playlistId export to fail"
    );
    let whitespace_error = whitespace.error.expect("whitespace export error");
    assert!(matches!(whitespace_error.code, ErrorCode::ValidationError));
}

#[test]
fn scan_library_detects_real_flac_wav_and_aif_fixtures() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");
    copy_audio_fixture(
        &media,
        "formats/track_format_flac.flac",
        "Fixture Format.flac",
    );
    copy_audio_fixture(&media, "formats/track_format_wav.wav", "Fixture Format.wav");
    copy_audio_fixture(&media, "formats/track_format_aif.aif", "Fixture Format.aif");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let items = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    assert_eq!(items.len(), 3, "all three format fixtures should scan");
}

#[test]
fn scan_library_detects_wav_extensible_kind_for_real_fixtures() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");
    copy_audio_fixture(&media, "formats/track_format_wav.wav", "Plain.wav");
    copy_audio_fixture(
        &media,
        "formats/track_format_wav_extensible.wav",
        "Extensible Pcm.wav",
    );
    copy_audio_fixture(
        &media,
        "formats/track_format_wav_extensible_other.wav",
        "Extensible Other.wav",
    );

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let items = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    assert_eq!(items.len(), 3);

    let kind_for = |needle: &str| {
        items
            .iter()
            .find(|t| t.file_path.contains(needle))
            .unwrap_or_else(|| panic!("missing scanned track for {needle}"))
            .wav_extensible_kind
            .clone()
    };
    assert_eq!(kind_for("Plain.wav"), None);
    assert_eq!(
        kind_for("Extensible Pcm.wav"),
        Some("extensible_pcm".to_string())
    );
    assert_eq!(
        kind_for("Extensible Other.wav"),
        Some("extensible_other".to_string())
    );
}

#[test]
fn export_to_usb_normalizes_extensible_pcm_wav_and_leaves_extensible_other_untouched() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    copy_audio_fixture(
        &media,
        "formats/track_format_wav_extensible.wav",
        "Wav Extensible Artist - Extensible Pcm.wav",
    );
    copy_audio_fixture(
        &media,
        "formats/track_format_wav_extensible_other.wav",
        "Wav Extensible Artist - Extensible Other.wav",
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
            query: "wav extensible".to_string(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    assert_eq!(tracks.len(), 2, "expected both extensible wav fixtures");
    let track_ids = tracks.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Wav Extensible Export".to_string(),
    });
    assert!(created.ok, "create playlist failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids,
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add tracks failed: {added:?}");

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");
    assert_eq!(exported.data.expect("export data").exported_tracks, 2);

    let conn = open_edb(
        &usb.join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db"),
    );
    let mut stmt = conn
        .prepare("SELECT path FROM content ORDER BY path")
        .expect("prepare content path query");
    let edb_paths = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query content paths")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect content paths");

    let exported_pcm = edb_paths
        .iter()
        .find(|p| p.contains("Extensible Pcm"))
        .map(|p| usb.join(p.trim_start_matches('/')))
        .expect("exported extensible-pcm path");
    let exported_other = edb_paths
        .iter()
        .find(|p| p.contains("Extensible Other"))
        .map(|p| usb.join(p.trim_start_matches('/')))
        .expect("exported extensible-other path");

    // Extensible-PCM should be normalized: fmt chunk shrunk to standard 16
    // bytes with format tag 1 (PCM), sample data preserved verbatim.
    let pcm_bytes = fs::read(&exported_pcm).expect("read exported pcm wav");
    let format_tag = u16::from_le_bytes(pcm_bytes[20..22].try_into().unwrap());
    assert_eq!(format_tag, 1, "expected normalized PCM format tag");
    let fmt_chunk_size = u32::from_le_bytes(pcm_bytes[16..20].try_into().unwrap());
    assert_eq!(fmt_chunk_size, 16, "expected standard 16-byte fmt chunk");
    let source_pcm_bytes = fs::read(fixture_audio_path(
        "formats/track_format_wav_extensible.wav",
    ))
    .unwrap();
    assert!(
        pcm_bytes.ends_with(&source_pcm_bytes[source_pcm_bytes.len() - 100..]),
        "sample data tail must be preserved verbatim through normalization"
    );

    // Extensible-other has no safe subformat to convert to, so it must be
    // exported byte-for-byte unchanged (still carrying the hard warning).
    let other_bytes = fs::read(&exported_other).expect("read exported other wav");
    let source_other_bytes = fs::read(fixture_audio_path(
        "formats/track_format_wav_extensible_other.wav",
    ))
    .unwrap();
    assert_eq!(
        other_bytes, source_other_bytes,
        "extensible-other wav must be copied verbatim, not rewritten"
    );
}

#[test]
fn export_to_usb_preserves_flac_wav_and_aif_media_paths_in_edb_and_pdb() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    copy_audio_fixture(
        &media,
        "formats/track_format_flac.flac",
        "Format Artist - Format One.flac",
    );
    copy_audio_fixture(
        &media,
        "formats/track_format_wav.wav",
        "Format Artist - Format Two.wav",
    );
    copy_audio_fixture(
        &media,
        "formats/track_format_aif.aif",
        "Format Artist - Format Three.aif",
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
            query: "format".to_string(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    let track_ids = tracks
        .iter()
        .map(|track| track.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(track_ids.len(), 3, "expected three format tracks");
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Format Path Coverage".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids,
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add tracks failed: {added:?}");

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");

    let conn = open_edb(
        &usb.join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db"),
    );
    let mut stmt = conn
        .prepare("SELECT path FROM content ORDER BY path")
        .expect("prepare content path query");
    let edb_paths = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query content paths")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect content paths")
        .into_iter()
        .map(|path| path.to_lowercase())
        .collect::<Vec<_>>();

    let parsed = parse_pdb(
        &usb.join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb"),
    )
    .expect("parse PDB");
    let pdb_paths = parsed
        .tracks
        .iter()
        .map(|row| row.track_file_path.to_lowercase())
        .collect::<Vec<_>>();

    for suffix in [
        "/contents/format artist/media/format artist - format one.flac",
        "/contents/format artist/media/format artist - format two.wav",
        "/contents/format artist/media/format artist - format three.aif",
    ] {
        assert!(
            edb_paths.iter().any(|path| path == suffix),
            "eDB should preserve media path suffix {suffix}: {edb_paths:?}"
        );
        assert!(
            pdb_paths.iter().any(|path| path == suffix),
            "PDB should preserve media path suffix {suffix}: {pdb_paths:?}"
        );
    }
}

#[test]
fn export_to_usb_does_not_change_player_menu_in_edb_or_pdb() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");
    copy_audio_fixture(
        &media,
        "formats/track_format_wav.wav",
        "Menu Guard Artist - Menu Guard Track.wav",
    );

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");
    let menu_before = snapshot_usb_menu(&usb);
    assert_eq!(
        menu_before
            .edb_tables
            .get("menuItem")
            .expect("menuItem snapshot")
            .len(),
        27,
        "initialized eDB should carry the full menu catalog"
    );
    assert_eq!(
        menu_before
            .pdb_menu_pages
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![16, 17, 18],
        "initialized PDB should include all menu tables"
    );

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let tracks = backend
        .search_tracks(SearchTracksRequest {
            query: "menu guard".to_string(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    let track_ids = tracks
        .iter()
        .map(|track| track.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(track_ids.len(), 1, "expected one menu guard track");
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Menu Export Guard".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids,
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add tracks failed: {added:?}");

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");

    let menu_after = snapshot_usb_menu(&usb);
    assert_usb_menu_unchanged(&menu_before, &menu_after, "fresh initialized USB");
}

#[test]
fn export_to_usb_many_playlists_pdb_growth_does_not_change_player_menu_in_edb_or_pdb() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    // 30 tracks: 10 copies of each format under distinct names so every track
    // gets a unique path and the PDB grows across multiple pages.
    for i in 0..10usize {
        copy_audio_fixture(
            &media,
            "formats/track_format_wav.wav",
            &format!("Growth Artist {i} - Growth Track {i}.wav"),
        );
        copy_audio_fixture(
            &media,
            "formats/track_format_flac.flac",
            &format!("Growth Artist {i} - Growth Track {i}.flac"),
        );
        copy_audio_fixture(
            &media,
            "formats/track_format_aif.aif",
            &format!("Growth Artist {i} - Growth Track {i}.aif"),
        );
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

    let all_tracks = backend
        .search_tracks(SearchTracksRequest {
            query: "growth".to_string(),
            limit: 50,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    assert_eq!(all_tracks.len(), 30, "expected 30 growth tracks");
    let all_ids: Vec<_> = all_tracks.iter().map(|t| t.id.clone()).collect();
    seed_tracks_as_analyzed(&data_dir, &all_ids);

    // Build 10 playlists of 3 tracks each.
    let mut playlist_ids = Vec::with_capacity(10);
    for i in 0..10usize {
        let created = backend.create_playlist(CreatePlaylistRequest {
            name: format!("Growth Playlist {i}"),
        });
        assert!(created.ok, "create playlist {i} failed: {created:?}");
        let pid = created.data.expect("playlist data").playlist_id;
        let chunk: Vec<_> = all_ids[i * 3..(i + 1) * 3].to_vec();
        let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: pid.clone(),
            track_ids: chunk,
            dedupe: DedupeMode::Skip,
        });
        assert!(added.ok, "add tracks to playlist {i} failed: {added:?}");
        playlist_ids.push(pid);
    }

    // Export playlists 0–8, growing the PDB across many pages.
    for (i, pid) in playlist_ids[..9].iter().enumerate() {
        let exported = backend.export_to_usb(ExportToUsbRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
            playlist_id: pid.clone(),
            options: Some(ExportToUsbOptions {
                include_artwork: false,
                include_analysis: false,
                prune_stale: false,
                ..Default::default()
            }),
        });
        assert!(exported.ok, "export playlist {i} failed: {exported:?}");
    }

    // Snapshot after 9 exports — the PDB is now large with menu chains at high
    // page indices.
    let menu_before = snapshot_usb_menu(&usb);
    assert_eq!(
        menu_before
            .edb_tables
            .get("menuItem")
            .expect("menuItem snapshot")
            .len(),
        27,
        "eDB should carry the full menu catalog after 9 exports"
    );
    assert_eq!(
        menu_before
            .pdb_menu_pages
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![16, 17, 18],
        "PDB should include all menu tables after 9 exports"
    );

    // Export the 10th playlist — fresh write against a large PDB; must not
    // disturb the menu chains.
    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_ids[9].clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export playlist 9 failed: {exported:?}");

    let menu_after = snapshot_usb_menu(&usb);
    assert_usb_menu_unchanged(&menu_before, &menu_after, "many-playlist growth");
}

#[test]
fn export_to_usb_preserves_t07_playlist_tree_tombstone_page() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    copy_audio_fixture(
        &media,
        "formats/track_format_wav.wav",
        "Tombstone Artist - First.wav",
    );
    copy_audio_fixture(
        &media,
        "formats/track_format_aif.aif",
        "Tombstone Artist - Second.aif",
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
            query: "tombstone".to_string(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    assert_eq!(tracks.len(), 2, "expected two tombstone test tracks");
    let track_ids = tracks
        .iter()
        .map(|track| track.id.clone())
        .collect::<Vec<_>>();
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let created_first = backend.create_playlist(CreatePlaylistRequest {
        name: "Tombstone Seed".to_string(),
    });
    assert!(
        created_first.ok,
        "create first playlist failed: {created_first:?}"
    );
    let first_playlist_id = created_first.data.expect("first playlist data").playlist_id;
    let added_first = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: first_playlist_id.clone(),
        track_ids: vec![track_ids[0].clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(added_first.ok, "add first track failed: {added_first:?}");

    let options = Some(ExportToUsbOptions {
        include_artwork: false,
        include_analysis: false,
        prune_stale: false,
        ..Default::default()
    });

    let export_first = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: first_playlist_id,
        options: options.clone(),
    });
    assert!(export_first.ok, "first export failed: {export_first:?}");

    let pdb_path = usb
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let mut before_second = fs::read(&pdb_path).expect("read PDB after first export");
    let page_size = read_u32_le(&before_second, 0x04).expect("PDB page size") as usize;
    assert_eq!(page_size, 4096, "test expects 4096-byte PDB pages");
    let (_ec_before, _first_before, t07_last_before) =
        pdb_table_ptr(&before_second, 7).expect("t07 table pointer");
    let t07_last_off = t07_last_before as usize * page_size;
    let t07_next_before =
        read_u32_le(&before_second, t07_last_off + 0x0c).expect("t07 next before");
    let used_before =
        read_u16_le(&before_second, t07_last_off + 0x1e).expect("t07 used_s before") as usize;
    assert!(used_before > 0, "first export should create a t07 row");
    let tombstone_payload_before =
        before_second[t07_last_off + 40..t07_last_off + 40 + used_before].to_vec();

    let rowpf_off = t07_last_off + page_size - 4;
    let tranrf_off = t07_last_off + page_size - 2;
    assert_ne!(
        read_u16_le(&before_second, rowpf_off).unwrap() & 0x0001,
        0,
        "seed playlist row should be active before tombstone mutation"
    );

    // Simulate the tombstone shape seen on real sticks: row payload
    // remains on the page, rowpf marks it inactive, tranrf keeps history.
    before_second[t07_last_off + 0x18] = 1;
    before_second[t07_last_off + 0x19] = 0;
    before_second[t07_last_off + 0x1a] = 0;
    write_u16_le(&mut before_second, rowpf_off, 0x0000);
    write_u16_le(&mut before_second, tranrf_off, 0x0001);
    fs::write(&pdb_path, &before_second).expect("write mutated tombstone PDB");

    let created_second = backend.create_playlist(CreatePlaylistRequest {
        name: "Tombstone Append".to_string(),
    });
    assert!(
        created_second.ok,
        "create second playlist failed: {created_second:?}"
    );
    let second_playlist_id = created_second
        .data
        .expect("second playlist data")
        .playlist_id;
    let added_second = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: second_playlist_id.clone(),
        track_ids: vec![track_ids[1].clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(added_second.ok, "add second track failed: {added_second:?}");

    let export_second = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: second_playlist_id,
        options,
    });
    assert!(export_second.ok, "second export failed: {export_second:?}");

    let after_second = fs::read(&pdb_path).expect("read PDB after second export");
    let (_ec_after, _first_after, t07_last_after) =
        pdb_table_ptr(&after_second, 7).expect("t07 table pointer after");
    assert_eq!(
        t07_last_after, t07_last_before,
        "normal export must not move t07 tail when tombstone page has capacity"
    );
    assert_eq!(
        read_u32_le(&after_second, t07_last_off + 0x0c).expect("t07 next after"),
        t07_next_before,
        "normal export must not relink t07 tombstone tail page"
    );
    assert_eq!(
        &after_second[t07_last_off + 40..t07_last_off + 40 + used_before],
        tombstone_payload_before.as_slice(),
        "inactive t07 row payload must be preserved byte-for-byte"
    );

    let rowpf_after = read_u16_le(&after_second, rowpf_off).expect("rowpf after");
    let tranrf_after = read_u16_le(&after_second, tranrf_off).expect("tranrf after");
    assert_eq!(
        rowpf_after & 0x0001,
        0,
        "original tombstoned t07 row must remain inactive"
    );
    assert_ne!(
        tranrf_after & 0x0001,
        0,
        "original tombstoned t07 transaction bit must remain set"
    );
    assert_ne!(
        rowpf_after & !0x0001,
        0,
        "second export should append at least one active t07 row to the tombstone page"
    );
    assert_eq!(
        tranrf_after & rowpf_after,
        rowpf_after,
        "new active t07 rows should also be marked in tranrf"
    );

    let parsed = parse_pdb(&pdb_path).expect("parse PDB after tombstone append");
    assert!(
        parsed
            .playlist_tree
            .iter()
            .any(|row| !row.row_is_folder && row.name == "Tombstone Append"),
        "second playlist should remain visible after tombstone-preserving append"
    );
}

#[test]
fn export_edb_and_pdb_media_and_analysis_paths_match_exactly() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    copy_audio_fixture(
        &media,
        "formats/track_format_flac.flac",
        "Path Test Artist - Path Track.flac",
    );

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let tracks = backend
        .search_tracks(SearchTracksRequest {
            query: "path".to_string(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    assert_eq!(tracks.len(), 1, "expected one track");
    seed_tracks_as_analyzed(&data_dir, &[tracks[0].id.clone()]);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Path Parity".to_string(),
    });
    assert!(created.ok);
    let playlist_id = created.data.expect("playlist data").playlist_id;

    backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![tracks[0].id.clone()],
        dedupe: DedupeMode::Skip,
    });

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");

    // Read eDB paths
    let conn = open_edb(
        &usb.join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db"),
    );
    let (edb_media, edb_anlz): (String, String) = conn
        .query_row(
            "SELECT path, analysisDataFilePath FROM content LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read eDB content row");

    // Read PDB paths
    let parsed = parse_pdb(
        &usb.join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb"),
    )
    .expect("parse PDB");
    let pdb_track = parsed
        .tracks
        .iter()
        .find(|t| !t.track_file_path.is_empty())
        .expect("PDB should have a track with media path");

    // Media paths must match exactly between eDB and PDB
    assert_eq!(
        edb_media, pdb_track.track_file_path,
        "eDB and PDB media paths must be identical"
    );

    // Analysis paths must match exactly
    assert!(
        !edb_anlz.is_empty(),
        "eDB analysis path must not be empty after export with analysis"
    );
    assert_eq!(
        edb_anlz, pdb_track.anlz_path,
        "eDB and PDB analysis paths must be identical"
    );

    // Both paths must be USB-relative (start with /)
    assert!(
        edb_media.starts_with("/Contents/"),
        "eDB media path must be USB-relative: {edb_media}"
    );
    assert!(
        edb_anlz.starts_with("/PIONEER/USBANLZ/"),
        "eDB analysis path must be USB-relative: {edb_anlz}"
    );

    // PDB file_name field must match the filename component of the media path
    let expected_filename = Path::new(&edb_media)
        .file_name()
        .and_then(|s| s.to_str())
        .expect("media path should have a filename");
    assert_eq!(
        pdb_track.file_name.as_deref(),
        Some(expected_filename),
        "PDB file_name must match filename from media path"
    );
}

#[test]
fn export_edb_and_pdb_artwork_paths_both_resolve_when_artwork_enabled() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    // Use the folder fixture that includes cover.jpg
    let fixture_dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/folder");
    for entry in fs::read_dir(&fixture_dir).expect("read fixture dir") {
        let entry = entry.expect("dir entry");
        fs::copy(entry.path(), media.join(entry.file_name())).expect("copy fixture");
    }

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let tracks = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    assert!(!tracks.is_empty(), "should have scanned tracks");
    let track_ids = tracks
        .iter()
        .map(|track| track.id.clone())
        .collect::<Vec<_>>();
    seed_tracks_as_analyzed(&data_dir, &track_ids);
    let cover_path = media.join("cover.jpg");
    assert!(
        cover_path.is_file(),
        "expected fixture cover at {}",
        cover_path.display()
    );
    for track_id in &track_ids {
        seed_track_artwork_path(&data_dir, track_id, &cover_path);
    }

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Artwork Parity".to_string(),
    });
    assert!(created.ok);
    let playlist_id = created.data.expect("playlist data").playlist_id;

    backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: tracks.iter().map(|t| t.id.clone()).collect(),
        dedupe: DedupeMode::Skip,
    });

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: true,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");

    // Read eDB artwork path
    let conn = open_edb(
        &usb.join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db"),
    );
    let edb_artwork: Option<String> = conn
        .query_row(
            "SELECT i.path FROM content c JOIN image i ON c.imageFilePath_id = i.image_id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok();

    // Read PDB artwork path
    let parsed = parse_pdb(
        &usb.join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb"),
    )
    .expect("parse PDB");
    let pdb_track = parsed
        .tracks
        .iter()
        .find(|t| t.artwork_id != 0)
        .expect("PDB should have a track with artwork");
    let pdb_artwork = parsed.artworks.get(&pdb_track.artwork_id).cloned();

    // Both sides should have artwork when artwork was included
    if let Some(edb_art) = &edb_artwork {
        assert!(
            edb_art.starts_with("/PIONEER/"),
            "eDB artwork path must be USB-relative: {edb_art}"
        );
    }
    if let Some(pdb_art) = &pdb_artwork {
        assert!(
            pdb_art.starts_with("/PIONEER/"),
            "PDB artwork path must be USB-relative: {pdb_art}"
        );
    }
    // Both must resolve (not be empty) when artwork export was enabled
    assert!(
        edb_artwork.is_some() || pdb_artwork.is_some(),
        "at least one side should have artwork when cover.jpg exists in source"
    );
}

#[test]
fn export_to_usb_rejects_when_playlist_tracks_are_missing_analysis() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    copy_audio_fixture(
        &media,
        "formats/track_format_wav.wav",
        "Artist - Unanalyzed.wav",
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
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .map(|track| track.id)
        .collect::<Vec<_>>();

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Unanalyzed Export Block".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let add = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids,
        dedupe: DedupeMode::Skip,
    });
    assert!(add.ok, "add failed: {add:?}");

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(!exported.ok, "export should fail without prior analysis");
    let err = exported.error.expect("export error");
    assert!(matches!(err.code, ErrorCode::ValidationError));
    let details = err.details.expect("structured validation details");
    assert_eq!(details["validationType"], "missing_analysis");
    assert_eq!(
        details["requiredFields"],
        serde_json::json!(["waveform", "bpm", "duration"])
    );
    assert_eq!(details["missingTrackCount"], 1);
    assert_eq!(details["totalTrackCount"], 1);
}

#[test]
fn export_to_usb_skips_missing_source_files_and_reports_warning() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let keep_path = media.join("Artist 1 - Keep.mp3");
    let missing_path = media.join("Artist 2 - Missing.mp3");
    fs::write(&keep_path, b"audio-keep").expect("write keep track");
    fs::write(&missing_path, b"audio-missing").expect("write missing track");

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

    let search = backend.search_tracks(SearchTracksRequest {
        query: String::new(),
        limit: 20,
        cursor: None,
    });
    assert!(search.ok, "search failed: {search:?}");
    let items = search.data.expect("search data").items;
    assert_eq!(items.len(), 2, "expected two tracks after scan");
    let track_ids = items.into_iter().map(|item| item.id).collect::<Vec<_>>();

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Missing Source Coverage".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let add = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids,
        dedupe: DedupeMode::Skip,
    });
    assert!(add.ok, "add failed: {add:?}");
    assert_eq!(add.data.expect("add data").added, 2);
    let seeded_ids = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .map(|item| item.id)
        .collect::<Vec<_>>();
    seed_tracks_as_analyzed(&data_dir, &seeded_ids);

    fs::remove_file(&missing_path).expect("remove one source file before export");

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");
    let data = exported.data.expect("export data");
    assert_eq!(data.exported_tracks, 1, "expected one exported track");
    assert_eq!(data.skipped_tracks, 1, "expected one skipped track");
    assert!(
        data.warnings
            .iter()
            .any(|w| { w.source == "export" && w.level == "warn" && w.code == "export.warn" }),
        "expected structured export warning, got {:?}",
        data.warnings
    );
}

#[test]
fn export_import_add_roundtrip_for_noart_fixture_keeps_exact_track_without_key_or_artwork() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/noart/track_no_art.mp3");
    assert!(fixture.is_file(), "fixture missing: {}", fixture.display());
    let track_path = media.join("Fixture Artist - No Art.mp3");
    fs::copy(&fixture, &track_path).expect("copy noart fixture");

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

    let track = backend
        .search_tracks(SearchTracksRequest {
            query: "no art".to_string(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .next()
        .expect("scanned noart track");

    let db_path = data_dir.join("backend.db");
    let conn = rusqlite::Connection::open(&db_path).expect("open backend db");
    let noart_anlz = seed_test_analysis_bundle(&data_dir, "noart-fixture");
    conn.execute(
        "UPDATE tracks
         SET bpm = 126.0,
             tonality = NULL,
             duration_ms = 315000,
             artwork_path = NULL,
             waveform_peaks_path = ?1
         WHERE id = ?2",
        rusqlite::params![noart_anlz.to_string_lossy().to_string(), track.id],
    )
    .expect("seed analyzed noart fixture fields");
    drop(conn);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "No Art Roundtrip Source".to_string(),
    });
    assert!(created.ok, "create source playlist failed: {created:?}");
    let source_playlist_id = created.data.expect("source playlist data").playlist_id;

    let add = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: source_playlist_id.clone(),
        track_ids: vec![track.id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(add.ok, "add source track failed: {add:?}");
    assert_eq!(add.data.expect("add source data").added, 1);

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: source_playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");
    assert_eq!(exported.data.expect("export data").exported_tracks, 1);

    let usb_playlists = backend.fetch_usb_playlists(FetchUsbPlaylistsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(
        usb_playlists.ok,
        "fetch usb playlists failed: {usb_playlists:?}"
    );
    let usb_playlist = usb_playlists
        .data
        .expect("usb playlist data")
        .items
        .into_iter()
        .find(|p| p.name == "No Art Roundtrip Source")
        .expect("roundtrip usb playlist");
    let usb_track = usb_playlist
        .tracks
        .into_iter()
        .find(|t| t.title.contains("No Art"))
        .expect("roundtrip usb track");

    assert!(
        usb_track
            .local_track_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .is_some(),
        "expected materialized local track id on usb row: {:?}",
        usb_track
    );
    assert!(
        usb_track
            .key
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty(),
        "usb row should stay keyless: {:?}",
        usb_track
    );
    assert!(
        usb_track
            .artwork_path
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty(),
        "usb row should stay artwork-less: {:?}",
        usb_track
    );

    let target = backend.create_playlist(CreatePlaylistRequest {
        name: "No Art Roundtrip Target".to_string(),
    });
    assert!(target.ok, "create target playlist failed: {target:?}");
    let target_playlist_id = target.data.expect("target playlist data").playlist_id;

    let add_from_usb = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: target_playlist_id.clone(),
        track_ids: vec![
            usb_track
                .local_track_id
                .clone()
                .expect("usb local track id"),
        ],
        dedupe: DedupeMode::Skip,
    });
    assert!(add_from_usb.ok, "add from usb failed: {add_from_usb:?}");
    assert_eq!(add_from_usb.data.expect("add usb data").added, 1);

    let target_tracks = backend.get_playlist_tracks(GetPlaylistTracksRequest {
        playlist_id: target_playlist_id,
    });
    assert!(target_tracks.ok, "get target playlist tracks failed");
    let target_items = target_tracks.data.expect("target playlist data").items;
    assert_eq!(
        target_items.len(),
        1,
        "expected exactly one added target track"
    );
    let added = &target_items[0];
    assert!(
        added.title.contains("No Art"),
        "wrong track added to playlist: {:?}",
        added
    );
    assert!(
        added.file_path.contains("/Contents/"),
        "expected imported usb-backed local track path: {:?}",
        added
    );
    assert!(
        added.key.as_deref().map(str::trim).unwrap_or("").is_empty(),
        "added local playlist row should stay keyless: {:?}",
        added
    );
    assert!(
        added
            .artwork_path
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty(),
        "added local playlist row should stay artwork-less: {:?}",
        added
    );
}

#[test]
fn export_to_usb_option_matrix_controls_artwork_analysis_and_prune_behavior() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let cover_path = media.join("cover.jpg");
    copy_audio_fixture(
        &media,
        "formats/track_format_wav.wav",
        "Artist - Matrix.wav",
    );
    let cover_fixture = cover_fixture_path();
    assert!(
        cover_fixture.is_file(),
        "cover fixture missing: {}",
        cover_fixture.display()
    );
    fs::copy(&cover_fixture, &cover_path).expect("copy cover fixture");

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

    let track = backend
        .search_tracks(SearchTracksRequest {
            query: "matrix".to_string(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .next()
        .expect("scanned track");

    seed_tracks_as_analyzed(&data_dir, &[track.id.clone()]);
    seed_track_artwork_path(&data_dir, &track.id, &cover_path);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Option Matrix".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track.id],
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add tracks failed: {added:?}");
    let seeded_ids = backend
        .search_tracks(SearchTracksRequest {
            query: "freshinit".to_string(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .map(|item| item.id)
        .collect::<Vec<_>>();
    seed_tracks_as_analyzed(&data_dir, &seeded_ids);

    let export_with_assets = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: true,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(
        export_with_assets.ok,
        "first export failed: {export_with_assets:?}"
    );
    let first = export_with_assets.data.expect("first export data");
    assert!(
        first.exported_artworks >= 1,
        "expected artwork export when include_artwork=true"
    );
    assert!(
        first.exported_analysis_files >= 3,
        "expected analysis export when include_analysis=true"
    );

    let artwork_count_before = WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ARTWORK_DIR))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    let analysis_count_before = WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ANALYSIS_DIR))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    assert!(
        artwork_count_before >= 1,
        "expected artwork files after first export"
    );
    assert!(
        analysis_count_before >= 3,
        "expected analysis files after first export"
    );

    let export_without_assets = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: true,
            ..Default::default()
        }),
    });
    assert!(
        export_without_assets.ok,
        "second export failed: {export_without_assets:?}"
    );
    let second = export_without_assets.data.expect("second export data");
    assert_eq!(
        second.exported_artworks, 0,
        "expected no artwork export when include_artwork=false"
    );
    assert_eq!(
        second.exported_analysis_files, 0,
        "expected no analysis export when include_analysis=false"
    );
    assert!(
        second
            .warnings
            .iter()
            .any(|w| w.source == "export" && w.level == "info" && w.code == "export.info"),
        "expected structured prune/export info warning"
    );

    let artwork_count_after = WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ARTWORK_DIR))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    let analysis_count_after = WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ANALYSIS_DIR))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    assert_eq!(
        artwork_count_after, 0,
        "expected stale artwork to be pruned"
    );
    assert_eq!(
        analysis_count_after, 0,
        "expected stale analysis bundles to be pruned"
    );
}

#[test]
fn export_to_usb_reexport_is_idempotent_for_media_and_assets() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let cover_path = media.join("cover.jpg");
    copy_audio_fixture(
        &media,
        "formats/track_format_wav.wav",
        "Artist - Idempotent.wav",
    );
    let cover_fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/folder/cover.jpg");
    assert!(
        cover_fixture.is_file(),
        "cover fixture missing: {}",
        cover_fixture.display()
    );
    fs::copy(&cover_fixture, &cover_path).expect("copy cover fixture");

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

    let track = backend
        .search_tracks(SearchTracksRequest {
            query: "idempotent".to_string(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .next()
        .expect("scanned track");

    seed_tracks_as_analyzed(&data_dir, &[track.id.clone()]);
    seed_track_artwork_path(&data_dir, &track.id, &cover_path);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Idempotency".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track.id],
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add tracks failed: {added:?}");

    let options = Some(ExportToUsbOptions {
        include_artwork: true,
        include_analysis: true,
        prune_stale: false,
        ..Default::default()
    });

    let first = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: options.clone(),
    });
    assert!(first.ok, "first export failed: {first:?}");

    let media_count_after_first = WalkDir::new(usb.join("Contents"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    let artwork_count_after_first =
        WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ARTWORK_DIR))
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .count();
    let analysis_count_after_first =
        WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ANALYSIS_DIR))
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .count();

    assert_eq!(media_count_after_first, 1, "expected one media file");
    assert!(
        artwork_count_after_first >= 1,
        "expected artwork after first export"
    );
    assert!(
        analysis_count_after_first >= 3,
        "expected analysis files after first export"
    );

    let second = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options,
    });
    assert!(second.ok, "second export failed: {second:?}");

    let media_count_after_second = WalkDir::new(usb.join("Contents"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    let artwork_count_after_second =
        WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ARTWORK_DIR))
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .count();
    let analysis_count_after_second =
        WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ANALYSIS_DIR))
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .count();

    assert_eq!(
        media_count_after_second, media_count_after_first,
        "re-export should not duplicate media files"
    );
    assert_eq!(
        artwork_count_after_second, artwork_count_after_first,
        "re-export should not duplicate artwork files"
    );
    assert_eq!(
        analysis_count_after_second, analysis_count_after_first,
        "re-export should not duplicate analysis files"
    );
}

#[test]
fn export_to_usb_reuses_existing_usb_assets_for_usb_origin_tracks() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb dir");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let usb_track_path = usb.join("Contents/USB Artist/USB Album/usb-origin.aif");
    let usb_art_small = usb.join("PIONEER/Artwork/00001/a00001.jpg");
    let usb_art_medium = usb.join("PIONEER/Artwork/00001/a00001_m.jpg");
    let usb_track_relative = "/Contents/USB Artist/USB Album/usb-origin.aif";
    let [usb_anlz_dat, usb_anlz_ext, usb_anlz_2ex] =
        canonical_usb_analysis_bundle_paths(&usb, usb_track_relative);

    fs::create_dir_all(usb_track_path.parent().expect("track parent")).expect("create track dir");
    fs::create_dir_all(usb_art_small.parent().expect("art parent")).expect("create art dir");
    fs::create_dir_all(usb_anlz_dat.parent().expect("anlz parent")).expect("create anlz dir");

    let usb_media_parent = usb_track_path.parent().expect("track parent");
    copy_audio_fixture(
        usb_media_parent,
        "formats/track_format_aif.aif",
        "usb-origin.aif",
    );
    let cover_fixture = cover_fixture_path();
    fs::copy(&cover_fixture, &usb_art_small).expect("seed small artwork");
    fs::copy(&cover_fixture, &usb_art_medium).expect("seed medium artwork");
    fs::write(&usb_anlz_dat, b"dat").expect("seed DAT");
    fs::write(&usb_anlz_ext, b"ext").expect("seed EXT");
    fs::write(&usb_anlz_2ex, b"2ex").expect("seed 2EX");

    let materialized = backend.materialize_source_track(MaterializeSourceTrackRequest {
        file_path: usb_track_path.to_string_lossy().to_string(),
        title: "USB Origin".to_string(),
        artist: "USB Artist".to_string(),
        album: Some("USB Album".to_string()),
        track_number: Some(1),
        key: None,
        file_size_bytes: None,
        format_ext: Some("aif".to_string()),
        sample_rate_hz: None,
        bit_depth: None,
        bitrate_kbps: None,
    });
    assert!(materialized.ok, "materialize failed: {materialized:?}");
    let track_id = materialized.data.expect("materialize data").track_id;

    let conn = rusqlite::Connection::open(data_dir.join("backend.db")).expect("open backend db");
    conn.execute(
        r#"
        UPDATE tracks
        SET bpm = 127.0,
            duration_ms = 248000,
            artwork_path = ?1,
            waveform_peaks_path = ?2
        WHERE id = ?3
        "#,
        rusqlite::params![
            usb_art_small.to_string_lossy().to_string(),
            usb_anlz_dat.to_string_lossy().to_string(),
            track_id
        ],
    )
    .expect("seed usb-origin analysis/artwork");

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "USB Origin Reuse".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_id],
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add failed: {added:?}");

    let contents_before = WalkDir::new(usb.join("Contents"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    let artwork_before = WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ARTWORK_DIR))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    let analysis_before = WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ANALYSIS_DIR))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: true,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");
    let data = exported.data.expect("export data");
    assert_eq!(data.exported_artworks, 0, "USB-origin art should be reused");
    assert_eq!(
        data.exported_analysis_files, 0,
        "USB-origin analysis bundle should be reused"
    );

    let contents_after = WalkDir::new(usb.join("Contents"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    let artwork_after = WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ARTWORK_DIR))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    let analysis_after = WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ANALYSIS_DIR))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();

    assert_eq!(
        contents_after, contents_before,
        "should not create new media"
    );
    assert_eq!(
        artwork_after, artwork_before,
        "should not create new artwork"
    );
    assert_eq!(
        analysis_after, analysis_before,
        "should not create new analysis bundle files"
    );

    let generated_art_paths = WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ARTWORK_DIR))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            e.path()
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .filter(|name| name.starts_with('b'))
        .collect::<Vec<_>>();
    assert!(
        generated_art_paths.is_empty(),
        "USB-origin re-export should not generate app-owned b*.jpg artwork files: {generated_art_paths:?}"
    );
}

#[test]
fn export_to_usb_does_not_use_post_write_parity_merge() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb dir");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");
    copy_audio_fixture(&media, "formats/track_format_flac.flac", "Artist - A.flac");
    copy_audio_fixture(&media, "formats/track_format_wav.wav", "Artist - B.wav");
    copy_audio_fixture(&media, "formats/track_format_aif.aif", "Artist - C.aif");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");
    let items = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    let track_ids = items.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Testi".to_string(),
    });
    let playlist_id = created.data.expect("playlist data").playlist_id;
    let add = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: track_ids.clone(),
        dedupe: DedupeMode::Skip,
    });
    assert!(add.ok, "add failed: {add:?}");

    let first = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(first.ok, "first export failed: {first:?}");

    let vendor_db = usb
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("exportLibrary.db");
    let conn = open_edb(&vendor_db);
    conn.execute(
        "INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (100, 'Testi', 0, 100)",
        [],
    )
    .expect("insert duplicate same-name playlist");
    let content_c: i64 = conn
        .query_row(
            "SELECT content_id FROM content WHERE path = '/Contents/Artist/media/Artist - C.aif' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("content c");
    conn.execute(
        "INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (100, ?1, 1)",
        [content_c],
    )
    .expect("link duplicate row");
    conn.execute(
        "DELETE FROM playlist_content WHERE playlist_id = 1 AND content_id = ?1",
        [content_c],
    )
    .expect("remove one row from primary db playlist");
    drop(conn);

    let second = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(second.ok, "second export failed: {second:?}");
    let warnings = second.data.expect("second data").warnings;
    assert!(
        warnings
            .iter()
            .all(|w| !w.message.starts_with("parity merge")),
        "normal export should not perform post-write parity reconciliation: {warnings:?}"
    );

    let imported = backend.fetch_usb_playlists(FetchUsbPlaylistsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(imported.ok, "usb import failed: {imported:?}");
    let testi = imported
        .data
        .expect("usb import data")
        .items
        .into_iter()
        .find(|pl| pl.name == "Testi")
        .expect("Testi playlist");
    assert_eq!(
        testi.track_count, 3,
        "post-export import should show merged parity"
    );
}

#[test]
fn first_export_after_initialize_usb_writes_export_pdb_without_skip_warning() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    copy_audio_fixture(
        &media,
        "formats/track_format_wav.wav",
        "Artist - FreshInit.wav",
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

    let track = backend
        .search_tracks(SearchTracksRequest {
            query: "freshinit".to_string(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .next()
        .expect("scanned track");
    let track_id = track.id.clone();

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Fresh Init Export".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add tracks failed: {added:?}");
    seed_tracks_as_analyzed(&data_dir, &[track_id]);

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");
    let data = exported.data.expect("export data");
    assert_eq!(data.exported_tracks, 1);
    assert!(
        !data
            .warnings
            .iter()
            .any(|w| w.code == "export.warn" && w.source == "export"),
        "expected seeded PDB to be appendable, warnings: {:?}",
        data.warnings
    );
}

#[test]
fn repeated_same_playlist_export_keeps_pdb_and_db_membership_stable() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    copy_audio_fixture(
        &media,
        "formats/track_format_wav.wav",
        "Artist - StableOne.wav",
    );
    copy_audio_fixture(
        &media,
        "formats/track_format_aif.aif",
        "Artist - StableTwo.aif",
    );

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");
    seed_edb_for_export(
        &usb.join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db"),
    );

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track_ids = backend
        .search_tracks(SearchTracksRequest {
            query: "stable".to_string(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .map(|t| t.id)
        .collect::<Vec<_>>();
    assert_eq!(track_ids.len(), 2, "expected two scanned stable tracks");

    let playlist_name = "Stable Export";
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
    assert!(added.ok, "add tracks failed: {added:?}");
    assert_eq!(added.data.expect("add data").added, 2);
    let seeded_ids = backend
        .search_tracks(SearchTracksRequest {
            query: "stable".to_string(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .map(|item| item.id)
        .collect::<Vec<_>>();
    seed_tracks_as_analyzed(&data_dir, &seeded_ids);

    let options = Some(ExportToUsbOptions {
        include_artwork: false,
        include_analysis: false,
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

    let parsed = parse_pdb(
        &usb.join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb"),
    )
    .expect("parse PDB after repeated export");
    let leaf_playlists = parsed
        .playlist_tree
        .iter()
        .filter(|row| !row.row_is_folder && row.name == playlist_name)
        .collect::<Vec<_>>();
    assert_eq!(
        leaf_playlists.len(),
        1,
        "expected a single same-name playlist row in PDB"
    );
    let playlist_pdb_id = leaf_playlists[0].id;
    let entry_count = parsed
        .playlist_entries
        .iter()
        .filter(|entry| entry.playlist_id == playlist_pdb_id)
        .count();
    assert_eq!(
        entry_count, 2,
        "expected stable playlist entry count in PDB"
    );

    let conn = open_edb(
        &usb.join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db"),
    );
    let db_playlist_count: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM playlist WHERE name = ?1",
            [playlist_name],
            |row| row.get(0),
        )
        .expect("count playlist rows by name");
    assert_eq!(
        db_playlist_count, 1,
        "expected a single same-name playlist row in eDB"
    );
    let db_entry_count: i64 = conn
        .query_row(
            r#"
            SELECT COUNT(1)
            FROM playlist_content pc
            JOIN playlist p ON p.playlist_id = pc.playlist_id
            WHERE p.name = ?1
            "#,
            [playlist_name],
            |row| row.get(0),
        )
        .expect("count playlist_content rows by playlist name");
    assert_eq!(
        db_entry_count, 2,
        "expected stable playlist entry count in eDB"
    );
}

#[test]
fn export_sync_mode_additive_preserves_existing_playlist_entries() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    copy_audio_fixture(&media, "formats/track_format_wav.wav", "Alpha.wav");
    copy_audio_fixture(&media, "formats/track_format_aif.aif", "Beta.aif");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");
    seed_edb_for_export(
        &usb.join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db"),
    );

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
    assert_eq!(track_ids.len(), 2, "expected two scanned tracks");
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let playlist_name = "Sync Mode Coverage";
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
    assert!(added.ok, "add tracks failed: {added:?}");
    assert_eq!(added.data.expect("add data").added, 2);

    let seed_export = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(seed_export.ok, "seed export failed: {seed_export:?}");
    let seed_paths = usb_playlist_track_paths(&usb, playlist_name);
    assert_eq!(
        seed_paths.len(),
        2,
        "seed export should write both playlist entries"
    );

    let removed = backend.remove_tracks_from_playlist(RemoveTracksFromPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_ids[0].clone()],
    });
    assert!(removed.ok, "remove failed: {removed:?}");
    assert_eq!(removed.data.expect("remove data").removed, 1);

    let additive = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(additive.ok, "additive export failed: {additive:?}");
    let additive_paths = usb_playlist_track_paths(&usb, playlist_name);
    assert_eq!(
        additive_paths.len(),
        2,
        "additive mode should preserve existing playlist entries"
    );
    assert!(
        additive_paths.iter().any(|p| p.contains("alpha.wav")),
        "alpha should still exist after additive export"
    );
    assert!(
        additive_paths.iter().any(|p| p.contains("beta.aif")),
        "beta should exist after additive export"
    );

    let mirror = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: true,
            ..Default::default()
        }),
    });
    assert!(mirror.ok, "mirror export failed: {mirror:?}");
    let mirror_paths = usb_playlist_track_paths(&usb, playlist_name);
    assert_eq!(
        mirror_paths.len(),
        1,
        "mirror mode should rewrite playlist to exact local set"
    );
    assert!(
        mirror_paths[0].contains("beta.aif"),
        "only beta should remain after mirror export"
    );
}

#[test]
fn export_sync_mode_mirror_new_playlist_does_not_prune_existing_playlists() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    copy_audio_fixture(&media, "formats/track_format_wav.wav", "Alpha.wav");
    copy_audio_fixture(&media, "formats/track_format_aif.aif", "Beta.aif");
    copy_audio_fixture(&media, "formats/track_format_wav.wav", "Gamma.wav");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");
    seed_edb_for_export(
        &usb.join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db"),
    );

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
    assert_eq!(track_ids.len(), 3, "expected three scanned tracks");
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let playlist_a_name = "Playlist A";
    let created_a = backend.create_playlist(CreatePlaylistRequest {
        name: playlist_a_name.to_string(),
    });
    assert!(created_a.ok, "create A failed: {created_a:?}");
    let playlist_a_id = created_a.data.expect("playlist A data").playlist_id;
    let added_a = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_a_id.clone(),
        track_ids: vec![track_ids[0].clone(), track_ids[1].clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(added_a.ok, "add A failed: {added_a:?}");
    assert_eq!(added_a.data.expect("add A data").added, 2);

    let export_a = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_a_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export_a.ok, "export A failed: {export_a:?}");

    let playlist_a_paths_before = usb_playlist_track_paths(&usb, playlist_a_name);
    assert_eq!(playlist_a_paths_before.len(), 2);
    let pdb_path = usb
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let pdb_size_before = fs::metadata(&pdb_path).expect("stat pdb before").len();

    let playlist_b_name = "Playlist B";
    let created_b = backend.create_playlist(CreatePlaylistRequest {
        name: playlist_b_name.to_string(),
    });
    assert!(created_b.ok, "create B failed: {created_b:?}");
    let playlist_b_id = created_b.data.expect("playlist B data").playlist_id;
    let added_b = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_b_id.clone(),
        track_ids: vec![track_ids[2].clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(added_b.ok, "add B failed: {added_b:?}");
    assert_eq!(added_b.data.expect("add B data").added, 1);

    // Mirror export on a brand-new playlist must not prune unrelated playlists.
    let export_b_mirror = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_b_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: true,
            ..Default::default()
        }),
    });
    assert!(
        export_b_mirror.ok,
        "mirror export for new playlist failed: {export_b_mirror:?}"
    );

    let playlist_a_paths_after = usb_playlist_track_paths(&usb, playlist_a_name);
    assert_eq!(
        playlist_a_paths_after.len(),
        playlist_a_paths_before.len(),
        "mirror export on new playlist should not prune existing playlist A entries"
    );
    let playlist_b_paths_after = usb_playlist_track_paths(&usb, playlist_b_name);
    assert_eq!(playlist_b_paths_after.len(), 1);
    let pdb_size_after = fs::metadata(&pdb_path).expect("stat pdb after").len();
    assert!(
        pdb_size_after >= pdb_size_before,
        "mirror export on new playlist should not shrink PDB (before={pdb_size_before}, after={pdb_size_after})"
    );
}

fn usb_playlist_track_paths(usb_root: &Path, playlist_name: &str) -> Vec<String> {
    let parsed = parse_pdb(
        &usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb"),
    )
    .expect("parse PDB");

    let playlist_ids = parsed
        .playlist_tree
        .iter()
        .filter(|row| !row.row_is_folder && row.name == playlist_name)
        .map(|row| row.id)
        .collect::<Vec<_>>();
    let by_track_id = parsed
        .tracks
        .iter()
        .map(|row| (row.id, row.track_file_path.to_lowercase()))
        .collect::<std::collections::HashMap<_, _>>();
    let mut out = parsed
        .playlist_entries
        .iter()
        .filter(|entry| playlist_ids.contains(&entry.playlist_id))
        .filter_map(|entry| by_track_id.get(&entry.track_id).cloned())
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

fn seed_edb_for_export(db_path: &std::path::Path) {
    std::fs::remove_file(db_path).ok();
    let artwork_path = format!("/{USB_VENDOR_ROOT_DIR}/{USB_ARTWORK_DIR}/template.jpg");
    let conn = open_edb(db_path);
    conn.execute_batch(
        &format!(
            r#"
        CREATE TABLE IF NOT EXISTS playlist (
          playlist_id INTEGER PRIMARY KEY,
          name TEXT,
          attribute INTEGER,
          sequenceNo INTEGER
        );
        CREATE TABLE IF NOT EXISTS content (
          content_id INTEGER PRIMARY KEY,
          title TEXT,
          path TEXT,
          analysisDataFilePath TEXT,
          bpmx100 INTEGER,
          key_id INTEGER,
          image_id INTEGER
        );
        CREATE TABLE IF NOT EXISTS image (
          image_id INTEGER PRIMARY KEY,
          path TEXT
        );
        CREATE TABLE IF NOT EXISTS "key" (
          key_id INTEGER PRIMARY KEY,
          name TEXT
        );
        CREATE TABLE IF NOT EXISTS playlist_content (
          playlist_id INTEGER,
          content_id INTEGER,
          sequenceNo INTEGER
        );
        INSERT OR IGNORE INTO playlist (playlist_id, name, attribute, sequenceNo)
          VALUES (1, 'Template Playlist', 0, 1);
        INSERT OR IGNORE INTO content (content_id, title, path, analysisDataFilePath, bpmx100, key_id, image_id)
          VALUES (1, 'Template Content', '/Contents/template.mp3', NULL, 0, NULL, NULL);
        INSERT OR IGNORE INTO image (image_id, path)
          VALUES (1, '{artwork_path}');
        INSERT OR IGNORE INTO "key" (key_id, name)
          VALUES (1, '8A');
        "#,
        ),
    )
    .expect("seed export db");
}

#[test]
fn export_backup_creates_timestamped_files_when_enabled() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    copy_audio_fixture(&media, "formats/track_format_flac.flac", "Backup Test.flac");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");
    let tracks = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    let track_ids: Vec<_> = tracks.iter().map(|t| t.id.clone()).collect();
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Backup Test".to_string(),
    });
    let playlist_id = created.data.expect("playlist data").playlist_id;
    backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: track_ids.clone(),
        dedupe: DedupeMode::Skip,
    });

    // First export — establishes PDB and eDB on USB.
    let first = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            backup_before_export: false,
        }),
    });
    assert!(first.ok, "first export failed: {first:?}");

    let pdb_path = usb
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let edb_path = usb
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("exportLibrary.db");
    assert!(pdb_path.is_file(), "PDB must exist after first export");
    assert!(edb_path.is_file(), "eDB must exist after first export");

    // Second export — backup should be created.
    let second = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            backup_before_export: true,
        }),
    });
    assert!(second.ok, "second export (with backup) failed: {second:?}");

    let backup_dir = usb
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("backups");
    assert!(backup_dir.is_dir(), "backups/ directory must be created");

    let backup_entries: Vec<_> = fs::read_dir(&backup_dir)
        .expect("read backups dir")
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(
        backup_entries.len(),
        2,
        "expected exactly two backup files (PDB and eDB)"
    );

    let names: std::collections::HashSet<String> = backup_entries
        .iter()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        names
            .iter()
            .any(|n| n.starts_with("export_") && n.ends_with(".pdb")),
        "expected a backup file matching export_*.pdb, got: {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|n| n.starts_with("exportLibrary_") && n.ends_with(".db")),
        "expected a backup file matching exportLibrary_*.db, got: {names:?}"
    );

    for entry in &backup_entries {
        let size = entry.metadata().expect("backup file metadata").len();
        assert!(
            size > 0,
            "backup file must be non-empty: {:?}",
            entry.file_name()
        );
    }

    // Warnings should mention the backup files.
    let warnings: Vec<_> = second
        .data
        .expect("export data")
        .warnings
        .iter()
        .map(|w| w.message.clone())
        .collect();
    assert!(
        warnings.iter().any(|w| w.starts_with("Backup:")),
        "expected at least one Backup: warning entry, got: {warnings:?}"
    );
}

#[test]
fn export_backup_skipped_when_disabled() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    copy_audio_fixture(
        &media,
        "formats/track_format_flac.flac",
        "No Backup Test.flac",
    );

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");
    let tracks = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    let track_ids: Vec<_> = tracks.iter().map(|t| t.id.clone()).collect();
    seed_tracks_as_analyzed(&data_dir, &track_ids);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "No Backup".to_string(),
    });
    let playlist_id = created.data.expect("playlist data").playlist_id;
    backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids,
        dedupe: DedupeMode::Skip,
    });

    // First export to establish databases.
    backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            backup_before_export: false,
        }),
    });

    // Second export with backup disabled.
    let result = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            backup_before_export: false,
        }),
    });
    assert!(result.ok, "export failed: {result:?}");

    let backup_dir = usb
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("backups");
    assert!(
        !backup_dir.exists(),
        "backups/ directory must not be created when backup is disabled"
    );
}
