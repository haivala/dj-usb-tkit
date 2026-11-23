//! Playback requests, status and events.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvePlaybackSourceRequest {
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub bpm: Option<f64>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub file_size_bytes: Option<i64>,
    /// The id of the row this playback request originated from, if any
    /// (any origin -- library, playlist, USB, history). Lets
    /// `resolve_playback_source` take a fast, indexed self-lookup path for
    /// the common case (a genuine local row) before falling back to the
    /// fingerprint/title search, and lets a playlist entry that still
    /// references a stale USB placeholder self-heal on next play.
    #[serde(default)]
    pub track_id: Option<String>,
}

/// How a track from elsewhere (a USB, an import) was matched to a library
/// track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackMatch {
    /// The track's own id is a library track.
    #[serde(rename = "self")]
    SelfId,
    /// Matching title/artist/album fingerprint, not an audio-byte hash.
    Hash,
    /// Best-effort metadata and filename score.
    Metadata,
    /// The request's own file is a local file on disk; no library track
    /// has its path yet.
    #[serde(rename = "file")]
    OwnFile,
    /// No library track.
    #[serde(rename = "none")]
    NoMatch,
}

impl TrackMatch {
    /// A library track, or the track's own local file, was found.
    pub fn is_match(self) -> bool {
        self != Self::NoMatch
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelfId => "self",
            Self::Hash => "hash",
            Self::Metadata => "metadata",
            Self::OwnFile => "file",
            Self::NoMatch => "none",
        }
    }
}

impl std::fmt::Display for TrackMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a candidate track got its library track id: a [`TrackMatch`], or one
/// of the ways a playlist add resolves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResolvedBy {
    #[serde(rename = "self")]
    SelfId,
    Hash,
    Metadata,
    #[serde(rename = "file")]
    OwnFile,
    #[serde(rename = "none")]
    NoMatch,
    /// A new library track was made for it.
    Materialized,
    /// A new library track was made for a USB track and linked to the USB.
    UsbMaterialized,
    /// The candidate carried the id of its library copy.
    LocalTrackId,
    /// A USB track that stays USB-only.
    UsbOrigin,
}

impl From<TrackMatch> for ResolvedBy {
    fn from(matched: TrackMatch) -> Self {
        match matched {
            TrackMatch::SelfId => Self::SelfId,
            TrackMatch::Hash => Self::Hash,
            TrackMatch::Metadata => Self::Metadata,
            TrackMatch::OwnFile => Self::OwnFile,
            TrackMatch::NoMatch => Self::NoMatch,
        }
    }
}

/// Where a resolved track is played from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackSource {
    Library,
    Usb,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvePlaybackSourceData {
    pub resolved_path: Option<String>,
    pub matched_by: TrackMatch,
    pub track_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayResolvedTrackRequest {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub bpm: Option<f64>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub file_size_bytes: Option<i64>,
    #[serde(default)]
    pub track_id: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub usb_root: Option<String>,
    #[serde(default)]
    pub usb_root_valid: bool,
    #[serde(default)]
    pub start_offset_ms: Option<u64>,
    #[serde(default)]
    pub start_ratio: Option<f64>,
    /// Load (or seek) and hold there, paused, without starting the audio.
    #[serde(default)]
    pub start_paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayResolvedTrackData {
    pub path: String,
    pub playing: bool,
    /// Loaded but held at `position_ms` (a `start_paused` request).
    pub paused: bool,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub track_id: Option<String>,
    pub matched_by: TrackMatch,
    pub source: PlaybackSource,
    pub library_resolved: bool,
    pub has_usb_context: bool,
    /// The status line while it plays: "Playing from Library: Artist - Title".
    pub playing_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StopPlaybackData {
    pub stopped: bool,
    pub previous_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackStatusData {
    pub path: Option<String>,
    pub playing: bool,
    /// A track is loaded but held at `position_ms` (see `pause_playback_native`).
    pub paused: bool,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackEventPayload {
    pub event: String,
    pub path: Option<String>,
    pub playing: bool,
    pub paused: bool,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub message: Option<String>,
    /// The resolved local track id the backend is actually playing, forwarded
    /// on `playback.started` / `playback.seeked` so the frontend marks the
    /// right row as playing without re-resolving a path against its (paginated)
    /// track list. `None` on error/stop events and legacy paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
    pub timestamp: String,
}

/// The cue editor's metronome: clicks on this beat grid are mixed into
/// whatever the native engine plays (see `crate::metronome`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPlaybackMetronomeRequest {
    pub enabled: bool,
    #[serde(default)]
    pub first_beat_ms: Option<f64>,
    #[serde(default)]
    pub bpm: Option<f64>,
    /// Music/click balance, 0..=1: 0 music only, 0.5 both at full level
    /// (the default), 1 clicks only.
    #[serde(default)]
    pub mix: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackMetronomeData {
    /// False when asked to turn on without a usable BPM.
    pub enabled: bool,
}
