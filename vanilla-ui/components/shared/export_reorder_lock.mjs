// Single frontend copy of the backend reorder-lock rule
// (playlist_locks_reorder_on_export in backend/src/service/export.rs): additive
// export never rewrites the order of entries already on the USB, so reordering a
// playlist that already exists there is a no-op and the track list is locked.
export function playlistLocksReorderOnExport(pruneStale, sameNameExistsOnUsb) {
  return !pruneStale && !!sameNameExistsOnUsb;
}

// Returns a NEW Map with shallow-cloned entries whose locksReorder is re-derived
// from the cached sameNameExistsOnUsb and the current pruneStale setting. A fresh
// Map (rather than in-place mutation) keeps entry objects held elsewhere -- e.g.
// the diagnostics report -- from changing under them, and matches the style of
// playlistUsbExportStatusById() in components/usb/actions.mjs.
export function recomputeReorderLocks(statusById, pruneStale) {
  const next = new Map();
  if (statusById instanceof Map) {
    for (const [id, entry] of statusById) {
      next.set(id, {
        ...entry,
        locksReorder: playlistLocksReorderOnExport(pruneStale, entry?.sameNameExistsOnUsb)
      });
    }
  }
  return next;
}

export function exportReorderLockTooltip(playlistName) {
  return `Won't reorder on USB — "${playlistName}" already exists there, and additive export keeps its existing track order unchanged. New tracks are still added in your chosen order.`;
}

// Applies the reorder lock to the currently rendered playlist track grid: sets
// data-sort-locked on the [data-track-grid] wrapper (which gates the column-sort
// header click) and the "won't reorder" tooltip on the sortable headers. Returns
// the two values renderTrackTable() consumes. Kept free of any `state` dependency
// -- callers pass state.playlistUsbExportStatusById as statusById.
export function applyPlaylistReorderLockToGrid(el, playlist, { searchActive } = {}, statusById) {
  const locksReorder = !!(statusById instanceof Map && statusById.get(playlist.id)?.locksReorder);
  const enableDragReorder = !searchActive && !locksReorder;
  const tooltip = locksReorder ? exportReorderLockTooltip(playlist.name) : null;
  const dragDisabledTooltip = !searchActive ? tooltip : null;
  const grid = el.playlistTracksBody?.closest("[data-track-grid]");
  if (grid) {
    grid.dataset.sortLocked = locksReorder ? "true" : "false";
    grid.querySelectorAll('.sortable[role="columnheader"]').forEach((h) => {
      if (tooltip) {
        h.setAttribute("data-tooltip", tooltip);
      } else {
        h.removeAttribute("data-tooltip");
      }
    });
  }
  return { enableDragReorder, dragDisabledTooltip };
}
