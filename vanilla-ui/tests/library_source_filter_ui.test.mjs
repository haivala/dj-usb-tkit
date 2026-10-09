import test from "node:test";
import assert from "node:assert/strict";
import {
  refreshSourceRootAnalysisStatus,
  relocateSourceRoot,
  renderSourceChips,
  scanRekordboxDb
} from "../components/library/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

function makeChipHarness(state) {
  const calls = { persisted: null, scanLabels: 0, indicators: 0 };
  const ctx = makeTestCtx({
    state,
    persistSourceRootEnabled: (map) => { calls.persisted = { ...map }; },
    updateScanLibraryButtonLabel: () => { calls.scanLabels += 1; },
    updateSourceFilterIndicator: () => { calls.indicators += 1; },
  });
  return {
    calls,
    ctx,
    render: () => renderSourceChips(ctx),
    chips: () => ctx.el.sourceChipsContainer.querySelectorAll(".source-chip")
  };
}

async function runScanRekordboxDb(command) {
  const statuses = [];
  const logged = [];
  const ctx = makeTestCtx({
    emitStatus: (message) => statuses.push(message),
    command,
    refreshCurrentPlaylistTracks: async () => {},
    detectExternalRekordboxDb: async () => {},
    logWarnings: (source, warnings) => { logged.push({ source, warnings }); }
  });
  ctx.state.externalRekordboxDbPath = "/path/to/master.db";
  await scanRekordboxDb(ctx);
  return { statuses, logged };
}

// NOTE: library list fetch/pagination/search/sort + the "enabled folders +
// master.db -> one browse_source_files request" behaviour now live in the
// shared TrackListController (components/shared/track_list_controller.mjs, wired
// in main.js) and are covered by tests/track_list_controller.test.mjs plus the
// e2e specs (sort_cycling, selection_add, scan_analysis_batches,
// source_root_removal, waveform_startup_hydration).

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
  const harness = makeChipHarness(state);
  harness.render();
  assert.equal(harness.chips()[0].classList.contains("source-chip-analyzed"), true);
  assert.equal(harness.chips()[1].classList.contains("source-chip-analyzed"), false);
  // Map is untouched by rendering.
  assert.deepEqual(state.sourceRootAnalysisStatus, { "/music/a": true, "/music/b": false });
});

test("relocateSourceRoot replaces source and preserves playlist track identity state", async () => {
  const calls = [];
  const statuses = [];
  let persistedRoots = null;
  let persistedEnabled = null;
  let refreshedPlaylists = 0;

  const ctx = makeTestCtx({
    pickSourceFolders: async () => ["/music/new"],
    command: async (name, payload) => {
      calls.push({ name, payload });
      if (name === "check_source_roots") return { missing: [] };
      if (name !== "relocate_source_root") return {};
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
    refreshCurrentPlaylistTracks: async () => { refreshedPlaylists += 1; },
    emitStatus: (message) => statuses.push(message)
  });
  const { state } = ctx;
  state.sourceRoots = ["/music/old"];
  state.sourceRootEnabled = { "/music/old": true };
  state.missingSourceRoots = new Set(["/music/old"]);

  await relocateSourceRoot(ctx, "/music/old");

  assert.deepEqual(calls[0], { name: "relocate_source_root", payload: { oldRoot: "/music/old", newRoot: "/music/new" } });
  assert.ok(calls.some((c) => c.name === "browse_source_files"), "the library reloads from the new root");
  assert.deepEqual(state.sourceRoots, ["/music/new"]);
  assert.deepEqual(persistedRoots, ["/music/new"]);
  assert.equal(persistedEnabled["/music/new"], true);
  assert.equal(Object.hasOwn(persistedEnabled, "/music/old"), false);
  assert.equal(ctx.el.sourceChipsContainer.querySelector(".source-chip-path").textContent, "/music/new");
  assert.equal(refreshedPlaylists, 1);
  assert.ok(statuses.at(-1).includes("2 track path(s) updated"));
});

test("relocateSourceRoot keeps the chip when the new root differs only in case", async () => {
  let persistedRoots = null;
  const ctx = makeTestCtx({
    pickSourceFolders: async () => ["/home/dj/Music/Sets"],
    command: async (name, payload) => {
      if (name === "check_source_roots") return { missing: [] };
      if (name !== "relocate_source_root") return {};
      return { ...payload, matched: 87, updated: 87, unchanged: 0, missingAtNewRoot: 0, conflicts: 0 };
    },
    persistSourceRoots: (roots) => { persistedRoots = [...roots]; },
  });
  const { state } = ctx;
  state.sourceRoots = ["/home/dj/music/Sets"];
  state.sourceRootEnabled = { "/home/dj/music/Sets": true };

  await relocateSourceRoot(ctx, "/home/dj/music/Sets");

  assert.deepEqual(state.sourceRoots, ["/home/dj/Music/Sets"]);
  assert.deepEqual(persistedRoots, ["/home/dj/Music/Sets"]);
  assert.deepEqual(state.sourceRootEnabled, { "/home/dj/Music/Sets": true });
  assert.equal(ctx.el.sourceChipsContainer.querySelector(".source-chip-path").textContent, "/home/dj/Music/Sets");
});

test("scanRekordboxDb reports success, failure, and structured warnings", async () => {
  const success = await runScanRekordboxDb(async () => ({
    indexed: 3,
    updated: 1,
    notFound: [],
    warnings: []
  }));
  assert.equal(success.statuses[0], "Importing from desktop library...");
  assert.ok(success.statuses.at(-1).startsWith("Desktop library import done:"), success.statuses.at(-1));

  const failure = await runScanRekordboxDb(async () => { throw new Error("db locked"); });
  assert.equal(failure.statuses[0], "Importing from desktop library...");
  assert.ok(failure.statuses[1].startsWith("Desktop library import failed:"), failure.statuses[1]);

  const warning = await runScanRekordboxDb(async () => ({
    indexed: 3,
    updated: 1,
    notFound: [],
    warnings: [{
      level: "warn",
      code: "rekordbox_db.scan_diag",
      message: "3 file(s) had unreadable ANLZ analysis",
      source: "scan_rekordbox_db"
    }]
  }));
  assert.equal(warning.logged.length, 1);
  assert.equal(warning.logged[0].warnings.length, 1);
  assert.equal(warning.logged[0].warnings[0].level, "warn");
  assert.equal(warning.logged[0].warnings[0].message, "3 file(s) had unreadable ANLZ analysis");
  assert.notEqual(typeof warning.logged[0].warnings[0].message, "object");
});

test("refreshSourceRootAnalysisStatus asks about every non-missing root and skips all-missing sets", async () => {
  const state = {
    sourceRoots: ["/music/a", "/music/b"],
    sourceRootEnabled: { "/music/a": true, "/music/b": false },
    rekordboxDbEnabled: true,
    sourceRootAnalysisStatus: {}
  };
  const calls = [];

  const harness = makeChipHarness(state);
  await refreshSourceRootAnalysisStatus(Object.assign(harness.ctx, {
    command: async (name, payload) => {
      calls.push({ name, payload });
      return {
        items: [
          { sourceRoot: "/music/a", total: 3, analyzed: 3, fullyAnalyzed: true },
          { sourceRoot: "/music/b", total: 2, analyzed: 2, fullyAnalyzed: true }
        ]
      };
    }
  }));

  assert.deepEqual(calls, [
    { name: "get_source_root_analysis", payload: { sourceRoots: ["/music/a", "/music/b"] } }
  ]);
  assert.equal(state.sourceRootAnalysisStatus["/music/a"], true);
  assert.equal(state.sourceRootAnalysisStatus["/music/b"], true);
  assert.equal(harness.chips()[1].classList.contains("source-chip-analyzed"), true, "chips re-render with the new status");

  const missing = {
    sourceRoots: ["/music/a"],
    sourceRootEnabled: { "/music/a": true },
    missingSourceRoots: new Set(["/music/a"]),
    rekordboxDbEnabled: false,
    sourceRootAnalysisStatus: {}
  };
  let noOpCalls = 0;
  await refreshSourceRootAnalysisStatus(makeTestCtx({
    state: missing,
    command: async () => { noOpCalls += 1; return {}; }
  }));
  assert.equal(noOpCalls, 0);
});
