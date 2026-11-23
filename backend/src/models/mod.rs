//! The request, response and event types the backend sends to and gets
//! from the UI, one module per area. Everything is re-exported here, so
//! `crate::models::X` names any of them.

mod analysis;
mod diagnostics;
mod export;
mod import;
mod library;
mod playback;
mod playlists;
mod settings;
mod track;
mod track_edits;
mod transcode;
mod usb;

pub use analysis::*;
pub use diagnostics::*;
pub use export::*;
pub use import::*;
pub use library::*;
pub use playback::*;
pub use playlists::*;
pub use settings::*;
pub use track::*;
pub use track_edits::*;
pub use transcode::*;
pub use usb::*;

use crate::error::ErrorPayload;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiResponse<T: Serialize> {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorPayload>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn failure(error: ErrorPayload) -> Self {
        // Single choke point: every error returned by any command — Tauri or
        // headless — passes through here, so this is where every failure
        // reaches the Event Log. See backend/src/logging.rs.
        // The Event Log shows the message as the user got it; the log file gets English.
        crate::logging::emit_line(&crate::logging::LogLine {
            level: crate::logging::Level::Error,
            source: error.code.as_str(),
            code: "",
            message: &error.message,
            english: error.log_text(),
        });
        Self {
            ok: false,
            data: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobEventPayload {
    pub event: String,
    pub job_id: String,
    pub job_type: crate::job_conflicts::JobType,
    pub stage: String,
    pub current: usize,
    pub total: usize,
    pub percent: usize,
    /// The step and what it's working on: the progress bar's right side.
    pub message: String,
    /// Counts, sizes, read speed (`crate::progress`): shown next to the
    /// elapsed time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    /// The BPM pill's tooltip for `bpm`: where it came from (see
    /// `Track::bpm_source_text`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm_source_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_color: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artwork_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waveform_peaks_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waveform_preview: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_ready: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_started: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reanalysis: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<bool>,
    /// Authoritative "this track now has its core analysis" (see
    /// `Track::analysis_ready`). Present on analysis progress events; the
    /// frontend patches rows from it and never recomputes readiness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis_ready: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// On a failed analysis track: the Event Log entry, "Track analysis failed: Title (id) - why".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_text: Option<String>,
    /// On an analysis track-ready event: the library footer's "Total time" line, now that
    /// the track's length is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_duration_text: Option<String>,
    /// On an analysis track-ready event: the track's whole library row, which
    /// the frontend shows in place of the one it has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<Box<Track>>,
    /// On `job.tracks_waiting` / `job.tracks_not_waiting`: which tracks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_track_ids: Option<Vec<String>>,
    /// On `job.tracks_waiting`: the tooltip for those rows' own button.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_tooltip: Option<String>,
    /// On an analysis `job.completed` with warnings: the status line's link to the Event Log
    /// ("(2 warnings)"), shown after `message` (the warnings reached the Event Log as the
    /// backend logged them).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_text: Option<String>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WarningEntry {
    pub level: String,
    pub code: String,
    pub message: String,
    pub source: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire<T: Serialize>(value: T) -> String {
        serde_json::to_value(value)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn match_and_source_enums_keep_their_wire_strings() {
        for (matched, text) in [
            (TrackMatch::SelfId, "self"),
            (TrackMatch::Hash, "hash"),
            (TrackMatch::Metadata, "metadata"),
            (TrackMatch::OwnFile, "file"),
            (TrackMatch::NoMatch, "none"),
        ] {
            assert_eq!(wire(matched), text);
            assert_eq!(matched.to_string(), text);
            assert_eq!(wire(ResolvedBy::from(matched)), text);
        }
        for (resolved, text) in [
            (ResolvedBy::Materialized, "materialized"),
            (ResolvedBy::UsbMaterialized, "usbMaterialized"),
            (ResolvedBy::LocalTrackId, "localTrackId"),
            (ResolvedBy::UsbOrigin, "usbOrigin"),
        ] {
            assert_eq!(wire(resolved), text);
        }
        assert_eq!(wire(PlaybackSource::Library), "library");
        assert_eq!(wire(PlaybackSource::Usb), "usb");
    }
}
