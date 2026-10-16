use std::fs;

use backend::commands::BackendCommands;
use backend::models::{
    AddLibrarySelectionToPlaylistRequest, AddLibrarySelectionToPlaylistsRequest, AddTrackCandidate,
    AddTrackCandidatesToPlaylistsRequest, BrowseSourceFilesRequest, CreatePlaylistRequest,
    GetPlaylistMembershipRequest, GetPlaylistTracksRequest, ListMatchingTrackIdsRequest,
    PlaylistMembershipState, ScanLibraryRequest,
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

fn playlist_track_ids(backend: &BackendCommands, playlist_id: &str) -> Vec<String> {
    backend
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist_id.to_string(),
            ..Default::default()
        })
        .data
        .expect("playlist tracks")
        .items
        .into_iter()
        .map(|track| track.id)
        .collect()
}

fn membership(
    backend: &BackendCommands,
    track_ids: &[Option<&str>],
) -> Vec<(String, usize, PlaylistMembershipState)> {
    let data = backend
        .get_playlist_membership(GetPlaylistMembershipRequest {
            track_ids: track_ids.iter().map(|id| id.map(str::to_string)).collect(),
        })
        .data
        .expect("membership");
    assert_eq!(data.selected, track_ids.len());
    data.items
        .into_iter()
        .map(|item| (item.name, item.contained, item.state))
        .collect()
}

#[test]
fn add_to_picker_shows_membership_and_adds_the_selection_to_several_playlists() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");
    for name in ["Artist - One.mp3", "Artist - Two.mp3"] {
        fs::write(media.join(name), b"audio").expect("write track");
    }
    let backend = BackendCommands::new(root.path().join("data")).expect("create backend");
    let roots = vec![media.to_string_lossy().to_string()];
    assert!(
        backend
            .scan_library(ScanLibraryRequest {
                source_roots: roots.clone(),
                incremental: true,
            })
            .ok
    );
    let ids = backend
        .list_matching_track_ids(ListMatchingTrackIdsRequest {
            source_roots: roots.clone(),
            include_rekordbox_db: false,
            include_mixxx_db: false,
            query: String::new(),
        })
        .data
        .expect("matching ids")
        .track_ids;
    let create = |name: &str| {
        backend
            .create_playlist(CreatePlaylistRequest {
                name: name.to_string(),
            })
            .data
            .expect("playlist")
            .playlist_id
    };
    let warmup = create("Warmup");
    let peak = create("Peak");
    let added = backend
        .add_library_selection_to_playlist(AddLibrarySelectionToPlaylistRequest {
            playlist_id: warmup.clone(),
            source_roots: roots.clone(),
            track_ids: vec![ids[0].clone()],
            ..Default::default()
        })
        .data
        .expect("add first");
    assert_eq!(added.added, 1);

    // Sidebar order (newest first). A track with no local row (`None`)
    // counts toward the selection but is in no playlist.
    use PlaylistMembershipState::{All, None as Neither, Some as Part};
    assert_eq!(
        membership(&backend, &[Some(&ids[0]), Some(&ids[1]), None]),
        vec![
            ("Peak".to_string(), 0, Neither),
            ("Warmup".to_string(), 1, Part)
        ]
    );
    assert_eq!(
        membership(&backend, &[Some(&ids[0])]),
        vec![
            ("Peak".to_string(), 0, Neither),
            ("Warmup".to_string(), 1, All)
        ]
    );

    // No playlist, or one that doesn't exist: nothing is added anywhere.
    let none = backend.add_library_selection_to_playlists(AddLibrarySelectionToPlaylistsRequest {
        source_roots: roots.clone(),
        track_ids: ids.clone(),
        ..Default::default()
    });
    assert!(!none.ok);
    let unknown =
        backend.add_library_selection_to_playlists(AddLibrarySelectionToPlaylistsRequest {
            playlist_ids: vec![peak.clone(), "missing".to_string()],
            source_roots: roots.clone(),
            track_ids: ids.clone(),
            ..Default::default()
        });
    assert!(!unknown.ok);
    assert!(playlist_track_ids(&backend, &peak).is_empty());

    // Both playlists, one named twice: each takes what it lacks.
    let multi = backend
        .add_library_selection_to_playlists(AddLibrarySelectionToPlaylistsRequest {
            playlist_ids: vec![warmup.clone(), peak.clone(), warmup.clone()],
            source_roots: roots,
            track_ids: ids.clone(),
            ..Default::default()
        })
        .data
        .expect("add to playlists");
    assert_eq!(
        (multi.requested, multi.resolved, multi.unresolved),
        (2, 2, 0)
    );
    assert_eq!((multi.added, multi.skipped), (3, 1));
    let per_playlist: Vec<_> = multi
        .items
        .iter()
        .map(|item| (item.playlist_id.clone(), item.added, item.skipped))
        .collect();
    assert_eq!(
        per_playlist,
        vec![(warmup.clone(), 1, 1), (peak.clone(), 2, 0)]
    );
    assert_eq!(playlist_track_ids(&backend, &warmup), ids);
    assert_eq!(playlist_track_ids(&backend, &peak), ids);
    assert_eq!(
        membership(&backend, &[Some(&ids[0]), Some(&ids[1])]),
        vec![("Peak".to_string(), 2, All), ("Warmup".to_string(), 2, All)]
    );
}

#[test]
fn add_to_picker_resolves_track_candidates_once_for_every_playlist() {
    let root = tempdir().expect("temp root");
    let file_path = root.path().join("Artist - Loose.mp3");
    fs::write(&file_path, b"audio").expect("write track");
    let backend = BackendCommands::new(root.path().join("data")).expect("create backend");
    let playlist_ids: Vec<String> = ["A", "B"]
        .into_iter()
        .map(|name| {
            backend
                .create_playlist(CreatePlaylistRequest {
                    name: name.to_string(),
                })
                .data
                .expect("playlist")
                .playlist_id
        })
        .collect();

    let path = file_path.to_string_lossy().to_string();
    let added = backend
        .add_track_candidates_to_playlists(AddTrackCandidatesToPlaylistsRequest {
            playlist_ids: playlist_ids.clone(),
            tracks: vec![AddTrackCandidate {
                track_id: Some(path.clone()),
                title: "Loose".to_string(),
                artist: "Artist".to_string(),
                file_path: Some(path),
                ..Default::default()
            }],
            ..Default::default()
        })
        .data
        .expect("add candidates");
    assert_eq!((added.requested, added.resolved, added.added), (1, 1, 2));
    assert_eq!(added.resolutions.len(), 1);
    assert!(added.resolutions[0].materialized);
    let local_id = added.resolutions[0].track_id.clone().expect("resolved id");
    for playlist_id in &playlist_ids {
        assert_eq!(
            playlist_track_ids(&backend, playlist_id),
            vec![local_id.clone()]
        );
    }
}
