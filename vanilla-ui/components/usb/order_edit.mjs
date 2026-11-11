import { t } from "../../ui_texts.mjs";
import { clearTrackSort, renderTrackSortHeader } from "../shell/actions.mjs";
import { moveArrayItem } from "../shared/list_drag_reorder.mjs";
import { clearUsbDiagnostics, isUsbRootChangeBlocked, refreshUsb, renderUsbPlaylists } from "./actions.mjs";

// Edit order: the USB playlists' order and the order of the tracks in them,
// edited as one pending change the backend holds (`begin_usb_order_edit` …
// `commit_usb_order_edit`). Nothing is written to the stick until Save order.
// `state.usbOrderEdit` is the backend's `UsbOrderEditState`, as last answered.

const BODY_ID = "usbPlaylistTracks";

export function isUsbOrderEditing(state) {
  return !!state.usbOrderEdit?.active;
}

// The edit lives on the USB Playlists page, which can't be left with it on.
// What can still be reached from there and writes the USB (renaming the drive
// from the header) waits for it to end: true, with the reason on the status
// line, when it must not start.
export function refuseWhileUsbOrderEditing(ctx) {
  if (!isUsbOrderEditing(ctx.state)) return false;
  ctx.emitStatus(t("ui-usb-playlists-order-edit-blocks-jobs"));
  return true;
}

// Stores the backend's edit state (and, from a list read, why Edit order can't
// start) and re-renders what shows it.
export function applyUsbOrderEdit(ctx, orderEdit, blockedText) {
  const { state } = ctx;
  const wasEditing = isUsbOrderEditing(state);
  state.usbOrderEdit = orderEdit || { active: false };
  if (blockedText !== undefined) state.usbOrderEditBlockedText = blockedText || null;
  const edited = new Set(state.usbOrderEdit.editedPlaylistIds || []);
  for (const playlist of state.usbPlaylists) playlist.orderEdited = edited.has(String(playlist.id));
  if (wasEditing && !isUsbOrderEditing(state)) forgetUsbOrderSort(ctx);
  renderUsbOrderEditControls(ctx);
  renderUsbPlaylists(ctx);
  // The track rows' drag handles show only while editing.
  if (wasEditing !== isUsbOrderEditing(state)) ctx.usbPlaylistTracksCtl.rerender();
}

export function renderUsbOrderEditControls(ctx) {
  const { state, el } = ctx;
  const editing = isUsbOrderEditing(state);
  const busy = isUsbRootChangeBlocked(state);
  el.usbOrderEditBar.classList.toggle("hidden", !state.usbRootValid && !editing);
  el.usbOrderEditText.textContent = editing ? state.usbOrderEdit.editingText || "" : "";
  el.usbOrderEditText.classList.toggle("hidden", !editing);
  el.usbEditOrderBtn.classList.toggle("hidden", editing);
  el.usbCancelOrderBtn.classList.toggle("hidden", !editing);
  el.usbSaveOrderBtn.classList.toggle("hidden", !editing);
  const blocked = state.usbOrderEditBlockedText || (busy ? t("ui-usb-playlists-edit-order-busy") : null);
  el.usbEditOrderBtn.disabled = !!blocked;
  el.usbEditOrderBtn.dataset.tooltip = blocked || t("ui-usb-playlists-edit-order-tooltip");
  el.usbSaveOrderBtn.disabled = busy || !state.usbOrderEdit?.changed;
  el.usbCancelOrderBtn.disabled = busy;
  el.usbPlaylists.classList.toggle("is-order-editing", editing);
  // No column changes while editing (track_columns.mjs).
  ctx.renderTrackColumnsButtons();
}

export async function beginUsbOrderEdit(ctx) {
  const { state } = ctx;
  const data = await ctx.command("begin_usb_order_edit", { usbRoot: state.usbRoot });
  // A view sort already on the open playlist stays a view sort: it becomes
  // the playlist's order only when a drag starts (commitSort).
  applyUsbOrderEdit(ctx, data.orderEdit);
  ctx.emitStatus(data.statusText, { warningText: data.warningText });
}

// Drops the pending order. `status` false: no status line (the caller has its own).
export async function cancelUsbOrderEdit(ctx, { status = true } = {}) {
  const data = await ctx.command("cancel_usb_order_edit", { usbRoot: ctx.state.usbRoot });
  applyUsbOrderEdit(ctx, data.orderEdit);
  // Back to the order on the stick.
  await refreshUsb(ctx).catch((err) => console.error("USB playlist re-read failed:", err));
  if (status) ctx.emitStatus(data.statusText, { warningText: data.warningText });
}

// Writes the pending order to the stick. True when it was saved; on a failure
// the edit stays on (Save again retries, Cancel drops it) and the error shows.
export async function saveUsbOrderEdit(ctx) {
  let data;
  try {
    data = await ctx.command("commit_usb_order_edit", { usbRoot: ctx.state.usbRoot });
  } catch (err) {
    // The list and the edit state as the backend has them now.
    await refreshUsb(ctx).catch((readErr) => console.error("USB playlist re-read failed:", readErr));
    ctx.emitStatus(err?.message || String(err));
    return false;
  }
  applyUsbOrderEdit(ctx, data.orderEdit);
  clearUsbDiagnostics(ctx);
  await refreshUsb(ctx).catch((err) => console.error("USB playlist re-read failed:", err));
  ctx.emitStatus(data.statusText, { warningText: data.warningText });
  return true;
}

// Leaving the USB Playlists page with an edit on, for any other page. With
// nothing changed the edit just ends; otherwise the user saves, discards or
// stays. True when the caller may go ahead.
export async function settleUsbOrderEditBeforeLeaving(ctx) {
  const { state } = ctx;
  if (!isUsbOrderEditing(state)) return true;
  if (!state.usbOrderEdit.changed) {
    await cancelUsbOrderEdit(ctx, { status: false });
    return true;
  }
  const choice = await ctx.confirmDialog.openChoice({
    title: t("ui-usb-playlists-leave-title"),
    message: t("ui-usb-playlists-leave-message"),
    confirmLabel: t("ui-usb-playlists-save-order"),
    altLabel: t("ui-usb-playlists-leave-discard"),
    cancelLabel: t("ui-usb-playlists-leave-stay"),
  });
  if (choice === "confirm") return saveUsbOrderEdit(ctx);
  if (choice === "alt") {
    await cancelUsbOrderEdit(ctx);
    return true;
  }
  return false;
}

// A playlist row dropped at a new index in the list.
export async function moveUsbPlaylistInOrder(ctx, fromIndex, toIndex) {
  const { state } = ctx;
  const moved = moveArrayItem(state.usbPlaylists, fromIndex, toIndex);
  const playlistId = moved[toIndex].id;
  const beforePlaylistId = moved[toIndex + 1]?.id ?? null;
  try {
    const data = await ctx.command("move_usb_playlist", { usbRoot: state.usbRoot, playlistId, beforePlaylistId });
    state.usbPlaylists = moved;
    applyUsbOrderEdit(ctx, data.orderEdit);
  } catch (err) {
    // The drag already moved the row: read the order back.
    await refreshUsb(ctx).catch((readErr) => console.error("USB playlist re-read failed:", readErr));
    ctx.emitStatus(err?.message || String(err));
  }
}

// --- The open playlist's track order while editing ----------------------
//
// A column-sort click while editing makes that sort the playlist's pending
// order (`sort_usb_playlist_tracks`); the pages are then fetched in natural
// order, which is the pending one. The header keeps showing the choice so the
// next click goes on through the header's cycle; `state.usbOrderSortChoice` is
// the last choice the backend took, and `ctl.sortBy` stays null: the header
// isn't a view sort then. A view sort carried into the edit (`ctl.sortBy` set)
// is committed when a drag starts.

function usbPlaylistGrid(ctx) {
  return ctx.el[BODY_ID].closest("[data-track-grid]");
}

// Forgets the header choice (after a drag, a playlist switch, the edit's end).
export function forgetUsbOrderSort(ctx) {
  ctx.state.usbOrderSortChoice = null;
  if (ctx.usbPlaylistTracksCtl.sortBy) return; // a view sort: still the view
  clearTrackSort(ctx.tableSortState, BODY_ID, usbPlaylistGrid(ctx));
}

// A view sort is on (fetches come sorted): what a drag commits first.
export function isUsbViewSortActive(ctx) {
  return !!ctx.usbPlaylistTracksCtl.sortBy;
}

async function sortPendingOrder(ctx, sort) {
  const { state } = ctx;
  const ctl = ctx.usbPlaylistTracksCtl;
  const data = await ctx.command("sort_usb_playlist_tracks", {
    usbRoot: state.usbRoot,
    playlistId: ctl.scopeId,
    sortBy: sort.key,
    sortDir: sort.dir || "asc",
  });
  applyUsbOrderEdit(ctx, data.orderEdit);
}

// The header was clicked while editing: `tableSortState` already holds the
// next choice of the header's cycle (none: back to the unsorted state).
export async function applyUsbOrderSortClick(ctx) {
  const { state } = ctx;
  const ctl = ctx.usbPlaylistTracksCtl;
  const sort = ctx.tableSortState[BODY_ID] || null;
  try {
    if (sort && ctl.scopeId != null) await sortPendingOrder(ctx, sort);
  } catch (err) {
    // Back to the last choice the backend took.
    const previous = state.usbOrderSortChoice;
    if (previous) ctx.tableSortState[BODY_ID] = previous;
    else delete ctx.tableSortState[BODY_ID];
    renderTrackSortHeader(usbPlaylistGrid(ctx), previous);
    ctx.emitStatus(err?.message || String(err));
    return;
  }
  // The unsorted state leaves the pending order as it is.
  state.usbOrderSortChoice = sort;
  ctl.sortBy = null;
  ctl.sortDir = null;
  await ctl.reload();
}

// Dragstart with a view sort on: that sorted order becomes the pending order
// first, so the drop moves the row among what's on screen.
export async function commitUsbViewSort(ctx) {
  const ctl = ctx.usbPlaylistTracksCtl;
  const sort = ctl.sortBy ? { key: ctl.sortBy, dir: ctl.sortDir } : null;
  ctl.sortBy = null;
  ctl.sortDir = null;
  clearTrackSort(ctx.tableSortState, BODY_ID, usbPlaylistGrid(ctx));
  ctx.state.usbOrderSortChoice = null;
  if (sort && ctl.scopeId != null) await sortPendingOrder(ctx, sort);
}

export async function moveUsbPlaylistTrackInOrder(ctx, { moveTrackId, beforeTrackId }) {
  const { state } = ctx;
  const data = await ctx.command("move_usb_playlist_track", {
    usbRoot: state.usbRoot,
    playlistId: ctx.usbPlaylistTracksCtl.scopeId,
    moveTrackId,
    beforeTrackId,
  });
  applyUsbOrderEdit(ctx, data.orderEdit);
  forgetUsbOrderSort(ctx);
}
