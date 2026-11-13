//! Importing from other DJ libraries (rekordbox `master.db`, Mixxx
//! `mixxxdb.sqlite`) through the commands the import dialogs call: library
//! scans, playlist listing, and playlist / history / crate imports.

mod library_sources;

use std::fs;
use std::path::{Path, PathBuf};

use backend::commands::BackendCommands;
use backend::models::{
    BrowseSourceFilesRequest, ExternalLibrary, ExternalPlaylistKind, GetTrackDetailRequest,
    ImportExternalPlaylistData, ImportExternalPlaylistRequest, ImportExternalPlaylistsData,
    ImportExternalPlaylistsItem, ImportExternalPlaylistsRequest, ImportedPlaylistRef,
    ListExternalPlaylistsRequest, SaveTrackAnalysisEditsRequest, ScanMixxxDbRequest,
    ScanRekordboxDbRequest, TrackCueInput,
};
use backend::service::anlz::{WaveformData, build_anlz_dat_file, build_anlz_ext_file};
use backend::service::cues::DEFAULT_HOTCUE_COLOR_ID;
use backend::service::usb_vendor_compat::DEFAULT_MASTER_DB_KEY;
use rusqlite::{Connection, params};
use tempfile::{TempDir, tempdir};

// --- shared ------------------------------------------------------------

/// A fresh app: its data dir (kept alive by the `TempDir`) and its commands.
fn app() -> (TempDir, PathBuf, BackendCommands) {
    let dir = tempdir().expect("app data dir");
    let data_dir = dir.path().to_path_buf();
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    (dir, data_dir, backend)
}

/// The app's library database, for checks the commands don't report.
fn library_db(data_dir: &Path) -> Connection {
    Connection::open(data_dir.join("backend.db")).expect("open backend db")
}

/// Placeholder media files; the imports take their metadata from the source library.
fn media_files(dir: &Path, names: &[&str]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|name| {
            let path = dir.join(name);
            fs::write(&path, b"audio").expect("write media");
            path
        })
        .collect()
}

fn import_lists(
    backend: &BackendCommands,
    items: Vec<ImportExternalPlaylistsItem>,
) -> ImportExternalPlaylistsData {
    let response = backend.import_external_playlists(ImportExternalPlaylistsRequest { items });
    assert!(response.ok, "import playlists: {response:?}");
    response.data.expect("import data").data
}

fn import_item(
    library: ExternalLibrary,
    path: &Option<String>,
    kind: ExternalPlaylistKind,
    id: &str,
    force: bool,
) -> ImportExternalPlaylistsItem {
    ImportExternalPlaylistsItem {
        library,
        name: id.to_string(),
        request: ImportExternalPlaylistRequest {
            path: path.clone(),
            kind,
            id: id.to_string(),
            force,
        },
    }
}

/// Imports one list, as the dialog does with a single list ticked.
fn import_one(
    backend: &BackendCommands,
    library: ExternalLibrary,
    path: &Option<String>,
    kind: ExternalPlaylistKind,
    id: &str,
    force: bool,
) -> Result<ImportExternalPlaylistData, String> {
    let mut data = import_lists(backend, vec![import_item(library, path, kind, id, force)]);
    let outcome = data.results.remove(0);
    match (outcome.playlist, outcome.error) {
        (Some(playlist), _) => Ok(playlist),
        (None, Some(error)) => Err(error),
        (None, None) => panic!("import outcome has neither a playlist nor an error"),
    }
}

fn playlist_titles(data_dir: &Path, playlist_id: &str) -> Vec<String> {
    library_db(data_dir)
        .prepare(
            "SELECT t.title FROM playlist_tracks pt JOIN tracks t ON t.id = pt.track_id
             WHERE pt.playlist_id = ?1 ORDER BY pt.position",
        )
        .unwrap()
        .query_map(params![playlist_id], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn playlist_count(data_dir: &Path) -> i64 {
    library_db(data_dir)
        .query_row("SELECT COUNT(*) FROM playlists", [], |r| r.get(0))
        .unwrap()
}

fn track_id_by_title(data_dir: &Path, title: &str) -> String {
    library_db(data_dir)
        .query_row("SELECT id FROM tracks WHERE title = ?1", [title], |r| {
            r.get(0)
        })
        .expect("track id")
}

fn first_local_playlist_imported_from(backend: &BackendCommands) -> Option<String> {
    let state = backend.list_playlists().data.expect("playlists");
    state.playlists[0].imported_from.clone()
}

// --- rekordbox -----------------------------------------------------------

/// A `master.db` shaped like rekordbox 6/7's: text ids, `root` parents,
/// folders (Attribute 1) and a smart playlist (Attribute 4).
fn create_rekordbox_db_with_playlists(root: &Path, media: &[PathBuf]) -> PathBuf {
    let path = root.join("master.db");
    let conn = open_rekordbox_db(&path);
    conn.execute_batch(
        r#"
        CREATE TABLE djmdArtist (ID VARCHAR(255) PRIMARY KEY, Name TEXT);
        CREATE TABLE djmdAlbum (ID VARCHAR(255) PRIMARY KEY, Name TEXT);
        CREATE TABLE djmdKey (ID VARCHAR(255) PRIMARY KEY, ScaleName TEXT);
        CREATE TABLE djmdContent (
          ID VARCHAR(255) PRIMARY KEY, FolderPath TEXT, Title TEXT, SrcArtistName TEXT,
          ArtistID TEXT, AlbumID TEXT, BPM INTEGER, KeyID TEXT, Length INTEGER,
          AnalysisDataPath TEXT, ImagePath TEXT, GenreID TEXT, rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdGenre (ID VARCHAR(255) PRIMARY KEY, Name TEXT);
        INSERT INTO djmdGenre VALUES ('g1', 'Drum & Bass');
        CREATE TABLE djmdPlaylist (
          ID VARCHAR(255) PRIMARY KEY, Seq INTEGER, Name TEXT, Attribute INTEGER,
          ParentID VARCHAR(255), SmartList TEXT, rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdSongPlaylist (
          ID VARCHAR(255) PRIMARY KEY, PlaylistID VARCHAR(255), ContentID VARCHAR(255),
          TrackNo INTEGER, rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdHistory (
          ID VARCHAR(255) PRIMARY KEY, Seq INTEGER, Name TEXT, Attribute INTEGER,
          ParentID VARCHAR(255), DateCreated TEXT, rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdSongHistory (
          ID VARCHAR(255) PRIMARY KEY, HistoryID VARCHAR(255), ContentID VARCHAR(255),
          TrackNo INTEGER, rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdCue (
          ID VARCHAR(255) PRIMARY KEY, ContentID VARCHAR(255), InMsec INTEGER,
          OutMsec INTEGER, Kind INTEGER, Color INTEGER, ColorTableIndex INTEGER,
          Comment TEXT, rb_local_deleted INTEGER DEFAULT 0
        );
        INSERT INTO djmdCue VALUES
          ('q1', '103', 53, -1, 0, -1, NULL, NULL, 0),
          ('q2', '103', 54911, -1, 1, -1, 3, 'Drop', 0),
          ('q3', '103', 60000, -1, 5, -1, NULL, NULL, 0),
          ('q4', '103', 70000, -1, 2, -1, 99, NULL, 1);
        INSERT INTO djmdPlaylist VALUES
          ('10', 1, 'Sets', 1, 'root', NULL, 0),
          ('11', 1, 'Friday', 0, '10', NULL, 0),
          ('12', 2, 'Smart', 4, '10', '<NODE/>', 0),
          ('13', 2, 'Openers', 0, 'root', NULL, 0),
          ('14', 3, 'Removed', 0, 'root', NULL, 1),
          ('15', 4, 'All missing', 0, 'root', NULL, 0),
          ('16', 5, 'Empty', 0, 'root', NULL, 0);
        INSERT INTO djmdSongPlaylist VALUES
          ('a', '11', '103', 1, 0),
          ('b', '11', '101', 2, 0),
          ('c', '11', '102', 3, 0),
          ('d', '11', '101', 4, 0),
          ('e', '11', '104', 5, 0),
          ('f', '11', '105', 6, 0),
          ('g', '11', '102', 7, 1),
          ('h', '13', '102', 1, 0),
          ('i', '14', '101', 1, 0),
          ('j', '15', '105', 1, 0);
        INSERT INTO djmdHistory VALUES
          ('2025', 1, '2025', 1, 'root', '2025-01-01 10:00:00', 0),
          ('202501', 1, '1', 1, '2025', '2025-01-01 10:00:00', 0),
          ('h1', 1, 'HISTORY 2025-01-01', 0, '202501', '2025-01-01 10:00:00', 0),
          ('h2', 2, 'HISTORY 2025-01-02', 0, '202501', '2025-01-02 10:00:00', 0);
        INSERT INTO djmdSongHistory VALUES
          ('x', 'h1', '101', 1, 0),
          ('y', 'h2', '103', 1, 0),
          ('z', 'h2', '102', 2, 0);
        "#,
    )
    .expect("create schema");
    let rows = [
        ("101", media[0].to_str().unwrap(), "Alpha", 0),
        ("102", media[1].to_str().unwrap(), "Bravo", 0),
        ("103", media[2].to_str().unwrap(), "Charlie", 0),
        ("104", media[0].to_str().unwrap(), "Deleted", 1),
        ("105", "/nowhere/missing.mp3", "Missing", 0),
    ];
    for (id, file_path, title, deleted) in rows {
        conn.execute(
            "INSERT INTO djmdContent (ID, FolderPath, Title, SrcArtistName, BPM, Length, GenreID, rb_local_deleted)
             VALUES (?1, ?2, ?3, 'Artist', 12800, 200, 'g1', ?4)",
            params![id, file_path, title, deleted],
        )
        .expect("insert content");
    }
    path
}

fn open_rekordbox_db(path: &Path) -> Connection {
    let conn = Connection::open(path).expect("open master db");
    conn.execute_batch(&format!("PRAGMA key='{DEFAULT_MASTER_DB_KEY}';"))
        .expect("key");
    conn
}

/// A rekordbox library with three present media files (`a.mp3`, `b.flac`,
/// `c.wav`) behind the fixture `master.db`.
struct RekordboxLibrary {
    root: TempDir,
    _media: TempDir,
    master: PathBuf,
}

impl RekordboxLibrary {
    fn new() -> Self {
        let root = tempdir().expect("rb root");
        let media_dir = tempdir().expect("media root");
        let media = media_files(media_dir.path(), &["a.mp3", "b.flac", "c.wav"]);
        let master = create_rekordbox_db_with_playlists(root.path(), &media);
        Self {
            root,
            _media: media_dir,
            master,
        }
    }

    fn path(&self) -> Option<String> {
        Some(self.master.to_string_lossy().to_string())
    }

    /// Like a Windows rekordbox library: Charlie's analysis at a
    /// `/PIONEER/...` virtual path resolving under `<share>`, with both `.DAT`
    /// (beat grid, first beat 437 ms) and `.EXT`. Returns the folder and the
    /// two files' bytes.
    fn add_charlie_analysis(&self) -> (PathBuf, Vec<u8>, Vec<u8>) {
        let anlz_dir = self.root.path().join("share/PIONEER/USBANLZ/abc/def");
        fs::create_dir_all(&anlz_dir).expect("anlz dir");
        let waveform = WaveformData::empty();
        let dat = build_anlz_dat_file(&waveform, "", Some(128.0), 200_000, Some(437), &[]);
        let ext = build_anlz_ext_file(&waveform, "", Some(128.0), 200_000, Some(437), &[]);
        fs::write(anlz_dir.join("ANLZ0000.DAT"), &dat).expect("write dat");
        fs::write(anlz_dir.join("ANLZ0000.EXT"), &ext).expect("write ext");
        open_rekordbox_db(&self.master)
            .execute(
                "UPDATE djmdContent SET AnalysisDataPath = '/PIONEER/USBANLZ/abc/def/ANLZ0000.DAT' WHERE ID = '103'",
                [],
            )
            .expect("set analysis path");
        (anlz_dir, dat, ext)
    }
}

fn import_rekordbox(
    backend: &BackendCommands,
    path: &Option<String>,
    kind: ExternalPlaylistKind,
    id: &str,
    force: bool,
) -> Result<ImportExternalPlaylistData, String> {
    import_one(backend, ExternalLibrary::Rekordbox, path, kind, id, force)
}

#[test]
fn list_and_import_rekordbox_playlists_and_history() {
    let (_app_dir, data_dir, backend) = app();
    let library = RekordboxLibrary::new();
    let path = library.path();

    let listed = backend
        .list_rekordbox_playlists(ListExternalPlaylistsRequest { path: path.clone() })
        .data
        .expect("list")
        .items;
    let summary: Vec<_> = listed
        .iter()
        .map(|p| (p.id.as_str(), p.name.as_str(), p.kind, p.track_count))
        .collect();
    assert_eq!(listed[0].note_text, "5 tracks");
    // Folder path in the name, tree order, no folder / smart / deleted /
    // empty lists; deleted entries and deleted tracks not counted.
    assert_eq!(
        summary,
        vec![
            ("11", "Sets / Friday", ExternalPlaylistKind::Playlist, 5),
            ("13", "Openers", ExternalPlaylistKind::Playlist, 1),
            ("15", "All missing", ExternalPlaylistKind::Playlist, 1),
            ("h2", "HISTORY 2025-01-02", ExternalPlaylistKind::History, 2),
            ("h1", "HISTORY 2025-01-01", ExternalPlaylistKind::History, 1),
        ]
    );

    let imported = import_rekordbox(&backend, &path, ExternalPlaylistKind::Playlist, "11", false)
        .expect("import playlist");
    assert_eq!(imported.name, "Friday");
    assert_eq!(imported.added, 3);
    assert_eq!(imported.not_found, vec!["/nowhere/missing.mp3".to_string()]);
    let codes: Vec<_> = imported.warnings.iter().map(|w| w.code.as_str()).collect();
    for code in [
        "scan.rekordbox-db.playlist-duplicates-skipped",
        "scan.rekordbox-db.playlist-tracks-deleted",
        "scan.rekordbox-db.file-not-found",
    ] {
        assert!(codes.contains(&code), "expected {code}, got {codes:?}");
    }
    assert_eq!(
        playlist_titles(&data_dir, &imported.playlist_id),
        vec!["Charlie", "Alpha", "Bravo"]
    );
    let db = library_db(&data_dir);
    let charlie_cues: Vec<(i64, Option<i64>, Option<String>, bool)> = db
        .prepare(
            "SELECT c.position_ms, c.color_id, c.name, c.is_playback_start
             FROM track_cues c JOIN tracks t ON t.id = c.track_id
             WHERE t.title = 'Charlie' ORDER BY c.sort_order",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let charlie: (Option<f64>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT bpm, bpm_analyzer, genre FROM tracks WHERE title = 'Charlie'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        charlie,
        (
            Some(128.0),
            Some("rekordbox".to_string()),
            Some("Drum & Bass".to_string())
        )
    );
    assert_eq!(
        charlie_cues,
        vec![
            (53, None, None, true),
            (54_911, Some(3), Some("Drop".to_string()), false),
            (
                60_000,
                Some(i64::from(DEFAULT_HOTCUE_COLOR_ID)),
                None,
                false
            ),
        ]
    );

    let history = import_rekordbox(&backend, &path, ExternalPlaylistKind::History, "h2", false)
        .expect("import history");
    assert_eq!(history.indexed, 0, "tracks reused from the first import");
    assert_eq!(
        playlist_titles(&data_dir, &history.playlist_id),
        vec!["Charlie", "Bravo"]
    );

    // Nothing importable: an error, and no empty playlist left behind.
    let err =
        import_rekordbox(&backend, &path, ExternalPlaylistKind::Playlist, "15", false).unwrap_err();
    assert!(err.contains("the only track in"), "{err}");
    assert_eq!(playlist_count(&data_dir), 2);

    for (kind, id) in [
        (ExternalPlaylistKind::Playlist, "10"), // a folder
        (ExternalPlaylistKind::Playlist, "12"), // a smart playlist
        (ExternalPlaylistKind::Playlist, "14"), // deleted
        (ExternalPlaylistKind::Crate, "11"),
    ] {
        assert!(
            import_rekordbox(&backend, &path, kind, id, false).is_err(),
            "{kind:?} {id} should not import"
        );
    }
}

/// More memory cues than hot-cue pads, named and coloured: every one is
/// imported as a memory cue (none fills a hot-cue slot, none is skipped),
/// except one on a hot cue's position.
#[test]
fn import_keeps_every_rekordbox_memory_cue() {
    let (_app_dir, data_dir, backend) = app();
    let library = RekordboxLibrary::new();
    let master = open_rekordbox_db(&library.master);
    for i in 0..10i64 {
        let (color, comment) = match i % 3 {
            0 => (None, None),
            1 => (Some(4), Some(format!("Part {i}"))),
            _ => (Some(2), None),
        };
        master
            .execute(
                "INSERT INTO djmdCue VALUES (?1, '103', ?2, -1, 0, -1, ?3, ?4, 0)",
                params![format!("m{i}"), 70_000 + i * 10_000, color, comment],
            )
            .expect("insert memory cue");
    }
    master
        .execute(
            "INSERT INTO djmdCue VALUES ('on-hot', '103', 54911, -1, 0, -1, 1, 'Dup', 0)",
            [],
        )
        .expect("insert memory cue on the hot cue");
    drop(master);

    let imported = import_rekordbox(
        &backend,
        &library.path(),
        ExternalPlaylistKind::Playlist,
        "11",
        false,
    )
    .expect("import playlist");
    assert!(
        imported
            .warnings
            .iter()
            .all(|w| !w.code.contains("memory-cues-skipped"))
    );

    // (position, color_id, name, is_playback_start, is_memory)
    type ImportedCue = (i64, Option<i64>, Option<String>, bool, bool);
    let cues: Vec<ImportedCue> = library_db(&data_dir)
        .prepare(
            "SELECT c.position_ms, c.color_id, c.name, c.is_playback_start, c.is_memory
             FROM track_cues c JOIN tracks t ON t.id = c.track_id
             WHERE t.title = 'Charlie' ORDER BY c.sort_order",
        )
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    // The start cue, the two hot cues, then the ten memory cues.
    assert_eq!(cues.len(), 13);
    assert_eq!(cues[0], (53, None, None, true, false));
    assert_eq!(
        cues[1],
        (54_911, Some(3), Some("Drop".to_string()), false, false)
    );
    let memory: Vec<_> = cues.iter().filter(|c| c.4).collect();
    assert_eq!(memory.len(), 10);
    assert_eq!(*memory[0], (70_000, None, None, false, true));
    assert_eq!(
        *memory[1],
        (80_000, Some(4), Some("Part 1".to_string()), false, true)
    );
    assert_eq!(*memory[2], (90_000, Some(2), None, false, true));
}

#[test]
fn reimporting_a_rekordbox_playlist_updates_it_and_force_takes_rekordbox_values() {
    let (_app_dir, data_dir, backend) = app();
    let library = RekordboxLibrary::new();
    let path = library.path();
    let import = |force| {
        import_rekordbox(&backend, &path, ExternalPlaylistKind::Playlist, "11", force)
            .expect("import")
    };
    let charlie = || -> (f64, Option<String>, Vec<i64>) {
        let db = library_db(&data_dir);
        let (bpm, source) = db
            .query_row(
                "SELECT bpm, bpm_analyzer FROM tracks WHERE title = 'Charlie'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("charlie");
        let cues = db
            .prepare(
                "SELECT c.position_ms FROM track_cues c JOIN tracks t ON t.id = c.track_id
                 WHERE t.title = 'Charlie' ORDER BY c.sort_order",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        (bpm, source, cues)
    };

    let first = import(false);
    assert_eq!(
        first_local_playlist_imported_from(&backend).as_deref(),
        Some("rekordbox")
    );
    library_db(&data_dir)
        .execute_batch(
            "UPDATE tracks SET bpm = 99.0, bpm_analyzer = 'user' WHERE title = 'Charlie';
             DELETE FROM track_cues;
             INSERT INTO track_cues (id, track_id, position_ms, sort_order, created_at, updated_at)
               SELECT 'local', id, 7777, 0, 'x', 'x' FROM tracks WHERE title = 'Charlie';",
        )
        .expect("local edits");

    let again = import(false);
    assert!(again.updated_existing);
    assert_eq!(again.playlist_id, first.playlist_id);
    assert_eq!(charlie(), (99.0, Some("user".to_string()), vec![7777]));

    let forced = import(true);
    assert_eq!(forced.playlist_id, first.playlist_id);
    assert_eq!(
        charlie(),
        (
            128.0,
            Some("rekordbox".to_string()),
            vec![53, 54_911, 60_000]
        )
    );
}

#[test]
fn rekordbox_import_takes_first_beat_from_rekordbox_beat_grid() {
    let (_app_dir, data_dir, backend) = app();
    let library = RekordboxLibrary::new();
    library.add_charlie_analysis();
    let path = library.path();
    let import = |force| {
        import_rekordbox(&backend, &path, ExternalPlaylistKind::Playlist, "11", force)
            .expect("import")
    };
    let charlie = || -> (Option<i64>, Option<String>) {
        library_db(&data_dir)
            .query_row(
                "SELECT first_beat_ms, first_beat_ms_source FROM tracks WHERE title = 'Charlie'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("charlie")
    };

    import(false);
    assert_eq!(charlie(), (Some(437), Some("rekordbox".to_string())));
    let detail = backend
        .get_track_detail(GetTrackDetailRequest {
            track_id: track_id_by_title(&data_dir, "Charlie"),
        })
        .data
        .expect("track detail");
    assert_eq!(detail.first_beat_ms, Some(437));

    // A grid moved in the cue editor survives a plain re-import...
    library_db(&data_dir)
        .execute(
            "UPDATE tracks SET first_beat_ms = 900, first_beat_ms_source = 'user' WHERE title = 'Charlie'",
            [],
        )
        .unwrap();
    import(false);
    assert_eq!(charlie(), (Some(900), Some("user".to_string())));
    // ...and a forced one takes rekordbox's again.
    import(true);
    assert_eq!(charlie(), (Some(437), Some("rekordbox".to_string())));
}

#[test]
fn saving_edits_never_writes_into_rekordbox_anlz_files() {
    let (_app_dir, data_dir, backend) = app();
    let library = RekordboxLibrary::new();
    let (anlz_dir, dat, ext) = library.add_charlie_analysis();
    import_rekordbox(
        &backend,
        &library.path(),
        ExternalPlaylistKind::Playlist,
        "11",
        false,
    )
    .expect("import");

    let saved = backend.save_track_analysis_edits(SaveTrackAnalysisEditsRequest {
        track_id: track_id_by_title(&data_dir, "Charlie"),
        first_beat_ms: Some(900),
        bpm: None,
        key: None,
        cues: Some(vec![TrackCueInput {
            position_ms: 1_500,
            color_id: None,
            name: Some("Intro".to_string()),
            playback_start: false,
            memory: false,
        }]),
    });
    assert!(saved.ok, "save edits: {saved:?}");
    assert!(!saved.data.expect("save data").anlz_regenerated);
    assert_eq!(fs::read(anlz_dir.join("ANLZ0000.DAT")).unwrap(), dat);
    assert_eq!(fs::read(anlz_dir.join("ANLZ0000.EXT")).unwrap(), ext);
}

#[test]
fn import_external_playlists_totals_imports_and_keeps_going_after_a_failure() {
    let (_app_dir, _data_dir, backend) = app();
    let library = RekordboxLibrary::new();
    let path = library.path();
    let item = |id: &str| {
        import_item(
            ExternalLibrary::Rekordbox,
            &path,
            ExternalPlaylistKind::Playlist,
            id,
            false,
        )
    };

    let data = import_lists(&backend, vec![item("15"), item("11"), item("13")]);

    // "All missing" has no importable track; the other two still import.
    assert_eq!((data.imported, data.failed), (2, 1));
    assert!(data.results[0].playlist.is_none());
    assert!(
        data.results[0]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("could not be imported")),
        "{:?}",
        data.results[0].error
    );
    let friday = data.results[1].playlist.as_ref().expect("Friday imported");
    let openers = data.results[2].playlist.as_ref().expect("Openers imported");
    assert_eq!(friday.name, "Friday");
    // Friday lists a missing file, a deleted track and a repeat.
    assert!(friday.skipped > 0, "{}", friday.skipped);
    assert_eq!(data.added, friday.added + openers.added);
    assert_eq!(data.skipped, friday.skipped + openers.skipped);
    assert!(
        data.status_text.starts_with(&format!(
            "Imported 2 of 3 playlists: {} tracks | ",
            data.added
        )) && data.status_text.contains(" skipped (see event log)")
            && data
                .status_text
                .ends_with(" | 1 failed: 15 (see event log)"),
        "{}",
        data.status_text
    );

    // Nothing imported: the status line is the first failure, with its
    // library when there was only one list.
    let one = import_lists(&backend, vec![item("15")]);
    assert_eq!((one.imported, one.failed), (0, 1));
    let reason = one.results[0].error.clone().expect("its reason");
    assert_eq!(
        one.status_text,
        format!("rekordbox playlist import failed: {reason}")
    );
    let both = import_lists(&backend, vec![item("15"), item("15")]);
    assert_eq!(
        both.status_text,
        format!("Playlist import failed: {reason}")
    );
}

// --- rekordbox library scan --------------------------------------------

/// A `master.db` with integer ids at `<root>/rekordbox/master.db`: one row
/// per `(file_path, title, anlz_path, image_path)` plus one deleted row.
fn create_rekordbox_scan_fixture(
    root: &Path,
    rows: &[(&str, &str, Option<&str>, Option<&str>)],
) -> PathBuf {
    let master_path = root.join("rekordbox").join("master.db");
    fs::create_dir_all(master_path.parent().unwrap()).expect("create master db dir");
    let conn = open_rekordbox_db(&master_path);
    conn.execute_batch(
        r#"
        CREATE TABLE djmdArtist (ID INTEGER PRIMARY KEY, Name TEXT);
        CREATE TABLE djmdAlbum (ID INTEGER PRIMARY KEY, Name TEXT);
        CREATE TABLE djmdKey (ID INTEGER PRIMARY KEY, ScaleName TEXT);
        CREATE TABLE djmdContent (
          ID INTEGER PRIMARY KEY, FolderPath TEXT, Title TEXT, SrcArtistName TEXT,
          ArtistID INTEGER, AlbumID INTEGER, BPM INTEGER, KeyID INTEGER, Length INTEGER,
          AnalysisDataPath TEXT, ImagePath TEXT, rb_local_deleted INTEGER DEFAULT 0
        );
        INSERT INTO djmdArtist (ID, Name) VALUES (1, 'Fixture Artist');
        INSERT INTO djmdAlbum (ID, Name) VALUES (1, 'Fixture Album');
        INSERT INTO djmdKey (ID, ScaleName) VALUES (1, '8A');
        "#,
    )
    .expect("create master db schema");
    for (idx, (file_path, title, anlz_path, image_path)) in rows.iter().enumerate() {
        conn.execute(
            "INSERT INTO djmdContent
               (ID, FolderPath, Title, SrcArtistName, ArtistID, AlbumID, BPM, KeyID, Length,
                AnalysisDataPath, ImagePath, rb_local_deleted)
             VALUES (?1, ?2, ?3, 'Fallback Artist', 1, 1, 12600, 1, 245, ?4, ?5, 0)",
            params![idx as i64 + 1, file_path, title, anlz_path, image_path],
        )
        .expect("insert master db row");
    }
    conn.execute(
        "INSERT INTO djmdContent (ID, FolderPath, Title, rb_local_deleted)
         VALUES (999, '/deleted/track.mp3', 'Deleted', 1)",
        [],
    )
    .expect("insert deleted row");
    master_path
}

fn scan_rekordbox(
    backend: &BackendCommands,
    master_path: &Path,
) -> backend::models::ScanLibraryData {
    let scanned = backend.scan_rekordbox_db(ScanRekordboxDbRequest {
        path: Some(master_path.to_string_lossy().to_string()),
    });
    assert!(scanned.ok, "scan rekordbox db: {scanned:?}");
    scanned.data.expect("scan data")
}

#[test]
fn scan_rekordbox_db_maps_wine_drive_paths_to_linux_paths() {
    let tmp = tempdir().expect("tempdir");
    let prefix = tmp.path().join("prefix");
    fs::create_dir_all(prefix.join("dosdevices")).expect("dosdevices");
    let music = prefix.join("drive_c/users/dj/Music");
    fs::create_dir_all(&music).expect("music dir");
    std::os::unix::fs::symlink("../drive_c", prefix.join("dosdevices/c:")).expect("c:");
    fs::write(music.join("a.mp3"), b"audio").expect("write track");
    let (_app_dir, data_dir, backend) = app();

    let master_path = create_rekordbox_scan_fixture(
        &prefix.join("drive_c/users/dj/AppData/Roaming/Pioneer"),
        &[("C:/users/dj/Music/a.mp3", "Under Wine", None, None)],
    );
    let result = scan_rekordbox(&backend, &master_path);
    assert_eq!(result.indexed, 1);
    assert!(result.not_found.is_empty(), "{:?}", result.not_found);

    let file_path: String = library_db(&data_dir)
        .query_row(
            "SELECT file_path FROM tracks WHERE title = 'Under Wine'",
            [],
            |row| row.get(0),
        )
        .expect("imported track");
    let expected = music.canonicalize().expect("music").join("a.mp3");
    assert_eq!(PathBuf::from(file_path), expected);
}

#[test]
fn scan_rekordbox_db_imports_updates_keeps_missing_and_reports_resource_warnings() {
    let master_root = tempdir().expect("master root");
    let media_root = tempdir().expect("media root");
    let (_app_dir, data_dir, backend) = app();

    let existing_path = media_root.path().join("existing.mp3");
    let new_nulls_path = media_root.path().join("new-nulls.aif");
    let new_missing_assets_path = media_root.path().join("new-missing.flac");
    fs::write(&existing_path, b"existing").expect("write existing file");
    fs::write(&new_nulls_path, b"new").expect("write new file");
    fs::write(&new_missing_assets_path, b"missing").expect("write missing-assets file");

    let share_dir = master_root.path().join("rekordbox").join("share");
    let anlz_path = share_dir.join("PIONEER/USBANLZ/a/ANLZ0000.EXT");
    let image_path = share_dir.join("PIONEER/Artwork/a/COVER.JPG");
    fs::create_dir_all(anlz_path.parent().unwrap()).expect("create anlz dir");
    fs::create_dir_all(image_path.parent().unwrap()).expect("create image dir");
    fs::write(&anlz_path, b"anlz").expect("write anlz");
    fs::write(&image_path, b"jpg").expect("write artwork");

    let removed_path = media_root.path().join("removed.mp3");
    let master_path = create_rekordbox_scan_fixture(
        master_root.path(),
        &[
            (
                existing_path.to_str().unwrap(),
                "Updated Existing",
                Some("/PIONEER/USBANLZ/a/ANLZ0000.DAT"),
                Some("/PIONEER/Artwork/a/COVER.JPG"),
            ),
            (
                new_nulls_path.to_str().unwrap(),
                "Inserted Nulls",
                None,
                None,
            ),
            (
                new_missing_assets_path.to_str().unwrap(),
                "Inserted Missing Assets",
                Some("/PIONEER/USBANLZ/missing/ANLZ0000.DAT"),
                Some("/PIONEER/Artwork/missing/COVER.JPG"),
            ),
            (removed_path.to_str().unwrap(), "Removed", None, None),
        ],
    );

    let db = library_db(&data_dir);
    for (id, title, path) in [
        ("existing-track", "Old", &existing_path),
        ("removed-track", "Gone", &removed_path),
    ] {
        db.execute(
            "INSERT INTO tracks (id, title, artist, file_path, match_fingerprint, created_at, updated_at)
             VALUES (?1, ?2, 'Old Artist', ?3, 'old-fp', 'old', 'old')",
            params![id, title, path.to_str().unwrap()],
        )
        .expect("seed track");
    }
    drop(db);

    let imported = || {
        backend
            .detect_external_rekordbox_db()
            .data
            .expect("detect")
            .imported
    };
    assert!(!imported(), "nothing imported yet");
    let result = scan_rekordbox(&backend, &master_path);
    assert!(imported());
    assert_eq!(
        result.status_text,
        "Desktop library import done: 2 new, 1 updated | 1 file not found (see event log)"
    );
    assert_eq!(result.indexed, 2);
    assert_eq!(result.updated, 1);
    // A missing file is reported, never deleted (its drive may just be
    // unplugged).
    assert_eq!(result.removed, 0);
    assert_eq!(
        result.not_found,
        vec![removed_path.to_string_lossy().to_string()]
    );
    let warning_codes: Vec<_> = result.warnings.iter().map(|w| w.code.as_str()).collect();
    // The resolved/sample counts are file only; the user gets what's missing.
    for code in [
        "scan.rekordbox-db.anlz-miss-summary",
        "scan.rekordbox-db.artwork-miss-summary",
        "scan.rekordbox-db.file-not-found",
    ] {
        assert!(
            warning_codes.contains(&code),
            "expected warning code {code}, got {warning_codes:?}"
        );
    }

    let db = library_db(&data_dir);
    let updated: (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = db
        .query_row(
            "SELECT title, artist, tonality, waveform_peaks_path, artwork_path
               FROM tracks WHERE id = 'existing-track'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("updated row");
    assert_eq!(updated.0, "Updated Existing");
    assert_eq!(updated.1, "Fixture Artist");
    assert_eq!(updated.2.as_deref(), Some("8A"));
    assert_eq!(updated.3.as_deref(), Some(anlz_path.to_str().unwrap()));
    assert!(
        updated
            .4
            .as_deref()
            .is_some_and(|path| path.ends_with(".JPG"))
    );

    let count = |sql: &str| -> i64 { db.query_row(sql, [], |row| row.get(0)).expect(sql) };
    assert_eq!(
        count("SELECT COUNT(*) FROM tracks WHERE id = 'removed-track'"),
        1
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM tracks WHERE rekordbox_db_source = 1"),
        3
    );
}

// --- Mixxx -----------------------------------------------------------------

// Values of Mixxx's own schema.
const MIXXX_CUE_HOT: i64 = 1;
const MIXXX_CUE_MAIN: i64 = 2;
const MIXXX_COVER_TYPE_FILE: i64 = 2;
const MIXXX_PLAYLIST_REGULAR: i64 = 0;
const MIXXX_PLAYLIST_AUTO_DJ: i64 = 1;
const MIXXX_PLAYLIST_SET_LOG: i64 = 2;

/// `(file_path, title, key_id, samplerate, cover_type, cover_location, mixxx_deleted)`
type FixtureTrack<'a> = (
    &'a str,
    &'a str,
    Option<i64>,
    i64,
    i64,
    Option<&'a str>,
    i64,
);

/// `(track_id, type, position, hotcue, label, color)`
type FixtureCue<'a> = (i64, i64, f64, i64, Option<&'a str>, i64);

/// Shorthand for a present, not-deleted track without key or cover.
fn plain_track(path: &Path, title: &'static str) -> (String, &'static str) {
    (path.to_str().unwrap().to_string(), title)
}

fn create_mixxx_db_fixture(
    root: &Path,
    tracks: &[FixtureTrack<'_>],
    cues: &[FixtureCue<'_>],
) -> PathBuf {
    let path = root.join("mixxxdb.sqlite");
    let conn = Connection::open(&path).expect("create mixxx db");
    conn.execute_batch(
        r#"
        CREATE TABLE track_locations (
          id INTEGER PRIMARY KEY, location TEXT, filename TEXT, directory TEXT,
          filesize INTEGER, fs_deleted INTEGER, needs_verification INTEGER
        );
        CREATE TABLE library (
          id INTEGER PRIMARY KEY, artist TEXT, title TEXT, album TEXT, location INTEGER,
          duration REAL, bpm REAL, key TEXT, key_id INTEGER, samplerate INTEGER,
          mixxx_deleted INTEGER, coverart_type INTEGER, coverart_location TEXT, genre TEXT
        );
        CREATE TABLE cues (
          id INTEGER PRIMARY KEY, track_id INTEGER, type INTEGER, position REAL,
          length REAL, hotcue INTEGER, label TEXT, color INTEGER
        );
        "#,
    )
    .expect("create mixxx schema");
    for (i, (file_path, title, key_id, samplerate, cover_type, cover_location, deleted)) in
        tracks.iter().enumerate()
    {
        let id = i as i64 + 1;
        conn.execute(
            "INSERT INTO track_locations (id, location, fs_deleted) VALUES (?1, ?2, 0)",
            params![id, file_path],
        )
        .expect("insert location");
        conn.execute(
            "INSERT INTO library (id, artist, title, album, location, duration, bpm, key,
               key_id, samplerate, mixxx_deleted, coverart_type, coverart_location, genre)
             VALUES (?1, 'Artist', ?2, 'Album', ?1, 185.5, 124.0, 'Am', ?3, ?4, ?5, ?6, ?7, 'Techno')",
            params![
                id,
                title,
                key_id,
                samplerate,
                deleted,
                cover_type,
                cover_location
            ],
        )
        .expect("insert library row");
    }
    for (track_id, cue_type, position, hotcue, label, color) in cues {
        conn.execute(
            "INSERT INTO cues (track_id, type, position, length, hotcue, label, color)
             VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6)",
            params![track_id, cue_type, position, hotcue, label, color],
        )
        .expect("insert cue");
    }
    path
}

/// Adds Mixxx's playlist + crate tables to a fixture library.
fn add_mixxx_playlists(
    path: &Path,
    playlists: &[(i64, &str, i64, i64, &[i64])],
    crates: &[(i64, &str, &[i64])],
) {
    let conn = Connection::open(path).expect("open mixxx fixture");
    conn.execute_batch(
        r#"
        CREATE TABLE Playlists (
          id INTEGER PRIMARY KEY, name TEXT, position INTEGER, hidden INTEGER,
          date_created TEXT, date_modified TEXT, locked INTEGER
        );
        CREATE TABLE PlaylistTracks (
          id INTEGER PRIMARY KEY, playlist_id INTEGER, track_id INTEGER, position INTEGER
        );
        CREATE TABLE crates (id INTEGER PRIMARY KEY, name TEXT, count INTEGER, show INTEGER);
        CREATE TABLE crate_tracks (crate_id INTEGER, track_id INTEGER);
        "#,
    )
    .expect("create playlist schema");
    for (id, name, position, hidden, tracks) in playlists {
        conn.execute(
            "INSERT INTO Playlists (id, name, position, hidden, locked) VALUES (?1, ?2, ?3, ?4, 0)",
            params![id, name, position, hidden],
        )
        .expect("insert playlist");
        // Stored out of order: the import must sort by `position`.
        for (index, track_id) in tracks.iter().enumerate().rev() {
            conn.execute(
                "INSERT INTO PlaylistTracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                params![id, track_id, index as i64 + 1],
            )
            .expect("insert playlist track");
        }
    }
    for (id, name, tracks) in crates {
        conn.execute(
            "INSERT INTO crates (id, name, show) VALUES (?1, ?2, 1)",
            params![id, name],
        )
        .expect("insert crate");
        for track_id in *tracks {
            conn.execute(
                "INSERT INTO crate_tracks (crate_id, track_id) VALUES (?1, ?2)",
                params![id, track_id],
            )
            .expect("insert crate track");
        }
    }
}

fn import_mixxx(
    backend: &BackendCommands,
    path: &Option<String>,
    kind: ExternalPlaylistKind,
    id: &str,
    force: bool,
) -> Result<ImportExternalPlaylistData, String> {
    import_one(backend, ExternalLibrary::Mixxx, path, kind, id, force)
}

#[test]
fn scan_mixxx_db_imports_tracks_cues_and_cover_and_keeps_local_cues() {
    let mixxx_root = tempdir().expect("mixxx root");
    let media_root = tempdir().expect("media root");
    let (_app_dir, data_dir, backend) = app();

    let new_path = media_root.path().join("new.mp3");
    let existing_path = media_root.path().join("existing.flac");
    let deleted_path = media_root.path().join("deleted-in-mixxx.mp3");
    let missing_path = media_root.path().join("missing.mp3");
    let module_path = media_root.path().join("tune.it");
    for p in [&new_path, &existing_path, &deleted_path, &module_path] {
        fs::write(p, b"audio").expect("write media");
    }
    fs::write(media_root.path().join("cover.jpg"), b"jpg").expect("write cover");

    let mixxx_path = create_mixxx_db_fixture(
        mixxx_root.path(),
        &[
            (
                new_path.to_str().unwrap(),
                "New",
                Some(22),
                48_000,
                MIXXX_COVER_TYPE_FILE,
                Some("cover.jpg"),
                0,
            ),
            (
                existing_path.to_str().unwrap(),
                "Existing",
                Some(1),
                44_100,
                1,
                None,
                0,
            ),
            (
                deleted_path.to_str().unwrap(),
                "Deleted",
                None,
                44_100,
                0,
                None,
                1,
            ),
            (
                missing_path.to_str().unwrap(),
                "Missing",
                None,
                44_100,
                0,
                None,
                0,
            ),
            (
                module_path.to_str().unwrap(),
                "Module",
                None,
                44_100,
                0,
                None,
                0,
            ),
        ],
        &[
            (
                1,
                MIXXX_CUE_HOT,
                96_000.0 * 4.0,
                0,
                Some("Intro"),
                0x4E_B6_48,
            ),
            (1, MIXXX_CUE_MAIN, 96_000.0, -1, None, 0),
            (2, MIXXX_CUE_HOT, 88_200.0, 0, None, 0xE1_24_24),
        ],
    );

    let db = library_db(&data_dir);
    db.execute(
        "INSERT INTO tracks (id, title, artist, file_path, bpm, match_fingerprint, created_at, updated_at)
         VALUES ('existing-track', 'Old', 'Old Artist', ?1, 128.0, 'old-fp', 'old', 'old')",
        params![existing_path.to_str().unwrap()],
    )
    .expect("seed existing track");
    db.execute(
        "INSERT INTO track_cues (id, track_id, position_ms, color_id, name, sort_order, created_at, updated_at)
         VALUES ('local-cue', 'existing-track', 7777, 1, 'mine', 0, 'old', 'old')",
        [],
    )
    .expect("seed local cue");
    // Imported earlier; its file is gone now (drive unplugged). It has a cue
    // and a playlist place, which the import must not take away.
    db.execute_batch(&format!(
        "INSERT INTO tracks (id, title, artist, file_path, match_fingerprint, created_at, updated_at)
           VALUES ('missing-track', 'Missing', 'Artist', '{}', 'fp', 'old', 'old');
         INSERT INTO track_cues (id, track_id, position_ms, sort_order, created_at, updated_at)
           VALUES ('missing-cue', 'missing-track', 1000, 0, 'old', 'old');
         INSERT INTO playlists (id, name, source, created_at, updated_at)
           VALUES ('pl-1', 'Set', 'local', 'old', 'old');
         INSERT INTO playlist_tracks (id, playlist_id, track_id, position, added_at)
           VALUES ('pt-1', 'pl-1', 'missing-track', 1, 'old');",
        missing_path.to_str().unwrap()
    ))
    .expect("seed missing track");
    drop(db);

    let imported = || {
        backend
            .detect_external_mixxx_db()
            .data
            .expect("detect")
            .imported
    };
    assert!(!imported(), "nothing imported yet");
    let scanned = backend.scan_mixxx_db(ScanMixxxDbRequest {
        path: Some(mixxx_path.to_string_lossy().to_string()),
    });
    assert!(scanned.ok, "scan mixxx db: {scanned:?}");
    let result = scanned.data.expect("scan data");
    assert!(imported());
    assert_eq!(result.indexed, 1);
    assert_eq!(result.updated, 1);
    assert_eq!(
        result.status_text,
        "Mixxx library import done: 1 new, 1 updated | 1 file not found (see event log)"
    );
    assert_eq!(result.removed, 0);
    assert_eq!(
        result.not_found,
        vec![missing_path.to_string_lossy().to_string()]
    );
    // The missing file is logged, so the status line's "(see event log)" finds it.
    let missing = missing_path.to_string_lossy();
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.code == "scan.mixxx-db.file-not-found"
                && w.message == format!("track file not found, skipped: {missing}")),
        "{:?}",
        result.warnings
    );
    let codes: Vec<_> = result.warnings.iter().map(|w| w.code.as_str()).collect();
    for code in [
        "scan.mixxx-db.cues-imported",
        "scan.mixxx-db.cues-kept-local",
        "scan.mixxx-db.unsupported-skipped",
    ] {
        assert!(codes.contains(&code), "expected {code}, got {codes:?}");
    }

    let db = library_db(&data_dir);
    let (new_id, title, key, bpm): (String, String, Option<String>, Option<f64>) = db
        .query_row(
            "SELECT id, title, tonality, bpm FROM tracks WHERE file_path = ?1",
            params![new_path.to_str().unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("new track row");
    let (duration_ms, sample_rate, artwork, flags): (
        Option<i64>,
        Option<i64>,
        Option<String>,
        (i64, i64),
    ) = db
        .query_row(
            "SELECT duration_ms, sample_rate_hz, artwork_path, mixxx_db_source, rekordbox_db_source
             FROM tracks WHERE id = ?1",
            params![new_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, (r.get(3)?, r.get(4)?))),
        )
        .expect("new track details");
    assert_eq!(title, "New");
    assert_eq!(key.as_deref(), Some("Am"));
    assert_eq!(bpm, Some(124.0));
    assert_eq!(duration_ms, Some(185_500));
    assert_eq!(sample_rate, Some(48_000));
    assert_eq!(flags, (1, 0));
    // Imported BPM / key are marked as Mixxx's, so analysis keeps them.
    let sources: (Option<String>, Option<String>) = db
        .query_row(
            "SELECT bpm_analyzer, tonality_source FROM tracks WHERE id = ?1",
            params![new_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("new track sources");
    assert_eq!(
        sources,
        (Some("mixxx".to_string()), Some("mixxx".to_string()))
    );
    let artwork = artwork.expect("cover copied");
    assert!(Path::new(&artwork).is_file());
    assert!(artwork.ends_with(&format!("{new_id}.jpg")));

    let new_cues: Vec<(i64, Option<i64>, Option<String>, bool)> = db
        .prepare(
            "SELECT position_ms, color_id, name, is_playback_start FROM track_cues
             WHERE track_id = ?1 ORDER BY sort_order",
        )
        .unwrap()
        .query_map(params![new_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        new_cues,
        vec![
            (1_000, None, None, true),
            (4_000, Some(5), Some("Intro".to_string()), false),
        ]
    );

    // Existing track: metadata refreshed, local analysis + cues kept.
    let (bpm, key, mixxx_flag): (Option<f64>, Option<String>, i64) = db
        .query_row(
            "SELECT bpm, tonality, mixxx_db_source FROM tracks WHERE id = 'existing-track'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("existing row");
    assert_eq!(bpm, Some(128.0));
    assert_eq!(key.as_deref(), Some("C"));
    assert_eq!(mixxx_flag, 1);
    // Only the value this import filled in (the key) is marked Mixxx's;
    // the BPM the track already had keeps its (empty) source.
    let sources: (Option<String>, Option<String>) = db
        .query_row(
            "SELECT bpm_analyzer, tonality_source FROM tracks WHERE id = 'existing-track'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("existing track sources");
    assert_eq!(sources, (None, Some("mixxx".to_string())));
    let existing_cues: Vec<String> = db
        .prepare("SELECT id FROM track_cues WHERE track_id = 'existing-track'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(existing_cues, vec!["local-cue".to_string()]);

    // The missing file's track is kept, with its cue and playlist place.
    let kept: (i64, i64, i64) = db
        .query_row(
            "SELECT (SELECT COUNT(*) FROM tracks WHERE id = 'missing-track'),
                    (SELECT COUNT(*) FROM track_cues WHERE track_id = 'missing-track'),
                    (SELECT COUNT(*) FROM playlist_tracks WHERE track_id = 'missing-track')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("missing track");
    assert_eq!(kept, (1, 1, 1));

    // Genre comes from Mixxx, for new and existing tracks.
    let genres: Vec<Option<String>> = db
        .prepare("SELECT genre FROM tracks WHERE id IN (?1, 'existing-track') ORDER BY id")
        .unwrap()
        .query_map(params![new_id], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(genres, vec![Some("Techno".to_string()); 2]);

    // Tracks deleted in Mixxx are skipped entirely.
    let deleted: i64 = db
        .query_row(
            "SELECT COUNT(1) FROM tracks WHERE file_path = ?1",
            params![deleted_path.to_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(deleted, 0);
    drop(db);

    // Browse: Mixxx tracks show only when the Mixxx source is included.
    let browse = |include_rekordbox_db: bool, include_mixxx_db: bool| {
        library_sources::show_only(&backend, vec![], include_rekordbox_db, include_mixxx_db);
        backend
            .browse_source_files(BrowseSourceFilesRequest {
                query: String::new(),
                limit: 100,
                ..Default::default()
            })
            .data
            .expect("browse")
            .total
    };
    assert_eq!(browse(false, true), 2);
    assert_eq!(browse(true, false), 0);
    assert_eq!(browse(false, false), 0);
}

#[test]
fn list_and_import_mixxx_playlists_crates_and_set_logs() {
    let mixxx_root = tempdir().expect("mixxx root");
    let media_root = tempdir().expect("media root");
    let (_app_dir, data_dir, backend) = app();

    let paths = media_files(media_root.path(), &["a.mp3", "b.mp3", "c.flac", "tune.it"]);
    let missing = media_root.path().join("missing.mp3");
    let tracks = [
        plain_track(&paths[0], "Alpha"),
        plain_track(&paths[1], "Bravo"),
        plain_track(&paths[2], "Charlie"),
        plain_track(&paths[3], "Module"),
        plain_track(&missing, "Missing"),
    ];
    let mut fixture: Vec<FixtureTrack<'_>> = tracks
        .iter()
        .map(|(path, title)| (path.as_str(), *title, None, 44_100, 0, None, 0))
        .collect();
    fixture.push((tracks[0].0.as_str(), "Deleted", None, 44_100, 0, None, 1));
    let mixxx_path = create_mixxx_db_fixture(
        mixxx_root.path(),
        &fixture,
        &[(2, MIXXX_CUE_HOT, 88_200.0, 0, None, 0)],
    );
    add_mixxx_playlists(
        &mixxx_path,
        &[
            (1, "Auto DJ", 1, MIXXX_PLAYLIST_AUTO_DJ, &[]),
            (2, "placeholder", 2, -1, &[1]),
            // Ids 1-5 = library rows above; 6 = deleted in Mixxx; 99 = gone.
            (
                3,
                "Warmup",
                3,
                MIXXX_PLAYLIST_REGULAR,
                &[3, 1, 2, 1, 4, 5, 6, 99],
            ),
            (4, "2026-09-29", 4, MIXXX_PLAYLIST_SET_LOG, &[2]),
            (5, "2026-09-30", 5, MIXXX_PLAYLIST_SET_LOG, &[1, 2]),
            (6, "Empty", 6, MIXXX_PLAYLIST_REGULAR, &[]),
        ],
        &[(1, "Peak", &[3, 1]), (2, "Gone", &[5])],
    );
    let path = Some(mixxx_path.to_string_lossy().to_string());

    let listed = backend
        .list_mixxx_playlists(ListExternalPlaylistsRequest { path: path.clone() })
        .data
        .expect("list playlists")
        .items;
    let summary: Vec<_> = listed
        .iter()
        .map(|p| (p.name.as_str(), p.kind, p.track_count))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("Warmup", ExternalPlaylistKind::Playlist, 8),
            ("2026-09-30", ExternalPlaylistKind::History, 2),
            ("2026-09-29", ExternalPlaylistKind::History, 1),
            ("Gone", ExternalPlaylistKind::Crate, 1),
            ("Peak", ExternalPlaylistKind::Crate, 2),
        ]
    );

    let imported = import_mixxx(&backend, &path, ExternalPlaylistKind::Playlist, "3", false)
        .expect("import playlist");
    assert_eq!(imported.name, "Warmup");
    assert_eq!(imported.added, 3);
    assert_eq!(imported.indexed, 3);
    assert_eq!(
        imported.not_found,
        vec![missing.to_string_lossy().to_string()]
    );
    let codes: Vec<_> = imported.warnings.iter().map(|w| w.code.as_str()).collect();
    for code in [
        "scan.mixxx-db.unsupported-skipped",
        "scan.mixxx-db.cues-imported",
        "scan.mixxx-db.playlist-duplicates-skipped",
        "scan.mixxx-db.playlist-tracks-deleted",
    ] {
        assert!(codes.contains(&code), "expected {code}, got {codes:?}");
    }
    assert_eq!(
        playlist_titles(&data_dir, &imported.playlist_id),
        vec!["Charlie", "Alpha", "Bravo"]
    );
    let db = library_db(&data_dir);
    let (mixxx_flag, cues): (i64, i64) = db
        .query_row(
            "SELECT t.mixxx_db_source, (SELECT COUNT(1) FROM track_cues c WHERE c.track_id = t.id)
             FROM tracks t WHERE t.title = 'Bravo'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("bravo row");
    assert_eq!((mixxx_flag, cues), (1, 1));
    // Playlist import marks the BPM / key like the library import does,
    // so the first analysis keeps them.
    let sources: (Option<f64>, Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT bpm, bpm_analyzer, tonality, tonality_source FROM tracks WHERE title = 'Bravo'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("bravo sources");
    assert_eq!(
        sources,
        (
            Some(124.0),
            Some("mixxx".to_string()),
            Some("Am".to_string()),
            Some("mixxx".to_string())
        )
    );
    drop(db);

    // Re-importing reuses the library rows; a crate imports artist/title order.
    let crate_import = import_mixxx(&backend, &path, ExternalPlaylistKind::Crate, "1", false)
        .expect("import crate");
    assert_eq!(crate_import.indexed, 0);
    assert_eq!(
        playlist_titles(&data_dir, &crate_import.playlist_id),
        vec!["Alpha", "Charlie"]
    );

    // Only a missing file: an error, and no empty playlist left behind.
    let err = import_mixxx(&backend, &path, ExternalPlaylistKind::Crate, "2", false).unwrap_err();
    assert!(err.contains("the only track in"), "{err}");
    assert_eq!(playlist_count(&data_dir), 2);

    let err =
        import_mixxx(&backend, &path, ExternalPlaylistKind::Playlist, "42", false).unwrap_err();
    assert!(err.contains("not found"), "{err}");
}

#[test]
fn reimporting_a_mixxx_playlist_updates_it_and_refreshes_or_forces_track_data() {
    let mixxx_root = tempdir().expect("mixxx root");
    let media_root = tempdir().expect("media root");
    let (_app_dir, data_dir, backend) = app();
    let paths = media_files(media_root.path(), &["a.mp3", "b.mp3"]);
    let tracks = [
        plain_track(&paths[0], "Alpha"),
        plain_track(&paths[1], "Bravo"),
    ];
    let fixture: Vec<FixtureTrack<'_>> = tracks
        .iter()
        .map(|(path, title)| (path.as_str(), *title, Some(22), 44_100, 0, None, 0))
        .collect();
    let mixxx_path = create_mixxx_db_fixture(mixxx_root.path(), &fixture, &[]);
    // Playlist 3 and crate 3: same id, different lists.
    add_mixxx_playlists(
        &mixxx_path,
        &[(3, "Warmup", 1, MIXXX_PLAYLIST_REGULAR, &[1, 2])],
        &[(3, "Warmup crate", &[2])],
    );
    let path = Some(mixxx_path.to_string_lossy().to_string());
    let import = |kind, force| import_mixxx(&backend, &path, kind, "3", force).expect("import");
    let track = |title: &str| -> (f64, Option<String>, i64) {
        library_db(&data_dir)
            .query_row(
                "SELECT t.bpm, t.bpm_analyzer,
                        (SELECT COUNT(1) FROM track_cues c WHERE c.track_id = t.id)
                 FROM tracks t WHERE t.title = ?1",
                params![title],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("track row")
    };

    let first = import(ExternalPlaylistKind::Playlist, false);
    assert!(!first.updated_existing);
    assert_eq!(
        first_local_playlist_imported_from(&backend).as_deref(),
        Some("Mixxx")
    );
    let listed = backend
        .list_mixxx_playlists(ListExternalPlaylistsRequest { path: path.clone() })
        .data
        .expect("list")
        .items;
    let warmup = listed
        .iter()
        .find(|p| p.kind == ExternalPlaylistKind::Playlist)
        .unwrap();
    assert_eq!(
        warmup.existing_playlist,
        Some(ImportedPlaylistRef {
            id: first.playlist_id.clone(),
            name: "Warmup".to_string()
        })
    );
    let crate_item = listed
        .iter()
        .find(|p| p.kind == ExternalPlaylistKind::Crate)
        .unwrap();
    assert_eq!(
        crate_item.existing_playlist, None,
        "crate 3 is a different list"
    );

    // In the app: the playlist renamed, Alpha's BPM set by hand and given
    // a cue. In Mixxx: new BPMs, the order swapped, and cues on both.
    library_db(&data_dir)
        .execute_batch(&format!(
            "UPDATE playlists SET name = 'My Warmup' WHERE id = '{}';
             UPDATE tracks SET bpm = 130.0, bpm_analyzer = 'user' WHERE title = 'Alpha';
             INSERT INTO track_cues (id, track_id, position_ms, sort_order, created_at, updated_at)
               SELECT 'local', id, 7777, 0, 'x', 'x' FROM tracks WHERE title = 'Alpha';",
            first.playlist_id
        ))
        .expect("local edits");
    Connection::open(&mixxx_path)
        .expect("open mixxx fixture")
        .execute_batch(
            "UPDATE library SET bpm = 126.0 WHERE id = 1;
             UPDATE library SET bpm = 128.0 WHERE id = 2;
             UPDATE PlaylistTracks SET position = 3 - position WHERE playlist_id = 3;
             INSERT INTO cues (track_id, type, position, length, hotcue, color)
               VALUES (1, 1, 88200, 0, 0, 0), (2, 1, 176400, 0, 0, 0);",
        )
        .expect("mixxx edits");

    // Plain re-import: the same playlist, its name kept, the new order;
    // Mixxx-owned values refresh, the user's edits stay.
    let again = import(ExternalPlaylistKind::Playlist, false);
    assert!(again.updated_existing);
    assert_eq!(again.playlist_id, first.playlist_id);
    assert_eq!(again.name, "My Warmup");
    assert_eq!(
        playlist_titles(&data_dir, &again.playlist_id),
        vec!["Bravo", "Alpha"]
    );
    assert_eq!(track("Alpha"), (130.0, Some("user".to_string()), 1));
    assert_eq!(track("Bravo"), (128.0, Some("mixxx".to_string()), 1));

    // The crate with the same id becomes its own playlist.
    let crate_import = import(ExternalPlaylistKind::Crate, false);
    assert!(!crate_import.updated_existing);
    assert_ne!(crate_import.playlist_id, first.playlist_id);

    // Forced: Mixxx wins, BPM and cues included.
    let forced = import(ExternalPlaylistKind::Playlist, true);
    assert!(forced.updated_existing);
    assert_eq!(track("Alpha"), (126.0, Some("mixxx".to_string()), 1));
    let alpha_cue: i64 = library_db(&data_dir)
        .query_row(
            "SELECT c.position_ms FROM track_cues c JOIN tracks t ON t.id = c.track_id
             WHERE t.title = 'Alpha'",
            [],
            |r| r.get(0),
        )
        .expect("alpha cue");
    assert_eq!(alpha_cue, 1_000, "Mixxx's cue replaced the local one");
    assert_eq!(playlist_count(&data_dir), 2);
}

#[test]
fn scan_mixxx_db_rejects_non_mixxx_database() {
    let root = tempdir().expect("root");
    let path = root.path().join("other.sqlite");
    Connection::open(&path)
        .expect("create db")
        .execute_batch("CREATE TABLE something (id INTEGER);")
        .expect("schema");
    let (_app_dir, _data_dir, backend) = app();
    let scanned = backend.scan_mixxx_db(ScanMixxxDbRequest {
        path: Some(path.to_string_lossy().to_string()),
    });
    assert!(!scanned.ok);
    let error = scanned.error.expect("error payload");
    assert!(
        error.message.contains("library/track_locations"),
        "{error:?}"
    );
}
