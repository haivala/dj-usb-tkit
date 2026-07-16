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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
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
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetSystemParallelismData {
    pub workers: usize,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseSourceFilesRequest {
    pub source_roots: Vec<String>,
    #[serde(default)]
    pub include_master_db: bool,
    pub query: String,
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPlaylistTracksRequest {
    pub playlist_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPlaylistTracksData {
    pub playlist_id: String,
    pub items: Vec<Track>,
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
    pub warnings: Vec<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchUsbPlaylistsData {
    pub items: Vec<UsbPlaylist>,
    pub stats: UsbImportStats,
    pub warnings: Vec<WarningEntry>,
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
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeNewTracksRequest {
    #[serde(default)]
    pub track_ids: Vec<String>,
    #[serde(default)]
    pub analysis_engine: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeNewTracksData {
    pub job_id: String,
    pub analyzed: usize,
    pub failed: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeTrackPieceRequest {
    pub track_id: String,
    pub piece: String,
    #[serde(default)]
    pub bpm_min: Option<u32>,
    #[serde(default)]
    pub bpm_max: Option<u32>,
    #[serde(default)]
    pub analysis_engine: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeTrackPieceData {
    pub track_id: String,
    pub piece: String,
    pub updated: bool,
    pub bpm: Option<f64>,
    pub bpm_analyzer: Option<String>,
    pub key: Option<String>,
    pub duration_ms: Option<u64>,
    pub artwork_path: Option<String>,
    pub waveform_peaks_path: Option<String>,
    pub waveform_preview: Option<Vec<u8>>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdj_counter_snapshot: Option<PlayerCounterSnapshot>,
    pub warnings: Vec<WarningEntry>,
    pub duration_ms: u64,
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
}

fn default_menu_origin() -> UsbPlayerMenuItemOrigin {
    UsbPlayerMenuItemOrigin::Both
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
