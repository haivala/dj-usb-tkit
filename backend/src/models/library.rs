//! Library requests: scanning, source folders and browsing them, and
//! matching a file to a library track.

use super::*;
use serde::{Deserialize, Serialize};

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
    /// Files the scan or import couldn't find. Not sent: each is logged as a warning (an
    /// import's `scan.*.file-not-found`, the folder scan's missing-folders line).
    #[serde(default, skip_serializing)]
    pub not_found: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<WarningEntry>,
    /// The status line: for a folder scan the folder count and the tracks
    /// indexed, for a library import what it brought in.
    #[serde(default)]
    pub status_text: String,
    /// A library import's tracks whose values differ from the library's (kept
    /// as they were): the question for the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicts: Option<ImportConflicts>,
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
    /// The footer, "Total time: 1:02:03 (2 without length)", from `total_duration_ms` and
    /// `duration_unknown_count`.
    #[serde(default)]
    pub duration_text: String,
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
    /// Every track imported from the library has its core analysis (its
    /// chip shows green, as a fully analyzed folder's does).
    pub rekordbox_db_fully_analyzed: bool,
    pub mixxx_db_fully_analyzed: bool,
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
    /// Some source's tracks are hidden from the library: a folder switched off
    /// or missing, or an imported library switched off. The library's filter
    /// mark shows it.
    pub filtering: bool,
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
    /// The scan that indexed the added folders; `None` when nothing was
    /// added or the scan failed. Not sent: the Tauri command reports it as a
    /// scan job.
    #[serde(default, skip_serializing)]
    pub scan: Option<ScanLibraryData>,
    /// The scan's status line, the failed scan's, or (nothing added) the
    /// folder count.
    pub status_text: String,
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
    /// The scan that indexed the new folder; `None` when it failed. Not
    /// sent: the Tauri command reports it as a scan job.
    #[serde(default, skip_serializing)]
    pub scan: Option<ScanLibraryData>,
    /// The relocation's line, with the files the scan indexed or why it failed.
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
