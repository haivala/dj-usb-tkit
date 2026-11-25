//! The library track row and what is derived for it: its files in other
//! formats and its player compatibility.

use serde::{Deserialize, Serialize};

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
    /// `AnalysisEngine::bpm_analyzer` for a detected value (`"stratum-v2"`,
    /// `"essentia"`; none for a key analyzed before the source was written),
    /// `"user"` after a manual edit, `"rekordbox"` / `"mixxx"` from an import.
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
    /// does; see `local_analysis_bundle_unusable`). The frontend
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
    /// The track can't be added to a playlist, as its format can never go on
    /// a USB: the row's "+" is off, with the warning as its tooltip.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub add_blocked: bool,
}
