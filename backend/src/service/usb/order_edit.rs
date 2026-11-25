//! Editing the order of a USB's playlists and of the tracks in them.
//!
//! The USB view's **Edit order** starts an edit (`begin_usb_order_edit`). The
//! moves and sorts change a pending order held here, never the stick; the
//! playlist reads (`fetch_usb_playlists`, `fetch_usb_playlist_tracks`) show
//! the stick's playlists in that pending order, so the paged track table
//! scrolls through the edited order. **Save order**
//! (`commit_usb_order_edit`) writes all of it at once: one backup, one write
//! per database, one Engine DJ rebuild. **Cancel** (`cancel_usb_order_edit`)
//! drops it.
//!
//! The pending track order is keyed by media path, not by row id: the view's
//! rows are the eDB's when the eDB holds the playlist (ids are eDB
//! `content_id`s) and the PDB's otherwise, and the two databases can differ
//! in membership. Each database is ordered from its own rows by path, with
//! [`apply_pending_order`], the same function that orders the view, so what
//! the view shows is what gets written.

use super::playlist_tree::{self, MoveRefusal, PendingTree, Placement, UsbTreeNode};
use super::root::{can_write_file_at, has_write_access, resolve_usb_root};
use super::staging::{SessionBackup, UsbDatabases, UsbWriteGuard};
use super::text::canonicalize_playlist_name;
use crate::error::{BackendError, BackendResult};
use crate::i18n::tr;
use crate::logging::{self, Level};
use crate::models::{
    MoveUsbPlaylistRequest, MoveUsbPlaylistTrackRequest, SortUsbPlaylistTracksRequest,
    UsbOrderEditData, UsbOrderEditRequest, UsbOrderEditState, UsbPlaylist, UsbTrack, WarningEntry,
};
use crate::pdb_reader::{
    ParsedPdb, PdbPlaylistEntryRow, PdbPlaylistTreeRow, PlaylistPath, PlaylistTree,
    parse_pdb_bytes, pdb_page_size,
};
use crate::pdb_writer::{PdbPlaylistTreeSortOrderPatch, patch_playlist_tree_sort_orders_in_place};
use crate::service::BackendService;
use crate::service::diagnostics::{
    FieldNoteParity, RatingColourParity, build_usb_parity_comparison,
};
use crate::service::export_helpers::canonicalize_track_path_identity;
use crate::service::repair::write_edb_playlist_track_order;
use crate::utils::warning_text;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{MutexGuard, PoisonError};

/// The reason recorded on the backup a save takes. A save whose stick's
/// newest backup has this reason rewrites that one instead of adding one.
const BACKUP_REASON: &str = "Before order edit";

/// The pending order: what a save would write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PendingOrder {
    /// The playlists' and folders' pending places.
    tree: PendingTree,
    /// Per edited playlist (`UsbPlaylist::id`), the media path key
    /// ([`track_order_key`]) of each of its rows in the new order.
    tracks: HashMap<String, Vec<String>>,
}

impl PendingOrder {
    fn is_empty(&self) -> bool {
        self.tree.is_empty() && self.tracks.is_empty()
    }

    /// Stores `keys` as `playlist_id`'s order, or drops it when it is the
    /// stick's order again (`base`), so moving a row back leaves nothing to
    /// save.
    fn set_tracks(&mut self, playlist_id: &str, keys: Vec<String>, base: &[String]) {
        if keys == base {
            self.tracks.remove(playlist_id);
        } else {
            self.tracks.insert(playlist_id.to_string(), keys);
        }
    }
}

/// An order edit of one USB. At most one exists, on
/// `BackendService::usb_order_edit`, shared across the service's clones.
#[derive(Debug)]
pub(crate) struct UsbOrderEdit {
    usb_root: PathBuf,
    pending: PendingOrder,
    /// A save is running: everything else that touches the edit is refused.
    saving: bool,
}

pub(crate) type UsbOrderEditSlot = std::sync::Mutex<Option<UsbOrderEdit>>;

/// Puts `rows` in the pending order: the rows whose key `pending` lists come
/// first, in its order, then the rest in their existing relative order. The
/// view (`fetch_usb_playlists`, `fetch_usb_playlist_tracks`) and the save (the
/// PDB's playlist entries, the eDB's `playlist_content` rows) all order their
/// rows with this. An empty key is never listed.
pub(crate) fn apply_pending_order<T>(
    rows: &mut [T],
    pending: &[String],
    key: impl Fn(&T) -> String,
) {
    if pending.is_empty() {
        return;
    }
    // The first position of a key wins.
    let position: HashMap<&str, usize> = pending
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, key)| !key.is_empty())
        .map(|(index, key)| (key.as_str(), index))
        .collect();
    // Stable, so the rows the list doesn't have keep their order.
    rows.sort_by_cached_key(|row| {
        position
            .get(key(row).as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
}

/// A track row's key in a pending track order: its media path, as both
/// databases' paths compare.
pub(crate) fn track_order_key(track: &UsbTrack) -> String {
    canonicalize_track_path_identity(track.identity_path())
}

/// Moves `rows[from]` to just before `rows[before]`, or to the end.
pub(crate) fn move_before<T>(rows: &mut Vec<T>, from: usize, before: Option<usize>) {
    let item = rows.remove(from);
    let to = match before {
        Some(before) if before > from => before - 1,
        Some(before) => before,
        None => rows.len(),
    };
    rows.insert(to, item);
}

fn is_history_id(id: &str) -> bool {
    id.starts_with("usb-h-")
}

/// The PDB playlist id of a USB playlist id (`usb-pl-<id>`).
fn pdb_playlist_id(id: &str) -> Option<u32> {
    id.strip_prefix("usb-pl-")
        .filter(|rest| !rest.starts_with("name-"))
        .and_then(|rest| rest.parse::<u32>().ok())
}

/// The path keys an eDB playlist can have to be this USB playlist, as the
/// playlist read pairs them: an eDB-only playlist's own key, a PDB
/// playlist's path key.
fn edb_playlist_names(id: &str, parsed: Option<&ParsedPdb>) -> Vec<String> {
    if let Some(name) = id.strip_prefix("usb-pl-name-") {
        return vec![name.to_string()];
    }
    let (Some(pdb_id), Some(parsed)) = (pdb_playlist_id(id), parsed) else {
        return Vec::new();
    };
    let Some(leaf) = parsed
        .playlist_tree
        .iter()
        .find(|row| row.id == pdb_id && !row.is_folder)
    else {
        return Vec::new();
    };
    vec![
        crate::pdb_reader::PlaylistTree::new(&parsed.playlist_tree)
            .path(leaf)
            .key(),
    ]
}

fn order_edit_state(edit: &UsbOrderEdit) -> UsbOrderEditState {
    let mut edited_playlist_ids: Vec<String> = edit.pending.tracks.keys().cloned().collect();
    edited_playlist_ids.sort();
    let count = edited_playlist_ids.len();
    let editing_text = match (!edit.pending.tree.is_empty(), count) {
        (false, 0) => tr::usb_order_edit_editing(),
        (false, count) => tr::usb_order_edit_editing_changed(count),
        (true, 0) => tr::usb_order_edit_editing_list_changed(),
        (true, count) => tr::usb_order_edit_editing_list_and_changed(count),
    };
    UsbOrderEditState {
        active: true,
        changed: !edit.pending.is_empty(),
        edited_playlist_ids,
        editing_text: editing_text.render(),
    }
}

/// Where the save puts one row of a database's playlist tree: its place
/// among its folder's items and, when it moved, its folder's row id (0: the
/// top level). `name` is the item's, for messages.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RowPlace {
    position: u32,
    parent: Option<u32>,
    name: String,
}

/// Each item's row id in one database's playlist tree (`rows`), by tree id
/// (`UsbTreeNode::id`): a folder by its path, as the tree pairs folders; a
/// playlist by `playlist_tree_id`.
fn rows_by_tree_id(
    rows: &[PdbPlaylistTreeRow],
    playlist_tree_id: impl Fn(&PdbPlaylistTreeRow, &PlaylistPath) -> Option<String>,
) -> HashMap<String, u32> {
    let tree = PlaylistTree::new(rows);
    rows.iter()
        .filter_map(|row| {
            let path = tree.path(row);
            let id = if row.is_folder {
                let mut segments = path.folders;
                segments.push(path.name);
                playlist_tree::folder_id(&segments)
            } else {
                playlist_tree_id(row, &path)?
            };
            Some((id, row.id))
        })
        .collect()
}

/// The rows the save places in one database (`database`, for messages):
/// every item of each folder in the pending order, numbered from 0, folders
/// and playlists together, as rekordbox and export number them. A moved
/// item also gets its new folder's row. Refused when this database holds a
/// moved item but not its new folder.
fn row_places(
    pending: &PendingTree,
    nodes: &[UsbTreeNode],
    rows: &HashMap<String, u32>,
    database: &str,
) -> BackendResult<HashMap<u32, RowPlace>> {
    let name_of = |id: &str| {
        nodes
            .iter()
            .find(|node| node.id == id)
            .map_or_else(|| id.to_string(), |node| node.name.clone())
    };
    let mut places = HashMap::new();
    for (folder, ids) in &pending.order {
        for (position, id) in (0u32..).zip(ids) {
            let Some(&row) = rows.get(id) else {
                continue;
            };
            let parent = match pending.parents.get(id) {
                None => None,
                Some(to) if to == playlist_tree::TOP => Some(0),
                Some(to) => Some(*rows.get(to).ok_or_else(|| {
                    BackendError::Validation(tr::usb_order_edit_folder_missing(
                        name_of(id),
                        name_of(to),
                        database,
                    ))
                })?),
            };
            debug_assert!(parent.is_none() || pending.parents[id] == *folder);
            places.insert(
                row,
                RowPlace {
                    position,
                    parent,
                    name: name_of(id),
                },
            );
        }
    }
    Ok(places)
}

/// Writes `places` (by eDB `playlist_id`) to the eDB's playlist tree, in the
/// caller's transaction: `sequenceNo` and, for a moved row,
/// `playlist_id_parent`, both as export writes them. Returns the rows
/// changed.
fn write_edb_playlist_places(
    tx: &rusqlite::Transaction<'_>,
    places: &HashMap<u32, RowPlace>,
) -> BackendResult<usize> {
    let mut updated = 0;
    for (id, place) in places {
        updated += tx.execute(
            "UPDATE playlist SET sequenceNo = ?1 WHERE playlist_id = ?2 \
             AND sequenceNo IS NOT ?1",
            rusqlite::params![i64::from(place.position), i64::from(*id)],
        )?;
        if let Some(parent) = place.parent {
            updated += tx.execute(
                "UPDATE playlist SET playlist_id_parent = ?1 WHERE playlist_id = ?2 \
                 AND playlist_id_parent IS NOT ?1",
                rusqlite::params![i64::from(parent), i64::from(*id)],
            )?;
        }
    }
    Ok(updated)
}

/// The names of the rows of `rows` (one database's tree, as saved) that
/// aren't where `places` put them.
fn misplaced(rows: &[PdbPlaylistTreeRow], places: &HashMap<u32, RowPlace>) -> Vec<String> {
    let saved: HashMap<u32, &PdbPlaylistTreeRow> = rows.iter().map(|row| (row.id, row)).collect();
    places
        .iter()
        .filter(|(id, place)| {
            saved.get(id).is_none_or(|row| {
                row.sort_order != place.position
                    || place.parent.is_some_and(|parent| parent != row.parent_id)
            })
        })
        .map(|(_, place)| place.name.clone())
        .collect()
}

/// Checks, on the staged copies, that the save wrote what the edit asked
/// for: every placed row in each database at its place, in its folder, and
/// each edited playlist's tracks in the same order in the PDB and the eDB
/// (those both list; the parity report's comparison,
/// `build_usb_parity_comparison`, read only for those playlists). A
/// mismatch fails the save before any write-back, with a hint at the strict
/// parity repair.
fn verify_saved_order(
    usb_root: &Path,
    pending: &PendingOrder,
    parsed: Option<&ParsedPdb>,
    pdb_places: &HashMap<u32, RowPlace>,
    edb_places: &HashMap<u32, RowPlace>,
) -> BackendResult<()> {
    let mut differing = Vec::new();
    if let Some(parsed) = parsed {
        differing.extend(misplaced(&parsed.playlist_tree, pdb_places));
    }
    if UsbDatabases::of(usb_root)?.edb().is_some() {
        let conn = UsbDatabases::of(usb_root)?
            .open_edb_writable()
            .ok_or_else(|| BackendError::Validation(tr::usb_order_edit_edb_open_failed()))?;
        differing.extend(misplaced(
            &crate::edb::read_edb_playlist_tree(&conn)?,
            edb_places,
        ));
        if let Some(parsed) = parsed
            && !pending.tracks.is_empty()
        {
            let edb_playlists = crate::edb::read_edb(&conn, usb_root, crate::edb::EdbRead::ALL)
                .map(|library| library.resolved_playlists(usb_root))
                .unwrap_or_default();
            let (_, _, details, _) = build_usb_parity_comparison(
                parsed,
                &edb_playlists,
                &RatingColourParity::default(),
                &FieldNoteParity::default(),
                None,
            );
            let track_edited: HashSet<String> = pending
                .tracks
                .keys()
                .flat_map(|id| edb_playlist_names(id, Some(parsed)))
                .collect();
            differing.extend(
                details
                    .iter()
                    .filter(|detail| {
                        detail.order_mismatch
                            && track_edited.contains(&canonicalize_playlist_name(&detail.name))
                    })
                    .map(|detail| detail.name.clone()),
            );
        }
    }
    if differing.is_empty() {
        return Ok(());
    }
    differing.sort();
    differing.dedup();
    Err(BackendError::Validation(tr::usb_order_edit_verify_failed(
        differing.join(", "),
    )))
}

/// The edit of `usb_root`, when one is on and not being saved.
fn active_edit<'a>(
    slot: &'a mut Option<UsbOrderEdit>,
    usb_root: &Path,
) -> BackendResult<&'a mut UsbOrderEdit> {
    match slot.as_mut() {
        Some(edit) if edit.usb_root == usb_root && edit.saving => {
            Err(BackendError::Validation(tr::usb_order_edit_saving()))
        }
        Some(edit) if edit.usb_root == usb_root => Ok(edit),
        _ => Err(BackendError::Validation(tr::usb_order_edit_not_active())),
    }
}

/// Refuses a USB that can't be written, as `export_to_usb` does.
fn ensure_usb_writable(usb_root: &Path) -> BackendResult<()> {
    if can_write_file_at(usb_root) {
        return Ok(());
    }
    Err(BackendError::ValidationWithDetails(
        tr::usb_order_edit_read_only(),
        json!({
            "validationType": "usb_read_only",
            "statusText": tr::usb_order_edit_read_only().render(),
        }),
    ))
}

/// Holds an edit's `saving` mark and clears it however the save ends.
struct OrderSaveGuard<'a> {
    service: &'a BackendService,
    usb_root: PathBuf,
}

impl Drop for OrderSaveGuard<'_> {
    fn drop(&mut self) {
        if let Some(edit) = self
            .service
            .order_edit_slot()
            .as_mut()
            .filter(|edit| edit.usb_root == self.usb_root)
        {
            edit.saving = false;
        }
    }
}

impl BackendService {
    fn order_edit_slot(&self) -> MutexGuard<'_, Option<UsbOrderEdit>> {
        self.usb_order_edit
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The pending places of `usb_root`'s edit's playlists and folders;
    /// empty without one.
    pub(crate) fn pending_usb_playlist_order(&self, usb_root: &Path) -> PendingTree {
        self.order_edit_slot()
            .as_ref()
            .filter(|edit| edit.usb_root == usb_root)
            .map(|edit| edit.pending.tree.clone())
            .unwrap_or_default()
    }

    /// The pending track order of a playlist in `usb_root`'s edit, if it has one.
    pub(crate) fn pending_usb_track_order(
        &self,
        usb_root: &Path,
        playlist_id: &str,
    ) -> Option<Vec<String>> {
        self.order_edit_slot()
            .as_ref()
            .filter(|edit| edit.usb_root == usb_root)
            .and_then(|edit| edit.pending.tracks.get(playlist_id).cloned())
    }

    /// The order edit as the USB view shows it: inactive unless `usb_root` has one.
    pub(crate) fn usb_order_edit_state(&self, usb_root: &Path) -> UsbOrderEditState {
        self.order_edit_slot()
            .as_ref()
            .filter(|edit| edit.usb_root == usb_root)
            .map(order_edit_state)
            .unwrap_or_default()
    }

    /// Whether an order edit is open (for the selected USB: selecting another drops it).
    pub fn usb_order_edit_open(&self) -> bool {
        self.order_edit_slot().is_some()
    }

    /// Edit order's tooltip while the USB can't be written; `None` when it can.
    pub(crate) fn usb_order_edit_blocked_text(usb_root: &Path) -> Option<String> {
        (!has_write_access(usb_root)).then(|| tr::usb_order_edit_read_only().render())
    }

    /// `usb_root` (none: no USB) becomes the selected USB: another USB's edit
    /// is dropped. Refused while that edit is being saved.
    pub(crate) fn switch_usb_order_edit_root(&self, usb_root: Option<&Path>) -> BackendResult<()> {
        let mut slot = self.order_edit_slot();
        match slot.as_ref() {
            Some(edit) if Some(edit.usb_root.as_path()) != usb_root => {
                if edit.saving {
                    return Err(BackendError::Validation(tr::usb_order_edit_saving()));
                }
                *slot = None;
            }
            _ => {}
        }
        Ok(())
    }

    fn require_usb_order_edit(&self, usb_root: &Path) -> BackendResult<()> {
        active_edit(&mut self.order_edit_slot(), usb_root).map(|_| ())
    }

    /// Runs `change` on `usb_root`'s pending order and answers with the edit's state.
    fn change_usb_order_edit(
        &self,
        usb_root: &Path,
        change: impl FnOnce(&mut PendingOrder) -> BackendResult<()>,
    ) -> BackendResult<UsbOrderEditData> {
        let mut slot = self.order_edit_slot();
        let edit = active_edit(&mut slot, usb_root)?;
        change(&mut edit.pending)?;
        Ok(UsbOrderEditData {
            order_edit: order_edit_state(edit),
            ..UsbOrderEditData::default()
        })
    }

    /// The stick's playlist `playlist_id`, as the read resolved it.
    fn resolved_usb_playlist(
        &self,
        usb_root: &Path,
        playlist_id: &str,
    ) -> BackendResult<UsbPlaylist> {
        let resolved = self.usb_playlists_resolved(usb_root)?;
        resolved
            .items
            .iter()
            .find(|playlist| playlist.id == playlist_id)
            .cloned()
            .ok_or_else(|| BackendError::NotFound(tr::error_usb_playlist_not_found(playlist_id)))
    }

    /// Starts an order edit of the USB. Refused when the USB can't be written
    /// or a USB write is running. Already editing this USB: the edit goes on.
    pub fn begin_usb_order_edit(
        &self,
        req: UsbOrderEditRequest,
    ) -> BackendResult<UsbOrderEditData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        if self.try_lock_usb_writes().is_none() {
            return Err(BackendError::Validation(tr::usb_write_busy()));
        }
        ensure_usb_writable(&usb_root)?;
        let mut slot = self.order_edit_slot();
        match slot.as_ref() {
            Some(edit) if edit.saving => {
                return Err(BackendError::Validation(tr::usb_order_edit_saving()));
            }
            Some(edit) if edit.usb_root == usb_root => {}
            _ => {
                *slot = Some(UsbOrderEdit {
                    usb_root,
                    pending: PendingOrder::default(),
                    saving: false,
                });
            }
        }
        let edit = slot.as_ref().expect("the edit was just set");
        Ok(UsbOrderEditData {
            order_edit: order_edit_state(edit),
            status_text: tr::usb_order_edit_started().render(),
            ..UsbOrderEditData::default()
        })
    }

    /// Moves a playlist or a folder (with what it holds) in the pending
    /// order: into `into_folder_id` (at its end), before
    /// `before_playlist_id`, after `after_playlist_id` (in that item's
    /// folder), or to its own folder's end. See [`PendingTree::moved`] for
    /// what is refused.
    pub fn move_usb_playlist(
        &self,
        req: MoveUsbPlaylistRequest,
    ) -> BackendResult<UsbOrderEditData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        self.require_usb_order_edit(&usb_root)?;
        let resolved = self.usb_playlists_resolved(&usb_root)?;
        let nodes = &resolved.tree;
        let placement = match (
            &req.into_folder_id,
            &req.before_playlist_id,
            &req.after_playlist_id,
        ) {
            (Some(folder), _, _) => Placement::Inside(folder),
            (None, Some(before), _) => Placement::Before(before),
            (None, None, Some(after)) => Placement::After(after),
            (None, None, None) => Placement::End,
        };
        let mut data = self.change_usb_order_edit(&usb_root, |pending| {
            pending.tree = pending
                .tree
                .moved(nodes, &req.playlist_id, placement)
                .map_err(|refusal| match refusal {
                    MoveRefusal::NotFound(id) => {
                        BackendError::NotFound(tr::error_usb_playlist_not_found(&id))
                    }
                    MoveRefusal::NotAFolder => {
                        BackendError::Validation(tr::usb_order_edit_move_into_playlist())
                    }
                    MoveRefusal::IntoItself => {
                        BackendError::Validation(tr::usb_order_edit_move_into_itself())
                    }
                    MoveRefusal::NameTaken(name) => {
                        BackendError::Validation(tr::usb_order_edit_move_name_taken(name))
                    }
                })?;
            Ok(())
        })?;
        let pending = self.pending_usb_playlist_order(&usb_root);
        data.tree = Some(playlist_tree::tree_rows(
            &playlist_tree::walk(nodes, &pending),
            &pending,
        ));
        Ok(data)
    }

    /// Moves one row of a USB playlist in its pending track order. Not for a history.
    pub fn move_usb_playlist_track(
        &self,
        req: MoveUsbPlaylistTrackRequest,
    ) -> BackendResult<UsbOrderEditData> {
        if is_history_id(&req.playlist_id) {
            return Err(BackendError::Validation(
                tr::usb_order_edit_history_refused(),
            ));
        }
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        self.require_usb_order_edit(&usb_root)?;
        let playlist = self.resolved_usb_playlist(&usb_root, &req.playlist_id)?;
        let base: Vec<String> = playlist.tracks.iter().map(track_order_key).collect();
        self.change_usb_order_edit(&usb_root, |pending| {
            let mut rows: Vec<&UsbTrack> = playlist.tracks.iter().collect();
            apply_pending_order(
                &mut rows,
                pending
                    .tracks
                    .get(&playlist.id)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
                |track| track_order_key(track),
            );
            let position = |id: &str| {
                rows.iter()
                    .position(|track| track.id == id)
                    .ok_or_else(|| BackendError::NotFound(tr::usb_order_edit_track_not_found(id)))
            };
            let from = position(&req.move_track_id)?;
            let before = req.before_track_id.as_deref().map(position).transpose()?;
            move_before(&mut rows, from, before);
            let keys = rows.iter().map(|track| track_order_key(track)).collect();
            pending.set_tracks(&playlist.id, keys, &base);
            Ok(())
        })
    }

    /// Puts a whole USB playlist's pending order in a column sort, the one
    /// `fetch_usb_playlist_tracks` sorts a page with. Rows the sort ties keep
    /// their pending order.
    pub fn sort_usb_playlist_tracks(
        &self,
        req: SortUsbPlaylistTracksRequest,
    ) -> BackendResult<UsbOrderEditData> {
        if is_history_id(&req.playlist_id) {
            return Err(BackendError::Validation(
                tr::usb_order_edit_history_refused(),
            ));
        }
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        self.require_usb_order_edit(&usb_root)?;
        let playlist = self.resolved_usb_playlist(&usb_root, &req.playlist_id)?;
        let base: Vec<String> = playlist.tracks.iter().map(track_order_key).collect();
        self.change_usb_order_edit(&usb_root, |pending| {
            let mut rows = playlist.tracks.clone();
            apply_pending_order(
                &mut rows,
                pending
                    .tracks
                    .get(&playlist.id)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
                track_order_key,
            );
            crate::service::TrackSort::new(req.sort_by.as_deref(), req.sort_dir.as_deref())
                .apply(&mut rows);
            let keys = rows.iter().map(track_order_key).collect();
            pending.set_tracks(&playlist.id, keys, &base);
            Ok(())
        })
    }

    /// Drops the pending order. Never touches the stick, so it works after a
    /// disconnect too. Failed saves restore the databases before returning.
    pub fn cancel_usb_order_edit(
        &self,
        req: UsbOrderEditRequest,
    ) -> BackendResult<UsbOrderEditData> {
        // A disconnected stick has no root to resolve: its edit is the one there is.
        let usb_root = resolve_usb_root(req.usb_root.as_deref()).ok();
        let mut slot = self.order_edit_slot();
        if let Some(edit) = slot.as_ref()
            && usb_root.as_ref().is_none_or(|root| *root == edit.usb_root)
        {
            if edit.saving {
                return Err(BackendError::Validation(tr::usb_order_edit_saving()));
            }
            *slot = None;
        }
        Ok(UsbOrderEditData {
            status_text: tr::usb_order_edit_cancelled().render(),
            ..UsbOrderEditData::default()
        })
    }

    pub fn commit_usb_order_edit(
        &self,
        req: UsbOrderEditRequest,
    ) -> BackendResult<UsbOrderEditData> {
        self.commit_usb_order_edit_with_progress(req, |_, _, _| {})
    }

    /// Writes the pending order to the stick: one backup, then each database
    /// that holds an edited playlist patched in its staged copy and written
    /// back once, then one Engine DJ rebuild. Holds the USB write lock
    /// throughout, so export and the USB track saves wait their turn. A
    /// failure keeps the pending order (Save again retries it on fresh USB
    /// data) and drops the staged databases' changes.
    pub fn commit_usb_order_edit_with_progress<F>(
        &self,
        req: UsbOrderEditRequest,
        mut on_progress: F,
    ) -> BackendResult<UsbOrderEditData>
    where
        F: crate::progress::ProgressFn,
    {
        on_progress(0, 100, &tr::usb_progress_resolving_usb_root().render());
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let _usb_write_guard = self
            .try_lock_usb_writes()
            .ok_or_else(|| BackendError::Validation(tr::usb_write_busy()))?;
        let pending = {
            let mut slot = self.order_edit_slot();
            let edit = active_edit(&mut slot, &usb_root)?;
            edit.saving = true;
            edit.pending.clone()
        };
        let _saving = OrderSaveGuard {
            service: self,
            usb_root: usb_root.clone(),
        };
        ensure_usb_writable(&usb_root)?;
        if pending.is_empty() {
            *self.order_edit_slot() = None;
            return Ok(UsbOrderEditData {
                status_text: tr::usb_order_edit_unchanged().render(),
                ..UsbOrderEditData::default()
            });
        }

        let mut warnings = Vec::<WarningEntry>::new();
        let written = self.write_usb_order(
            &_usb_write_guard,
            &usb_root,
            &pending,
            &mut warnings,
            &mut on_progress,
        )?;
        if written {
            on_progress(90, 100, &tr::usb_progress_order_edit_engine().render());
            warnings.extend(self.refresh_engine_library(&usb_root));
        }
        *self.order_edit_slot() = None;
        on_progress(
            100,
            100,
            &tr::job_commit_usb_order_edit_completed().render(),
        );

        let warning_count = logging::count_warnings(&warnings);
        Ok(UsbOrderEditData {
            order_edit: UsbOrderEditState::default(),
            status_text: tr::usb_order_edit_saved().render(),
            warning_text: warning_text(warning_count),
            warnings,
            tree: None,
        })
    }

    /// The save's write: backup, patch the staged databases, write each
    /// changed one back. Returns whether a database was written.
    fn write_usb_order<F>(
        &self,
        usb_write_guard: &UsbWriteGuard<'_>,
        usb_root: &Path,
        pending: &PendingOrder,
        warnings: &mut Vec<WarningEntry>,
        on_progress: &mut F,
    ) -> BackendResult<bool>
    where
        F: crate::progress::ProgressFn,
    {
        on_progress(
            5,
            100,
            &tr::usb_progress_backing_up_usb_databases().render(),
        );
        let mut session = self.usb_write_session(
            usb_write_guard,
            usb_root,
            SessionBackup::ReplacingSameReason {
                reason: BACKUP_REASON,
            },
            warnings,
        );
        let databases = UsbDatabases::of(usb_root)?;

        on_progress(20, 100, &tr::usb_progress_order_edit_pdb().render());
        // A database missing from the stick is skipped.
        let (mut pdb, parsed) = if let Some(path) = session.pdb_path()? {
            let bytes = std::fs::read(&path)?;
            let parsed = parse_pdb_bytes(&bytes)?;
            (Some((path, bytes)), Some(parsed))
        } else {
            (None, None)
        };
        let parsed = parsed.as_ref();

        // Playlist places, from the stick's tree (what the moves used): each
        // item's row in each database, found before anything is written,
        // since a move changes the paths the rows are paired by.
        let resolved = self.usb_playlists_resolved(usb_root)?;
        let nodes = &resolved.tree;
        let pdb_places = match parsed {
            Some(parsed) => {
                let rows = rows_by_tree_id(&parsed.playlist_tree, |row, _| {
                    Some(format!("usb-pl-{}", row.id))
                });
                row_places(&pending.tree, nodes, &rows, "PDB")?
            }
            None => HashMap::new(),
        };

        // Track order in the PDB, from its own entries: each mapped to its
        // track's path and ordered by the pending list, renumbered from the
        // playlist's lowest entry index.
        let mut entry_patches = Vec::<(u32, u32, u32)>::new();
        if let Some(parsed) = parsed {
            let path_of: HashMap<u32, String> = parsed
                .tracks
                .iter()
                .map(|track| {
                    (
                        track.id,
                        canonicalize_track_path_identity(&track.track_file_path),
                    )
                })
                .collect();
            for (playlist_id, order) in &pending.tracks {
                let Some(pdb_id) = pdb_playlist_id(playlist_id) else {
                    continue;
                };
                let mut entries: Vec<&PdbPlaylistEntryRow> = parsed
                    .playlist_entries
                    .iter()
                    .filter(|entry| entry.playlist_id == pdb_id)
                    .collect();
                entries.sort_by_key(|entry| entry.entry_index);
                let Some(base) = entries.first().map(|entry| entry.entry_index) else {
                    continue;
                };
                let distinct: HashSet<u32> = entries.iter().map(|entry| entry.track_id).collect();
                if distinct.len() != entries.len() {
                    // The in-place patch finds an entry by (playlist, track).
                    warnings.push(logging::log_text(
                        Level::Warn,
                        "usb-order-edit",
                        "usb.order-edit.pdb-duplicate-entries",
                        &tr::usb_order_edit_warning_pdb_duplicates(playlist_id),
                    ));
                    continue;
                }
                apply_pending_order(&mut entries, order, |entry| {
                    path_of.get(&entry.track_id).cloned().unwrap_or_default()
                });
                entry_patches.extend(
                    entries
                        .iter()
                        .zip(base..)
                        .map(|(entry, index)| (pdb_id, entry.track_id, index)),
                );
            }
        }
        let mut pdb_changed = false;
        if let Some((_, bytes)) = pdb.as_mut() {
            let page_size = pdb_page_size(bytes)?;
            let patches: Vec<PdbPlaylistTreeSortOrderPatch> = pdb_places
                .iter()
                .map(|(id, place)| PdbPlaylistTreeSortOrderPatch {
                    id: *id,
                    sort_order: place.position,
                    parent_id: place.parent,
                })
                .collect();
            let placed = patch_playlist_tree_sort_orders_in_place(bytes, &patches, page_size)?;
            let entries = crate::pdb_writer::patch_t08_entry_indices_in_place(
                bytes,
                &entry_patches,
                page_size,
            );
            pdb_changed = placed + entries > 0;
        }

        // The eDB, from its own rows, in one transaction.
        on_progress(40, 100, &tr::usb_progress_order_edit_edb().render());
        let mut edb_changed = false;
        let mut edb_places = HashMap::new();
        if databases.edb().is_some() {
            let conn = session
                .edb()?
                .ok_or_else(|| BackendError::Validation(tr::usb_order_edit_edb_open_failed()))?;
            let tx = conn.transaction()?;
            // An eDB playlist is the tree's item whose path key it has.
            let tree_id_by_key: HashMap<String, &str> = nodes
                .iter()
                .filter(|node| node.kind == crate::models::PlaylistTreeKind::Playlist)
                .flat_map(|node| {
                    edb_playlist_names(&node.id, parsed)
                        .into_iter()
                        .map(|key| (key, node.id.as_str()))
                })
                .collect();
            let rows = rows_by_tree_id(&crate::edb::read_edb_playlist_tree(&tx)?, |_, path| {
                tree_id_by_key.get(&path.key()).map(|id| id.to_string())
            });
            edb_places = row_places(&pending.tree, nodes, &rows, "eDB")?;
            // Track order first: it finds playlists by their path, which
            // the places below may change.
            let mut updated = 0;
            for (playlist_id, order) in &pending.tracks {
                let names: HashSet<String> = edb_playlist_names(playlist_id, parsed)
                    .into_iter()
                    .collect();
                updated += write_edb_playlist_track_order(&tx, &names, |rows| {
                    apply_pending_order(rows, order, |(_, _, path)| {
                        canonicalize_track_path_identity(path)
                    });
                })?;
            }
            updated += write_edb_playlist_places(&tx, &edb_places)?;
            tx.commit()?;
            session.close_edb()?;
            edb_changed = updated > 0;
        }
        if pdb_changed && let Some((path, bytes)) = &pdb {
            std::fs::write(path, bytes)?;
        }

        // Nothing reaches the stick unless the databases now hold what the
        // edit asked for, the same in both.
        if pdb_changed || edb_changed {
            on_progress(60, 100, &tr::usb_progress_order_edit_verify().render());
            let saved = pdb
                .as_ref()
                .map(|(_, bytes)| parse_pdb_bytes(bytes))
                .transpose()?;
            verify_saved_order(usb_root, pending, saved.as_ref(), &pdb_places, &edb_places)?;
        }

        on_progress(70, 100, &tr::usb_progress_order_edit_write_back().render());
        session.commit()?;
        Ok(pdb_changed || edb_changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(folders: &[&str], name: &str) -> PlaylistPath {
        PlaylistPath {
            folders: folders.iter().map(|f| f.to_string()).collect(),
            name: name.to_string(),
        }
    }

    #[test]
    fn a_move_into_a_folder_one_database_lacks_is_refused_there() {
        let nodes = playlist_tree::build_usb_tree(
            [(path(&[], "Sets"), 0)],
            [
                ("usb-pl-2".to_string(), path(&[], "Top"), 1),
                ("usb-pl-3".to_string(), path(&["Sets"], "Warm"), 0),
            ],
        );
        let sets = playlist_tree::folder_id(&["Sets".to_string()]);
        let tree = PendingTree::default()
            .moved(&nodes, "usb-pl-2", Placement::Inside(&sets))
            .expect("into Sets");
        // A database with all three rows: each placed, "Top" in "Sets".
        let rows = HashMap::from([
            (sets.clone(), 1),
            ("usb-pl-2".to_string(), 2),
            ("usb-pl-3".to_string(), 3),
        ]);
        let places = row_places(&tree, &nodes, &rows, "PDB").expect("placed");
        assert_eq!(places[&2].parent, Some(1));
        assert_eq!((places[&2].position, places[&3].position), (1, 0));
        assert_eq!(places[&3].parent, None);
        // One without the folder can't take "Top" into it.
        let rows = HashMap::from([("usb-pl-2".to_string(), 2)]);
        let err = row_places(&tree, &nodes, &rows, "eDB").expect_err("no Sets row");
        let message = err.english();
        assert!(
            message.contains("Top") && message.contains("Sets") && message.contains("eDB"),
            "{message}"
        );
    }

    #[test]
    fn the_save_check_names_a_row_not_where_the_save_put_it() {
        let row = |id, parent_id, sort_order| PdbPlaylistTreeRow {
            id,
            parent_id,
            sort_order,
            is_folder: false,
            name: String::new(),
        };
        let place = |position, parent, name: &str| RowPlace {
            position,
            parent,
            name: name.to_string(),
        };
        let places = HashMap::from([(2, place(1, Some(1), "Top")), (3, place(0, None, "Warm"))]);
        // As placed: "Warm" keeps whatever folder it is in.
        assert!(misplaced(&[row(2, 1, 1), row(3, 7, 0)], &places).is_empty());
        // "Top" left in its old folder, though at its new place.
        assert_eq!(misplaced(&[row(2, 0, 1), row(3, 1, 0)], &places), ["Top"]);
        // A row that's gone, or at another place.
        let mut names = misplaced(&[row(3, 1, 5)], &places);
        names.sort();
        assert_eq!(names, ["Top", "Warm"]);
    }
}
