//! rekordbox and Mixxx imports: their libraries, playlists and track data.

use super::*;
use serde::{Deserialize, Serialize};

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
    /// Files the import couldn't find. Not sent: each is logged as a warning
    /// (`scan.*.file-not-found`).
    #[serde(default, skip_serializing)]
    pub not_found: Vec<String>,
    pub warnings: Vec<WarningEntry>,
    /// Tracks whose values differ from the library's. Not sent: the
    /// several-list import merges them into its own `conflicts`.
    #[serde(default, skip_serializing)]
    pub conflicts: Option<ImportConflicts>,
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
    /// The status line: the imported lists, or (none imported) why the first
    /// one failed.
    pub status_text: String,
    /// One question per library whose tracks differ (see [`ImportConflicts`]).
    #[serde(default)]
    pub conflicts: Vec<ImportConflicts>,
}

/// After a rekordbox / Mixxx import: the tracks whose cues, BPM, key or beat
/// grid differ from the library's in values the import may not replace on its
/// own (edited in the app, analyzed, or from elsewhere). The import kept them;
/// the dialog asks whether to take the library's (`apply_external_track_data`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportConflicts {
    pub library: ExternalLibrary,
    /// The library database the import read, for the apply call.
    #[serde(default)]
    pub path: Option<String>,
    pub track_ids: Vec<String>,
    pub title_text: String,
    pub message_text: String,
    /// The button that takes the library's values.
    pub use_label: String,
    pub rows: Vec<ImportConflictRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportConflictRow {
    pub track_id: String,
    /// "Artist – Title".
    pub name_text: String,
    /// What differs: "cues, BPM".
    pub differs_text: String,
}

/// Take the library's cues, BPM, key and beat grid for these library tracks,
/// replacing their own.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyExternalTrackDataRequest {
    pub library: ExternalLibrary,
    #[serde(default)]
    pub path: Option<String>,
    pub track_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyExternalTrackDataData {
    /// Tracks whose values were replaced.
    pub updated: usize,
    pub status_text: String,
}
