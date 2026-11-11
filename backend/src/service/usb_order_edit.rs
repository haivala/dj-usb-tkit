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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{MutexGuard, PoisonError};

use serde_json::json;

use crate::error::{BackendError, BackendResult};
use crate::i18n::tr;
use crate::logging::{self, Level};
use crate::models::{
    MoveUsbPlaylistRequest, MoveUsbPlaylistTrackRequest, SortUsbPlaylistTracksRequest,
    UsbOrderEditData, UsbOrderEditRequest, UsbOrderEditState, UsbPlaylist, UsbTrack, WarningEntry,
};
use crate::pdb_reader::{ParsedPdb, PdbPlaylistEntryRow, parse_pdb_bytes, pdb_page_size};
use crate::utils::warning_text;

use super::BackendService;
use super::export_helpers::canonicalize_track_path_identity;
use super::repair::{
    restore_pdb_playlist_sort_orders, write_edb_playlist_order, write_edb_playlist_track_order,
};
use super::usb_staging::{self, DbKind};
use super::usb_utils::{
    can_write_file_at, canonicalize_playlist_name, has_write_access, resolve_usb_root,
};
use super::usb_vendor_compat::{vendor_edb_path, vendor_pdb_path};

/// The reason recorded on the backup a save takes. A save whose stick's
/// newest backup has this reason rewrites that one instead of adding one.
const BACKUP_REASON: &str = "Before order edit";

/// The pending order: what a save would write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PendingOrder {
    /// USB playlist ids (`UsbPlaylist::id`) in the new order; `None` while the
    /// list is in the stick's order.
    playlists: Option<Vec<String>>,
    /// Per edited playlist (`UsbPlaylist::id`), the media path key
    /// ([`track_order_key`]) of each of its rows in the new order.
    tracks: HashMap<String, Vec<String>>,
}

impl PendingOrder {
    fn is_empty(&self) -> bool {
        self.playlists.is_none() && self.tracks.is_empty()
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
    /// A save wrote one database and then failed to write the other.
    partially_saved: bool,
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
fn move_before<T>(rows: &mut Vec<T>, from: usize, before: Option<usize>) {
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

/// The canonical names an eDB playlist can have to be this USB playlist, as
/// the playlist read pairs them: an eDB-only playlist's own name, a PDB
/// playlist's leaf name and its "Folder / Leaf" name.
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
        .find(|row| row.id == pdb_id && !row.row_is_folder)
    else {
        return Vec::new();
    };
    let mut names = vec![canonicalize_playlist_name(&leaf.name)];
    if let Some(folder) = parsed.playlist_tree.iter().find(|p| p.id == leaf.parent_id) {
        names.push(canonicalize_playlist_name(&format!(
            "{} / {}",
            folder.name, leaf.name
        )));
    }
    names
}

fn order_edit_state(edit: &UsbOrderEdit) -> UsbOrderEditState {
    let mut edited_playlist_ids: Vec<String> = edit.pending.tracks.keys().cloned().collect();
    edited_playlist_ids.sort();
    let count = edited_playlist_ids.len();
    let editing_text = match (edit.pending.playlists.is_some(), count) {
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

/// Why a save failed, and whether a database was written before it did.
struct OrderSaveFailure {
    error: BackendError,
    partial: bool,
}

impl From<BackendError> for OrderSaveFailure {
    fn from(error: BackendError) -> Self {
        Self {
            error,
            partial: false,
        }
    }
}

impl From<rusqlite::Error> for OrderSaveFailure {
    fn from(error: rusqlite::Error) -> Self {
        BackendError::from(error).into()
    }
}

impl From<std::io::Error> for OrderSaveFailure {
    fn from(error: std::io::Error) -> Self {
        BackendError::from(error).into()
    }
}

fn db_label(kind: DbKind) -> &'static str {
    match kind {
        DbKind::Pdb => "PDB",
        DbKind::Edb => "eDB",
    }
}

/// Checkpoints the eDB's write-ahead log into its main file: the staging
/// write-back copies only that file. An incomplete checkpoint fails the save.
fn checkpoint_edb(conn: &rusqlite::Connection) -> BackendResult<()> {
    // A reader still on an older snapshot gets this long to finish.
    conn.busy_timeout(std::time::Duration::from_secs(1))?;
    let (busy, log_frames, checkpointed) =
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
    if busy != 0 || (log_frames >= 0 && checkpointed != log_frames) {
        return Err(BackendError::Validation(
            tr::usb_order_edit_edb_checkpoint_failed(),
        ));
    }
    Ok(())
}

/// After a failed save: drops the staged databases' local changes and stages
/// them again from the stick, so no later writer reuses them. With the stick
/// gone, the local copies are only removed.
fn discard_staged_order_changes(usb_root: &Path) {
    for kind in [DbKind::Pdb, DbKind::Edb] {
        if let Err(err) = usb_staging::restage_from_usb(usb_root, kind) {
            logging::debug(
                "usb-order-edit",
                format!(
                    "Dropping the staged {} after a failed save failed: {}",
                    db_label(kind),
                    err.english()
                ),
            );
        }
    }
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

    /// The pending playlist order of `usb_root`'s edit, if it has one.
    pub(crate) fn pending_usb_playlist_order(&self, usb_root: &Path) -> Option<Vec<String>> {
        self.order_edit_slot()
            .as_ref()
            .filter(|edit| edit.usb_root == usb_root)
            .and_then(|edit| edit.pending.playlists.clone())
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
        if matches!(
            self.usb_write_lock.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ) {
            return Err(BackendError::Validation(tr::usb_order_edit_busy()));
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
                    partially_saved: false,
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

    /// Moves a playlist in the pending playlist order.
    pub fn move_usb_playlist(
        &self,
        req: MoveUsbPlaylistRequest,
    ) -> BackendResult<UsbOrderEditData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        self.require_usb_order_edit(&usb_root)?;
        let resolved = self.usb_playlists_resolved(&usb_root)?;
        let base: Vec<String> = resolved.items.iter().map(|p| p.id.clone()).collect();
        self.change_usb_order_edit(&usb_root, |pending| {
            let mut order = base.clone();
            apply_pending_order(
                &mut order,
                pending.playlists.as_deref().unwrap_or_default(),
                Clone::clone,
            );
            let position = |id: &str| {
                order
                    .iter()
                    .position(|other| other == id)
                    .ok_or_else(|| BackendError::NotFound(tr::error_usb_playlist_not_found(id)))
            };
            let from = position(&req.playlist_id)?;
            let before = req
                .before_playlist_id
                .as_deref()
                .map(position)
                .transpose()?;
            move_before(&mut order, from, before);
            pending.playlists = (order != base).then_some(order);
            Ok(())
        })
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
            super::TrackSort::new(req.sort_by.as_deref(), req.sort_dir.as_deref()).apply(&mut rows);
            let keys = rows.iter().map(track_order_key).collect();
            pending.set_tracks(&playlist.id, keys, &base);
            Ok(())
        })
    }

    /// Drops the pending order. Never touches the stick, so it works after a
    /// disconnect too; a save that wrote one database already stays written.
    pub fn cancel_usb_order_edit(
        &self,
        req: UsbOrderEditRequest,
    ) -> BackendResult<UsbOrderEditData> {
        // A disconnected stick has no root to resolve: its edit is the one there is.
        let usb_root = resolve_usb_root(req.usb_root.as_deref()).ok();
        let mut slot = self.order_edit_slot();
        let mut status = tr::usb_order_edit_cancelled();
        if let Some(edit) = slot.as_ref()
            && usb_root.as_ref().is_none_or(|root| *root == edit.usb_root)
        {
            if edit.saving {
                return Err(BackendError::Validation(tr::usb_order_edit_saving()));
            }
            if edit.partially_saved {
                status = tr::usb_order_edit_cancelled_after_partial_save();
            }
            *slot = None;
        }
        Ok(UsbOrderEditData {
            status_text: status.render(),
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
        F: FnMut(usize, usize, &str),
    {
        on_progress(0, 100, &tr::usb_progress_resolving_usb_root().render());
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let _usb_write_guard = match self.usb_write_lock.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(BackendError::Validation(tr::usb_order_edit_busy()));
            }
        };
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
        let written =
            match self.write_usb_order(&usb_root, &pending, &mut warnings, &mut on_progress) {
                Ok(written) => written,
                Err(failure) => {
                    discard_staged_order_changes(&usb_root);
                    self.invalidate_usb_parse_cache();
                    if failure.partial
                        && let Some(edit) = self.order_edit_slot().as_mut()
                    {
                        edit.partially_saved = true;
                    }
                    return Err(failure.error);
                }
            };
        if written {
            on_progress(90, 100, &tr::usb_progress_order_edit_engine().render());
            warnings.extend(self.refresh_engine_library(&usb_root));
        }
        self.invalidate_usb_parse_cache();
        *self.order_edit_slot() = None;
        on_progress(100, 100, &tr::usb_progress_order_edit_saved().render());

        let warning_count = logging::count_warnings(&warnings);
        Ok(UsbOrderEditData {
            order_edit: UsbOrderEditState::default(),
            status_text: tr::usb_order_edit_saved().render(),
            warning_text: warning_text(warning_count),
            warnings,
        })
    }

    /// The save's write: backup, patch the staged databases, write each
    /// changed one back. Returns whether a database was written.
    fn write_usb_order<F>(
        &self,
        usb_root: &Path,
        pending: &PendingOrder,
        warnings: &mut Vec<WarningEntry>,
        on_progress: &mut F,
    ) -> Result<bool, OrderSaveFailure>
    where
        F: FnMut(usize, usize, &str),
    {
        on_progress(
            5,
            100,
            &tr::usb_progress_backing_up_usb_databases().render(),
        );
        warnings.extend(self.backup_usb_databases_replacing_same_reason(usb_root, BACKUP_REASON));

        on_progress(20, 100, &tr::usb_progress_order_edit_pdb().render());
        // A database missing from the stick is skipped; a local copy left from
        // an earlier session isn't this stick's.
        let (mut pdb, parsed) = if vendor_pdb_path(usb_root).is_file() {
            let path = usb_staging::stage_pdb(usb_root)?;
            let bytes = std::fs::read(&path)?;
            let parsed = parse_pdb_bytes(&bytes)?;
            (Some((path, bytes)), Some(parsed))
        } else {
            (None, None)
        };
        let parsed = parsed.as_ref();

        // Playlist order: every playlist's place in the whole list, in both
        // databases, so a read sorts rows from either database together.
        let mut pdb_sort = HashMap::<u32, u32>::new();
        let mut edb_sort = HashMap::<String, u32>::new();
        for (position, id) in (0u32..).zip(pending.playlists.iter().flatten()) {
            if let Some(pdb_id) = pdb_playlist_id(id) {
                pdb_sort.insert(pdb_id, position);
            }
            for name in edb_playlist_names(id, parsed) {
                edb_sort.insert(name, position);
            }
        }

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
            let sorted = restore_pdb_playlist_sort_orders(bytes, &pdb_sort)?;
            let page_size = pdb_page_size(bytes)?;
            let entries = crate::pdb_writer::patch_t08_entry_indices_in_place(
                bytes,
                &entry_patches,
                page_size,
            );
            pdb_changed = sorted + entries > 0;
        }

        // The eDB, from its own rows, in one transaction.
        on_progress(40, 100, &tr::usb_progress_order_edit_edb().render());
        let mut edb_changed = false;
        if vendor_edb_path(usb_root).is_file() {
            let mut conn = crate::edb::open_edb_rw(usb_root)
                .ok_or_else(|| BackendError::Validation(tr::usb_order_edit_edb_open_failed()))?;
            let tx = conn.transaction()?;
            let mut updated = write_edb_playlist_order(&tx, &edb_sort)?;
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
            tx.commit()?;
            checkpoint_edb(&conn)?;
            let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
            drop(conn);
            edb_changed = updated > 0;
        }
        if pdb_changed && let Some((path, bytes)) = &pdb {
            std::fs::write(path, bytes)?;
        }

        // Every write-back is checked before any starts, so a stick changed
        // elsewhere fails the save before it writes anything.
        on_progress(70, 100, &tr::usb_progress_order_edit_write_back().render());
        let kinds: Vec<DbKind> = [(pdb_changed, DbKind::Pdb), (edb_changed, DbKind::Edb)]
            .into_iter()
            .filter_map(|(changed, kind)| changed.then_some(kind))
            .collect();
        for kind in &kinds {
            usb_staging::check_write_back(usb_root, *kind)?;
        }
        let mut written: Option<DbKind> = None;
        for kind in kinds {
            if let Err(error) = usb_staging::write_back_if_changed(usb_root, kind) {
                return Err(match written {
                    Some(saved) => OrderSaveFailure {
                        error: BackendError::Validation(tr::usb_order_edit_partial_save(
                            db_label(saved),
                            db_label(kind),
                            &error,
                        )),
                        partial: true,
                    },
                    None => error.into(),
                });
            }
            written = Some(kind);
        }
        Ok(pdb_changed || edb_changed)
    }
}
