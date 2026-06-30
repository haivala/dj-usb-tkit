import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { bindLibraryEvents } from "../components/library/events.mjs";

function makeCtx() {
  const dom = new JSDOM(`<!doctype html><body>
    <div id="chips"></div>
    <button id="addSource"></button>
    <button id="importMasterDb"></button>
    <button id="scan"></button>
    <div id="libraryWrap"></div>
    <input id="librarySearch" />
    <button id="addSelected"></button>
    <input id="selectAll" type="checkbox" />
    <table><tbody id="libraryBody"></tbody></table>
  </body>`);
  const document = dom.window.document;
  const state = {
    sourceRoots: [],
    sourceRootEnabled: {},
    tracks: [],
    filteredTracks: [],
    selectedTrackIds: new Set(),
    libraryQuery: "",
    playbackRowKey: null
  };
  const el = {
    sourceChipsContainer: document.querySelector("#chips"),
    addSourceBtn: document.querySelector("#addSource"),
    importMasterDbBtn: document.querySelector("#importMasterDb"),
    scanLibraryBtn: document.querySelector("#scan"),
    libraryTableWrap: document.querySelector("#libraryWrap"),
    librarySearch: document.querySelector("#librarySearch"),
    addSelectedBtn: document.querySelector("#addSelected"),
    selectAllTracks: document.querySelector("#selectAll"),
    libraryTableBody: document.querySelector("#libraryBody")
  };
  return { dom, document, state, el };
}

test("bindLibraryEvents wires scan button to scanLibrary", async () => {
  const { state, el, dom } = makeCtx();
  let scanned = 0;
  bindLibraryEvents({
    state,
    el,
    window: dom.window,
    constants: { LIBRARY_LOAD_LIMIT_DEFAULT: 100 },
    setStatus: () => {},
    renderSourceChips: () => {},
    syncAssetScopePaths: async () => {},
    applySearchLocalFilter: () => {},
    updateSelectionCount: () => {},
    command: async () => ({}),
    resetAndLoadLibraryTracks: async () => {},
    refreshCurrentPlaylistTracks: async () => {},
    withProgress: async (_label, fn) => fn(() => {}),
    persistSourceRoots: () => {},
    persistSourceRootEnabled: () => {},
    enabledSourceRoots: () => [],
    pickSourceFolders: async () => [],
    scanLibrary: async () => { scanned += 1; },
    scheduleApplySearchLocalFilter: () => {},
    addTracksToCurrentPlaylist: async () => {},
    getLibraryVisibleTracks: () => [],
    analyzeSingleTrack: async () => {},
    getPlaybackUiStateHelpers: () => null,
    isTrackCurrentlyPlaying: () => false,
    stopPlaybackFromUi: async () => {},
    playTrackFromOrigin: async () => {},
    scrubRatioFromPointer: () => 0,
    handleLibraryTableWrapScroll: () => {},
    handleWindowLibraryScroll: () => {},
    renderLibraryRows: () => {}
  });

  el.scanLibraryBtn.click();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(scanned, 1);
});

test("bindLibraryEvents wires importMasterDbBtn to scanMasterDb", async () => {
  const { state, el, dom } = makeCtx();
  let imported = 0;
  bindLibraryEvents({
    state,
    el,
    window: dom.window,
    constants: { LIBRARY_LOAD_LIMIT_DEFAULT: 100 },
    setStatus: () => {},
    renderSourceChips: () => {},
    syncAssetScopePaths: async () => {},
    applySearchLocalFilter: () => {},
    updateSelectionCount: () => {},
    command: async () => ({}),
    resetAndLoadLibraryTracks: async () => {},
    refreshCurrentPlaylistTracks: async () => {},
    withProgress: async (_label, fn) => fn(() => {}),
    persistSourceRoots: () => {},
    persistSourceRootEnabled: () => {},
    enabledSourceRoots: () => [],
    pickSourceFolders: async () => [],
    scanLibrary: async () => {},
    scanMasterDb: async () => { imported += 1; },
    scheduleApplySearchLocalFilter: () => {},
    addTracksToCurrentPlaylist: async () => {},
    getLibraryVisibleTracks: () => [],
    handleLibraryTableWrapScroll: () => {},
    renderLibraryRows: () => {}
  });

  el.importMasterDbBtn.click();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(imported, 1);
});

test("master.db chip checkbox is a pure filter - does not call scanMasterDb", async () => {
  const { state, el, dom } = makeCtx();
  state.sourceRoots = [];
  state.masterDbEnabled = false;
  state.libraryQuery = "";
  let imported = 0;
  let reloaded = 0;
  bindLibraryEvents({
    state,
    el,
    window: dom.window,
    constants: { LIBRARY_LOAD_LIMIT_DEFAULT: 100 },
    setStatus: () => {},
    renderSourceChips: () => {},
    syncAssetScopePaths: async () => {},
    applySearchLocalFilter: () => {},
    updateSelectionCount: () => {},
    updateSourceFilterIndicator: () => {},
    persistMasterDbEnabled: () => {},
    command: async () => ({}),
    resetAndLoadLibraryTracks: async () => { reloaded += 1; },
    refreshCurrentPlaylistTracks: async () => {},
    withProgress: async (_label, fn) => fn(() => {}),
    persistSourceRoots: () => {},
    persistSourceRootEnabled: () => {},
    enabledSourceRoots: () => [],
    pickSourceFolders: async () => [],
    scanLibrary: async () => {},
    scanMasterDb: async () => { imported += 1; },
    scheduleApplySearchLocalFilter: () => {},
    addTracksToCurrentPlaylist: async () => {},
    getLibraryVisibleTracks: () => [],
    handleLibraryTableWrapScroll: () => {},
    renderLibraryRows: () => {}
  });

  // inject a master.db chip checkbox into the chips container
  const chip = dom.window.document.createElement("span");
  chip.innerHTML = `<input class="source-chip-toggle" type="checkbox" data-master-db="true" />`;
  el.sourceChipsContainer.appendChild(chip);
  const checkbox = el.sourceChipsContainer.querySelector(".source-chip-toggle");
  checkbox.checked = true;
  checkbox.dispatchEvent(new dom.window.Event("change", { bubbles: true }));

  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(imported, 0, "scanMasterDb must not be called when chip is toggled");
  assert.equal(reloaded, 1, "resetAndLoadLibraryTracks should be called to apply filter");
  assert.equal(state.masterDbEnabled, true);
});
