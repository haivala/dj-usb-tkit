import test from "node:test";
import assert from "node:assert/strict";
import {
  refreshLibrarySources,
  relocateSourceRoot,
  renderSourceChips,
  scanRekordboxDb
} from "../components/library/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

// The backend's `get_library_sources` answer.
function sources(folders, extra = {}) {
  return {
    folders: folders.map((folder) => ({ label: folder.path, parent: null, enabled: true, missing: false, fullyAnalyzed: false, ...folder })),
    rekordboxDbEnabled: false,
    mixxxDbEnabled: false,
    everConfigured: folders.length > 0,
    ...extra,
  };
}

function makeChipHarness(librarySources) {
  const calls = { scanLabels: 0, indicators: 0 };
  const ctx = makeTestCtx({
    updateAnalyzeLibraryButtonLabel: () => { calls.scanLabels += 1; },
    updateSourceFilterIndicator: () => { calls.indicators += 1; },
  });
  ctx.state.librarySources = librarySources;
  return {
    calls,
    ctx,
    render: () => renderSourceChips(ctx),
    chips: () => ctx.el.sourceChipsContainer.querySelectorAll(".source-chip")
  };
}

// `scan` answers the import; the import then switches the library's chip on.
async function runScanRekordboxDb(scan) {
  const statuses = [];
  const logged = [];
  const switched = [];
  const ctx = makeTestCtx({
    emitStatus: (message) => statuses.push(message),
    command: async (name, payload) => {
      if (name === "set_external_library_enabled") {
        switched.push(payload);
        return sources([], { rekordboxDbEnabled: payload.enabled, everConfigured: true });
      }
      return name === "scan_rekordbox_db" ? scan() : {};
    },
    refreshCurrentPlaylistTracks: async () => {},
    detectExternalRekordboxDb: async () => {},
    logWarnings: (source, warnings) => { logged.push({ source, warnings }); }
  });
  ctx.state.externalRekordboxDbPath = "/path/to/master.db";
  await scanRekordboxDb(ctx);
  return { statuses, logged, switched, ctx };
}

// NOTE: library list fetch/pagination/search/sort + the "enabled folders +
// master.db -> one browse_source_files request" behaviour now live in the
// shared TrackListController (components/shared/track_list_controller.mjs, wired
// in main.js) and are covered by tests/track_list_controller.test.mjs plus the
// e2e specs (sort_cycling, selection_add, scan_analysis_batches,
// source_root_removal, waveform_startup_hydration).

test("renderSourceChips draws the backend's analyzed, missing, and switched-off folders as given", () => {
  // Every folder fact is the backend's: the chip never looks at tracks.
  const harness = makeChipHarness(sources([
    { path: "/music/a", fullyAnalyzed: true },
    { path: "/music/b", enabled: false },
    { path: "/music/missing", missing: true },
  ]));
  harness.ctx.state.tracks = [{ filePath: "/music/b/1.mp3", analysisReady: true }];
  harness.render();

  const chips = harness.chips();
  assert.equal(chips.length, 3);
  assert.equal(chips[0].classList.contains("source-chip-analyzed"), true);
  assert.equal(chips[1].classList.contains("source-chip-analyzed"), false);
  assert.equal(chips[0].querySelector(".source-chip-toggle").checked, true);
  assert.equal(chips[1].querySelector(".source-chip-toggle").checked, false);
  assert.equal(harness.calls.scanLabels, 1);
  assert.equal(harness.calls.indicators, 1);

  const missing = chips[2];
  const checkbox = missing.querySelector(".source-chip-toggle");
  assert.ok(missing.classList.contains("source-chip-missing"));
  assert.equal(missing.dataset.sourceRelocateIndex, "2");
  assert.equal(checkbox.checked, false);
  assert.equal(checkbox.disabled, true);
  assert.equal(checkbox.getAttribute("aria-label"), "Source folder missing");
  assert.match(missing.querySelector(".source-chip-path").getAttribute("data-tooltip"), /Click to relocate/);
});

test("relocateSourceRoot shows the folders the backend answers with and reloads the library", async () => {
  const calls = [];
  const statuses = [];
  let refreshedPlaylists = 0;

  const ctx = makeTestCtx({
    pickSourceFolders: async () => ["/music/new"],
    command: async (name, payload) => {
      calls.push({ name, payload });
      if (name !== "relocate_source_root") return {};
      return {
        oldRoot: payload.oldRoot,
        newRoot: payload.newRoot,
        matched: 2,
        updated: 2,
        unchanged: 0,
        missingAtNewRoot: 0,
        conflicts: 0,
        sources: sources([{ path: "/music/new" }]),
        statusText: "Source relocated: 2 track paths updated",
      };
    },
    refreshCurrentPlaylistTracks: async () => { refreshedPlaylists += 1; },
    emitStatus: (message) => statuses.push(message)
  });
  ctx.state.librarySources = sources([{ path: "/music/old", missing: true }]);

  await relocateSourceRoot(ctx, "/music/old");

  assert.deepEqual(calls[0], { name: "relocate_source_root", payload: { oldRoot: "/music/old", newRoot: "/music/new" } });
  assert.ok(calls.some((c) => c.name === "browse_source_files"), "the library reloads from the new root");
  assert.equal(ctx.el.sourceChipsContainer.querySelector(".source-chip-path").textContent, "/music/new");
  assert.equal(refreshedPlaylists, 1);
  assert.equal(statuses.at(-1), "Source relocated: 2 track paths updated", "the backend's status line");
});

test("refreshLibrarySources draws what get_library_sources answers", async () => {
  const harness = makeChipHarness(sources([]));
  const calls = [];
  harness.ctx.command = async (name) => {
    calls.push(name);
    return sources([{ path: "/music/a", fullyAnalyzed: true }]);
  };
  await refreshLibrarySources(harness.ctx);

  assert.deepEqual(calls, ["get_library_sources"]);
  assert.equal(harness.chips().length, 1);
  assert.equal(harness.chips()[0].classList.contains("source-chip-analyzed"), true);
});

test("scanRekordboxDb reports success, failure, and structured warnings", async () => {
  const success = await runScanRekordboxDb(async () => ({
    indexed: 3,
    updated: 1,
    warnings: [],
    statusText: "Desktop library import done: 3 new, 1 updated"
  }));
  assert.equal(success.statuses[0], "Importing from desktop library...");
  assert.equal(success.statuses.at(-1), "Desktop library import done: 3 new, 1 updated");
  assert.deepEqual(success.switched, [{ library: "rekordbox", enabled: true }]);
  assert.equal(success.ctx.state.librarySources.rekordboxDbEnabled, true);

  const failure = await runScanRekordboxDb(async () => { throw new Error("db locked"); });
  assert.equal(failure.statuses[0], "Importing from desktop library...");
  assert.ok(failure.statuses[1].startsWith("Desktop library import failed:"), failure.statuses[1]);

  // The backend logged its warnings (missing files too) when it made them: JS logs nothing.
  const warning = await runScanRekordboxDb(async () => ({
    indexed: 3,
    updated: 1,
    warnings: [{
      level: "warn",
      code: "scan.rekordbox-db.file-not-found",
      message: "track file not found, skipped: /music/gone.mp3",
      source: "scan-rekordbox-db"
    }]
  }));
  assert.equal(warning.logged.length, 0);
});
