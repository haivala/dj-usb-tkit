//! Mixxx library import: tracks + hot cues from `mixxxdb.sqlite`.
//!
//! Mixxx keeps its library in a plain (unencrypted) SQLite file. Tracks are
//! upserted into the local `tracks` table flagged `mixxx_db_source`, the same
//! way `scan_rekordbox_db` imports a rekordbox library. Mixxx waveforms use
//! Mixxx's own format, so imported tracks go through the app's own analysis
//! like any folder track; hot cues are imported straight into `track_cues`
//! (they only need a position, not analysis), and the first beat of Mixxx's
//! beat grid becomes the track's first beat, as rekordbox's does.

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
    external_playlist_key, has_tracks_flagged, no_importable_tracks_error, non_empty_db_value, now,
    save_imported_playlist,
};
use crate::edb::{load_table_columns, table_exists};
use crate::error::{BackendError, BackendResult};
use crate::i18n::{Text, tr};
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
            imported: false,
        },
        None => DetectExternalMixxxDbData {
            found: false,
            path: None,
            imported: false,
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

/// The first beat of a Mixxx beat grid (`library.beats`), in ms. Mixxx
/// stores it as protobuf: a constant grid (`BeatGrid-2.0`) as its BPM and
/// first beat, a variable one (`BeatMap-1.0`) as every beat. Positions are in
/// frames, not the interleaved samples of `cues`. A grid starting before the
/// track is moved forward by whole beats, which leaves the grid unchanged.
fn mixxx_first_beat_ms(version: Option<&str>, beats: &[u8], sample_rate: u32) -> Option<u32> {
    if sample_rate == 0 {
        return None;
    }
    let frame = match version? {
        "BeatGrid-2.0" => {
            let fields = proto_fields(beats)?;
            let bpm = proto_message(&fields, 1)
                .and_then(|bpm| proto_fixed64(&bpm, 1))
                .map(f64::from_bits)
                .filter(|bpm| bpm.is_finite() && *bpm > 0.0)?;
            let first = mixxx_beat_frame(&proto_message(&fields, 2)?)?;
            let beat = 60.0 * f64::from(sample_rate) / bpm;
            if first < 0.0 {
                first + (-first / beat).ceil() * beat
            } else {
                first
            }
        }
        "BeatMap-1.0" => proto_fields(beats)?
            .iter()
            .filter_map(|(field, value)| match value {
                ProtoValue::Bytes(beat) if *field == 1 => proto_fields(beat),
                _ => None,
            })
            .filter_map(|beat| mixxx_beat_frame(&beat))
            .find(|frame| *frame >= 0.0)?,
        _ => return None,
    };
    let ms = (frame / f64::from(sample_rate) * 1000.0).round();
    (ms <= f64::from(u32::MAX)).then_some(ms as u32)
}

/// A Mixxx `Beat`'s frame position; `None` for a beat switched off.
fn mixxx_beat_frame(beat: &[(u64, ProtoValue<'_>)]) -> Option<f64> {
    let enabled = proto_varint(beat, 2).is_none_or(|v| v != 0);
    // int32: a negative position is sent sign-extended to 64 bits.
    let frame = proto_varint(beat, 1).map_or(0, |v| v as i64 as i32);
    enabled.then_some(f64::from(frame))
}

enum ProtoValue<'a> {
    Varint(u64),
    Fixed64(u64),
    Bytes(&'a [u8]),
    Fixed32,
}

/// The fields of one protobuf message, in order; `None` if it is malformed.
fn proto_fields(mut buf: &[u8]) -> Option<Vec<(u64, ProtoValue<'_>)>> {
    fn varint(buf: &mut &[u8]) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let (&byte, rest) = buf.split_first()?;
            *buf = rest;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }
    let mut fields = Vec::new();
    while !buf.is_empty() {
        let key = varint(&mut buf)?;
        let value = match key & 7 {
            0 => ProtoValue::Varint(varint(&mut buf)?),
            1 => {
                let (bytes, rest) = buf.split_first_chunk::<8>()?;
                buf = rest;
                ProtoValue::Fixed64(u64::from_le_bytes(*bytes))
            }
            2 => {
                let len = usize::try_from(varint(&mut buf)?).ok()?;
                let bytes = buf.get(..len)?;
                buf = &buf[len..];
                ProtoValue::Bytes(bytes)
            }
            5 => {
                buf = buf.get(4..)?;
                ProtoValue::Fixed32
            }
            _ => return None,
        };
        fields.push((key >> 3, value));
    }
    Some(fields)
}

fn proto_varint(fields: &[(u64, ProtoValue<'_>)], number: u64) -> Option<u64> {
    fields.iter().rev().find_map(|(field, value)| match value {
        ProtoValue::Varint(v) if *field == number => Some(*v),
        _ => None,
    })
}

fn proto_fixed64(fields: &[(u64, ProtoValue<'_>)], number: u64) -> Option<u64> {
    fields.iter().rev().find_map(|(field, value)| match value {
        ProtoValue::Fixed64(v) if *field == number => Some(*v),
        _ => None,
    })
}

fn proto_message<'a>(
    fields: &[(u64, ProtoValue<'a>)],
    number: u64,
) -> Option<Vec<(u64, ProtoValue<'a>)>> {
    fields.iter().rev().find_map(|(field, value)| match value {
        ProtoValue::Bytes(bytes) if *field == number => proto_fields(bytes),
        _ => None,
    })
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
/// lies before the first hot cue (the same rule `split_cues`
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
                    memory: false,
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
            memory: false,
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
    genre: Option<String>,
    first_beat_ms: Option<u32>,
}

fn load_mixxx_tracks(conn: &Connection) -> BackendResult<Vec<MixxxTrack>> {
    let library_cols = load_table_columns(conn, "library")?;
    let location_cols = load_table_columns(conn, "track_locations")?;
    let sql = format!(
        r#"
        SELECT l.id, tl.location, l.title, l.artist, l.album, l.bpm,
               {key_id}, l.key, l.duration, l.samplerate, {cover_type}, {cover_location},
               {genre}, {beats}, {beats_version}
        FROM library l
        JOIN track_locations tl ON tl.id = l.location
        WHERE IFNULL({mixxx_deleted}, 0) = 0
          AND IFNULL({fs_deleted}, 0) = 0
          AND tl.location IS NOT NULL
        "#,
        key_id = column_or_null(&library_cols, "l", "key_id"),
        cover_type = column_or_null(&library_cols, "l", "coverart_type"),
        cover_location = column_or_null(&library_cols, "l", "coverart_location"),
        genre = column_or_null(&library_cols, "l", "genre"),
        beats = column_or_null(&library_cols, "l", "beats"),
        beats_version = column_or_null(&library_cols, "l", "beats_version"),
        mixxx_deleted = column_or_null(&library_cols, "l", "mixxx_deleted"),
        fs_deleted = column_or_null(&location_cols, "tl", "fs_deleted"),
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| BackendError::Validation(tr::library_error_mixxx_query(e)))?;
    let tracks = stmt
        .query_map([], |row| {
            let key_id: Option<i64> = row.get(6)?;
            let key_text: Option<String> = row.get(7)?;
            let cover_type: Option<i64> = row.get(10)?;
            let cover_location: Option<String> = row.get(11)?;
            let sample_rate = row
                .get::<_, Option<i64>>(9)?
                .and_then(|sr| u32::try_from(sr).ok())
                .filter(|sr| *sr > 0);
            let beats: Option<Vec<u8>> = row.get(13)?;
            let beats_version: Option<String> = row.get(14)?;
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
                sample_rate,
                cover_location: cover_location
                    .filter(|_| cover_type == Some(MIXXX_COVER_TYPE_FILE))
                    .filter(|l| !l.trim().is_empty()),
                genre: row
                    .get::<_, Option<String>>(12)?
                    .as_deref()
                    .and_then(non_empty_db_value)
                    .map(str::to_string),
                first_beat_ms: beats.zip(sample_rate).and_then(|(beats, sr)| {
                    mixxx_first_beat_ms(beats_version.as_deref(), &beats, sr)
                }),
            })
        })
        .map_err(|e| BackendError::Validation(tr::library_error_mixxx_row(e)))?
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
            .ok_or_else(|| BackendError::Validation(tr::library_error_mixxx_not_found()))?
    };
    if !mixxx_path.is_file() {
        return Err(BackendError::Validation(
            tr::library_error_mixxx_not_found_at(mixxx_path.display()),
        ));
    }
    let mixxx = Connection::open_with_flags(&mixxx_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| BackendError::Validation(tr::library_error_mixxx_open(e)))?;
    if !table_exists(&mixxx, "library") || !table_exists(&mixxx, "track_locations") {
        return Err(BackendError::Validation(tr::library_error_mixxx_tables()));
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
                    note_text: String::new(),
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
                note_text: String::new(),
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
        .map_err(|e| BackendError::Validation(tr::library_error_mixxx_playlist_query(e)))?
        .ok_or_else(|| BackendError::NotFound(tr::library_error_mixxx_playlist_not_found(id)))?;
    let mut stmt = mixxx
        .prepare(entries_sql)
        .map_err(|e| BackendError::Validation(tr::library_error_mixxx_playlist_query(e)))?;
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
        // A missing file is skipped and reported, never deleted: the drive it's
        // on may just be unplugged, and deleting the track would take its
        // cues, edits and places in every playlist with it.
        if !Path::new(&t.file_path).exists() {
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
        let format_ext = crate::audio_format::format_ext_from_path(&t.file_path);

        if existing_id.is_some() {
            tx.execute(
                // Mixxx's BPM / key / first beat replace the track's when it has
                // none, when they still came from Mixxx (so a tempo corrected in
                // Mixxx comes across), or when forced -- never a value the user
                // edited or the app's analysis set otherwise. A value taken is
                // marked as Mixxx's, so the first analysis keeps it (see
                // `kept_analysis_values`).
                // SET expressions see the row's old values.
                r#"UPDATE tracks SET
                    title = ?1, artist = ?2, album = ?3,
                    genre = CASE WHEN genre_source = 'user' THEN genre ELSE COALESCE(?14, genre) END,
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
                    first_beat_ms_source = CASE WHEN ?15 IS NOT NULL
                                                 AND (?13 OR first_beat_ms IS NULL OR first_beat_ms_source = 'mixxx')
                                                THEN 'mixxx' ELSE first_beat_ms_source END,
                    first_beat_ms = CASE WHEN ?15 IS NOT NULL
                                          AND (?13 OR first_beat_ms IS NULL OR first_beat_ms_source = 'mixxx')
                                         THEN ?15 ELSE first_beat_ms END,
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
                    t.genre,
                    t.first_beat_ms,
                ],
            )?;
            self.updated += 1;
        } else {
            tx.execute(
                r#"INSERT INTO tracks (
                    id, title, artist, album, bpm, tonality, file_path, format_ext,
                    duration_ms, sample_rate_hz, artwork_path, match_fingerprint,
                    mixxx_db_source, created_at, updated_at,
                    bpm_analyzer, tonality_source, genre,
                    first_beat_ms, first_beat_ms_source
                   ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,1,?13,?13,
                     CASE WHEN ?5 IS NOT NULL THEN 'mixxx' END,
                     CASE WHEN ?6 IS NOT NULL THEN 'mixxx' END, ?14,
                     ?15, CASE WHEN ?15 IS NOT NULL THEN 'mixxx' END)"#,
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
                    t.genre,
                    t.first_beat_ms,
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
            self.warnings.push(logging::log_text(
                Level::Warn,
                "scan-mixxx-db",
                "scan.mixxx-db.artwork-not-found",
                &tr::scan_warning_cover_not_found(location, &t.file_path),
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
                    "scan-mixxx-db",
                    "scan.mixxx-db.artwork-copy-failed",
                    &tr::scan_warning_artwork_copy_failed(src.display(), e),
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
        let mut note = |level, code: &str, text: Text| {
            self.warnings
                .push(logging::log_text(level, "scan-mixxx-db", code, &text));
        };
        // Each skipped track's file, so the status line's "(see event log)" finds them.
        for path in &self.not_found {
            note(
                Level::Warn,
                "scan.mixxx-db.file-not-found",
                tr::scan_warning_track_file_not_found(path),
            );
        }
        if self.unsupported > 0 {
            note(
                Level::Info,
                "scan.mixxx-db.unsupported-skipped",
                tr::scan_note_unsupported_skipped(self.unsupported),
            );
        }
        if self.cue_tracks > 0 {
            note(
                Level::Info,
                "scan.mixxx-db.cues-imported",
                tr::scan_note_hot_cues_imported(self.cue_tracks),
            );
        }
        if self.cue_tracks_kept_local > 0 {
            note(
                Level::Info,
                "scan.mixxx-db.cues-kept-local",
                tr::scan_note_cues_kept_local(self.cue_tracks_kept_local, "Mixxx"),
            );
        }
        if self.artwork_miss > 0 {
            note(
                Level::Warn,
                "scan.mixxx-db.artwork-miss-summary",
                tr::scan_warning_cover_missing(self.artwork_miss),
            );
        }
        if self.artwork_ok > 0 {
            logging::debug(
                "scan-mixxx-db",
                format!("cover files copied: {}", self.artwork_ok),
            );
        }
        std::mem::take(&mut self.warnings)
    }
}

impl BackendService {
    pub fn detect_external_mixxx_db(&self) -> BackendResult<DetectExternalMixxxDbData> {
        let conn = self.db.connect()?;
        Ok(DetectExternalMixxxDbData {
            imported: has_tracks_flagged(&conn, "mixxx_db_source")?,
            ..detect_external_mixxx_db()
        })
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
        let status_text = match importer.not_found.len() {
            0 => crate::i18n::tr::library_import_done_mixxx(importer.indexed, importer.updated),
            count => crate::i18n::tr::library_import_done_mixxx_not_found(
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
            // Imports never remove tracks (see `MixxxTrackImporter::upsert`).
            removed: 0,
            not_found: importer.not_found,
            warnings,
            status_text,
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
            BackendError::Internal(format!("invalid Mixxx playlist id: {:?}", req.id))
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
            warnings.push(logging::log_text(
                Level::Info,
                "scan-mixxx-db",
                "scan.mixxx-db.playlist-duplicates-skipped",
                &tr::scan_note_playlist_duplicates(duplicates, &name),
            ));
        }
        if deleted_in_mixxx > 0 {
            warnings.push(logging::log_text(
                Level::Warn,
                "scan-mixxx-db",
                "scan.mixxx-db.playlist-tracks-deleted",
                &tr::scan_warning_playlist_tracks_deleted(deleted_in_mixxx, &name, "Mixxx"),
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Protobuf for a Mixxx `Beat`.
    fn proto_beat(frame: i32, enabled: Option<bool>) -> Vec<u8> {
        let mut out = vec![0x08];
        proto_varint_bytes(i64::from(frame) as u64, &mut out);
        if let Some(enabled) = enabled {
            out.extend([0x10, u8::from(enabled)]);
        }
        out
    }

    fn proto_varint_bytes(mut v: u64, out: &mut Vec<u8>) {
        while v >= 0x80 {
            out.push((v as u8) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }

    fn proto_bytes(field: u8, bytes: &[u8], out: &mut Vec<u8>) {
        out.push(field << 3 | 2);
        proto_varint_bytes(bytes.len() as u64, out);
        out.extend_from_slice(bytes);
    }

    /// Protobuf for a Mixxx `BeatGrid` (`BeatGrid-2.0`).
    fn proto_beat_grid(bpm: f64, first_frame: i32) -> Vec<u8> {
        let mut bpm_message = vec![0x09];
        bpm_message.extend(bpm.to_bits().to_le_bytes());
        bpm_message.extend([0x10, 1]); // source, ignored
        let mut out = Vec::new();
        proto_bytes(1, &bpm_message, &mut out);
        proto_bytes(2, &proto_beat(first_frame, None), &mut out);
        out
    }

    /// Protobuf for a Mixxx `BeatMap` (`BeatMap-1.0`).
    fn proto_beat_map(beats: &[(i32, Option<bool>)]) -> Vec<u8> {
        let mut out = Vec::new();
        for &(frame, enabled) in beats {
            proto_bytes(1, &proto_beat(frame, enabled), &mut out);
        }
        out
    }

    #[test]
    fn mixxx_first_beat_reads_beat_grids_in_frames() {
        let grid = proto_beat_grid(120.0, 22_050);
        assert_eq!(
            mixxx_first_beat_ms(Some("BeatGrid-2.0"), &grid, 44_100),
            Some(500)
        );
        assert_eq!(
            mixxx_first_beat_ms(Some("BeatGrid-2.0"), &grid, 48_000),
            Some(459)
        );
        // A grid starting before the track moves forward by whole beats
        // (22 050 frames at 120 BPM).
        let early = proto_beat_grid(120.0, -11_025);
        assert_eq!(
            mixxx_first_beat_ms(Some("BeatGrid-2.0"), &early, 44_100),
            Some(250)
        );
        // A grid with no BPM can't be placed.
        let mut no_bpm = Vec::new();
        proto_bytes(2, &proto_beat(22_050, None), &mut no_bpm);
        assert_eq!(
            mixxx_first_beat_ms(Some("BeatGrid-2.0"), &no_bpm, 44_100),
            None
        );
    }

    #[test]
    fn mixxx_first_beat_reads_the_first_enabled_beat_of_a_beat_map() {
        let map = proto_beat_map(&[
            (-500, None),
            (1_000, Some(false)),
            (44_100, Some(true)),
            (66_150, None),
        ]);
        assert_eq!(
            mixxx_first_beat_ms(Some("BeatMap-1.0"), &map, 44_100),
            Some(1000)
        );
        let none_enabled = proto_beat_map(&[(1_000, Some(false))]);
        assert_eq!(
            mixxx_first_beat_ms(Some("BeatMap-1.0"), &none_enabled, 44_100),
            None
        );
    }

    #[test]
    fn mixxx_first_beat_ignores_unknown_or_broken_grids() {
        let grid = proto_beat_grid(120.0, 22_050);
        assert_eq!(mixxx_first_beat_ms(None, &grid, 44_100), None);
        assert_eq!(
            mixxx_first_beat_ms(Some("BeatGrid-1.0"), &grid, 44_100),
            None
        );
        assert_eq!(mixxx_first_beat_ms(Some("BeatGrid-2.0"), &grid, 0), None);
        assert_eq!(
            mixxx_first_beat_ms(Some("BeatGrid-2.0"), &grid[..5], 44_100),
            None
        );
        assert_eq!(
            mixxx_first_beat_ms(Some("BeatMap-1.0"), &[0xff; 3], 44_100),
            None
        );
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
    fn scan_mixxx_db_imports_the_first_beat_of_mixxx_beat_grids() {
        let mixxx_root = tempfile::tempdir().expect("mixxx root");
        let media_root = tempfile::tempdir().expect("media root");
        let (_service_dir, service) = test_service();
        let paths: Vec<PathBuf> = ["grid.mp3", "map.mp3", "none.mp3"]
            .iter()
            .map(|n| media_root.path().join(n))
            .collect();
        for p in &paths {
            std::fs::write(p, b"audio").expect("write media");
        }
        let track = |path: &Path, title| (path.to_str().unwrap().to_string(), title);
        let tracks = [
            track(&paths[0], "Grid"),
            track(&paths[1], "Map"),
            track(&paths[2], "None"),
        ];
        let fixture: Vec<FixtureTrack<'_>> = tracks
            .iter()
            .map(|(path, title)| (path.as_str(), *title, Some(22), 44_100, 0, None, 0))
            .collect();
        let mixxx_path = create_mixxx_db_fixture(mixxx_root.path(), &fixture, &[]);
        let set_grid = |id: i64, version: &str, beats: Vec<u8>| {
            Connection::open(&mixxx_path)
                .expect("open mixxx fixture")
                .execute(
                    "UPDATE library SET beats_version = ?2, beats = ?3 WHERE id = ?1",
                    params![id, version, beats],
                )
                .expect("set beats");
        };
        Connection::open(&mixxx_path)
            .expect("open mixxx fixture")
            .execute_batch(
                "ALTER TABLE library ADD COLUMN beats BLOB;
                 ALTER TABLE library ADD COLUMN beats_version TEXT;",
            )
            .expect("add beats columns");
        set_grid(1, "BeatGrid-2.0", proto_beat_grid(124.0, 22_050));
        set_grid(2, "BeatMap-1.0", proto_beat_map(&[(44_100, None)]));
        let scan = || {
            service
                .scan_mixxx_db(ScanMixxxDbRequest {
                    path: Some(mixxx_path.to_string_lossy().to_string()),
                })
                .expect("scan mixxx db");
        };
        let conn = || service.db.connect().expect("service db");
        let beat = |title: &str| -> (Option<u32>, Option<String>) {
            conn()
                .query_row(
                    "SELECT first_beat_ms, first_beat_ms_source FROM tracks WHERE title = ?1",
                    params![title],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .expect("track row")
        };

        scan();
        assert_eq!(beat("Grid"), (Some(500), Some("mixxx".to_string())));
        assert_eq!(beat("Map"), (Some(1000), Some("mixxx".to_string())));
        assert_eq!(beat("None"), (None, None));

        // The first analysis keeps it, with the Mixxx BPM it belongs to.
        let ids: Vec<String> = ["Grid", "None"]
            .iter()
            .map(|title| {
                conn()
                    .query_row("SELECT id FROM tracks WHERE title = ?1", [title], |r| {
                        r.get(0)
                    })
                    .expect("track id")
            })
            .collect();
        let analysis =
            super::super::analysis::collect_tracks_for_analysis(&conn(), &ids).expect("collect");
        assert_eq!(analysis[0].kept_first_beat_ms, Some(500));
        assert_eq!(analysis[1].kept_first_beat_ms, None);

        // A grid moved in Mixxx comes across; one moved in the app is kept.
        set_grid(1, "BeatGrid-2.0", proto_beat_grid(124.0, 33_075));
        set_grid(2, "BeatMap-1.0", proto_beat_map(&[(88_200, None)]));
        conn()
            .execute(
                "UPDATE tracks SET first_beat_ms = 900, first_beat_ms_source = 'user'
                 WHERE title = 'Map'",
                [],
            )
            .expect("move first beat");
        scan();
        assert_eq!(beat("Grid"), (Some(750), Some("mixxx".to_string())));
        assert_eq!(beat("Map"), (Some(900), Some("user".to_string())));
    }
}
