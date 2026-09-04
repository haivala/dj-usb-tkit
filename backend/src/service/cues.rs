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
    GetTrackDetailRequest, SaveTrackAnalysisEditsData, SaveTrackAnalysisEditsRequest, TrackCue,
    TrackCueInput, TrackDetail,
};

use super::anlz::{
    AnlzAnalysisEdits, AnlzCue, apply_analysis_edits_to_anlz, atomic_write_bytes,
    read_cues_from_anlz, read_first_beat_from_anlz,
};
use super::usb_utils::read_pwv5_from_anlz;
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

    // Collapse memory + hot entries at the same position into one cue point,
    // preferring a non-empty comment / a valid colour.
    let mut by_position: BTreeMap<u32, (Option<u8>, Option<String>)> = BTreeMap::new();
    for cue in read_cues_from_anlz(&bytes) {
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

    let now = now();
    for (index, (position_ms, (color_id, name))) in
        by_position.into_iter().take(MAX_HOT_CUES as usize).enumerate()
    {
        tx.execute(
            "INSERT INTO track_cues
               (id, track_id, position_ms, color_id, name, sort_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                Uuid::now_v7().to_string(),
                track_id,
                i64::from(position_ms),
                color_id.map(i64::from),
                name,
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

impl BackendService {
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

        let normalized = match req.cues.as_deref() {
            Some(inputs) => Some(normalize_cues(inputs, duration_ms)?),
            None => None,
        };

        let now = now();
        let tx = conn.transaction()?;

        if let Some(first_beat_ms) = req.first_beat_ms {
            tx.execute(
                "UPDATE tracks SET first_beat_ms = ?1, first_beat_ms_source = 'user', updated_at = ?2 WHERE id = ?3",
                params![i64::from(first_beat_ms), now, track_id],
            )?;
        }

        if let Some(cues) = &normalized {
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

        if req.first_beat_ms.is_some() || normalized.is_some() {
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
        let first_beat_ms: Option<u32> = conn
            .query_row(
                "SELECT first_beat_ms FROM tracks WHERE id = ?1",
                params![track_id],
                |row| row.get::<_, Option<i64>>(0),
            )?
            .map(|v| v.max(0) as u32);

        Ok(SaveTrackAnalysisEditsData {
            track_id,
            first_beat_ms,
            cues,
            anlz_regenerated,
        })
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
        let ext_path = dat_path.with_extension("EXT");
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

        let dat = std::fs::read(dat_path)?;
        atomic_write_bytes(dat_path, &apply_analysis_edits_to_anlz(&dat, &edits))?;
        if ext_path.is_file() {
            let ext = std::fs::read(&ext_path)?;
            atomic_write_bytes(&ext_path, &apply_analysis_edits_to_anlz(&ext, &edits))?;
        }
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

    fn cue(id: &str, pos: u32, color: Option<u8>) -> TrackCue {
        TrackCue {
            id: id.to_string(),
            position_ms: pos,
            color_id: color,
            name: None,
        }
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
}
