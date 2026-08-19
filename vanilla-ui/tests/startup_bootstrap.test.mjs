import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  applySidebarCollapsedUi,
  hydrateAppVersionLabel,
  restoreStoredUiPrefs,
  runDeferredInitialLoad,
  showHelpOnFirstVisit
} from "../startup_bootstrap.mjs";

const prefConstants = {
  STORAGE_KEY_EXPORT_PRUNE_STALE: "prune",
  STORAGE_KEY_EXPORT_BACKUP: "backup",
  STORAGE_KEY_ANALYSIS_BPM_RANGE: "bpm",
  STORAGE_KEY_SIDEBAR_COLLAPSED: "sidebar"
};

function prefEls() {
  return {
    exportSyncModeMirror: { checked: false },
    exportSyncModeAdditive: { checked: false },
    exportBackupCheckbox: { checked: false },
    analysisBpmRangeSelect: { value: "" }
  };
}

function restorePrefs(state, el, values) {
  restoreStoredUiPrefs(state, el, {
    localStorageObj: { getItem: (key) => values[key] ?? null },
    constants: prefConstants,
    normalizeAnalysisBpmRange: (value) => value,
    defaultAnalysisBpmRange: "all"
  });
}

function deferredDeps(calls = [], overrides = {}) {
  return {
    setTimeoutFn: (cb) => cb(),
    withProgress: async (_label, fn) => {
      await fn((pct, text) => calls.push(`progress:${pct}:${text}`));
    },
    loadPlaylists: async () => { calls.push("playlists"); },
    resetAndLoadLibraryTracks: async () => { calls.push("tracks"); },
    libraryLoadLimitInit: 111,
    updateModeText: () => { calls.push("mode"); },
    updateSelectionCount: () => { calls.push("selection"); },
    renderUsbPlaylistTracks: () => { calls.push("usb"); },
    renderWaveformsIn: () => { calls.push("wave"); },
    documentObj: {},
    setStatus: () => {},
    logError: () => {},
    ...overrides
  };
}

test("hydrateAppVersionLabel uses fallback and tauri override", async () => {
  const dom = new JSDOM(`<!doctype html><body><span id="v"></span></body>`);
  const el = { settingsVersionText: dom.window.document.querySelector("#v") };

  for (const [tauriIsTauri, expected] of [[() => false, "0.1.0"], [() => true, "9.9.9"]]) {
    await hydrateAppVersionLabel(el, {
      appVersionFallback: "0.1.0",
      tauriIsTauri,
      tauriGetVersion: async () => "9.9.9"
    });
    assert.equal(el.settingsVersionText.textContent, `Version ${expected}`);
  }
});

test("restoreStoredUiPrefs reads stored controls and defaults backup to true", () => {
  const storedState = { exportPruneStale: true, exportBackup: true, analysisBpmRange: "", sidebarCollapsed: false };
  const storedEl = prefEls();
  restorePrefs(storedState, storedEl, { prune: "0", backup: "0", bpm: "club", sidebar: "1" });
  assert.equal(storedState.exportPruneStale, false);
  assert.equal(storedState.exportBackup, false);
  assert.equal(storedState.analysisBpmRange, "club");
  assert.equal(storedState.sidebarCollapsed, true);
  assert.equal(storedEl.exportSyncModeMirror.checked, false);
  assert.equal(storedEl.exportSyncModeAdditive.checked, true);
  assert.equal(storedEl.exportBackupCheckbox.checked, false);

  const defaultState = { exportPruneStale: true, exportBackup: false, analysisBpmRange: "", sidebarCollapsed: false };
  const defaultEl = prefEls();
  restorePrefs(defaultState, defaultEl, {});
  assert.equal(defaultState.exportBackup, true);
  assert.equal(defaultEl.exportBackupCheckbox.checked, true);
});

test("applySidebarCollapsedUi and showHelpOnFirstVisit update DOM", () => {
  const dom = new JSDOM(`<!doctype html><body><div id="nav"></div><div id="help" class="hidden"></div></body>`);
  const el = {
    navSidebar: dom.window.document.querySelector("#nav"),
    helpOverlay: dom.window.document.querySelector("#help")
  };
  const btn = dom.window.document.createElement("button");
  applySidebarCollapsedUi({ sidebarCollapsed: true }, el, { sidebarExpandBtn: btn });
  showHelpOnFirstVisit(el, {
    localStorageObj: { getItem: () => null },
    storageKeyHelpSeen: "help"
  });

  assert.equal(el.navSidebar.classList.contains("collapsed"), true);
  assert.equal(btn.classList.contains("visible"), true);
  assert.equal(el.helpOverlay.classList.contains("hidden"), false);
});

test("runDeferredInitialLoad loads initial data, selects fallback playlists, and preserves valid current playlists", async () => {
  const calls = [];
  const first = { playlists: [{ id: "p1" }], currentPlaylistId: null, startupPhase: true };
  runDeferredInitialLoad(first, deferredDeps(calls));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(first.currentPlaylistId, "p1");
  assert.equal(first.startupPhase, false);
  assert.equal(calls.includes("playlists"), true);
  assert.equal(calls.includes("tracks"), true);

  const existing = {
    playlists: [{ id: "p1" }, { id: "p2" }],
    currentPlaylistId: "p2",
    startupPhase: true
  };
  runDeferredInitialLoad(existing, deferredDeps([], {
    loadPlaylists: async () => {},
    resetAndLoadLibraryTracks: async () => {}
  }));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(existing.currentPlaylistId, "p2");
  assert.equal(existing.startupPhase, false);
});
