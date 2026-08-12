//! Functional coverage for `service::usb`: root validation, single-track
//! inspection, and history import. These paths don't need the essentia
//! runner (unlike `user_flow_functional.rs`'s end-to-end flow), so they run
//! unconditionally in CI and sandboxed environments alike.

use std::fs;
use std::path::{Path, PathBuf};

use backend::commands::BackendCommands;
use backend::error::ErrorCode;
use backend::models::{
    AddTracksToPlaylistRequest, CreatePlaylistRequest, DedupeMode, ExportToUsbOptions,
    ExportToUsbRequest, FetchUsbHistoriesRequest, InitializeUsbRequest, InspectUsbTrackItem,
    InspectUsbTrackRequest, InspectUsbTracksRequest, ScanLibraryRequest, SearchTracksRequest,
    ValidateUsbRootRequest,
};
use backend::pdb_reader::parse_pdb;
use tempfile::tempdir;

const USB_VENDOR_ROOT_DIR: &str = "PIONEER";
const USB_VENDOR_DB_DIR: &str = "rekordbox";

fn pdb_path(usb_root: &Path) -> PathBuf {
    usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb")
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

fn seed_test_analysis_bundle(data_dir: &Path, stem: &str) -> PathBuf {
    let dir = data_dir.join("analysis").join("waveforms");
    fs::create_dir_all(&dir).expect("create test analysis dir");
    let dat = dir.join(format!("{stem}.DAT"));
    let ext = dir.join(format!("{stem}.EXT"));
    let twoex = dir.join(format!("{stem}.2EX"));
    fs::write(&dat, b"test-dat").expect("write test DAT");
    fs::write(&ext, b"test-ext").expect("write test EXT");
    fs::write(&twoex, b"test-2ex").expect("write test 2EX");
    dat
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

/// Scans one fixture track into its own source dir, adds it to a fresh
/// playlist, and exports that playlist to `usb`. Returns the local track id
/// plus its scanned title/artist, for callers that need to cross-check
/// against what ends up on the USB.
fn scan_and_export_single_track(
    backend: &BackendCommands,
    data_dir: &Path,
    media: &Path,
    usb: &Path,
    fixture_relative: &str,
    target_name: &str,
    playlist_name: &str,
) -> (String, String, String) {
    fs::create_dir_all(media).expect("create media dir");
    copy_audio_fixture(media, fixture_relative, target_name);

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let scanned = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    let track = scanned
        .iter()
        .find(|t| t.file_path.contains(target_name))
        .unwrap_or_else(|| panic!("expected scanned track for {target_name}"));
    let track_id = track.id.clone();
    let title = track.title.clone();
    let artist = track.artist.clone();
    seed_tracks_as_analyzed(data_dir, std::slice::from_ref(&track_id));

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: playlist_name.to_string(),
    });
    assert!(created.ok, "create playlist failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_id.clone()],
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
    assert_eq!(exported.data.expect("export data").exported_tracks, 1);

    (track_id, title, artist)
}

/// Scans several fixture tracks into their own source dir, adds them all to
/// one fresh playlist (in the order given), and exports that playlist to
/// `usb` in a single export call. Returns (track_id, title, artist) per
/// track, in the same order as `files`.
fn scan_and_export_tracks(
    backend: &BackendCommands,
    data_dir: &Path,
    media: &Path,
    usb: &Path,
    files: &[(&str, &str)],
    playlist_name: &str,
) -> Vec<(String, String, String)> {
    fs::create_dir_all(media).expect("create media dir");
    for (fixture_relative, target_name) in files {
        copy_audio_fixture(media, fixture_relative, target_name);
    }

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let scanned = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 50,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;

    let mut track_ids = Vec::with_capacity(files.len());
    let mut result = Vec::with_capacity(files.len());
    for (_, target_name) in files {
        let track = scanned
            .iter()
            .find(|t| t.file_path.contains(target_name))
            .unwrap_or_else(|| panic!("expected scanned track for {target_name}"));
        track_ids.push(track.id.clone());
        result.push((track.id.clone(), track.title.clone(), track.artist.clone()));
    }
    seed_tracks_as_analyzed(data_dir, &track_ids);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: playlist_name.to_string(),
    });
    assert!(created.ok, "create playlist failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: track_ids.clone(),
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
    assert_eq!(
        exported.data.expect("export data").exported_tracks,
        files.len()
    );

    result
}

// --- validate_usb_root -------------------------------------------------

#[test]
fn validate_usb_root_reports_empty_path_as_invalid() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let result = backend.validate_usb_root(ValidateUsbRootRequest {
        path: "   ".to_string(),
    });
    assert!(result.ok, "validate should succeed even for an empty path");
    let data = result.data.expect("validate data");
    assert!(!data.valid);
    assert!(!data.has_write_access);
    assert_eq!(data.normalized_root, None);
    assert!(
        data.warnings
            .iter()
            .any(|w| w.code == "usb.import.path-empty"),
        "expected empty-path warning, got {:?}",
        data.warnings
    );
}

#[test]
fn validate_usb_root_reports_missing_path_as_invalid() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let missing = root.path().join("does-not-exist");

    let result = backend.validate_usb_root(ValidateUsbRootRequest {
        path: missing.to_string_lossy().to_string(),
    });
    assert!(result.ok);
    let data = result.data.expect("validate data");
    assert!(!data.valid);
    assert_eq!(data.normalized_root, None);
    assert!(
        data.warnings
            .iter()
            .any(|w| w.code == "usb.import.path-not-found"),
        "expected path-not-found warning, got {:?}",
        data.warnings
    );
}

#[test]
fn validate_usb_root_reports_uninitialized_directory_as_invalid() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create bare usb dir");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let result = backend.validate_usb_root(ValidateUsbRootRequest {
        path: usb.to_string_lossy().to_string(),
    });
    assert!(result.ok);
    let data = result.data.expect("validate data");
    assert!(
        !data.valid,
        "a bare directory without PIONEER/Contents should be invalid"
    );
    assert!(!data.has_vendor_root);
    assert!(!data.has_contents);
    assert!(!data.has_pdb);
    assert!(data.normalized_root.is_some());
    assert!(
        data.warnings
            .iter()
            .any(|w| w.code == "usb.import.missing-vendor-root")
    );
    assert!(
        data.warnings
            .iter()
            .any(|w| w.code == "usb.import.missing-contents")
    );
}

#[test]
fn validate_usb_root_reports_initialized_root_as_valid_with_pdb_and_edb() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb dir");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let result = backend.validate_usb_root(ValidateUsbRootRequest {
        path: usb.to_string_lossy().to_string(),
    });
    assert!(result.ok);
    let data = result.data.expect("validate data");
    assert!(data.valid, "freshly initialized USB root should be valid");
    assert!(data.has_vendor_root);
    assert!(data.has_contents);
    assert!(data.has_pdb, "initialize_usb should seed a full-shape PDB");
    assert!(data.has_edb, "initialize_usb should seed a full-shape eDB");
    assert!(data.has_write_access);
    assert!(
        data.warnings.is_empty(),
        "no warnings expected for a fully initialized root: {:?}",
        data.warnings
    );
}

// --- inspect_usb_track ---------------------------------------------------

#[test]
fn inspect_usb_track_finds_track_by_id_with_no_hints() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb dir");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let (_local_id, title, artist) = scan_and_export_single_track(
        &backend,
        &data_dir,
        &media,
        &usb,
        "formats/track_format_flac.flac",
        "Inspect Artist - Inspect Title.flac",
        "Inspect Playlist",
    );

    let parsed = parse_pdb(&pdb_path(&usb)).expect("parse exported pdb");
    let usb_track_id = parsed
        .tracks
        .first()
        .expect("expected exported track in pdb")
        .id;

    let inspected = backend.inspect_usb_track(InspectUsbTrackRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        track_id: usb_track_id.to_string(),
        file_path: None,
        title: None,
        artist: None,
    });
    assert!(inspected.ok, "inspect usb track failed: {inspected:?}");
    let data = inspected.data.expect("inspect data");
    assert_eq!(data.source, "pdb");
    assert_eq!(data.track.title, title);
    assert_eq!(data.track.artist, artist);
    assert!(data.track.usb_media_path.is_some());
}

#[test]
fn inspect_usb_track_matches_via_title_and_artist_hints() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb dir");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let (_local_id, title, artist) = scan_and_export_single_track(
        &backend,
        &data_dir,
        &media,
        &usb,
        "noart/track_no_art.mp3",
        "Hint Artist - Hint Title.mp3",
        "Hint Playlist",
    );

    let parsed = parse_pdb(&pdb_path(&usb)).expect("parse exported pdb");
    let usb_track_id = parsed
        .tracks
        .first()
        .expect("expected exported track in pdb")
        .id;

    let inspected = backend.inspect_usb_track(InspectUsbTrackRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        track_id: usb_track_id.to_string(),
        file_path: Some("Hint Artist - Hint Title.mp3".to_string()),
        title: Some(title),
        artist: Some(artist),
    });
    assert!(
        inspected.ok,
        "inspect usb track with matching hints failed: {inspected:?}"
    );
    let data = inspected.data.expect("inspect data");
    assert_eq!(data.source, "pdb");
}

#[test]
fn inspect_usb_track_falls_back_to_edb_when_title_and_artist_hints_reject_the_pdb_row() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb dir");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let (_local_id, title, artist) = scan_and_export_single_track(
        &backend,
        &data_dir,
        &media,
        &usb,
        "formats/track_format_flac.flac",
        "Fallback Artist - Fallback Title.flac",
        "Fallback Playlist",
    );

    let parsed = parse_pdb(&pdb_path(&usb)).expect("parse exported pdb");
    let usb_track_id = parsed
        .tracks
        .first()
        .expect("expected exported track in pdb")
        .id;

    // Title/artist hints disagree with the PDB row (score <= 0), but no
    // file_path hint is supplied -- the eDB fallback only checks file_hint,
    // so the lookup should still resolve the id via the eDB and return the
    // *real* title/artist rather than failing outright.
    let inspected = backend.inspect_usb_track(InspectUsbTrackRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        track_id: usb_track_id.to_string(),
        file_path: None,
        title: Some("Completely Different Title".to_string()),
        artist: Some("Completely Different Artist".to_string()),
    });
    assert!(
        inspected.ok,
        "expected eDB fallback to resolve despite mismatched PDB hints: {inspected:?}"
    );
    let data = inspected.data.expect("inspect data");
    assert_eq!(data.source, "eDB");
    assert_eq!(data.track.title, title);
    assert_eq!(data.track.artist, artist);
}

#[test]
fn inspect_usb_track_reports_not_found_when_hints_dont_match_pdb_or_edb() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb dir");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    scan_and_export_single_track(
        &backend,
        &data_dir,
        &media,
        &usb,
        "formats/track_format_flac.flac",
        "Mismatch Artist - Mismatch Title.flac",
        "Mismatch Playlist",
    );

    let parsed = parse_pdb(&pdb_path(&usb)).expect("parse exported pdb");
    let usb_track_id = parsed
        .tracks
        .first()
        .expect("expected exported track in pdb")
        .id;

    // The id is real, but every hint disagrees with both the PDB row and the
    // eDB fallback, so the lookup should exhaust both sources and fail
    // rather than return a mismatched track.
    let inspected = backend.inspect_usb_track(InspectUsbTrackRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        track_id: usb_track_id.to_string(),
        file_path: Some("completely/unrelated/path.flac".to_string()),
        title: Some("Completely Unrelated Title".to_string()),
        artist: Some("Completely Unrelated Artist".to_string()),
    });
    assert!(
        !inspected.ok,
        "expected inspect to fail when no hint matches: {inspected:?}"
    );
    let error = inspected.error.expect("inspect error");
    assert!(matches!(error.code, ErrorCode::ValidationError));
    assert!(error.message.contains("not found on USB metadata sources"));
}

#[test]
fn inspect_usb_track_rejects_non_numeric_track_id() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb dir");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let inspected = backend.inspect_usb_track(InspectUsbTrackRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        track_id: "not-a-number".to_string(),
        file_path: None,
        title: None,
        artist: None,
    });
    assert!(!inspected.ok, "expected non-numeric trackId to be rejected");
    let error = inspected.error.expect("inspect error");
    assert!(matches!(error.code, ErrorCode::ValidationError));
    assert!(error.message.contains("must be a numeric USB track id"));
}

#[test]
fn inspect_usb_track_reports_not_found_for_unknown_id() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb dir");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let inspected = backend.inspect_usb_track(InspectUsbTrackRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        track_id: "999999".to_string(),
        file_path: None,
        title: None,
        artist: None,
    });
    assert!(!inspected.ok, "expected unknown trackId to be rejected");
    let error = inspected.error.expect("inspect error");
    assert!(matches!(error.code, ErrorCode::ValidationError));
    assert!(error.message.contains("not found on USB metadata sources"));
}

// --- inspect_usb_tracks (batch) -------------------------------------------

#[test]
fn inspect_usb_tracks_resolves_multiple_real_exported_tracks_in_one_call() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb dir");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let exported = scan_and_export_tracks(
        &backend,
        &data_dir,
        &media,
        &usb,
        &[
            (
                "formats/track_format_flac.flac",
                "Batch Artist A - Song A.flac",
            ),
            ("noart/track_no_art.mp3", "Batch Artist B - Song B.mp3"),
            (
                "formats/track_format_wav.wav",
                "Batch Artist C - Song C.wav",
            ),
        ],
        "Batch Inspect Playlist",
    );

    let parsed = parse_pdb(&pdb_path(&usb)).expect("parse exported pdb");
    let mut items = Vec::new();
    let mut expected_by_id = std::collections::HashMap::new();
    for (target_name_hint, (_track_id, title, artist)) in [
        "Batch Artist A - Song A.flac",
        "Batch Artist B - Song B.mp3",
        "Batch Artist C - Song C.wav",
    ]
    .into_iter()
    .zip(exported.iter())
    {
        let pdb_track = parsed
            .tracks
            .iter()
            .find(|t| t.track_file_path.contains(target_name_hint))
            .unwrap_or_else(|| panic!("expected exported pdb row for {target_name_hint}"));
        let usb_track_id = pdb_track.id.to_string();
        expected_by_id.insert(usb_track_id.clone(), (title.clone(), artist.clone()));
        items.push(InspectUsbTrackItem {
            track_id: usb_track_id,
            file_path: None,
            title: None,
            artist: None,
        });
    }

    let batch = backend.inspect_usb_tracks(InspectUsbTracksRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        items,
    });
    assert!(batch.ok, "inspect usb tracks failed: {batch:?}");
    let data = batch.data.expect("inspect usb tracks data");
    assert_eq!(data.items.len(), 3);
    for result in &data.items {
        let (title, artist) = expected_by_id
            .get(&result.track_id)
            .unwrap_or_else(|| panic!("unexpected track id in batch result: {}", result.track_id));
        assert_eq!(result.source.as_deref(), Some("pdb"));
        let track = result
            .track
            .as_ref()
            .unwrap_or_else(|| panic!("expected resolved track for {}", result.track_id));
        assert_eq!(&track.title, title);
        assert_eq!(&track.artist, artist);
    }
}

#[test]
fn inspect_usb_tracks_mixes_resolved_and_unresolved_items_in_one_batch() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb dir");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    scan_and_export_tracks(
        &backend,
        &data_dir,
        &media,
        &usb,
        &[(
            "formats/track_format_flac.flac",
            "Mixed Batch Artist - Mixed Batch Song.flac",
        )],
        "Mixed Batch Playlist",
    );

    let parsed = parse_pdb(&pdb_path(&usb)).expect("parse exported pdb");
    let valid_id = parsed
        .tracks
        .first()
        .expect("expected exported track in pdb")
        .id
        .to_string();

    let batch = backend.inspect_usb_tracks(InspectUsbTracksRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        items: vec![
            InspectUsbTrackItem {
                track_id: "not-a-number".to_string(),
                file_path: None,
                title: None,
                artist: None,
            },
            InspectUsbTrackItem {
                track_id: valid_id.clone(),
                file_path: None,
                title: None,
                artist: None,
            },
            InspectUsbTrackItem {
                track_id: "999999".to_string(),
                file_path: None,
                title: None,
                artist: None,
            },
        ],
    });
    assert!(batch.ok, "inspect usb tracks failed: {batch:?}");
    let data = batch.data.expect("inspect usb tracks data");
    assert_eq!(
        data.items.len(),
        3,
        "expected one result per requested item"
    );

    let bad = data
        .items
        .iter()
        .find(|i| i.track_id == "not-a-number")
        .expect("non-numeric id result");
    assert!(bad.track.is_none());
    assert!(bad.source.is_none());

    let unknown = data
        .items
        .iter()
        .find(|i| i.track_id == "999999")
        .expect("unknown id result");
    assert!(unknown.track.is_none());
    assert!(unknown.source.is_none());

    let good = data
        .items
        .iter()
        .find(|i| i.track_id == valid_id)
        .expect("valid id result");
    assert!(good.track.is_some());
    assert_eq!(good.source.as_deref(), Some("pdb"));
}

// --- fetch_usb_histories --------------------------------------------------

#[test]
fn fetch_usb_histories_returns_empty_result_when_pdb_is_missing() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create bare usb dir");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let histories = backend.fetch_usb_histories(FetchUsbHistoriesRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(histories.ok, "fetch usb histories failed: {histories:?}");
    let data = histories.data.expect("history data");
    assert!(data.items.is_empty());
    assert_eq!(data.counts.imported_playlists, 0);
    assert_eq!(data.counts.imported_tracks, 0);
    assert!(
        data.warnings
            .iter()
            .any(|w| w.code == "usb.histories.pdb-not-found")
    );
}

#[test]
fn fetch_usb_histories_reads_injected_pdb_history_rows_and_materializes_tracks() {
    let root = tempdir().expect("temp root");
    let media_a = root.path().join("media-a");
    let media_b = root.path().join("media-b");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb dir");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    scan_and_export_single_track(
        &backend,
        &data_dir,
        &media_a,
        &usb,
        "formats/track_format_flac.flac",
        "History Artist - History Track A.flac",
        "History Source Playlist A",
    );
    scan_and_export_single_track(
        &backend,
        &data_dir,
        &media_b,
        &usb,
        "noart/track_no_art.mp3",
        "History Artist - History Track B.mp3",
        "History Source Playlist B",
    );

    let parsed = parse_pdb(&pdb_path(&usb)).expect("parse exported pdb");
    let history_track_ids = parsed.tracks.iter().map(|t| t.id).collect::<Vec<_>>();
    assert_eq!(history_track_ids.len(), 2, "need two exported track ids");
    append_history_to_pdb(&pdb_path(&usb), 1, &history_track_ids);

    let histories = backend.fetch_usb_histories(FetchUsbHistoriesRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(histories.ok, "fetch usb histories failed: {histories:?}");
    let data = histories.data.expect("history data");
    assert_eq!(data.counts.imported_playlists, 1);
    assert_eq!(data.counts.pdb_t11_playlists, 1);
    assert_eq!(data.counts.pdb_t12_entries, 2);
    assert_eq!(data.items.len(), 1, "expected the single injected history");
    let history = &data.items[0];
    assert_eq!(
        history.tracks.len(),
        2,
        "expected both tracks in history order"
    );
    assert!(
        history.tracks.iter().all(|t| t.local_track_id.is_some()),
        "history tracks should be materialized into the local library: {:?}",
        history.tracks
    );
}

fn append_history_to_pdb(pdb_path: &Path, history_id: u32, track_ids: &[u32]) {
    let mut bytes = fs::read(pdb_path).expect("read PDB");
    let len_page = read_u32_le(&bytes, 4).expect("len_page") as usize;
    let num_tables = read_u32_le(&bytes, 8).expect("num_tables") as usize;
    let max_page = parse_max_last_page(&bytes, num_tables) as u32;

    let history_playlist_page = max_page + 1;
    let history_entries_page = max_page + 2;

    let playlist_row = build_history_playlist_row(history_id, "HISTORY 1");
    let entry_rows = track_ids
        .iter()
        .enumerate()
        .map(|(idx, track_id)| build_history_entry_row(*track_id, history_id, (idx + 1) as u32))
        .collect::<Vec<_>>();

    let playlist_page = build_pdb_page(
        11,
        history_playlist_page,
        history_playlist_page,
        &[playlist_row],
        len_page,
    );
    let entries_page = build_pdb_page(
        12,
        history_entries_page,
        history_entries_page,
        &entry_rows,
        len_page,
    );

    bytes.extend_from_slice(&playlist_page);
    bytes.extend_from_slice(&entries_page);

    let p1 = 28 + num_tables * 16;
    let p2 = p1 + 16;
    bytes[p1..p1 + 4].copy_from_slice(&11u32.to_le_bytes());
    bytes[p1 + 8..p1 + 12].copy_from_slice(&history_playlist_page.to_le_bytes());
    bytes[p1 + 12..p1 + 16].copy_from_slice(&history_playlist_page.to_le_bytes());

    bytes[p2..p2 + 4].copy_from_slice(&12u32.to_le_bytes());
    bytes[p2 + 8..p2 + 12].copy_from_slice(&history_entries_page.to_le_bytes());
    bytes[p2 + 12..p2 + 16].copy_from_slice(&history_entries_page.to_le_bytes());

    bytes[8..12].copy_from_slice(&((num_tables as u32) + 2).to_le_bytes());
    fs::write(pdb_path, bytes).expect("write PDB with history tables");
}

fn build_history_playlist_row(id: u32, name: &str) -> Vec<u8> {
    let mut row = Vec::<u8>::new();
    row.extend_from_slice(&id.to_le_bytes());
    row.extend_from_slice(&encode_pdb_ascii_string(name));
    row
}

fn build_history_entry_row(track_id: u32, playlist_id: u32, entry_index: u32) -> Vec<u8> {
    let mut row = vec![0u8; 12];
    row[0..4].copy_from_slice(&track_id.to_le_bytes());
    row[4..8].copy_from_slice(&playlist_id.to_le_bytes());
    row[8..12].copy_from_slice(&entry_index.to_le_bytes());
    row
}

fn encode_pdb_ascii_string(value: &str) -> Vec<u8> {
    let bytes = value.as_bytes();
    let mut out = Vec::<u8>::with_capacity(4 + bytes.len());
    out.push(0u8);
    out.extend_from_slice(&(4u16 + bytes.len() as u16).to_le_bytes());
    out.push(0u8);
    out.extend_from_slice(bytes);
    out
}

fn build_pdb_page(
    table_type: u32,
    page_index: u32,
    seq: u32,
    rows: &[Vec<u8>],
    len_page: usize,
) -> Vec<u8> {
    let mut page = vec![0u8; len_page];
    page[4..8].copy_from_slice(&page_index.to_le_bytes());
    page[8..12].copy_from_slice(&table_type.to_le_bytes());
    page[12..16].copy_from_slice(&0u32.to_le_bytes());
    page[16..20].copy_from_slice(&seq.to_le_bytes());

    let mut payload_offset = 0usize;
    let mut row_offsets = Vec::<u16>::new();
    for row in rows {
        row_offsets.push(payload_offset as u16);
        let start = 40 + payload_offset;
        let end = start + row.len();
        page[start..end].copy_from_slice(row);
        payload_offset += row.len();
    }

    page[24] = (rows.len() % 256) as u8;
    page[30..32].copy_from_slice(&(payload_offset as u16).to_le_bytes());
    page[34..36].copy_from_slice(&((rows.len().saturating_sub(1)) as u16).to_le_bytes());

    let mut cursor = len_page;
    for group_start in (0..rows.len()).step_by(16) {
        cursor -= 4;
        let group_len = (rows.len() - group_start).min(16);
        let bits = ((1u32 << group_len) - 1) as u16;
        page[cursor..cursor + 2].copy_from_slice(&bits.to_le_bytes());
        for j in 0..group_len {
            cursor -= 2;
            page[cursor..cursor + 2].copy_from_slice(&row_offsets[group_start + j].to_le_bytes());
        }
    }

    page
}

fn parse_max_last_page(bytes: &[u8], num_tables: usize) -> usize {
    let mut cursor = 28usize;
    let mut max_page = 0usize;
    for _ in 0..num_tables {
        let Some(last_page) = read_u32_le(bytes, cursor + 12) else {
            break;
        };
        max_page = max_page.max(last_page as usize);
        cursor += 16;
    }
    max_page
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let a = *bytes.get(offset)?;
    let b = *bytes.get(offset + 1)?;
    let c = *bytes.get(offset + 2)?;
    let d = *bytes.get(offset + 3)?;
    Some(u32::from_le_bytes([a, b, c, d]))
}
