use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::ErrorPayload;

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
    pub job_type: String,
    pub stage: String,
    pub current: usize,
    pub total: usize,
    pub percent: usize,
    pub message: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_total_duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_duration_unknown_count: Option<usize>,
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
    /// On an analysis `job.completed`: entries for the Event Log.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<WarningEntry>>,
    /// On an analysis `job.completed`: how many of `warnings` are warn or error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_count: Option<usize>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    /// Release year from the file's tags.
    #[serde(default)]
    pub year: Option<u32>,
    /// Genre: the file's tag, or the one set in the cue editor.
    #[serde(default)]
    pub genre: Option<String>,
    pub track_number: Option<u32>,
    pub bpm: Option<f64>,
    /// `AnalysisEngine::bpm_analyzer` for a detected value (`"stratum-v2"`,
    /// `"essentia"`; a bare `"stratum"` is from before BPM refinement),
    /// `"user"` after a manual edit.
    pub bpm_analyzer: Option<String>,
    /// The BPM pill's tooltip: where `bpm` came from ("Manually set",
    /// "Analyzed with: stratum-v2"). Display only; set by
    /// `service::apply_frontend_track_fields`.
    #[serde(default)]
    pub bpm_source_text: Option<String>,
    pub key: Option<String>,
    /// `"stratum"` / `"essentia"` for a detected value, `"user"` after a manual edit.
    pub key_source: Option<String>,
    /// `key` in the user's display notation (Classic/Camelot). Display only;
    /// set by `service::apply_frontend_track_fields`.
    #[serde(default)]
    pub key_display: Option<String>,
    /// Key pill colour group 0..=11 (Camelot wheel number − 1).
    #[serde(default)]
    pub key_color: Option<u8>,
    pub file_path: String,
    pub file_size_bytes: Option<i64>,
    pub format_ext: Option<String>,
    pub sample_rate_hz: Option<u32>,
    pub bit_depth: Option<u8>,
    pub bitrate_kbps: Option<u32>,
    pub wav_extensible_kind: Option<String>,
    pub duration_ms: Option<u64>,
    pub artwork_path: Option<String>,
    pub artwork_data_url: Option<String>,
    pub waveform_peaks_path: Option<String>,
    pub waveform_preview: Option<Vec<u8>>,
    pub waveform_color_data: Option<Vec<u8>>,
    pub created_at: String,
    pub updated_at: String,
    pub rekordbox_db_source: bool,
    /// Imported from a Mixxx library (`mixxxdb.sqlite`). Independent of
    /// `rekordbox_db_source`: a file in both libraries carries both flags.
    #[serde(default)]
    pub mixxx_db_source: bool,
    /// Authoritative: true when `file_path` falls under any known USB device
    /// root (current or previously pruned). Computed fresh on every read
    /// against the `usb_devices` registry -- not stored. See
    /// `untainted_usb_root_paths`/`browse_path_matches_root`.
    pub is_usb_path: bool,
    /// Authoritative: true when the track has its core analysis (waveform
    /// peaks path + BPM > 0 + duration > 0). Computed fresh on every read
    /// from the DB row -- not stored, no filesystem check, except on playlist
    /// tracks, which also need their local analysis files (as the export gate
    /// does; see `local_analysis_bundle_missing`). The frontend
    /// treats this as the only signal for "needs analysis". See
    /// `service::has_core_analysis_fields`.
    pub analysis_ready: bool,
    /// CDJ playback-compatibility judgement for this track's audio format,
    /// derived from `format_ext` + the technical fields above. Computed fresh
    /// on every read (see `service::format_compat`) -- the frontend renders the
    /// badge/tooltip and never re-derives the rule.
    #[serde(default)]
    pub format_compat: FormatCompat,
    /// Extra files of this track in other formats (AIFF / WAV / MP3 320),
    /// made by the user from the library. Computed on every read from the
    /// `track_transcodes` table (see `service::transcode`).
    #[serde(default)]
    pub transcodes: Vec<TrackTranscode>,
    /// In an app playlist read for a USB (`GetPlaylistTracksRequest::
    /// usb_root`): what export puts on that USB, when it isn't the track's
    /// own file (see `service::transcode_export`). The FMT column shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_file: Option<TrackExportFile>,
    /// In an app playlist page: where the track's file lives and whether it's
    /// available, for the row's icon (see `service::playlists::track_file_source`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_source: Option<TrackFileSource>,
}

/// The source icon on a playlist row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackFileSource {
    /// `"usb"`: on a USB stick (`Track::is_usb_path`). `"local"`: anywhere else.
    pub kind: String,
    /// Not available: on a USB other than the selected one, or the file is gone.
    pub missing: bool,
    pub tooltip: String,
}

/// What export puts on a USB for a track, when it isn't the track's own
/// file (see `transcode_export::ExportFile`), as the FMT column shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackExportFile {
    /// `"transcode"`: one of its transcodes goes. `"convert"`: export makes
    /// the transcode first. `"refused"`: it can't go (export is blocked).
    pub kind: String,
    /// The FMT badge: the format that goes (`"MP3"`); for `"refused"`, the
    /// track's own.
    pub badge: String,
    /// The badge's tooltip, naming the track's own file as the source.
    pub tooltip: String,
}

/// One transcode of a track: the same track as another file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackTranscode {
    /// `"aiff"`, `"wav"` or `"mp3"`.
    pub format: String,
    /// What the library shows, e.g. `"MP3 320"`.
    pub label: String,
    pub file_path: String,
    pub size_bytes: i64,
    /// The source file changed after the transcode was made.
    pub outdated: bool,
    /// Its line in the FMT column (`"MP3"`) and that line's tooltip.
    #[serde(default)]
    pub badge: String,
    #[serde(default)]
    pub tooltip: String,
    /// Export puts this one on the USB the track list was read for
    /// (`Track::export_file`): the FMT badge shows it, not a line of its own.
    #[serde(default)]
    pub exported: bool,
}

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
    /// Tracks that need no transcode in this format (not lossless, already
    /// this format, already up to date, file missing).
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetUsbAudioFormatsRequest {
    pub usb_root: String,
}

/// One Player format of a USB stick.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbAudioFormatSetting {
    /// `flac`, `alac`, `aiff`, `wav`, `mp3` or `aac`.
    pub format: String,
    /// What the USB panel shows (`FLAC`); ignored in a request.
    #[serde(default)]
    pub label: String,
    pub allowed: bool,
    /// Its box can't be changed (the last ticked one, or a read-only USB);
    /// ignored in a request.
    #[serde(default)]
    pub locked: bool,
    /// Why, when it's the last ticked one; ignored in a request.
    #[serde(default)]
    pub tooltip: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetUsbAudioFormatsRequest {
    pub usb_root: String,
    /// Every format once, in the order of preference, at least one allowed.
    pub formats: Vec<UsbAudioFormatSetting>,
}

/// The formats export puts on a USB stick (USB audio formats, `drive.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbAudioFormatsData {
    /// Every format, in the order of preference.
    pub formats: Vec<UsbAudioFormatSetting>,
    /// False on a read-only stick: the list can't be changed.
    pub writable: bool,
    /// The line next to the row's title.
    #[serde(default)]
    pub hint: String,
}

/// How a track's audio format is expected to behave on CDJ hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FormatCompatSeverity {
    /// No known playback problem.
    #[default]
    Ok,
    /// Plays as-is, but the source has a header quirk the export pipeline
    /// rewrites automatically (WAVE_FORMAT_EXTENSIBLE wrapping plain PCM).
    Autofix,
    /// May not play on CDJ hardware and there's nothing we can safely do.
    Warn,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatCompat {
    pub severity: FormatCompatSeverity,
    /// Human-readable reason, present whenever `severity` isn't `Ok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    /// The FMT badge's text: the file's extension in capitals, or "Unknown".
    #[serde(default)]
    pub badge: String,
    /// The badge's tooltip: the sample rate and bit depth or bitrate when
    /// known, then the warning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub source: String,
    pub last_exported_at: Option<String>,
    pub last_exported_usb_root: Option<String>,
    pub last_exported_track_count: Option<usize>,
    pub created_at: String,
    pub updated_at: String,
    /// The library an imported playlist came from, as shown ("Mixxx",
    /// "rekordbox"); derived from `playlists.import_source`.
    #[serde(default)]
    pub imported_from: Option<String>,
    /// The name the drive had when the playlist was exported to it, when it
    /// had one. Exports from before this was recorded have none (the mount
    /// path may since belong to another stick, so it isn't guessed).
    #[serde(default)]
    pub last_exported_usb_name: Option<String>,
    /// What the dialog asks before deleting the playlist.
    #[serde(default)]
    pub delete_confirm_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanLibraryRequest {
    /// The folders to scan; empty scans the library's switched-on folders.
    #[serde(default)]
    pub source_roots: Vec<String>,
    pub incremental: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanLibraryData {
    pub job_id: String,
    pub indexed: usize,
    pub updated: usize,
    pub removed: usize,
    #[serde(default)]
    pub not_found: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<WarningEntry>,
    /// The status line: for a folder scan the folder count and the tracks
    /// indexed, for a library import what it brought in.
    #[serde(default)]
    pub status_text: String,
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseSourceFilesRequest {
    pub query: String,
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<String>,
    /// Absolute start position in the filtered + sorted list; takes
    /// precedence over `cursor`. Lets a client jump straight to any part of
    /// the list (the track tables load the rows under the viewport).
    #[serde(default)]
    pub offset: Option<usize>,
    /// One of `title` | `artist` | `album` | `year` | `format` | `bpm` | `durationMs` |
    /// `key`. Absent ⇒ the natural file-path order. See `TrackSort`.
    #[serde(default)]
    pub sort_by: Option<String>,
    /// `asc` (default) | `desc`.
    #[serde(default)]
    pub sort_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseSourceFilesData {
    pub total: usize,
    pub items: Vec<Track>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
    #[serde(default)]
    pub total_duration_ms: u64,
    #[serde(default)]
    pub duration_unknown_count: usize,
    /// Whole list before the search filter -- see `track_list::list_totals`.
    #[serde(default)]
    pub list_track_count: usize,
    #[serde(default)]
    pub list_size_bytes: u64,
    /// "30 tracks · 412 MB", from the two values above.
    #[serde(default)]
    pub list_summary_text: String,
}

/// One configured library source folder, as its chip shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFolder {
    pub path: String,
    /// The chip's text: the path inside `parent` (`Kursa/`), else `path`.
    pub label: String,
    /// The nearest configured folder this one lies inside (through symlinks
    /// too). The list comes in tree order: each folder right after its
    /// parent's earlier children.
    pub parent: Option<String>,
    /// Switched on in the library filter (a missing folder keeps its switch).
    pub enabled: bool,
    /// Not a directory right now (unplugged drive, moved folder).
    pub missing: bool,
    /// Every track in the folder has its core analysis -- counted over the
    /// whole folder, whatever the search or the switch.
    pub fully_analyzed: bool,
}

/// The library filter, kept by the backend: the source folders in the order
/// they were added, each with its switch, and whether the imported rekordbox
/// / Mixxx libraries' tracks are shown. Every library listing reads it here.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySourcesData {
    pub folders: Vec<SourceFolder>,
    pub rekordbox_db_enabled: bool,
    pub mixxx_db_enabled: bool,
    /// A source has been set up at some point (a folder added or a library
    /// imported), even if none is left: the library then shows its table
    /// instead of the "add a folder" onboarding.
    pub ever_configured: bool,
    /// The text of the button that checks these folders and analyzes them.
    pub analyze_button_label: String,
    /// The status line while that button's scan runs.
    pub analyze_scanning_status: String,
    /// The status line when a track to analyze isn't in the library.
    pub track_not_in_library_status: String,
    /// The status line after a source's switch changes: how many of the
    /// folders and imported libraries are switched on.
    pub filter_status_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddSourceFoldersRequest {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddSourceFoldersData {
    pub sources: LibrarySourcesData,
    /// The paths that weren't configured yet, in the order given.
    pub added: Vec<String>,
    /// The status line when nothing was added; otherwise the scan that
    /// indexes the added folders answers with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveSourceFolderRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveSourceFolderData {
    pub sources: LibrarySourcesData,
    /// Tracks removed from the library with the folder.
    pub removed: usize,
    pub status_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSourceFolderEnabledRequest {
    pub path: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExternalLibrary {
    Rekordbox,
    Mixxx,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetExternalLibraryEnabledRequest {
    pub library: ExternalLibrary,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterializeSourceTrackRequest {
    pub file_path: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub track_number: Option<u32>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub file_size_bytes: Option<i64>,
    #[serde(default)]
    pub format_ext: Option<String>,
    #[serde(default)]
    pub sample_rate_hz: Option<u32>,
    #[serde(default)]
    pub bit_depth: Option<u8>,
    #[serde(default)]
    pub bitrate_kbps: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterializeSourceTrackData {
    pub track_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveTrackIdentityRequest {
    #[serde(default)]
    pub track_id: Option<String>,
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
    pub track_number: Option<u32>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub format_ext: Option<String>,
    #[serde(default)]
    pub sample_rate_hz: Option<u32>,
    #[serde(default)]
    pub bit_depth: Option<u8>,
    #[serde(default)]
    pub bitrate_kbps: Option<u32>,
    #[serde(default)]
    pub usb_root: Option<String>,
    #[serde(default)]
    pub usb_root_valid: bool,
    #[serde(default)]
    pub usb_analysis_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveTrackIdentityData {
    pub track_id: Option<String>,
    pub resolved_by: ResolvedBy,
    pub materialized: bool,
    /// When `materialized`: the new library row of the listed file, shown in
    /// place of the file's row (whose id was its path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelocateSourceRootRequest {
    pub old_root: String,
    pub new_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelocateSourceRootData {
    pub old_root: String,
    pub new_root: String,
    pub matched: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub missing_at_new_root: usize,
    pub conflicts: usize,
    /// Tracks still pointing at the old folder: `missing_at_new_root` plus
    /// `conflicts`.
    pub unresolved: usize,
    /// The source folders after the move: the old folder is replaced by the
    /// new one, or kept beside it while some of its tracks couldn't move.
    pub sources: LibrarySourcesData,
    pub status_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTracksByIdsRequest {
    pub track_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTracksByIdsData {
    pub items: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePlaylistRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePlaylistData {
    pub playlist_id: String,
    pub name: String,
    pub status_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePlaylistRequest {
    pub playlist_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePlaylistData {
    pub playlist_id: String,
    pub name: String,
}

/// The full new sidebar order of the local playlists, top first. Must name
/// every playlist exactly once.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderPlaylistsRequest {
    pub ordered_playlist_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderPlaylistsData {
    pub reordered: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePlaylistRequest {
    pub playlist_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePlaylistData {
    pub playlist_id: String,
    pub deleted: bool,
    pub status_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListPlaylistsData {
    pub items: Vec<Playlist>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPlaylistTracksRequest {
    pub playlist_id: String,
    /// Substring match on title/artist/album. Empty ⇒ no filter.
    #[serde(default)]
    pub query: String,
    /// See `BrowseSourceFilesRequest::sort_by`. Absent ⇒ playlist position order.
    #[serde(default)]
    pub sort_by: Option<String>,
    #[serde(default)]
    pub sort_dir: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    /// Absolute start position in the filtered + sorted list; takes
    /// precedence over `cursor`. Lets a client jump straight to any part of
    /// the list (the track tables load the rows under the viewport).
    #[serde(default)]
    pub offset: Option<usize>,
    /// `0` ⇒ unpaginated (the whole playlist), for backward compatibility.
    #[serde(default)]
    pub limit: usize,
    /// The selected USB: each returned track then carries
    /// `Track::export_file` for it.
    #[serde(default)]
    pub usb_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPlaylistExportSpaceRequest {
    pub playlist_id: String,
    /// The connected USB.
    pub usb_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPlaylistExportSpaceData {
    /// `None` when the estimate can't be made (no staged PDB, unknown
    /// playlist): the heading just leaves it out.
    pub export_space: Option<PlaylistExportSpace>,
}

/// What exporting a playlist to the connected USB would newly copy, against
/// the drive's free space. Worked out from recorded data only: audio sizes
/// from the app DB, what is already on the USB from the locally staged
/// databases' track lists, and the free space from one filesystem query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistExportSpace {
    /// Audio of the tracks not on the USB yet, by their recorded sizes.
    pub bytes_to_copy: u64,
    /// How many of the playlist's tracks aren't on the USB yet.
    pub tracks_to_copy: usize,
    /// `None` when the free space can't be read.
    pub available_bytes: Option<u64>,
    /// False when the copy plus the export's headroom exceeds the free space,
    /// the same rule the export's own preflight blocks on.
    pub fits: bool,
    /// The playlist heading's line about this estimate.
    #[serde(default)]
    pub summary_text: String,
    /// What the USB audio formats mean for the export, for the playlist
    /// heading: the tracks it converts first ("2 tracks will be converted to
    /// MP3 320 on export") and the lossy ones it refuses (see
    /// `transcode_export`). `None` when it does neither.
    #[serde(default)]
    pub format_notice: Option<String>,
    /// The notice says the export is blocked (a track is refused).
    #[serde(default)]
    pub format_notice_blocks: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPlaylistTracksData {
    pub playlist_id: String,
    pub items: Vec<Track>,
    /// Count of tracks matching `query` (before pagination).
    #[serde(default)]
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub has_more: bool,
    /// Sum over the whole filtered playlist (not just the returned page).
    #[serde(default)]
    pub total_duration_ms: u64,
    #[serde(default)]
    pub duration_unknown_count: usize,
    /// Tracks in the whole filtered playlist the backend flags as not yet
    /// analysis-ready (drives the "Analyze Missing Tracks (N)" button, which
    /// can't count them from a single loaded page).
    #[serde(default)]
    pub unanalyzed_count: usize,
    /// The "Analyze Missing Tracks (N)" button's label and tooltip.
    #[serde(default)]
    pub analyze_missing_label: String,
    #[serde(default)]
    pub analyze_missing_tooltip: String,
    /// Whole list before the search filter -- see `track_list::list_totals`.
    #[serde(default)]
    pub list_track_count: usize,
    #[serde(default)]
    pub list_size_bytes: u64,
    /// "30 tracks · 412 MB", from the two values above.
    #[serde(default)]
    pub list_summary_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTracksToPlaylistRequest {
    pub playlist_id: String,
    pub track_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTracksToPlaylistData {
    pub playlist_id: String,
    pub added: usize,
    pub skipped: usize,
    /// "Added 2 tracks (skipped 0) to Warmup".
    #[serde(default)]
    pub status_text: String,
}

/// The ticked tracks of one track-list view, for the selection bar's total
/// time. The library names its list by its filter, the others by the shown
/// list's `id`; `track_ids` are the rows' `id`s. Ids no longer in the list add
/// nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "view", rename_all = "camelCase")]
pub enum GetSelectionDurationRequest {
    #[serde(rename_all = "camelCase")]
    Library { track_ids: Vec<String> },
    #[serde(rename_all = "camelCase")]
    Playlist { id: String, track_ids: Vec<String> },
    #[serde(rename_all = "camelCase")]
    UsbPlaylist {
        #[serde(default)]
        usb_root: Option<String>,
        id: String,
        track_ids: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    UsbHistory {
        #[serde(default)]
        usb_root: Option<String>,
        id: String,
        track_ids: Vec<String>,
    },
}

/// Summed like the list's own "Total time": `total_duration_ms` over the
/// `track_count` selected tracks found in the list, `duration_unknown_count`
/// of them without a known length.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetSelectionDurationData {
    pub total_duration_ms: u64,
    pub duration_unknown_count: usize,
    pub track_count: usize,
    /// The time's tooltip when some selected tracks have no length yet
    /// ("2 of the selected tracks have no length yet"); empty when none.
    #[serde(default)]
    pub duration_unknown_text: String,
}

/// Every local track id that matches the current library filter -- so
/// "Select all" doesn't have to page the whole list into the frontend just to
/// enumerate ids.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMatchingTrackIdsRequest {
    #[serde(default)]
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMatchingTrackIdsData {
    pub track_ids: Vec<String>,
    pub total: usize,
}

/// Add a library selection to a playlist entirely server-side: the frontend
/// sends the search plus either the ticked ids or `all_matching`, and
/// the backend enumerates, materializes any browse-only rows, and appends --
/// so a selection that spans pages the user has scrolled past is never lost.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddLibrarySelectionToPlaylistRequest {
    pub playlist_id: String,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub track_ids: Vec<String>,
    #[serde(default)]
    pub all_matching: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTrackCandidate {
    #[serde(default, alias = "id")]
    pub track_id: Option<String>,
    #[serde(default)]
    pub local_track_id: Option<String>,
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
    pub track_number: Option<u32>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub format_ext: Option<String>,
    #[serde(default)]
    pub sample_rate_hz: Option<u32>,
    #[serde(default)]
    pub bit_depth: Option<u8>,
    #[serde(default)]
    pub bitrate_kbps: Option<u32>,
    #[serde(default)]
    pub usb_root: Option<String>,
    #[serde(default)]
    pub usb_root_valid: bool,
    #[serde(default)]
    pub usb_analysis_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTrackCandidateResolution {
    #[serde(default)]
    pub previous_id: Option<String>,
    #[serde(default)]
    pub track_id: Option<String>,
    pub resolved_by: ResolvedBy,
    pub materialized: bool,
    /// As `ResolveTrackIdentityData::track`: a listed file's new library row,
    /// shown in place of the row whose id was `previous_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTrackCandidatesToPlaylistRequest {
    pub playlist_id: String,
    #[serde(default)]
    pub tracks: Vec<AddTrackCandidate>,
    #[serde(default)]
    pub usb_root: Option<String>,
    #[serde(default)]
    pub usb_root_valid: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTrackCandidatesToPlaylistData {
    pub playlist_id: String,
    pub requested: usize,
    pub resolved: usize,
    pub unresolved: usize,
    pub added: usize,
    pub skipped: usize,
    #[serde(default)]
    pub resolutions: Vec<AddTrackCandidateResolution>,
    /// As `AddTracksToPlaylistData::status_text`; empty when nothing resolved.
    #[serde(default)]
    pub status_text: String,
}

/// The selection bar's "Add to…": the same selection as
/// `AddLibrarySelectionToPlaylistRequest`, added to every playlist in
/// `playlist_ids`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddLibrarySelectionToPlaylistsRequest {
    pub playlist_ids: Vec<String>,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub track_ids: Vec<String>,
}

/// The selection bar's "Add to…" for USB rows: the same candidates as
/// `AddTrackCandidatesToPlaylistRequest`, added to every playlist in
/// `playlist_ids`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTrackCandidatesToPlaylistsRequest {
    pub playlist_ids: Vec<String>,
    #[serde(default)]
    pub tracks: Vec<AddTrackCandidate>,
    #[serde(default)]
    pub usb_root: Option<String>,
    #[serde(default)]
    pub usb_root_valid: bool,
}

/// One add to several playlists: the selection is resolved to local tracks
/// once (`resolved` of `requested`; `resolutions` per candidate for USB rows),
/// then `items` says what each playlist took, in request order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTracksToPlaylistsData {
    pub requested: usize,
    pub resolved: usize,
    pub unresolved: usize,
    pub added: usize,
    pub skipped: usize,
    pub items: Vec<AddTracksToPlaylistData>,
    #[serde(default)]
    pub resolutions: Vec<AddTrackCandidateResolution>,
    /// "Added 2 tracks (skipped 0) to Warmup, Peak", the playlists in request
    /// order; empty when no track resolved.
    #[serde(default)]
    pub status_text: String,
}

/// The "Add to…" picker: which playlists already hold the selection. One
/// entry per selected track, its local id, or `None` for a track with no
/// local row yet (a USB row never added, a library file not indexed yet).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPlaylistMembershipRequest {
    #[serde(default)]
    pub track_ids: Vec<Option<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaylistMembershipState {
    None,
    Some,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistMembership {
    pub playlist_id: String,
    pub name: String,
    /// How many of the selected tracks the playlist already holds.
    pub contained: usize,
    pub state: PlaylistMembershipState,
    /// The note beside it: "Already in", "1 of 2 in", or empty.
    pub note_text: String,
}

/// Every local playlist in sidebar order, with how much of the selection
/// (`selected` tracks) it holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPlaylistMembershipData {
    pub selected: usize,
    pub items: Vec<PlaylistMembership>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveTracksFromPlaylistRequest {
    pub playlist_id: String,
    pub track_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveTracksFromPlaylistData {
    pub playlist_id: String,
    pub removed: usize,
    pub status_text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderPlaylistTracksRequest {
    pub playlist_id: String,
    /// Explicit-order mode: the complete new track order. Must match the
    /// playlist's current track set exactly.
    #[serde(default)]
    pub ordered_track_ids: Vec<String>,
    /// Sort-commit mode: reorder the whole playlist by this column (same
    /// comparator as `get_playlist_tracks`), then persist it as the new
    /// position order. Used when a client column sort is committed on
    /// navigate-away / export.
    #[serde(default)]
    pub sort_by: Option<String>,
    #[serde(default)]
    pub sort_dir: Option<String>,
    /// Single-move mode (drag-reorder): move this track to immediately before
    /// `before_track_id` (or to the end when that is absent). Lets a paginated
    /// client reorder without holding the whole list.
    #[serde(default)]
    pub move_track_id: Option<String>,
    #[serde(default)]
    pub before_track_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderPlaylistTracksData {
    pub playlist_id: String,
    pub reordered: usize,
}

/// `get_ui_texts`: every `ui-` text of the catalog, in the current language, for the text
/// JS and `index.html` show. Sent once at startup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetUiTextsData {
    /// The language code, for `Intl.PluralRules` in `tCount`.
    pub language: String,
    pub texts: std::collections::BTreeMap<String, UiText>,
}

/// One `ui-` text: a string with `{name}` placeholders, or a counted text's forms by plural
/// category (`{ "one": "{count} selected", "other": "{count} selected" }`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UiText {
    Text(String),
    Counted(std::collections::BTreeMap<String, String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFrontendSettingsData {
    pub values: HashMap<String, String>,
    pub node_available: bool,
    pub essentia_installed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetFrontendSettingRequest {
    pub key: String,
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetFrontendSettingData {
    pub saved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidateUsbRootRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidateUsbRootData {
    pub valid: bool,
    pub normalized_root: Option<String>,
    // Backend only (CLI tools, tests): the UI gets `can_initialize` and the text.
    #[serde(default, skip_serializing)]
    pub has_write_access: bool,
    #[serde(default, skip_serializing)]
    pub has_vendor_root: bool,
    #[serde(default, skip_serializing)]
    pub has_contents: bool,
    #[serde(default, skip_serializing)]
    pub has_pdb: bool,
    #[serde(default, skip_serializing)]
    pub has_edb: bool,
    /// The folder isn't a usable USB yet, but the app can set it up.
    #[serde(default)]
    pub can_initialize: bool,
    /// The status line after selecting the folder.
    #[serde(default)]
    pub status_text: String,
    /// Why the folder needs setting up, or can't be. Empty for a usable USB.
    #[serde(default)]
    pub init_hint_text: String,
    pub warnings: Vec<WarningEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeUsbRequest {
    pub usb_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeUsbData {
    pub path: String,
    pub created_dirs: Vec<String>,
    /// The status line after setting up the USB.
    #[serde(default)]
    pub status_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchUsbPlaylistsRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
}

/// What a `run_data_migrations` pass did: the ids it completed, and those
/// that will run again on the next launch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDataMigrationsData {
    pub ran: Vec<String>,
    pub retry_later: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeUsbPlaceholderTracksData {
    pub merged: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbDeviceSummary {
    pub id: String,
    pub root_path: String,
    #[serde(default)]
    pub label: Option<String>,
    pub mounted: bool,
    pub first_seen_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListUsbDevicesData {
    pub items: Vec<UsbDeviceSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PruneUsbDeviceRequest {
    pub id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PruneUsbDeviceData {
    pub pruned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetUsbDeviceNameRequest {
    pub usb_root: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetUsbDeviceNameData {
    pub name: Option<String>,
    /// Best-effort guess at a name, drawn from the OS's own filesystem label
    /// when the root is on a real removable USB device. Only populated when
    /// `name` is `None` -- purely a prefill suggestion for the naming
    /// prompt, never applied automatically.
    #[serde(default)]
    pub suggested_name: Option<String>,
    /// Set when the drive carries a name that belongs to another drive (it's
    /// a copy, or got that drive's name): the name it carries. The drive
    /// counts as unnamed until it's given its own name, or this one again
    /// if it is that drive.
    #[serde(default)]
    pub name_taken_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetUsbDeviceNameRequest {
    pub usb_root: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetUsbDeviceNameData {
    /// The name as saved (trimmed).
    pub name: String,
    /// False when the drive already had exactly this name.
    pub updated: bool,
    /// The status line after a rename from the header.
    pub status_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListUsbBackupsRequest {
    pub usb_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbBackupFile {
    pub stem: String,
    pub filename: String,
    pub size_bytes: u64,
}

/// One backup "event": the PDB and eDB snapshots taken together in a single
/// `backup_usb_databases` call, sharing a timestamp. Always presented and
/// acted on (restore/delete) as a unit, never as separate PDB/eDB rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbBackupEntry {
    pub timestamp: String,
    /// "usb" or "cache"
    pub location: String,
    pub size_bytes: u64,
    pub files: Vec<UsbBackupFile>,
    /// The timestamp as the Backups list shows it.
    pub time_text: String,
    /// Why it was taken, as the Backups list shows it.
    pub reason_text: String,
    /// "2.5 MB · On USB · 4 playlists".
    pub detail_text: String,
    /// The restore dialog's question.
    pub restore_confirm_text: String,
    /// The delete dialog's question.
    pub delete_confirm_text: String,
    /// Why this backup was taken (e.g. "Before export"), or `None` for
    /// backups made before this field existed.
    pub reason: Option<String>,
    /// Playlist count on the drive at backup time, or `None` for backups
    /// made before this field existed.
    pub playlist_count: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListUsbBackupsData {
    pub items: Vec<UsbBackupEntry>,
    /// "6 backups", above the list.
    pub summary_text: String,
    /// What the list shows with no items.
    pub empty_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreUsbBackupRequest {
    pub usb_root: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreUsbBackupData {
    pub restored: bool,
    /// The status line after the restore.
    #[serde(default)]
    pub status_text: String,
    /// Computed from the restored databases. Empty if that failed (logged),
    /// because the restore itself has already succeeded.
    #[serde(default)]
    pub playlist_usb_export_status: Vec<PlaylistUsbExportStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteUsbBackupRequest {
    pub usb_root: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteUsbBackupData {
    pub deleted: bool,
    /// The status line after the delete.
    #[serde(default)]
    pub status_text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbTrack {
    pub id: String,
    #[serde(default)]
    pub local_track_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    /// Release year from the PDB track row.
    #[serde(default)]
    pub year: Option<u32>,
    /// Genre from the PDB track row.
    #[serde(default)]
    pub genre: Option<String>,
    pub track_number: Option<u32>,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    /// Same as `Track::key_display`; set when the row is hydrated for the UI.
    #[serde(default)]
    pub key_display: Option<String>,
    /// Same as `Track::key_color`.
    #[serde(default)]
    pub key_color: Option<u8>,
    pub file_path: String,
    /// Lowercase file extension, derived from the PDB track path so the
    /// frontend never has to infer the format badge (mirrors `Track.format_ext`).
    #[serde(default)]
    pub format_ext: Option<String>,
    #[serde(default)]
    pub usb_media_path: Option<String>,
    pub artwork_path: Option<String>,
    pub artwork_data_url: Option<String>,
    pub waveform_peaks_path: Option<String>,
    pub usb_analysis_path: Option<String>,
    #[serde(default)]
    pub usb_analysis_path_raw: Option<String>,
    pub waveform_preview: Option<Vec<u8>>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// File size as recorded at export/analysis time (the PDB row's, or the
    /// eDB `content.fileSize`), not a fresh stat of the mounted file. Used to gate fingerprint
    /// matching against local tracks so same-length variants (e.g. Clean vs
    /// Explicit edits) don't get silently merged.
    #[serde(default)]
    pub file_size_bytes: Option<i64>,
    /// CDJ format-compatibility badge (mirrors `Track.format_compat`). USB rows
    /// only carry `format_ext`, so this flags an extension players don't read,
    /// or a format the stick's USB audio formats leave unticked. Filled in during
    /// page hydration -- see `service::usb::hydrate_usb_track_in_place`.
    #[serde(default)]
    pub format_compat: FormatCompat,
    /// True when the row is still missing display data (waveform bytes,
    /// artwork, BPM, or key) that an `inspect_usb_track` deep-resolve could
    /// fill in. Set during page hydration; false on the row an inspect
    /// returns.
    #[serde(default)]
    pub needs_hydration: bool,
}

impl UsbTrack {
    pub fn identity_path(&self) -> &str {
        self.usb_media_path
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.file_path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbPlaylist {
    pub id: String,
    pub name: String,
    pub source: String,
    pub track_count: usize,
    /// The status line when the playlist is clicked.
    pub selected_text: String,
    /// What the remove dialog asks.
    pub remove_confirm_text: String,
    /// Backend only: the UI reads pages with `fetch_usb_playlist_tracks`.
    #[serde(skip)]
    pub tracks: Vec<UsbTrack>,
    /// Sum of `duration_ms` over tracks where it's known (>0). Computed once
    /// server-side from the full track list so the frontend never needs to
    /// sum durations itself, regardless of how much of the list it has
    /// rendered/hydrated.
    pub total_duration_ms: u64,
    /// How many of `tracks` have no known duration: the "N without length"
    /// count.
    pub duration_unknown_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbImportStats {
    pub indexed_tracks: usize,
    pub playlist_referenced_tracks: usize,
    pub playlist_entries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WarningEntry {
    pub level: String,
    pub code: String,
    pub message: String,
    pub source: String,
}

/// For one local playlist, whether it shares a name with a playlist already
/// on the connected USB, and -- combining that with the user's current
/// export sync mode -- whether reordering its tracks right now would have no
/// visible effect on the next export (an additive export never rewrites the
/// order of entries already on the device). Computed once server-side (see
/// `service::export::compute_playlist_usb_export_status`) so the frontend
/// never re-derives this business rule from raw state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistUsbExportStatus {
    pub playlist_id: String,
    pub playlist_name: String,
    /// A valid USB is selected, so `same_name_exists_on_usb` means something.
    #[serde(default)]
    pub usb_selected: bool,
    pub same_name_exists_on_usb: bool,
    pub locks_reorder: bool,
    /// The sidebar's mark, by its label: "On this USB" while a valid USB is
    /// selected and holds a playlist of this name, "Exported to USB" with none
    /// selected and the playlist exported before. `None`: no mark.
    #[serde(default)]
    pub mark_label: Option<String>,
    /// The export button's label: "Append to (<name>) on USB: (<dir>)" (additive
    /// export onto a same-named playlist), "Export to USB: <dir>", or with no
    /// valid USB selected "Select USB first". Built server-side (see
    /// `service::export::compute_playlist_usb_export_status`).
    pub export_button_text: String,
    /// Tooltip paired with `export_button_text`.
    pub export_button_title: String,
    /// The sortable columns' tooltip while `locks_reorder`; `None` otherwise.
    #[serde(default)]
    pub reorder_lock_tooltip: Option<String>,
}

/// Request for `refresh_playlist_export_status` -- a cheap recompute of every
/// playlist's `PlaylistUsbExportStatus` (staged PDB/eDB only, no USB access)
/// for when the export sync-mode setting changes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshPlaylistExportStatusRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshPlaylistExportStatusData {
    pub playlist_usb_export_status: Vec<PlaylistUsbExportStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchUsbPlaylistsData {
    pub items: Vec<UsbPlaylist>,
    pub stats: UsbImportStats,
    /// The panel's totals ("3 playlists, 30 tracks"). The track figure is the sum
    /// over `items` (post name-dedupe), not a distinct count, so a track in two
    /// playlists counts twice.
    pub summary_text: String,
    /// The status line after the read.
    pub status_text: String,
    /// The progress footer's last line after the read.
    pub done_text: String,
    pub warnings: Vec<WarningEntry>,
    /// The warn/error entries among `warnings`, for the status line's
    /// "(N warnings)" (`logging::count_warnings`).
    #[serde(default)]
    pub warning_count: usize,
    pub playlist_usb_export_status: Vec<PlaylistUsbExportStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveUsbPlaylistRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    #[serde(default)]
    pub playlist_id: Option<String>,
    pub playlist_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveUsbPlaylistData {
    pub playlist_name: String,
    /// The status line after the removal.
    #[serde(default)]
    pub status_text: String,
    pub removed_from_edb: usize,
    pub removed_from_pdb: usize,
    pub tracks_removed: usize,
    pub files_deleted: usize,
    pub tracks_kept_shared: usize,
    pub warnings: Vec<WarningEntry>,
    /// The warn/error entries among `warnings`, for the status line's
    /// "(N warnings)" (`logging::count_warnings`).
    #[serde(default)]
    pub warning_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderUsbPlaylistsRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    pub ordered_playlist_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderUsbPlaylistsData {
    pub reordered: usize,
    /// The status line after the reorder.
    #[serde(default)]
    pub status_text: String,
    pub warnings: Vec<WarningEntry>,
    /// The warn/error entries among `warnings`, for the status line's
    /// "(N warnings)" (`logging::count_warnings`).
    #[serde(default)]
    pub warning_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchUsbHistoriesRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbHistory {
    pub id: String,
    pub name: String,
    pub created_at: Option<String>,
    /// False for a session with no tracks.
    pub can_export_tracklist: bool,
    pub track_count: usize,
    /// Backend only: the UI reads pages with `fetch_usb_history_tracks`.
    #[serde(skip)]
    pub tracks: Vec<UsbTrack>,
    /// See `UsbPlaylist::total_duration_ms`.
    pub total_duration_ms: u64,
    /// See `UsbPlaylist::duration_unknown_count`.
    pub duration_unknown_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetUsbHistoryTracklistOptionsRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetUsbHistoryTracklistOptionsData {
    /// The Export Tracklist dialog's start-track labels, in session order.
    pub start_tracks: Vec<String>,
}

/// Where a tracklist line shows its running time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TracklistTimeMode {
    Off,
    Before,
    After,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildUsbHistoryTracklistRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    pub id: String,
    pub start_index: usize,
    pub time_mode: TracklistTimeMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildUsbHistoryTracklistData {
    pub suggested_file_name: String,
    pub contents: String,
    /// The status line after the file is saved.
    pub exported_text: String,
    /// The status line if the save dialog is dismissed.
    pub cancelled_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbHistoryCounts {
    pub imported_playlists: usize,
    pub imported_tracks: usize,
    pub pdb_t11_playlists: usize,
    pub pdb_t12_entries: usize,
    pub pdb_t17_playlists: usize,
    pub pdb_t18_entries: usize,
    pub edb_history_rows: usize,
    pub edb_history_content_rows: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchUsbHistoriesData {
    pub items: Vec<UsbHistory>,
    pub counts: UsbHistoryCounts,
    /// The panel's totals ("2 sessions, 9 tracks").
    #[serde(default)]
    pub summary_text: String,
    /// The status line after the read.
    #[serde(default)]
    pub status_text: String,
    pub warnings: Vec<WarningEntry>,
    /// The warn/error entries among `warnings`, for the status line's
    /// "(N warnings)" (`logging::count_warnings`).
    #[serde(default)]
    pub warning_count: usize,
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
    pub source_label: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectUsbTrackRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    pub track_id: String,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectUsbTrackData {
    pub source: String,
    pub track: UsbTrack,
    pub warnings: Vec<WarningEntry>,
}

/// One paginated/searched/sorted page of a USB playlist or history session,
/// with waveform-preview bytes + artwork data URLs already hydrated server-side.
/// See the "Paginated track lists" envelope in `docs/COMMANDS.md`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchUsbTracksRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    /// `UsbPlaylist.id` or `UsbHistory.id`, depending on the command.
    pub id: String,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub sort_by: Option<String>,
    #[serde(default)]
    pub sort_dir: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    /// Absolute start position in the filtered + sorted list; takes
    /// precedence over `cursor`. Lets a client jump straight to any part of
    /// the list (the track tables load the rows under the viewport).
    #[serde(default)]
    pub offset: Option<usize>,
    /// `0` ⇒ the whole (filtered) list.
    #[serde(default)]
    pub limit: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchUsbTracksData {
    pub items: Vec<UsbTrack>,
    /// Count matching `query`, before pagination.
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
    /// Sum over the whole filtered list, not just the page.
    pub total_duration_ms: u64,
    pub duration_unknown_count: usize,
    /// Whole playlist/history before the search filter -- see
    /// `track_list::list_totals`.
    pub list_track_count: usize,
    pub list_size_bytes: u64,
    /// "30 tracks · 412 MB", from the two values above.
    pub list_summary_text: String,
    pub warnings: Vec<WarningEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeNewTracksRequest {
    #[serde(default)]
    pub track_ids: Vec<String>,
    /// When `track_ids` is empty: restrict auto-selection to the tracks in this
    /// playlist that still need core analysis (over the whole playlist, not a
    /// page). Ignored when `track_ids` is non-empty.
    #[serde(default)]
    pub playlist_id: Option<String>,
    /// When `track_ids` is empty and `playlist_id` is unset: restrict
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportToUsbOptions {
    pub include_artwork: bool,
    pub include_analysis: bool,
    pub prune_stale: bool,
    pub backup_before_export: bool,
}

impl Default for ExportToUsbOptions {
    fn default() -> Self {
        Self {
            include_artwork: true,
            include_analysis: true,
            prune_stale: true,
            backup_before_export: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportToUsbRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    pub playlist_id: String,
    #[serde(default)]
    pub options: Option<ExportToUsbOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportToUsbData {
    pub job_id: String,
    pub playlist_id: String,
    pub playlist_name: String,
    pub usb_root: String,
    pub exported_tracks: usize,
    pub skipped_tracks: usize,
    pub exported_artworks: usize,
    pub exported_analysis_files: usize,
    /// The status line after the export.
    #[serde(default)]
    pub status_text: String,
    pub warnings: Vec<WarningEntry>,
    /// The warn/error entries among `warnings`, for the status line's
    /// "(N warnings)" (`logging::count_warnings`).
    #[serde(default)]
    pub warning_count: usize,
}

// --- USB Diagnostics ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUsbDiagnosticsRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagStatus {
    Pass,
    Warn,
    Fail,
}

impl DiagStatus {
    pub fn worst(a: &DiagStatus, b: &DiagStatus) -> DiagStatus {
        match (a, b) {
            (DiagStatus::Fail, _) | (_, DiagStatus::Fail) => DiagStatus::Fail,
            (DiagStatus::Warn, _) | (_, DiagStatus::Warn) => DiagStatus::Warn,
            _ => DiagStatus::Pass,
        }
    }

    pub fn worst_of(statuses: &[&DiagStatus]) -> DiagStatus {
        let mut result = DiagStatus::Pass;
        for s in statuses {
            result = DiagStatus::worst(&result, s);
        }
        result
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagCountsSummary {
    pub contents_count: usize,
    pub indexed_count: usize,
    pub mismatch_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagCheck {
    /// The catalog key of `label`: what the report's layout and overview match
    /// a check on, so they never compare translated text. Empty for a check
    /// named by data (a playlist's row).
    #[serde(skip)]
    pub key: &'static str,
    pub label: String,
    pub status: DiagStatus,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

impl DiagCheck {
    pub fn new(
        label: crate::i18n::Text,
        status: DiagStatus,
        detail: impl std::fmt::Display,
    ) -> Self {
        Self {
            key: label.key(),
            label: label.render(),
            status,
            detail: detail.to_string(),
            link: None,
        }
    }

    /// A check named by data, such as a playlist's name.
    pub fn named(label: String, status: DiagStatus, detail: impl std::fmt::Display) -> Self {
        Self {
            key: "",
            label,
            status,
            detail: detail.to_string(),
            link: None,
        }
    }

    pub fn with_link(mut self, link: impl Into<String>) -> Self {
        self.link = Some(link.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagSummaryRow {
    pub label: String,
    pub status: DiagStatus,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagSection {
    pub title: String,
    pub status: DiagStatus,
    pub checks: Vec<DiagCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts: Option<DiagCountsSummary>,
}

/// One plain-language row of the diagnostics overview. `status: None` means
/// not applicable (e.g. Engine DJ library not set up).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagOverviewItem {
    pub title: String,
    pub status: Option<DiagStatus>,
    pub detail: String,
}

/// Diagnostics sections shown side by side under one heading.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagSectionGroup {
    pub title: String,
    pub sections: Vec<DiagSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistDiagEntry {
    pub name: String,
    pub total_entries: usize,
    pub resolved_entries: usize,
    pub resolution_rate: f64,
    pub status: DiagStatus,
    #[serde(default)]
    pub pdb_entries: usize,
    #[serde(default)]
    pub edb_entries: usize,
    #[serde(default)]
    pub matched_entries: usize,
    #[serde(default)]
    pub pdb_match_rate: f64,
    #[serde(default)]
    pub edb_match_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUsbDiagnosticsData {
    pub overall_status: DiagStatus,
    /// Plain-language summary the desktop UI shows first
    /// (`service::diagnostics::diagnostics_overview`).
    pub overview: Vec<DiagOverviewItem>,
    /// The technical report: the stage sections below regrouped into display
    /// order (`service::diagnostics::regroup_sections`).
    pub section_groups: Vec<DiagSectionGroup>,
    // Per-stage sections, as computed. Backend-only (CLI tool, tests); the UI
    // gets the same checks through `sections`.
    #[serde(skip_serializing)]
    pub pdb_integrity: DiagSection,
    #[serde(skip_serializing)]
    pub edb_access: DiagSection,
    #[serde(skip_serializing)]
    pub contents_integrity: DiagSection,
    #[serde(skip_serializing)]
    pub analysis_integrity: DiagSection,
    #[serde(skip_serializing)]
    pub playlist_resolution: DiagSection,
    pub playlist_details: Vec<PlaylistDiagEntry>,
    /// Raw player-counter table signals -- consumed by the `run_usb_diagnostics`
    /// CLI debug tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdj_counter_snapshot: Option<PlayerCounterSnapshot>,
    /// The player-counter snapshot rendered as a `DiagSection`.
    #[serde(default, skip_serializing)]
    pub cdj_counter_section: Option<DiagSection>,
    /// Present when the USB has an Engine DJ library (`Engine Library/Database2`).
    #[serde(default, skip_serializing)]
    pub engine_library_section: Option<DiagSection>,
    pub warnings: Vec<WarningEntry>,
    pub duration_ms: u64,
    /// The status line after the run.
    #[serde(default)]
    pub status_text: String,
    /// "Completed in 370ms", in the report's header.
    #[serde(default)]
    pub duration_text: String,
    /// "PASS", on the report's badge.
    #[serde(default)]
    pub status_label: String,
    /// "USB health: good", the health dots' tooltip.
    #[serde(default)]
    pub health_text: String,
    pub playlist_usb_export_status: Vec<PlaylistUsbExportStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerCounterSnapshot {
    pub playlist_count_candidate: usize,
    pub song_count_candidate: usize,
    pub confidence: String,
    pub shape_mode: String,
    pub baseline_init_like: bool,
    /// The PDB has tracks but t19 does not count them (still the init
    /// template, or an app-written ring at another count), so older players
    /// show 0 or 1 songs (repaired by `repair_pdb_t19_template_state`).
    #[serde(default)]
    pub t19_template_state: bool,
    pub t00_tracks: usize,
    pub t08_entries: usize,
    pub t11: PlayerTableSignal,
    pub t12: PlayerTableSignal,
    pub t17: PlayerTableSignal,
    pub t18: PlayerTableSignal,
    pub t19: PlayerTableSignal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerTableSignal {
    pub table_type: u32,
    pub ec: u32,
    pub first: u32,
    pub last: u32,
    pub chain_len: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_page: Option<PlayerPageSignal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerPageSignal {
    pub page: u32,
    pub seq: u32,
    pub nrs: u8,
    pub u3: u8,
    pub num_rl: u16,
    pub rowpf0: u16,
    pub tranrf0: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUsbParityReportRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbParityPlaylistDetail {
    pub name: String,
    pub pdb_tracks: usize,
    pub edb_tracks: usize,
    pub matched_tracks: usize,
    pub only_in_pdb: usize,
    pub only_in_edb: usize,
    pub order_mismatch: bool,
    #[serde(default)]
    pub path_mismatch_tracks: usize,
    #[serde(default)]
    pub dictionary_id_issue_tracks: usize,
    #[serde(default)]
    pub playlist_id_match: bool,
    #[serde(default)]
    pub sort_order_match: bool,
    #[serde(default)]
    pub parent_match: Option<bool>,
    #[serde(default)]
    pub pdb_playlist_id: Option<u32>,
    #[serde(default)]
    pub edb_playlist_id: Option<u32>,
    #[serde(default)]
    pub pdb_sort_order: Option<u32>,
    #[serde(default)]
    pub edb_sort_order: Option<u32>,
    #[serde(default)]
    pub pdb_duplicate_entries: usize,
    #[serde(default)]
    pub edb_missing_core_metadata: usize,
    #[serde(default)]
    pub pdb_missing_core_metadata: usize,
    #[serde(default)]
    pub artwork_mismatch_tracks: usize,
    /// Tracks whose star rating or colour differs between the PDB and the eDB.
    #[serde(default)]
    pub rating_colour_mismatch_tracks: usize,
    /// Tracks whose other synced fields (comment, ISRC, genre, label, …)
    /// differ between the PDB and the eDB. A note: never fails parity.
    #[serde(default)]
    pub field_note_tracks: usize,
    #[serde(default)]
    pub sample_only_in_pdb: Vec<String>,
    #[serde(default)]
    pub sample_only_in_edb: Vec<String>,
    #[serde(default)]
    pub sample_metadata_mismatches: Vec<String>,
    pub status: DiagStatus,
    /// Short human-readable badges for the parity table's "Issues" column
    /// (e.g. `"+PDB 3"`, `"order mismatch"`). Built backend-side from the
    /// counters above so the frontend renders them verbatim.
    #[serde(default)]
    pub issue_labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUsbParityReportData {
    pub overall_status: DiagStatus,
    /// Plain-language summary the desktop UI shows first
    /// (`service::diagnostics::parity_overview`).
    pub overview: Vec<DiagOverviewItem>,
    /// The technical report: `checks` regrouped into columns
    /// (`service::diagnostics::parity_section_groups`).
    pub section_groups: Vec<DiagSectionGroup>,
    // Flat check list and per-category counts. Backend-only (CLI tools,
    // tests); the UI gets the same checks through `section_groups`.
    #[serde(skip_serializing)]
    pub checks: Vec<DiagCheck>,
    #[serde(default, skip_serializing)]
    pub summary_rows: Vec<DiagSummaryRow>,
    pub playlist_details: Vec<UsbParityPlaylistDetail>,
    pub warnings: Vec<WarningEntry>,
    pub duration_ms: u64,
    /// The status line after the run.
    #[serde(default)]
    pub status_text: String,
    /// "Completed in 370ms", in the report's header.
    #[serde(default)]
    pub duration_text: String,
    /// "Parity PASS", on its badge.
    #[serde(default)]
    pub status_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbPlayerMenuItem {
    pub menu_item_id: u32,
    pub kind: u32,
    pub name: String,
    pub is_visible: bool,
    #[serde(default)]
    pub sequence_no: Option<u32>,
    /// False for the browse categories a CDJ requires (TRACK/PLAYLIST/FOLDER/
    /// SEARCH/HISTORY). `update_usb_player_menu_config` rejects a request that
    /// drops one; the frontend just disables the Remove button.
    #[serde(default = "default_menu_removable")]
    pub removable: bool,
    /// The row's text: `name`, or "Menu <kind>" for a row with no name.
    #[serde(default)]
    pub label: String,
    /// Whether the Up and Down buttons apply to this item.
    #[serde(default)]
    pub can_move_up: bool,
    #[serde(default)]
    pub can_move_down: bool,
}

fn default_menu_removable() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetUsbPlayerMenuConfigRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetUsbPlayerMenuConfigData {
    /// The status line after the read.
    #[serde(default)]
    pub status_text: String,
    pub current_items: Vec<UsbPlayerMenuItem>,
    pub available_items: Vec<UsbPlayerMenuItem>,
    pub warnings: Vec<WarningEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUsbPlayerMenuConfigRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    /// What to do with `kind`; the backend applies it to the stick's current
    /// menu order.
    pub action: UsbPlayerMenuEdit,
    pub kind: u32,
}

/// One edit in the player menu editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UsbPlayerMenuEdit {
    /// Show the kind, last in the menu.
    Add,
    /// Hide the kind (rejected for the browse categories a CDJ requires).
    Remove,
    /// Swap the kind with the one before it.
    MoveUp,
    /// Swap the kind with the one after it.
    MoveDown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUsbPlayerMenuConfigData {
    pub updated: bool,
    /// The status line after the edit.
    #[serde(default)]
    pub status_text: String,
    pub current_items: Vec<UsbPlayerMenuItem>,
    pub available_items: Vec<UsbPlayerMenuItem>,
    pub warnings: Vec<WarningEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetUsbBackgroundColorRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetUsbBackgroundColorRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    /// 0 = default, then rekordbox's order: 1 pink, 2 red, 3 orange,
    /// 4 yellow, 5 green, 6 aqua, 7 blue, 8 purple.
    pub color: u8,
}

/// The player background color each database holds. rekordbox lets the two
/// differ; setting a color writes it to both.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbBackgroundColorData {
    /// What older players read (PDB t19); `None` without a t19 data page.
    pub pdb_color: Option<u8>,
    /// What the CDJ-3000 and rekordbox read (eDB `property.backGroundColorType`);
    /// `None` without a readable eDB `property` row.
    pub edb_color: Option<u8>,
    /// Whether a color can be set: both databases have a place for it.
    pub writable: bool,
    /// Why not, when `writable` is false.
    #[serde(default)]
    pub reason: Option<String>,
    /// Whether the set wrote anything (always false for a get).
    #[serde(default)]
    pub updated: bool,
    /// One chip per color, ready to show.
    #[serde(default)]
    pub chips: Vec<UsbBackgroundColorChip>,
    /// Why no color can be set, or that rekordbox left two. Empty otherwise.
    #[serde(default)]
    pub note_text: String,
    /// The status line after a set. Empty for a get.
    #[serde(default)]
    pub status_text: String,
    pub warnings: Vec<WarningEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbBackgroundColorChip {
    /// The value both databases store.
    pub color: u8,
    pub label: String,
    /// Both databases hold this color.
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairUsbDiagnosticsRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    #[serde(default)]
    pub apply: bool,
    #[serde(default)]
    pub selected_fix_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairUnsupportedItem {
    pub issue: String,
    pub reason: String,
    /// `"event-log"` when the Event Log already lists the affected items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairFixProposal {
    pub id: String,
    pub title: String,
    pub description: String,
    pub supported: bool,
    pub destructive: bool,
    pub estimated_writes: usize,
    pub estimated_deletes: usize,
    /// Structural-prerequisite fixes that must run before a strict-parity
    /// upgrade -- the editor locks their checkbox checked. Set backend-side
    /// (see `REPAIR_FIX_DISPLAY_ORDER` / the apply order in `service::repair`).
    #[serde(default)]
    pub always_applied: bool,
    /// rekordbox's own transaction bookkeeping (real rekordbox exports carry
    /// it and play fine): not counted as an issue or listed in the editor,
    /// but applied with every repair that runs. Set backend-side.
    #[serde(default)]
    pub bookkeeping: bool,
    /// `"event-log"` when the Event Log already lists the affected items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

/// One row of the repair preview's fix list, ready to show: a fix that can be
/// chosen, or something only described. `service::repair::repair_rows` builds
/// them from `proposed_fixes` and `unsupported_items`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairRow {
    pub id: String,
    pub title: String,
    pub description: String,
    /// "✓ supported · safe · 3 writes · always applied".
    pub meta_text: String,
    /// The row has a checkbox.
    pub selectable: bool,
    /// The checkbox starts ticked.
    pub checked: bool,
    /// The checkbox can't be unticked.
    pub locked: bool,
    /// Why, when `locked`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked_tooltip: Option<String>,
    /// `"event-log"` when the Event Log already lists the affected items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairUsbDiagnosticsData {
    /// One-sentence plain-language headline of the preview.
    #[serde(default)]
    pub summary: String,
    /// Plain-language rows of what Apply Fixes changes, by area
    /// (`service::repair::repair_overview`); the fix list is the detail.
    #[serde(default)]
    pub overview: Vec<DiagOverviewItem>,
    /// The preview's fix list, ready to show.
    #[serde(default)]
    pub rows: Vec<RepairRow>,
    /// There's nothing to fix, not even by hand.
    #[serde(default)]
    pub nothing_to_fix: bool,
    // Backend only (apply logic, CLI tools, tests): the UI gets `rows` and the text.
    #[serde(default, skip_serializing)]
    pub detected_issues: Vec<String>,
    #[serde(default, skip_serializing)]
    pub proposed_fixes: Vec<RepairFixProposal>,
    #[serde(default, skip_serializing)]
    pub unsupported_items: Vec<RepairUnsupportedItem>,
    #[serde(default, skip_serializing)]
    pub applied_fixes: Vec<String>,
    #[serde(default, skip_serializing)]
    pub skipped_fixes: Vec<String>,
    #[serde(default, skip_serializing)]
    pub failed_fixes: Vec<String>,
    #[serde(default, skip_serializing)]
    pub estimated_file_writes: usize,
    #[serde(default, skip_serializing)]
    pub estimated_file_deletes: usize,
    pub warnings: Vec<WarningEntry>,
    pub duration_ms: u64,
    /// The status line after a preview or an apply.
    #[serde(default)]
    pub status_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Box<RunUsbDiagnosticsData>>,
    /// On apply: from `diagnostics`, or computed separately if that run failed.
    /// Empty on preview, or if computing it failed too (in `warnings`).
    #[serde(default)]
    pub playlist_usb_export_status: Vec<PlaylistUsbExportStatus>,
    /// Whether an apply may have changed what the USB lists and player menu
    /// show, so the UI must stop showing what it has loaded.
    #[serde(default)]
    pub usb_content_changed: bool,
    /// The "Write Engine DJ library" setting after this call; the
    /// `keep_engine_library_up_to_date` fix turns it on.
    #[serde(default)]
    pub engine_library_enabled: bool,
}

// ── detect_external_rekordbox_db ──────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectExternalRekordboxDbData {
    pub found: bool,
    pub path: Option<String>,
    /// Tracks from this library are in the local library (it was imported);
    /// until then its source chip can't be turned on.
    #[serde(default)]
    pub imported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRekordboxDbRequest {
    #[serde(default)]
    pub path: Option<String>,
}

// ── Mixxx library import ───────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectExternalMixxxDbData {
    pub found: bool,
    pub path: Option<String>,
    /// Tracks from this library are in the local library (it was imported);
    /// until then its source chip can't be turned on.
    #[serde(default)]
    pub imported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanMixxxDbRequest {
    #[serde(default)]
    pub path: Option<String>,
}

/// Which kind of list a playlist import reads from an external library: a
/// playlist (incl. Mixxx's Auto DJ), a Mixxx crate, or a history session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExternalPlaylistKind {
    Playlist,
    Crate,
    History,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalPlaylistSummary {
    /// The source library's own id (Mixxx: numeric, rekordbox: text).
    pub id: String,
    pub name: String,
    pub kind: ExternalPlaylistKind,
    pub track_count: usize,
    /// The local playlist an earlier import of this list made; importing it
    /// again updates that playlist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_playlist: Option<ImportedPlaylistRef>,
    /// The import dialog's note on the list: its track count, and which
    /// playlist importing it again updates.
    #[serde(default)]
    pub note_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedPlaylistRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListExternalPlaylistsRequest {
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListExternalPlaylistsData {
    pub items: Vec<ExternalPlaylistSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportExternalPlaylistRequest {
    #[serde(default)]
    pub path: Option<String>,
    pub kind: ExternalPlaylistKind,
    pub id: String,
    /// Replace the tracks' BPM, key and cues with the library's, even where
    /// they were edited or reanalyzed in the app.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportExternalPlaylistData {
    pub playlist_id: String,
    pub name: String,
    /// An earlier import's playlist was updated instead of a new one created.
    pub updated_existing: bool,
    /// Tracks placed in the new playlist.
    pub added: usize,
    /// Entries of the source list not placed: missing or unsupported files,
    /// tracks no longer in the library, and repeats of a track.
    pub skipped: usize,
    /// Tracks new to the local library.
    pub indexed: usize,
    pub not_found: Vec<String>,
    pub warnings: Vec<WarningEntry>,
}

/// One list picked in the playlist import dialog: which library it is from,
/// plus the single-list import's fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportExternalPlaylistsItem {
    pub library: ExternalLibrary,
    /// The list's name as the dialog showed it, for the status line when its
    /// import fails.
    #[serde(default)]
    pub name: String,
    #[serde(flatten)]
    pub request: ImportExternalPlaylistRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportExternalPlaylistsRequest {
    pub items: Vec<ImportExternalPlaylistsItem>,
}

/// One list's import: its playlist, or why it failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportExternalPlaylistOutcome {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist: Option<ImportExternalPlaylistData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportExternalPlaylistsData {
    /// One per requested list, in request order.
    pub results: Vec<ImportExternalPlaylistOutcome>,
    pub imported: usize,
    pub failed: usize,
    /// Tracks placed, and entries skipped, over every imported list.
    pub added: usize,
    pub skipped: usize,
    /// The status line once at least one list was imported.
    pub status_text: String,
}

// ── track cues + beat-grid editing ────────────────────────

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

// ── USB-native cue editing (edits an on-device bundle directly) ────────────

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
