use std::collections::{HashMap, HashSet};
use std::fs;

use backend::commands::BackendCommands;
use backend::models::{
    AddTracksToPlaylistRequest, CreatePlaylistRequest, DedupeMode, FetchUsbPlaylistsRequest,
    GetPlaylistTracksRequest, InitializeUsbRequest, RemoveTracksFromPlaylistRequest,
    RunUsbDiagnosticsRequest, ScanLibraryRequest, SearchTracksRequest,
};
use tempfile::tempdir;

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
        .get_playlist_tracks(GetPlaylistTracksRequest { playlist_id })
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
