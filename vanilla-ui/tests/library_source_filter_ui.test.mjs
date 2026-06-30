import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  getLibraryVisibleTracks,
  applySearchLocalFilter,
  loadTracks,
  normalizeTrack,
  renderSourceChips,
  scanMasterDb
} from "../components/library/actions.mjs";

test("getLibraryVisibleTracks returns filtered list", () => {
  const state = { filteredTracks: [{ id: 1 }] };
  assert.deepEqual(getLibraryVisibleTracks(state), [{ id: 1 }]);
});

test("applySearchLocalFilter clears state when no source roots and masterDb disabled", () => {
  const state = {
    sourceRoots: [],
    masterDbEnabled: false,
    tracks: [{ id: "a" }],
    selectedTrackIds: new Set(["a"]),
    filteredTracks: [{ id: "a" }]
  };
  let renders = 0;
  let selectionUpdates = 0;
  applySearchLocalFilter(state, { librarySearch: { value: "" } }, {
    enabledSourceRoots: (roots) => roots,
    trackPathMatchesAnyRoot: () => false,
    renderLibraryRows: () => { renders += 1; },
    updateSelectionCount: () => { selectionUpdates += 1; }
  });
  assert.equal(state.filteredTracks.length, 0);
  assert.equal(state.selectedTrackIds.size, 0);
  assert.equal(renders, 1);
  assert.equal(selectionUpdates, 1);
});

test("applySearchLocalFilter filters by query and prunes selected ids", () => {
  const tracks = [
    { id: "1", title: "Alpha", artist: "A", album: "One", searchText: "alpha a one", filePath: "/music/alpha.mp3", masterDbSource: false },
    { id: "2", title: "Beta", artist: "B", album: "Two", searchText: "beta b two", filePath: "/music/beta.mp3", masterDbSource: false }
  ];
  const state = {
    sourceRoots: ["/music"],
    sourceRootEnabled: { "/music": true },
    masterDbEnabled: false,
    tracks,
    selectedTrackIds: new Set(["1", "2"]),
    filteredTracks: [],
    libraryQuery: ""
  };
  applySearchLocalFilter(state, { librarySearch: { value: "alpha" } }, {
    enabledSourceRoots: (roots, enabled) => roots.filter((r) => enabled[r] !== false),
    trackPathMatchesAnyRoot: (fp, roots) => roots.some((r) => String(fp).startsWith(r)),
    renderLibraryRows: () => {},
    updateSelectionCount: () => {}
  });
  assert.deepEqual(state.filteredTracks.map((t) => t.id), ["1"]);
  assert.deepEqual(Array.from(state.selectedTrackIds), ["1"]);
});

test("loadTracks uses one browse request for enabled folders and master.db", async () => {
  const state = {
    sourceRoots: ["/music/a", "/music/b"],
    sourceRootEnabled: { "/music/a": true, "/music/b": false },
    masterDbEnabled: true,
    tracks: [],
    filteredTracks: [],
    libraryRequestSeq: 1,
    libraryLoading: false
  };
  const calls = [];

  await loadTracks(state, "alpha", 25, "cursor-1", { requestSeq: 1 }, {
    command: async (name, payload) => {
      calls.push({ name, payload });
      return {
        total: 2,
        items: [
          { id: "folder-1", title: "Alpha", artist: "A", filePath: "/music/a/alpha.mp3" },
          { id: "db-1", title: "Desktop Alpha", artist: "Desktop", filePath: "/library/alpha.mp3", masterDbSource: true }
        ],
        nextCursor: "cursor-2",
        hasMore: true
      };
    },
    normalizeTrack,
    readLibraryPagination: (data) => ({ nextCursor: data.nextCursor, hasMore: data.hasMore }),
    renderSourceChips: () => {},
    applySearchLocalFilter: () => { state.filteredTracks = [...state.tracks]; },
    hydrateLoadedTracksPreviewsInBackground: async () => {}
  });

  assert.equal(calls.length, 1);
  assert.equal(calls[0].name, "browse_source_files");
  assert.deepEqual(calls[0].payload.sourceRoots, ["/music/a"]);
  assert.equal(calls[0].payload.includeMasterDb, true);
  assert.equal(calls[0].payload.query, "alpha");
  assert.equal(calls[0].payload.limit, 25);
  assert.equal(calls[0].payload.cursor, "cursor-1");
  assert.deepEqual(state.tracks.map((track) => track.id), ["folder-1", "db-1"]);
  assert.equal(state.libraryLoadedTotal, 2);
  assert.equal(state.libraryNextCursor, "cursor-2");
  assert.equal(state.libraryHasMore, true);
});

test("renderSourceChips renders chips and toggles analyzed class", () => {
  const dom = new JSDOM(`<!doctype html><body><div id="chips"></div></body>`);
  const state = {
    sourceRoots: ["/music/a", "/music/b"],
    sourceRootEnabled: {},
    tracks: [
      { filePath: "/music/a/1.mp3", durationMs: 120000, analyzed: true },
      { filePath: "/music/b/2.mp3", durationMs: 120000, analyzed: false }
    ]
  };
  const el = { sourceChipsContainer: dom.window.document.querySelector("#chips") };
  let persisted = null;
  let scanLabelUpdates = 0;
  let indicatorUpdates = 0;

  renderSourceChips(state, el, {
    documentObj: dom.window.document,
    escapeHtml: (v) => String(v),
    trackPathMatchesAnyRoot: (filePath, roots) => String(filePath).startsWith(String(roots[0])),
    trackHasCoreAnalysis: (track) => !!track.analyzed,
    persistSourceRootEnabled: (map) => { persisted = { ...map }; },
    updateScanLibraryButtonLabel: () => { scanLabelUpdates += 1; },
    updateSourceFilterIndicator: () => { indicatorUpdates += 1; }
  });

  const chips = el.sourceChipsContainer.querySelectorAll(".source-chip");
  assert.equal(chips.length, 2);
  assert.equal(chips[0].classList.contains("source-chip-analyzed"), true);
  assert.equal(chips[1].classList.contains("source-chip-analyzed"), false);
  assert.deepEqual(Object.keys(persisted).sort(), ["/music/a", "/music/b"]);
  assert.equal(scanLabelUpdates, 1);
  assert.equal(indicatorUpdates, 1);
});

test("renderSourceChips shows importMasterDbBtn when externalMasterDbPath is set", () => {
  const dom = new JSDOM(`<!doctype html><body><div id="chips"></div><button id="importBtn" class="hidden"></button></body>`);
  const document = dom.window.document;
  const importBtn = document.querySelector("#importBtn");
  const state = {
    sourceRoots: [],
    sourceRootEnabled: {},
    tracks: [],
    externalMasterDbPath: "/path/to/master.db",
    masterDbEnabled: false
  };
  const el = {
    sourceChipsContainer: document.querySelector("#chips"),
    importMasterDbBtn: importBtn
  };
  renderSourceChips(state, el, {
    documentObj: document,
    persistSourceRootEnabled: () => {},
    updateScanLibraryButtonLabel: () => {},
    updateSourceFilterIndicator: () => {}
  });
  assert.equal(importBtn.classList.contains("hidden"), false);
});

test("renderSourceChips hides importMasterDbBtn when externalMasterDbPath is not set", () => {
  const dom = new JSDOM(`<!doctype html><body><div id="chips"></div><button id="importBtn"></button></body>`);
  const document = dom.window.document;
  const importBtn = document.querySelector("#importBtn");
  const state = {
    sourceRoots: [],
    sourceRootEnabled: {},
    tracks: [],
    externalMasterDbPath: null,
    masterDbEnabled: false
  };
  const el = {
    sourceChipsContainer: document.querySelector("#chips"),
    importMasterDbBtn: importBtn
  };
  renderSourceChips(state, el, {
    documentObj: document,
    persistSourceRootEnabled: () => {},
    updateScanLibraryButtonLabel: () => {},
    updateSourceFilterIndicator: () => {}
  });
  assert.equal(importBtn.classList.contains("hidden"), true);
});

test("renderSourceChips master.db chip has generic label and aria-label", () => {
  const dom = new JSDOM(`<!doctype html><body><div id="chips"></div></body>`);
  const document = dom.window.document;
  const state = {
    sourceRoots: [],
    sourceRootEnabled: {},
    tracks: [],
    externalMasterDbPath: "/path/to/master.db",
    masterDbEnabled: true
  };
  const el = { sourceChipsContainer: document.querySelector("#chips") };
  renderSourceChips(state, el, {
    documentObj: document,
    persistSourceRootEnabled: () => {},
    updateScanLibraryButtonLabel: () => {},
    updateSourceFilterIndicator: () => {}
  });
  const chip = el.sourceChipsContainer.querySelector(".source-chip-master-db");
  assert.ok(chip, "master.db chip should be rendered");
  const checkbox = chip.querySelector("input[data-master-db]");
  assert.equal(checkbox.getAttribute("aria-label"), "Toggle desktop library");
  const label = chip.querySelector(".source-chip-path");
  assert.equal(label.textContent, "master.db");
});

test("scanMasterDb emits generic status on success", async () => {
  const statuses = [];
  const state = { externalMasterDbPath: "/path/to/master.db" };
  await scanMasterDb(state, {
    emitStatus: (msg) => statuses.push(msg),
    command: async () => ({ indexed: 3, updated: 1, notFound: [], warnings: [] }),
    resetAndLoadLibraryTracks: async () => {},
    LIBRARY_LOAD_LIMIT_POST_SCAN: 500,
    refreshCurrentPlaylistTracks: async () => {},
    persistMasterDbEnabled: () => {},
    persistSourcesEverConfigured: () => {},
    renderSourceChips: () => {},
    logWarnings: () => {}
  });
  assert.equal(statuses[0], "Importing from desktop library...");
  assert.ok(statuses[statuses.length - 1].startsWith("Desktop library import done:"), statuses[statuses.length - 1]);
});

test("scanMasterDb emits generic error status on command failure", async () => {
  const statuses = [];
  const state = { externalMasterDbPath: "/path/to/master.db" };
  await scanMasterDb(state, {
    emitStatus: (msg) => statuses.push(msg),
    command: async () => { throw new Error("db locked"); },
    resetAndLoadLibraryTracks: async () => {},
    LIBRARY_LOAD_LIMIT_POST_SCAN: 500,
    refreshCurrentPlaylistTracks: async () => {},
    persistMasterDbEnabled: () => {},
    persistSourcesEverConfigured: () => {},
    renderSourceChips: () => {},
    logWarnings: () => {}
  });
  assert.equal(statuses[0], "Importing from desktop library...");
  assert.ok(statuses[1].startsWith("Desktop library import failed:"), statuses[1]);
});
