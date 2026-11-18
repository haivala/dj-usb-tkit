//! What a rekordbox / Mixxx import does with a track that is already in the
//! library: which of its cues, BPM, key and beat grid differ from the external
//! library's in a way the user has to decide, and what happens to its cues.
//! Shared by both importers so they treat a track identically.
//!
//! A value is taken without asking when the track has none, or when it came
//! from the same library before (so a change made there comes across). A value
//! from anywhere else (the cue editor, the app's analysis, the other library, a
//! USB stick) that differs from the library's is a conflict: the import keeps
//! it and reports the track, and the user picks a side afterwards
//! (`apply_external_track_data`).

use std::collections::HashSet;
use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::BackendResult;
use crate::i18n::tr;
use crate::models::{
    ApplyExternalTrackDataData, ApplyExternalTrackDataRequest, ExternalLibrary, ImportConflictRow,
    ImportConflicts, TrackCue,
};

use super::BackendService;
use super::cues::load_track_cues;
use super::key_notation::camelot_position;
use super::playlists::external_library_label;

/// The library track's current values and where each came from.
#[derive(Debug, Default)]
pub(crate) struct LocalTrackData {
    pub bpm: Option<f64>,
    pub bpm_source: Option<String>,
    pub key: Option<String>,
    pub key_source: Option<String>,
    pub first_beat_ms: Option<i64>,
    pub first_beat_source: Option<String>,
    pub cues: Vec<TrackCue>,
    pub cue_source: Option<String>,
}

pub(crate) fn load_local_track_data(
    conn: &Connection,
    track_id: &str,
) -> BackendResult<LocalTrackData> {
    let row = conn
        .query_row(
            "SELECT bpm, bpm_analyzer, tonality, tonality_source, first_beat_ms,
                    first_beat_ms_source, cue_source
               FROM tracks WHERE id = ?1",
            params![track_id],
            |row| {
                Ok(LocalTrackData {
                    bpm: row.get(0)?,
                    bpm_source: row.get(1)?,
                    key: row.get(2)?,
                    key_source: row.get(3)?,
                    first_beat_ms: row.get(4)?,
                    first_beat_source: row.get(5)?,
                    cues: Vec::new(),
                    cue_source: row.get(6)?,
                })
            },
        )
        .optional()?;
    let Some(mut local) = row else {
        return Ok(LocalTrackData::default());
    };
    local.cues = load_track_cues(conn, track_id)?;
    Ok(local)
}

/// The file paths of these library tracks, as an import matches them.
pub(crate) fn track_file_paths(
    conn: &Connection,
    track_ids: &[String],
) -> BackendResult<HashSet<PathBuf>> {
    let mut stmt = conn.prepare("SELECT file_path FROM tracks WHERE id = ?1")?;
    let mut paths = HashSet::new();
    for id in track_ids {
        if let Some(path) = stmt
            .query_row(params![id], |row| row.get::<_, String>(0))
            .optional()?
        {
            paths.insert(PathBuf::from(path));
        }
    }
    Ok(paths)
}

/// The external library's values for the same file.
#[derive(Debug, Default)]
pub(crate) struct ExternalTrackData<'a> {
    pub bpm: Option<f64>,
    pub key: Option<&'a str>,
    pub first_beat_ms: Option<i64>,
    pub cues: &'a [TrackCue],
}

/// The fields the user has to decide on for one track.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DifferingFields {
    pub cues: bool,
    pub bpm: bool,
    pub key: bool,
    pub grid: bool,
}

impl DifferingFields {
    pub fn any(self) -> bool {
        self.cues || self.bpm || self.key || self.grid
    }
}

fn from_elsewhere(local_source: Option<&str>, source: &str) -> bool {
    local_source != Some(source)
}

pub(crate) fn same_bpm(a: f64, b: f64) -> bool {
    // The PDB keeps BPM in hundredths.
    (a * 100.0).round() == (b * 100.0).round()
}

pub(crate) fn same_key(a: &str, b: &str) -> bool {
    match (camelot_position(a), camelot_position(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a.trim().eq_ignore_ascii_case(b.trim()),
    }
}

/// Grid tolerance: the first beat may sit a whole number of beats apart and
/// still be the same grid.
const GRID_TOLERANCE_MS: f64 = 2.0;

pub(crate) fn same_grid(a_ms: i64, b_ms: i64, bpm: Option<f64>) -> bool {
    let diff = (a_ms - b_ms).unsigned_abs() as f64;
    match bpm.filter(|bpm| *bpm > 0.0) {
        Some(bpm) => {
            let beat_ms = 60_000.0 / bpm;
            let rest = diff % beat_ms;
            rest.min(beat_ms - rest) <= GRID_TOLERANCE_MS
        }
        None => diff <= GRID_TOLERANCE_MS,
    }
}

/// Cue positions converted from samples can be a millisecond apart.
const CUE_TOLERANCE_MS: u32 = 1;

pub(crate) fn same_cues(a: &[TrackCue], b: &[TrackCue]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    fn sorted(cues: &[TrackCue]) -> Vec<&TrackCue> {
        let mut cues: Vec<&TrackCue> = cues.iter().collect();
        cues.sort_by_key(|cue| (cue.position_ms, cue.memory, cue.playback_start));
        cues
    }
    let name = |cue: &TrackCue| {
        cue.name
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    sorted(a).iter().zip(sorted(b)).all(|(x, y)| {
        x.position_ms.abs_diff(y.position_ms) <= CUE_TOLERANCE_MS
            && x.memory == y.memory
            && x.playback_start == y.playback_start
            && x.color_id == y.color_id
            && name(x) == name(y)
    })
}

/// Which of the track's values conflict with the library's: the library has a
/// value, the track has its own from somewhere else, and they differ. An
/// emptied cue list is the user's own too.
pub(crate) fn differing_fields(
    local: &LocalTrackData,
    external: &ExternalTrackData<'_>,
    source: &str,
) -> DifferingFields {
    let bpm = match (local.bpm, external.bpm) {
        (Some(mine), Some(theirs)) => {
            from_elsewhere(local.bpm_source.as_deref(), source) && !same_bpm(mine, theirs)
        }
        _ => false,
    };
    let key = match (local.key.as_deref(), external.key) {
        (Some(mine), Some(theirs)) if !mine.trim().is_empty() && !theirs.trim().is_empty() => {
            from_elsewhere(local.key_source.as_deref(), source) && !same_key(mine, theirs)
        }
        _ => false,
    };
    let grid = match (local.first_beat_ms, external.first_beat_ms) {
        (Some(mine), Some(theirs)) => {
            from_elsewhere(local.first_beat_source.as_deref(), source)
                && !same_grid(mine, theirs, external.bpm)
        }
        _ => false,
    };
    let local_cues_owned = !local.cues.is_empty() || local.cue_source.as_deref() == Some("user");
    let cues = !external.cues.is_empty()
        && local_cues_owned
        && from_elsewhere(local.cue_source.as_deref(), source)
        && !same_cues(&local.cues, external.cues);
    DifferingFields {
        cues,
        bpm,
        key,
        grid,
    }
}

/// What an import does with the track's cue list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CueAction {
    /// Leave the track's cues as they are.
    Keep,
    /// Replace them with the library's.
    Write,
    /// They already match the library's: only record it as their source.
    AdoptSource,
}

pub(crate) fn cue_action(
    local: &LocalTrackData,
    external: &ExternalTrackData<'_>,
    force: bool,
    differing: DifferingFields,
) -> CueAction {
    if external.cues.is_empty() {
        return CueAction::Keep;
    }
    if force {
        return CueAction::Write;
    }
    if differing.cues {
        return CueAction::Keep;
    }
    if same_cues(&local.cues, external.cues) {
        return match local.cue_source {
            None if !local.cues.is_empty() => CueAction::AdoptSource,
            _ => CueAction::Keep,
        };
    }
    // Different but not a conflict: the track has none of its own, or its
    // cues came from this library before.
    CueAction::Write
}

/// A track the import left as it was because its values differ.
#[derive(Debug, Clone)]
pub(crate) struct ConflictedTrack {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub fields: DifferingFields,
}

/// The question for the user after an import, or `None` when nothing differs.
pub(crate) fn import_conflicts(
    library: ExternalLibrary,
    path: Option<&str>,
    tracks: &[ConflictedTrack],
) -> Option<ImportConflicts> {
    if tracks.is_empty() {
        return None;
    }
    let label = external_library_label(library);
    let rows = tracks
        .iter()
        .map(|track| ImportConflictRow {
            track_id: track.track_id.clone(),
            name_text: match track.artist.trim() {
                "" => track.title.clone(),
                artist => format!("{artist} – {}", track.title),
            },
            differs_text: differs_text(track.fields),
        })
        .collect();
    Some(ImportConflicts {
        library,
        path: path.map(str::to_string),
        track_ids: tracks.iter().map(|track| track.track_id.clone()).collect(),
        title_text: tr::import_conflicts_title(label).render(),
        message_text: tr::import_conflicts_message(label, tracks.len()).render(),
        use_label: tr::import_conflicts_use(label).render(),
        rows,
    })
}

fn differs_text(fields: DifferingFields) -> String {
    let mut parts = Vec::new();
    if fields.cues {
        parts.push(tr::import_conflicts_field_cues().render());
    }
    if fields.bpm {
        parts.push(tr::import_conflicts_field_bpm().render());
    }
    if fields.key {
        parts.push(tr::import_conflicts_field_key().render());
    }
    if fields.grid {
        parts.push(tr::import_conflicts_field_grid().render());
    }
    parts.join(", ")
}

/// Merge one library's conflicts from several imports (the playlist import
/// runs one per list) into a single question per library.
pub(crate) fn merge_import_conflicts(all: Vec<ImportConflicts>) -> Vec<ImportConflicts> {
    let mut merged: Vec<(ExternalLibrary, Option<String>, Vec<ImportConflictRow>)> = Vec::new();
    for conflicts in all {
        let entry = match merged
            .iter_mut()
            .find(|(library, path, _)| *library == conflicts.library && *path == conflicts.path)
        {
            Some(entry) => entry,
            None => {
                merged.push((conflicts.library, conflicts.path.clone(), Vec::new()));
                merged.last_mut().expect("just pushed")
            }
        };
        for row in conflicts.rows {
            if !entry.2.iter().any(|seen| seen.track_id == row.track_id) {
                entry.2.push(row);
            }
        }
    }
    merged
        .into_iter()
        .map(|(library, path, rows)| {
            let label = external_library_label(library);
            ImportConflicts {
                library,
                path,
                track_ids: rows.iter().map(|row| row.track_id.clone()).collect(),
                title_text: tr::import_conflicts_title(label).render(),
                message_text: tr::import_conflicts_message(label, rows.len()).render(),
                use_label: tr::import_conflicts_use(label).render(),
                rows,
            }
        })
        .collect()
}

impl BackendService {
    /// The conflict dialog's "Use rekordbox's / Mixxx's".
    pub fn apply_external_track_data(
        &self,
        req: ApplyExternalTrackDataRequest,
    ) -> BackendResult<ApplyExternalTrackDataData> {
        match req.library {
            ExternalLibrary::Rekordbox => self.apply_rekordbox_track_data(&req),
            ExternalLibrary::Mixxx => self.apply_mixxx_track_data(&req),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(position_ms: u32) -> TrackCue {
        TrackCue {
            id: format!("c{position_ms}"),
            position_ms,
            color_id: None,
            name: None,
            playback_start: false,
            memory: false,
        }
    }

    fn local_with_cues(cues: Vec<TrackCue>, source: Option<&str>) -> LocalTrackData {
        LocalTrackData {
            cues,
            cue_source: source.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn bpm_compares_in_hundredths() {
        assert!(same_bpm(128.0, 128.004));
        assert!(!same_bpm(128.0, 128.01));
    }

    #[test]
    fn key_spellings_of_the_same_key_match() {
        assert!(same_key("Am", "8A"));
        assert!(same_key("F#m", "Gbm"));
        assert!(!same_key("Am", "C"));
    }

    #[test]
    fn grid_shifted_by_whole_beats_is_the_same_grid() {
        // 120 BPM: 500 ms a beat.
        assert!(same_grid(100, 2_100, Some(120.0)));
        assert!(same_grid(100, 1_601, Some(120.0)));
        assert!(!same_grid(100, 350, Some(120.0)));
        assert!(!same_grid(100, 110, None));
    }

    #[test]
    fn cues_match_within_a_millisecond_regardless_of_order() {
        assert!(same_cues(
            &[cue(1_000), cue(5_000)],
            &[cue(5_001), cue(999)]
        ));
        assert!(!same_cues(&[cue(1_000)], &[cue(1_002)]));
        assert!(!same_cues(&[cue(1_000)], &[cue(1_000), cue(2_000)]));
        let mut named = cue(1_000);
        named.name = Some("Drop".into());
        assert!(!same_cues(&[cue(1_000)], &[named]));
    }

    #[test]
    fn cues_edited_in_the_app_conflict_and_are_kept() {
        let external_cues = [cue(2_000)];
        let external = ExternalTrackData {
            cues: &external_cues,
            ..Default::default()
        };
        let local = local_with_cues(vec![cue(1_000)], Some("user"));
        let fields = differing_fields(&local, &external, "rekordbox");
        assert!(fields.cues);
        assert_eq!(
            cue_action(&local, &external, false, fields),
            CueAction::Keep
        );
        assert_eq!(
            cue_action(&local, &external, true, fields),
            CueAction::Write
        );
    }

    #[test]
    fn cues_from_the_same_library_refresh_without_asking() {
        let external_cues = [cue(2_000)];
        let external = ExternalTrackData {
            cues: &external_cues,
            ..Default::default()
        };
        let local = local_with_cues(vec![cue(1_000)], Some("rekordbox"));
        let fields = differing_fields(&local, &external, "rekordbox");
        assert!(!fields.any());
        assert_eq!(
            cue_action(&local, &external, false, fields),
            CueAction::Write
        );
        // The other library's cues are someone else's.
        let fields = differing_fields(&local, &external, "mixxx");
        assert!(fields.cues);
    }

    #[test]
    fn unknown_source_cues_that_match_adopt_the_library() {
        let external_cues = [cue(1_000)];
        let external = ExternalTrackData {
            cues: &external_cues,
            ..Default::default()
        };
        let local = local_with_cues(vec![cue(1_000)], None);
        let fields = differing_fields(&local, &external, "mixxx");
        assert!(!fields.any());
        assert_eq!(
            cue_action(&local, &external, false, fields),
            CueAction::AdoptSource
        );
    }

    #[test]
    fn an_emptied_cue_list_is_the_users_but_no_cues_fill_in() {
        let external_cues = [cue(1_000)];
        let external = ExternalTrackData {
            cues: &external_cues,
            ..Default::default()
        };
        let emptied = local_with_cues(Vec::new(), Some("user"));
        assert!(differing_fields(&emptied, &external, "rekordbox").cues);
        let never = local_with_cues(Vec::new(), None);
        let fields = differing_fields(&never, &external, "rekordbox");
        assert!(!fields.any());
        assert_eq!(
            cue_action(&never, &external, false, fields),
            CueAction::Write
        );
    }

    #[test]
    fn analysed_bpm_and_key_conflict_but_not_when_missing() {
        let external = ExternalTrackData {
            bpm: Some(128.0),
            key: Some("8A"),
            ..Default::default()
        };
        let local = LocalTrackData {
            bpm: Some(127.5),
            bpm_source: Some("stratum".into()),
            key: Some("C".into()),
            key_source: Some("stratum".into()),
            ..Default::default()
        };
        let fields = differing_fields(&local, &external, "rekordbox");
        assert!(fields.bpm && fields.key && !fields.cues && !fields.grid);
        let empty = LocalTrackData::default();
        assert!(!differing_fields(&empty, &external, "rekordbox").any());
    }
}
