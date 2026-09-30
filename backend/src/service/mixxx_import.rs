//! Mixxx library import: tracks + hot cues from `mixxxdb.sqlite`.
//!
//! Mixxx keeps its library in a plain (unencrypted) SQLite file. Tracks are
//! upserted into the local `tracks` table flagged `mixxx_db_source`, the same
//! way `scan_master_db` imports a rekordbox library. Mixxx waveforms/beat
//! grids use Mixxx's own formats, so imported tracks go through the app's own
//! analysis like any folder track; hot cues are imported straight into
//! `track_cues` (they only need a position, not analysis).

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, params};
use uuid::Uuid;

use super::cues::{
    KEY_OPTIONS, MAX_HOT_CUES, insert_track_cues, nearest_palette_color_id, track_has_cues,
};
use super::key_notation::camelot_position;
use super::{BackendService, build_track_match_fingerprint, non_empty_db_value, now};
use crate::edb::{load_table_columns, table_exists};
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{
    DetectExternalMixxxDbData, ScanLibraryData, ScanMixxxDbRequest, TrackCue, WarningEntry,
};
use crate::scanner::is_library_audio_file;

pub(crate) const MIXXX_DB_ENV_KEY: &str = "DJUSBTKIT_MIXXX_DB_PATH";
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
/// hot-cue pad (plain hot cues and saved loops) becomes a cue point in
/// position order, capped at [`MAX_HOT_CUES`]; the main cue becomes the
/// playback-start cue when it lies before the first hot cue (the same rule
/// `split_playback_start` applies on export).
pub(crate) fn mixxx_cues_to_track_cues(cues: &[MixxxCue], sample_rate: u32) -> Vec<TrackCue> {
    let mut hot: Vec<TrackCue> = cues
        .iter()
        .filter(|c| (c.cue_type == MIXXX_CUE_HOT || c.cue_type == MIXXX_CUE_LOOP) && c.hotcue >= 0)
        .filter_map(|c| {
            let position_ms = mixxx_samples_to_ms(c.position, sample_rate)?;
            Some(TrackCue {
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
            })
        })
        .collect();
    hot.sort_by_key(|c| c.position_ms);
    hot.dedup_by_key(|c| c.position_ms);
    hot.truncate(MAX_HOT_CUES as usize);

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

impl BackendService {
    pub fn detect_external_mixxx_db(&self) -> BackendResult<DetectExternalMixxxDbData> {
        Ok(detect_external_mixxx_db())
    }

    pub fn scan_mixxx_db(&self, req: ScanMixxxDbRequest) -> BackendResult<ScanLibraryData> {
        let mixxx_path = if let Some(p) = req.path.as_deref().filter(|s| !s.trim().is_empty()) {
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

        // Read-only: Mixxx may be running (its WAL is still read consistently).
        let mixxx = Connection::open_with_flags(&mixxx_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| BackendError::Validation(format!("could not open Mixxx library: {e}")))?;
        if !table_exists(&mixxx, "library") || !table_exists(&mixxx, "track_locations") {
            return Err(BackendError::Validation(
                "Mixxx library opened but library/track_locations tables not found".to_string(),
            ));
        }
        let has_cues_table = table_exists(&mixxx, "cues");
        let tracks = load_mixxx_tracks(&mixxx)?;

        let now = now();
        let mut db_conn = self.db.connect()?;
        let tx = db_conn.transaction()?;

        let mut existing: std::collections::HashMap<String, String> = {
            let mut stmt = tx.prepare("SELECT file_path, id FROM tracks")?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect()
        };

        let artwork_dir = self.db.data_dir().join("analysis").join("artwork");
        let _ = std::fs::create_dir_all(&artwork_dir);

        let mut indexed = 0usize;
        let mut updated = 0usize;
        let mut removed = 0usize;
        let mut not_found: Vec<String> = Vec::new();
        let mut artwork_ok = 0usize;
        let mut artwork_miss = 0usize;
        let mut cue_tracks = 0usize;
        let mut cue_tracks_kept_local = 0usize;
        let mut unsupported = 0usize;
        let mut warnings = Vec::<WarningEntry>::new();

        for t in &tracks {
            if !Path::new(&t.file_path).exists() {
                if existing.remove(&t.file_path).is_some() {
                    tx.execute(
                        "DELETE FROM tracks WHERE file_path = ?1",
                        params![t.file_path],
                    )?;
                    removed += 1;
                }
                not_found.push(t.file_path.clone());
                continue;
            }

            // Same rule as the folder scan: Mixxx also plays tracker modules
            // (.it/.xm/...) and videos, which the library leaves out.
            if !is_library_audio_file(Path::new(&t.file_path)) {
                unsupported += 1;
                continue;
            }

            let fingerprint = build_track_match_fingerprint(
                &t.title,
                &t.artist,
                Some(t.album.as_str()).filter(|s| !s.is_empty()),
            );
            let existing_id = existing.get(&t.file_path).cloned();
            let track_id = existing_id
                .clone()
                .unwrap_or_else(|| Uuid::now_v7().to_string());

            let artwork_path = match t.cover_location.as_deref() {
                None => None,
                Some(location) => match resolve_mixxx_cover_file(&t.file_path, location) {
                    None => {
                        artwork_miss += 1;
                        warnings.push(logging::log(
                            Level::Warn,
                            "scan-mixxx-db",
                            "scan.mixxx-db.artwork-not-found",
                            format!("cover file not found ({location:?} for {:?})", t.file_path),
                        ));
                        None
                    }
                    Some(src) => {
                        let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
                        let dest = artwork_dir.join(format!("{track_id}.{ext}"));
                        match std::fs::copy(&src, &dest) {
                            Ok(_) => {
                                artwork_ok += 1;
                                Some(dest.to_string_lossy().to_string())
                            }
                            Err(e) => {
                                artwork_miss += 1;
                                warnings.push(logging::log(
                                    Level::Error,
                                    "scan-mixxx-db",
                                    "scan.mixxx-db.artwork-copy-failed",
                                    format!("cover copy failed {src:?} -> {dest:?}: {e}"),
                                ));
                                None
                            }
                        }
                    }
                },
            };
            let album = Some(&t.album).filter(|a| !a.is_empty());
            let format_ext = crate::utils::format_ext_from_path(&t.file_path);

            if existing_id.is_some() {
                tx.execute(
                    r#"UPDATE tracks SET
                        title = ?1, artist = ?2, album = ?3,
                        bpm = COALESCE(bpm, ?4),
                        tonality = COALESCE(tonality, ?5),
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
                        now,
                        track_id,
                    ],
                )?;
                updated += 1;
            } else {
                tx.execute(
                    r#"INSERT INTO tracks (
                        id, title, artist, album, bpm, tonality, file_path, format_ext,
                        duration_ms, sample_rate_hz, artwork_path, match_fingerprint,
                        mixxx_db_source, created_at, updated_at
                       ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,1,?13,?13)"#,
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
                        now,
                    ],
                )?;
                existing.insert(t.file_path.clone(), track_id.clone());
                indexed += 1;
            }

            // Hot cues: only onto a track without local cues (local edits win).
            let Some(sample_rate) = t.sample_rate.filter(|_| has_cues_table) else {
                continue;
            };
            let cues = mixxx_cues_to_track_cues(&load_mixxx_cues(&mixxx, t.mixxx_id)?, sample_rate);
            if cues.is_empty() {
                continue;
            }
            if track_has_cues(&tx, &track_id)? {
                cue_tracks_kept_local += 1;
            } else {
                insert_track_cues(&tx, &track_id, &cues)?;
                cue_tracks += 1;
            }
        }

        tx.commit()?;

        if unsupported > 0 {
            warnings.push(logging::log(
                Level::Info,
                "scan-mixxx-db",
                "scan.mixxx-db.unsupported-skipped",
                format!("{unsupported} track(s) skipped: not a supported audio file"),
            ));
        }
        if cue_tracks > 0 {
            warnings.push(logging::log(
                Level::Info,
                "scan-mixxx-db",
                "scan.mixxx-db.cues-imported",
                format!("hot cues imported for {cue_tracks} track(s)"),
            ));
        }
        if cue_tracks_kept_local > 0 {
            warnings.push(logging::log(
                Level::Info,
                "scan-mixxx-db",
                "scan.mixxx-db.cues-kept-local",
                format!(
                    "{cue_tracks_kept_local} track(s) already had cues; their Mixxx cues were not imported"
                ),
            ));
        }
        if artwork_ok > 0 {
            warnings.push(logging::log(
                Level::Info,
                "scan-mixxx-db",
                "scan.mixxx-db.artwork-ok",
                format!("{artwork_ok} cover file(s) copied OK"),
            ));
        }
        if artwork_miss > 0 {
            warnings.push(logging::log(
                Level::Warn,
                "scan-mixxx-db",
                "scan.mixxx-db.artwork-miss-summary",
                format!("{artwork_miss} cover file(s) not found or copy failed"),
            ));
        }

        Ok(ScanLibraryData {
            job_id: Uuid::now_v7().to_string(),
            indexed,
            updated,
            removed,
            not_found,
            // Like the master.db import: not scoped to source folders.
            scoped_track_count: 0,
            album_count: 0,
            unanalyzed_count: 0,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::BrowseSourceFilesRequest;

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
