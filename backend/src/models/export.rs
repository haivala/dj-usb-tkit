//! Export requests, results and the batch export overview.

use super::*;
use serde::{Deserialize, Serialize};

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
    /// The playlist's column sort, saved as its track order before the export, unless the
    /// export can't change the order on the USB (`reorder_playlist_tracks`'s lock).
    #[serde(default)]
    pub sort: Option<PlaylistTrackSort>,
}

/// A column sort of a playlist's tracks, as `get_playlist_tracks` sorts them.
/// The export sync mode Settings saves: mirror (`prune_stale`) or additive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetExportSyncModeRequest {
    pub prune_stale: bool,
    /// The open playlist, whose order the new mode may lock.
    #[serde(default)]
    pub playlist_id: Option<String>,
    /// Its active column sort, saved as its order before the mode locks it.
    #[serde(default)]
    pub sort: Option<PlaylistTrackSort>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetExportSyncModeData {
    pub prune_stale: bool,
    /// The open playlist's order is locked in the new mode: its column sort
    /// can't be saved any more (it was saved first, or `status_text` says it
    /// couldn't be).
    pub locked: bool,
    pub status_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistTrackSort {
    pub sort_by: String,
    #[serde(default)]
    pub sort_dir: Option<String>,
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
    /// The status line's link to the Event Log ("(2 warnings)"), shown after `status_text`;
    /// empty without warnings (`utils::warning_text`).
    #[serde(default)]
    pub warning_text: String,
}

/// Export several playlists to the USB as one job (the Export playlists page).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPlaylistsToUsbRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    /// The chosen playlists; exported in the app's order whatever order they come in.
    pub playlist_ids: Vec<String>,
    /// The chosen playlists go on top of the stick's list in the app's order. Only with
    /// every exportable playlist chosen.
    #[serde(default)]
    pub force_playlist_order: bool,
    /// The playlists' entries on the stick become the app's tracks in the app's order,
    /// even in additive mode.
    #[serde(default)]
    pub force_track_order: bool,
    #[serde(default)]
    pub options: Option<ExportToUsbOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPlaylistsToUsbData {
    pub usb_root: String,
    pub exported_playlists: usize,
    pub exported_tracks: usize,
    pub skipped_tracks: usize,
    /// The status line after the export.
    pub status_text: String,
    pub warnings: Vec<WarningEntry>,
    /// The status line's link to the Event Log; empty without warnings.
    #[serde(default)]
    pub warning_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetBatchExportOverviewRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
}

/// Whether the stick already has a playlist (by name), and whether it changed since.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BatchExportStatus {
    /// Not on the stick.
    New,
    /// On the stick, exported there last and unchanged since.
    OnStick,
    /// On the stick, but changed since (or last exported elsewhere).
    OnStickChanged,
}

/// One playlist of the Export playlists page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchExportRow {
    /// Analysis is the only export blocker; the row can be selected for analysis.
    #[serde(default)]
    pub needs_analysis: bool,
    pub playlist_id: String,
    pub name: String,
    pub track_count: usize,
    /// Its tracks' total length ("1:02:03"); empty when none has one.
    pub duration_text: String,
    /// Its tracks' audio size.
    pub size_text: String,
    /// What exporting it would copy to this USB ("12 tracks · 340 MB", "Nothing new").
    pub copy_text: String,
    /// When and to which USB it was last exported ("2026-04-07 13:00 · MY USB"), or "Never".
    pub last_exported_text: String,
    pub status: BatchExportStatus,
    pub status_text: String,
    /// Why it can't be exported now; `None` when it can.
    #[serde(default)]
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchExportOverviewData {
    pub rows: Vec<BatchExportRow>,
    /// The USB's name, for the page heading.
    pub usb_name: String,
    /// Forcing the track order means something only in additive mode (mirror rewrites it
    /// anyway).
    pub force_track_order_available: bool,
    pub force_track_order_tooltip: String,
    /// Tracks in any of the playlists that still need analysis (each counted
    /// once), for the page's Analyze Missing Tracks button; 0 hides it.
    pub unanalyzed_count: usize,
    pub analyze_missing_label: String,
    pub analyze_missing_tooltip: String,
}
