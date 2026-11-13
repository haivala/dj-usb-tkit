//! The library side of the app through its commands: playlists and their
//! tracks, library sources and browsing, track identity, playback sources,
//! and frontend settings.
//!
//! The tests talk to a small adapter (`Service`) with the service's method
//! names; each method calls the matching `BackendCommands` command and
//! unwraps its response, so a test reads like the flow it checks.

mod common;
mod library_sources;

use std::path::{Path, PathBuf};

use common::copy_audio_fixture;

use backend::commands::BackendCommands;
use backend::error::ErrorPayload;
use backend::models::*;
use backend::service::build_track_match_fingerprint;
use rusqlite::{Connection, params};
use serde::Serialize;

// --- adapter -------------------------------------------------------------

/// A failed command, by the kind of error it reported. The message is
/// there for a failed assertion's output.
#[derive(Debug)]
#[allow(dead_code)]
enum BackendError {
    Validation(String),
    NotFound(String),
    Internal(String),
    Other(String),
}

impl From<ErrorPayload> for BackendError {
    fn from(error: ErrorPayload) -> Self {
        let message = error.log_message.clone().unwrap_or(error.message.clone());
        match error.code {
            backend::error::ErrorCode::ValidationError => Self::Validation(message),
            backend::error::ErrorCode::NotFound => Self::NotFound(message),
            backend::error::ErrorCode::InternalError => Self::Internal(message),
            _ => Self::Other(message),
        }
    }
}

type BackendResult<T> = Result<T, BackendError>;

fn result<T: Serialize>(response: ApiResponse<T>) -> BackendResult<T> {
    if response.ok {
        Ok(response.data.expect("a successful response carries data"))
    } else {
        Err(response
            .error
            .expect("a failed response carries an error")
            .into())
    }
}

/// The app's library database, for seeding and checks the commands don't report.
struct LibraryDb {
    data_dir: PathBuf,
}

impl LibraryDb {
    fn connect(&self) -> rusqlite::Result<Connection> {
        Connection::open(self.data_dir.join("backend.db"))
    }

    fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}

/// The playlists as the sidebar lists them.
struct ListedPlaylists {
    items: Vec<Playlist>,
}

struct Service {
    backend: BackendCommands,
    db: LibraryDb,
}

macro_rules! commands {
    ($($name:ident($req:ty) -> $out:ty;)*) => {
        $(fn $name(&self, req: $req) -> BackendResult<$out> {
            result(self.backend.$name(req))
        })*
    };
}

macro_rules! playlist_commands {
    ($($name:ident($req:ty) -> $out:ty;)*) => {
        $(fn $name(&self, req: $req) -> BackendResult<$out> {
            result(self.backend.$name(req)).map(|with_playlists| with_playlists.data)
        })*
    };
}

impl Service {
    commands! {
        get_playlist_tracks(GetPlaylistTracksRequest) -> GetPlaylistTracksData;
        browse_source_files(BrowseSourceFilesRequest) -> BrowseSourceFilesData;
        resolve_playback_source(ResolvePlaybackSourceRequest) -> ResolvePlaybackSourceData;
        set_frontend_setting(SetFrontendSettingRequest) -> SetFrontendSettingData;
        set_source_folder_enabled(SetSourceFolderEnabledRequest) -> LibrarySourcesData;
        relocate_source_root(RelocateSourceRootRequest) -> RelocateSourceRootData;
        materialize_source_track(MaterializeSourceTrackRequest) -> MaterializeSourceTrackData;
        reorder_playlists(ReorderPlaylistsRequest) -> ReorderPlaylistsData;
        resolve_track_identity(ResolveTrackIdentityRequest) -> ResolveTrackIdentityData;
        remove_source_folder(RemoveSourceFolderRequest) -> RemoveSourceFolderData;
        get_selection_duration(GetSelectionDurationRequest) -> GetSelectionDurationData;
        add_source_folders(AddSourceFoldersRequest) -> AddSourceFoldersData;
    }

    playlist_commands! {
        create_playlist(CreatePlaylistRequest) -> CreatePlaylistData;
        reorder_playlist_tracks(ReorderPlaylistTracksRequest) -> ReorderPlaylistTracksData;
        rename_playlist(RenamePlaylistRequest) -> RenamePlaylistData;
        delete_playlist(DeletePlaylistRequest) -> DeletePlaylistData;
        remove_tracks_from_playlist(RemoveTracksFromPlaylistRequest) -> RemoveTracksFromPlaylistData;
        add_track_candidates_to_playlist(AddTrackCandidatesToPlaylistRequest) -> AddTrackCandidatesToPlaylistData;
    }

    fn get_frontend_settings(&self) -> BackendResult<GetFrontendSettingsData> {
        result(self.backend.get_frontend_settings())
    }

    fn get_library_sources(&self) -> BackendResult<LibrarySourcesData> {
        result(self.backend.get_library_sources())
    }

    fn remove_essentia(&self) -> BackendResult<()> {
        result(self.backend.remove_essentia())
    }

    fn list_playlists(&self) -> BackendResult<ListedPlaylists> {
        result(self.backend.list_playlists()).map(|state| ListedPlaylists {
            items: state.playlists,
        })
    }

    /// Adds library tracks by id, the way the app adds a track it already
    /// knows: a candidate carrying the track's own id.
    fn add_tracks_to_playlist(
        &self,
        req: AddTracksToPlaylistRequest,
    ) -> BackendResult<AddTracksToPlaylistData> {
        let requested = req.track_ids.len();
        let added = self.add_track_candidates_to_playlist(AddTrackCandidatesToPlaylistRequest {
            playlist_id: req.playlist_id,
            tracks: req
                .track_ids
                .into_iter()
                .map(|id| AddTrackCandidate {
                    track_id: Some(id),
                    ..Default::default()
                })
                .collect(),
            usb_root: None,
            usb_root_valid: false,
        })?;
        assert_eq!(
            added.resolved, requested,
            "every track id resolves: {added:?}"
        );
        Ok(AddTracksToPlaylistData {
            playlist_id: added.playlist_id,
            added: added.added,
            skipped: added.skipped,
            status_text: added.status_text,
        })
    }
}

fn test_service() -> (tempfile::TempDir, Service) {
    let dir = tempfile::tempdir().expect("app data dir");
    let backend = BackendCommands::new(dir.path()).expect("create backend");
    let db = LibraryDb {
        data_dir: dir.path().to_path_buf(),
    };
    (dir, Service { backend, db })
}

/// The library's sources through the app's commands: `roots` switched on
/// (added if new), every other folder off, and the imported libraries on or
/// off as given.
fn set_library_sources(service: &Service, roots: &[&str], rekordbox_db: bool, mixxx_db: bool) {
    library_sources::show_only(
        &service.backend,
        roots.iter().map(|root| root.to_string()).collect(),
        rekordbox_db,
        mixxx_db,
    );
}

#[allow(clippy::too_many_arguments)]
fn insert_full_track(
    conn: &Connection,
    id: &str,
    title: &str,
    artist: &str,
    file_path: &str,
    duration_ms: Option<i64>,
    file_size_bytes: Option<i64>,
    rekordbox_db_source: bool,
) {
    let fp = build_track_match_fingerprint(title, artist, None);
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, file_size_bytes, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'), datetime('now'))",
        params![id, title, artist, file_path, duration_ms, file_size_bytes, fp, rekordbox_db_source as i64],
    )
    .expect("insert track");
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .expect("file name")
        .to_string_lossy()
        .into_owned()
}

// Setting keys the frontend sends.
const SETTING_UI_THEME: &str = "ui_theme_v1";
const SETTING_UI_ANALYSIS_ENGINE: &str = "ui_analysis_engine_v1";

// --- tests -----------------------------------------------------------------

#[test]
fn list_playlists_names_the_drive_as_it_was_called_at_export() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("service db");
    // Another stick, "Booth Stick", now uses the mount path both playlists
    // were exported to.
    conn.execute_batch(&format!(
        "INSERT INTO usb_devices (id, root_path, root_path_key, label, first_seen_at, last_seen_at, created_at, updated_at)
           VALUES ('d1', '/run/media/dj/STICK', '{}', 'Booth Stick', 'x', 'x', 'x', 'x');
         INSERT INTO playlists (id, name, source, last_exported_at, last_exported_usb_root, last_exported_usb_name, created_at, updated_at)
           VALUES ('p1', 'Named', 'local', 'x', '/run/media/dj/STICK', 'Club Stick', '1', '1'),
                  ('p2', 'Before names were kept', 'local', 'x', '/run/media/dj/STICK', NULL, '2', '2'),
                  ('p3', 'Never', 'local', NULL, NULL, NULL, '3', '3');",
        "/run/media/dj/stick"
    ))
    .expect("seed");
    drop(conn);
    let names: Vec<Option<String>> = service
        .list_playlists()
        .expect("list")
        .items
        .into_iter()
        .map(|p| p.last_exported_usb_name)
        .collect();
    // Never reordered, so newest first: p3, p2, p1. The path's current
    // drive names neither export.
    assert_eq!(names, vec![None, None, Some("Club Stick".to_string())]);
}

#[test]
fn reorder_playlists_saves_sidebar_order_and_new_playlists_go_on_top() {
    let (_dir, service) = test_service();
    let create = |name: &str| {
        service
            .create_playlist(CreatePlaylistRequest {
                name: name.to_string(),
            })
            .expect("create playlist")
            .playlist_id
    };
    let listed_names = || -> Vec<String> {
        service
            .list_playlists()
            .expect("list")
            .items
            .into_iter()
            .map(|p| p.name)
            .collect()
    };
    let a = create("A");
    let b = create("B");
    let c = create("C");
    assert_eq!(listed_names(), ["C", "B", "A"]);

    let saved = service
        .reorder_playlists(ReorderPlaylistsRequest {
            ordered_playlist_ids: vec![a.clone(), c.clone(), b.clone()],
        })
        .expect("reorder");
    assert_eq!(saved.reordered, 3);
    assert_eq!(listed_names(), ["A", "C", "B"]);

    create("D");
    assert_eq!(listed_names(), ["D", "A", "C", "B"]);

    // Missing, duplicate or unknown ids change nothing.
    for ids in [
        vec![a.clone(), c.clone(), b.clone()],
        vec![a.clone(), a.clone(), c.clone(), b.clone()],
        vec![a.clone(), c.clone(), b.clone(), "nope".to_string()],
    ] {
        let err = service
            .reorder_playlists(ReorderPlaylistsRequest {
                ordered_playlist_ids: ids,
            })
            .unwrap_err();
        assert!(matches!(err, BackendError::Internal(_)));
    }
    assert_eq!(listed_names(), ["D", "A", "C", "B"]);
}

#[test]
fn create_playlist_rejects_empty_name() {
    let (_dir, service) = test_service();
    let err = service
        .create_playlist(CreatePlaylistRequest {
            name: "   ".to_string(),
        })
        .unwrap_err();
    assert!(matches!(err, BackendError::Validation(_)));
}

#[test]
fn playlist_create_rename_delete_roundtrip() {
    let (_dir, service) = test_service();
    let created = service
        .create_playlist(CreatePlaylistRequest {
            name: "  My Playlist  ".to_string(),
        })
        .expect("create playlist");
    assert_eq!(created.name, "My Playlist");
    assert_eq!(created.status_text, "Playlist created: My Playlist");

    let listed = service.list_playlists().expect("list playlists");
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].id, created.playlist_id);
    assert_eq!(
        listed.items[0].delete_confirm_text,
        "Delete \"My Playlist\"?\n\nThis removes the app playlist and its app playlist-track links."
    );

    let renamed = service
        .rename_playlist(RenamePlaylistRequest {
            playlist_id: created.playlist_id.clone(),
            name: "Renamed".to_string(),
        })
        .expect("rename playlist");
    assert_eq!(renamed.name, "Renamed");

    let deleted = service
        .delete_playlist(DeletePlaylistRequest {
            playlist_id: created.playlist_id.clone(),
        })
        .expect("delete playlist");
    assert!(deleted.deleted);
    assert_eq!(deleted.status_text, "Playlist deleted: Renamed");

    let err = service
        .rename_playlist(RenamePlaylistRequest {
            playlist_id: created.playlist_id,
            name: "Nope".to_string(),
        })
        .unwrap_err();
    assert!(matches!(err, BackendError::NotFound(_)));
}

#[test]
fn add_track_candidates_to_playlist_materializes_local_source_rows() {
    let (_dir, service) = test_service();
    let source_dir = tempfile::tempdir().expect("source dir");
    let file_path = source_dir.path().join("track.mp3");
    std::fs::write(&file_path, b"audio").expect("write source file");
    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");

    let result = service
        .add_track_candidates_to_playlist(AddTrackCandidatesToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            tracks: vec![AddTrackCandidate {
                track_id: Some(file_path.to_string_lossy().to_string()),
                title: "Track".to_string(),
                artist: "Artist".to_string(),
                file_path: Some(file_path.to_string_lossy().to_string()),
                ..Default::default()
            }],
            usb_root: None,
            usb_root_valid: false,
        })
        .expect("add candidate");

    assert_eq!(result.requested, 1);
    assert_eq!(result.resolved, 1);
    assert_eq!(result.unresolved, 0);
    assert_eq!(result.added, 1);
    assert_eq!(result.skipped, 0);
    assert!(result.resolutions[0].materialized);
    assert_eq!(
        result.resolutions[0].resolved_by,
        backend::models::ResolvedBy::Materialized
    );

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(tracks.items.len(), 1);
    assert_eq!(tracks.items[0].title, "Track");
}

#[test]
fn add_track_candidates_to_playlist_does_not_fuzzy_resolve_usb_origin_rows() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "local-1",
        "Same Song",
        "Same Artist",
        "/music/same.mp3",
        None,
        Some(1000),
        false,
    );
    drop(conn);
    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");

    let result = service
        .add_track_candidates_to_playlist(AddTrackCandidatesToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            tracks: vec![AddTrackCandidate {
                track_id: Some("usb-1".to_string()),
                title: "Same Song".to_string(),
                artist: "Same Artist".to_string(),
                file_path: Some("/usb/Contents/same.mp3".to_string()),
                file_size_bytes: Some(1000),
                usb_root: Some("/usb".to_string()),
                usb_analysis_path: Some("/usb/PIONEER/USBANLZ/P001/A/ANLZ0000.DAT".to_string()),
                ..Default::default()
            }],
            usb_root: None,
            usb_root_valid: false,
        })
        .expect("add candidate");

    assert_eq!(result.requested, 1);
    assert_eq!(result.resolved, 0);
    assert_eq!(result.unresolved, 1);
    assert_eq!(result.added, 0);
    assert_eq!(result.skipped, 0);
    assert_eq!(
        result.resolutions[0].resolved_by,
        backend::models::ResolvedBy::UsbOrigin
    );

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert!(tracks.items.is_empty());
}

#[test]
fn add_tracks_to_playlist_dedupe_skip_counts_skipped_and_get_playlist_tracks_orders_by_position() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song A",
        "Artist",
        "/music/a.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t2",
        "Song B",
        "Artist",
        "/music/b.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");

    let added = service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string()],
        })
        .expect("add tracks");
    assert_eq!(added.added, 2);
    assert_eq!(added.skipped, 0);

    let skip_result = service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string()],
        })
        .expect("add duplicate with skip mode");
    assert_eq!(skip_result.added, 0);
    assert_eq!(skip_result.skipped, 1);

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(
        tracks
            .items
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        vec!["t1".to_string(), "t2".to_string()]
    );
}

#[test]
fn get_playlist_tracks_list_totals_ignore_search() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song A",
        "Artist",
        "/music/a.mp3",
        Some(200_000),
        Some(8_000_000),
        false,
    );
    insert_full_track(
        &conn,
        "t2",
        "Song B",
        "Artist",
        "/music/b.mp3",
        Some(100_000),
        Some(2_000_000),
        false,
    );
    // Unknown size: counted as a track, adds nothing to the size.
    insert_full_track(
        &conn,
        "t3",
        "Song C",
        "Artist",
        "/music/c.mp3",
        Some(100_000),
        None,
        false,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        })
        .expect("add tracks");

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            query: "song b".to_string(),
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(tracks.total, 1);
    assert_eq!(tracks.total_duration_ms, 100_000);
    assert_eq!(tracks.list_track_count, 3);
    assert_eq!(tracks.list_size_bytes, 10_000_000);
    assert_eq!(tracks.list_summary_text, "3 tracks · 10 MB");
}

#[test]
fn get_playlist_tracks_computes_total_duration_server_side() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song A",
        "Artist",
        "/music/a.mp3",
        Some(200_000),
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t2",
        "Song B",
        "Artist",
        "/music/b.mp3",
        None,
        None,
        false,
    );
    // master.db-sourced -- must survive get_playlist_tracks' hand-written
    // SELECT (regression: its column list was misaligned vs `row_to_track`,
    // which reads rekordbox_db_source positionally, so every playlist track
    // came back rekordbox_db_source=false).
    insert_full_track(
        &conn,
        "t3",
        "Song C",
        "Artist",
        "/music/c.mp3",
        Some(100_000),
        None,
        true,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        })
        .expect("add tracks");

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(tracks.total_duration_ms, 300_000);
    assert_eq!(tracks.duration_unknown_count, 1);

    let by_id = |id: &str| {
        tracks
            .items
            .iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("missing track {id}"))
    };
    assert!(
        by_id("t3").rekordbox_db_source,
        "rekordbox_db_source must survive get_playlist_tracks"
    );
    assert!(!by_id("t1").rekordbox_db_source);
}

#[test]
fn get_selection_duration_sums_only_the_ticked_playlist_tracks() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    for (id, path, duration) in [
        ("t1", "/music/a.mp3", Some(200_000)),
        ("t2", "/music/b.mp3", None),
        ("t3", "/music/c.mp3", Some(100_000)),
    ] {
        insert_full_track(&conn, id, "Song", "Artist", path, duration, None, false);
    }
    drop(conn);
    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        })
        .expect("add tracks");

    // t3 unticked; "gone" is no longer in the playlist and adds nothing.
    let data = service
        .get_selection_duration(GetSelectionDurationRequest::Playlist {
            id: playlist.playlist_id,
            track_ids: vec!["t1".to_string(), "t2".to_string(), "gone".to_string()],
        })
        .expect("selection duration");
    assert_eq!(data.total_duration_ms, 200_000);
    assert_eq!(data.duration_unknown_count, 1);
    assert_eq!(data.track_count, 2);
}

#[test]
fn get_playlist_tracks_paginates_filters_and_sorts_server_side() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    for (id, title) in [("t1", "Charlie"), ("t2", "Alpha"), ("t3", "Bravo")] {
        insert_full_track(
            &conn,
            id,
            title,
            "Artist",
            &format!("/m/{id}.mp3"),
            None,
            None,
            false,
        );
    }
    drop(conn);
    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    let pid = playlist.playlist_id.clone();
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: pid.clone(),
            track_ids: vec!["t1".into(), "t2".into(), "t3".into()],
        })
        .expect("add tracks");

    // Default order is playlist position; totals span the whole playlist.
    let page1 = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: pid.clone(),
            limit: 2,
            ..Default::default()
        })
        .expect("page 1");
    assert_eq!(page1.total, 3);
    assert_eq!(
        page1.items.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
        ["t1", "t2"]
    );
    assert!(page1.has_more);
    let page2 = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: pid.clone(),
            limit: 2,
            cursor: page1.next_cursor.clone(),
            ..Default::default()
        })
        .expect("page 2");
    assert_eq!(
        page2.items.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
        ["t3"]
    );
    assert!(!page2.has_more);

    // An offset jumps straight to a position, with no cursor needed.
    let jumped = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: pid.clone(),
            limit: 1,
            offset: Some(1),
            ..Default::default()
        })
        .expect("offset page");
    assert_eq!(
        jumped
            .items
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        ["t2"]
    );
    assert!(jumped.has_more);

    // Server-side title sort spans the whole playlist.
    let sorted = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: pid.clone(),
            sort_by: Some("title".into()),
            ..Default::default()
        })
        .expect("sorted");
    assert_eq!(
        sorted
            .items
            .iter()
            .map(|t| t.title.clone())
            .collect::<Vec<_>>(),
        ["Alpha", "Bravo", "Charlie"]
    );

    // A cursor from one sort/query is rejected under another.
    let err = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: pid.clone(),
            cursor: page1.next_cursor,
            sort_by: Some("title".into()),
            ..Default::default()
        })
        .unwrap_err();
    assert!(matches!(err, BackendError::Internal(_)));

    // Query filters and still reports whole-playlist... no -- total is the
    // filtered count, duration spans the filtered set.
    let filtered = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: pid,
            query: "alph".into(),
            ..Default::default()
        })
        .expect("filtered");
    assert_eq!(filtered.total, 1);
    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].id, "t2");
}

#[test]
fn remove_tracks_from_playlist_recompacts_positions() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song A",
        "Artist",
        "/music/a.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t2",
        "Song B",
        "Artist",
        "/music/b.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t3",
        "Song C",
        "Artist",
        "/music/c.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        })
        .expect("add tracks");

    let removed = service
        .remove_tracks_from_playlist(RemoveTracksFromPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t2".to_string()],
        })
        .expect("remove track");
    assert_eq!(removed.removed, 1);

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(
        tracks
            .items
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        vec!["t1".to_string(), "t3".to_string()]
    );
}

#[test]
fn reorder_playlist_tracks_applies_arbitrary_new_order() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song A",
        "Artist",
        "/music/a.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t2",
        "Song B",
        "Artist",
        "/music/b.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t3",
        "Song C",
        "Artist",
        "/music/c.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        })
        .expect("add tracks");

    let reordered = service
        .reorder_playlist_tracks(ReorderPlaylistTracksRequest {
            playlist_id: playlist.playlist_id.clone(),
            ordered_track_ids: vec!["t3".to_string(), "t1".to_string(), "t2".to_string()],
            ..Default::default()
        })
        .expect("reorder tracks");
    assert_eq!(reordered.reordered, 3);

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(
        tracks
            .items
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        vec!["t3".to_string(), "t1".to_string(), "t2".to_string()]
    );
}

#[test]
fn reorder_playlist_tracks_sort_commit_mode_orders_the_whole_playlist() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn, "t1", "Charlie", "Artist", "/m/c.mp3", None, None, false,
    );
    insert_full_track(
        &conn, "t2", "Alpha", "Artist", "/m/a.mp3", None, None, false,
    );
    insert_full_track(
        &conn, "t3", "Bravo", "Artist", "/m/b.mp3", None, None, false,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        })
        .expect("add tracks");

    let reordered = service
        .reorder_playlist_tracks(ReorderPlaylistTracksRequest {
            playlist_id: playlist.playlist_id.clone(),
            sort_by: Some("title".to_string()),
            sort_dir: Some("asc".to_string()),
            ..Default::default()
        })
        .expect("sort-commit reorder");
    assert_eq!(reordered.reordered, 3);

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(
        tracks
            .items
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        vec!["t2".to_string(), "t3".to_string(), "t1".to_string()]
    );
}

#[test]
fn reorder_playlist_tracks_single_move_mode_repositions_one_track() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(&conn, "t1", "One", "Artist", "/m/1.mp3", None, None, false);
    insert_full_track(&conn, "t2", "Two", "Artist", "/m/2.mp3", None, None, false);
    insert_full_track(
        &conn, "t3", "Three", "Artist", "/m/3.mp3", None, None, false,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        })
        .expect("add tracks");

    // Move t3 to before t2 -> [t1, t3, t2].
    service
        .reorder_playlist_tracks(ReorderPlaylistTracksRequest {
            playlist_id: playlist.playlist_id.clone(),
            move_track_id: Some("t3".to_string()),
            before_track_id: Some("t2".to_string()),
            ..Default::default()
        })
        .expect("single move");
    let order = |svc: &Service| {
        svc.get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id.clone(),
            ..Default::default()
        })
        .expect("get playlist tracks")
        .items
        .iter()
        .map(|t| t.id.clone())
        .collect::<Vec<_>>()
    };
    assert_eq!(order(&service), vec!["t1", "t3", "t2"]);

    // before_track_id absent -> move t1 to the end -> [t3, t2, t1].
    service
        .reorder_playlist_tracks(ReorderPlaylistTracksRequest {
            playlist_id: playlist.playlist_id.clone(),
            move_track_id: Some("t1".to_string()),
            ..Default::default()
        })
        .expect("move to end");
    assert_eq!(order(&service), vec!["t3", "t2", "t1"]);

    // Unknown move_track_id is rejected.
    assert!(
        service
            .reorder_playlist_tracks(ReorderPlaylistTracksRequest {
                playlist_id: playlist.playlist_id.clone(),
                move_track_id: Some("nope".to_string()),
                ..Default::default()
            })
            .is_err()
    );
}

#[test]
fn reorder_playlist_tracks_rejects_mismatched_track_set() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song A",
        "Artist",
        "/music/a.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t2",
        "Song B",
        "Artist",
        "/music/b.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string()],
        })
        .expect("add tracks");

    let missing_track = service.reorder_playlist_tracks(ReorderPlaylistTracksRequest {
        playlist_id: playlist.playlist_id.clone(),
        ordered_track_ids: vec!["t1".to_string()],
        ..Default::default()
    });
    assert!(missing_track.is_err());

    let unknown_track = service.reorder_playlist_tracks(ReorderPlaylistTracksRequest {
        playlist_id: playlist.playlist_id.clone(),
        ordered_track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        ..Default::default()
    });
    assert!(unknown_track.is_err());

    let duplicate_track = service.reorder_playlist_tracks(ReorderPlaylistTracksRequest {
        playlist_id: playlist.playlist_id.clone(),
        ordered_track_ids: vec!["t1".to_string(), "t1".to_string()],
        ..Default::default()
    });
    assert!(duplicate_track.is_err());

    // No-mode request is rejected.
    let no_mode = service.reorder_playlist_tracks(ReorderPlaylistTracksRequest {
        playlist_id: playlist.playlist_id,
        ..Default::default()
    });
    assert!(no_mode.is_err());
}

#[test]
fn frontend_settings_roundtrip_and_reject_unsupported_key() {
    let (_dir, service) = test_service();
    let defaults = service.get_frontend_settings().expect("get settings");
    assert!(defaults.values.is_empty());

    service
        .set_frontend_setting(SetFrontendSettingRequest {
            key: SETTING_UI_THEME.to_string(),
            value: Some("dark".to_string()),
        })
        .expect("set theme");
    let after_set = service.get_frontend_settings().expect("get settings");
    assert_eq!(
        after_set.values.get(SETTING_UI_THEME).map(String::as_str),
        Some("dark")
    );

    service
        .set_frontend_setting(SetFrontendSettingRequest {
            key: SETTING_UI_THEME.to_string(),
            value: None,
        })
        .expect("clear theme");
    let after_clear = service.get_frontend_settings().expect("get settings");
    assert!(!after_clear.values.contains_key(SETTING_UI_THEME));

    let err = service
        .set_frontend_setting(SetFrontendSettingRequest {
            key: "not_a_real_setting".to_string(),
            value: Some("x".to_string()),
        })
        .unwrap_err();
    assert!(matches!(err, BackendError::Internal(_)));
}

#[test]
fn remove_essentia_removes_dir_and_clears_setting() {
    let (_dir, service) = test_service();
    let essentia_dir = service.db.data_dir().join("essentia");
    std::fs::create_dir_all(&essentia_dir).expect("create essentia dir");
    std::fs::write(essentia_dir.join("marker.txt"), b"x").expect("write marker");
    service
        .set_frontend_setting(SetFrontendSettingRequest {
            key: SETTING_UI_ANALYSIS_ENGINE.to_string(),
            value: Some("essentia".to_string()),
        })
        .expect("set analysis engine");

    service.remove_essentia().expect("remove essentia");

    assert!(!essentia_dir.exists());
    let settings = service.get_frontend_settings().expect("get settings");
    assert!(!settings.values.contains_key(SETTING_UI_ANALYSIS_ENGINE));
}

#[test]
fn browse_source_files_empty_without_roots_or_rekordbox_db() {
    let (_dir, service) = test_service();
    let result = service
        .browse_source_files(BrowseSourceFilesRequest {
            query: String::new(),
            limit: 100,
            cursor: None,
            ..Default::default()
        })
        .expect("browse source files");
    assert_eq!(result.total, 0);
    assert!(result.items.is_empty());
}

#[test]
fn browse_source_files_includes_rekordbox_db_tracks_when_requested() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Master Song",
        "Artist",
        "/master/a.mp3",
        None,
        None,
        true,
    );
    drop(conn);

    set_library_sources(&service, &[], true, false);
    let result = service
        .browse_source_files(BrowseSourceFilesRequest {
            query: String::new(),
            limit: 100,
            cursor: None,
            ..Default::default()
        })
        .expect("browse source files");
    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].id, "t1");
    assert!(result.items[0].rekordbox_db_source);
}

#[test]
fn browse_source_files_computes_duration_total_over_countable_tracks_only() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    // Fully core-analyzed (bpm + waveform + duration): countable via the
    // primary rule.
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, waveform_peaks_path, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES ('t1', 'Analyzed', 'Artist', '/master/a.mp3', 200000, 120.0, '/data/a.dat', ?1, 1, datetime('now'), datetime('now'))",
        params![build_track_match_fingerprint("Analyzed", "Artist", None)],
    )
    .expect("insert t1");
    // master.db track with only a duration: countable via the
    // rekordbox-db-specific OR-branch, not the primary rule.
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES ('t2', 'Master Only', 'Artist', '/master/b.mp3', 100000, ?1, 1, datetime('now'), datetime('now'))",
        params![build_track_match_fingerprint("Master Only", "Artist", None)],
    )
    .expect("insert t2");
    // No analysis at all: not countable, excluded from both the sum and
    // counted as without length, and still present in `total`.
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES ('t3', 'Unanalyzed', 'Artist', '/master/c.mp3', ?1, 1, datetime('now'), datetime('now'))",
        params![build_track_match_fingerprint("Unanalyzed", "Artist", None)],
    )
    .expect("insert t3");
    drop(conn);

    set_library_sources(&service, &[], true, false);
    let result = service
        .browse_source_files(BrowseSourceFilesRequest {
            query: String::new(),
            limit: 100,
            cursor: None,
            ..Default::default()
        })
        .expect("browse source files");
    assert_eq!(result.total, 3);
    assert_eq!(result.duration_unknown_count, 1);
    assert_eq!(result.total_duration_ms, 300_000);
}

#[test]
fn browse_source_files_sorts_server_side_across_pages() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    for (id, title) in [("m1", "Zeta"), ("m2", "Alpha"), ("m3", "Mu")] {
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, match_fingerprint, rekordbox_db_source, created_at, updated_at)
             VALUES (?1, ?2, 'Artist', ?3, ?4, 1, datetime('now'), datetime('now'))",
            params![id, title, format!("/master/{id}.mp3"), build_track_match_fingerprint(title, "Artist", None)],
        )
        .expect("insert");
    }
    drop(conn);

    set_library_sources(&service, &[], true, false);
    let req = |cursor: Option<String>| BrowseSourceFilesRequest {
        limit: 2,
        cursor,
        sort_by: Some("title".to_string()),
        sort_dir: Some("desc".to_string()),
        ..Default::default()
    };
    let page1 = service.browse_source_files(req(None)).expect("page 1");
    assert_eq!(
        page1
            .items
            .iter()
            .map(|t| t.title.clone())
            .collect::<Vec<_>>(),
        ["Zeta", "Mu"]
    );
    assert!(page1.has_more);
    let page2 = service
        .browse_source_files(req(page1.next_cursor))
        .expect("page 2");
    assert_eq!(
        page2
            .items
            .iter()
            .map(|t| t.title.clone())
            .collect::<Vec<_>>(),
        ["Alpha"]
    );

    // An offset jumps straight to a position in the sorted list.
    let jumped = service
        .browse_source_files(BrowseSourceFilesRequest {
            offset: Some(1),
            limit: 1,
            ..req(None)
        })
        .expect("offset page");
    assert_eq!(
        jumped
            .items
            .iter()
            .map(|t| t.title.clone())
            .collect::<Vec<_>>(),
        ["Mu"]
    );
    assert!(jumped.has_more);
    assert_eq!(jumped.total, 3);
}

#[test]
fn track_rows_carry_analysis_ready_computed_from_db_fields() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    // Fully core-analyzed: bpm + waveform path + duration.
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, waveform_peaks_path, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES ('ready', 'Ready', 'Artist', '/master/ready.mp3', 200000, 120.0, '/data/ready.dat', ?1, 1, datetime('now'), datetime('now'))",
        params![build_track_match_fingerprint("Ready", "Artist", None)],
    )
    .expect("insert ready");
    // Has duration + bpm but no waveform path -> still needs analysis.
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES ('missing', 'Missing', 'Artist', '/master/missing.mp3', 200000, 120.0, ?1, 1, datetime('now'), datetime('now'))",
        params![build_track_match_fingerprint("Missing", "Artist", None)],
    )
    .expect("insert missing");
    drop(conn);

    set_library_sources(&service, &[], true, false);
    let result = service
        .browse_source_files(BrowseSourceFilesRequest {
            query: String::new(),
            limit: 100,
            cursor: None,
            ..Default::default()
        })
        .expect("browse source files");
    let by_id = |id: &str| {
        result
            .items
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .analysis_ready
    };
    assert!(by_id("ready"));
    assert!(!by_id("missing"));
}

#[test]
fn track_rows_derive_format_ext_from_path_when_the_column_is_null() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    // master.db import path leaves format_ext NULL.
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES ('m1', 'Song', 'Artist', '/master/Artist/Song.FLAC', ?1, 1, datetime('now'), datetime('now'))",
        params![build_track_match_fingerprint("Song", "Artist", None)],
    )
    .expect("insert null-format row");
    drop(conn);

    set_library_sources(&service, &[], true, false);
    let browsed = service
        .browse_source_files(BrowseSourceFilesRequest {
            query: String::new(),
            limit: 100,
            cursor: None,
            ..Default::default()
        })
        .expect("browse source files");
    assert_eq!(
        browsed
            .items
            .iter()
            .find(|t| t.id == "m1")
            .unwrap()
            .format_ext
            .as_deref(),
        Some("flac"),
        "row_to_track must fall back to the file-path extension"
    );

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["m1".to_string()],
        })
        .expect("add track");
    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(tracks.items[0].format_ext.as_deref(), Some("flac"));
}

#[test]
fn browse_source_files_lists_only_switched_on_present_folders() {
    let (_dir, service) = test_service();
    let root_a = tempfile::tempdir().expect("root a");
    let root_b = tempfile::tempdir().expect("root b");
    copy_audio_fixture(root_a.path(), "formats/track_format_wav.wav", "a.wav");
    copy_audio_fixture(root_b.path(), "formats/track_format_wav.wav", "b.wav");
    let root_a_str = root_a.path().to_string_lossy().to_string();
    let root_b_str = root_b.path().to_string_lossy().to_string();
    let gone = "/definitely/does/not/exist/anywhere";
    set_library_sources(&service, &[&root_a_str, &root_b_str, gone], false, false);
    let listed = || -> Vec<String> {
        let mut names: Vec<String> = service
            .browse_source_files(BrowseSourceFilesRequest {
                limit: 100,
                ..Default::default()
            })
            .expect("browse source files")
            .items
            .into_iter()
            .map(|track| file_name(&track.file_path))
            .collect();
        names.sort();
        names
    };
    assert_eq!(listed(), ["a.wav", "b.wav"]);

    service
        .set_source_folder_enabled(SetSourceFolderEnabledRequest {
            path: root_b_str.clone(),
            enabled: false,
        })
        .expect("switch off");
    assert_eq!(listed(), ["a.wav"]);
}

#[test]
fn browse_source_files_lists_a_subfolders_tracks_once() {
    let (_dir, service) = test_service();
    let parent = tempfile::tempdir().expect("parent");
    let parent_str = parent.path().to_string_lossy().to_string();
    let child_str = format!("{parent_str}/sub");
    std::fs::create_dir(&child_str).expect("subfolder");
    let conn = service.db.connect().expect("connect");
    for (id, folder) in [("top", &parent_str), ("nested", &child_str)] {
        let path = format!("{folder}/{id}.mp3");
        std::fs::write(&path, b"data").expect("write track");
        insert_full_track(&conn, id, id, "Artist", &path, None, None, false);
    }
    drop(conn);
    set_library_sources(&service, &[&parent_str, &child_str], false, false);
    let listed = || {
        let data = service
            .browse_source_files(BrowseSourceFilesRequest {
                limit: 100,
                ..Default::default()
            })
            .expect("browse source files");
        let mut ids: Vec<String> = data.items.into_iter().map(|track| track.id).collect();
        ids.sort();
        (ids, data.total, data.list_track_count)
    };
    assert_eq!(listed(), (vec!["nested".into(), "top".into()], 2, 2));
    let filtering = || {
        service
            .get_library_sources()
            .expect("library sources")
            .filtering
    };
    assert!(!filtering());

    // Switching the parent off leaves the subfolder as a filter.
    service
        .set_source_folder_enabled(SetSourceFolderEnabledRequest {
            path: parent_str.clone(),
            enabled: false,
        })
        .expect("switch off");
    assert_eq!(listed(), (vec!["nested".into()], 1, 1));
    assert!(filtering());
}

#[test]
fn get_library_sources_nests_subfolders_under_their_parent() {
    let (_dir, service) = test_service();
    let base = tempfile::tempdir().expect("base");
    let base_str = base.path().to_string_lossy().to_string();
    let music = format!("{base_str}/music");
    let other = format!("{base_str}/other");
    for dir in [
        format!("{music}/a/deep"),
        format!("{music}/b"),
        other.clone(),
    ] {
        std::fs::create_dir_all(dir).expect("folder");
    }
    // `link/b` names `music/b` through a symlink.
    let link = format!("{base_str}/link");
    std::os::unix::fs::symlink(&music, &link).expect("symlink");
    let deep = format!("{music}/a/deep");
    let a = format!("{music}/a");
    let linked_b = format!("{link}/b");
    set_library_sources(
        &service,
        &[&deep, &other, &music, &linked_b, &a],
        false,
        false,
    );

    let folders: Vec<(String, String, Option<String>)> = service
        .get_library_sources()
        .expect("sources")
        .folders
        .into_iter()
        .map(|folder| (folder.path, folder.label, folder.parent))
        .collect();
    assert_eq!(
        folders,
        [
            (other.clone(), other.clone(), None),
            (music.clone(), music.clone(), None),
            (linked_b.clone(), "b/".to_string(), Some(music.clone())),
            (a.clone(), "a/".to_string(), Some(music.clone())),
            (deep.clone(), "deep/".to_string(), Some(a.clone())),
        ]
    );
}

#[test]
fn get_library_sources_reports_each_folder_on_its_own() {
    let (_dir, service) = test_service();
    let root_a = tempfile::tempdir().expect("root a");
    let root_b = tempfile::tempdir().expect("root b");
    copy_audio_fixture(root_a.path(), "formats/track_format_wav.wav", "alpha.wav");
    copy_audio_fixture(root_b.path(), "formats/track_format_wav.wav", "bravo.wav");
    copy_audio_fixture(root_b.path(), "formats/track_format_wav.wav", "charlie.wav");
    let root_a_str = root_a.path().to_string_lossy().to_string();
    let root_b_str = root_b.path().to_string_lossy().to_string();
    let gone = "/definitely/does/not/exist/anywhere";
    set_library_sources(&service, &[&root_a_str, &root_b_str, gone], false, false);

    // root_a: its one track analyzed. root_b: one analyzed, one not.
    let conn = service.db.connect().expect("connect");
    for path in [
        root_a.path().join("alpha.wav"),
        root_b.path().join("bravo.wav"),
    ] {
        let updated = conn
            .execute(
                "UPDATE tracks SET duration_ms = 200000, bpm = 120.0, waveform_peaks_path = '/data/w.dat'
                 WHERE file_path = ?1",
                params![path.to_string_lossy().to_string()],
            )
            .expect("mark analyzed");
        assert_eq!(updated, 1, "{} indexed", path.display());
    }
    drop(conn);
    // A switched-off folder still reports its own status.
    service
        .set_source_folder_enabled(SetSourceFolderEnabledRequest {
            path: root_a_str.clone(),
            enabled: false,
        })
        .expect("switch off");

    let sources = service.get_library_sources().expect("library sources");
    let summary: Vec<(&str, bool, bool, bool)> = sources
        .folders
        .iter()
        .map(|f| (f.path.as_str(), f.enabled, f.missing, f.fully_analyzed))
        .collect();
    assert_eq!(
        summary,
        [
            (root_a_str.as_str(), false, false, true),
            (root_b_str.as_str(), true, false, false),
            (gone, true, true, false),
        ]
    );
    assert!(sources.ever_configured);
}

#[test]
fn add_source_folders_adds_only_new_folders_switched_on() {
    let (_dir, service) = test_service();
    assert!(
        !service
            .get_library_sources()
            .expect("sources")
            .ever_configured
    );
    let add = |paths: &[&str]| {
        service
            .add_source_folders(AddSourceFoldersRequest {
                paths: paths.iter().map(|p| p.to_string()).collect(),
            })
            .expect("add source folders")
    };
    assert_eq!(
        add(&["/music/a", "/music/b"]).added,
        ["/music/a", "/music/b"]
    );
    let again = add(&["/music/b", "/music/c"]);
    assert_eq!(again.added, ["/music/c"]);
    let folders: Vec<(&str, bool)> = again
        .sources
        .folders
        .iter()
        .map(|f| (f.path.as_str(), f.enabled))
        .collect();
    assert_eq!(
        folders,
        [("/music/a", true), ("/music/b", true), ("/music/c", true)]
    );
    assert!(again.sources.ever_configured);
}

#[test]
fn relocate_source_root_updates_matching_track_paths() {
    let (_dir, service) = test_service();
    let old_root = tempfile::tempdir().expect("old root");
    let new_root = tempfile::tempdir().expect("new root");
    std::fs::write(new_root.path().join("song.mp3"), b"data").expect("write new file");

    let old_path = old_root.path().join("song.mp3");
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song",
        "Artist",
        old_path.to_str().unwrap(),
        None,
        None,
        false,
    );
    drop(conn);

    let result = service
        .relocate_source_root(RelocateSourceRootRequest {
            old_root: old_root.path().to_string_lossy().to_string(),
            new_root: new_root.path().to_string_lossy().to_string(),
        })
        .expect("relocate source root");
    assert_eq!(result.matched, 1);
    assert_eq!(result.updated, 1);

    let conn = service.db.connect().expect("connect");
    let new_path: String = conn
        .query_row("SELECT file_path FROM tracks WHERE id = 't1'", [], |r| {
            r.get(0)
        })
        .expect("read updated path");
    assert_eq!(new_path, new_root.path().join("song.mp3").to_string_lossy());
}

#[test]
fn relocate_source_root_replaces_the_folder_once_every_track_moved() {
    let (_dir, service) = test_service();
    let old_root = tempfile::tempdir().expect("old root");
    let new_root = tempfile::tempdir().expect("new root");
    let old_str = old_root.path().to_string_lossy().to_string();
    let new_str = new_root.path().to_string_lossy().to_string();
    // Configured before the tracks go in: adding a folder indexes it.
    set_library_sources(&service, &["/music/other", &old_str], false, false);
    // Only "moved.mp3" exists at the new root; "left.mp3" can't move.
    std::fs::write(new_root.path().join("moved.mp3"), b"data").expect("write new file");
    let conn = service.db.connect().expect("connect");
    for id in ["moved", "left"] {
        let path = old_root.path().join(format!("{id}.mp3"));
        insert_full_track(
            &conn,
            id,
            id,
            "Artist",
            path.to_str().unwrap(),
            None,
            None,
            false,
        );
    }
    drop(conn);
    service
        .set_source_folder_enabled(SetSourceFolderEnabledRequest {
            path: old_str.clone(),
            enabled: false,
        })
        .expect("switch off");
    let relocate = || {
        service
            .relocate_source_root(RelocateSourceRootRequest {
                old_root: old_str.clone(),
                new_root: new_str.clone(),
            })
            .expect("relocate source root")
    };
    let folders = |data: &RelocateSourceRootData| -> Vec<(String, bool)> {
        data.sources
            .folders
            .iter()
            .map(|f| (f.path.clone(), f.enabled))
            .collect()
    };

    // A track left behind keeps the old folder; the new one joins it, with
    // the old one's switch.
    let partial = relocate();
    assert_eq!(partial.missing_at_new_root, 1);
    assert_eq!(
        partial.status_text,
        "Source relocated: 1 track path updated | 1 track still unresolved"
    );
    // "/music/other" is switched on but missing, so it counts as off.
    assert_eq!(
        partial.sources.filter_status_text,
        "Source filters: 0/3 enabled"
    );
    assert!(partial.sources.filtering);
    assert_eq!(
        folders(&partial),
        [
            ("/music/other".to_string(), true),
            (old_str.clone(), false),
            (new_str.clone(), false),
        ]
    );

    // Once the last track can move too, the old folder goes.
    std::fs::write(new_root.path().join("left.mp3"), b"data").expect("write new file");
    let full = relocate();
    assert_eq!(full.missing_at_new_root, 0);
    assert_eq!(full.status_text, "Source relocated: 1 track path updated");
    assert_eq!(
        folders(&full),
        [("/music/other".to_string(), true), (new_str.clone(), false)]
    );
}

#[test]
fn relocate_source_root_rejects_same_root() {
    let (_dir, service) = test_service();
    let err = service
        .relocate_source_root(RelocateSourceRootRequest {
            old_root: "/music".to_string(),
            new_root: "/music/".to_string(),
        })
        .unwrap_err();
    assert!(matches!(err, BackendError::Validation(_)));
}

#[test]
fn relocate_source_root_accepts_a_case_only_rename() {
    let (_dir, service) = test_service();
    let base = tempfile::tempdir().expect("base");
    let old_root = base.path().join("music").join("Sets");
    let new_root = base.path().join("Music").join("Sets");
    std::fs::create_dir_all(&new_root).expect("create new root");
    std::fs::write(new_root.join("song.mp3"), b"data").expect("write new file");

    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song",
        "Artist",
        old_root.join("song.mp3").to_str().unwrap(),
        None,
        None,
        false,
    );
    drop(conn);

    let result = service
        .relocate_source_root(RelocateSourceRootRequest {
            old_root: old_root.to_string_lossy().to_string(),
            new_root: new_root.to_string_lossy().to_string(),
        })
        .expect("a case-only rename is a real move on Linux");
    assert_eq!(result.updated, 1);

    let conn = service.db.connect().expect("connect");
    let new_path: String = conn
        .query_row("SELECT file_path FROM tracks WHERE id = 't1'", [], |r| {
            r.get(0)
        })
        .expect("read updated path");
    assert_eq!(new_path, new_root.join("song.mp3").to_string_lossy());
}

#[test]
fn materialize_source_track_inserts_new_then_updates_existing() {
    let (_dir, service) = test_service();
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("song.mp3");
    std::fs::write(&file_path, b"data").expect("write file");

    let inserted = service
        .materialize_source_track(MaterializeSourceTrackRequest {
            file_path: file_path.to_string_lossy().to_string(),
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            track_number: None,
            key: None,
            file_size_bytes: None,
            format_ext: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
        })
        .expect("materialize new track");

    let updated = service
        .materialize_source_track(MaterializeSourceTrackRequest {
            file_path: file_path.to_string_lossy().to_string(),
            title: "Song Updated".to_string(),
            artist: "Artist".to_string(),
            album: None,
            track_number: None,
            key: None,
            file_size_bytes: None,
            format_ext: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
        })
        .expect("materialize existing track");
    assert_eq!(inserted.track_id, updated.track_id);

    let conn = service.db.connect().expect("connect");
    let title: String = conn
        .query_row(
            "SELECT title FROM tracks WHERE id = ?1",
            params![updated.track_id],
            |r| r.get(0),
        )
        .expect("read title");
    assert_eq!(title, "Song Updated");
}

#[test]
fn materialize_source_track_rejects_missing_file() {
    let (_dir, service) = test_service();
    let err = service
        .materialize_source_track(MaterializeSourceTrackRequest {
            file_path: "/definitely/does/not/exist.mp3".to_string(),
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            track_number: None,
            key: None,
            file_size_bytes: None,
            format_ext: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
        })
        .unwrap_err();
    assert!(matches!(err, BackendError::NotFound(_)));
}

#[test]
fn resolve_track_identity_materializes_safe_local_source_path() {
    let (_dir, service) = test_service();
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("song.mp3");
    std::fs::write(&file_path, b"data").expect("write file");

    let result = service
        .resolve_track_identity(ResolveTrackIdentityRequest {
            track_id: Some(file_path.to_string_lossy().to_string()),
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            file_path: Some(file_path.to_string_lossy().to_string()),
            file_size_bytes: None,
            track_number: None,
            key: Some("8A".to_string()),
            format_ext: Some("mp3".to_string()),
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            usb_root: Some("/usb".to_string()),
            usb_root_valid: true,
            usb_analysis_path: None,
        })
        .expect("resolve identity");

    assert!(result.materialized);
    assert_eq!(
        result.resolved_by,
        backend::models::ResolvedBy::Materialized
    );
    assert!(result.track_id.is_some());
    // The new library row, shown in place of the file's row.
    let track = result.track.expect("materialized row");
    assert_eq!(Some(track.id), result.track_id);
    assert_eq!(track.title, "Song");
}

#[test]
fn resolve_track_identity_skips_usb_materialization_and_resolves_local_match() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "local-1",
        "Song",
        "Artist",
        "/music/song.mp3",
        None,
        Some(1234),
        false,
    );
    drop(conn);

    let result = service
        .resolve_track_identity(ResolveTrackIdentityRequest {
            track_id: Some("usb-placeholder".to_string()),
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            file_path: Some("/usb/Contents/song.mp3".to_string()),
            file_size_bytes: Some(1234),
            track_number: None,
            key: None,
            format_ext: Some("mp3".to_string()),
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            usb_root: Some("/usb".to_string()),
            usb_root_valid: true,
            usb_analysis_path: Some("/usb/PIONEER/USBANLZ/ANLZ0000.DAT".to_string()),
        })
        .expect("resolve identity");

    assert!(!result.materialized);
    assert_eq!(result.resolved_by, backend::models::ResolvedBy::Hash);
    assert_eq!(result.track_id.as_deref(), Some("local-1"));
    assert!(result.track.is_none());
}

#[test]
fn remove_source_folder_drops_it_and_deletes_its_tracks() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "A",
        "Artist",
        "/music/keep/a.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t2",
        "B",
        "Artist",
        "/music/drop/b.mp3",
        None,
        None,
        false,
    );
    drop(conn);
    set_library_sources(&service, &["/music/keep", "/music/drop"], false, false);

    let result = service
        .remove_source_folder(RemoveSourceFolderRequest {
            path: "/music/drop".to_string(),
        })
        .expect("remove source folder");
    assert_eq!(result.removed, 1);
    assert_eq!(
        result.status_text,
        "Source folders: 1 | removed 1 track from library"
    );
    let folders: Vec<&str> = result
        .sources
        .folders
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(folders, ["/music/keep"]);

    let conn = service.db.connect().expect("connect");
    let remaining: Vec<String> = conn
        .prepare("SELECT id FROM tracks")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("ids");
    assert_eq!(remaining, ["t1"]);
}

#[test]
fn remove_source_folder_keeps_tracks_another_folder_covers() {
    let (_dir, service) = test_service();
    set_library_sources(&service, &["/music", "/music/sub"], false, false);
    let conn = service.db.connect().expect("connect");
    for (id, path) in [
        ("top", "/music/a.mp3"),
        ("sub", "/music/sub/b.mp3"),
        ("other", "/elsewhere/c.mp3"),
    ] {
        insert_full_track(&conn, id, id, "Artist", path, None, None, false);
    }
    drop(conn);
    let remaining = || -> Vec<String> {
        let conn = service.db.connect().expect("connect");
        conn.prepare("SELECT id FROM tracks ORDER BY id")
            .expect("prepare")
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("ids")
    };
    let remove = |root: &str| {
        service
            .remove_source_folder(RemoveSourceFolderRequest {
                path: root.to_string(),
            })
            .expect("remove source folder")
            .removed
    };

    // The parent still covers the subfolder's track.
    assert_eq!(remove("/music/sub"), 0);
    assert_eq!(remaining(), ["other", "sub", "top"]);

    // The still-configured subfolder keeps its track when the parent goes.
    set_library_sources(&service, &["/music/sub"], false, false);
    assert_eq!(remove("/music"), 1);
    assert_eq!(remaining(), ["other", "sub"]);
}

#[test]
fn resolve_playback_source_fast_path_by_track_id() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song",
        "Artist",
        "/music/a.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let result = service
        .resolve_playback_source(ResolvePlaybackSourceRequest {
            title: String::new(),
            artist: String::new(),
            album: None,
            bpm: None,
            file_path: None,
            file_size_bytes: None,
            track_id: Some("t1".to_string()),
        })
        .expect("resolve playback source");
    assert_eq!(result.matched_by, backend::models::TrackMatch::SelfId);
    assert_eq!(result.resolved_path.as_deref(), Some("/music/a.mp3"));
    assert_eq!(result.track_id.as_deref(), Some("t1"));
}

#[test]
fn resolve_playback_source_falls_back_to_fingerprint_hash_match() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song",
        "Artist",
        "/music/a.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let result = service
        .resolve_playback_source(ResolvePlaybackSourceRequest {
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            file_path: None,
            file_size_bytes: None,
            track_id: None,
        })
        .expect("resolve playback source");
    assert_eq!(result.matched_by, backend::models::TrackMatch::Hash);
    assert_eq!(result.track_id.as_deref(), Some("t1"));
}

#[test]
fn resolve_playback_source_fast_path_skips_row_whose_file_is_missing() {
    let (dir, service) = test_service();
    let live_path = dir.path().join("live.mp3");
    std::fs::write(&live_path, b"x").expect("write live file");
    let live_path = live_path.to_string_lossy().to_string();

    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "stale",
        "Song",
        "Artist",
        "/music/gone.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn, "live", "Song", "Artist", &live_path, None, None, false,
    );
    drop(conn);

    let result = service
        .resolve_playback_source(ResolvePlaybackSourceRequest {
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            file_path: None,
            file_size_bytes: None,
            track_id: Some("stale".to_string()),
        })
        .expect("resolve playback source");
    assert_eq!(result.resolved_path.as_deref(), Some(live_path.as_str()));
    assert_eq!(result.track_id.as_deref(), Some("live"));
}

#[test]
fn resolve_playback_source_best_candidate_prefers_file_on_disk() {
    let (dir, service) = test_service();
    let live_path = dir.path().join("live.mp3");
    std::fs::write(&live_path, b"x").expect("write live file");
    let live_path = live_path.to_string_lossy().to_string();

    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "stale",
        "Song",
        "Artist",
        "/music/gone.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn, "live", "Song", "Artist", &live_path, None, None, false,
    );
    drop(conn);

    let result = service
        .resolve_playback_source(ResolvePlaybackSourceRequest {
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            file_path: None,
            file_size_bytes: None,
            track_id: None,
        })
        .expect("resolve playback source");
    assert_eq!(result.resolved_path.as_deref(), Some(live_path.as_str()));
    assert_eq!(result.track_id.as_deref(), Some("live"));
}

fn song_request(file_path: Option<&str>) -> ResolvePlaybackSourceRequest {
    ResolvePlaybackSourceRequest {
        title: "Song".to_string(),
        artist: "Artist".to_string(),
        album: None,
        bpm: None,
        file_path: file_path.map(str::to_string),
        file_size_bytes: None,
        track_id: None,
    }
}

#[test]
fn resolve_playback_source_plays_own_local_file_over_stale_export_row() {
    // A library file not indexed yet (e.g. its row was removed) must play
    // itself, not an old export row that shares its title and artist.
    let (dir, service) = test_service();
    let own = dir.path().join("Song.mp3");
    std::fs::write(&own, b"x").expect("write own file");
    let own = own.to_string_lossy().to_string();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "stale-export",
        "Song",
        "Artist",
        "/gone/USB_OLD/Contents/Artist/Song.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let result = service
        .resolve_playback_source(song_request(Some(&own)))
        .expect("resolve playback source");
    assert_eq!(result.matched_by, backend::models::TrackMatch::OwnFile);
    assert_eq!(result.resolved_path.as_deref(), Some(own.as_str()));
    assert_eq!(result.track_id, None);

    let conn = service.db.connect().expect("connect");
    insert_full_track(&conn, "own", "Song", "Artist", &own, None, None, false);
    drop(conn);
    let result = service
        .resolve_playback_source(song_request(Some(&own)))
        .expect("resolve playback source");
    assert_eq!(result.matched_by, backend::models::TrackMatch::SelfId);
    assert_eq!(result.track_id.as_deref(), Some("own"));
}

#[test]
fn playlist_tracks_say_where_their_file_is_and_whether_its_usb_is_selected() {
    let (dir, service) = test_service();
    let stick = dir.path().join("STICK");
    std::fs::create_dir_all(stick.join("PIONEER")).expect("vendor dir");
    std::fs::create_dir_all(stick.join("Contents")).expect("contents dir");
    let on_stick = stick.join("Contents").join("Song.mp3");
    std::fs::write(&on_stick, b"x").expect("write stick file");
    let local = dir.path().join("local.mp3");
    std::fs::write(&local, b"x").expect("write local file");
    let gone = dir.path().join("gone.mp3");
    let conn = service.db.connect().expect("connect");
    for (id, path) in [("stick", &on_stick), ("local", &local), ("gone", &gone)] {
        insert_full_track(
            &conn,
            id,
            id,
            "Artist",
            &path.to_string_lossy(),
            None,
            None,
            false,
        );
    }
    drop(conn);
    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["stick".to_string(), "local".to_string(), "gone".to_string()],
        })
        .expect("add tracks");

    let read = |usb_root: Option<String>| {
        service
            .get_playlist_tracks(GetPlaylistTracksRequest {
                playlist_id: playlist.playlist_id.clone(),
                query: String::new(),
                sort_by: None,
                sort_dir: None,
                cursor: None,
                offset: None,
                limit: 0,
                usb_root,
            })
            .expect("playlist tracks")
            .items
            .into_iter()
            .map(|track| {
                let source = track.file_source.expect("file source");
                (source.kind, source.missing)
            })
            .collect::<Vec<_>>()
    };
    let expect = |stick_missing: bool| {
        vec![
            ("usb".to_string(), stick_missing),
            ("local".to_string(), false),
            ("local".to_string(), true),
        ]
    };

    // The stick's file is there, but no USB is selected, or another one is.
    assert_eq!(read(None), expect(true));
    let other = dir.path().join("OTHER").to_string_lossy().to_string();
    assert_eq!(read(Some(other)), expect(true));
    assert_eq!(
        read(Some(stick.to_string_lossy().to_string())),
        expect(false)
    );
}

#[test]
fn resolve_playback_source_falls_back_to_missing_file_row_when_nothing_on_disk() {
    // The USB fallback in play_resolved_track relies on still getting a
    // library path here even when its file is gone -- don't regress that.
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "stale",
        "Song",
        "Artist",
        "/music/gone.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let result = service
        .resolve_playback_source(ResolvePlaybackSourceRequest {
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            file_path: None,
            file_size_bytes: None,
            track_id: None,
        })
        .expect("resolve playback source");
    assert_eq!(result.matched_by, backend::models::TrackMatch::Hash);
    assert_eq!(result.resolved_path.as_deref(), Some("/music/gone.mp3"));
}

#[test]
fn resolve_playback_source_keeps_missing_self_row_as_last_resort() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "stale",
        "Song",
        "Artist",
        "/music/gone.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    // Metadata that matches nothing by fingerprint or title, so the search
    // passes come up empty and the missing-file self row is all we have.
    let result = service
        .resolve_playback_source(ResolvePlaybackSourceRequest {
            title: "Totally Different".to_string(),
            artist: "Nobody".to_string(),
            album: None,
            bpm: None,
            file_path: None,
            file_size_bytes: None,
            track_id: Some("stale".to_string()),
        })
        .expect("resolve playback source");
    assert_eq!(result.matched_by, backend::models::TrackMatch::SelfId);
    assert_eq!(result.resolved_path.as_deref(), Some("/music/gone.mp3"));
}

#[test]
fn resolve_playback_source_returns_none_for_blank_title_or_artist() {
    let (_dir, service) = test_service();
    let result = service
        .resolve_playback_source(ResolvePlaybackSourceRequest {
            title: "   ".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            file_path: None,
            file_size_bytes: None,
            track_id: None,
        })
        .expect("resolve playback source");
    assert_eq!(result.matched_by, backend::models::TrackMatch::NoMatch);
    assert!(result.resolved_path.is_none());
}
