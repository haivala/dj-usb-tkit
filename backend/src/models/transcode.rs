//! Transcoding requests and results.

use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscodeTracksRequest {
    pub track_ids: Vec<String>,
    /// `"aiff"`, `"wav"` or `"mp3"`.
    pub format: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscodeTracksData {
    #[serde(default)]
    pub job_id: String,
    pub transcoded: usize,
    /// Tracks that need no transcode in this format (not lossless, OGG or Opus,
    /// already this format, already up to date, file missing).
    pub skipped: usize,
    pub failed: usize,
    pub cancelled: bool,
    /// One line per skipped or failed track, each also logged to the Event
    /// Log by the backend.
    pub messages: Vec<String>,
    /// The selected tracks as they are now, for the tables to show.
    pub tracks: Vec<Track>,
    /// For the status bar: "Converting done: 2 tracks converted, 1 skipped".
    #[serde(default)]
    pub summary: String,
    /// Selected rows that were files the scan lists but hadn't indexed: the
    /// backend indexed them, so each `track_id` row (in `tracks`) is shown
    /// in place of the row whose id was `previous_id` (its file path).
    #[serde(default)]
    pub replaced: Vec<ReplacedTrackRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacedTrackRow {
    pub previous_id: String,
    pub track_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveTrackTranscodesRequest {
    pub track_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveTrackTranscodesData {
    pub removed: usize,
    /// The selected tracks as they are now, for the tables to show.
    pub tracks: Vec<Track>,
    /// For the status bar: "Removed 2 transcodes".
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscodeFolderData {
    pub path: String,
    /// No folder was chosen; `path` is the default inside the app data.
    pub is_default: bool,
}

/// The formats Convert… offers, in the dialog's order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscodeFormatsData {
    pub formats: Vec<TranscodeFormatChoice>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscodeFormatChoice {
    /// What `transcode_tracks` takes (`"mp3"`).
    pub format: String,
    /// The choice's text (`"MP3 320 kbps"`).
    pub label: String,
    /// Picked when the dialog opens.
    pub is_default: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTranscodeFolderRequest {
    /// `None` or empty goes back to the default folder.
    #[serde(default)]
    pub path: Option<String>,
}
