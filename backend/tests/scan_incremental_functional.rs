use std::fs;

use backend::commands::BackendCommands;
use backend::models::{
    AddTracksToPlaylistRequest, CreatePlaylistRequest, DedupeMode, GetPlaylistTracksRequest,
    RelocateSourceRootRequest, ScanLibraryRequest, SearchTracksRequest,
};
use tempfile::tempdir;

#[test]
fn scan_incremental_covers_unchanged_new_and_deleted_files() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");

    let track_a = media.join("Artist - A.mp3");
    let track_b = media.join("Artist - B.mp3");
    let track_c = media.join("Artist - C.mp3");
    fs::write(&track_a, b"audio-a").expect("write track a");
    fs::write(&track_b, b"audio-b").expect("write track b");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let first = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(first.ok, "first scan failed: {first:?}");
    let first_data = first.data.expect("first scan data");
    assert_eq!(first_data.indexed, 2);
    assert_eq!(first_data.updated, 0);
    assert_eq!(first_data.removed, 0);

    let unchanged = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(unchanged.ok, "unchanged scan failed: {unchanged:?}");
    let unchanged_data = unchanged.data.expect("unchanged scan data");
    assert_eq!(unchanged_data.indexed, 0);
    assert_eq!(unchanged_data.updated, 0);
    assert_eq!(unchanged_data.removed, 0);

    fs::write(&track_c, b"audio-c").expect("write track c");
    let with_new = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(with_new.ok, "scan with new track failed: {with_new:?}");
    let with_new_data = with_new.data.expect("scan with new data");
    assert_eq!(with_new_data.indexed, 1);
    assert_eq!(with_new_data.updated, 0);
    assert_eq!(with_new_data.removed, 0);

    fs::remove_file(&track_b).expect("remove track b");
    let with_delete = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(with_delete.ok, "scan with delete failed: {with_delete:?}");
    let with_delete_data = with_delete.data.expect("scan with delete data");
    assert_eq!(with_delete_data.indexed, 0);
    assert_eq!(with_delete_data.updated, 0);
    assert_eq!(with_delete_data.removed, 1);

    let final_tracks = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 50,
            cursor: None,
        })
        .data
        .expect("final search data");
    assert_eq!(final_tracks.total, 2, "expected A and C after delete");
}

#[test]
fn scan_missing_source_root_reports_not_found_without_pruning_tracks() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");
    fs::write(media.join("Artist - A.mp3"), b"audio-a").expect("write track a");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let source_root = media.to_string_lossy().to_string();

    let first = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![source_root.clone()],
        incremental: true,
    });
    assert!(first.ok, "first scan failed: {first:?}");
    assert_eq!(first.data.expect("first scan data").indexed, 1);

    fs::remove_dir_all(&media).expect("remove media dir");

    let missing = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![source_root.clone()],
        incremental: true,
    });
    assert!(missing.ok, "missing-root scan failed: {missing:?}");
    let missing_data = missing.data.expect("missing scan data");
    assert_eq!(missing_data.indexed, 0);
    assert_eq!(missing_data.updated, 0);
    assert_eq!(missing_data.removed, 0);
    assert_eq!(missing_data.not_found, vec![source_root]);
    assert!(!missing_data.warnings.is_empty());

    let tracks = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 50,
            cursor: None,
        })
        .data
        .expect("search data");
    assert_eq!(tracks.total, 1, "missing root must not prune tracks");
}

#[test]
fn relocate_source_root_rewrites_paths_and_preserves_playlist_membership() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let nested = media.join("Artist");
    fs::create_dir_all(&nested).expect("create media dir");
    fs::write(nested.join("Artist - A.mp3"), b"audio-a").expect("write track a");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let old_root = media.to_string_lossy().to_string();

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![old_root.clone()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let tracks = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 50,
            cursor: None,
        })
        .data
        .expect("search data");
    assert_eq!(tracks.total, 1);
    let track_id = tracks.items[0].id.clone();

    let playlist = backend
        .create_playlist(CreatePlaylistRequest {
            name: "Crate".to_string(),
        })
        .data
        .expect("playlist data");
    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist.playlist_id.clone(),
        track_ids: vec![track_id.clone()],
        dedupe: DedupeMode::Allow,
    });
    assert!(added.ok, "add tracks failed: {added:?}");

    let new_media = root.path().join("relocated");
    fs::rename(&media, &new_media).expect("move media dir");
    let new_root = new_media.to_string_lossy().to_string();

    let relocated = backend.relocate_source_root(RelocateSourceRootRequest {
        old_root: old_root.clone(),
        new_root: new_root.clone(),
    });
    assert!(relocated.ok, "relocation failed: {relocated:?}");
    let relocated_data = relocated.data.expect("relocation data");
    assert_eq!(relocated_data.matched, 1);
    assert_eq!(relocated_data.updated, 1);
    assert_eq!(relocated_data.missing_at_new_root, 0);
    assert_eq!(relocated_data.conflicts, 0);

    let tracks_after = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 50,
            cursor: None,
        })
        .data
        .expect("search data after relocation");
    assert_eq!(tracks_after.total, 1);
    assert_eq!(tracks_after.items[0].id, track_id);
    assert!(tracks_after.items[0].file_path.starts_with(&new_root));

    let playlist_tracks = backend
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
        })
        .data
        .expect("playlist tracks data");
    assert_eq!(playlist_tracks.items.len(), 1);
    assert_eq!(playlist_tracks.items[0].id, track_id);
    assert!(playlist_tracks.items[0].file_path.starts_with(&new_root));
}
