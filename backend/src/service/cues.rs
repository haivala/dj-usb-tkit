//! Track cue points and beat-grid ("first beat") editing.
//!
//! This app targets CDJ playback directly (not Rekordbox), so a cue is just a
//! position + optional name + colour. The list is capped at 8; on save/export
//! each cue is written as BOTH a memory point and a hot-cue pad (A–H).
//!
//! Cues live in the local `track_cues` table. `get_track_detail` /
//! `save_track_analysis_edits` read and replace them; on save the cached ANLZ
//! bundle is rewritten so the local `.DAT`/`.EXT` carry the new cue list + beat
//! grid, and export/import plumb the same data onto the USB Rekordbox database
//! (see `service::anlz`, `service::export`).

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine as _;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{
    GetTrackDetailRequest, GetUsbTrackDetailRequest, ResolvePlaybackSourceRequest,
    SaveTrackAnalysisEditsData, SaveTrackAnalysisEditsRequest, SaveUsbTrackAnalysisEditsData,
    SaveUsbTrackAnalysisEditsRequest, TrackCue, TrackCueInput, TrackDetail, UsbTrackAnalysisDetail,
    WarningEntry,
};

use crate::edb::{find_content_id_by_path, find_key_id_by_name, open_edb_rw};

use super::anlz::{
    AnlzAnalysisEdits, AnlzCue, apply_analysis_edits_to_anlz, atomic_write_bytes,
    read_cues_from_anlz, read_first_beat_from_anlz,
};
use super::export_helpers::{load_table_columns_tx, write_edb_cues_for_content};
use super::usb_utils::{read_pwv5_from_anlz, resolve_usb_root, resolve_usb_side_path};
use super::{BackendService, TRACK_COLS, apply_is_usb_path, now, row_to_track};

/// Highest number of cue points a track can carry (one per CDJ hot-cue pad A–H).
pub const MAX_HOT_CUES: u8 = 8;

/// Import cue points + beat-grid anchor from an on-USB ANLZ bundle into the
/// local DB for a freshly materialised USB track — but only when the local
/// track has no cues yet (local edits always win over what's on the stick).
///
/// `dat_path` is the absolute filesystem path to the on-USB `ANLZ0000.DAT`;
/// the sibling `.EXT` (richer: colour + comment) is preferred when present.
/// The exported bundle carries each cue as a memory + hot pair, so entries are
/// deduped by position back into one `track_cues` row.
pub fn import_anlz_cues_for_track(
    tx: &rusqlite::Transaction<'_>,
    track_id: &str,
    dat_path: &Path,
) -> BackendResult<()> {
    let existing: i64 = tx.query_row(
        "SELECT COUNT(1) FROM track_cues WHERE track_id = ?1",
        params![track_id],
        |row| row.get(0),
    )?;
    if existing > 0 {
        return Ok(());
    }

    let ext_path = dat_path.with_extension("EXT");
    let bytes = std::fs::read(&ext_path)
        .or_else(|_| std::fs::read(dat_path))
        .unwrap_or_default();
    if bytes.is_empty() {
        return Ok(());
    }

    let now = now();
    for (index, cue) in collapse_anlz_cues(&bytes).into_iter().enumerate() {
        tx.execute(
            "INSERT INTO track_cues
               (id, track_id, position_ms, color_id, name, sort_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                cue.id,
                track_id,
                i64::from(cue.position_ms),
                cue.color_id.map(i64::from),
                cue.name,
                index as i64,
                now,
            ],
        )?;
    }

    // Seed the beat-grid anchor when the local row doesn't have one.
    let local_first_beat: Option<i64> = tx
        .query_row(
            "SELECT first_beat_ms FROM tracks WHERE id = ?1",
            params![track_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    if local_first_beat.is_none()
        && let Some(first_beat) = read_first_beat_from_anlz(&bytes)
    {
        tx.execute(
            "UPDATE tracks SET first_beat_ms = ?1 WHERE id = ?2 AND first_beat_ms IS NULL",
            params![i64::from(first_beat), track_id],
        )?;
    }
    Ok(())
}

/// Collapse the memory + hot-cue entries in an ANLZ bundle into one dedup-by-
/// position `TrackCue` list (capped at [`MAX_HOT_CUES`], ordered by position),
/// preferring a non-empty comment / a valid palette colour. Each returned cue
/// gets a fresh synthetic id — callers that persist it assign their own.
pub fn collapse_anlz_cues(bytes: &[u8]) -> Vec<TrackCue> {
    let mut by_position: BTreeMap<u32, (Option<u8>, Option<String>)> = BTreeMap::new();
    for cue in read_cues_from_anlz(bytes) {
        let entry = by_position.entry(cue.position_ms).or_default();
        if entry.0.is_none() && cue.color_id != 0 && is_valid_color_id(cue.color_id) {
            entry.0 = Some(cue.color_id);
        }
        if entry.1.is_none() {
            let trimmed = cue.comment.trim();
            if !trimmed.is_empty() {
                entry.1 = Some(trimmed.to_string());
            }
        }
    }
    by_position
        .into_iter()
        .take(MAX_HOT_CUES as usize)
        .map(|(position_ms, (color_id, name))| TrackCue {
            id: Uuid::now_v7().to_string(),
            position_ms,
            color_id,
            name,
        })
        .collect()
}

/// Read/rewrite an ANLZ `.DAT` bundle and its sibling `.EXT` in place with
/// `edits` applied (`.EXT` only when it exists). Shared by the local
/// analysis-cache regenerator and the USB-native save.
fn rewrite_anlz_bundle_files(dat_path: &Path, edits: &AnlzAnalysisEdits<'_>) -> BackendResult<()> {
    let ext_path = dat_path.with_extension("EXT");
    let dat = std::fs::read(dat_path)?;
    atomic_write_bytes(dat_path, &apply_analysis_edits_to_anlz(&dat, edits))?;
    if ext_path.is_file() {
        let ext = std::fs::read(&ext_path)?;
        atomic_write_bytes(&ext_path, &apply_analysis_edits_to_anlz(&ext, edits))?;
    }
    Ok(())
}

/// A hot-cue colour: the palette index stored in `track_cues.color_id` plus the
/// RGB and Rekordbox "colour code" derived from it for the ANLZ `PCP2` entry and
/// the eDB `cue.colorTableIndex`.
#[derive(Debug, Clone, Copy)]
pub struct HotcuePaletteEntry {
    pub id: u8,
    pub rgb: (u8, u8, u8),
    pub color_code: u8,
}

/// The colours the track-detail modal offers for cues.
///
/// TODO(cue-palette): the exact index↔RGB↔code mapping is a hardware-verification
/// item (`docs/CDJ_TEST_MATRIX.md`) — a wrong index only mis-tints the pad.
pub const HOTCUE_PALETTE: &[HotcuePaletteEntry] = &[
    HotcuePaletteEntry { id: 1, rgb: (0xDE, 0x44, 0xCF), color_code: 1 }, // pink
    HotcuePaletteEntry { id: 2, rgb: (0xE1, 0x24, 0x24), color_code: 2 }, // red
    HotcuePaletteEntry { id: 3, rgb: (0xE9, 0x7A, 0x1E), color_code: 3 }, // orange
    HotcuePaletteEntry { id: 4, rgb: (0xE3, 0xC7, 0x1B), color_code: 4 }, // yellow
    HotcuePaletteEntry { id: 5, rgb: (0x4E, 0xB6, 0x48), color_code: 5 }, // green
    HotcuePaletteEntry { id: 6, rgb: (0x1F, 0xAD, 0xC4), color_code: 6 }, // aqua
    HotcuePaletteEntry { id: 7, rgb: (0x2A, 0x5B, 0xD8), color_code: 7 }, // blue
    HotcuePaletteEntry { id: 8, rgb: (0x8A, 0x3F, 0xD1), color_code: 8 }, // purple
];

/// Default cue colour index applied when the UI omits one.
pub const DEFAULT_HOTCUE_COLOR_ID: u8 = 5;

pub fn palette_entry(color_id: u8) -> Option<HotcuePaletteEntry> {
    HOTCUE_PALETTE.iter().copied().find(|e| e.id == color_id)
}

pub fn is_valid_color_id(color_id: u8) -> bool {
    palette_entry(color_id).is_some()
}

fn row_to_track_cue(row: &rusqlite::Row<'_>, base: usize) -> rusqlite::Result<TrackCue> {
    Ok(TrackCue {
        id: row.get(base)?,
        position_ms: row.get::<_, i64>(base + 1)?.max(0) as u32,
        color_id: row.get::<_, Option<i64>>(base + 2)?.map(|v| v as u8),
        name: row.get(base + 3)?,
    })
}

/// Read a track's cue list in stable render order.
pub fn load_track_cues(conn: &Connection, track_id: &str) -> BackendResult<Vec<TrackCue>> {
    let mut stmt = conn.prepare(
        "SELECT id, position_ms, color_id, name
           FROM track_cues WHERE track_id = ?1
          ORDER BY sort_order, position_ms",
    )?;
    let rows = stmt.query_map(params![track_id], |row| row_to_track_cue(row, 0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Batch-load cue lists for many tracks at once (one query), keyed by track id.
/// Tracks with no cues are absent from the map.
pub fn load_track_cues_bulk(
    conn: &Connection,
    track_ids: &[String],
) -> BackendResult<std::collections::HashMap<String, Vec<TrackCue>>> {
    let mut out: std::collections::HashMap<String, Vec<TrackCue>> =
        std::collections::HashMap::new();
    if track_ids.is_empty() {
        return Ok(out);
    }
    let placeholders = vec!["?"; track_ids.len()].join(", ");
    let sql = format!(
        "SELECT track_id, id, position_ms, color_id, name
           FROM track_cues WHERE track_id IN ({placeholders})
          ORDER BY track_id, sort_order, position_ms"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(track_ids.iter()), |row| {
        Ok((row.get::<_, String>(0)?, row_to_track_cue(row, 1)?))
    })?;
    for row in rows {
        let (track_id, cue) = row?;
        out.entry(track_id).or_default().push(cue);
    }
    Ok(out)
}

/// Cue list for a track as ANLZ encoder inputs.
pub fn anlz_cues_for_track(conn: &Connection, track_id: &str) -> BackendResult<Vec<AnlzCue>> {
    Ok(anlz_cues_from_track_cues(&load_track_cues(conn, track_id)?))
}

/// Expand each cue point into a memory `AnlzCue` **and** a hot `AnlzCue`
/// (slot 1..=8 by position order). The `PCOB`/`PCO2` encoders split on `is_hot()`.
pub fn anlz_cues_from_track_cues(cues: &[TrackCue]) -> Vec<AnlzCue> {
    let mut sorted: Vec<&TrackCue> = cues.iter().collect();
    sorted.sort_by_key(|c| c.position_ms);

    let mut out = Vec::with_capacity(sorted.len() * 2);
    for (i, cue) in sorted.iter().take(MAX_HOT_CUES as usize).enumerate() {
        let (color_id, rgb, code) = match cue.color_id.and_then(palette_entry) {
            Some(entry) => (entry.id, entry.rgb, entry.color_code),
            None => (0, (0, 0, 0), 0),
        };
        let comment = cue.name.clone().unwrap_or_default();
        let make = |hot_cue: u32| AnlzCue {
            position_ms: cue.position_ms,
            hot_cue,
            color_id,
            color_rgb: rgb,
            color_code: code,
            comment: comment.clone(),
        };
        out.push(make(0)); // memory point
        out.push(make((i + 1) as u32)); // hot-cue pad A..H
    }
    out
}

/// The musical keys the track-detail modal's key stepper offers, in standard
/// notation exactly as `stratum-dsp`'s `Key::name()` and the essentia runner
/// emit it (sharp-only, e.g. "C#" never "Db"): 12 majors then 12 minors.
/// Mirrored in `vanilla-ui/components/track-detail/actions.mjs` as
/// `KEY_OPTIONS`.
pub const KEY_OPTIONS: &[&str] = &[
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B", "Cm", "C#m", "Dm", "D#m",
    "Em", "Fm", "F#m", "Gm", "G#m", "Am", "A#m", "Bm",
];

/// Trim and validate a user-entered musical key against `KEY_OPTIONS`. Only
/// gates new *user* edits through the save endpoints — a track's stored key
/// (from essentia, which can emit flat spellings, or a legacy value) is never
/// re-validated by this.
fn normalize_key_input(key: &str) -> BackendResult<String> {
    let trimmed = key.trim();
    if !KEY_OPTIONS.contains(&trimmed) {
        return Err(BackendError::Validation(format!("unknown key: {trimmed}")));
    }
    Ok(trimmed.to_string())
}

/// A validated, normalised cue point ready to be persisted.
#[derive(Debug)]
struct NormalizedCue {
    position_ms: u32,
    color_id: Option<u8>,
    name: Option<String>,
}

fn normalize_cues(
    inputs: &[TrackCueInput],
    duration_ms: Option<u64>,
) -> BackendResult<Vec<NormalizedCue>> {
    if inputs.len() > MAX_HOT_CUES as usize {
        return Err(BackendError::Validation(format!(
            "at most {MAX_HOT_CUES} cue points are allowed"
        )));
    }
    let max_pos = duration_ms
        .filter(|d| *d > 0)
        .map(|d| (d - 1) as u32)
        .unwrap_or(u32::MAX);

    let mut out = Vec::with_capacity(inputs.len());
    for input in inputs {
        let color_id = match input.color_id {
            Some(id) if is_valid_color_id(id) => Some(id),
            Some(id) => {
                return Err(BackendError::Validation(format!("unknown cue colorId {id}")));
            }
            None => Some(DEFAULT_HOTCUE_COLOR_ID),
        };
        out.push(NormalizedCue {
            position_ms: input.position_ms.min(max_pos),
            color_id,
            name: input
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        });
    }
    Ok(out)
}

impl NormalizedCue {
    fn to_track_cue(&self) -> TrackCue {
        TrackCue {
            id: Uuid::now_v7().to_string(),
            position_ms: self.position_ms,
            color_id: self.color_id,
            name: self.name.clone(),
        }
    }
}

fn normalized_to_track_cues(cues: &[NormalizedCue]) -> Vec<TrackCue> {
    cues.iter().map(NormalizedCue::to_track_cue).collect()
}

/// Apply a first-beat / cue-list / bpm / key edit to the local master in
/// `tx`: write `tracks.first_beat_ms` and/or `tracks.bpm` and/or
/// `tracks.tonality` and/or replace `track_cues`, and — when any of these
/// changed — bump `tracks.updated_at` and reset the export markers of every
/// app playlist containing the track (a stale on-USB bundle is refreshed only
/// by a re-export). Shared by the local save and the USB-native save.
fn apply_local_analysis_edits_tx(
    tx: &rusqlite::Transaction<'_>,
    track_id: &str,
    first_beat_ms: Option<u32>,
    cues: Option<&[NormalizedCue]>,
    bpm: Option<f64>,
    key: Option<&str>,
    now: &str,
) -> BackendResult<()> {
    if let Some(first_beat_ms) = first_beat_ms {
        tx.execute(
            "UPDATE tracks SET first_beat_ms = ?1, first_beat_ms_source = 'user', updated_at = ?2 WHERE id = ?3",
            params![i64::from(first_beat_ms), now, track_id],
        )?;
    }

    if let Some(bpm) = bpm {
        tx.execute(
            "UPDATE tracks SET bpm = ?1, bpm_analyzer = 'user', updated_at = ?2 WHERE id = ?3",
            params![bpm, now, track_id],
        )?;
    }

    if let Some(key) = key {
        tx.execute(
            "UPDATE tracks SET tonality = ?1, tonality_source = 'user', updated_at = ?2 WHERE id = ?3",
            params![key, now, track_id],
        )?;
    }

    if let Some(cues) = cues {
        tx.execute("DELETE FROM track_cues WHERE track_id = ?1", params![track_id])?;
        for (index, cue) in cues.iter().enumerate() {
            tx.execute(
                "INSERT INTO track_cues
                   (id, track_id, position_ms, color_id, name, sort_order, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![
                    Uuid::now_v7().to_string(),
                    track_id,
                    i64::from(cue.position_ms),
                    cue.color_id.map(i64::from),
                    cue.name,
                    index as i64,
                    now,
                ],
            )?;
        }
    }

    if first_beat_ms.is_some() || cues.is_some() || bpm.is_some() || key.is_some() {
        tx.execute(
            "UPDATE tracks SET updated_at = ?1 WHERE id = ?2",
            params![now, track_id],
        )?;
        tx.execute(
            "UPDATE playlists
                SET updated_at = ?1,
                    last_exported_at = NULL,
                    last_exported_usb_root = NULL,
                    last_exported_track_count = NULL
              WHERE id IN (SELECT playlist_id FROM playlist_tracks WHERE track_id = ?2)",
            params![now, track_id],
        )?;
    }
    Ok(())
}

impl BackendService {
    /// Import a USB track's ANLZ cue points / beat-grid anchor into an
    /// already-existing local `tracks` row, if not already imported. Called
    /// from the add-to-playlist path for candidates that resolved to a real
    /// row without going through `materialize_usb_add_candidate` (e.g. a
    /// page-materialized row, or a genuine local copy). Cheap when already
    /// imported: one `COUNT(1)` and no file read.
    pub(crate) fn ensure_usb_analysis_imported(
        &self,
        track_id: &str,
        anlz_abs_path: &str,
    ) -> BackendResult<()> {
        let anlz_abs_path = anlz_abs_path.trim();
        if track_id.trim().is_empty() || anlz_abs_path.is_empty() {
            return Ok(());
        }
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        import_anlz_cues_for_track(&tx, track_id, std::path::Path::new(anlz_abs_path))?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_track_detail(&self, req: GetTrackDetailRequest) -> BackendResult<TrackDetail> {
        let track_id = req.track_id.trim();
        if track_id.is_empty() {
            return Err(BackendError::Validation("trackId is required".to_string()));
        }
        let conn = self.db.connect()?;

        let mut track = conn
            .prepare(&format!("SELECT {TRACK_COLS} FROM tracks WHERE id = ?1"))?
            .query_row(params![track_id], |row| row_to_track(row, true))
            .optional()?
            .ok_or_else(|| BackendError::NotFound(format!("track not found: {track_id}")))?;
        apply_is_usb_path(&conn, std::slice::from_mut(&mut track))?;

        let first_beat_ms: Option<u32> = conn
            .query_row(
                "SELECT first_beat_ms FROM tracks WHERE id = ?1",
                params![track_id],
                |row| row.get::<_, Option<i64>>(0),
            )?
            .map(|v| v.max(0) as u32);

        let detail_waveform = track
            .waveform_peaks_path
            .as_deref()
            .and_then(read_pwv5_from_anlz)
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes));

        let cues = load_track_cues(&conn, track_id)?;

        Ok(TrackDetail {
            track,
            first_beat_ms,
            cues,
            detail_waveform,
        })
    }

    pub fn save_track_analysis_edits(
        &self,
        req: SaveTrackAnalysisEditsRequest,
    ) -> BackendResult<SaveTrackAnalysisEditsData> {
        let track_id = req.track_id.trim().to_string();
        if track_id.is_empty() {
            return Err(BackendError::Validation("trackId is required".to_string()));
        }

        let mut conn = self.db.connect()?;

        let duration_ms: Option<u64> = conn
            .query_row(
                "SELECT duration_ms FROM tracks WHERE id = ?1",
                params![track_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .ok_or_else(|| BackendError::NotFound(format!("track not found: {track_id}")))?
            .map(|v| v.max(0) as u64);

        if let Some(first_beat_ms) = req.first_beat_ms
            && let Some(dur) = duration_ms.filter(|d| *d > 0)
            && u64::from(first_beat_ms) >= dur
        {
            return Err(BackendError::Validation(
                "firstBeatMs must be less than the track duration".to_string(),
            ));
        }

        if let Some(bpm) = req.bpm
            && !(bpm > 0.0 && bpm <= 999.0)
        {
            return Err(BackendError::Validation(
                "bpm must be greater than 0 and at most 999".to_string(),
            ));
        }

        let key = req.key.as_deref().map(normalize_key_input).transpose()?;

        let normalized = match req.cues.as_deref() {
            Some(inputs) => Some(normalize_cues(inputs, duration_ms)?),
            None => None,
        };

        let now = now();
        let tx = conn.transaction()?;
        apply_local_analysis_edits_tx(
            &tx,
            &track_id,
            req.first_beat_ms,
            normalized.as_deref(),
            req.bpm,
            key.as_deref(),
            &now,
        )?;
        tx.commit()?;

        // Bake the edits into the cached local ANLZ bundle so the local
        // `.DAT`/`.EXT` already carry them (export then only injects the
        // USB-relative PPTH). A filesystem failure here is logged, not fatal:
        // the edits are persisted and get re-applied at the next
        // analysis/export.
        let anlz_regenerated = match self.regenerate_cached_anlz_analysis_edits(&track_id) {
            Ok(regenerated) => regenerated,
            Err(err) => {
                logging::emit(
                    Level::Warn,
                    "cues.anlz-regenerate-failed",
                    &format!("could not rewrite cached ANLZ for {track_id}: {err}"),
                );
                false
            }
        };

        let conn = self.db.connect()?;
        let cues = load_track_cues(&conn, &track_id)?;
        let (first_beat_ms, bpm, bpm_analyzer, key, key_source) = conn.query_row(
            "SELECT first_beat_ms, bpm, bpm_analyzer, tonality, tonality_source FROM tracks WHERE id = ?1",
            params![track_id],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?.map(|v| v.max(0) as u32),
                    row.get::<_, Option<f64>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )?;

        Ok(SaveTrackAnalysisEditsData {
            track_id,
            first_beat_ms,
            cues,
            bpm,
            bpm_analyzer,
            key,
            key_source,
            anlz_regenerated,
        })
    }

    /// Read cue points + beat-grid anchor + colour waveform straight off an
    /// on-USB ANLZ bundle, for the cue editor opened from a USB playlist /
    /// history row. No local `tracks` row need exist.
    pub fn get_usb_track_detail(
        &self,
        req: GetUsbTrackDetailRequest,
    ) -> BackendResult<UsbTrackAnalysisDetail> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let raw = req.usb_analysis_path_raw.trim();
        if raw.is_empty() {
            return Err(BackendError::Validation(
                "usbAnalysisPathRaw is required".to_string(),
            ));
        }
        let dat_abs = resolve_usb_side_path(&usb_root, raw)
            .ok_or_else(|| BackendError::NotFound(format!("USB analysis path not found: {raw}")))?;
        let dat_path = Path::new(&dat_abs);
        let bytes = std::fs::read(dat_path.with_extension("EXT"))
            .or_else(|_| std::fs::read(dat_path))
            .map_err(|_| {
                BackendError::NotFound(format!("USB analysis bundle not readable: {raw}"))
            })?;
        if bytes.is_empty() {
            return Err(BackendError::NotFound(format!(
                "USB analysis bundle is empty: {raw}"
            )));
        }

        Ok(UsbTrackAnalysisDetail {
            first_beat_ms: read_first_beat_from_anlz(&bytes),
            cues: collapse_anlz_cues(&bytes),
            detail_waveform: read_pwv5_from_anlz(&dat_abs)
                .map(|b| base64::engine::general_purpose::STANDARD.encode(b)),
        })
    }

    /// Save a cue / beat-grid edit made from a USB view: write the on-device
    /// ANLZ + eDB **in place** (so a CDJ sees it without a re-export) and also
    /// write the resolved local master (so the two never diverge and export
    /// never has to merge two edit sets). The USB must be connected — a
    /// not-connected / missing-bundle / not-in-eDB state blocks the save with
    /// an error rather than a silent local-only downgrade.
    pub fn save_usb_track_analysis_edits(
        &self,
        req: SaveUsbTrackAnalysisEditsRequest,
    ) -> BackendResult<SaveUsbTrackAnalysisEditsData> {
        // 1. USB connected? A failure here is the "block the save" contract.
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;

        let raw = req.usb_analysis_path_raw.trim();
        if raw.is_empty() {
            return Err(BackendError::Validation(
                "usbAnalysisPathRaw is required".to_string(),
            ));
        }
        // 2. Absolute ANLZ path, must exist.
        let dat_abs = resolve_usb_side_path(&usb_root, raw)
            .ok_or_else(|| BackendError::NotFound(format!("USB analysis path not found: {raw}")))?;
        let dat_path = Path::new(&dat_abs);
        if !dat_path.is_file() {
            return Err(BackendError::NotFound(format!(
                "USB analysis bundle missing: {raw}"
            )));
        }

        // 3. Validate + normalise the incoming edits.
        if let Some(first_beat_ms) = req.first_beat_ms
            && let Some(dur) = req.duration_ms.filter(|d| *d > 0)
            && u64::from(first_beat_ms) >= dur
        {
            return Err(BackendError::Validation(
                "firstBeatMs must be less than the track duration".to_string(),
            ));
        }
        if let Some(bpm) = req.bpm
            && !(bpm > 0.0 && bpm <= 999.0)
        {
            return Err(BackendError::Validation(
                "bpm must be greater than 0 and at most 999".to_string(),
            ));
        }
        let key = req.key.as_deref().map(normalize_key_input).transpose()?;
        let normalized = match req.cues.as_deref() {
            Some(inputs) => Some(normalize_cues(inputs, req.duration_ms)?),
            None => None,
        };

        // 4. Open the device eDB and locate the track *before* writing
        //    anything, so a missing content row blocks cleanly.
        let mut warnings: Vec<WarningEntry> = Vec::new();
        let mut edb_conn = open_edb_rw(&usb_root, &mut warnings).ok_or_else(|| {
            BackendError::NotFound(
                "USB library database (exportLibrary.db) not found or unreadable".to_string(),
            )
        })?;
        let content_id = find_content_id_by_path(&edb_conn, &req.usb_media_path_raw)?
            .ok_or_else(|| {
                BackendError::NotFound(format!(
                    "track not in this USB's library database: {}",
                    req.usb_media_path_raw
                ))
            })?;

        // The cue list to reconcile onto the device: the new list when this is
        // a cue edit (empty list clears), otherwise the bundle's current list
        // (a first-beat-only edit must not drop existing cues).
        let effective_cues: Vec<TrackCue> = match &normalized {
            Some(n) => normalized_to_track_cues(n),
            None => collapse_anlz_cues(
                &std::fs::read(dat_path.with_extension("EXT"))
                    .or_else(|_| std::fs::read(dat_path))
                    .unwrap_or_default(),
            ),
        };

        // 5. ANLZ write, in place on the device.
        let anlz_cues = anlz_cues_from_track_cues(&effective_cues);
        rewrite_anlz_bundle_files(
            dat_path,
            &AnlzAnalysisEdits {
                bpm: req.bpm,
                duration_ms: req.duration_ms,
                first_beat_ms: req.first_beat_ms,
                cues: Some(&anlz_cues),
            },
        )?;

        // 6. Local master write (mandatory when a match is found) — before the
        //    eDB commit so a local failure aborts before the device diverges.
        let local_updated = match self.resolve_local_track_id_for_usb(&usb_root, &req) {
            Some(track_id) => {
                let mut conn = self.db.connect()?;
                let now = now();
                let tx = conn.transaction()?;
                apply_local_analysis_edits_tx(
                    &tx,
                    &track_id,
                    req.first_beat_ms,
                    normalized.as_deref(),
                    req.bpm,
                    key.as_deref(),
                    &now,
                )?;
                tx.commit()?;
                if let Err(err) = self.regenerate_cached_anlz_analysis_edits(&track_id) {
                    logging::emit(
                        Level::Warn,
                        "cues.usb-save.local-anlz-regenerate-failed",
                        &format!("could not rewrite cached ANLZ for {track_id}: {err}"),
                    );
                }
                true
            }
            None => {
                logging::emit(
                    Level::Warn,
                    "cues.usb-save.no-local-match",
                    &format!(
                        "USB cue edit for {} matched no local track; device written, local master not",
                        req.usb_media_path_raw
                    ),
                );
                false
            }
        };

        // 7. eDB write + write-back to the device.
        let tx = edb_conn.transaction()?;
        let content_columns = load_table_columns_tx(&tx, "content")?;
        write_edb_cues_for_content(&tx, content_id, &effective_cues, &content_columns)?;
        if let Some(bpm) = req.bpm
            && content_columns.contains("bpmx100")
        {
            let bpmx100 = (bpm * 100.0).round() as i64;
            tx.execute(
                "UPDATE content SET bpmx100 = ?1 WHERE content_id = ?2",
                params![bpmx100, content_id],
            )?;
        }
        if let Some(key) = key.as_deref()
            && content_columns.contains("key_id")
        {
            let key_id = find_key_id_by_name(&tx, Some(key))?;
            tx.execute(
                "UPDATE content SET key_id = ?1 WHERE content_id = ?2",
                params![key_id, content_id],
            )?;
        }
        tx.commit()?;
        drop(edb_conn);
        super::usb_staging::write_back_if_changed(&usb_root, super::usb_staging::DbKind::Edb)?;

        self.invalidate_usb_parse_cache();

        // 8. Re-read the device bundle for the response.
        let bytes = std::fs::read(dat_path.with_extension("EXT"))
            .or_else(|_| std::fs::read(dat_path))
            .unwrap_or_default();
        Ok(SaveUsbTrackAnalysisEditsData {
            first_beat_ms: read_first_beat_from_anlz(&bytes),
            cues: collapse_anlz_cues(&bytes),
            bpm: req.bpm,
            bpm_analyzer: req.bpm.map(|_| "user".to_string()),
            key: key.clone(),
            key_source: key.map(|_| "user".to_string()),
            anlz_updated: true,
            edb_updated: true,
            local_updated,
        })
    }

    /// Resolve the local `tracks` id a USB row maps to, in priority order:
    /// frontend hint → `track_usb_links` → `resolve_playback_source`
    /// (fingerprint/title). Only the USB→local direction is ever needed.
    fn resolve_local_track_id_for_usb(
        &self,
        usb_root: &Path,
        req: &SaveUsbTrackAnalysisEditsRequest,
    ) -> Option<String> {
        let conn = self.db.connect().ok()?;

        // 1. Frontend hint — accept only when the row still exists.
        if let Some(hint) = req
            .local_track_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let existing: Option<String> = conn
                .query_row("SELECT id FROM tracks WHERE id = ?1", params![hint], |r| {
                    r.get(0)
                })
                .optional()
                .ok()?;
            if existing.is_some() {
                return existing;
            }
        }

        // 2. Authoritative link row for this device + media path.
        let media_raw = req.usb_media_path_raw.trim();
        if !media_raw.is_empty() {
            let root_key =
                super::normalize_source_root_for_matching(&usb_root.to_string_lossy());
            let device_id: Option<String> = conn
                .query_row(
                    "SELECT id FROM usb_devices WHERE root_path_key = ?1",
                    params![root_key],
                    |r| r.get(0),
                )
                .optional()
                .ok()?;
            if let Some(device_id) = device_id {
                let resolved = resolve_usb_side_path(usb_root, media_raw);
                let link: Option<String> = conn
                    .query_row(
                        "SELECT track_id FROM track_usb_links
                           WHERE usb_device_id = ?1 AND usb_file_path IN (?2, ?3) LIMIT 1",
                        params![device_id, media_raw, resolved],
                        |r| r.get(0),
                    )
                    .optional()
                    .ok()?;
                if link.is_some() {
                    return link;
                }
            }
        }

        // 3. Fingerprint / title fallback.
        let title = req.title.as_deref().unwrap_or_default().trim().to_string();
        let artist = req.artist.as_deref().unwrap_or_default().trim().to_string();
        if title.is_empty() && artist.is_empty() {
            return None;
        }
        let resolved = self
            .resolve_playback_source(ResolvePlaybackSourceRequest {
                title,
                artist,
                album: req.album.clone(),
                bpm: req.bpm,
                file_path: Some(req.usb_media_path_raw.clone()),
                file_size_bytes: None,
                track_id: None,
            })
            .ok()?;
        matches!(resolved.matched_by.as_str(), "self" | "hash" | "metadata")
            .then_some(resolved.track_id)
            .flatten()
    }

    /// Rewrite the cached local ANLZ bundle (`.DAT`/`.EXT`) for a track so it
    /// carries the current `track_cues` + stored first beat.
    ///
    /// Returns `false` when the track has no analysis cache yet (nothing to
    /// rewrite).
    fn regenerate_cached_anlz_analysis_edits(&self, track_id: &str) -> BackendResult<bool> {
        let conn = self.db.connect()?;
        let row = conn
            .query_row(
                "SELECT waveform_peaks_path, bpm, duration_ms, first_beat_ms
                   FROM tracks WHERE id = ?1",
                params![track_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<f64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .optional()?;

        let Some((Some(dat_path), bpm, duration_ms, first_beat_ms)) = row else {
            return Ok(false);
        };
        let dat_path = Path::new(&dat_path);
        if !dat_path.is_file() {
            return Ok(false);
        }

        let cues = anlz_cues_for_track(&conn, track_id)?;
        let edits = AnlzAnalysisEdits {
            bpm,
            duration_ms: duration_ms.map(|v| v.max(0) as u64),
            first_beat_ms: first_beat_ms.map(|v| v.max(0) as u32),
            cues: Some(&cues),
        };

        rewrite_anlz_bundle_files(dat_path, &edits)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(pos: u32, color: Option<u8>) -> TrackCueInput {
        TrackCueInput {
            position_ms: pos,
            color_id: color,
            name: None,
        }
    }

    fn named_input(pos: u32, color: Option<u8>, name: Option<&str>) -> TrackCueInput {
        TrackCueInput {
            position_ms: pos,
            color_id: color,
            name: name.map(str::to_string),
        }
    }

    fn cue(id: &str, pos: u32, color: Option<u8>) -> TrackCue {
        TrackCue {
            id: id.to_string(),
            position_ms: pos,
            color_id: color,
            name: None,
        }
    }

    fn cue_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE tracks (
              id TEXT PRIMARY KEY,
              first_beat_ms INTEGER,
              first_beat_ms_source TEXT,
              bpm REAL,
              bpm_analyzer TEXT,
              tonality TEXT,
              tonality_source TEXT,
              updated_at TEXT
            );
            CREATE TABLE track_cues (
              id TEXT PRIMARY KEY,
              track_id TEXT NOT NULL,
              position_ms INTEGER NOT NULL,
              color_id INTEGER,
              name TEXT,
              sort_order INTEGER NOT NULL DEFAULT 0,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            CREATE TABLE playlists (
              id TEXT PRIMARY KEY,
              updated_at TEXT NOT NULL,
              last_exported_at TEXT,
              last_exported_usb_root TEXT,
              last_exported_track_count INTEGER
            );
            CREATE TABLE playlist_tracks (
              playlist_id TEXT NOT NULL,
              track_id TEXT NOT NULL
            );
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn normalize_clamps_position_to_duration() {
        let out = normalize_cues(&[input(999_999, None)], Some(10_000)).expect("ok");
        assert_eq!(out[0].position_ms, 9_999);
        assert_eq!(out[0].color_id, Some(DEFAULT_HOTCUE_COLOR_ID));
    }

    #[test]
    fn normalize_rejects_more_than_8_cues() {
        let inputs: Vec<_> = (0..9).map(|i| input(i * 1000, None)).collect();
        let err = normalize_cues(&inputs, Some(300_000)).expect_err("9 cues");
        assert!(err.to_string().contains("at most 8"));
    }

    #[test]
    fn normalize_rejects_unknown_color() {
        let err = normalize_cues(&[input(0, Some(99))], None).expect_err("bad color");
        assert!(err.to_string().contains("colorId"));
    }

    #[test]
    fn normalize_key_input_accepts_canonical_values() {
        assert_eq!(normalize_key_input("Am").expect("valid key"), "Am");
        assert_eq!(normalize_key_input("  F#  ").expect("trims"), "F#");
    }

    #[test]
    fn normalize_key_input_rejects_non_canonical_values() {
        assert!(normalize_key_input("Eb").is_err(), "flat spelling not in KEY_OPTIONS");
        assert!(normalize_key_input("").is_err(), "empty key");
        assert!(normalize_key_input("8B").is_err(), "camelot notation not in KEY_OPTIONS");
    }

    #[test]
    fn normalize_trims_names_keeps_valid_color_and_treats_zero_duration_as_unbounded() {
        let out = normalize_cues(
            &[
                named_input(u32::MAX, Some(8), Some("  Drop  ")),
                named_input(1234, None, Some("   ")),
            ],
            Some(0),
        )
        .expect("ok");

        assert_eq!(out[0].position_ms, u32::MAX);
        assert_eq!(out[0].color_id, Some(8));
        assert_eq!(out[0].name.as_deref(), Some("Drop"));
        assert_eq!(out[1].color_id, Some(DEFAULT_HOTCUE_COLOR_ID));
        assert_eq!(out[1].name, None);
    }

    #[test]
    fn normalized_to_track_cues_preserves_normalized_fields_with_fresh_ids() {
        let normalized =
            normalize_cues(&[named_input(2000, Some(3), Some("Build"))], None).expect("ok");
        let cues = normalized_to_track_cues(&normalized);

        assert_eq!(cues.len(), 1);
        assert!(!cues[0].id.is_empty());
        assert_eq!(cues[0].position_ms, 2000);
        assert_eq!(cues[0].color_id, Some(3));
        assert_eq!(cues[0].name.as_deref(), Some("Build"));
    }

    #[test]
    fn anlz_cues_expands_each_point_to_memory_plus_hot() {
        let cues = vec![
            cue("c1", 3000, Some(2)),
            cue("c2", 1000, Some(5)),
            cue("c3", 8000, None),
        ];
        let anlz = anlz_cues_from_track_cues(&cues);
        assert_eq!(anlz.len(), 6);
        // Ordered by position; slots assigned 1,2,3.
        let hots: Vec<_> = anlz.iter().filter(|c| c.hot_cue != 0).collect();
        let mems: Vec<_> = anlz.iter().filter(|c| c.hot_cue == 0).collect();
        assert_eq!(hots.len(), 3);
        assert_eq!(mems.len(), 3);
        assert_eq!(hots[0].position_ms, 1000);
        assert_eq!(hots[0].hot_cue, 1);
        assert_eq!(hots[1].position_ms, 3000);
        assert_eq!(hots[1].hot_cue, 2);
        assert_eq!(hots[1].color_id, 2);
        assert!(mems.iter().any(|c| c.position_ms == 8000));
    }

    #[test]
    fn load_track_cues_orders_by_sort_order_then_position_and_clamps_negative_positions() {
        let conn = cue_conn();
        conn.execute(
            "INSERT INTO track_cues
               (id, track_id, position_ms, color_id, name, sort_order, created_at, updated_at)
             VALUES
               ('late', 'track-1', 9000, NULL, NULL, 2, 'old', 'old'),
               ('neg', 'track-1', -50, 4, 'Start', 1, 'old', 'old'),
               ('early', 'track-1', 1000, 5, 'Intro', 1, 'old', 'old')",
            [],
        )
        .unwrap();

        let cues = load_track_cues(&conn, "track-1").expect("cues");
        assert_eq!(
            cues.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["neg", "early", "late"]
        );
        assert_eq!(cues[0].position_ms, 0);
        assert_eq!(cues[0].color_id, Some(4));
        assert_eq!(cues[0].name.as_deref(), Some("Start"));
    }

    #[test]
    fn load_track_cues_bulk_groups_cues_by_track_and_skips_tracks_without_cues() {
        let conn = cue_conn();
        conn.execute(
            "INSERT INTO track_cues
               (id, track_id, position_ms, color_id, name, sort_order, created_at, updated_at)
             VALUES
               ('a2', 'track-a', 2000, 2, NULL, 2, 'old', 'old'),
               ('a1', 'track-a', 1000, 1, NULL, 1, 'old', 'old'),
               ('b1', 'track-b', 3000, NULL, 'Break', 1, 'old', 'old')",
            [],
        )
        .unwrap();

        let out = load_track_cues_bulk(
            &conn,
            &[
                "track-a".to_string(),
                "track-b".to_string(),
                "track-c".to_string(),
            ],
        )
        .expect("bulk cues");

        assert_eq!(
            out.get("track-a")
                .unwrap()
                .iter()
                .map(|cue| cue.id.as_str())
                .collect::<Vec<_>>(),
            ["a1", "a2"]
        );
        assert_eq!(
            out.get("track-b").unwrap()[0].name.as_deref(),
            Some("Break")
        );
        assert!(!out.contains_key("track-c"));
    }

    #[test]
    fn apply_local_analysis_edits_replaces_cues_and_invalidates_playlist_export_markers() {
        let mut conn = cue_conn();
        conn.execute(
            "INSERT INTO tracks (id, first_beat_ms, first_beat_ms_source, updated_at)
             VALUES ('track-1', NULL, NULL, 'old')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO playlists
               (id, updated_at, last_exported_at, last_exported_usb_root, last_exported_track_count)
             VALUES ('playlist-1', 'old', 'exported', '/usb', 3)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO playlist_tracks (playlist_id, track_id) VALUES ('playlist-1', 'track-1')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO track_cues
               (id, track_id, position_ms, color_id, name, sort_order, created_at, updated_at)
             VALUES ('old-cue', 'track-1', 10, 1, 'Old', 0, 'old', 'old')",
            [],
        )
        .unwrap();

        let normalized = normalize_cues(
            &[
                named_input(3000, Some(2), Some("Two")),
                named_input(1000, Some(1), Some("One")),
            ],
            Some(10_000),
        )
        .expect("normalized");
        let tx = conn.transaction().unwrap();
        apply_local_analysis_edits_tx(
            &tx,
            "track-1",
            Some(500),
            Some(&normalized),
            Some(128.3),
            Some("F#m"),
            "new",
        )
        .expect("apply edits");
        tx.commit().unwrap();

        let first_beat: (i64, String, String) = conn
            .query_row(
                "SELECT first_beat_ms, first_beat_ms_source, updated_at FROM tracks WHERE id = 'track-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(first_beat, (500, "user".to_string(), "new".to_string()));

        let bpm: (f64, String) = conn
            .query_row(
                "SELECT bpm, bpm_analyzer FROM tracks WHERE id = 'track-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(bpm, (128.3, "user".to_string()));

        let key: (String, String) = conn
            .query_row(
                "SELECT tonality, tonality_source FROM tracks WHERE id = 'track-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(key, ("F#m".to_string(), "user".to_string()));

        let cues = load_track_cues(&conn, "track-1").expect("cues");
        assert_eq!(
            cues.iter()
                .map(|cue| cue.name.as_deref())
                .collect::<Vec<_>>(),
            [Some("Two"), Some("One")]
        );
        let playlist: (String, Option<String>, Option<String>, Option<i64>) = conn
            .query_row(
                "SELECT updated_at, last_exported_at, last_exported_usb_root, last_exported_track_count
                   FROM playlists WHERE id = 'playlist-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(playlist, ("new".to_string(), None, None, None));
    }

    #[test]
    fn apply_local_analysis_edits_with_no_changes_leaves_timestamps_untouched() {
        let mut conn = cue_conn();
        conn.execute(
            "INSERT INTO tracks (id, first_beat_ms, first_beat_ms_source, updated_at)
             VALUES ('track-1', NULL, NULL, 'old')",
            [],
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        apply_local_analysis_edits_tx(&tx, "track-1", None, None, None, None, "new").expect("noop");
        tx.commit().unwrap();

        let updated_at: String = conn
            .query_row(
                "SELECT updated_at FROM tracks WHERE id = 'track-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(updated_at, "old");
    }
}
