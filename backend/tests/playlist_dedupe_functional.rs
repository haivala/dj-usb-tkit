use std::fs;

use backend::commands::BackendCommands;
use backend::models::{
    AddTracksToPlaylistRequest, CreatePlaylistRequest, DedupeMode, GetPlaylistTracksRequest,
    ScanLibraryRequest, SearchTracksRequest,
};
use tempfile::tempdir;

#[test]
fn add_tracks_to_playlist_honors_skip_and_allow_dedupe_modes() {
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
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
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

    let add_skip = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_id.clone(), track_id.clone(), track_id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(add_skip.ok, "add skip failed: {add_skip:?}");
    let add_skip_data = add_skip.data.expect("add skip data");
    assert_eq!(add_skip_data.added, 1);
    assert_eq!(add_skip_data.skipped, 2);

    let add_skip_again = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(
        add_skip_again.ok,
        "add skip again failed: {add_skip_again:?}"
    );
    let add_skip_again_data = add_skip_again.data.expect("add skip again data");
    assert_eq!(add_skip_again_data.added, 0);
    assert_eq!(add_skip_again_data.skipped, 1);

    let add_allow = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_id.clone(), track_id.clone()],
        dedupe: DedupeMode::Allow,
    });
    assert!(add_allow.ok, "add allow failed: {add_allow:?}");
    let add_allow_data = add_allow.data.expect("add allow data");
    assert_eq!(add_allow_data.added, 2);
    assert_eq!(add_allow_data.skipped, 0);

    let playlist_tracks = backend
        .get_playlist_tracks(GetPlaylistTracksRequest { playlist_id })
        .data
        .expect("playlist tracks data")
        .items;
    assert_eq!(playlist_tracks.len(), 3);
    assert!(playlist_tracks.iter().all(|track| track.id == track_id));
}
