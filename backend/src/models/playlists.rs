//! Playlist requests and results, and the playlist state sent with them.

use super::*;
use serde::{Deserialize, Serialize};

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
    /// The line under the playlist's title with no USB selected: "Last exported
    /// 2026-04-07 13:00 to MY USB (12 tracks)." or "Not exported yet."
    #[serde(default)]
    pub export_status_text: String,
    /// That line's tooltip: the USB's folder, when the line names the USB by its name.
    #[serde(default)]
    pub export_status_tooltip: Option<String>,
    /// The library folder the playlist is in; none at the top level.
    #[serde(default)]
    pub folder_id: Option<String>,
    /// Its folders' names from the top down, joined as "Sets / Friday"; none
    /// at the top level.
    #[serde(default)]
    pub folder_path: Option<String>,
}

/// A library playlist folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistFolder {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub collapsed: bool,
    /// What the dialog asks before deleting the folder.
    pub delete_confirm_text: String,
    /// The collapse arrow's tooltip.
    pub toggle_tooltip: String,
}

/// A folder or a playlist in the sidebar tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaylistTreeKind {
    Folder,
    Playlist,
}

/// One row the sidebar draws, top first: the children of a collapsed folder
/// are left out.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistSidebarRow {
    pub kind: PlaylistTreeKind,
    pub id: String,
    /// Folders above it: 0 at the top level.
    pub depth: usize,
}

/// One row of a list of every playlist in the sidebar's tree, every folder
/// open (the Add to… dialog, the Export playlists page): a folder, or a
/// playlist whose own row the list carries by its id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistListRow {
    pub kind: PlaylistTreeKind,
    pub id: String,
    /// Folders above it: 0 at the top level.
    pub depth: usize,
    /// A folder's name; empty for a playlist.
    #[serde(default)]
    pub name: String,
    /// A folder's playlists, at any depth, which its box ticks; empty for a
    /// playlist. A folder holding none is left out.
    #[serde(default)]
    pub playlist_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePlaylistRequest {
    pub name: String,
    /// The folder it goes in, at the top; none for the top level.
    #[serde(default)]
    pub folder_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePlaylistFolderRequest {
    pub name: String,
    /// The folder it goes in, at the top; none for the top level.
    #[serde(default)]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePlaylistFolderData {
    pub folder_id: String,
    pub name: String,
    pub status_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePlaylistFolderRequest {
    pub folder_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePlaylistFolderData {
    pub folder_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePlaylistFolderRequest {
    pub folder_id: String,
}

/// The folder is gone; what it held moved up one level, in its place.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePlaylistFolderData {
    pub folder_id: String,
    pub deleted: bool,
    pub status_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPlaylistFolderCollapsedRequest {
    pub folder_id: String,
    pub collapsed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPlaylistFolderCollapsedData {
    pub folder_id: String,
    pub collapsed: bool,
}

/// A folder or a playlist, by id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistTreeItem {
    pub kind: PlaylistTreeKind,
    pub id: String,
}

/// Where a moved item lands, next to `target` or inside it (a folder, at the end).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaylistTreePlacement {
    Before,
    After,
    Inside,
}

/// Moves a folder (with what it holds) or a playlist in the sidebar tree.
/// The tree is the library's own; it never changes anything on the USB.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovePlaylistTreeItemRequest {
    pub item: PlaylistTreeItem,
    pub target: PlaylistTreeItem,
    pub placement: PlaylistTreePlacement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovePlaylistTreeItemData {
    pub item: PlaylistTreeItem,
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
    /// In tree order: top first, depth-first.
    pub items: Vec<Playlist>,
    pub folders: Vec<PlaylistFolder>,
    pub sidebar: Vec<PlaylistSidebarRow>,
}

/// The playlists as they are, with their export status against the selected USB: what
/// `list_playlists` answers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistsState {
    /// In tree order: top first, depth-first.
    pub playlists: Vec<Playlist>,
    pub folders: Vec<PlaylistFolder>,
    /// What the sidebar draws, top first.
    pub sidebar: Vec<PlaylistSidebarRow>,
    pub playlist_usb_export_status: Vec<PlaylistUsbExportStatus>,
}

/// The response of a command that changed the playlists: its own `data`, and the playlists
/// as they now are, so the window redraws from them instead of asking again.
#[derive(Debug, Clone, Serialize)]
pub struct WithPlaylists<T> {
    #[serde(flatten)]
    pub data: T,
    #[serde(flatten)]
    pub state: PlaylistsState,
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

/// The playlists chosen on the Export playlists page, against the connected USB.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPlaylistsExportSpaceRequest {
    pub playlist_ids: Vec<String>,
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
    /// The footer, "Total time: 1:02:03 (2 without length)", from `total_duration_ms` and
    /// `duration_unknown_count`.
    #[serde(default)]
    pub duration_text: String,
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
    /// Tracks left out because their format can never go on a USB.
    #[serde(default)]
    pub unplayable: usize,
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
    /// The selection bar's time, "· 3:45", with "+" when some have no length yet; empty
    /// when no selected track has a length.
    #[serde(default)]
    pub duration_text: String,
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

/// Every local playlist in sidebar order, with how much of the selection it holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPlaylistMembershipData {
    pub items: Vec<PlaylistMembership>,
    /// The dialog's rows: `items` by id, under their folders.
    pub tree: Vec<PlaylistListRow>,
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
