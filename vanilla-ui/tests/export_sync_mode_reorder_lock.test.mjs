import test from "node:test";
import assert from "node:assert/strict";
import { bindSettingsEvents } from "../components/settings/events.mjs";
import { applyPlaylistReorderLockToGrid } from "../components/shared/export_reorder_lock.mjs";
import { makeTestCtx, withSilencedConsole } from "./test_helpers.mjs";

// Regression: changing the export-sync mode must release/engage the reorder lock
// on the currently open playlist immediately, without waiting for a USB rescan.

function setup({ sortActive = false, commitThrows = false } = {}) {
  const ctx = makeTestCtx();
  const { dom, el, document: doc } = ctx;
  const grid = doc.querySelector('[data-track-grid][data-body-id="playlistTracksBody"]');
  grid.dataset.sortLocked = "true";
  const state = {
    exportPruneStale: false,
    playlistTrackSearch: "",
    playlists: [{ id: "p1", name: "Testi", tracks: [{ id: "t1" }, { id: "t2" }] }],
    currentPlaylistId: "p1",
    playlistUsbExportStatusById: new Map([
      ["p1", { playlistId: "p1", playlistName: "Testi", usbSelected: true, sameNameExistsOnUsb: true, locksReorder: true }]
    ])
  };
  const statuses = [];
  const commitCalls = [];
  const persistCalls = [];
  const getCurrentPlaylist = () => state.playlists.find((p) => p.id === state.currentPlaylistId) || null;

  bindSettingsEvents(Object.assign(ctx, {
    state,
    persistSetting: (storageKey, dbKey, value) => persistCalls.push({ dbKey, value }),
    emitStatus: (message) => statuses.push(message),
    command: async () => {},
    getTauriEventListen: async () => null,
    setProgress: () => {},
    closeSettingsDrawer: () => {},
    switchView: async () => {},
    normalizeAnalysisBpmRange: (value) => value,
    updatePlaylistExportButtons: () => {},
    getCurrentPlaylist,
    // Stand-in for the real renderer: exercises the shared helper so the DOM
    // reflects the freshly recomputed lock, same as the live render path.
    renderCurrentPlaylistTracksFromState: async () => {
      applyPlaylistReorderLockToGrid(
        el,
        getCurrentPlaylist().id,
        { searchActive: !!state.playlistTrackSearch },
        state.playlistUsbExportStatusById
      );
    },
    commitActivePlaylistSort: async (id) => {
      // Capture that the commit runs while the playlist is still unlocked.
      commitCalls.push({ id, locksReorderAtCommit: state.playlistUsbExportStatusById.get(id)?.locksReorder });
      if (commitThrows) throw new Error("disk full");
    },
    isPlaylistSortActive: () => sortActive,
    // Stand-in for the `refresh_playlist_export_status` backend command: applies
    // the same rule the backend does (`!pruneStale && sameNameExistsOnUsb`).
    refreshPlaylistExportStatus: async () => {
      const next = new Map();
      for (const [id, entry] of state.playlistUsbExportStatusById) {
        next.set(id, {
          ...entry,
          locksReorder: !state.exportPruneStale && !!entry.sameNameExistsOnUsb
        });
      }
      state.playlistUsbExportStatusById = next;
    }
  }));

  const fire = (radio) => {
    el.exportSyncModeMirror.checked = radio === "mirror";
    el.exportSyncModeAdditive.checked = radio === "additive";
    const target = radio === "mirror" ? el.exportSyncModeMirror : el.exportSyncModeAdditive;
    target.dispatchEvent(new dom.window.Event("change", { bubbles: true }));
    return new Promise((resolve) => setTimeout(resolve, 0));
  };

  return { state, el, statuses, commitCalls, persistCalls, grid, fire };
}

test("the mode change still goes through when the pre-lock sort commit fails", async () => {
  const { state, statuses, persistCalls, grid, fire } = setup({ sortActive: true, commitThrows: true });

  await fire("mirror"); // unlock -> succeeds (no commit needed)
  persistCalls.length = 0;
  // The events handler logs the caught commit error via console.error -- expected.
  await withSilencedConsole(() => fire("additive")); // would lock -> commit throws -> mode change proceeds anyway

  assert.equal(state.exportPruneStale, false, "additive mode applied");
  assert.ok(persistCalls.some((c) => c.value === "0"), "additive is persisted");
  assert.equal(state.playlistUsbExportStatusById.get("p1").locksReorder, true, "lock engaged");
  assert.equal(grid.dataset.sortLocked, "true");
  assert.ok(statuses.some((m) => /couldn't save the current sort first/.test(m)));
});

test("no open playlist: mode change still recomputes the map without touching the DOM", async () => {
  const { state, grid, fire } = setup();
  state.currentPlaylistId = null;

  await fire("mirror");

  assert.equal(state.playlistUsbExportStatusById.get("p1").locksReorder, false);
  assert.equal(grid.dataset.sortLocked, "true"); // untouched — no render for a closed playlist
});
