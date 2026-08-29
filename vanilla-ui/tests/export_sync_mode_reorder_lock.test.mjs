import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { bindSettingsEvents } from "../components/settings/events.mjs";
import { applyPlaylistReorderLockToGrid } from "../components/shared/export_reorder_lock.mjs";

// Regression: changing the export-sync mode must release/engage the reorder lock
// on the currently open playlist immediately, without waiting for a USB rescan.

const HTML = `<!doctype html><body>
  <fieldset id="exportSyncModeGroup">
    <label><input type="radio" name="exportSyncMode" id="exportSyncModeMirror" value="mirror" /></label>
    <label><input type="radio" name="exportSyncMode" id="exportSyncModeAdditive" value="additive" checked /></label>
  </fieldset>
  <div data-track-grid data-body-id="playlistTracksBody" data-sort-locked="true">
    <div class="track-grid-cell sortable" role="columnheader" data-sort-key="title" data-tooltip="stale"></div>
    <div class="track-grid-cell sortable" role="columnheader" data-sort-key="artist" data-tooltip="stale"></div>
    <div id="playlistTracksBody"></div>
  </div>
</body>`;

const CONSTANTS = {
  STORAGE_KEY_HELP_SEEN: "help",
  STORAGE_KEY_EXPORT_PRUNE_STALE: "prune",
  STORAGE_KEY_EXPORT_BACKUP: "backup",
  STORAGE_KEY_BACKUP_RETENTION_COUNT: "backup_retention",
  STORAGE_KEY_ANALYSIS_BPM_RANGE: "bpm",
  STORAGE_KEY_ANALYSIS_ENGINE: "engine",
  FRONTEND_DB_KEY_HELP_SEEN: "ui_help_seen_v1",
  FRONTEND_DB_KEY_EXPORT_PRUNE_STALE: "ui_export_prune_stale_v1",
  FRONTEND_DB_KEY_EXPORT_BACKUP: "ui_export_backup_v1",
  FRONTEND_DB_KEY_BACKUP_RETENTION_COUNT: "ui_backup_retention_count_v1",
  FRONTEND_DB_KEY_ANALYSIS_BPM_RANGE: "ui_analysis_bpm_range_v1",
  FRONTEND_DB_KEY_ANALYSIS_ENGINE: "ui_analysis_engine_v1"
};

function setup({ sortActive = false } = {}) {
  const dom = new JSDOM(HTML);
  const doc = dom.window.document;
  const el = {
    exportSyncModeGroup: doc.querySelector("#exportSyncModeGroup"),
    exportSyncModeMirror: doc.querySelector("#exportSyncModeMirror"),
    exportSyncModeAdditive: doc.querySelector("#exportSyncModeAdditive"),
    playlistTracksBody: doc.querySelector("#playlistTracksBody")
  };
  const state = {
    exportPruneStale: false,
    playlistTrackSearch: "",
    playlists: [{ id: "p1", name: "Testi", tracks: [{ id: "t1" }, { id: "t2" }] }],
    currentPlaylistId: "p1",
    playlistUsbExportStatusById: new Map([
      ["p1", { playlistId: "p1", playlistName: "Testi", sameNameExistsOnUsb: true, locksReorder: true }]
    ])
  };
  const statuses = [];
  const commitCalls = [];
  const getCurrentPlaylist = () => state.playlists.find((p) => p.id === state.currentPlaylistId) || null;

  bindSettingsEvents({
    state,
    el,
    document: doc,
    window: dom.window,
    navigator: {},
    constants: CONSTANTS,
    persistSetting: () => {},
    setStatus: (message) => statuses.push(message),
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
        getCurrentPlaylist(),
        { searchActive: !!state.playlistTrackSearch },
        state.playlistUsbExportStatusById
      );
    },
    commitActivePlaylistSort: async (id) => {
      // Capture that the commit runs while the playlist is still unlocked.
      commitCalls.push({ id, locksReorderAtCommit: state.playlistUsbExportStatusById.get(id)?.locksReorder });
    },
    isPlaylistSortActive: () => sortActive
  });

  const grid = doc.querySelector("[data-track-grid]");
  const headers = () => [...doc.querySelectorAll('.sortable[role="columnheader"]')];
  const fire = (radio) => {
    el.exportSyncModeMirror.checked = radio === "mirror";
    el.exportSyncModeAdditive.checked = radio === "additive";
    const target = radio === "mirror" ? el.exportSyncModeMirror : el.exportSyncModeAdditive;
    target.dispatchEvent(new dom.window.Event("change", { bubbles: true }));
    return new Promise((resolve) => setTimeout(resolve, 0));
  };

  return { state, statuses, commitCalls, grid, headers, fire };
}

test("switching to mirror mode releases the open playlist's reorder lock", async () => {
  const { state, statuses, grid, headers, fire } = setup();

  await fire("mirror");

  assert.equal(state.exportPruneStale, true);
  assert.equal(state.playlistUsbExportStatusById.get("p1").locksReorder, false);
  assert.equal(state.playlistUsbExportStatusById.get("p1").sameNameExistsOnUsb, true);
  assert.equal(grid.dataset.sortLocked, "false");
  assert.ok(headers().every((h) => !h.hasAttribute("data-tooltip")));
  assert.ok(statuses.some((m) => m.includes("mirror")));
});

test("switching back to additive re-engages the lock for a same-named USB playlist", async () => {
  const { state, statuses, grid, headers, fire } = setup();

  await fire("mirror");
  await fire("additive");

  assert.equal(state.playlistUsbExportStatusById.get("p1").locksReorder, true);
  assert.equal(grid.dataset.sortLocked, "true");
  assert.ok(headers().every((h) => /Won't reorder on USB/.test(h.getAttribute("data-tooltip") || "")));
  assert.ok(statuses.some((m) => m.includes("locked here")));
});

test("engaging the lock commits an active column sort first, while still unlocked", async () => {
  const { commitCalls, fire } = setup({ sortActive: true });

  await fire("mirror"); // unlock -> no commit
  await fire("additive"); // lock -> commit the active sort first

  assert.equal(commitCalls.length, 1);
  assert.equal(commitCalls[0].id, "p1");
  assert.equal(commitCalls[0].locksReorderAtCommit, false);
});

test("no open playlist: mode change still recomputes the map without touching the DOM", async () => {
  const { state, grid, fire } = setup();
  state.currentPlaylistId = null;

  await fire("mirror");

  assert.equal(state.playlistUsbExportStatusById.get("p1").locksReorder, false);
  assert.equal(grid.dataset.sortLocked, "true"); // untouched — no render for a closed playlist
});
