//! USB requests and results: selecting and naming a stick, its playlists,
//! histories and tracks, order edits, backups, audio formats, Player Menu and
//! background colour.

use super::*;
use serde::{Deserialize, Serialize};

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
pub struct SelectUsbRequest {
    /// The folder to select; empty selects no USB.
    pub path: String,
}

/// `select_usb`: the folder's check (`valid`, `normalizedRoot`, `statusText`, `canInitialize`,
/// `initHintText`) and, for a valid USB, its audio formats and drive name. `devices` is the
/// list of known drives, which the selection may have added to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectUsbData {
    #[serde(flatten)]
    pub usb: ValidateUsbRootData,
    pub audio_formats: Option<UsbAudioFormatsData>,
    pub device_name: Option<GetUsbDeviceNameData>,
    pub devices: Vec<UsbDeviceSummary>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PruneUsbDeviceData {
    pub pruned: bool,
    /// The known drives after the prune (`list_usb_devices`).
    pub devices: Vec<UsbDeviceSummary>,
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
    /// The backups list after the restore (it took a "Before restore" backup).
    pub backups: ListUsbBackupsData,
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
    /// The backups list without the deleted one.
    pub backups: ListUsbBackupsData,
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
    /// The pending order edit changed this playlist's track order (the list's mark).
    #[serde(default)]
    pub order_edited: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbImportStats {
    pub indexed_tracks: usize,
    pub playlist_referenced_tracks: usize,
    pub playlist_entries: usize,
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
    /// The line under the playlist's title: with a USB selected, whether it's on it
    /// ("On this USB. Last exported …"); without, `Playlist::export_status_text`.
    #[serde(default)]
    pub export_status_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchUsbPlaylistsData {
    /// In the pending order while an order edit is on.
    pub items: Vec<UsbPlaylist>,
    /// The order edit's state for this USB.
    #[serde(default)]
    pub order_edit: UsbOrderEditState,
    /// Edit order's tooltip when the USB can't be written (the button is disabled); `None`
    /// when it can.
    #[serde(default)]
    pub order_edit_blocked_text: Option<String>,
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
    /// The status line's link to the Event Log ("(2 warnings)"), shown after `status_text`;
    /// empty without warnings (`utils::warning_text`).
    #[serde(default)]
    pub warning_text: String,
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
    /// The status line's link to the Event Log ("(2 warnings)"), shown after `status_text`;
    /// empty without warnings (`utils::warning_text`).
    #[serde(default)]
    pub warning_text: String,
}

/// The USB a USB order-edit command works on: `begin_usb_order_edit`,
/// `commit_usb_order_edit`, `cancel_usb_order_edit`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbOrderEditRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
}

/// Moves a USB playlist to just before `before_playlist_id` in the pending
/// playlist order, or to the end when that is `None`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveUsbPlaylistRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    pub playlist_id: String,
    #[serde(default)]
    pub before_playlist_id: Option<String>,
}

/// Moves one row (`UsbTrack::id`) of a USB playlist to just before
/// `before_track_id` in its pending order, or to the end when that is `None`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveUsbPlaylistTrackRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    pub playlist_id: String,
    pub move_track_id: String,
    #[serde(default)]
    pub before_track_id: Option<String>,
}

/// Puts a whole USB playlist's pending order in this column sort (the sort
/// `fetch_usb_playlist_tracks` uses).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortUsbPlaylistTracksRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    pub playlist_id: String,
    #[serde(default)]
    pub sort_by: Option<String>,
    #[serde(default)]
    pub sort_dir: Option<String>,
}

/// Where the USB view's order edit stands, as the Edit order controls show it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbOrderEditState {
    /// An order edit is on for this USB.
    pub active: bool,
    /// The pending order differs from the USB's: Save order has something to write.
    pub changed: bool,
    /// The playlists whose track order is edited (`UsbPlaylist::order_edited`).
    #[serde(default)]
    pub edited_playlist_ids: Vec<String>,
    /// "Editing order: 2 playlists changed"; empty while no edit is on.
    #[serde(default)]
    pub editing_text: String,
}

/// The answer of every USB order-edit command.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbOrderEditData {
    pub order_edit: UsbOrderEditState,
    /// The status line after the command; empty for a move or a sort.
    #[serde(default)]
    pub status_text: String,
    #[serde(default)]
    pub warnings: Vec<WarningEntry>,
    /// The status line's link to the Event Log ("(2 warnings)"), shown after `status_text`;
    /// empty without warnings (`utils::warning_text`).
    #[serde(default)]
    pub warning_text: String,
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
    /// The database the tracks come from: "PDB" or "eDB" (see
    /// `UsbPlaylist::source`).
    #[serde(default)]
    pub source: String,
    /// Both databases have the session (the heading says "Both DBs").
    #[serde(skip)]
    pub in_both: bool,
    /// The track list's heading: the name with the database that has the
    /// session, e.g. "HISTORY 007 (Both DBs)".
    #[serde(default)]
    pub heading_text: String,
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
    /// The warn/error entries among `warnings` (`logging::count_warnings`), kept so
    /// `warning_text` is rendered when answering, as the other texts are.
    #[serde(default, skip_serializing)]
    pub warning_count: usize,
    /// The status line's link to the Event Log ("(2 warnings)"), shown after `status_text`;
    /// empty without warnings (`utils::warning_text`).
    #[serde(default)]
    pub warning_text: String,
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
    /// The footer, "Total time: 1:02:03 (2 without length)", from `total_duration_ms` and
    /// `duration_unknown_count`.
    pub duration_text: String,
    pub warnings: Vec<WarningEntry>,
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
