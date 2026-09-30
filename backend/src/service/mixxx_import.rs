//! Mixxx library import: tracks + hot cues from `mixxxdb.sqlite`.
//!
//! Mixxx keeps its library in a plain (unencrypted) SQLite file. Tracks are
//! upserted into the local `tracks` table flagged `mixxx_db_source`, the same
//! way `scan_master_db` imports a rekordbox library. Mixxx waveforms/beat
//! grids use Mixxx's own formats, so imported tracks go through the app's own
//! analysis like any folder track; hot cues are imported straight into
//! `track_cues` (they only need a position, not analysis).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use uuid::Uuid;

use super::cues::{
    KEY_OPTIONS, MAX_HOT_CUES, insert_track_cues, nearest_palette_color_id, track_has_cues,
};
use super::key_notation::camelot_position;
use super::{
    BackendService, annotate_imported_playlists, build_track_match_fingerprint,
    external_playlist_key, no_importable_tracks_error, non_empty_db_value, now,
    save_imported_playlist,
};
use crate::edb::{load_table_columns, table_exists};
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{
    DetectExternalMixxxDbData, ExternalPlaylistKind, ExternalPlaylistSummary,
    ImportExternalPlaylistData, ImportExternalPlaylistRequest, ListExternalPlaylistsData,
    ListExternalPlaylistsRequest, ScanLibraryData, ScanMixxxDbRequest, TrackCue, WarningEntry,
};
use crate::scanner::is_library_audio_file;

pub(crate) const MIXXX_DB_ENV_KEY: &str = "DJUSBTKIT_MIXXX_DB_PATH";
/// `playlists.import_source` prefix of playlists imported from Mixxx.
const MIXXX_LIBRARY: &str = "mixxx";
const MIXXX_DB_FILE: &str = "mixxxdb.sqlite";

// Mixxx `cues.type` values (mixxx::CueType).
const MIXXX_CUE_HOT: i64 = 1;
const MIXXX_CUE_MAIN: i64 = 2;
const MIXXX_CUE_LOOP: i64 = 4;
// Mixxx `library.coverart_type` value for a cover image file next to the track.
const MIXXX_COVER_TYPE_FILE: i64 = 2;

pub fn detect_external_mixxx_db() -> DetectExternalMixxxDbData {
    match external_mixxx_db_candidates()
        .into_iter()
        .find(|c| c.is_file())
    {
        Some(path) => DetectExternalMixxxDbData {
            found: true,
            path: Some(path.to_string_lossy().to_string()),
        },
        None => DetectExternalMixxxDbData {
            found: false,
            path: None,
        },
    }
}

/// Where Mixxx keeps `mixxxdb.sqlite`, env override first.
pub(crate) fn external_mixxx_db_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(env_path) = std::env::var(MIXXX_DB_ENV_KEY) {
        let trimmed = env_path.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        // Linux
        candidates.push(home.join(".mixxx").join(MIXXX_DB_FILE));
        // macOS: sandboxed (App Store / signed) builds, then unsandboxed ones
        candidates.push(
            home.join("Library/Containers/org.mixxx.mixxx/Data/Library/Application Support/Mixxx")
                .join(MIXXX_DB_FILE),
        );
        candidates.push(
            home.join("Library/Application Support/Mixxx")
                .join(MIXXX_DB_FILE),
        );
    }

    // Windows
    if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local_appdata)
                .join("Mixxx")
                .join(MIXXX_DB_FILE),
        );
    }

    candidates
}

/// Classic key name for a Mixxx track. `key_id` is Mixxx's ChromaticKey
/// (1..=12 C..B major, 13..=24 Cm..Bm minor), which lines up with
/// [`KEY_OPTIONS`]. Falls back to the `key` text column when it is a key the
/// app recognises (Mixxx writes it in the user's chosen notation).
pub(crate) fn mixxx_key_name(key_id: Option<i64>, key_text: Option<&str>) -> Option<String> {
    if let Some(id) = key_id.filter(|id| (1..=24).contains(id)) {
        return Some(KEY_OPTIONS[(id - 1) as usize].to_string());
    }
    key_text
        .and_then(non_empty_db_value)
        .filter(|k| camelot_position(k).is_some())
        .map(str::to_string)
}

/// Mixxx stores cue positions as interleaved stereo samples (frames × 2).
fn mixxx_samples_to_ms(position: f64, sample_rate: u32) -> Option<u32> {
    if !position.is_finite() || position < 0.0 || sample_rate == 0 {
        return None;
    }
    let ms = (position / 2.0 / f64::from(sample_rate) * 1000.0).round();
    (ms <= f64::from(u32::MAX)).then_some(ms as u32)
}

/// One row of Mixxx's `cues` table.
#[derive(Debug, Clone)]
pub(crate) struct MixxxCue {
    pub cue_type: i64,
    pub position: f64,
    pub hotcue: i64,
    pub label: Option<String>,
    pub color: Option<i64>,
}

/// Convert a track's Mixxx cues into this app's cue list: every cue on a
/// hot-cue pad (plain hot cues and saved loops) becomes a cue point, in
/// position order, capped at [`MAX_HOT_CUES`]. Mixxx allows more pads than
/// that: the lowest-numbered pads win (pads 1-8 are the ones an 8-pad
/// controller plays). The main cue becomes the playback-start cue when it
/// lies before the first hot cue (the same rule `split_playback_start`
/// applies on export).
pub(crate) fn mixxx_cues_to_track_cues(cues: &[MixxxCue], sample_rate: u32) -> Vec<TrackCue> {
    let mut on_pads: Vec<(i64, TrackCue)> = cues
        .iter()
        .filter(|c| (c.cue_type == MIXXX_CUE_HOT || c.cue_type == MIXXX_CUE_LOOP) && c.hotcue >= 0)
        .filter_map(|c| {
            let position_ms = mixxx_samples_to_ms(c.position, sample_rate)?;
            Some((
                c.hotcue,
                TrackCue {
                    id: Uuid::now_v7().to_string(),
                    position_ms,
                    color_id: Some(
                        c.color
                            .and_then(|rgb| u32::try_from(rgb).ok())
                            .map(nearest_palette_color_id)
                            .unwrap_or(super::cues::DEFAULT_HOTCUE_COLOR_ID),
                    ),
                    name: c
                        .label
                        .as_deref()
                        .and_then(non_empty_db_value)
                        .map(str::to_string),
                    playback_start: false,
                },
            ))
        })
        .collect();
    // Lowest pads first; of two pads on the same position, the lower one.
    on_pads.sort_by_key(|(pad, c)| (*pad, c.position_ms));
    let mut positions = HashSet::new();
    let mut hot: Vec<TrackCue> = on_pads
        .into_iter()
        .map(|(_, c)| c)
        .filter(|c| positions.insert(c.position_ms))
        .take(MAX_HOT_CUES as usize)
        .collect();
    hot.sort_by_key(|c| c.position_ms);

    let start = hot.first().and_then(|first| {
        cues.iter()
            .filter(|c| c.cue_type == MIXXX_CUE_MAIN)
            .find_map(|c| mixxx_samples_to_ms(c.position, sample_rate))
            .filter(|&ms| ms < first.position_ms)
    });

    let mut out = Vec::with_capacity(hot.len() + 1);
    if let Some(position_ms) = start {
        out.push(TrackCue {
            id: Uuid::now_v7().to_string(),
            position_ms,
            color_id: None,
            name: None,
            playback_start: true,
        });
    }
    out.extend(hot);
    out
}

/// A Mixxx cover file location is relative to the track's folder, or absolute.
fn resolve_mixxx_cover_file(track_path: &str, location: &str) -> Option<PathBuf> {
    let location = Path::new(location);
    let candidate = if location.is_absolute() {
        location.to_path_buf()
    } else {
        Path::new(track_path).parent()?.join(location)
    };
    candidate.is_file().then_some(candidate)
}

/// `table.column` when the Mixxx schema has it, else `NULL` -- older Mixxx
/// versions lack some of the optional columns this import reads.
fn column_or_null(columns: &[String], table_alias: &str, column: &str) -> String {
    if columns.iter().any(|c| c == column) {
        format!("{table_alias}.{column}")
    } else {
        "NULL".to_string()
    }
}

struct MixxxTrack {
    mixxx_id: i64,
    file_path: String,
    title: String,
    artist: String,
    album: String,
    bpm: Option<f64>,
    key: Option<String>,
    duration_ms: Option<i64>,
    sample_rate: Option<u32>,
    cover_location: Option<String>,
}

fn load_mixxx_tracks(conn: &Connection) -> BackendResult<Vec<MixxxTrack>> {
    let library_cols = load_table_columns(conn, "library")?;
    let location_cols = load_table_columns(conn, "track_locations")?;
    let sql = format!(
        r#"
        SELECT l.id, tl.location, l.title, l.artist, l.album, l.bpm,
               {key_id}, l.key, l.duration, l.samplerate, {cover_type}, {cover_location}
        FROM library l
        JOIN track_locations tl ON tl.id = l.location
        WHERE IFNULL({mixxx_deleted}, 0) = 0
          AND IFNULL({fs_deleted}, 0) = 0
          AND tl.location IS NOT NULL
        "#,
        key_id = column_or_null(&library_cols, "l", "key_id"),
        cover_type = column_or_null(&library_cols, "l", "coverart_type"),
        cover_location = column_or_null(&library_cols, "l", "coverart_location"),
        mixxx_deleted = column_or_null(&library_cols, "l", "mixxx_deleted"),
        fs_deleted = column_or_null(&location_cols, "tl", "fs_deleted"),
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| BackendError::Validation(format!("Mixxx library query failed: {e}")))?;
    let tracks = stmt
        .query_map([], |row| {
            let key_id: Option<i64> = row.get(6)?;
            let key_text: Option<String> = row.get(7)?;
            let cover_type: Option<i64> = row.get(10)?;
            let cover_location: Option<String> = row.get(11)?;
            Ok(MixxxTrack {
                mixxx_id: row.get(0)?,
                file_path: row.get(1)?,
                title: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                artist: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                album: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                bpm: row.get::<_, Option<f64>>(5)?.filter(|b| *b > 0.0),
                key: mixxx_key_name(key_id, key_text.as_deref()),
                duration_ms: row
                    .get::<_, Option<f64>>(8)?
                    .filter(|d| *d > 0.0)
                    .map(|d| (d * 1000.0).round() as i64),
                sample_rate: row
                    .get::<_, Option<i64>>(9)?
                    .and_then(|sr| u32::try_from(sr).ok())
                    .filter(|sr| *sr > 0),
                cover_location: cover_location
                    .filter(|_| cover_type == Some(MIXXX_COVER_TYPE_FILE))
                    .filter(|l| !l.trim().is_empty()),
            })
        })
        .map_err(|e| BackendError::Validation(format!("Mixxx library row error: {e}")))?
        .filter_map(|r| r.ok())
        .filter(|t| !t.file_path.trim().is_empty())
        .collect();
    Ok(tracks)
}

fn load_mixxx_cues(conn: &Connection, mixxx_track_id: i64) -> BackendResult<Vec<MixxxCue>> {
    let mut stmt = conn.prepare_cached(
        "SELECT type, position, IFNULL(hotcue, -1), label, color FROM cues WHERE track_id = ?1",
    )?;
    let rows = stmt.query_map(params![mixxx_track_id], |row| {
        Ok(MixxxCue {
            cue_type: row.get::<_, Option<i64>>(0)?.unwrap_or(0),
            position: row.get::<_, Option<f64>>(1)?.unwrap_or(-1.0),
            hotcue: row.get(2)?,
            label: row.get(3)?,
            color: row.get(4)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Resolve the Mixxx library to use (explicit path, else auto-detect) and open
/// it read-only -- Mixxx may be running; its WAL is still read consistently.
fn open_mixxx_db(path: Option<&str>) -> BackendResult<Connection> {
    let mixxx_path = if let Some(p) = path.filter(|s| !s.trim().is_empty()) {
        PathBuf::from(p.trim())
    } else {
        external_mixxx_db_candidates()
            .into_iter()
            .find(|c| c.is_file())
            .ok_or_else(|| BackendError::Validation("Mixxx library not found".to_string()))?
    };
    if !mixxx_path.is_file() {
        return Err(BackendError::Validation(format!(
            "Mixxx library not found: {}",
            mixxx_path.display()
        )));
    }
    let mixxx = Connection::open_with_flags(&mixxx_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| BackendError::Validation(format!("could not open Mixxx library: {e}")))?;
    if !table_exists(&mixxx, "library") || !table_exists(&mixxx, "track_locations") {
        return Err(BackendError::Validation(
            "Mixxx library opened but library/track_locations tables not found".to_string(),
        ));
    }
    Ok(mixxx)
}

// Mixxx `Playlists.hidden` values: a regular playlist, the Auto DJ queue, and a
// history set log.
const MIXXX_PLAYLIST_REGULAR: i64 = 0;
const MIXXX_PLAYLIST_AUTO_DJ: i64 = 1;
const MIXXX_PLAYLIST_SET_LOG: i64 = 2;

/// The Mixxx playlists, crates and history set logs that have tracks, in the
/// order Mixxx's sidebar shows them.
fn list_mixxx_playlists_from(mixxx: &Connection) -> BackendResult<Vec<ExternalPlaylistSummary>> {
    let mut items = Vec::new();
    if table_exists(mixxx, "Playlists") && table_exists(mixxx, "PlaylistTracks") {
        let mut stmt = mixxx.prepare(
            "SELECT p.id, p.name, p.hidden, COUNT(pt.id)
             FROM Playlists p JOIN PlaylistTracks pt ON pt.playlist_id = p.id
             WHERE p.hidden IN (?1, ?2, ?3)
             GROUP BY p.id
             ORDER BY p.hidden = ?3, CASE WHEN p.hidden = ?3 THEN -p.id ELSE p.position END",
        )?;
        let rows = stmt.query_map(
            params![
                MIXXX_PLAYLIST_REGULAR,
                MIXXX_PLAYLIST_AUTO_DJ,
                MIXXX_PLAYLIST_SET_LOG
            ],
            |row| {
                let hidden: i64 = row.get(2)?;
                Ok(ExternalPlaylistSummary {
                    existing_playlist: None,
                    id: row.get::<_, i64>(0)?.to_string(),
                    name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    kind: if hidden == MIXXX_PLAYLIST_SET_LOG {
                        ExternalPlaylistKind::History
                    } else {
                        ExternalPlaylistKind::Playlist
                    },
                    track_count: row.get::<_, i64>(3)?.max(0) as usize,
                })
            },
        )?;
        items.extend(rows.filter_map(|r| r.ok()));
    }
    if table_exists(mixxx, "crates") && table_exists(mixxx, "crate_tracks") {
        let mut stmt = mixxx.prepare(
            "SELECT c.id, c.name, COUNT(ct.track_id)
             FROM crates c JOIN crate_tracks ct ON ct.crate_id = c.id
             GROUP BY c.id
             ORDER BY c.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ExternalPlaylistSummary {
                existing_playlist: None,
                id: row.get::<_, i64>(0)?.to_string(),
                name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                kind: ExternalPlaylistKind::Crate,
                track_count: row.get::<_, i64>(2)?.max(0) as usize,
            })
        })?;
        items.extend(rows.filter_map(|r| r.ok()));
    }
    Ok(items)
}

/// The Mixxx track ids of one playlist/set log (in play order) or crate (by
/// artist + title -- crates are unordered), plus its name.
fn load_mixxx_playlist_entries(
    mixxx: &Connection,
    kind: ExternalPlaylistKind,
    id: i64,
) -> BackendResult<(String, Vec<i64>)> {
    let (name_sql, entries_sql) = match kind {
        ExternalPlaylistKind::Playlist | ExternalPlaylistKind::History => (
            "SELECT name FROM Playlists WHERE id = ?1",
            "SELECT track_id FROM PlaylistTracks WHERE playlist_id = ?1 ORDER BY position, id",
        ),
        ExternalPlaylistKind::Crate => (
            "SELECT name FROM crates WHERE id = ?1",
            "SELECT ct.track_id FROM crate_tracks ct LEFT JOIN library l ON l.id = ct.track_id
             WHERE ct.crate_id = ?1
             ORDER BY l.artist COLLATE NOCASE, l.title COLLATE NOCASE, ct.track_id",
        ),
    };
    let name: Option<String> = mixxx
        .query_row(name_sql, params![id], |row| row.get(0))
        .optional()
        .map_err(|e| BackendError::Validation(format!("Mixxx playlist query failed: {e}")))?
        .ok_or_else(|| BackendError::NotFound(format!("Mixxx playlist not found: {id}")))?;
    let mut stmt = mixxx
        .prepare(entries_sql)
        .map_err(|e| BackendError::Validation(format!("Mixxx playlist query failed: {e}")))?;
    let ids = stmt
        .query_map(params![id], |row| row.get::<_, i64>(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok((name.unwrap_or_default(), ids))
}

/// Upserts Mixxx tracks into the local `tracks` table (plus their cover file
/// and hot cues) and tallies what happened. Shared by the whole-library import
/// and the single-playlist import so both treat a track identically.
struct MixxxTrackImporter<'a> {
    mixxx: &'a Connection,
    /// Take BPM, key and cues from Mixxx even over the app's own values
    /// (the playlist import's "force update").
    force: bool,
    has_cues_table: bool,
    existing: HashMap<String, String>,
    artwork_dir: PathBuf,
    now: String,
    indexed: usize,
    updated: usize,
    removed: usize,
    not_found: Vec<String>,
    artwork_ok: usize,
    artwork_miss: usize,
    cue_tracks: usize,
    cue_tracks_kept_local: usize,
    unsupported: usize,
    warnings: Vec<WarningEntry>,
}

impl<'a> MixxxTrackImporter<'a> {
    fn new(mixxx: &'a Connection, tx: &Connection, data_dir: &Path) -> BackendResult<Self> {
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
            mixxx,
            force: false,
            has_cues_table: table_exists(mixxx, "cues"),
            existing,
            artwork_dir,
            now: now(),
            indexed: 0,
            updated: 0,
            removed: 0,
            not_found: Vec::new(),
            artwork_ok: 0,
            artwork_miss: 0,
            cue_tracks: 0,
            cue_tracks_kept_local: 0,
            unsupported: 0,
            warnings: Vec::new(),
        })
    }

    /// Insert or update one track; `None` when it was skipped (file missing
    /// or not a supported audio file).
    fn upsert(&mut self, tx: &Connection, t: &MixxxTrack) -> BackendResult<Option<String>> {
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

        // Same rule as the folder scan: Mixxx also plays tracker modules
        // (.it/.xm/...) and videos, which the library leaves out.
        if !is_library_audio_file(Path::new(&t.file_path)) {
            self.unsupported += 1;
            return Ok(None);
        }

        let fingerprint = build_track_match_fingerprint(
            &t.title,
            &t.artist,
            Some(t.album.as_str()).filter(|s| !s.is_empty()),
        );
        let existing_id = self.existing.get(&t.file_path).cloned();
        let track_id = existing_id
            .clone()
            .unwrap_or_else(|| Uuid::now_v7().to_string());
        let artwork_path = self.copy_cover(t, &track_id);
        let album = Some(&t.album).filter(|a| !a.is_empty());
        let format_ext = crate::utils::format_ext_from_path(&t.file_path);

        if existing_id.is_some() {
            tx.execute(
                // Mixxx's BPM / key replace the track's when it has none, when
                // they still came from Mixxx (so a tempo corrected in Mixxx comes
                // across), or when forced -- never a value the user edited or the
                // app's analysis set otherwise. A value taken is marked as Mixxx's,
                // so the first analysis keeps it (see `kept_analysis_values`).
                // SET expressions see the row's old values.
                r#"UPDATE tracks SET
                    title = ?1, artist = ?2, album = ?3,
                    bpm_analyzer = CASE WHEN ?4 IS NOT NULL
                                         AND (?13 OR bpm IS NULL OR bpm_analyzer = 'mixxx')
                                        THEN 'mixxx' ELSE bpm_analyzer END,
                    bpm = CASE WHEN ?4 IS NOT NULL
                                AND (?13 OR bpm IS NULL OR bpm_analyzer = 'mixxx')
                               THEN ?4 ELSE bpm END,
                    tonality_source = CASE WHEN ?5 IS NOT NULL
                                            AND (?13 OR tonality IS NULL OR tonality_source = 'mixxx')
                                           THEN 'mixxx' ELSE tonality_source END,
                    tonality = CASE WHEN ?5 IS NOT NULL
                                     AND (?13 OR tonality IS NULL OR tonality_source = 'mixxx')
                                    THEN ?5 ELSE tonality END,
                    duration_ms = COALESCE(duration_ms, ?6),
                    sample_rate_hz = COALESCE(sample_rate_hz, ?7),
                    artwork_path = COALESCE(?8, artwork_path),
                    format_ext = COALESCE(format_ext, ?9),
                    match_fingerprint = ?10,
                    mixxx_db_source = 1,
                    updated_at = ?11
                   WHERE id = ?12"#,
                params![
                    t.title,
                    t.artist,
                    album,
                    t.bpm,
                    t.key,
                    t.duration_ms,
                    t.sample_rate,
                    artwork_path,
                    format_ext,
                    fingerprint,
                    self.now,
                    track_id,
                    self.force,
                ],
            )?;
            self.updated += 1;
        } else {
            tx.execute(
                r#"INSERT INTO tracks (
                    id, title, artist, album, bpm, tonality, file_path, format_ext,
                    duration_ms, sample_rate_hz, artwork_path, match_fingerprint,
                    mixxx_db_source, created_at, updated_at,
                    bpm_analyzer, tonality_source
                   ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,1,?13,?13,
                     CASE WHEN ?5 IS NOT NULL THEN 'mixxx' END,
                     CASE WHEN ?6 IS NOT NULL THEN 'mixxx' END)"#,
                params![
                    track_id,
                    t.title,
                    t.artist,
                    album,
                    t.bpm,
                    t.key,
                    t.file_path,
                    format_ext,
                    t.duration_ms,
                    t.sample_rate,
                    artwork_path,
                    fingerprint,
                    self.now,
                ],
            )?;
            self.existing.insert(t.file_path.clone(), track_id.clone());
            self.indexed += 1;
        }

        self.import_cues(tx, t, &track_id)?;
        Ok(Some(track_id))
    }

    fn copy_cover(&mut self, t: &MixxxTrack, track_id: &str) -> Option<String> {
        let location = t.cover_location.as_deref()?;
        let Some(src) = resolve_mixxx_cover_file(&t.file_path, location) else {
            self.artwork_miss += 1;
            self.warnings.push(logging::log(
                Level::Warn,
                "scan-mixxx-db",
                "scan.mixxx-db.artwork-not-found",
                format!("cover file not found ({location:?} for {:?})", t.file_path),
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
                    "scan-mixxx-db",
                    "scan.mixxx-db.artwork-copy-failed",
                    format!("cover copy failed {src:?} -> {dest:?}: {e}"),
                ));
                None
            }
        }
    }

    /// Hot cues: only onto a track without local cues (local edits win).
    fn import_cues(
        &mut self,
        tx: &Connection,
        t: &MixxxTrack,
        track_id: &str,
    ) -> BackendResult<()> {
        let Some(sample_rate) = t.sample_rate.filter(|_| self.has_cues_table) else {
            return Ok(());
        };
        let cues = mixxx_cues_to_track_cues(&load_mixxx_cues(self.mixxx, t.mixxx_id)?, sample_rate);
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

    /// The summary log entries, appended after the per-track warnings.
    fn finish_warnings(&mut self) -> Vec<WarningEntry> {
        let mut summary = |level, code: &str, message: String| {
            self.warnings
                .push(logging::log(level, "scan-mixxx-db", code, message));
        };
        if self.unsupported > 0 {
            summary(
                Level::Info,
                "scan.mixxx-db.unsupported-skipped",
                format!(
                    "{} track(s) skipped: not a supported audio file",
                    self.unsupported
                ),
            );
        }
        if self.cue_tracks > 0 {
            summary(
                Level::Info,
                "scan.mixxx-db.cues-imported",
                format!("hot cues imported for {} track(s)", self.cue_tracks),
            );
        }
        if self.cue_tracks_kept_local > 0 {
            summary(
                Level::Info,
                "scan.mixxx-db.cues-kept-local",
                format!(
                    "{} track(s) already had cues; their Mixxx cues were not imported",
                    self.cue_tracks_kept_local
                ),
            );
        }
        if self.artwork_ok > 0 {
            summary(
                Level::Info,
                "scan.mixxx-db.artwork-ok",
                format!("{} cover file(s) copied OK", self.artwork_ok),
            );
        }
        if self.artwork_miss > 0 {
            summary(
                Level::Warn,
                "scan.mixxx-db.artwork-miss-summary",
                format!(
                    "{} cover file(s) not found or copy failed",
                    self.artwork_miss
                ),
            );
        }
        std::mem::take(&mut self.warnings)
    }
}

impl BackendService {
    pub fn detect_external_mixxx_db(&self) -> BackendResult<DetectExternalMixxxDbData> {
        Ok(detect_external_mixxx_db())
    }

    pub fn scan_mixxx_db(&self, req: ScanMixxxDbRequest) -> BackendResult<ScanLibraryData> {
        let mixxx = open_mixxx_db(req.path.as_deref())?;
        let tracks = load_mixxx_tracks(&mixxx)?;

        let mut db_conn = self.db.connect()?;
        let tx = db_conn.transaction()?;
        let mut importer = MixxxTrackImporter::new(&mixxx, &tx, &self.db.data_dir())?;
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
            // Like the master.db import: not scoped to source folders.
            scoped_track_count: 0,
            album_count: 0,
            unanalyzed_count: 0,
            warnings,
        })
    }

    pub fn list_mixxx_playlists(
        &self,
        req: ListExternalPlaylistsRequest,
    ) -> BackendResult<ListExternalPlaylistsData> {
        let mixxx = open_mixxx_db(req.path.as_deref())?;
        let mut items = list_mixxx_playlists_from(&mixxx)?;
        let conn = self.db.connect()?;
        annotate_imported_playlists(&conn, MIXXX_LIBRARY, &mut items)?;
        Ok(ListExternalPlaylistsData { items })
    }

    /// Import one Mixxx playlist, crate or set log as a new local playlist.
    /// Its tracks are imported the same way the whole-library import does it
    /// (metadata, cover, hot cues); tracks the library leaves out (missing or
    /// unsupported files) are skipped, and a track listed twice is added once.
    pub fn import_mixxx_playlist(
        &self,
        req: ImportExternalPlaylistRequest,
    ) -> BackendResult<ImportExternalPlaylistData> {
        let id: i64 = req.id.trim().parse().map_err(|_| {
            BackendError::Validation(format!("invalid Mixxx playlist id: {:?}", req.id))
        })?;
        let mixxx = open_mixxx_db(req.path.as_deref())?;
        let (mixxx_name, entry_ids) = load_mixxx_playlist_entries(&mixxx, req.kind, id)?;
        let name = non_empty_db_value(&mixxx_name)
            .map(str::to_string)
            .unwrap_or_else(|| "Mixxx playlist".to_string());

        let wanted: HashSet<i64> = entry_ids.iter().copied().collect();
        let tracks_by_id: HashMap<i64, MixxxTrack> = load_mixxx_tracks(&mixxx)?
            .into_iter()
            .filter(|t| wanted.contains(&t.mixxx_id))
            .map(|t| (t.mixxx_id, t))
            .collect();

        let mut db_conn = self.db.connect()?;
        let tx = db_conn.transaction()?;
        let mut importer = MixxxTrackImporter::new(&mixxx, &tx, &self.db.data_dir())?;
        importer.force = req.force;

        let mut local_ids: Vec<String> = Vec::new();
        let mut seen_mixxx: HashSet<i64> = HashSet::new();
        let mut duplicates = 0usize;
        let mut deleted_in_mixxx = 0usize;
        for mixxx_id in &entry_ids {
            if !seen_mixxx.insert(*mixxx_id) {
                duplicates += 1;
                continue;
            }
            let Some(t) = tracks_by_id.get(mixxx_id) else {
                // Removed from the Mixxx library (or its file flagged deleted).
                deleted_in_mixxx += 1;
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
        let key = external_playlist_key(MIXXX_LIBRARY, req.kind, &req.id);
        let (playlist, updated_existing) = save_imported_playlist(&tx, &key, &name, &local_ids)?;
        tx.commit()?;

        let mut warnings = importer.finish_warnings();
        if duplicates > 0 {
            warnings.push(logging::log(
                Level::Info,
                "scan-mixxx-db",
                "scan.mixxx-db.playlist-duplicates-skipped",
                format!("{duplicates} repeated track(s) in {name:?} added once"),
            ));
        }
        if deleted_in_mixxx > 0 {
            warnings.push(logging::log(
                Level::Warn,
                "scan-mixxx-db",
                "scan.mixxx-db.playlist-tracks-deleted",
                format!(
                    "{deleted_in_mixxx} track(s) in {name:?} are no longer in the Mixxx library"
                ),
            ));
        }
        Ok(ImportExternalPlaylistData {
            playlist_id: playlist.id,
            name: playlist.name,
            updated_existing,
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
    use crate::models::{BrowseSourceFilesRequest, ImportedPlaylistRef};

    fn test_service() -> (tempfile::TempDir, BackendService) {
        let dir = tempfile::tempdir().expect("service data dir");
        let service = BackendService::new(dir.path()).expect("backend service");
        (dir, service)
    }

    fn cue(cue_type: i64, position: f64, hotcue: i64, color: Option<i64>) -> MixxxCue {
        MixxxCue {
            cue_type,
            position,
            hotcue,
            label: None,
            color,
        }
    }

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

    fn create_mixxx_db_fixture(
        root: &Path,
        tracks: &[FixtureTrack<'_>],
        cues: &[FixtureCue<'_>],
    ) -> PathBuf {
        let path = root.join(MIXXX_DB_FILE);
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
              mixxx_deleted INTEGER, coverart_type INTEGER, coverart_location TEXT
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
                   key_id, samplerate, mixxx_deleted, coverart_type, coverart_location)
                 VALUES (?1, 'Artist', ?2, 'Album', ?1, 185.5, 124.0, 'Am', ?3, ?4, ?5, ?6, ?7)",
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

    #[test]
    fn mixxx_key_name_maps_chromatic_key_ids_and_falls_back_to_text() {
        assert_eq!(mixxx_key_name(Some(1), None).as_deref(), Some("C"));
        assert_eq!(mixxx_key_name(Some(2), None).as_deref(), Some("C#"));
        assert_eq!(mixxx_key_name(Some(12), None).as_deref(), Some("B"));
        assert_eq!(mixxx_key_name(Some(13), None).as_deref(), Some("Cm"));
        assert_eq!(mixxx_key_name(Some(22), Some("8A")).as_deref(), Some("Am"));
        assert_eq!(mixxx_key_name(Some(0), Some(" 8A ")).as_deref(), Some("8A"));
        assert_eq!(mixxx_key_name(None, Some("not a key")), None);
        assert_eq!(mixxx_key_name(Some(25), None), None);
    }

    #[test]
    fn mixxx_samples_to_ms_uses_interleaved_stereo_positions() {
        // 1 s of stereo audio = 2 × sample-rate samples.
        assert_eq!(mixxx_samples_to_ms(88_200.0, 44_100), Some(1000));
        assert_eq!(mixxx_samples_to_ms(96_000.0, 48_000), Some(1000));
        assert_eq!(mixxx_samples_to_ms(-1.0, 44_100), None);
        assert_eq!(mixxx_samples_to_ms(1000.0, 0), None);
    }

    #[test]
    fn mixxx_cues_become_position_ordered_hot_cues_with_playback_start() {
        let cues = vec![
            cue(MIXXX_CUE_HOT, 88_200.0 * 10.0, 0, Some(0xE1_24_24)), // 10 s, red
            MixxxCue {
                label: Some("  Drop ".to_string()),
                ..cue(MIXXX_CUE_HOT, 88_200.0 * 5.0, 3, Some(0x1F_AD_C4)) // 5 s, aqua
            },
            cue(MIXXX_CUE_LOOP, 88_200.0 * 20.0, 1, None), // saved loop on a pad
            cue(MIXXX_CUE_LOOP, 88_200.0 * 30.0, -1, None), // loop not on a pad
            cue(MIXXX_CUE_MAIN, 88_200.0 * 2.0, -1, None),
            cue(MIXXX_CUE_HOT, -1.0, 4, None), // invalid position
        ];
        let out = mixxx_cues_to_track_cues(&cues, 44_100);
        let summary: Vec<_> = out
            .iter()
            .map(|c| {
                (
                    c.position_ms,
                    c.color_id,
                    c.name.as_deref(),
                    c.playback_start,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (2_000, None, None, true),
                (5_000, Some(6), Some("Drop"), false),
                (10_000, Some(2), None, false),
                (
                    20_000,
                    Some(super::super::cues::DEFAULT_HOTCUE_COLOR_ID),
                    None,
                    false
                ),
            ]
        );
    }

    #[test]
    fn mixxx_main_cue_dropped_without_hot_cues_or_after_first_hot_cue() {
        let only_main = vec![cue(MIXXX_CUE_MAIN, 88_200.0, -1, None)];
        assert!(mixxx_cues_to_track_cues(&only_main, 44_100).is_empty());

        let main_after = vec![
            cue(MIXXX_CUE_HOT, 88_200.0, 0, None),
            cue(MIXXX_CUE_MAIN, 88_200.0 * 3.0, -1, None),
        ];
        let out = mixxx_cues_to_track_cues(&main_after, 44_100);
        assert_eq!(out.len(), 1);
        assert!(!out[0].playback_start);
    }

    #[test]
    fn mixxx_lowest_pads_win_over_earlier_extra_pads() {
        // Pads 1-8 spread through the track; pads 9-12 set near the start.
        let mut cues: Vec<MixxxCue> = (0..8)
            .map(|pad| {
                cue(
                    MIXXX_CUE_HOT,
                    88_200.0 * f64::from(60 + pad * 10),
                    i64::from(pad),
                    None,
                )
            })
            .collect();
        cues.extend((8..12).map(|pad| {
            cue(
                MIXXX_CUE_HOT,
                88_200.0 * f64::from(pad),
                i64::from(pad),
                None,
            )
        }));
        let out = mixxx_cues_to_track_cues(&cues, 44_100);
        let seconds: Vec<u32> = out.iter().map(|c| c.position_ms / 1000).collect();
        assert_eq!(seconds, vec![60, 70, 80, 90, 100, 110, 120, 130]);
    }

    #[test]
    fn mixxx_cues_capped_at_max_hot_cues() {
        let cues: Vec<_> = (0..12)
            .map(|i| {
                cue(
                    MIXXX_CUE_HOT,
                    88_200.0 * f64::from(i + 1),
                    i64::from(i),
                    None,
                )
            })
            .collect();
        assert_eq!(
            mixxx_cues_to_track_cues(&cues, 44_100).len(),
            MAX_HOT_CUES as usize
        );
    }

    #[test]
    fn scan_mixxx_db_imports_tracks_cues_and_cover_and_keeps_local_cues() {
        let mixxx_root = tempfile::tempdir().expect("mixxx root");
        let media_root = tempfile::tempdir().expect("media root");
        let (_service_dir, service) = test_service();

        let new_path = media_root.path().join("new.mp3");
        let existing_path = media_root.path().join("existing.flac");
        let deleted_path = media_root.path().join("deleted-in-mixxx.mp3");
        let missing_path = media_root.path().join("missing.mp3");
        let module_path = media_root.path().join("tune.it");
        for p in [&new_path, &existing_path, &deleted_path, &module_path] {
            std::fs::write(p, b"audio").expect("write media");
        }
        std::fs::write(media_root.path().join("cover.jpg"), b"jpg").expect("write cover");

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

        let conn = service.db.connect().expect("service db");
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, bpm, match_fingerprint, created_at, updated_at)
             VALUES ('existing-track', 'Old', 'Old Artist', ?1, 128.0, 'old-fp', 'old', 'old')",
            params![existing_path.to_str().unwrap()],
        )
        .expect("seed existing track");
        conn.execute(
            "INSERT INTO track_cues (id, track_id, position_ms, color_id, name, sort_order, created_at, updated_at)
             VALUES ('local-cue', 'existing-track', 7777, 1, 'mine', 0, 'old', 'old')",
            [],
        )
        .expect("seed local cue");
        drop(conn);

        let result = service
            .scan_mixxx_db(ScanMixxxDbRequest {
                path: Some(mixxx_path.to_string_lossy().to_string()),
            })
            .expect("scan mixxx db");
        assert_eq!(result.indexed, 1);
        assert_eq!(result.updated, 1);
        assert_eq!(result.removed, 0);
        assert_eq!(
            result.not_found,
            vec![missing_path.to_string_lossy().to_string()]
        );
        let codes: Vec<_> = result.warnings.iter().map(|w| w.code.as_str()).collect();
        for code in [
            "scan.mixxx-db.cues-imported",
            "scan.mixxx-db.cues-kept-local",
            "scan.mixxx-db.artwork-ok",
            "scan.mixxx-db.unsupported-skipped",
        ] {
            assert!(codes.contains(&code), "expected {code}, got {codes:?}");
        }

        let conn = service.db.connect().expect("service db");
        let row = |sql: &str| -> (String, String, Option<String>, Option<f64>) {
            conn.query_row(sql, params![new_path.to_str().unwrap()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .expect("new track row")
        };
        let (new_id, title, key, bpm) =
            row("SELECT id, title, tonality, bpm FROM tracks WHERE file_path = ?1");
        let (duration_ms, sample_rate, artwork, flags): (Option<i64>, Option<i64>, Option<String>, (i64, i64)) = conn
            .query_row(
                "SELECT duration_ms, sample_rate_hz, artwork_path, mixxx_db_source, master_db_source
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
        let sources: (Option<String>, Option<String>) = conn
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

        let new_cues: Vec<(i64, Option<i64>, Option<String>, bool)> = conn
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
        let (bpm, key, mixxx_flag): (Option<f64>, Option<String>, i64) = conn
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
        let sources: (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT bpm_analyzer, tonality_source FROM tracks WHERE id = 'existing-track'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("existing track sources");
        assert_eq!(sources, (None, Some("mixxx".to_string())));
        let existing_cues: Vec<String> = conn
            .prepare("SELECT id FROM track_cues WHERE track_id = 'existing-track'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(existing_cues, vec!["local-cue".to_string()]);

        // Tracks deleted in Mixxx are skipped entirely.
        let deleted: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM tracks WHERE file_path = ?1",
                params![deleted_path.to_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(deleted, 0);
        drop(conn);

        // Browse: Mixxx tracks show only when the Mixxx source is included.
        let browse = |include_master_db: bool, include_mixxx_db: bool| {
            service
                .browse_source_files(BrowseSourceFilesRequest {
                    source_roots: Vec::new(),
                    include_master_db,
                    include_mixxx_db,
                    query: String::new(),
                    limit: 100,
                    ..Default::default()
                })
                .expect("browse")
                .total
        };
        assert_eq!(browse(false, true), 2);
        assert_eq!(browse(true, false), 0);
        assert_eq!(browse(false, false), 0);
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
    fn list_and_import_mixxx_playlists_crates_and_set_logs() {
        let mixxx_root = tempfile::tempdir().expect("mixxx root");
        let media_root = tempfile::tempdir().expect("media root");
        let (_service_dir, service) = test_service();

        let paths: Vec<PathBuf> = ["a.mp3", "b.mp3", "c.flac", "tune.it"]
            .iter()
            .map(|name| media_root.path().join(name))
            .collect();
        for p in &paths {
            std::fs::write(p, b"audio").expect("write media");
        }
        let missing = media_root.path().join("missing.mp3");
        let mixxx_path = create_mixxx_db_fixture(
            mixxx_root.path(),
            &[
                (
                    paths[0].to_str().unwrap(),
                    "Alpha",
                    None,
                    44_100,
                    0,
                    None,
                    0,
                ),
                (
                    paths[1].to_str().unwrap(),
                    "Bravo",
                    None,
                    44_100,
                    0,
                    None,
                    0,
                ),
                (
                    paths[2].to_str().unwrap(),
                    "Charlie",
                    None,
                    44_100,
                    0,
                    None,
                    0,
                ),
                (
                    paths[3].to_str().unwrap(),
                    "Module",
                    None,
                    44_100,
                    0,
                    None,
                    0,
                ),
                (
                    missing.to_str().unwrap(),
                    "Missing",
                    None,
                    44_100,
                    0,
                    None,
                    0,
                ),
                (
                    paths[0].to_str().unwrap(),
                    "Deleted",
                    None,
                    44_100,
                    0,
                    None,
                    1,
                ),
            ],
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

        let listed = service
            .list_mixxx_playlists(ListExternalPlaylistsRequest { path: path.clone() })
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

        let imported = service
            .import_mixxx_playlist(ImportExternalPlaylistRequest {
                path: path.clone(),
                kind: ExternalPlaylistKind::Playlist,
                id: "3".to_string(),
                force: false,
            })
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
            playlist_titles(&service, &imported.playlist_id),
            vec!["Charlie", "Alpha", "Bravo"]
        );
        let conn = service.db.connect().expect("service db");
        let (mixxx_flag, cues): (i64, i64) = conn
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
        let sources: (Option<f64>, Option<String>, Option<String>, Option<String>) = conn
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
        drop(conn);

        // Re-importing reuses the library rows; a crate imports artist/title order.
        let crate_import = service
            .import_mixxx_playlist(ImportExternalPlaylistRequest {
                path: path.clone(),
                kind: ExternalPlaylistKind::Crate,
                id: "1".to_string(),
                force: false,
            })
            .expect("import crate");
        assert_eq!(crate_import.indexed, 0);
        assert_eq!(
            playlist_titles(&service, &crate_import.playlist_id),
            vec!["Alpha", "Charlie"]
        );

        // Only a missing file: an error, and no empty playlist left behind.
        let err = service
            .import_mixxx_playlist(ImportExternalPlaylistRequest {
                path: path.clone(),
                kind: ExternalPlaylistKind::Crate,
                id: "2".to_string(),
                force: false,
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

        let err = service
            .import_mixxx_playlist(ImportExternalPlaylistRequest {
                path,
                kind: ExternalPlaylistKind::Playlist,
                id: "42".to_string(),
                force: false,
            })
            .unwrap_err();
        assert!(err.to_string().contains("not found"), "{err}");
    }

    #[test]
    fn reimporting_a_mixxx_playlist_updates_it_and_refreshes_or_forces_track_data() {
        let mixxx_root = tempfile::tempdir().expect("mixxx root");
        let media_root = tempfile::tempdir().expect("media root");
        let (_service_dir, service) = test_service();
        let paths: Vec<PathBuf> = ["a.mp3", "b.mp3"]
            .iter()
            .map(|n| media_root.path().join(n))
            .collect();
        for p in &paths {
            std::fs::write(p, b"audio").expect("write media");
        }
        let mixxx_path = create_mixxx_db_fixture(
            mixxx_root.path(),
            &[
                (
                    paths[0].to_str().unwrap(),
                    "Alpha",
                    Some(22),
                    44_100,
                    0,
                    None,
                    0,
                ),
                (
                    paths[1].to_str().unwrap(),
                    "Bravo",
                    Some(22),
                    44_100,
                    0,
                    None,
                    0,
                ),
            ],
            &[],
        );
        // Playlist 3 and crate 3: same id, different lists.
        add_mixxx_playlists(
            &mixxx_path,
            &[(3, "Warmup", 1, MIXXX_PLAYLIST_REGULAR, &[1, 2])],
            &[(3, "Warmup crate", &[2])],
        );
        let path = Some(mixxx_path.to_string_lossy().to_string());
        let import = |kind, force| {
            service
                .import_mixxx_playlist(ImportExternalPlaylistRequest {
                    path: path.clone(),
                    kind,
                    id: "3".to_string(),
                    force,
                })
                .expect("import")
        };
        let conn = || service.db.connect().expect("service db");
        let track = |title: &str| -> (f64, Option<String>, i64) {
            conn()
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
        let local = service.list_playlists().expect("list playlists").items;
        assert_eq!(local[0].imported_from.as_deref(), Some("Mixxx"));
        let listed = service
            .list_mixxx_playlists(ListExternalPlaylistsRequest { path: path.clone() })
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
        conn()
            .execute_batch(&format!(
                "UPDATE playlists SET name = 'My Warmup' WHERE id = '{}';
                 UPDATE tracks SET bpm = 130.0, bpm_analyzer = 'user' WHERE title = 'Alpha';
                 INSERT INTO track_cues (id, track_id, position_ms, sort_order, created_at, updated_at)
                   SELECT 'local', id, 7777, 0, 'x', 'x' FROM tracks WHERE title = 'Alpha';",
                first.playlist_id
            ))
            .expect("local edits");
        let mixxx = Connection::open(&mixxx_path).expect("open mixxx fixture");
        mixxx
            .execute_batch(
                "UPDATE library SET bpm = 126.0 WHERE id = 1;
                 UPDATE library SET bpm = 128.0 WHERE id = 2;
                 UPDATE PlaylistTracks SET position = 3 - position WHERE playlist_id = 3;
                 INSERT INTO cues (track_id, type, position, length, hotcue, color)
                   VALUES (1, 1, 88200, 0, 0, 0), (2, 1, 176400, 0, 0, 0);",
            )
            .expect("mixxx edits");
        drop(mixxx);

        // Plain re-import: the same playlist, its name kept, the new order;
        // Mixxx-owned values refresh, the user's edits stay.
        let again = import(ExternalPlaylistKind::Playlist, false);
        assert!(again.updated_existing);
        assert_eq!(again.playlist_id, first.playlist_id);
        assert_eq!(again.name, "My Warmup");
        assert_eq!(
            playlist_titles(&service, &again.playlist_id),
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
        let alpha_cue: i64 = conn()
            .query_row(
                "SELECT c.position_ms FROM track_cues c JOIN tracks t ON t.id = c.track_id
                 WHERE t.title = 'Alpha'",
                [],
                |r| r.get(0),
            )
            .expect("alpha cue");
        assert_eq!(alpha_cue, 1_000, "Mixxx's cue replaced the local one");
        let playlists: i64 = conn()
            .query_row("SELECT COUNT(*) FROM playlists", [], |r| r.get(0))
            .unwrap();
        assert_eq!(playlists, 2);
    }

    #[test]
    fn scan_mixxx_db_rejects_non_mixxx_database() {
        let root = tempfile::tempdir().expect("root");
        let path = root.path().join("other.sqlite");
        Connection::open(&path)
            .expect("create db")
            .execute_batch("CREATE TABLE something (id INTEGER);")
            .expect("schema");
        let (_service_dir, service) = test_service();
        let err = service
            .scan_mixxx_db(ScanMixxxDbRequest {
                path: Some(path.to_string_lossy().to_string()),
            })
            .unwrap_err();
        assert!(err.to_string().contains("library/track_locations"), "{err}");
    }
}
