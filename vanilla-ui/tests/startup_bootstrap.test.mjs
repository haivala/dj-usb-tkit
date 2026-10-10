import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  applySidebarCollapsedUi,
  hydrateAppVersionLabel,
  restoreStoredUiPrefs,
  runDataMigrations,
  runDeferredInitialLoad,
  showHelpOnFirstVisit,
  switchView
} from "../startup_bootstrap.mjs";
import { withSilencedConsole } from "./test_helpers.mjs";
import {
  STORAGE_KEY_EXPORT_PRUNE_STALE,
  STORAGE_KEY_EXPORT_BACKUP,
  STORAGE_KEY_EXPORT_ENGINE_LIBRARY,
  STORAGE_KEY_ANALYSIS_BPM_RANGE,
  STORAGE_KEY_SIDEBAR_COLLAPSED
} from "../settings_keys.mjs";

function prefEls() {
  return {
    exportSyncModeMirror: { checked: false },
    exportSyncModeAdditive: { checked: false },
    exportBackupCheckbox: { checked: false },
    exportEngineLibraryCheckbox: { checked: false },
    analysisBpmRangeSelect: { value: "" }
  };
}

function restorePrefs(state, el, values) {
  restoreStoredUiPrefs({ state, el, localStorage: { getItem: (key) => values[key] ?? null } });
}

function deferredCtx(state, calls = [], overrides = {}) {
  return {
    state,
    setTimeoutFn: (cb) => cb(),
    withProgress: async (_label, fn) => {
      await fn((pct, text) => calls.push(`progress:${pct}:${text}`));
    },
    loadPlaylists: async () => { calls.push("playlists"); },
    resetAndLoadLibraryTracks: async () => { calls.push("tracks"); },
    updateModeText: () => { calls.push("mode"); },
    updateSelectionCount: () => { calls.push("selection"); },
    usbPlaylistTracksCtl: { clear: () => { calls.push("usb"); } },
    renderWaveformsIn: () => { calls.push("wave"); },
    document: {},
    emitStatus: () => {},
    logError: () => {},
    ...overrides
  };
}

test("hydrateAppVersionLabel uses fallback and tauri override", async () => {
  const dom = new JSDOM(`<!doctype html><body><span id="v"></span></body>`);
  const el = { settingsVersionText: dom.window.document.querySelector("#v") };

  for (const [tauriIsTauri, expected] of [[() => false, "Not set"], [() => true, "9.9.9"]]) {
    await hydrateAppVersionLabel({
      el,
      tauriIsTauri,
      tauriGetVersion: async () => "9.9.9"
    });
    assert.equal(el.settingsVersionText.textContent, `Version ${expected}`);
  }
});

test("restoreStoredUiPrefs reads stored controls and defaults backup on and Engine library off", () => {
  const storedState = { exportPruneStale: true, exportBackup: true, analysisBpmRange: "", sidebarCollapsed: false };
  const storedEl = prefEls();
  restorePrefs(storedState, storedEl, {
    [STORAGE_KEY_EXPORT_PRUNE_STALE]: "0",
    [STORAGE_KEY_EXPORT_BACKUP]: "0",
    [STORAGE_KEY_EXPORT_ENGINE_LIBRARY]: "1",
    [STORAGE_KEY_ANALYSIS_BPM_RANGE]: "90-160",
    [STORAGE_KEY_SIDEBAR_COLLAPSED]: "1"
  });
  assert.equal(storedState.exportPruneStale, false);
  assert.equal(storedState.exportBackup, false);
  assert.equal(storedState.analysisBpmRange, "90-160");
  assert.equal(storedState.sidebarCollapsed, true);
  assert.equal(storedEl.exportSyncModeMirror.checked, false);
  assert.equal(storedEl.exportSyncModeAdditive.checked, true);
  assert.equal(storedEl.exportBackupCheckbox.checked, false);
  assert.equal(storedState.exportEngineLibrary, true);
  assert.equal(storedEl.exportEngineLibraryCheckbox.checked, true);

  const defaultState = { exportPruneStale: true, exportBackup: false, analysisBpmRange: "", sidebarCollapsed: false };
  const defaultEl = prefEls();
  restorePrefs(defaultState, defaultEl, {});
  assert.equal(defaultState.exportBackup, true);
  assert.equal(defaultEl.exportBackupCheckbox.checked, true);
  assert.equal(defaultState.exportEngineLibrary, false);
  assert.equal(defaultEl.exportEngineLibraryCheckbox.checked, false);
});

test("applySidebarCollapsedUi and showHelpOnFirstVisit update DOM", () => {
  const dom = new JSDOM(`<!doctype html><body><div id="nav"></div><div id="help" class="hidden"></div></body>`);
  const el = {
    navSidebar: dom.window.document.querySelector("#nav"),
    helpOverlay: dom.window.document.querySelector("#help")
  };
  const btn = dom.window.document.createElement("button");
  applySidebarCollapsedUi({
    state: { sidebarCollapsed: true },
    el: { ...el, sidebarExpandBtn: btn },
    document: dom.window.document
  });
  showHelpOnFirstVisit({ el, localStorage: { getItem: () => null } });

  assert.equal(el.navSidebar.classList.contains("collapsed"), true);
  assert.equal(btn.classList.contains("visible"), true);
  assert.equal(el.helpOverlay.classList.contains("hidden"), false);
});

test("runDeferredInitialLoad loads initial data, selects fallback playlists, and preserves valid current playlists", async () => {
  const calls = [];
  const first = {
    playlists: [{ id: "p1" }, { id: "p2" }, { id: "p3" }],
    currentPlaylistId: null,
    startupPhase: true
  };
  runDeferredInitialLoad(deferredCtx(first, calls));
  await new Promise((resolve) => setTimeout(resolve, 0));
  // Newest playlist (last in created_at ASC order) is the default selection.
  assert.equal(first.currentPlaylistId, "p3");
  assert.equal(first.startupPhase, false);
  assert.equal(calls.includes("playlists"), true);
  assert.equal(calls.includes("tracks"), true);

  const existing = {
    playlists: [{ id: "p1" }, { id: "p2" }],
    currentPlaylistId: "p2",
    startupPhase: true
  };
  runDeferredInitialLoad(deferredCtx(existing, [], {
    loadPlaylists: async () => {},
    resetAndLoadLibraryTracks: async () => {}
  }));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(existing.currentPlaylistId, "p2");
  assert.equal(existing.startupPhase, false);
});

test("runDeferredInitialLoad runs the data migrations after the initial load", async () => {
  const calls = [];
  const state = { playlists: [], currentPlaylistId: null, startupPhase: true };
  runDeferredInitialLoad(deferredCtx(state, calls, {
    command: async (name) => { calls.push(`command:${name}`); return { ran: [], retryLater: [] }; }
  }));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(calls.at(-1), "command:run_data_migrations");
  assert.ok(calls.indexOf("tracks") < calls.indexOf("command:run_data_migrations"));
});

test("runDataMigrations logs a failure instead of throwing", async () => {
  const logs = [];
  const result = await runDataMigrations({
    command: async () => { throw new Error("disk full"); },
    pushEventLog: (entry) => logs.push(entry)
  });
  assert.equal(result, null);
  assert.equal(logs.length, 1);
  assert.equal(logs[0].level, "warn");
  assert.match(logs[0].message, /Data upgrade failed: disk full/);
  assert.equal(await runDataMigrations({}), null, "no backend: nothing to do");
});

// Every collaborator switchView touches, as no-ops; tests override the ones
// they observe.
function switchViewCtx(state, el, overrides = {}) {
  return {
    state,
    el,
    document: el.navSidebar.ownerDocument,
    stopPlaybackIfActive: async () => {},
    commitActivePlaylistSort: async () => {},
    syncLibraryOnboardingMode: () => {},
    updateModeText: () => {},
    populatePlaylistPanel: () => {},
    refreshCurrentPlaylistTracks: async () => {},
    renderEventLog: () => {},
    renderBackups: async () => {},
    requestAnimationFrameFn: () => {},
    renderWaveformsIn: () => {},
    emitStatus: () => {},
    ...overrides
  };
}

function switchViewDom() {
  const dom = new JSDOM(`<!doctype html><body>
    <nav id="navSidebar">
      <button class="nav-item" data-view="library"></button>
      <button class="nav-item" data-view="usb"></button>
    </nav>
    <ul id="navPlaylistList"></ul>
  </body>`);
  const document = dom.window.document;
  return {
    document,
    el: {
      navSidebar: document.getElementById("navSidebar"),
      navPlaylistList: document.getElementById("navPlaylistList"),
      panels: {
        library: document.createElement("div"),
        usb: document.createElement("div"),
        playlist: document.createElement("div")
      }
    }
  };
}

test("switchView commits the outgoing playlist's sort only when the view actually changes", async () => {
  const { el } = switchViewDom();
  const state = { activeTab: "p1", playlists: [{ id: "p1" }] };
  const commitCalls = [];

  const ctx = switchViewCtx(state, el, {
    commitActivePlaylistSort: async (playlistId) => { commitCalls.push(playlistId); }
  });
  await switchView(ctx, "usb");

  assert.deepEqual(commitCalls, ["p1"]);
  assert.equal(state.activeTab, "usb");

  // Re-selecting the same view should not fire another commit.
  await switchView(ctx, "usb");
  assert.deepEqual(commitCalls, ["p1"]);
});

test("switchView still completes the switch when the sort commit fails", async () => {
  const { el } = switchViewDom();
  const state = { activeTab: "p1", playlists: [{ id: "p1" }] };
  const statusMessages = [];

  // switchView logs the caught commit error via console.error -- expected
  // here since we're deliberately exercising that path, so silence it to
  // keep the test run's terminal output clean.
  await withSilencedConsole(() => switchView(switchViewCtx(state, el, {
    commitActivePlaylistSort: async () => { throw new Error("boom"); },
    emitStatus: (text) => statusMessages.push(text)
  }), "library"));

  assert.equal(state.activeTab, "library");
  assert.equal(statusMessages.length, 1);
  assert.match(statusMessages[0], /Save track order failed: boom/);
});
