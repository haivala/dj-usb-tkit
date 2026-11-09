//! A USB with one exported playlist that passes strict parity, built through the app's
//! commands. Needs `mod common;` and `mod library_sources;` in the test crate.

use std::fs;
use std::path::{Path, PathBuf};

use backend::commands::BackendCommands;
use backend::models::{
    AddLibrarySelectionToPlaylistRequest, BrowseSourceFilesRequest, CreatePlaylistRequest,
    ExportToUsbOptions, ExportToUsbRequest, InitializeUsbRequest, ScanLibraryRequest,
};
use tempfile::{TempDir, tempdir};

use crate::{common, library_sources};

fn seed_test_analysis_bundle(data_dir: &Path, stem: &str) -> PathBuf {
    let dir = data_dir.join("analysis").join("waveforms");
    fs::create_dir_all(&dir).expect("create test analysis dir");
    let dat = dir.join(format!("{stem}.DAT"));
    fs::write(&dat, b"test-dat").expect("write test DAT");
    fs::write(dir.join(format!("{stem}.EXT")), b"test-ext").expect("write test EXT");
    fs::write(dir.join(format!("{stem}.2EX")), b"test-2ex").expect("write test 2EX");
    dat
}

pub fn seed_tracks_as_analyzed(data_dir: &Path, track_ids: &[String]) {
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

pub fn fixture_audio_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("audio")
        .join(relative)
}

pub fn copy_audio_fixture(media_dir: &Path, fixture_relative: &str, target_name: &str) {
    common::copy_unique_audio(
        &fixture_audio_path(fixture_relative),
        &media_dir.join(target_name),
    );
}

pub fn setup_clean_strict_parity_fixture_with_local_key(
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

    library_sources::show_only(
        &backend,
        vec![media.to_string_lossy().to_string()],
        false,
        false,
    );
    let track_ids = backend
        .browse_source_files(BrowseSourceFilesRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
            ..Default::default()
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
    library_sources::show_only(
        &backend,
        vec![media.to_string_lossy().to_string()],
        false,
        false,
    );
    let added = backend.add_library_selection_to_playlist(AddLibrarySelectionToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids,
        ..Default::default()
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

pub fn setup_clean_strict_parity_fixture() -> (TempDir, BackendCommands, PathBuf, String) {
    setup_clean_strict_parity_fixture_with_local_key(None)
}
