//! Analysis requests and results.

use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeNewTracksRequest {
    #[serde(default)]
    pub track_ids: Vec<String>,
    /// When `track_ids` is empty: restrict auto-selection to the tracks in
    /// these playlists that still need core analysis (over each whole
    /// playlist, not a page). Ignored when `track_ids` is non-empty.
    #[serde(default)]
    pub playlist_ids: Vec<String>,
    /// When `track_ids` and `playlist_ids` are empty: restrict
    /// auto-selection to the library as filtered (its source folders and
    /// libraries, plus `query`) instead of the whole DB.
    #[serde(default)]
    pub scope_to_library_filter: bool,
    /// Preferred: the raw range string from the settings dropdown, e.g.
    /// `"70-180"`. The backend parses and validates it (see
    /// `resolve_analysis_bpm_range`). `bpm_min`/`bpm_max` are the older
    /// pre-parsed form, still accepted as a fallback.
    #[serde(default)]
    pub bpm_range: Option<String>,
    #[serde(default)]
    pub bpm_min: Option<u32>,
    #[serde(default)]
    pub bpm_max: Option<u32>,
    #[serde(default)]
    pub analysis_engine: Option<String>,
    /// The library search, for `scope_to_library_filter` and the live
    /// library total time.
    #[serde(default)]
    pub query: String,
    /// With `track_ids`: analyze each one afresh, replacing an imported or
    /// edited BPM, key and first beat even on its first analysis (the
    /// selection bar's Reanalyze). Without it a first analysis keeps them.
    #[serde(default)]
    pub reanalyze: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeNewTracksData {
    /// Tracks that came out analysis-ready (waveform + BPM + duration).
    pub analyzed: usize,
    /// Tracks whose audio decoded but analysis left something out (no BPM
    /// detected, no duration) -- they still show as needing analysis.
    #[serde(default)]
    pub incomplete: usize,
    pub failed: usize,
    /// `analyzed + incomplete + failed`.
    #[serde(default)]
    pub processed: usize,
    pub warnings: Vec<WarningEntry>,
    /// Requested tracks left out because another run is on them: their rows
    /// stay "analyzing" until that run reports them.
    #[serde(default)]
    pub already_analyzing_track_ids: Vec<String>,
}

/// What became of a request added to the analysis job. The job reports the rest
/// through its events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisSubmittedData {
    pub job_id: String,
    pub queued: usize,
    pub skipped: usize,
    /// For the status line.
    pub message: String,
    /// Picked rows that were files not in the library yet, now added: the
    /// window shows `tracks` in their place.
    #[serde(default)]
    pub replaced: Vec<crate::models::ReplacedTrackRow>,
    /// The library rows of `replaced`.
    #[serde(default)]
    pub tracks: Vec<crate::models::Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAnalysisPausedRequest {
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAnalysisPausedData {
    pub paused: bool,
}
