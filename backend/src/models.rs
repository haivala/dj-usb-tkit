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
        crate::logging::emit(
            crate::logging::Level::Error,
            error.code.as_str(),
            &error.message,
        );
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm_analyzer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
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
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub bpm: Option<f64>,
    pub bpm_analyzer: Option<String>,
    pub key: Option<String>,
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
    pub master_db_source: bool,
    /// Authoritative: true when `file_path` falls under any known USB device
    /// root (current or previously pruned). Computed fresh on every read
    /// against the `usb_devices` registry -- not stored. See
    /// `untainted_usb_root_paths`/`browse_path_matches_root`.
    pub is_usb_path: bool,
    /// Authoritative: true when the track has its core analysis (waveform
    /// peaks path + BPM > 0 + duration > 0). Computed fresh on every read
    /// from the DB row -- not stored, no filesystem check. The frontend
    /// treats this as the only signal for "needs analysis". See
    /// `service::has_core_analysis_fields`.
    pub analysis_ready: bool,
    /// CDJ playback-compatibility judgement for this track's audio format,
    /// derived from `format_ext` + the technical fields above. Computed fresh
    /// on every read (see `service::format_compat`) -- the frontend renders the
    /// badge/tooltip and never re-derives the rule.
    #[serde(default)]
    pub format_compat: FormatCompat,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanLibraryRequest {
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
    /// Tracks under the scanned roots, computed over the whole library after
    /// the scan commits (not the page the frontend reloads). `0` for the
    /// pre-scan validation-error / no-op returns.
    #[serde(default)]
    pub scoped_track_count: usize,
    /// Distinct non-empty album names among `scoped_track_count`.
    #[serde(default)]
    pub album_count: usize,
    /// How many of `scoped_track_count` still need core analysis
    /// (same predicate as `Track::analysis_ready`).
    #[serde(default)]
    pub unanalyzed_count: usize,
    #[serde(default)]
    pub warnings: Vec<WarningEntry>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvePlaybackSourceData {
    pub resolved_path: Option<String>,
    pub matched_by: String,
    pub track_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchTracksRequest {
    pub query: String,
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchTracksData {
    pub total: usize,
    pub items: Vec<Track>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTracksRequest {
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTracksData {
    pub total: usize,
    pub items: Vec<Track>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseSourceFilesRequest {
    pub source_roots: Vec<String>,
    #[serde(default)]
    pub include_master_db: bool,
    pub query: String,
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<String>,
    /// One of `title` | `artist` | `album` | `format` | `bpm` | `durationMs` |
    /// `key`. Absent ⇒ the natural file-path order. See `sort_tracks`.
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
    pub source_root_analysis: Vec<SourceRootAnalysisStatus>,
    #[serde(default)]
    pub total_duration_ms: u64,
    #[serde(default)]
    pub duration_known_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckSourceRootsRequest {
    pub source_roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRootStatus {
    pub source_root: String,
    pub exists: bool,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckSourceRootsData {
    pub items: Vec<SourceRootStatus>,
    #[serde(default)]
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRootAnalysisStatus {
    pub source_root: String,
    pub total: usize,
    pub analyzed: usize,
    pub fully_analyzed: bool,
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
    pub resolved_by: String,
    pub materialized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveTracksBySourceRootsRequest {
    pub source_roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveTracksBySourceRootsData {
    pub removed: usize,
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
    /// `0` ⇒ unpaginated (the whole playlist), for backward compatibility.
    #[serde(default)]
    pub limit: usize,
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
    pub duration_known_count: usize,
    /// Tracks in the whole filtered playlist the backend flags as not yet
    /// analysis-ready (drives the "Analyze Missing Tracks (N)" button, which
    /// can't count them from a single loaded page).
    #[serde(default)]
    pub unanalyzed_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DedupeMode {
    Allow,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTracksToPlaylistRequest {
    pub playlist_id: String,
    pub track_ids: Vec<String>,
    pub dedupe: DedupeMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTracksToPlaylistData {
    pub playlist_id: String,
    pub added: usize,
    pub skipped: usize,
}

/// Every local track id that matches the current library filter -- so
/// "Select all" doesn't have to page the whole list into the frontend just to
/// enumerate ids.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMatchingTrackIdsRequest {
    #[serde(default)]
    pub source_roots: Vec<String>,
    #[serde(default)]
    pub include_master_db: bool,
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
/// sends the current filter plus either the ticked ids or `all_matching`, and
/// the backend enumerates, materializes any browse-only rows, and appends --
/// so a selection that spans pages the user has scrolled past is never lost.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddLibrarySelectionToPlaylistRequest {
    pub playlist_id: String,
    #[serde(default)]
    pub source_roots: Vec<String>,
    #[serde(default)]
    pub include_master_db: bool,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub track_ids: Vec<String>,
    #[serde(default)]
    pub all_matching: bool,
    pub dedupe: DedupeMode,
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
    pub resolved_by: String,
    pub materialized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTrackCandidatesToPlaylistRequest {
    pub playlist_id: String,
    #[serde(default)]
    pub tracks: Vec<AddTrackCandidate>,
    pub dedupe: DedupeMode,
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
    #[serde(default)]
    pub has_write_access: bool,
    pub normalized_root: Option<String>,
    pub has_vendor_root: bool,
    pub has_contents: bool,
    pub has_pdb: bool,
    pub has_edb: bool,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchUsbPlaylistsRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
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
    pub saved: bool,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbTrack {
    pub id: String,
    #[serde(default)]
    pub local_track_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub bpm: Option<f64>,
    pub key: Option<String>,
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
    /// File size as recorded by Rekordbox at export/analysis time (from the
    /// PDB), not a fresh stat of the mounted file. Used to gate fingerprint
    /// matching against local tracks so same-length variants (e.g. Clean vs
    /// Explicit edits) don't get silently merged.
    #[serde(default)]
    pub file_size_bytes: Option<i64>,
    /// CDJ format-compatibility badge (mirrors `Track.format_compat`). USB rows
    /// only carry `format_ext`, so this is `ok` for every known extension and
    /// only flags a genuinely unrecognised one. Filled in during page
    /// hydration -- see `service::usb::hydrate_usb_track_in_place`.
    #[serde(default)]
    pub format_compat: FormatCompat,
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
    pub tracks: Vec<UsbTrack>,
    /// Sum of `duration_ms` over tracks where it's known (>0). Computed once
    /// server-side from the full track list so the frontend never needs to
    /// sum durations itself, regardless of how much of the list it has
    /// rendered/hydrated.
    pub total_duration_ms: u64,
    /// How many of `tracks` contributed to `total_duration_ms` (i.e. had a
    /// known duration). `tracks.len() - duration_known_count` is the "N
    /// without length" count.
    pub duration_known_count: usize,
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
    pub same_name_exists_on_usb: bool,
    pub locks_reorder: bool,
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
    /// Sum of `track_count` over `items` (post name-dedupe) -- the "M tracks"
    /// figure in the USB panel header. Computed server-side so the frontend
    /// never re-sums per-playlist counts. Deliberately the sum, not a distinct
    /// count, so a track in two playlists still counts twice.
    pub playlist_track_total: usize,
    pub warnings: Vec<WarningEntry>,
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
    pub removed_from_edb: usize,
    pub removed_from_pdb: usize,
    pub tracks_removed: usize,
    pub files_deleted: usize,
    pub tracks_kept_shared: usize,
    pub warnings: Vec<WarningEntry>,
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
    pub warnings: Vec<WarningEntry>,
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
    pub tracks: Vec<UsbTrack>,
    /// See `UsbPlaylist::total_duration_ms`.
    pub total_duration_ms: u64,
    /// See `UsbPlaylist::duration_known_count`.
    pub duration_known_count: usize,
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
    pub warnings: Vec<WarningEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayTrackRequest {
    pub path: String,
    #[serde(default)]
    pub start_offset_ms: Option<u64>,
    #[serde(default)]
    pub start_ratio: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayTrackData {
    pub path: String,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayResolvedTrackData {
    pub path: String,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub track_id: Option<String>,
    pub matched_by: String,
    pub source: String,
    pub source_label: String,
    pub library_resolved: bool,
    pub has_usb_context: bool,
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
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackEventPayload {
    pub event: String,
    pub path: Option<String>,
    pub playing: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackPreflightRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackPreflightData {
    pub path: String,
    pub file_exists: bool,
    pub file_readable: bool,
    pub safe_output_devices: Vec<String>,
    pub ready: bool,
    pub message: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectUsbTrackItem {
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
pub struct InspectUsbTracksRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    #[serde(default)]
    pub items: Vec<InspectUsbTrackItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectUsbTrackResult {
    pub track_id: String,
    pub source: Option<String>,
    pub track: Option<UsbTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectUsbTracksData {
    pub items: Vec<InspectUsbTrackResult>,
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
    pub duration_known_count: usize,
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
    /// auto-selection to the current library filter (`source_roots` +
    /// `include_master_db` + `query`) instead of the whole DB.
    #[serde(default)]
    pub scope_to_library_filter: bool,
    #[serde(default)]
    pub bpm_min: Option<u32>,
    #[serde(default)]
    pub bpm_max: Option<u32>,
    #[serde(default)]
    pub analysis_engine: Option<String>,
    #[serde(default)]
    pub source_roots: Vec<String>,
    #[serde(default)]
    pub include_master_db: bool,
    #[serde(default)]
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeNewTracksData {
    pub job_id: String,
    pub analyzed: usize,
    pub failed: usize,
    pub warnings: Vec<WarningEntry>,
    #[serde(default)]
    pub items: Vec<Track>,
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
    pub manifest_path: String,
    pub warnings: Vec<WarningEntry>,
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
    pub label: String,
    pub status: DiagStatus,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
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
    pub pdb_integrity: DiagSection,
    pub edb_access: DiagSection,
    pub contents_integrity: DiagSection,
    pub analysis_integrity: DiagSection,
    pub playlist_resolution: DiagSection,
    pub playlist_details: Vec<PlaylistDiagEntry>,
    /// Raw player-counter table signals -- consumed by the `run_usb_diagnostics`
    /// CLI debug tool. The desktop UI renders `cdj_counter_section` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdj_counter_snapshot: Option<PlayerCounterSnapshot>,
    /// The player-counter snapshot rendered as a `DiagSection` (built
    /// backend-side from `cdj_counter_snapshot` so the frontend just appends it
    /// alongside the other sections).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdj_counter_section: Option<DiagSection>,
    pub warnings: Vec<WarningEntry>,
    pub duration_ms: u64,
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
    pub checks: Vec<DiagCheck>,
    #[serde(default)]
    pub summary_rows: Vec<DiagSummaryRow>,
    pub playlist_details: Vec<UsbParityPlaylistDetail>,
    pub warnings: Vec<WarningEntry>,
    pub duration_ms: u64,
}

/// Which database(s) a menu item appears in.
///
/// Describes whether a player-menu item came from PDB t16, eDB category data,
/// or both representations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsbPlayerMenuItemOrigin {
    /// Row present in both PDB t16 and eDB menuItem.
    Both,
    /// Row only present in PDB t16 (eDB missing a matching kind).
    PdbOnly,
    /// Row only present in eDB menuItem (PDB t16 missing this kind).
    EdbOnly,
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
    #[serde(default = "default_menu_origin")]
    pub origin: UsbPlayerMenuItemOrigin,
    /// False for the browse categories a CDJ requires (TRACK/PLAYLIST/FOLDER/
    /// SEARCH/HISTORY). `update_usb_player_menu_config` rejects a request that
    /// drops one; the frontend just disables the Remove button.
    #[serde(default = "default_menu_removable")]
    pub removable: bool,
}

fn default_menu_origin() -> UsbPlayerMenuItemOrigin {
    UsbPlayerMenuItemOrigin::Both
}

fn default_menu_removable() -> bool {
    true
}

/// Kind-level mismatch between PDB t16 (master) and eDB category/menuItem.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsbPlayerMenuDivergence {
    /// Kinds visible in eDB but not present in PDB t16.
    pub in_edb_visible_only: Vec<u32>,
    /// Kinds present in PDB t16 but not visible in eDB.
    pub in_pdb_only: Vec<u32>,
    /// True if PDB t16 order and eDB visible sequenceNo order disagree for
    /// the kinds present on both sides.
    pub order_mismatch: bool,
    /// eDB menuItem kinds absent from PDB t16. Non-empty means PDB was
    /// trimmed (by old code) and older players have fewer browse categories.
    /// Use the PDB sync command to restore.
    #[serde(default)]
    pub pdb_missing_kinds: Vec<u32>,
    /// One-line description of the divergence for the editor's warning banner,
    /// or empty when there is nothing to report. Built backend-side.
    #[serde(default)]
    pub summary: String,
    /// Whether the "Sync eDB → PDB" action has anything to do
    /// (`in_edb_visible_only` non-empty).
    #[serde(default)]
    pub can_sync: bool,
    /// Whether the "Restore PDB categories" action has anything to do
    /// (`pdb_missing_kinds` non-empty).
    #[serde(default)]
    pub can_restore: bool,
}

impl UsbPlayerMenuDivergence {
    pub fn is_empty(&self) -> bool {
        self.in_edb_visible_only.is_empty()
    }
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
    pub current_items: Vec<UsbPlayerMenuItem>,
    pub available_items: Vec<UsbPlayerMenuItem>,
    #[serde(default)]
    pub divergence: UsbPlayerMenuDivergence,
    pub warnings: Vec<WarningEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUsbPlayerMenuConfigRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    /// Ordered list of menu_item_ids to keep visible. Kept for backwards
    /// compatibility; prefer `current_kinds` on new callers since `kind` is the
    /// universal identifier that works for PDB-only rows too (menu_item_id = 0
    /// for those).
    #[serde(default)]
    pub current_menu_item_ids: Vec<u32>,
    /// Ordered list of `kind` values to keep visible. When present, this takes
    /// precedence over `current_menu_item_ids` and is the source of truth for
    /// what gets written to PDB t16.
    #[serde(default)]
    pub current_kinds: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUsbPlayerMenuConfigData {
    pub updated: bool,
    pub current_items: Vec<UsbPlayerMenuItem>,
    pub available_items: Vec<UsbPlayerMenuItem>,
    #[serde(default)]
    pub divergence: UsbPlayerMenuDivergence,
    pub warnings: Vec<WarningEntry>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairUsbDiagnosticsData {
    pub detected_issues: Vec<String>,
    pub proposed_fixes: Vec<RepairFixProposal>,
    pub unsupported_items: Vec<RepairUnsupportedItem>,
    pub applied_fixes: Vec<String>,
    pub skipped_fixes: Vec<String>,
    pub failed_fixes: Vec<String>,
    pub estimated_file_writes: usize,
    pub estimated_file_deletes: usize,
    pub warnings: Vec<WarningEntry>,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Box<RunUsbDiagnosticsData>>,
}

// ── detect_external_master_db ──────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectExternalMasterDbData {
    pub found: bool,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanMasterDbRequest {
    #[serde(default)]
    pub path: Option<String>,
}
