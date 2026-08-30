use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use backend::commands::BackendCommands;
use backend::models::{
    AddTracksToPlaylistRequest, CheckSourceRootsRequest, CreatePlaylistRequest, DedupeMode,
    ExportToUsbRequest, FetchUsbHistoriesRequest, FetchUsbPlaylistsRequest,
    GetPlaylistTracksRequest, InitializeUsbRequest, ListTracksRequest, PlayResolvedTrackRequest,
    PlayTrackRequest, PlaybackPreflightRequest, PruneUsbDeviceRequest,
    RemoveTracksFromPlaylistRequest, RemoveUsbPlaylistRequest, RenamePlaylistRequest,
    ReorderPlaylistTracksRequest, ReorderUsbPlaylistsRequest, RunUsbDiagnosticsRequest,
    RunUsbParityReportRequest, ScanLibraryRequest, ScanMasterDbRequest, SearchTracksRequest,
    ValidateUsbRootRequest,
};
use backend::service::usb_vendor_compat::DEFAULT_USB_EDB_KEY;
use tempfile::tempdir;

fn vendor_db_dir(root: &Path) -> std::path::PathBuf {
    root.join("PIONEER").join("rekordbox")
}

#[test]
fn search_tracks_cursor_paginates_stably_and_rejects_query_mismatch_cursor() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");

    for name in [
        "Artist - 01.mp3",
        "Artist - 02.mp3",
        "Artist - 03.mp3",
        "Artist - 04.mp3",
        "Artist - 05.mp3",
    ] {
        fs::write(media.join(name), b"audio").expect("write fixture track");
    }

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let page1 = backend.search_tracks(SearchTracksRequest {
        query: String::new(),
        limit: 2,
        cursor: None,
    });
    assert!(page1.ok, "page1 failed: {page1:?}");
    let page1_data = page1.data.expect("page1 data");
    assert_eq!(page1_data.total, 5);
    assert_eq!(page1_data.items.len(), 2);
    assert!(page1_data.has_more);
    let page1_cursor = page1_data.next_cursor.clone().expect("page1 next cursor");

    let page2 = backend.search_tracks(SearchTracksRequest {
        query: String::new(),
        limit: 2,
        cursor: Some(page1_cursor.clone()),
    });
    assert!(page2.ok, "page2 failed: {page2:?}");
    let page2_data = page2.data.expect("page2 data");
    assert_eq!(page2_data.total, 5);
    assert_eq!(page2_data.items.len(), 2);
    assert!(page2_data.has_more);
    let page2_cursor = page2_data.next_cursor.clone().expect("page2 next cursor");

    let page3 = backend.search_tracks(SearchTracksRequest {
        query: String::new(),
        limit: 2,
        cursor: Some(page2_cursor),
    });
    assert!(page3.ok, "page3 failed: {page3:?}");
    let page3_data = page3.data.expect("page3 data");
    assert_eq!(page3_data.total, 5);
    assert_eq!(page3_data.items.len(), 1);
    assert!(!page3_data.has_more);

    let mut seen = HashSet::new();
    for item in page1_data
        .items
        .iter()
        .chain(page2_data.items.iter())
        .chain(page3_data.items.iter())
    {
        assert!(
            seen.insert(item.id.clone()),
            "duplicate track id across pages: {}",
            item.id
        );
    }
    assert_eq!(seen.len(), 5, "expected all tracks across 3 pages");

    let mismatch = backend.search_tracks(SearchTracksRequest {
        query: "Artist".to_string(),
        limit: 2,
        cursor: Some(page1_cursor),
    });
    assert!(!mismatch.ok, "expected cursor/query mismatch failure");
    let mismatch_error = mismatch.error.expect("mismatch error payload");
    assert!(
        mismatch_error
            .message
            .contains("cursor does not match current query"),
        "unexpected mismatch error: {mismatch_error:?}"
    );
}

#[test]
fn playlist_order_remains_stable_after_remove_and_readd() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");

    for name in ["Artist - A.mp3", "Artist - B.mp3", "Artist - C.mp3"] {
        fs::write(media.join(name), b"audio").expect("write fixture track");
    }

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let mut id_by_title = HashMap::new();
    for track in backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
    {
        id_by_title.insert(track.title.clone(), track.id);
    }

    let track_a = id_by_title.get("A").expect("track A id").clone();
    let track_b = id_by_title.get("B").expect("track B id").clone();
    let track_c = id_by_title.get("C").expect("track C id").clone();

    let playlist_id = backend
        .create_playlist(CreatePlaylistRequest {
            name: "Ordering Test".to_string(),
        })
        .data
        .expect("playlist data")
        .playlist_id;

    let add_initial = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_a.clone(), track_b.clone(), track_c.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(add_initial.ok, "initial add failed: {add_initial:?}");

    let remove_middle = backend.remove_tracks_from_playlist(RemoveTracksFromPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_b.clone()],
    });
    assert!(remove_middle.ok, "remove failed: {remove_middle:?}");
    assert_eq!(remove_middle.data.expect("remove data").removed, 1);

    let after_remove_titles = backend
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist_id.clone(),
            ..Default::default()
        })
        .data
        .expect("after remove data")
        .items
        .into_iter()
        .map(|t| t.title)
        .collect::<Vec<_>>();
    assert_eq!(after_remove_titles, vec!["A".to_string(), "C".to_string()]);

    let readd = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_b],
        dedupe: DedupeMode::Skip,
    });
    assert!(readd.ok, "re-add failed: {readd:?}");
    assert_eq!(readd.data.expect("re-add data").added, 1);

    let final_titles = backend
        .get_playlist_tracks(GetPlaylistTracksRequest { playlist_id, ..Default::default() })
        .data
        .expect("final tracks data")
        .items
        .into_iter()
        .map(|t| t.title)
        .collect::<Vec<_>>();
    assert_eq!(
        final_titles,
        vec!["A".to_string(), "C".to_string(), "B".to_string()]
    );
}

#[test]
fn reorder_playlist_tracks_persists_a_custom_order() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");

    for name in ["Artist - A.mp3", "Artist - B.mp3", "Artist - C.mp3"] {
        fs::write(media.join(name), b"audio").expect("write fixture track");
    }

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let mut id_by_title = HashMap::new();
    for track in backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
    {
        id_by_title.insert(track.title.clone(), track.id);
    }

    let track_a = id_by_title.get("A").expect("track A id").clone();
    let track_b = id_by_title.get("B").expect("track B id").clone();
    let track_c = id_by_title.get("C").expect("track C id").clone();

    let playlist_id = backend
        .create_playlist(CreatePlaylistRequest {
            name: "Reorder Test".to_string(),
        })
        .data
        .expect("playlist data")
        .playlist_id;

    let add_initial = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_a.clone(), track_b.clone(), track_c.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(add_initial.ok, "initial add failed: {add_initial:?}");

    let reorder = backend.reorder_playlist_tracks(ReorderPlaylistTracksRequest {
        playlist_id: playlist_id.clone(),
        ordered_track_ids: vec![track_c.clone(), track_a.clone(), track_b.clone()],
    });
    assert!(reorder.ok, "reorder failed: {reorder:?}");
    assert_eq!(reorder.data.expect("reorder data").reordered, 3);

    let reordered_titles = backend
        .get_playlist_tracks(GetPlaylistTracksRequest { playlist_id, ..Default::default() })
        .data
        .expect("reordered tracks data")
        .items
        .into_iter()
        .map(|t| t.title)
        .collect::<Vec<_>>();
    assert_eq!(
        reordered_titles,
        vec!["C".to_string(), "A".to_string(), "B".to_string()]
    );
}

#[test]
fn scan_library_reports_validation_for_empty_roots_and_tolerates_missing_root() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let empty_roots = backend.scan_library(ScanLibraryRequest {
        source_roots: Vec::new(),
        incremental: true,
    });
    assert!(!empty_roots.ok, "empty roots should fail validation");
    let empty_error = empty_roots.error.expect("empty roots error payload");
    assert!(
        empty_error
            .message
            .contains("sourceRoots must contain at least one path"),
        "unexpected empty-roots error: {empty_error:?}"
    );

    let missing_root = root.path().join("does-not-exist");
    let missing = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![missing_root.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(missing.ok, "missing root should not fail: {missing:?}");
    let missing_data = missing.data.expect("missing root scan data");
    assert_eq!(missing_data.indexed, 0);
    assert_eq!(missing_data.updated, 0);
    assert_eq!(missing_data.removed, 0);
}

#[test]
fn fetch_usb_playlists_with_progress_returns_ok_response_and_emits_progress() {
    let root = tempdir().expect("temp root");
    let usb_root = root.path().join("usb");
    fs::create_dir_all(&usb_root).expect("create usb root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let mut progress_messages = Vec::<String>::new();
    let response = backend.fetch_usb_playlists_with_progress(
        FetchUsbPlaylistsRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
        },
        |_, _, message| progress_messages.push(message.to_string()),
    );

    assert!(response.ok, "fetch with progress failed: {response:?}");
    assert!(
        !progress_messages.is_empty(),
        "expected at least one progress callback"
    );
}

#[test]
fn run_usb_diagnostics_with_progress_returns_api_failure_for_missing_root() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let missing_usb = root.path().join("missing-usb");

    let mut progress_events = 0usize;
    let response = backend.run_usb_diagnostics_with_progress(
        RunUsbDiagnosticsRequest {
            usb_root: Some(missing_usb.to_string_lossy().to_string()),
        },
        |_, _, _| {
            progress_events += 1;
        },
    );

    assert!(!response.ok, "missing root should fail: {response:?}");
    let error = response.error.expect("missing-root error payload");
    assert!(
        !error.message.trim().is_empty(),
        "expected non-empty error message"
    );
    let _ = progress_events;
}

#[test]
fn take_playback_transitions_hands_off_the_receiver_exactly_once() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    assert!(
        backend.take_playback_transitions().is_some(),
        "first caller should receive the transition channel"
    );
    assert!(
        backend.take_playback_transitions().is_none(),
        "second caller should find the channel already taken"
    );
}

#[test]
fn scan_master_db_reports_validation_error_for_missing_path() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let missing_path = root.path().join("does-not-exist/master.db");
    let response = backend.scan_master_db(ScanMasterDbRequest {
        path: Some(missing_path.to_string_lossy().to_string()),
    });

    assert!(!response.ok, "missing master.db should fail");
    let error = response.error.expect("error payload");
    assert!(error.message.contains("master.db not found"));
}

#[test]
fn list_tracks_paginates_the_full_library() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");
    for name in ["Artist - 01.mp3", "Artist - 02.mp3", "Artist - 03.mp3"] {
        fs::write(media.join(name), b"audio").expect("write fixture track");
    }
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let page = backend.list_tracks(ListTracksRequest {
        limit: 2,
        cursor: None,
    });
    assert!(page.ok, "list_tracks failed: {page:?}");
    let page_data = page.data.expect("page data");
    assert_eq!(page_data.total, 3);
    assert_eq!(page_data.items.len(), 2);
    assert!(page_data.has_more);

    let next_cursor = page_data.next_cursor.expect("next cursor");
    let page2 = backend.list_tracks(ListTracksRequest {
        limit: 2,
        cursor: Some(next_cursor),
    });
    assert!(page2.ok, "second page failed: {page2:?}");
    let page2_data = page2.data.expect("page2 data");
    assert_eq!(page2_data.items.len(), 1);
    assert!(!page2_data.has_more);
}

#[test]
fn check_source_roots_reports_missing_and_existing_roots() {
    let root = tempdir().expect("temp root");
    let existing = root.path().join("existing");
    fs::create_dir_all(&existing).expect("create existing root");
    let missing = root.path().join("missing");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let response = backend.check_source_roots(CheckSourceRootsRequest {
        source_roots: vec![
            existing.to_string_lossy().to_string(),
            missing.to_string_lossy().to_string(),
        ],
    });
    assert!(response.ok, "check_source_roots failed: {response:?}");
    let data = response.data.expect("check_source_roots data");
    assert_eq!(data.items.len(), 2);
    assert_eq!(data.missing, vec![missing.to_string_lossy().to_string()]);
}

#[test]
fn rename_playlist_updates_the_stored_name() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Original Name".to_string(),
    });
    assert!(created.ok, "create playlist failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let renamed = backend.rename_playlist(RenamePlaylistRequest {
        playlist_id: playlist_id.clone(),
        name: "New Name".to_string(),
    });
    assert!(renamed.ok, "rename failed: {renamed:?}");
    assert_eq!(renamed.data.expect("rename data").name, "New Name");

    let listed = backend.list_playlists();
    assert!(listed.ok, "list_playlists failed: {listed:?}");
    let found = listed
        .data
        .expect("list data")
        .items
        .into_iter()
        .find(|p| p.id == playlist_id)
        .expect("renamed playlist should still be present");
    assert_eq!(found.name, "New Name");
}

#[test]
fn list_usb_devices_and_prune_usb_device_roundtrip() {
    let root = tempdir().expect("temp root");
    let usb_root = root.path().join("USB_TEST");
    fs::create_dir_all(&usb_root).expect("create usb root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let validated = backend.validate_usb_root(ValidateUsbRootRequest {
        path: usb_root.to_string_lossy().to_string(),
    });
    assert!(validated.ok, "validate usb root failed: {validated:?}");

    let listed = backend.list_usb_devices();
    assert!(listed.ok, "list_usb_devices failed: {listed:?}");
    let devices = listed.data.expect("device list data").items;
    assert_eq!(devices.len(), 1, "expected exactly one registered device");
    let device_id = devices[0].id.clone();

    let pruned = backend.prune_usb_device(PruneUsbDeviceRequest {
        id: device_id.clone(),
    });
    assert!(pruned.ok, "prune failed: {pruned:?}");
    assert!(pruned.data.expect("prune data").pruned);

    let listed_after = backend.list_usb_devices();
    assert!(listed_after.ok, "list_usb_devices after prune failed");
    assert!(
        listed_after
            .data
            .expect("after prune data")
            .items
            .is_empty()
    );
}

#[test]
fn merge_orphaned_usb_placeholder_tracks_is_a_noop_on_an_empty_library() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let response = backend.merge_orphaned_usb_placeholder_tracks();
    assert!(response.ok, "merge failed: {response:?}");
    assert_eq!(response.data.expect("merge data").merged, 0);
}

#[test]
fn fetch_usb_histories_with_progress_returns_api_failure_for_missing_root() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let missing_usb = root.path().join("missing-usb");

    let mut progress_events = 0usize;
    let response = backend.fetch_usb_histories_with_progress(
        FetchUsbHistoriesRequest {
            usb_root: Some(missing_usb.to_string_lossy().to_string()),
        },
        |_, _, _| {
            progress_events += 1;
        },
    );

    assert!(!response.ok, "missing root should fail: {response:?}");
    let _ = progress_events;
}

#[test]
fn remove_usb_playlist_with_progress_removes_playlist_from_edb() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(vendor_db_dir(&usb)).expect("create usb dirs");
    let db_path = vendor_db_dir(&usb).join("exportLibrary.db");
    {
        let conn = rusqlite::Connection::open(&db_path).expect("create export db");
        conn.execute_batch(&format!("PRAGMA key='{DEFAULT_USB_EDB_KEY}';"))
            .expect("set SQLCipher key");
        conn.execute_batch(
            r#"
            CREATE TABLE playlist (
              playlist_id INTEGER PRIMARY KEY,
              name TEXT,
              attribute INTEGER,
              sequenceNo INTEGER
            );
            CREATE TABLE playlist_content (
              playlist_id INTEGER,
              content_id INTEGER,
              sequenceNo INTEGER
            );
            INSERT INTO playlist (playlist_id, name, attribute, sequenceNo)
              VALUES (1, 'Progress Playlist', 0, 1);
            INSERT INTO playlist_content (playlist_id, content_id, sequenceNo)
              VALUES (1, 10, 1);
            "#,
        )
        .expect("seed export db");
    }

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let mut progress_messages = Vec::<String>::new();
    let response = backend.remove_usb_playlist_with_progress(
        RemoveUsbPlaylistRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
            playlist_id: None,
            playlist_name: "Progress Playlist".to_string(),
        },
        |_, _, message| progress_messages.push(message.to_string()),
    );

    assert!(response.ok, "remove failed: {response:?}");
    assert_eq!(response.data.expect("remove data").removed_from_edb, 1);
    assert!(
        !progress_messages.is_empty(),
        "expected at least one progress callback"
    );
}

#[test]
fn reorder_usb_playlists_with_progress_rejects_ids_without_pdb_prefix() {
    let root = tempdir().expect("temp root");
    let usb_root = root.path().join("usb");
    fs::create_dir_all(&usb_root).expect("create usb root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let mut progress_events = 0usize;
    let response = backend.reorder_usb_playlists_with_progress(
        ReorderUsbPlaylistsRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
            ordered_playlist_ids: vec!["usb-pl-name-not-pdb-backed".to_string()],
        },
        |_, _, _| {
            progress_events += 1;
        },
    );

    assert!(
        !response.ok,
        "ids without a bare-u32 PDB suffix should fail validation"
    );
    let error = response.error.expect("error payload");
    assert!(error.message.contains("no PDB-backed playlist ids"));
    let _ = progress_events;
}

#[test]
fn set_analysis_paused_and_cancel_analysis_report_success() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let paused = backend.set_analysis_paused(true);
    assert!(paused.ok, "pause failed: {paused:?}");
    assert!(paused.data.expect("pause data").paused);

    let resumed = backend.set_analysis_paused(false);
    assert!(resumed.ok, "resume failed: {resumed:?}");
    assert!(!resumed.data.expect("resume data").paused);

    let cancelled = backend.cancel_analysis();
    assert!(cancelled.ok, "cancel failed: {cancelled:?}");
}

#[test]
fn export_to_usb_with_progress_reports_validation_error_for_empty_playlist_id() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let mut progress_events = 0usize;
    let response = backend.export_to_usb_with_progress(
        ExportToUsbRequest {
            usb_root: None,
            playlist_id: String::new(),
            options: None,
        },
        |_, _, _| {
            progress_events += 1;
        },
    );

    assert!(!response.ok, "empty playlist id should fail validation");
    let error = response.error.expect("error payload");
    assert!(error.message.contains("playlistId must not be empty"));
    let _ = progress_events;
}

#[test]
fn play_track_native_rejects_missing_file_before_touching_audio_hardware() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let response = backend.play_track_native(PlayTrackRequest {
        path: "/nonexistent/path/to/track.mp3".to_string(),
        start_offset_ms: None,
        start_ratio: None,
    });

    assert!(!response.ok, "missing file should be rejected");
}

#[test]
fn play_resolved_track_reports_not_found_without_library_or_usb_path() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let response = backend.play_resolved_track(PlayResolvedTrackRequest {
        title: "Missing".to_string(),
        artist: "Artist".to_string(),
        album: None,
        bpm: None,
        file_path: Some("/detached/track.mp3".to_string()),
        file_size_bytes: None,
        track_id: Some("usb-placeholder".to_string()),
        origin: Some("usb".to_string()),
        usb_root: Some("/usb".to_string()),
        usb_root_valid: true,
        start_offset_ms: None,
        start_ratio: None,
    });

    assert!(!response.ok, "unresolved track should fail");
    let error = response.error.expect("error payload");
    assert!(
        error
            .message
            .contains("track not found in Library or selected USB")
    );
}

#[test]
fn stop_and_status_playback_native_report_idle_state_without_hardware() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let stopped = backend.stop_playback_native();
    assert!(stopped.ok, "stop failed: {stopped:?}");
    assert!(stopped.data.expect("stop data").stopped);

    let status = backend.get_playback_status_native();
    assert!(status.ok, "status failed: {status:?}");
    assert!(!status.data.expect("status data").playing);
}

#[test]
fn playback_preflight_native_reports_readable_fixture() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/formats/track_format_flac.flac");
    let response = backend.playback_preflight_native(PlaybackPreflightRequest {
        path: fixture.to_string_lossy().to_string(),
    });

    assert!(response.ok, "preflight failed: {response:?}");
    let data = response.data.expect("preflight data");
    assert!(data.file_exists);
    assert!(data.file_readable);
}

#[test]
fn run_usb_parity_report_with_progress_returns_api_failure_for_missing_root() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let missing_usb = root.path().join("missing-usb");

    let mut progress_events = 0usize;
    let response = backend.run_usb_parity_report_with_progress(
        RunUsbParityReportRequest {
            usb_root: Some(missing_usb.to_string_lossy().to_string()),
        },
        |_, _, _| {
            progress_events += 1;
        },
    );

    assert!(!response.ok, "missing root should fail: {response:?}");
    let _ = progress_events;
}

#[test]
fn detect_external_master_db_returns_a_response() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let response = backend.detect_external_master_db();
    assert!(response.ok, "detect failed: {response:?}");
}
