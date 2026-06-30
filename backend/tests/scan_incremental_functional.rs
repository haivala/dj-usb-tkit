use std::fs;

use backend::commands::BackendCommands;
use backend::models::{ScanLibraryRequest, SearchTracksRequest};
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
