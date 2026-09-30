//! Rekordbox desktop library import: tracks and playlists from `master.db`.
//!
//! Tracks are upserted into the local `tracks` table flagged
//! `master_db_source`, pointing at the desktop library's own ANLZ files for
//! waveforms. A playlist or history session imports as a new local playlist,
//! importing its tracks the same way.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, params};
use uuid::Uuid;

use super::usb_utils::external_master_db_candidates;
use super::usb_vendor_compat::DEFAULT_MASTER_DB_KEY;
use super::{
    BackendService, build_track_match_fingerprint, insert_local_playlist,
    master_db_analysis_file_candidates, no_importable_tracks_error, non_empty_db_value, now,
    resolve_master_db_resource_path,
};
use crate::edb::table_exists;
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{
    ExternalPlaylistKind, ExternalPlaylistSummary, ImportExternalPlaylistData,
    ImportExternalPlaylistRequest, ListExternalPlaylistsData, ListExternalPlaylistsRequest,
    ScanLibraryData, ScanMasterDbRequest, WarningEntry,
};
use crate::scanner::is_library_audio_file;

// `djmdPlaylist.Attribute` / `djmdHistory.Attribute`: a list of tracks (as
// opposed to a folder, 1, or a smart playlist, 4, whose tracks rekordbox
// computes and never stores).
const RB_ATTRIBUTE_LIST: i64 = 0;
// `ParentID` of a top-level playlist / history folder.
const RB_ROOT_PARENT: &str = "root";

/// Resolve the `master.db` to use (explicit path, else auto-detect) and open
/// it read-only with the desktop library key.
fn open_master_db(path: Option<&str>) -> BackendResult<(PathBuf, Connection)> {
    let master_path = if let Some(p) = path.filter(|s| !s.trim().is_empty()) {
        PathBuf::from(p.trim())
    } else {
        external_master_db_candidates()
            .into_iter()
            .find(|c| c.is_file())
            .ok_or_else(|| BackendError::Validation("master.db not found".to_string()))?
    };

    if !master_path.is_file() {
        return Err(BackendError::Validation(format!(
            "master.db not found: {}",
            master_path.display()
        )));
    }

    // Open read-only with SQLCipher key
    let conn = Connection::open_with_flags(&master_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| BackendError::Validation(format!("could not open master.db: {e}")))?;
    conn.execute_batch(&format!("PRAGMA key='{}';", DEFAULT_MASTER_DB_KEY))
        .map_err(|e| BackendError::Validation(format!("master.db key failed: {e}")))?;

    // Verify we can read the schema
    if !table_exists(&conn, "djmdContent") {
        return Err(BackendError::Validation(
            "master.db opened but djmdContent table not found (wrong key or version)".to_string(),
        ));
    }
    Ok((master_path, conn))
}

struct RbTrack {
    content_id: String,
    file_path: String,
    title: String,
    artist: String,
    album: String,
    bpm: Option<f64>,
    tonality: Option<String>,
    duration_ms: Option<i64>,
    anlz_path: Option<String>,
    image_path: Option<String>,
}

fn load_master_db_tracks(conn: &Connection) -> BackendResult<Vec<RbTrack>> {
    // Query all non-deleted tracks with available metadata.
    // FolderPath is the full file path (despite the name).
    // BPM is stored as centiBPM integer (12600 = 126.00 BPM).
    // Key is a FK into djmdKey; ScaleName holds the human-readable name.
    // AnalysisDataPath and ImagePath are desktop library virtual paths on Windows
    // (/PIONEER/...) and resolve under the share directory.
    let mut stmt = conn
        .prepare(
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
          CAST(c.ID AS TEXT)
        FROM djmdContent c
        LEFT JOIN djmdArtist  ar ON ar.ID = c.ArtistID
        LEFT JOIN djmdAlbum   al ON al.ID = c.AlbumID
        LEFT JOIN djmdKey     k  ON k.ID  = c.KeyID
        WHERE IFNULL(c.rb_local_deleted, 0) = 0
          AND c.FolderPath IS NOT NULL
        "#,
        )
        .map_err(|e| BackendError::Validation(format!("master.db query failed: {e}")))?;

    let tracks = stmt
        .query_map([], |row| {
            Ok(RbTrack {
                file_path: row.get::<_, String>(0)?,
                title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                artist: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                album: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                bpm: row.get::<_, Option<i64>>(4)?.map(|b| b as f64 / 100.0),
                tonality: row.get::<_, Option<String>>(5)?,
                duration_ms: row.get::<_, Option<i64>>(6)?.map(|s| s * 1000),
                anlz_path: row.get::<_, Option<String>>(7)?,
                image_path: row.get::<_, Option<String>>(8)?,
                content_id: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
            })
        })
        .map_err(|e| BackendError::Validation(format!("master.db row error: {e}")))?
        .filter_map(|r| r.ok())
        .filter(|t| !t.file_path.trim().is_empty())
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
        ExternalPlaylistKind::Crate => Err(BackendError::Validation(
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
        .ok_or_else(|| BackendError::NotFound(format!("rekordbox playlist not found: {id}")))?;
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

/// Upserts `master.db` tracks into the local `tracks` table and tallies what
/// happened. Shared by the whole-library import and the playlist import so
/// both treat a track identically.
struct MasterDbTrackImporter {
    master_path: PathBuf,
    existing: HashMap<String, String>,
    artwork_dir: PathBuf,
    now: String,
    indexed: usize,
    updated: usize,
    removed: usize,
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
    warnings: Vec<WarningEntry>,
}

impl MasterDbTrackImporter {
    fn new(master_path: PathBuf, tx: &Connection, data_dir: &Path) -> BackendResult<Self> {
        // Load existing tracks by file_path for upsert logic
        let existing = {
            let mut stmt = tx.prepare("SELECT file_path, id FROM tracks")?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect()
        };
        let artwork_dir = data_dir.join("analysis").join("artwork");
        let _ = std::fs::create_dir_all(&artwork_dir);
        Ok(Self {
            master_path,
            existing,
            artwork_dir,
            now: now(),
            indexed: 0,
            updated: 0,
            removed: 0,
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
            warnings: Vec::new(),
        })
    }

    /// Insert or update one track; `None` when it was skipped (file missing
    /// or not a supported audio file).
    fn upsert(&mut self, tx: &Connection, t: &RbTrack) -> BackendResult<Option<String>> {
        if !Path::new(&t.file_path).exists() {
            // Remove from local DB if previously imported; skip upsert
            if self.existing.remove(&t.file_path).is_some() {
                tx.execute(
                    "DELETE FROM tracks WHERE file_path = ?1",
                    params![t.file_path],
                )?;
                self.removed += 1;
            }
            self.not_found.push(t.file_path.clone());
            return Ok(None);
        }

        // Same rule as the folder scan (rekordbox also takes video files).
        if !is_library_audio_file(Path::new(&t.file_path)) {
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
        let track_id = existing_id
            .clone()
            .unwrap_or_else(|| Uuid::now_v7().to_string());

        let waveform_path = self.resolve_anlz(t);
        let artwork_path = self.copy_artwork(t, &track_id);
        let album = Some(&t.album).filter(|a| !a.is_empty());

        if existing_id.is_some() {
            tx.execute(
                r#"UPDATE tracks SET
                    title = ?1, artist = ?2, album = ?3,
                    bpm = COALESCE(bpm, ?4),
                    tonality = COALESCE(tonality, ?5),
                    duration_ms = COALESCE(duration_ms, ?6),
                    waveform_peaks_path = COALESCE(?7, waveform_peaks_path),
                    artwork_path = COALESCE(?8, artwork_path),
                    format_ext = COALESCE(format_ext, ?12),
                    match_fingerprint = ?9,
                    master_db_source = 1,
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
                    crate::utils::format_ext_from_path(&t.file_path)
                ],
            )?;
            self.updated += 1;
        } else {
            tx.execute(
                r#"INSERT INTO tracks (
                    id, title, artist, album, bpm, tonality, file_path, format_ext,
                    duration_ms, waveform_peaks_path, artwork_path, match_fingerprint,
                    master_db_source, created_at, updated_at
                   ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,1,?13,?13)"#,
                params![
                    track_id,
                    t.title,
                    t.artist,
                    album,
                    t.bpm,
                    t.tonality,
                    t.file_path,
                    crate::utils::format_ext_from_path(&t.file_path),
                    t.duration_ms,
                    waveform_path,
                    artwork_path,
                    fingerprint,
                    self.now
                ],
            )?;
            self.existing.insert(t.file_path.clone(), track_id.clone());
            self.indexed += 1;
        }
        Ok(Some(track_id))
    }

    /// Waveform: store the original ANLZ path in place - no copy, no conversion.
    /// PWV4 bytes are extracted later when the track is loaded for display.
    fn resolve_anlz(&mut self, t: &RbTrack) -> Option<String> {
        let Some(anlz_rel) = t.anlz_path.as_deref().and_then(non_empty_db_value) else {
            self.anlz_null += 1;
            return None;
        };
        if self.sample_anlz.is_none() {
            self.sample_anlz = Some(anlz_rel.to_string());
        }
        let resolved = master_db_analysis_file_candidates(&self.master_path, anlz_rel)
            .into_iter()
            .find(|p| p.is_file());
        if let Some(anlz_abs) = resolved {
            self.anlz_ok += 1;
            anlz_abs.to_str().map(str::to_owned)
        } else {
            self.anlz_miss += 1;
            self.warnings.push(logging::log(
                Level::Warn,
                "scan-master-db",
                "scan.master-db.anlz-not-found",
                format!("ANLZ not found (AnalysisDataPath={anlz_rel:?})"),
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
            resolve_master_db_resource_path(&self.master_path, img_rel, |p| p.is_file())
        else {
            self.artwork_miss += 1;
            self.warnings.push(logging::log(
                Level::Warn,
                "scan-master-db",
                "scan.master-db.artwork-not-found",
                format!("artwork not found (ImagePath={img_rel:?})"),
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
                self.warnings.push(logging::log(
                    Level::Error,
                    "scan-master-db",
                    "scan.master-db.artwork-copy-failed",
                    format!("artwork copy failed {src:?} -> {dest:?}: {e}"),
                ));
                None
            }
        }
    }

    /// The summary log entries, appended after the per-track warnings.
    fn finish_warnings(&mut self) -> Vec<WarningEntry> {
        let mut summary = |level, code: &str, message: String| {
            self.warnings
                .push(logging::log(level, "scan-master-db", code, message));
        };
        if let Some(p) = &self.sample_anlz {
            summary(
                Level::Info,
                "scan.master-db.anlz-sample",
                format!("AnalysisDataPath sample: {p}"),
            );
        }
        if let Some(p) = &self.sample_img {
            summary(
                Level::Info,
                "scan.master-db.image-sample",
                format!("ImagePath sample: {p}"),
            );
        }
        if self.unsupported > 0 {
            summary(
                Level::Info,
                "scan.master-db.unsupported-skipped",
                format!(
                    "{} track(s) skipped: not a supported audio file",
                    self.unsupported
                ),
            );
        }
        if self.anlz_null > 0 {
            summary(
                Level::Info,
                "scan.master-db.anlz-null",
                format!("{} track(s) have no AnalysisDataPath", self.anlz_null),
            );
        }
        if self.anlz_miss > 0 {
            summary(
                Level::Warn,
                "scan.master-db.anlz-miss-summary",
                format!("{} ANLZ path(s) not found on disk", self.anlz_miss),
            );
        }
        if self.anlz_ok > 0 {
            summary(
                Level::Info,
                "scan.master-db.anlz-ok",
                format!("{} ANLZ path(s) resolved OK", self.anlz_ok),
            );
        }
        if self.artwork_null > 0 {
            summary(
                Level::Info,
                "scan.master-db.artwork-null",
                format!("{} track(s) have no ImagePath", self.artwork_null),
            );
        }
        if self.artwork_miss > 0 {
            summary(
                Level::Warn,
                "scan.master-db.artwork-miss-summary",
                format!(
                    "{} artwork source file(s) not found or copy failed",
                    self.artwork_miss
                ),
            );
        }
        if self.artwork_ok > 0 {
            summary(
                Level::Info,
                "scan.master-db.artwork-ok",
                format!("{} artwork file(s) copied OK", self.artwork_ok),
            );
        }
        std::mem::take(&mut self.warnings)
    }
}

impl BackendService {
    pub fn scan_master_db(&self, req: ScanMasterDbRequest) -> BackendResult<ScanLibraryData> {
        let (master_path, conn) = open_master_db(req.path.as_deref())?;
        let tracks = load_master_db_tracks(&conn)?;

        let mut db_conn = self.db.connect()?;
        let tx = db_conn.transaction()?;
        let mut importer = MasterDbTrackImporter::new(master_path, &tx, &self.db.data_dir())?;
        for t in &tracks {
            importer.upsert(&tx, t)?;
        }
        tx.commit()?;

        let warnings = importer.finish_warnings();
        Ok(ScanLibraryData {
            job_id: Uuid::now_v7().to_string(),
            indexed: importer.indexed,
            updated: importer.updated,
            removed: importer.removed,
            not_found: importer.not_found,
            // Master-DB scan isn't scoped to source folders; the post-scan
            // library facts are only surfaced for `scan_library`.
            scoped_track_count: 0,
            album_count: 0,
            unanalyzed_count: 0,
            warnings,
        })
    }

    pub fn list_rekordbox_playlists(
        &self,
        req: ListExternalPlaylistsRequest,
    ) -> BackendResult<ListExternalPlaylistsData> {
        let (_, conn) = open_master_db(req.path.as_deref())?;
        Ok(ListExternalPlaylistsData {
            items: list_rekordbox_playlists_from(&conn)?,
        })
    }

    /// Import one rekordbox playlist or history session as a new local
    /// playlist, importing its tracks like the whole-library import does.
    /// Tracks the library leaves out (missing or unsupported files) are
    /// skipped, and a track listed twice is added once.
    pub fn import_rekordbox_playlist(
        &self,
        req: ImportExternalPlaylistRequest,
    ) -> BackendResult<ImportExternalPlaylistData> {
        let (master_path, conn) = open_master_db(req.path.as_deref())?;
        let (rb_name, entry_ids) = load_rb_list_entries(&conn, req.kind, req.id.trim())?;
        let name = non_empty_db_value(&rb_name)
            .map(str::to_string)
            .unwrap_or_else(|| "rekordbox playlist".to_string());

        let wanted: HashSet<&str> = entry_ids.iter().map(String::as_str).collect();
        let tracks_by_id: HashMap<String, RbTrack> = load_master_db_tracks(&conn)?
            .into_iter()
            .filter(|t| wanted.contains(t.content_id.as_str()))
            .map(|t| (t.content_id.clone(), t))
            .collect();

        let mut db_conn = self.db.connect()?;
        let tx = db_conn.transaction()?;
        let mut importer = MasterDbTrackImporter::new(master_path, &tx, &self.db.data_dir())?;

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
        let playlist_id = insert_local_playlist(&tx, &name, &local_ids)?;
        tx.commit()?;

        let mut warnings = importer.finish_warnings();
        if duplicates > 0 {
            warnings.push(logging::log(
                Level::Info,
                "scan-master-db",
                "scan.master-db.playlist-duplicates-skipped",
                format!("{duplicates} repeated track(s) in {name:?} added once"),
            ));
        }
        if deleted_in_rekordbox > 0 {
            warnings.push(logging::log(
                Level::Warn,
                "scan-master-db",
                "scan.master-db.playlist-tracks-deleted",
                format!(
                    "{deleted_in_rekordbox} track(s) in {name:?} are no longer in the rekordbox library"
                ),
            ));
        }
        Ok(ImportExternalPlaylistData {
            playlist_id,
            name,
            added: local_ids.len(),
            indexed: importer.indexed,
            not_found: importer.not_found,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_service() -> (tempfile::TempDir, BackendService) {
        let dir = tempfile::tempdir().expect("service data dir");
        let service = BackendService::new(dir.path()).expect("backend service");
        (dir, service)
    }

    /// A `master.db` shaped like rekordbox 6/7's: text ids, `root` parents,
    /// folders (Attribute 1) and a smart playlist (Attribute 4).
    fn create_master_db_with_playlists(root: &Path, media: &[PathBuf]) -> PathBuf {
        let path = root.join("master.db");
        let conn = Connection::open(&path).expect("create master db");
        conn.execute_batch(&format!("PRAGMA key='{DEFAULT_MASTER_DB_KEY}';"))
            .expect("key");
        conn.execute_batch(
            r#"
            CREATE TABLE djmdArtist (ID VARCHAR(255) PRIMARY KEY, Name TEXT);
            CREATE TABLE djmdAlbum (ID VARCHAR(255) PRIMARY KEY, Name TEXT);
            CREATE TABLE djmdKey (ID VARCHAR(255) PRIMARY KEY, ScaleName TEXT);
            CREATE TABLE djmdContent (
              ID VARCHAR(255) PRIMARY KEY, FolderPath TEXT, Title TEXT, SrcArtistName TEXT,
              ArtistID TEXT, AlbumID TEXT, BPM INTEGER, KeyID TEXT, Length INTEGER,
              AnalysisDataPath TEXT, ImagePath TEXT, rb_local_deleted INTEGER DEFAULT 0
            );
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
                "INSERT INTO djmdContent (ID, FolderPath, Title, SrcArtistName, BPM, Length, rb_local_deleted)
                 VALUES (?1, ?2, ?3, 'Artist', 12800, 200, ?4)",
                params![id, file_path, title, deleted],
            )
            .expect("insert content");
        }
        path
    }

    fn playlist_titles(service: &BackendService, playlist_id: &str) -> Vec<String> {
        let conn = service.db.connect().expect("service db");
        conn.prepare(
            "SELECT t.title FROM playlist_tracks pt JOIN tracks t ON t.id = pt.track_id
             WHERE pt.playlist_id = ?1 ORDER BY pt.position",
        )
        .unwrap()
        .query_map(params![playlist_id], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    #[test]
    fn list_and_import_rekordbox_playlists_and_history() {
        let rb_root = tempfile::tempdir().expect("rb root");
        let media_root = tempfile::tempdir().expect("media root");
        let (_service_dir, service) = test_service();
        let media: Vec<PathBuf> = ["a.mp3", "b.flac", "c.wav"]
            .iter()
            .map(|n| media_root.path().join(n))
            .collect();
        for p in &media {
            std::fs::write(p, b"audio").expect("write media");
        }
        let path = Some(
            create_master_db_with_playlists(rb_root.path(), &media)
                .to_string_lossy()
                .to_string(),
        );

        let listed = service
            .list_rekordbox_playlists(ListExternalPlaylistsRequest { path: path.clone() })
            .expect("list")
            .items;
        let summary: Vec<_> = listed
            .iter()
            .map(|p| (p.id.as_str(), p.name.as_str(), p.kind, p.track_count))
            .collect();
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

        let imported = service
            .import_rekordbox_playlist(ImportExternalPlaylistRequest {
                path: path.clone(),
                kind: ExternalPlaylistKind::Playlist,
                id: "11".to_string(),
            })
            .expect("import playlist");
        assert_eq!(imported.name, "Friday");
        assert_eq!(imported.added, 3);
        assert_eq!(imported.not_found, vec!["/nowhere/missing.mp3".to_string()]);
        let codes: Vec<_> = imported.warnings.iter().map(|w| w.code.as_str()).collect();
        for code in [
            "scan.master-db.playlist-duplicates-skipped",
            "scan.master-db.playlist-tracks-deleted",
        ] {
            assert!(codes.contains(&code), "expected {code}, got {codes:?}");
        }
        assert_eq!(
            playlist_titles(&service, &imported.playlist_id),
            vec!["Charlie", "Alpha", "Bravo"]
        );

        let history = service
            .import_rekordbox_playlist(ImportExternalPlaylistRequest {
                path: path.clone(),
                kind: ExternalPlaylistKind::History,
                id: "h2".to_string(),
            })
            .expect("import history");
        assert_eq!(history.indexed, 0, "tracks reused from the first import");
        assert_eq!(
            playlist_titles(&service, &history.playlist_id),
            vec!["Charlie", "Bravo"]
        );

        // Nothing importable: an error, and no empty playlist left behind.
        let err = service
            .import_rekordbox_playlist(ImportExternalPlaylistRequest {
                path: path.clone(),
                kind: ExternalPlaylistKind::Playlist,
                id: "15".to_string(),
            })
            .unwrap_err();
        assert!(err.to_string().contains("none of the 1 track(s)"), "{err}");
        let playlists: i64 = service
            .db
            .connect()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM playlists", [], |r| r.get(0))
            .unwrap();
        assert_eq!(playlists, 2);

        for (kind, id) in [
            (ExternalPlaylistKind::Playlist, "10"), // a folder
            (ExternalPlaylistKind::Playlist, "12"), // a smart playlist
            (ExternalPlaylistKind::Playlist, "14"), // deleted
            (ExternalPlaylistKind::Crate, "11"),
        ] {
            assert!(
                service
                    .import_rekordbox_playlist(ImportExternalPlaylistRequest {
                        path: path.clone(),
                        kind,
                        id: id.to_string(),
                    })
                    .is_err(),
                "{kind:?} {id} should not import"
            );
        }
    }
}
