use std::fs;

use backend::commands::BackendCommands;
use backend::models::{
    AddLibrarySelectionToPlaylistRequest, BrowseSourceFilesRequest, CreatePlaylistRequest,
    GetPlaylistTracksRequest, ListMatchingTrackIdsRequest, ScanLibraryRequest,
};
use tempfile::tempdir;

#[test]
fn adding_a_track_already_in_the_playlist_skips_it() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");

    let track_path = media.join("Artist - Single.mp3");
    fs::write(&track_path, b"audio").expect("write track");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track_id = backend
        .browse_source_files(BrowseSourceFilesRequest {
            source_roots: vec![media.to_string_lossy().to_string()],
            query: String::new(),
            limit: 10,
            cursor: None,
            ..Default::default()
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .next()
        .expect("indexed track")
        .id;

    let playlist_id = backend
        .create_playlist(CreatePlaylistRequest {
            name: "Dedupe Test".to_string(),
        })
        .data
        .expect("playlist data")
        .playlist_id;

    let add_first =
        backend.add_library_selection_to_playlist(AddLibrarySelectionToPlaylistRequest {
            source_roots: vec![media.to_string_lossy().to_string()],
            playlist_id: playlist_id.clone(),
            track_ids: vec![track_id.clone()],
            ..Default::default()
        });
    assert!(add_first.ok, "add failed: {add_first:?}");
    let add_first_data = add_first.data.expect("add data");
    assert_eq!(add_first_data.added, 1);
    assert_eq!(add_first_data.skipped, 0);

    let add_again =
        backend.add_library_selection_to_playlist(AddLibrarySelectionToPlaylistRequest {
            source_roots: vec![media.to_string_lossy().to_string()],
            playlist_id: playlist_id.clone(),
            track_ids: vec![track_id.clone()],
            ..Default::default()
        });
    assert!(add_again.ok, "add again failed: {add_again:?}");
    let add_again_data = add_again.data.expect("add again data");
    assert_eq!(add_again_data.added, 0);
    assert_eq!(add_again_data.skipped, 1);

    let playlist_tracks = backend
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id,
            ..Default::default()
        })
        .data
        .expect("playlist tracks data")
        .items;
    assert_eq!(playlist_tracks.len(), 1);
    assert_eq!(playlist_tracks[0].id, track_id);
}

#[test]
fn library_selection_is_enumerated_and_added_entirely_server_side() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");
    for name in ["Artist - One.mp3", "Artist - Two.mp3", "Artist - Three.mp3"] {
        fs::write(media.join(name), b"audio").expect("write track");
    }

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let roots = vec![media.to_string_lossy().to_string()];
    assert!(
        backend
            .scan_library(ScanLibraryRequest {
                source_roots: roots.clone(),
                incremental: true,
            })
            .ok
    );

    // Every matching id, enumerated server-side (no need to page the list).
    let matching = backend
        .list_matching_track_ids(ListMatchingTrackIdsRequest {
            source_roots: roots.clone(),
            include_rekordbox_db: false,
            include_mixxx_db: false,
            query: String::new(),
        })
        .data
        .expect("matching ids");
    assert_eq!(matching.total, 3);
    assert_eq!(matching.track_ids.len(), 3);

    let playlist_id = backend
        .create_playlist(CreatePlaylistRequest {
            name: "Server-side selection".to_string(),
        })
        .data
        .expect("playlist data")
        .playlist_id;

    // `all_matching` adds the whole filtered set even though the caller passed
    // no ids at all.
    let added_all = backend
        .add_library_selection_to_playlist(AddLibrarySelectionToPlaylistRequest {
            playlist_id: playlist_id.clone(),
            source_roots: roots.clone(),
            include_rekordbox_db: false,
            include_mixxx_db: false,
            query: String::new(),
            track_ids: Vec::new(),
            all_matching: true,
        })
        .data
        .expect("add-all data");
    assert_eq!(added_all.added, 3);

    // A second call with explicit ids is fully de-duped.
    let added_again = backend
        .add_library_selection_to_playlist(AddLibrarySelectionToPlaylistRequest {
            playlist_id: playlist_id.clone(),
            source_roots: roots,
            include_rekordbox_db: false,
            include_mixxx_db: false,
            query: String::new(),
            track_ids: matching.track_ids.clone(),
            all_matching: false,
        })
        .data
        .expect("add-again data");
    assert_eq!(added_again.added, 0);
    assert_eq!(added_again.skipped, 3);

    let count = backend
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id,
            ..Default::default()
        })
        .data
        .expect("playlist tracks")
        .items
        .len();
    assert_eq!(count, 3);
}
