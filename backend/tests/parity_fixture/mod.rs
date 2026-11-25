//! A USB with one exported playlist that passes strict parity, built through the app's
//! commands. Needs `mod common;` and `mod library_sources;` in the test crate.
// Shared by several test crates: each uses only some of it.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use backend::commands::BackendCommands;
use backend::models::{
    AddLibrarySelectionToPlaylistRequest, BrowseSourceFilesRequest, CreatePlaylistRequest,
    ExportToUsbOptions, ExportToUsbRequest, InitializeUsbRequest, ScanLibraryRequest,
};
use tempfile::{TempDir, tempdir};

use crate::{common, library_sources};

pub fn seed_tracks_as_analyzed(data_dir: &Path, track_ids: &[String]) {
    let db_path = data_dir.join("backend.db");
    let conn = rusqlite::Connection::open(&db_path).expect("open backend db");
    for (idx, track_id) in track_ids.iter().enumerate() {
        let bundle = common::seed_test_analysis_bundle(data_dir, &format!("waveform-{idx}"));
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

pub use common::copy_audio_fixture;

pub fn setup_clean_strict_parity_fixture_with_local_key(
    local_key: Option<&str>,
) -> (TempDir, BackendCommands, PathBuf, String) {
    setup_clean_strict_parity_fixture_with(local_key, None)
}

/// The clean fixture with its track `duration_ms` long in the library, so the
/// export writes that length to the USB.
pub fn setup_clean_strict_parity_fixture_with_length(
    duration_ms: u64,
) -> (TempDir, BackendCommands, PathBuf, String) {
    setup_clean_strict_parity_fixture_with(None, Some(duration_ms))
}

fn setup_clean_strict_parity_fixture_with(
    local_key: Option<&str>,
    duration_ms: Option<u64>,
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
    if let Some(duration_ms) = duration_ms {
        let conn =
            rusqlite::Connection::open(data_dir.join("backend.db")).expect("open backend db");
        conn.execute(
            "UPDATE tracks SET duration_ms = ?1, sample_rate_hz = 44100",
            rusqlite::params![duration_ms as i64],
        )
        .expect("seed track length");
    }
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
        folder_id: None,
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").data.playlist_id;
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
    assert_eq!(added.data.expect("add data").data.added, 1);

    let export = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        }),
        sort: None,
    });
    assert!(export.ok, "export failed: {export:?}");

    (root, backend, usb, playlist_name)
}

pub fn setup_clean_strict_parity_fixture() -> (TempDir, BackendCommands, PathBuf, String) {
    setup_clean_strict_parity_fixture_with_local_key(None)
}
