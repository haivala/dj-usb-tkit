import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  hydrateAppVersionLabel,
  restoreStoredUiPrefs,
  applySidebarCollapsedUi,
  showHelpOnFirstVisit,
  runDeferredInitialLoad
} from "../startup_bootstrap.mjs";

test("hydrateAppVersionLabel uses fallback and tauri override", async () => {
  const dom = new JSDOM(`<!doctype html><body><span id="v"></span></body>`);
  const el = { settingsVersionText: dom.window.document.querySelector("#v") };
  await hydrateAppVersionLabel(el, {
    appVersionFallback: "0.1.0",
    tauriIsTauri: () => false,
    tauriGetVersion: async () => "9.9.9"
  });
  assert.equal(el.settingsVersionText.textContent, "Version 0.1.0");

  await hydrateAppVersionLabel(el, {
    appVersionFallback: "0.1.0",
    tauriIsTauri: () => true,
    tauriGetVersion: async () => "9.9.9"
  });
  assert.equal(el.settingsVersionText.textContent, "Version 9.9.9");
});

test("restoreStoredUiPrefs reads storage into state and controls", () => {
  const state = { exportPruneStale: true, exportBackup: true, analysisBpmRange: "", sidebarCollapsed: false };
  const el = {
    exportSyncModeMirror: { checked: false },
    exportSyncModeAdditive: { checked: false },
    exportBackupCheckbox: { checked: true },
    analysisBpmRangeSelect: { value: "" }
  };
  restoreStoredUiPrefs(state, el, {
    localStorageObj: {
      getItem: (k) => ({
        prune: "0",
        backup: "0",
        bpm: "club",
        sidebar: "1"
      }[k] ?? null)
    },
    constants: {
      STORAGE_KEY_EXPORT_PRUNE_STALE: "prune",
      STORAGE_KEY_EXPORT_BACKUP: "backup",
      STORAGE_KEY_ANALYSIS_BPM_RANGE: "bpm",
      STORAGE_KEY_SIDEBAR_COLLAPSED: "sidebar"
    },
    normalizeAnalysisBpmRange: (v) => v,
    defaultAnalysisBpmRange: "all"
  });
  assert.equal(state.exportPruneStale, false);
  assert.equal(state.exportBackup, false);
  assert.equal(state.analysisBpmRange, "club");
  assert.equal(state.sidebarCollapsed, true);
  assert.equal(el.exportSyncModeMirror.checked, false);
  assert.equal(el.exportSyncModeAdditive.checked, true);
  assert.equal(el.exportBackupCheckbox.checked, false);
});

test("restoreStoredUiPrefs defaults exportBackup to true when not in storage", () => {
  const state = { exportPruneStale: true, exportBackup: false, analysisBpmRange: "", sidebarCollapsed: false };
  const el = {
    exportSyncModeMirror: { checked: true },
    exportSyncModeAdditive: { checked: false },
    exportBackupCheckbox: { checked: false },
    analysisBpmRangeSelect: { value: "" }
  };
  restoreStoredUiPrefs(state, el, {
    localStorageObj: { getItem: () => null },
    constants: {
      STORAGE_KEY_EXPORT_PRUNE_STALE: "prune",
      STORAGE_KEY_EXPORT_BACKUP: "backup",
      STORAGE_KEY_ANALYSIS_BPM_RANGE: "bpm",
      STORAGE_KEY_SIDEBAR_COLLAPSED: "sidebar"
    },
    normalizeAnalysisBpmRange: (v) => v,
    defaultAnalysisBpmRange: "all"
  });
  assert.equal(state.exportBackup, true);
  assert.equal(el.exportBackupCheckbox.checked, true);
});

test("applySidebarCollapsedUi and showHelpOnFirstVisit update DOM", () => {
  const dom = new JSDOM(`<!doctype html><body><div id="nav"></div><div id="help" class="hidden"></div></body>`);
  const state = { sidebarCollapsed: true };
  const el = {
    navSidebar: dom.window.document.querySelector("#nav"),
    helpOverlay: dom.window.document.querySelector("#help")
  };
  const btn = dom.window.document.createElement("button");
  applySidebarCollapsedUi(state, el, { sidebarExpandBtn: btn });
  assert.equal(el.navSidebar.classList.contains("collapsed"), true);
  assert.equal(btn.classList.contains("visible"), true);

  showHelpOnFirstVisit(el, {
    localStorageObj: { getItem: () => null },
    storageKeyHelpSeen: "help"
  });
  assert.equal(el.helpOverlay.classList.contains("hidden"), false);
});

test("runDeferredInitialLoad executes deferred init flow", async () => {
  const state = { playlists: [{ id: "p1" }], currentPlaylistId: null, startupPhase: true };
  const calls = [];
  runDeferredInitialLoad(state, {
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
    logError: () => {}
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(state.currentPlaylistId, "p1");
  assert.equal(state.startupPhase, false);
  assert.equal(calls.includes("playlists"), true);
  assert.equal(calls.includes("tracks"), true);
});

test("runDeferredInitialLoad keeps existing current playlist when still present", async () => {
  const state = {
    playlists: [{ id: "p1" }, { id: "p2" }],
    currentPlaylistId: "p2",
    startupPhase: true
  };

  runDeferredInitialLoad(state, {
    setTimeoutFn: (cb) => cb(),
    withProgress: async (_label, fn) => {
      await fn(() => {});
    },
    loadPlaylists: async () => {},
    resetAndLoadLibraryTracks: async () => {},
    updateModeText: () => {},
    updateSelectionCount: () => {},
    renderUsbPlaylistTracks: () => {},
    renderWaveformsIn: () => {},
    documentObj: {},
    setStatus: () => {},
    logError: () => {}
  });
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(state.currentPlaylistId, "p2");
  assert.equal(state.startupPhase, false);
});
