import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import {
  setStatusText,
  updateModeText,
  updateActivePlaylistIndicators,
  updateAddToPlaylistButtons,
  updateSelectionCount,
  updateUsbSubNavDisabledState,
  updateUsbEmptyState,
  updateSourceFilterIndicator,
  updateScanLibraryButtonLabel,
  closeSettingsDrawer,
  updateUsbHealthDot,
  syncLibraryOnboardingMode,
  createConfirmDialogController,
  bindEvents
} from "../ui_controller.mjs";

function makeDom() {
  return new JSDOM(`
    <!doctype html>
    <body>
      <div id="statusText"></div>
      <div id="playlistBadge" class="playlist-badge inactive"></div>
      <div id="badgeLabel"></div>
      <ul id="navPlaylistList">
        <li><button class="nav-playlist-item" data-playlist-id="p1"></button></li>
        <li><button class="nav-playlist-item" data-playlist-id="p2"></button></li>
      </ul>
      <button data-action="add-library"></button>
      <button data-action="add-usb"></button>
      <button id="addSelectedBtn"></button>
      <div id="selectionCount"></div>
      <div id="selectionActions" class="hidden"></div>
      <nav id="navSidebar">
        <button class="nav-sub-item" data-view="usb-playlists"></button>
        <button class="nav-sub-item" data-view="usb-history"></button>
        <button class="nav-sub-item" data-view="usb-player-menu"></button>
      </nav>
      <button id="refreshUsbBtn"></button>
      <button id="refreshHistoryBtn"></button>
      <button id="selectUsbFolderBtn"></button>
      <div id="usbEmptyState"></div>
      <div id="sourceFilterIndicator"></div>
      <button id="scanLibraryBtn"></button>
      <div id="settingsDrawer"></div>
      <div id="settingsBackdrop"></div>
      <div id="usbHealthDot"></div>
    </body>
  `);
}

test("setStatusText updates status text", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = { statusText: document.getElementById("statusText") };
  setStatusText(el, "Loading");

  assert.equal(el.statusText.textContent, "Loading");
});

test("updateModeText reflects current playlist and delegates indicator updates", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = {
    playlistBadge: document.getElementById("playlistBadge"),
    badgeLabel: document.getElementById("badgeLabel")
  };
  let addCalls = 0;
  let indicatorCalls = 0;

  updateModeText(
    { currentPlaylistId: "p1" },
    el,
    {
      getCurrentPlaylist: () => ({ id: "p1", name: "House" }),
      updateAddToPlaylistButtons: () => { addCalls += 1; },
      updateActivePlaylistIndicators: () => { indicatorCalls += 1; }
    }
  );

  assert.equal(el.playlistBadge.className, "playlist-badge active");
  assert.equal(el.badgeLabel.textContent, "House");
  assert.equal(addCalls, 1);
  assert.equal(indicatorCalls, 1);
});

test("updateActivePlaylistIndicators marks the active playlist button", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = { navPlaylistList: document.getElementById("navPlaylistList") };

  updateActivePlaylistIndicators({ currentPlaylistId: "p2" }, el);

  assert.equal(document.querySelector('[data-playlist-id="p1"]').classList.contains("playlist-active-mode"), false);
  assert.equal(document.querySelector('[data-playlist-id="p2"]').classList.contains("playlist-active-mode"), true);
});

test("updateAddToPlaylistButtons and updateSelectionCount keep controls in sync", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = {
    selectionCount: document.getElementById("selectionCount"),
    selectionActions: document.getElementById("selectionActions"),
    addSelectedBtn: document.getElementById("addSelectedBtn")
  };
  const state = {
    currentPlaylistId: "p1",
    selectedTrackIds: new Set(["a", "b"])
  };

  updateAddToPlaylistButtons(state, document);
  updateSelectionCount(state, el);

  assert.equal(document.querySelector('[data-action="add-library"]').disabled, false);
  assert.equal(el.selectionCount.textContent, "2 selected");
  assert.equal(el.selectionActions.classList.contains("hidden"), false);
  assert.equal(el.addSelectedBtn.disabled, false);
});

test("updateUsbSubNavDisabledState reveals USB subnav and falls back when disconnected", async () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = {
    navSidebar: document.getElementById("navSidebar"),
    refreshUsbBtn: document.getElementById("refreshUsbBtn"),
    refreshHistoryBtn: document.getElementById("refreshHistoryBtn")
  };
  const switched = [];

  updateUsbSubNavDisabledState(
    { usbRoot: null, usbRootValid: false, activeTab: "usb-playlists" },
    el,
    { switchView: async (view) => { switched.push(view); } }
  );

  assert.equal(el.refreshUsbBtn.disabled, true);
  assert.deepEqual(switched, ["usb"]);
});

test("updateUsbSubNavDisabledState also falls back from usb-player-menu when disconnected", async () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = {
    navSidebar: document.getElementById("navSidebar"),
    refreshUsbBtn: document.getElementById("refreshUsbBtn"),
    refreshHistoryBtn: document.getElementById("refreshHistoryBtn")
  };
  const switched = [];

  updateUsbSubNavDisabledState(
    { usbRoot: null, usbRootValid: false, activeTab: "usb-player-menu" },
    el,
    { switchView: async (view) => { switched.push(view); } }
  );

  assert.equal(el.refreshUsbBtn.disabled, true);
  assert.deepEqual(switched, ["usb"]);
});

test("updateUsbEmptyState renders empty state only without root or recents", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const payloads = [];

  updateUsbEmptyState(
    { usbRoot: null, usbRootValid: false, usbRecentRoots: [] },
    document,
    { renderEmptyState: (container, payload) => payloads.push({ container, payload }) }
  );

  assert.equal(payloads.length, 1);
  assert.equal(payloads[0].payload.heading, "Connect a USB drive to browse and export");
});

test("updateSourceFilterIndicator, updateScanLibraryButtonLabel, closeSettingsDrawer, updateUsbHealthDot, and syncLibraryOnboardingMode work", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = {
    sourceFilterIndicator: document.getElementById("sourceFilterIndicator"),
    scanLibraryBtn: document.getElementById("scanLibraryBtn"),
    settingsDrawer: document.getElementById("settingsDrawer"),
    settingsBackdrop: document.getElementById("settingsBackdrop"),
    usbHealthDot: document.getElementById("usbHealthDot")
  };

  updateSourceFilterIndicator({ sourceRoots: ["/a"], sourceRootEnabled: { "/a": false } }, el);
  updateScanLibraryButtonLabel({ sourceRoots: ["/a"] }, el, {
    scanLibraryButtonLabel: (roots) => `Scan ${roots.length}`
  });
  closeSettingsDrawer(el);
  updateUsbHealthDot(el, "WARN");
  syncLibraryOnboardingMode({ activeTab: "library", sourceRoots: [] }, document);

  assert.equal(el.sourceFilterIndicator.classList.contains("active"), true);
  assert.equal(el.scanLibraryBtn.textContent, "Scan 1");
  assert.equal(el.settingsDrawer.classList.contains("hidden"), true);
  assert.equal(el.settingsBackdrop.classList.contains("hidden"), true);
  assert.equal(el.usbHealthDot.classList.contains("health-warn"), true);
  assert.equal(document.body.classList.contains("library-onboarding"), true);
});

test("updateSourceFilterIndicator is active when masterDb is filtered out", () => {
  const dom = makeDom();
  const el = { sourceFilterIndicator: dom.window.document.getElementById("sourceFilterIndicator") };
  // all filesystem roots enabled but master.db disabled
  updateSourceFilterIndicator({
    sourceRoots: ["/a"],
    sourceRootEnabled: { "/a": true },
    externalMasterDbPath: "/path/to/master.db",
    masterDbEnabled: false
  }, el);
  assert.equal(el.sourceFilterIndicator.classList.contains("active"), true);
});

test("updateSourceFilterIndicator is not active when all sources including masterDb are enabled", () => {
  const dom = makeDom();
  const el = { sourceFilterIndicator: dom.window.document.getElementById("sourceFilterIndicator") };
  updateSourceFilterIndicator({
    sourceRoots: ["/a"],
    sourceRootEnabled: { "/a": true },
    externalMasterDbPath: "/path/to/master.db",
    masterDbEnabled: true
  }, el);
  assert.equal(el.sourceFilterIndicator.classList.contains("active"), false);
});

test("createConfirmDialogController opens, closes, and resolves the dialog promise", async () => {
  const dom = new JSDOM(`
    <!doctype html>
    <body>
      <div id="confirmOverlay" hidden></div>
      <div id="confirmTitle"></div>
      <div id="confirmMessage"></div>
      <button id="confirmOkBtn" type="button"></button>
    </body>
  `, { pretendToBeVisual: true });
  const { document } = dom.window;
  const controller = createConfirmDialogController({
    confirmOverlay: document.getElementById("confirmOverlay"),
    confirmTitle: document.getElementById("confirmTitle"),
    confirmMessage: document.getElementById("confirmMessage"),
    confirmOkBtn: document.getElementById("confirmOkBtn")
  });

  const promise = controller.open({ title: "Delete", message: "Confirm?", confirmLabel: "Remove" });
  assert.equal(controller.isOpen(), true);
  assert.equal(document.getElementById("confirmOverlay").hidden, false);
  assert.equal(document.getElementById("confirmTitle").textContent, "Delete");
  assert.equal(document.getElementById("confirmMessage").textContent, "Confirm?");
  assert.equal(document.getElementById("confirmOkBtn").textContent, "Remove");

  controller.close(true);
  assert.equal(await promise, true);
  assert.equal(controller.isOpen(), false);
  assert.equal(document.getElementById("confirmOverlay").hidden, true);
});

test("bindEvents wires confirm buttons and sidebar collapse/expand", () => {
  const listeners = new Map();
  const makeEmitter = () => ({
    hidden: false,
    disabled: false,
    checked: false,
    value: "",
    textContent: "",
    dataset: {},
    focus() {},
    classList: {
      add() {},
      remove() {},
      toggle() {},
      contains() { return false; }
    },
    addEventListener(type, handler) {
      listeners.set(this, { ...(listeners.get(this) || {}), [type]: handler });
    },
    querySelectorAll() { return []; },
    querySelector() { return null; }
  });
  const confirmOkBtn = makeEmitter();
  const confirmCancelBtn = makeEmitter();
  const confirmOverlay = makeEmitter();
  const navSidebar = Object.assign(makeEmitter(), { classList: { add() {}, remove() {}, toggle() {}, contains() { return false; } } });
  const sidebarExpandBtn = Object.assign(makeEmitter(), { classList: { add() {}, remove() {}, toggle() {}, contains() { return false; } } });
  const documentListeners = {};
  const document = {
    activeElement: confirmOkBtn,
    body: { classList: { add() {}, remove() {}, toggle() {} } },
    addEventListener(type, handler) { documentListeners[type] = handler; },
    getElementById() { return null; },
    querySelector() { return null; },
    querySelectorAll() { return []; }
  };
  const windowObj = { addEventListener() {}, open() {}, __TAURI__: null, __TAURI_INTERNALS__: null };
  const navigatorObj = { clipboard: { writeText: async () => {} } };
  const state = {
    sourceRoots: [],
    sourceRootEnabled: {},
    usbRoot: null,
    usbRootValid: false,
    usbRecentRoots: [],
    activeTab: "library",
    currentPlaylistId: null,
    selectedTrackIds: new Set(),
    tracks: [],
    filteredTracks: [],
    usbPlaylists: [],
    usbPlaylistTracks: [],
    usbPlaylistTracksView: [],
    histories: [],
    historyTracks: [],
    historyTracksView: [],
    currentPlaylistTracksView: [],
    playlists: [],
    playbackActive: false,
    playbackRowKey: null,
    eventLogEntries: [],
    libraryQuery: "",
    exportPruneStale: true,
    analysisBpmRange: "wide"
  };
  const el = {
    confirmOkBtn,
    confirmCancelBtn,
    confirmOverlay,
    navSidebar,
    navPlaylistList: makeEmitter(),
    addPlaylistBtn: makeEmitter(),
    sidebarCollapseBtn: makeEmitter(),
    settingsBtn: makeEmitter(),
    settingsDrawer: makeEmitter(),
    settingsBackdrop: makeEmitter(),
    settingsCloseBtn: makeEmitter(),
    openEventLogBtn: makeEmitter(),
    eventLogLevelFilter: makeEmitter(),
    eventLogSourceFilter: makeEmitter(),
    eventLogClearBtn: makeEmitter(),
    helpBtn: makeEmitter(),
    helpCloseBtn: makeEmitter(),
    helpOverlay: makeEmitter(),
    sourceChipsContainer: makeEmitter(),
    externalMasterDbCheckbox: makeEmitter(),
    addSourceBtn: makeEmitter(),
    scanLibraryBtn: makeEmitter(),
    progressDismiss: makeEmitter(),
    refreshUsbBtn: makeEmitter(),
    selectUsbFolderBtn: makeEmitter(),
    usbRecentList: makeEmitter(),
    initializeUsbBtn: makeEmitter(),
    exportSyncModeGroup: makeEmitter(),
    analysisBpmRangeSelect: makeEmitter(),
    runUsbParityBtn: makeEmitter(),
    reDiagnoseBtn: makeEmitter(),
    previewRepairsBtn: makeEmitter(),
    applyRepairsBtn: makeEmitter(),
    diagBackToReportBtn: makeEmitter(),
    refreshHistoryBtn: makeEmitter(),
    libraryTableWrap: makeEmitter(),
    librarySearch: makeEmitter(),
    usbTrackSearch: makeEmitter(),
    historyTrackSearch: makeEmitter(),
    playlistSearchInput: makeEmitter(),
    addSelectedBtn: makeEmitter(),
    selectAllTracks: makeEmitter(),
    libraryTableBody: makeEmitter(),
    usbPlaylists: makeEmitter(),
    usbPlaylistTracks: makeEmitter(),
    historyList: makeEmitter(),
    historyTracks: makeEmitter(),
    panels: { playlist: makeEmitter() },
    exportPlaylistBtn: makeEmitter(),
    analyzePlaylistMissingBtn: makeEmitter(),
    usbSelectedPlaylistText: makeEmitter(),
    selectedHistoryText: makeEmitter()
  };
  const confirmDialog = createConfirmDialogController({
    confirmOverlay,
    confirmTitle: { textContent: "" },
    confirmMessage: { textContent: "" },
    confirmOkBtn
  });
  let persisted = [];

  bindEvents({
    state,
    el,
    document,
    window: windowObj,
    navigator: navigatorObj,
    eventLogStore: { clear() {} },
    sidebarExpandBtn,
    confirmDialog,
    constants: {
      STORAGE_KEY_SIDEBAR_COLLAPSED: "sidebar",
      FRONTEND_DB_KEY_SIDEBAR_COLLAPSED: "sidebar_db",
      STORAGE_KEY_HELP_SEEN: "help",
      FRONTEND_DB_KEY_HELP_SEEN: "help_db",
      STORAGE_KEY_EXPORT_PRUNE_STALE: "prune",
      FRONTEND_DB_KEY_EXPORT_PRUNE_STALE: "prune_db",
      STORAGE_KEY_ANALYSIS_BPM_RANGE: "bpm",
      FRONTEND_DB_KEY_ANALYSIS_BPM_RANGE: "bpm_db",
      LIBRARY_LOAD_LIMIT_DEFAULT: 200
    },
    setStatus() {},
    closeSettingsDrawer() {},
    renderEventLog() {},
    switchView: async () => {},
    deletePlaylist: async () => {},
    startPlaylistRename() {},
    promptNewPlaylist() {},
    persistSetting: (...args) => { persisted.push(args); },
    renderSourceChips() {},
    syncAssetScopePaths: async () => {},
    applySearchLocalFilter() {},
    updateSelectionCount() {},
    command: async () => ({ removed: 0 }),
    resetAndLoadLibraryTracks: async () => {},
    refreshCurrentPlaylistTracks: async () => {},
    withProgress: async (_, fn) => fn(() => {}),
    persistSourceRoots() {},
    persistSourceRootEnabled() {},
    enabledSourceRoots: () => [],
    pickSourceFolders: async () => [],
    scanLibrary: async () => {},
    dismissProgress() {},
    refreshUsb: async () => {},
    pickUsbFolder: async () => {},
    validateAndSetUsbRoot: async () => {},
    initializeUsb: async () => {},
    normalizeAnalysisBpmRange: (v) => v || "wide",
    updatePlaylistExportButtons() {},
    runUsbParityReport: async () => {},
    runUsbDiagnostics: async () => {},
    previewUsbRepairs: async () => {},
    applyUsbRepairs: async () => {},
    showDiagReportView() {},
    refreshHistory: async () => {},
    scheduleApplySearchLocalFilter() {},
    renderUsbPlaylistTracks() {},
    renderHistoryTracks() {},
    addTracksToCurrentPlaylist: async () => {},
    getLibraryVisibleTracks: () => [],
    analyzeSingleTrack: async () => {},
    getPlaybackUiStateHelpers: () => null,
    isTrackCurrentlyPlaying: () => false,
    stopPlaybackFromUi: async () => {},
    playTrackFromOrigin: async () => {},
    scrubRatioFromPointer: () => 0,
    removeUsbPlaylist: async () => {},
    stopPlaybackIfActive: async () => {},
    hydrateUsbTrackMetadata: async () => {},
    setActiveListItem() {},
    getHistoryDateDisplay: () => "",
    getCurrentPlaylist: () => null,
    loadPlaylists: async () => {},
    updateModeText() {},
    exportPlaylistToUsb: async () => {},
    isUsbOriginTrack: () => false,
    trackHasCoreAnalysis: () => false,
    analyzeTrackIds: async () => {},
    resolveLocalTrackId: () => null,
    handleSortHeaderClick() {},
    handleLibraryTableWrapScroll() {},
    handleWindowLibraryScroll() {},
    renderLibraryRows() {}
  });

  confirmDialog.open({ title: "T", message: "M" });
  listeners.get(confirmOkBtn).click();
  listeners.get(sidebarExpandBtn).click();
  listeners.get(el.sidebarCollapseBtn).click();
  documentListeners.keydown({ key: "Escape", preventDefault() {} });

  assert.equal(persisted.length, 2);
  assert.deepEqual(persisted[0], ["sidebar", "sidebar_db", "0"]);
  assert.deepEqual(persisted[1], ["sidebar", "sidebar_db", "1"]);
});
