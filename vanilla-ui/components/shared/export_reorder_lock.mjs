// The reorder-lock rule (additive export never rewrites the order of entries
// already on the USB, so reordering a playlist that already exists there is a
// no-op) lives only in the backend now -- `playlist_locks_reorder_on_export`
// in backend/src/service/export.rs. `state.playlistUsbExportStatusById` carries
// the backend's `locksReorder` verdict; this module only renders it.

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
