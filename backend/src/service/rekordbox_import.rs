//! Rekordbox desktop library import: tracks and playlists from `master.db`.
//!
//! Tracks are upserted into the local `tracks` table flagged
//! `rekordbox_db_source`, pointing at the desktop library's own ANLZ files for
//! waveforms. A playlist or history session imports as a new local playlist,
//! importing its tracks the same way.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, params};
use uuid::Uuid;

use super::anlz::read_first_beat_from_anlz;
use super::cues::{
    DEFAULT_HOTCUE_COLOR_ID, MAX_HOT_CUES, insert_track_cues, is_valid_color_id, track_has_cues,
};

use super::usb_utils::external_rekordbox_db_candidates;
use super::usb_vendor_compat::DEFAULT_MASTER_DB_KEY;
use super::wine_paths::{translate_wine_path, wine_prefix_of};
use super::{
    BackendService, annotate_imported_playlists, build_track_match_fingerprint,
    external_playlist_key, no_importable_tracks_error, non_empty_db_value, now,
    rekordbox_db_analysis_file_candidates, resolve_rekordbox_db_resource_path,
    save_imported_playlist,
};
use crate::edb::table_exists;
use crate::error::{BackendError, BackendResult};
use crate::i18n::{Text, tr};
use crate::logging::{self, Level};
use crate::models::{
    ExternalPlaylistKind, ExternalPlaylistSummary, ImportExternalPlaylistData,
    ImportExternalPlaylistRequest, ListExternalPlaylistsData, ListExternalPlaylistsRequest,
    ScanLibraryData, ScanRekordboxDbRequest, TrackCue, WarningEntry,
};
use crate::scanner::is_library_audio_file;

// `djmdPlaylist.Attribute` / `djmdHistory.Attribute`: a list of tracks (as
// opposed to a folder, 1, or a smart playlist, 4, whose tracks rekordbox
// computes and never stores).
const RB_ATTRIBUTE_LIST: i64 = 0;
/// `playlists.import_source` prefix of playlists imported from rekordbox.
const REKORDBOX_LIBRARY: &str = "rekordbox";
// `ParentID` of a top-level playlist / history folder.
const RB_ROOT_PARENT: &str = "root";

/// Resolve the `master.db` to use (explicit path, else auto-detect) and open
/// it read-only with the desktop library key.
fn open_rekordbox_db(path: Option<&str>) -> BackendResult<(PathBuf, Connection)> {
    let master_path = if let Some(p) = path.filter(|s| !s.trim().is_empty()) {
        PathBuf::from(p.trim())
    } else {
        external_rekordbox_db_candidates()
            .into_iter()
            .find(|c| c.is_file())
            .ok_or_else(|| BackendError::Validation(tr::library_error_rekordbox_not_found()))?
    };

    if !master_path.is_file() {
        return Err(BackendError::Validation(
            tr::library_error_rekordbox_not_found_at(master_path.display()),
        ));
    }

    // Open read-only with SQLCipher key
    let conn = Connection::open_with_flags(&master_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| BackendError::Validation(tr::library_error_rekordbox_open(e)))?;
    conn.execute_batch(&format!("PRAGMA key='{}';", DEFAULT_MASTER_DB_KEY))
        .map_err(|e| BackendError::Validation(tr::library_error_rekordbox_key(e)))?;

    // Verify we can read the schema
    if !table_exists(&conn, "djmdContent") {
        return Err(BackendError::Validation(
            tr::library_error_rekordbox_tables(),
        ));
    }
    Ok((master_path, conn))
}

struct RbTrack {
    content_id: String,
    file_path: PathBuf,
    title: String,
    artist: String,
    album: String,
    bpm: Option<f64>,
    tonality: Option<String>,
    duration_ms: Option<i64>,
    anlz_path: Option<String>,
    image_path: Option<String>,
    genre: Option<String>,
}

fn load_rekordbox_db_tracks(conn: &Connection, master_path: &Path) -> BackendResult<Vec<RbTrack>> {
    // Query all non-deleted tracks with available metadata.
    // FolderPath is the full file path (despite the name).
    // BPM is stored as centiBPM integer (12600 = 126.00 BPM).
    // Key is a FK into djmdKey; ScaleName holds the human-readable name.
    // AnalysisDataPath and ImagePath are desktop library virtual paths on Windows
    // (/PIONEER/...) and resolve under the share directory.
    // Genre is a FK into djmdGenre; read it only where the schema has both.
    // Under Wine, FolderPath is a Windows path into the prefix's drives.
    let wine_prefix = wine_prefix_of(master_path);
    let has_genre = table_exists(conn, "djmdGenre")
        && crate::edb::load_table_columns(conn, "djmdContent")?
            .iter()
            .any(|c| c == "GenreID");
    let (genre_col, genre_join) = if has_genre {
        ("g.Name", "LEFT JOIN djmdGenre g ON g.ID = c.GenreID")
    } else {
        ("NULL", "")
    };
    let mut stmt = conn
        .prepare(&format!(
            r#"
        SELECT
          c.FolderPath,
          c.Title,
          COALESCE(ar.Name, c.SrcArtistName, '') AS Artist,
          COALESCE(al.Name, '')                   AS Album,
          c.BPM,
          k.ScaleName,
          c.Length,
          c.AnalysisDataPath,
          c.ImagePath,
          CAST(c.ID AS TEXT),
          {genre_col}
        FROM djmdContent c
        LEFT JOIN djmdArtist  ar ON ar.ID = c.ArtistID
        LEFT JOIN djmdAlbum   al ON al.ID = c.AlbumID
        LEFT JOIN djmdKey     k  ON k.ID  = c.KeyID
        {genre_join}
        WHERE IFNULL(c.rb_local_deleted, 0) = 0
          AND c.FolderPath IS NOT NULL
        "#,
        ))
        .map_err(|e| BackendError::Validation(tr::library_error_rekordbox_query(e)))?;

    let tracks = stmt
        .query_map([], |row| {
            Ok(RbTrack {
                file_path: PathBuf::from(row.get::<_, String>(0)?),
                title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                artist: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                album: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                // 0 = not analyzed in rekordbox: unknown, not 0 BPM.
                bpm: row
                    .get::<_, Option<i64>>(4)?
                    .filter(|b| *b > 0)
                    .map(|b| b as f64 / 100.0),
                tonality: row.get::<_, Option<String>>(5)?,
                duration_ms: row.get::<_, Option<i64>>(6)?.map(|s| s * 1000),
                anlz_path: row.get::<_, Option<String>>(7)?,
                image_path: row.get::<_, Option<String>>(8)?,
                content_id: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                genre: row
                    .get::<_, Option<String>>(10)?
                    .as_deref()
                    .and_then(non_empty_db_value)
                    .map(str::to_string),
            })
        })
        .map_err(|e| BackendError::Validation(tr::library_error_rekordbox_row(e)))?
        .filter_map(|r| r.ok())
        .filter(|t| !t.file_path.to_string_lossy().trim().is_empty())
        .map(|mut t| {
            if let Some(path) = wine_prefix.and_then(|p| translate_wine_path(p, &t.file_path)) {
                t.file_path = path;
            }
            t
        })
        .collect();
    Ok(tracks)
}

/// One row of `djmdPlaylist` / `djmdHistory`.
struct RbListNode {
    id: String,
    name: String,
    seq: i64,
    attribute: i64,
    parent_id: String,
    date_created: String,
}

/// Where each kind of list lives in `master.db`.
struct RbListTables {
    lists: &'static str,
    entries: &'static str,
    entry_list_col: &'static str,
    date_created: &'static str,
}

const RB_PLAYLIST_TABLES: RbListTables = RbListTables {
    lists: "djmdPlaylist",
    entries: "djmdSongPlaylist",
    entry_list_col: "PlaylistID",
    date_created: "NULL",
};
const RB_HISTORY_TABLES: RbListTables = RbListTables {
    lists: "djmdHistory",
    entries: "djmdSongHistory",
    entry_list_col: "HistoryID",
    date_created: "DateCreated",
};

fn rb_tables(kind: ExternalPlaylistKind) -> BackendResult<&'static RbListTables> {
    match kind {
        ExternalPlaylistKind::Playlist => Ok(&RB_PLAYLIST_TABLES),
        ExternalPlaylistKind::History => Ok(&RB_HISTORY_TABLES),
        ExternalPlaylistKind::Crate => Err(BackendError::Internal(
            "rekordbox has no crates".to_string(),
        )),
    }
}

fn load_rb_list_nodes(
    conn: &Connection,
    tables: &RbListTables,
) -> BackendResult<HashMap<String, RbListNode>> {
    if !table_exists(conn, tables.lists) {
        return Ok(HashMap::new());
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT CAST(ID AS TEXT), Name, IFNULL(Seq, 0), IFNULL(Attribute, 0),
                IFNULL(CAST(ParentID AS TEXT), ''), IFNULL({date}, '')
         FROM {lists} WHERE IFNULL(rb_local_deleted, 0) = 0",
        date = tables.date_created,
        lists = tables.lists,
    ))?;
    let rows = stmt.query_map([], |row| {
        Ok(RbListNode {
            id: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
            name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            seq: row.get(2)?,
            attribute: row.get(3)?,
            parent_id: row.get(4)?,
            date_created: row.get(5)?,
        })
    })?;
    Ok(rows
        .filter_map(|r| r.ok())
        .map(|node| (node.id.clone(), node))
        .collect())
}

/// Folder names from the top down to `node` (inclusive), plus each level's
/// `Seq` for sorting in rekordbox's tree order. Stops at a missing parent or a
/// cycle.
fn rb_node_path<'a>(
    nodes: &'a HashMap<String, RbListNode>,
    node: &'a RbListNode,
) -> Vec<&'a RbListNode> {
    let mut path = vec![node];
    let mut seen: HashSet<&str> = HashSet::from([node.id.as_str()]);
    let mut current = node;
    while current.parent_id != RB_ROOT_PARENT {
        let Some(parent) = nodes.get(&current.parent_id) else {
            break;
        };
        if !seen.insert(parent.id.as_str()) {
            break;
        }
        path.push(parent);
        current = parent;
    }
    path.reverse();
    path
}

/// Non-deleted entry counts per list id, counting only entries whose track is
/// still in the library.
fn rb_entry_counts(
    conn: &Connection,
    tables: &RbListTables,
) -> BackendResult<HashMap<String, usize>> {
    if !table_exists(conn, tables.entries) {
        return Ok(HashMap::new());
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT CAST(e.{list_col} AS TEXT), COUNT(*)
         FROM {entries} e JOIN djmdContent c ON c.ID = e.ContentID
         WHERE IFNULL(e.rb_local_deleted, 0) = 0 AND IFNULL(c.rb_local_deleted, 0) = 0
         GROUP BY e.{list_col}",
        list_col = tables.entry_list_col,
        entries = tables.entries,
    ))?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?.unwrap_or_default(),
            row.get::<_, i64>(1)?.max(0) as usize,
        ))
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// The rekordbox playlists (tree order, folders shown as "Folder / Name") and
/// history sessions (newest first) that have tracks. Folders and smart
/// playlists are left out -- rekordbox stores no track list for either.
fn list_rekordbox_playlists_from(conn: &Connection) -> BackendResult<Vec<ExternalPlaylistSummary>> {
    let mut items = Vec::new();
    for (kind, tables) in [
        (ExternalPlaylistKind::Playlist, &RB_PLAYLIST_TABLES),
        (ExternalPlaylistKind::History, &RB_HISTORY_TABLES),
    ] {
        let nodes = load_rb_list_nodes(conn, tables)?;
        let counts = rb_entry_counts(conn, tables)?;
        let mut lists: Vec<(Vec<&RbListNode>, usize)> = nodes
            .values()
            .filter(|node| node.attribute == RB_ATTRIBUTE_LIST)
            .filter_map(|node| {
                let count = counts.get(&node.id).copied().filter(|n| *n > 0)?;
                Some((rb_node_path(&nodes, node), count))
            })
            .collect();
        if kind == ExternalPlaylistKind::History {
            lists.sort_by(|(a, _), (b, _)| {
                let (a, b) = (a.last().unwrap(), b.last().unwrap());
                b.date_created
                    .cmp(&a.date_created)
                    .then_with(|| b.id.cmp(&a.id))
            });
        } else {
            lists.sort_by(|(a, _), (b, _)| {
                let key = |path: &Vec<&RbListNode>| {
                    path.iter()
                        .map(|n| (n.seq, n.id.clone()))
                        .collect::<Vec<_>>()
                };
                key(a).cmp(&key(b))
            });
        }
        items.extend(lists.into_iter().map(|(path, count)| {
            let node = path.last().expect("path holds the node itself");
            // History folders are just year / month numbers; the session name
            // already carries the date.
            let name = if kind == ExternalPlaylistKind::History {
                node.name.clone()
            } else {
                path.iter()
                    .map(|n| n.name.trim())
                    .collect::<Vec<_>>()
                    .join(" / ")
            };
            ExternalPlaylistSummary {
                existing_playlist: None,
                note_text: String::new(),
                id: node.id.clone(),
                name,
                kind,
                track_count: count,
            }
        }));
    }
    Ok(items)
}

/// A playlist's / history session's name and its content ids in play order.
fn load_rb_list_entries(
    conn: &Connection,
    kind: ExternalPlaylistKind,
    id: &str,
) -> BackendResult<(String, Vec<String>)> {
    let tables = rb_tables(kind)?;
    let nodes = load_rb_list_nodes(conn, tables)?;
    let node = nodes
        .get(id)
        .filter(|n| n.attribute == RB_ATTRIBUTE_LIST)
        .ok_or_else(|| {
            BackendError::NotFound(tr::library_error_rekordbox_playlist_not_found(id))
        })?;
    let mut stmt = conn.prepare(&format!(
        "SELECT CAST(ContentID AS TEXT) FROM {entries}
         WHERE CAST({list_col} AS TEXT) = ?1 AND IFNULL(rb_local_deleted, 0) = 0
         ORDER BY TrackNo, ID",
        entries = tables.entries,
        list_col = tables.entry_list_col,
    ))?;
    let ids = stmt
        .query_map(params![id], |row| row.get::<_, Option<String>>(0))?
        .filter_map(|r| r.ok().flatten())
        .collect();
    Ok((node.name.clone(), ids))
}

/// One row of `djmdCue`.
#[derive(Debug, Clone)]
pub(crate) struct RbCue {
    pub kind: i64,
    pub in_msec: i64,
    pub color_table_index: Option<i64>,
    pub comment: Option<String>,
}

// `djmdCue.Kind`: 0 is a memory cue; hot-cue pads A-H are 1, 2, 3, 5, 6, 7,
// 8, 9 (rekordbox skips 4).
const RB_CUE_KIND_MEMORY: i64 = 0;

fn rb_is_hot_cue(kind: i64) -> bool {
    matches!(kind, 1..=3 | 5..=9)
}

/// Convert a track's rekordbox cues into this app's cue list, in position
/// order with the playback-start cue first:
///
/// - Every hot cue (and hot loop, at its start) becomes a hot cue, up to
///   [`MAX_HOT_CUES`].
/// - The earliest memory cue becomes the playback-start cue -- where a CDJ's
///   auto-cue loads the track -- when it lies before the first hot cue (with
///   no hot cues, when another memory cue follows) and has no name or colour
///   (the start cue carries neither).
/// - Every other memory cue becomes a memory cue with its name and colour;
///   one at a hot cue's position is dropped (that hot cue keeps its memory
///   point on export).
pub(crate) fn rekordbox_cues_to_track_cues(cues: &[RbCue]) -> Vec<TrackCue> {
    let to_cue = |c: &RbCue| {
        let memory = c.kind == RB_CUE_KIND_MEMORY;
        // The app writes its palette id as the USB `cue.colorTableIndex`;
        // read it back the same way. An unset (or outside the palette) hot
        // cue gets the default colour; a memory cue stays uncoloured.
        let color_id = c
            .color_table_index
            .and_then(|i| u8::try_from(i).ok())
            .filter(|i| is_valid_color_id(*i))
            .or((!memory).then_some(DEFAULT_HOTCUE_COLOR_ID));
        TrackCue {
            id: Uuid::now_v7().to_string(),
            position_ms: u32::try_from(c.in_msec).unwrap_or(u32::MAX),
            color_id,
            name: c
                .comment
                .as_deref()
                .and_then(non_empty_db_value)
                .map(str::to_string),
            playback_start: false,
            memory,
        }
    };
    let sorted = |memory: bool| {
        let mut out: Vec<TrackCue> = cues
            .iter()
            .filter(|c| c.in_msec >= 0)
            .filter(|c| {
                if memory {
                    c.kind == RB_CUE_KIND_MEMORY
                } else {
                    rb_is_hot_cue(c.kind)
                }
            })
            .map(to_cue)
            .collect();
        out.sort_by_key(|c| c.position_ms);
        out.dedup_by_key(|c| c.position_ms);
        out
    };

    let mut hot = sorted(false);
    hot.truncate(MAX_HOT_CUES as usize);
    let mut memory = sorted(true);

    // The start cue needs another cue after it: a hot cue, or with none,
    // another memory cue (the same rule `split_cues` applies on export).
    let first_hot = hot.first().map(|c| c.position_ms);
    let has_start = memory.first().is_some_and(|first| {
        first_hot.map_or(memory.len() > 1, |hot| first.position_ms < hot)
            && first.name.is_none()
            && first.color_id.is_none()
    });
    let start = has_start.then(|| memory.remove(0));

    let hot_positions: HashSet<u32> = hot.iter().map(|c| c.position_ms).collect();
    memory.retain(|c| !hot_positions.contains(&c.position_ms));
    let mut points = hot;
    points.extend(memory);
    points.sort_by_key(|c| c.position_ms);

    let mut out = Vec::with_capacity(points.len() + 1);
    if let Some(start) = start {
        out.push(TrackCue {
            playback_start: true,
            memory: false,
            ..start
        });
    }
    out.extend(points);
    out
}

fn load_rb_cues(conn: &Connection, content_id: &str) -> BackendResult<Vec<RbCue>> {
    let mut stmt = conn.prepare_cached(
        "SELECT IFNULL(Kind, 0), IFNULL(InMsec, -1), ColorTableIndex, Comment FROM djmdCue
         WHERE CAST(ContentID AS TEXT) = ?1 AND IFNULL(rb_local_deleted, 0) = 0",
    )?;
    let rows = stmt.query_map(params![content_id], |row| {
        Ok(RbCue {
            kind: row.get(0)?,
            in_msec: row.get(1)?,
            color_table_index: row.get(2)?,
            comment: row.get(3)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Upserts `master.db` tracks into the local `tracks` table and tallies what
/// happened. Shared by the whole-library import and the playlist import so
/// both treat a track identically.
struct RekordboxTrackImporter<'a> {
    master: &'a Connection,
    /// Take BPM, key and cues from rekordbox even over the app's own values
    /// (the playlist import's "force update").
    force: bool,
    has_cue_table: bool,
    master_path: PathBuf,
    existing: HashMap<PathBuf, String>,
    artwork_dir: PathBuf,
    now: String,
    indexed: usize,
    updated: usize,
    not_found: Vec<String>,
    unsupported: usize,
    anlz_null: usize,
    anlz_miss: usize,
    anlz_ok: usize,
    artwork_null: usize,
    artwork_miss: usize,
    artwork_ok: usize,
    sample_anlz: Option<String>,
    sample_img: Option<String>,
    cue_tracks: usize,
    cue_tracks_kept_local: usize,
    warnings: Vec<WarningEntry>,
}

impl<'a> RekordboxTrackImporter<'a> {
    fn new(
        master: &'a Connection,
        master_path: PathBuf,
        tx: &Connection,
        data_dir: &Path,
    ) -> BackendResult<Self> {
        // Load existing tracks by file_path for upsert logic
        let existing = {
            let mut stmt = tx.prepare("SELECT file_path, id FROM tracks")?;
            stmt.query_map([], |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    row.get::<_, String>(1)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect()
        };
        let artwork_dir = data_dir.join("analysis").join("artwork");
        let _ = std::fs::create_dir_all(&artwork_dir);
        Ok(Self {
            master,
            force: false,
            has_cue_table: table_exists(master, "djmdCue"),
            master_path,
            existing,
            artwork_dir,
            now: now(),
            indexed: 0,
            updated: 0,
            not_found: Vec::new(),
            unsupported: 0,
            anlz_null: 0,
            anlz_miss: 0,
            anlz_ok: 0,
            artwork_null: 0,
            artwork_miss: 0,
            artwork_ok: 0,
            sample_anlz: None,
            sample_img: None,
            cue_tracks: 0,
            cue_tracks_kept_local: 0,
            warnings: Vec::new(),
        })
    }

    /// Insert or update one track; `None` when it was skipped (file missing
    /// or not a supported audio file).
    fn upsert(&mut self, tx: &Connection, t: &RbTrack) -> BackendResult<Option<String>> {
        // A missing file is skipped and reported, never deleted: the drive it's
        // on may just be unplugged, and deleting the track would take its
        // cues, edits and places in every playlist with it.
        if !t.file_path.exists() {
            self.not_found
                .push(t.file_path.to_string_lossy().into_owned());
            return Ok(None);
        }

        // Same rule as the folder scan (rekordbox also takes video files).
        if !is_library_audio_file(&t.file_path) {
            self.unsupported += 1;
            return Ok(None);
        }

        let fingerprint = build_track_match_fingerprint(
            &t.title,
            &t.artist,
            Some(t.album.as_str()).filter(|s| !s.is_empty()),
        );

        // Resolve (or generate) the track ID before any file writes
        let existing_id = self.existing.get(&t.file_path).cloned();
        let file_path = t.file_path.to_string_lossy();
        let format_ext = crate::audio_format::format_ext_of(&t.file_path);
        let track_id = existing_id
            .clone()
            .unwrap_or_else(|| Uuid::now_v7().to_string());

        let anlz_abs = self.resolve_anlz(t);
        // rekordbox's tracks aren't analyzed locally, so its own beat grid is
        // the only source of the cue editor's grid anchor.
        let first_beat_ms = anlz_abs
            .as_deref()
            .and_then(read_rekordbox_first_beat)
            .map(i64::from);
        let waveform_path = anlz_abs.as_deref().and_then(Path::to_str);
        let artwork_path = self.copy_artwork(t, &track_id);
        let album = Some(&t.album).filter(|a| !a.is_empty());

        if existing_id.is_some() {
            tx.execute(
                // A BPM / key this import fills in is marked as rekordbox's,
                // so the first analysis keeps it (see `kept_analysis_values`).
                // SET expressions see the row's old values.
                r#"UPDATE tracks SET
                    title = ?1, artist = ?2, album = ?3,
                    genre = CASE WHEN genre_source = 'user' THEN genre ELSE COALESCE(?14, genre) END,
                    bpm_analyzer = CASE WHEN ?4 IS NOT NULL
                                         AND (?13 OR bpm IS NULL OR bpm_analyzer = 'rekordbox')
                                        THEN 'rekordbox' ELSE bpm_analyzer END,
                    bpm = CASE WHEN ?4 IS NOT NULL
                                AND (?13 OR bpm IS NULL OR bpm_analyzer = 'rekordbox')
                               THEN ?4 ELSE bpm END,
                    tonality_source = CASE WHEN ?5 IS NOT NULL
                                            AND (?13 OR tonality IS NULL OR tonality_source = 'rekordbox')
                                           THEN 'rekordbox' ELSE tonality_source END,
                    tonality = CASE WHEN ?5 IS NOT NULL
                                     AND (?13 OR tonality IS NULL OR tonality_source = 'rekordbox')
                                    THEN ?5 ELSE tonality END,
                    first_beat_ms_source = CASE WHEN ?15 IS NOT NULL
                                                 AND (?13 OR first_beat_ms IS NULL OR first_beat_ms_source = 'rekordbox')
                                                THEN 'rekordbox' ELSE first_beat_ms_source END,
                    first_beat_ms = CASE WHEN ?15 IS NOT NULL
                                          AND (?13 OR first_beat_ms IS NULL OR first_beat_ms_source = 'rekordbox')
                                         THEN ?15 ELSE first_beat_ms END,
                    duration_ms = COALESCE(duration_ms, ?6),
                    waveform_peaks_path = COALESCE(?7, waveform_peaks_path),
                    artwork_path = COALESCE(?8, artwork_path),
                    format_ext = COALESCE(format_ext, ?12),
                    match_fingerprint = ?9,
                    rekordbox_db_source = 1,
                    updated_at = ?10
                   WHERE id = ?11"#,
                params![
                    t.title,
                    t.artist,
                    album,
                    t.bpm,
                    t.tonality,
                    t.duration_ms,
                    waveform_path,
                    artwork_path,
                    fingerprint,
                    self.now,
                    track_id,
                    format_ext,
                    self.force,
                    t.genre,
                    first_beat_ms
                ],
            )?;
            self.updated += 1;
        } else {
            tx.execute(
                r#"INSERT INTO tracks (
                    id, title, artist, album, bpm, tonality, file_path, format_ext,
                    duration_ms, waveform_peaks_path, artwork_path, match_fingerprint,
                    rekordbox_db_source, created_at, updated_at,
                    bpm_analyzer, tonality_source, genre,
                    first_beat_ms, first_beat_ms_source
                   ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,1,?13,?13,
                     CASE WHEN ?5 IS NOT NULL THEN 'rekordbox' END,
                     CASE WHEN ?6 IS NOT NULL THEN 'rekordbox' END, ?14,
                     ?15, CASE WHEN ?15 IS NOT NULL THEN 'rekordbox' END)"#,
                params![
                    track_id,
                    t.title,
                    t.artist,
                    album,
                    t.bpm,
                    t.tonality,
                    file_path,
                    format_ext,
                    t.duration_ms,
                    waveform_path,
                    artwork_path,
                    fingerprint,
                    self.now,
                    t.genre,
                    first_beat_ms
                ],
            )?;
            self.existing.insert(t.file_path.clone(), track_id.clone());
            self.indexed += 1;
        }
        self.import_cues(tx, t, &track_id)?;
        Ok(Some(track_id))
    }

    /// Cues from `djmdCue`: only onto a track without local cues (local edits
    /// win).
    fn import_cues(&mut self, tx: &Connection, t: &RbTrack, track_id: &str) -> BackendResult<()> {
        if !self.has_cue_table || t.content_id.is_empty() {
            return Ok(());
        }
        let cues = rekordbox_cues_to_track_cues(&load_rb_cues(self.master, &t.content_id)?);
        if cues.is_empty() {
            return Ok(());
        }
        if self.force {
            tx.execute(
                "DELETE FROM track_cues WHERE track_id = ?1",
                params![track_id],
            )?;
        } else if track_has_cues(tx, track_id)? {
            self.cue_tracks_kept_local += 1;
            return Ok(());
        }
        insert_track_cues(tx, track_id, &cues)?;
        self.cue_tracks += 1;
        Ok(())
    }

    /// Waveform: store the original ANLZ path in place - no copy, no conversion.
    /// PWV4 bytes are extracted later when the track is loaded for display.
    fn resolve_anlz(&mut self, t: &RbTrack) -> Option<PathBuf> {
        let Some(anlz_rel) = t.anlz_path.as_deref().and_then(non_empty_db_value) else {
            self.anlz_null += 1;
            return None;
        };
        if self.sample_anlz.is_none() {
            self.sample_anlz = Some(anlz_rel.to_string());
        }
        let resolved = rekordbox_db_analysis_file_candidates(&self.master_path, anlz_rel)
            .into_iter()
            .find(|p| p.is_file());
        if let Some(anlz_abs) = resolved {
            self.anlz_ok += 1;
            Some(anlz_abs)
        } else {
            self.anlz_miss += 1;
            self.warnings.push(logging::log_text(
                Level::Warn,
                "scan-rekordbox-db",
                "scan.rekordbox-db.anlz-not-found",
                &tr::scan_warning_analysis_not_found(anlz_rel),
            ));
            None
        }
    }

    /// Artwork from djmdContent.ImagePath.
    fn copy_artwork(&mut self, t: &RbTrack, track_id: &str) -> Option<String> {
        let Some(img_rel) = t.image_path.as_deref().and_then(non_empty_db_value) else {
            self.artwork_null += 1;
            return None;
        };
        if self.sample_img.is_none() {
            self.sample_img = Some(img_rel.to_string());
        }
        let Some(src) =
            resolve_rekordbox_db_resource_path(&self.master_path, img_rel, |p| p.is_file())
        else {
            self.artwork_miss += 1;
            self.warnings.push(logging::log_text(
                Level::Warn,
                "scan-rekordbox-db",
                "scan.rekordbox-db.artwork-not-found",
                &tr::scan_warning_artwork_not_found(img_rel),
            ));
            return None;
        };
        let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
        let dest = self.artwork_dir.join(format!("{track_id}.{ext}"));
        match std::fs::copy(&src, &dest) {
            Ok(_) => {
                self.artwork_ok += 1;
                Some(dest.to_string_lossy().to_string())
            }
            Err(e) => {
                self.artwork_miss += 1;
                self.warnings.push(logging::log_text(
                    Level::Error,
                    "scan-rekordbox-db",
                    "scan.rekordbox-db.artwork-copy-failed",
                    &tr::scan_warning_artwork_copy_failed(src.display(), e),
                ));
                None
            }
        }
    }

    /// The summary log entries, appended after the per-track warnings.
    fn finish_warnings(&mut self) -> Vec<WarningEntry> {
        let mut note = |level, code: &str, text: Text| {
            self.warnings
                .push(logging::log_text(level, "scan-rekordbox-db", code, &text));
        };
        // Each skipped track's file, so the status line's "(see event log)" finds them.
        for path in &self.not_found {
            note(
                Level::Warn,
                "scan.rekordbox-db.file-not-found",
                tr::scan_warning_track_file_not_found(path),
            );
        }
        if self.cue_tracks > 0 {
            note(
                Level::Info,
                "scan.rekordbox-db.cues-imported",
                tr::scan_note_cues_imported(self.cue_tracks),
            );
        }
        if self.cue_tracks_kept_local > 0 {
            note(
                Level::Info,
                "scan.rekordbox-db.cues-kept-local",
                tr::scan_note_cues_kept_local(self.cue_tracks_kept_local, "rekordbox"),
            );
        }
        if self.unsupported > 0 {
            note(
                Level::Info,
                "scan.rekordbox-db.unsupported-skipped",
                tr::scan_note_unsupported_skipped(self.unsupported),
            );
        }
        if self.anlz_miss > 0 {
            note(
                Level::Warn,
                "scan.rekordbox-db.anlz-miss-summary",
                tr::scan_warning_analysis_missing(self.anlz_miss),
            );
        }
        if self.artwork_miss > 0 {
            note(
                Level::Warn,
                "scan.rekordbox-db.artwork-miss-summary",
                tr::scan_warning_artwork_missing(self.artwork_miss),
            );
        }
        // What the importer resolved: for a bug report, not the user.
        logging::debug(
            "scan-rekordbox-db",
            format!(
                "AnalysisDataPath sample: {:?}, ImagePath sample: {:?}; ANLZ paths: {} resolved, \
                 {} tracks without one; artwork: {} copied, {} tracks without one",
                self.sample_anlz,
                self.sample_img,
                self.anlz_ok,
                self.anlz_null,
                self.artwork_ok,
                self.artwork_null
            ),
        );
        std::mem::take(&mut self.warnings)
    }
}

impl BackendService {
    pub fn scan_rekordbox_db(&self, req: ScanRekordboxDbRequest) -> BackendResult<ScanLibraryData> {
        let (master_path, conn) = open_rekordbox_db(req.path.as_deref())?;
        let tracks = load_rekordbox_db_tracks(&conn, &master_path)?;

        let mut db_conn = self.db.connect()?;
        let tx = db_conn.transaction()?;
        let mut importer =
            RekordboxTrackImporter::new(&conn, master_path, &tx, &self.db.data_dir())?;
        for t in &tracks {
            importer.upsert(&tx, t)?;
        }
        tx.commit()?;

        let warnings = importer.finish_warnings();
        let status_text = match importer.not_found.len() {
            0 => crate::i18n::tr::library_import_done_rekordbox(importer.indexed, importer.updated),
            count => crate::i18n::tr::library_import_done_rekordbox_not_found(
                importer.indexed,
                importer.updated,
                count,
            ),
        }
        .render();
        Ok(ScanLibraryData {
            job_id: Uuid::now_v7().to_string(),
            indexed: importer.indexed,
            updated: importer.updated,
            // Imports never remove tracks (see `RekordboxTrackImporter::upsert`).
            removed: 0,
            not_found: importer.not_found,
            warnings,
            status_text,
        })
    }

    pub fn list_rekordbox_playlists(
        &self,
        req: ListExternalPlaylistsRequest,
    ) -> BackendResult<ListExternalPlaylistsData> {
        let (_, conn) = open_rekordbox_db(req.path.as_deref())?;
        let mut items = list_rekordbox_playlists_from(&conn)?;
        let local = self.db.connect()?;
        annotate_imported_playlists(&local, REKORDBOX_LIBRARY, &mut items)?;
        Ok(ListExternalPlaylistsData { items })
    }

    /// Import one rekordbox playlist or history session as a new local
    /// playlist, importing its tracks like the whole-library import does.
    /// Tracks the library leaves out (missing or unsupported files) are
    /// skipped, and a track listed twice is added once.
    pub fn import_rekordbox_playlist(
        &self,
        req: ImportExternalPlaylistRequest,
    ) -> BackendResult<ImportExternalPlaylistData> {
        let (master_path, conn) = open_rekordbox_db(req.path.as_deref())?;
        let (rb_name, entry_ids) = load_rb_list_entries(&conn, req.kind, req.id.trim())?;
        let name = non_empty_db_value(&rb_name)
            .map(str::to_string)
            .unwrap_or_else(|| "rekordbox playlist".to_string());

        let wanted: HashSet<&str> = entry_ids.iter().map(String::as_str).collect();
        let tracks_by_id: HashMap<String, RbTrack> = load_rekordbox_db_tracks(&conn, &master_path)?
            .into_iter()
            .filter(|t| wanted.contains(t.content_id.as_str()))
            .map(|t| (t.content_id.clone(), t))
            .collect();

        let mut db_conn = self.db.connect()?;
        let tx = db_conn.transaction()?;
        let mut importer =
            RekordboxTrackImporter::new(&conn, master_path, &tx, &self.db.data_dir())?;
        importer.force = req.force;

        let mut local_ids: Vec<String> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        let mut duplicates = 0usize;
        let mut deleted_in_rekordbox = 0usize;
        for content_id in &entry_ids {
            if !seen.insert(content_id.as_str()) {
                duplicates += 1;
                continue;
            }
            let Some(t) = tracks_by_id.get(content_id) else {
                deleted_in_rekordbox += 1;
                continue;
            };
            if let Some(track_id) = importer.upsert(&tx, t)? {
                local_ids.push(track_id);
            }
        }

        if local_ids.is_empty() {
            // Dropping `tx` rolls back the (partial) track upserts too.
            return Err(no_importable_tracks_error(&name, entry_ids.len()));
        }
        let key = external_playlist_key(REKORDBOX_LIBRARY, req.kind, &req.id);
        let (playlist, updated_existing) = save_imported_playlist(&tx, &key, &name, &local_ids)?;
        tx.commit()?;

        let mut warnings = importer.finish_warnings();
        if duplicates > 0 {
            warnings.push(logging::log_text(
                Level::Info,
                "scan-rekordbox-db",
                "scan.rekordbox-db.playlist-duplicates-skipped",
                &tr::scan_note_playlist_duplicates(duplicates, &name),
            ));
        }
        if deleted_in_rekordbox > 0 {
            warnings.push(logging::log_text(
                Level::Warn,
                "scan-rekordbox-db",
                "scan.rekordbox-db.playlist-tracks-deleted",
                &tr::scan_warning_playlist_tracks_deleted(deleted_in_rekordbox, &name, "rekordbox"),
            ));
        }
        Ok(ImportExternalPlaylistData {
            playlist_id: playlist.id,
            name: playlist.name,
            updated_existing,
            added: local_ids.len(),
            skipped: entry_ids.len() - local_ids.len(),
            indexed: importer.indexed,
            not_found: importer.not_found,
            warnings,
        })
    }
}

/// First beat of rekordbox's beat grid for a resolved ANLZ path. The grid
/// (`PQTZ`) lives in the `.DAT` file, while the stored path usually points at
/// the `.EXT` (preferred for its colour waveform), so read the `.DAT` first.
fn read_rekordbox_first_beat(path: &Path) -> Option<u32> {
    [path.with_extension("DAT"), path.to_path_buf()]
        .iter()
        .find_map(|p| read_first_beat_from_anlz(&std::fs::read(p).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rb_cue(kind: i64, in_msec: i64) -> RbCue {
        RbCue {
            kind,
            in_msec,
            color_table_index: None,
            comment: None,
        }
    }

    #[test]
    fn rekordbox_hot_cue_kinds_skip_four() {
        let hot: Vec<i64> = (0..=10).filter(|k| rb_is_hot_cue(*k)).collect();
        assert_eq!(hot, vec![1, 2, 3, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn rekordbox_cues_map_hot_cues_and_leading_memory_cue() {
        // A layout seen in a real rekordbox library: one memory cue, then pads A-H.
        let mut cues = vec![rb_cue(0, 53)];
        for (i, kind) in [1, 2, 3, 5, 6, 7, 8, 9].into_iter().enumerate() {
            cues.push(rb_cue(kind, 54_911 + i as i64 * 1000));
        }
        let out = rekordbox_cues_to_track_cues(&cues);
        assert_eq!(out.len(), 9);
        assert!(out[0].playback_start);
        assert_eq!(out[0].position_ms, 53);
        assert!(out[1..].iter().all(TrackCue::is_hot));
        assert_eq!(out[1].position_ms, 54_911);
        assert_eq!(out[8].position_ms, 61_911);
    }

    /// Each cue as (position, 'S' start / 'H' hot / 'M' memory).
    fn summary(cues: &[TrackCue]) -> Vec<(u32, char)> {
        cues.iter()
            .map(|c| {
                let kind = if c.playback_start {
                    'S'
                } else if c.memory {
                    'M'
                } else {
                    'H'
                };
                (c.position_ms, kind)
            })
            .collect()
    }

    #[test]
    fn rekordbox_memory_cues_stay_memory_cues_after_the_start() {
        let cues = vec![
            rb_cue(0, 5_000),
            rb_cue(1, 10_000),
            rb_cue(0, 20_000),
            rb_cue(0, 30_000),
        ];
        assert_eq!(
            summary(&rekordbox_cues_to_track_cues(&cues)),
            vec![(5_000, 'S'), (10_000, 'H'), (20_000, 'M'), (30_000, 'M')]
        );
    }

    #[test]
    fn rekordbox_memory_cues_have_no_limit_and_keep_name_and_colour() {
        // 8 hot cues and 12 memory cues: every one is kept.
        let mut cues: Vec<RbCue> = [1, 2, 3, 5, 6, 7, 8, 9]
            .into_iter()
            .enumerate()
            .map(|(i, kind)| rb_cue(kind, 10_000 + i as i64 * 1_000))
            .collect();
        cues.extend((0..12).map(|i| rb_cue(0, 40_000 + i * 1_000)));
        cues.push(RbCue {
            kind: 0,
            in_msec: 60_000,
            color_table_index: Some(3),
            comment: Some("Outro".to_string()),
        });
        let out = rekordbox_cues_to_track_cues(&cues);
        assert_eq!(out.iter().filter(|c| c.is_hot()).count(), 8);
        assert_eq!(out.iter().filter(|c| c.memory).count(), 13);
        let outro = out.last().unwrap();
        assert_eq!(
            (
                outro.position_ms,
                outro.memory,
                outro.color_id,
                outro.name.as_deref()
            ),
            (60_000, true, Some(3), Some("Outro"))
        );
        // An uncoloured memory cue stays uncoloured (no default colour).
        assert_eq!(out[8].color_id, None);
    }

    #[test]
    fn rekordbox_named_or_coloured_leading_memory_cue_is_a_memory_cue() {
        let named = RbCue {
            kind: 0,
            in_msec: 1_000,
            color_table_index: None,
            comment: Some("Intro".to_string()),
        };
        let coloured = RbCue {
            color_table_index: Some(2),
            comment: None,
            ..named.clone()
        };
        for leading in [named, coloured] {
            let out = rekordbox_cues_to_track_cues(&[leading, rb_cue(1, 10_000)]);
            assert_eq!(summary(&out), vec![(1_000, 'M'), (10_000, 'H')]);
        }
    }

    #[test]
    fn rekordbox_memory_only_track_gets_a_start_and_memory_cues() {
        // The first unnamed, uncoloured memory cue is the start; a lone one
        // has nothing after it, so it stays a memory cue.
        let cues: Vec<RbCue> = (0..12).map(|i| rb_cue(0, 1_000 + i * 1_000)).collect();
        let out = rekordbox_cues_to_track_cues(&cues);
        assert_eq!(out.len(), 12);
        assert!(out[0].playback_start);
        assert!(out[1..].iter().all(|c| c.memory));
        assert_eq!(
            summary(&rekordbox_cues_to_track_cues(&[rb_cue(0, 1_000)])),
            vec![(1_000, 'M')]
        );
    }

    #[test]
    fn rekordbox_memory_cue_after_first_hot_cue_is_no_start() {
        // No memory cue before the hot cue: nothing becomes the start, and a
        // memory cue on a hot cue's position is dropped (the hot cue keeps
        // its memory point).
        let cues = vec![rb_cue(0, 20_000), rb_cue(1, 10_000), rb_cue(0, 10_000)];
        assert_eq!(
            summary(&rekordbox_cues_to_track_cues(&cues)),
            vec![(10_000, 'H'), (20_000, 'M')]
        );
    }
}
