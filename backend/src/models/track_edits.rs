//! A track's detail and its edits: cues, beat grid, key and metadata, for
//! library tracks and for tracks on a USB stick.

use super::*;
use serde::{Deserialize, Serialize};

/// A cue point on a track. This app targets CDJ playback directly, so a cue is
/// just a position + optional name + colour. A *hot cue* (the default) is
/// written on export as both a memory point and a hot-cue pad (A–H); a track
/// has at most 8.
///
/// `memory` marks a *memory cue*: a memory point only (no pad), with an
/// optional name and colour, as rekordbox writes them to mark a track's
/// structure. A track can have any number; they don't count toward the 8.
///
/// `playback_start` marks the one optional *playback-start* cue: a memory
/// point only (no hot-cue pad, no colour, no name), never later than any hot cue, so a
/// CDJ's auto-cue loads there instead of on the first hot cue. It does not
/// count toward the 8. A cue is never both `memory` and `playback_start`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackCue {
    pub id: String,
    pub position_ms: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_id: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub playback_start: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub memory: bool,
}

impl TrackCue {
    /// A cue with a hot-cue pad: neither a memory cue nor the playback start.
    pub fn is_hot(&self) -> bool {
        !self.playback_start && !self.memory
    }
}

/// One cue in a `save_track_analysis_edits` request (no id — the save fully
/// replaces the track's cue list).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackCueInput {
    pub position_ms: u32,
    #[serde(default)]
    pub color_id: Option<u8>,
    #[serde(default)]
    pub name: Option<String>,
    /// The memory-only playback-start cue (see [`TrackCue::playback_start`]).
    #[serde(default)]
    pub playback_start: bool,
    /// A memory cue (see [`TrackCue::memory`]).
    #[serde(default)]
    pub memory: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTrackDetailRequest {
    pub track_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackDetail {
    pub track: Track,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_beat_ms: Option<u32>,
    pub cues: Vec<TrackCue>,
    /// Base64 of the raw PWV5 colour-detail waveform payload from the track's
    /// `.EXT` bundle (2 bytes/entry, BE u16). Absent when the track has no
    /// analysis. Base64 rather than `Vec<u8>` because the payload is tens of KB
    /// and a JSON number array would be ~3× that on the wire.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_waveform: Option<String>,
    /// The keys the cue editor offers, grouped for its picker. A save only
    /// accepts one of these.
    pub key_options: Vec<KeyOptionGroup>,
    /// How many hot cues the editor offers (its A–H slots); a save refuses more.
    pub max_hot_cues: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    /// Star rating, 0 (none) to 5.
    pub rating: u8,
    /// The release date export writes (`YYYY-MM-DD`: the track's date,
    /// completed from its file timestamp), or as much of one as it has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    /// Genres the editor's genre field suggests, sorted case-insensitively.
    pub genre_options: Vec<String>,
}

/// One group of the cue editor's key picker ("Major" / "Minor"), in order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeyOptionGroup {
    pub label: String,
    pub keys: Vec<KeyOption>,
}

/// One entry of the key picker: `value` is the classic key a save sends,
/// `label` is how it's shown in the user's key notation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeyOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveTrackAnalysisEditsRequest {
    pub track_id: String,
    /// `None` ⇒ leave the stored first beat unchanged.
    #[serde(default)]
    pub first_beat_ms: Option<u32>,
    /// `None` ⇒ leave the cue list unchanged; `Some(_)` ⇒ full replace.
    #[serde(default)]
    pub cues: Option<Vec<TrackCueInput>>,
    /// `None` ⇒ leave the stored bpm unchanged.
    #[serde(default)]
    pub bpm: Option<f64>,
    /// `None` ⇒ leave the stored musical key unchanged.
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveTrackAnalysisEditsData {
    pub track_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_beat_ms: Option<u32>,
    pub cues: Vec<TrackCue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm_analyzer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_color: Option<u8>,
    /// Whether the cached local ANLZ bundle was successfully rewritten with the
    /// new cues / beat grid. `false` when the track has no analysis cache yet
    /// (the edits are still persisted and applied at the next analysis/export).
    pub anlz_regenerated: bool,
    /// The track's whole library row after the save, for the track tables.
    pub track: Track,
    /// "Saved 3 cues", noting when the analysis cache wasn't updated.
    pub status_text: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetUsbTrackDetailRequest {
    pub usb_root: String,
    /// `UsbTrack.usb_analysis_path_raw` — the un-resolved on-device ANLZ path.
    pub usb_analysis_path_raw: String,
    /// The track's PDB-relative media path, to read its genre and rating.
    #[serde(default)]
    pub usb_media_path_raw: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbTrackAnalysisDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_beat_ms: Option<u32>,
    pub cues: Vec<TrackCue>,
    /// Base64 PWV5 colour-detail waveform, same encoding as `TrackDetail`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_waveform: Option<String>,
    /// Same as `TrackDetail::key_options`.
    pub key_options: Vec<KeyOptionGroup>,
    /// Same as `TrackDetail::max_hot_cues`.
    pub max_hot_cues: u8,
    /// The track's genre in the USB's PDB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    /// The track's star rating in the USB's PDB, 0 to 5.
    pub rating: u8,
    /// The track's release date in the USB's PDB, else its year.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    /// The library's genres and the USB's, sorted case-insensitively.
    pub genre_options: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveUsbTrackAnalysisEditsRequest {
    pub usb_root: String,
    pub usb_analysis_path_raw: String,
    /// eDB `content.path` lookup key (the PDB-relative media path).
    pub usb_media_path_raw: String,
    /// The track's current bpm (edited or not) — always required for the
    /// on-device beat-grid rewrite, since the ANLZ write is a full rebuild
    /// rather than an incremental patch.
    #[serde(default)]
    pub bpm: Option<f64>,
    /// `None` ⇒ leave the stored musical key unchanged.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub first_beat_ms: Option<u32>,
    /// `None` ⇒ leave the cue list unchanged; `Some(_)` ⇒ full replace.
    #[serde(default)]
    pub cues: Option<Vec<TrackCueInput>>,
    /// Hint from `UsbTrack.local_track_id`; the save still verifies the row.
    #[serde(default)]
    pub local_track_id: Option<String>,
    /// Metadata for the fingerprint fallback when no id hint / link row matches.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveUsbTrackAnalysisEditsData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_beat_ms: Option<u32>,
    pub cues: Vec<TrackCue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm_analyzer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_color: Option<u8>,
    pub anlz_updated: bool,
    pub edb_updated: bool,
    /// `false` only when no local `tracks` row could be matched.
    pub local_updated: bool,
    /// The local `tracks` row the save also wrote (resolved by the backend,
    /// not just the request's hint), so the UI can refresh that library track
    /// and the app playlists that contain it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_track_id: Option<String>,
    /// The edited file's whole stick row after the save, for the USB track
    /// tables (absent when no playlist or history lists the file).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<UsbTrack>,
    /// `local_track_id`'s whole library row after the save.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_track: Option<Track>,
    /// "Saved 3 cues to USB".
    pub status_text: String,
}

/// Genre / star rating / release date edit, from the cue editor (one track)
/// or the selection bar's Edit (many). Separate from the analysis save so a
/// metadata-only edit leaves the beat grid, key and cues alone. Every track
/// gets the same values.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveTrackMetadataEditsRequest {
    pub track_ids: Vec<String>,
    /// `None` ⇒ leave unchanged; an empty string clears the genre.
    #[serde(default)]
    pub genre: Option<String>,
    /// `None` ⇒ leave unchanged; 0 (no stars) to 5.
    #[serde(default)]
    pub rating: Option<u8>,
    /// `None` ⇒ leave unchanged; `YYYY`, `YYYY-MM` or `YYYY-MM-DD` sets the
    /// release date and its year, an empty string clears them.
    #[serde(default)]
    pub release_date: Option<String>,
}

/// A library track's genre, rating, release year and release date (as
/// `TrackDetail::release_date`) after a save.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackMetadataValues {
    pub track_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    pub rating: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveTrackMetadataEditsData {
    /// One per saved track, in request order.
    pub items: Vec<TrackMetadataValues>,
    /// The saved tracks' whole library rows, for the track tables.
    pub tracks: Vec<Track>,
    /// "Saved track details for 2 tracks".
    pub status_text: String,
}

/// One USB track of a [`SaveUsbTrackMetadataEditsRequest`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbMetadataEditTrack {
    /// eDB `content.path` / PDB track path lookup key.
    pub usb_media_path_raw: String,
    /// Same as `SaveUsbTrackAnalysisEditsRequest::local_track_id`.
    #[serde(default)]
    pub local_track_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
}

/// [`SaveTrackMetadataEditsRequest`] for tracks opened from a USB view:
/// written to the USB's PDB and eDB in place, and to the matched library
/// tracks.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveUsbTrackMetadataEditsRequest {
    pub usb_root: String,
    pub tracks: Vec<UsbMetadataEditTrack>,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub rating: Option<u8>,
    #[serde(default)]
    pub release_date: Option<String>,
}

/// What a USB metadata save did to one track's library copy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbMetadataEditResult {
    pub usb_media_path_raw: String,
    /// The library track the edit was also written to, if one matched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_track_id: Option<String>,
    /// The track's release year and date in the PDB after the save.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveUsbTrackMetadataEditsData {
    /// One per saved track, in request order.
    pub items: Vec<UsbMetadataEditResult>,
    /// The edited files' whole stick rows after the save, for the USB track
    /// tables (a file no playlist or history lists has none).
    pub tracks: Vec<UsbTrack>,
    /// The library tracks the edits were also written to, whole.
    pub local_tracks: Vec<Track>,
    /// "Saved track details for 2 tracks to USB".
    pub status_text: String,
}

/// One field across a set of tracks: the value they all share, or `mixed`
/// when they differ (then `value` is absent).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedValue<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<T>,
    pub mixed: bool,
}

/// What a set of tracks has in common, for the selection bar's Edit: each
/// field shared by all of them, or mixed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TracksMetadataSummary {
    pub count: usize,
    pub genre: SharedValue<String>,
    /// Star rating, 0 (none) to 5.
    pub rating: SharedValue<u8>,
    /// As `TrackDetail::release_date` (library) or the PDB's (USB).
    pub release_date: SharedValue<String>,
    /// Genres the editor's genre field suggests, sorted case-insensitively.
    pub genre_options: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTracksMetadataSummaryRequest {
    pub track_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetUsbTracksMetadataSummaryRequest {
    pub usb_root: String,
    pub usb_media_paths_raw: Vec<String>,
}
