import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  applySearchLocalFilter,
  getLibraryVisibleTracks,
  loadTracks,
  normalizeTrack,
  refreshSourceRootAnalysisStatus,
  relocateSourceRoot,
  renderSourceChips,
  scanMasterDb
} from "../components/library/actions.mjs";

function makeChipHarness(state, { includeSearch = false, deps = {} } = {}) {
  const dom = new JSDOM(`<!doctype html><body>
    <div id="chips"></div>${includeSearch ? '<input id="search" />' : ""}
  </body>`);
  const document = dom.window.document;
  const calls = { persisted: null, scanLabels: 0, indicators: 0 };
  const el = {
    sourceChipsContainer: document.querySelector("#chips"),
    ...(includeSearch ? { librarySearch: document.querySelector("#search") } : {})
  };
  const mergedDeps = {
    documentObj: document,
    escapeHtml: (value) => String(value),
    persistSourceRootEnabled: (map) => { calls.persisted = { ...map }; },
    updateScanLibraryButtonLabel: () => { calls.scanLabels += 1; },
    updateSourceFilterIndicator: () => { calls.indicators += 1; },
    ...deps
  };
  return {
    calls,
    el,
    render: () => renderSourceChips(state, el, mergedDeps),
    chips: () => el.sourceChipsContainer.querySelectorAll(".source-chip")
  };
}

async function runScanMasterDb(command) {
  const statuses = [];
  const logged = [];
  await scanMasterDb({ externalMasterDbPath: "/path/to/master.db" }, {
    emitStatus: (message) => statuses.push(message),
    command,
    resetAndLoadLibraryTracks: async () => {},
    LIBRARY_LOAD_LIMIT_POST_SCAN: 500,
    refreshCurrentPlaylistTracks: async () => {},
    persistMasterDbEnabled: () => {},
    persistSourcesEverConfigured: () => {},
    renderSourceChips: () => {},
    logWarnings: (source, warnings) => { logged.push({ source, warnings }); }
  });
  return { statuses, logged };
}

test("getLibraryVisibleTracks returns filtered list", () => {
  assert.deepEqual(getLibraryVisibleTracks({ filteredTracks: [{ id: 1 }] }), [{ id: 1 }]);
});

test("applySearchLocalFilter handles empty sources, query filtering, and selection pruning", () => {
  const emptyState = {
    sourceRoots: [],
    masterDbEnabled: false,
    tracks: [{ id: "a" }],
    selectedTrackIds: new Set(["a"]),
    filteredTracks: [{ id: "a" }]
  };
  let renders = 0;
  let selectionUpdates = 0;
  applySearchLocalFilter(emptyState, { librarySearch: { value: "" } }, {
    enabledSourceRoots: (roots) => roots,
    trackPathMatchesAnyRoot: () => false,
    renderLibraryRows: () => { renders += 1; },
    updateSelectionCount: () => { selectionUpdates += 1; }
  });
  assert.equal(emptyState.filteredTracks.length, 0);
  assert.equal(emptyState.selectedTrackIds.size, 0);
  assert.equal(renders, 1);
  assert.equal(selectionUpdates, 1);

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
    enabledSourceRoots: (roots, enabled) => roots.filter((root) => enabled[root] !== false),
    trackPathMatchesAnyRoot: (filePath, roots) => roots.some((root) => String(filePath).startsWith(root)),
    renderLibraryRows: () => {},
    updateSelectionCount: () => {}
  });
  assert.deepEqual(state.filteredTracks.map((track) => track.id), ["1"]);
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
        hasMore: true,
        sourceRootAnalysis: [{ sourceRoot: "/music/a", total: 1, analyzed: 1, fullyAnalyzed: true }]
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
  assert.equal(state.sourceRootAnalysisStatus["/music/a"], true);
});

test("renderSourceChips renders analyzed, missing, and disabled chip states", () => {
  // "Fully analyzed" per root is owned by the backend (state.sourceRootAnalysisStatus,
  // populated from `sourceRootAnalysis` in browse responses). renderSourceChips
  // only renders it -- it never inspects individual tracks.
  const state = {
    sourceRoots: ["/music/a", "/music/b"],
    sourceRootEnabled: {},
    sourceRootAnalysisStatus: { "/music/a": true, "/music/b": false },
    tracks: []
  };
  const harness = makeChipHarness(state);
  harness.render();

  let chips = harness.chips();
  assert.equal(chips.length, 2);
  assert.equal(chips[0].classList.contains("source-chip-analyzed"), true);
  assert.equal(chips[1].classList.contains("source-chip-analyzed"), false);
  assert.deepEqual(Object.keys(harness.calls.persisted).sort(), ["/music/a", "/music/b"]);
  assert.equal(harness.calls.scanLabels, 1);
  assert.equal(harness.calls.indicators, 1);

  state.sourceRootEnabled["/music/b"] = false;
  harness.render();
  chips = harness.chips();
  assert.equal(chips[0].classList.contains("source-chip-analyzed"), true);
  assert.equal(chips[1].classList.contains("source-chip-analyzed"), false);
  assert.equal(chips[1].querySelector(".source-chip-toggle").checked, false);
});

test("renderSourceChips renders missing source roots as unchecked relocation chips", () => {
  const state = {
    sourceRoots: ["/music/missing"],
    sourceRootEnabled: { "/music/missing": true },
    missingSourceRoots: new Set(["/music/missing"]),
    sourceRootAnalysisStatus: {},
    tracks: []
  };
  const harness = makeChipHarness(state);
  harness.render();

  const chip = harness.chips()[0];
  const checkbox = chip.querySelector(".source-chip-toggle");
  assert.ok(chip.classList.contains("source-chip-missing"));
  assert.equal(chip.dataset.sourceRelocateIndex, "0");
  assert.equal(checkbox.checked, false);
  assert.equal(checkbox.disabled, true);
  assert.equal(checkbox.getAttribute("aria-label"), "Source folder missing");
  assert.match(chip.querySelector(".source-chip-path").getAttribute("data-tooltip"), /Click to relocate/);
});

test("renderSourceChips renders sourceRootAnalysisStatus verbatim and never recomputes it", () => {
  // No client-side recompute: whatever the backend put in
  // sourceRootAnalysisStatus is what renders, regardless of the loaded tracks
  // (a partial page, an active query, etc. can't flip it).
  const state = {
    sourceRoots: ["/music/a", "/music/b"],
    sourceRootEnabled: { "/music/a": true, "/music/b": true },
    sourceRootAnalysisStatus: { "/music/a": true, "/music/b": false },
    tracks: [
      // Deliberately "analyzed-looking" tracks under /music/b -- must NOT flip
      // its chip, because the backend said it's not fully analyzed.
      { filePath: "/music/b/1.mp3", durationMs: 120000, analysisReady: true }
    ],
    libraryHasMore: true,
    libraryLoadedTotal: 999,
    libraryQuery: ""
  };
  const harness = makeChipHarness(state, { includeSearch: true });
  harness.render();
  assert.equal(harness.chips()[0].classList.contains("source-chip-analyzed"), true);
  assert.equal(harness.chips()[1].classList.contains("source-chip-analyzed"), false);
  // Map is untouched by rendering.
  assert.deepEqual(state.sourceRootAnalysisStatus, { "/music/a": true, "/music/b": false });
});

test("relocateSourceRoot replaces source and preserves playlist track identity state", async () => {
  const state = {
    sourceRoots: ["/music/old"],
    sourceRootEnabled: { "/music/old": true },
    missingSourceRoots: new Set(["/music/old"]),
    libraryQuery: ""
  };
  const calls = [];
  const statuses = [];
  let persistedRoots = null;
  let persistedEnabled = null;
  let rendered = 0;
  let reloaded = 0;
  let refreshedPlaylists = 0;

  await relocateSourceRoot(state, "/music/old", {
    pickSourceFolders: async () => ["/music/new"],
    command: async (name, payload) => {
      calls.push({ name, payload });
      return {
        oldRoot: payload.oldRoot,
        newRoot: payload.newRoot,
        matched: 2,
        updated: 2,
        unchanged: 0,
        missingAtNewRoot: 0,
        conflicts: 0
      };
    },
    persistSourceRoots: (roots) => { persistedRoots = [...roots]; },
    persistSourceRootEnabled: (enabled) => { persistedEnabled = { ...enabled }; },
    syncAssetScopePaths: async () => {},
    renderSourceChips: () => { rendered += 1; },
    resetAndLoadLibraryTracks: async () => { reloaded += 1; },
    refreshCurrentPlaylistTracks: async () => { refreshedPlaylists += 1; },
    refreshMissingSourceRoots: async () => { state.missingSourceRoots = new Set(); },
    LIBRARY_LOAD_LIMIT_DEFAULT: 25,
    emitStatus: (message) => statuses.push(message)
  });

  assert.deepEqual(calls, [
    { name: "relocate_source_root", payload: { oldRoot: "/music/old", newRoot: "/music/new" } }
  ]);
  assert.deepEqual(state.sourceRoots, ["/music/new"]);
  assert.deepEqual(persistedRoots, ["/music/new"]);
  assert.equal(persistedEnabled["/music/new"], true);
  assert.equal(Object.hasOwn(persistedEnabled, "/music/old"), false);
  assert.ok(rendered >= 1);
  assert.equal(reloaded, 1);
  assert.equal(refreshedPlaylists, 1);
  assert.ok(statuses.at(-1).includes("2 track path(s) updated"));
});

test("scanMasterDb reports success, failure, and structured warnings", async () => {
  const success = await runScanMasterDb(async () => ({
    indexed: 3,
    updated: 1,
    notFound: [],
    warnings: []
  }));
  assert.equal(success.statuses[0], "Importing from desktop library...");
  assert.ok(success.statuses.at(-1).startsWith("Desktop library import done:"), success.statuses.at(-1));

  const failure = await runScanMasterDb(async () => { throw new Error("db locked"); });
  assert.equal(failure.statuses[0], "Importing from desktop library...");
  assert.ok(failure.statuses[1].startsWith("Desktop library import failed:"), failure.statuses[1]);

  const warning = await runScanMasterDb(async () => ({
    indexed: 3,
    updated: 1,
    notFound: [],
    warnings: [{
      level: "warn",
      code: "master_db.scan_diag",
      message: "3 file(s) had unreadable ANLZ analysis",
      source: "scan_master_db"
    }]
  }));
  assert.equal(warning.logged.length, 1);
  assert.equal(warning.logged[0].warnings.length, 1);
  assert.equal(warning.logged[0].warnings[0].level, "warn");
  assert.equal(warning.logged[0].warnings[0].message, "3 file(s) had unreadable ANLZ analysis");
  assert.notEqual(typeof warning.logged[0].warnings[0].message, "object");
});

test("refreshSourceRootAnalysisStatus queries non-missing roots and skips all-missing sets", async () => {
  const state = {
    sourceRoots: ["/music/a", "/music/b"],
    sourceRootEnabled: { "/music/a": true, "/music/b": false },
    masterDbEnabled: true,
    sourceRootAnalysisStatus: {}
  };
  const calls = [];
  let rendered = 0;

  await refreshSourceRootAnalysisStatus(state, {
    command: async (name, payload) => {
      calls.push({ name, payload });
      return {
        sourceRootAnalysis: [
          { sourceRoot: "/music/a", total: 3, analyzed: 3, fullyAnalyzed: true },
          { sourceRoot: "/music/b", total: 2, analyzed: 2, fullyAnalyzed: true }
        ]
      };
    },
    renderSourceChips: () => { rendered += 1; }
  });

  assert.equal(calls.length, 1);
  assert.equal(calls[0].name, "browse_source_files");
  assert.deepEqual(calls[0].payload.sourceRoots, ["/music/a", "/music/b"]);
  assert.equal(calls[0].payload.includeMasterDb, false);
  assert.equal(calls[0].payload.limit, 1);
  assert.equal(state.sourceRootAnalysisStatus["/music/a"], true);
  assert.equal(state.sourceRootAnalysisStatus["/music/b"], true);
  assert.equal(rendered, 1);

  const missing = {
    sourceRoots: ["/music/a"],
    sourceRootEnabled: { "/music/a": true },
    missingSourceRoots: new Set(["/music/a"]),
    masterDbEnabled: false,
    sourceRootAnalysisStatus: {}
  };
  let noOpCalls = 0;
  rendered = 0;
  await refreshSourceRootAnalysisStatus(missing, {
    command: async () => { noOpCalls += 1; return {}; },
    renderSourceChips: () => { rendered += 1; }
  });
  assert.equal(noOpCalls, 0);
  assert.equal(rendered, 0);
});
