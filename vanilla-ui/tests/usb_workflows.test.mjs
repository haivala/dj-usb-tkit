import test from "node:test";
import assert from "node:assert/strict";

import {
  exportHistoryTracklist,
  exportPlaylistToUsb,
  handleUsbPlayerMenuListClick,
  refreshHistory,
  refreshUsb,
  renderUsbPlayerMenuEditor,
  runUsbDiagnostics,
  sanitizeTracklistFileName,
  syncUsbPlayerMenuEditorControls,
} from "../components/usb/actions.mjs";
import { buildTracklistText } from "../track_utils.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

// An app ctx for USB refresh flows with the progress footer stubbed out.
function usbCtx(overrides = {}) {
  return makeTestCtx({
    setProgress: () => {},
    startProgressHeartbeat: () => {},
    stopProgressHeartbeat: () => {},
    logWarnings: () => {},
    renderCurrentPlaylistTracksFromState: async () => {},
    ...overrides
  });
}

test("runUsbDiagnostics refreshes the open playlist's reorder lock from the new status", async () => {
  let renderOpenPlaylist = 0;
  const ctx = usbCtx({
    command: async () => ({
      overallStatus: "PASS",
      durationMs: 1,
      warnings: [],
      playlistUsbExportStatus: [
        { playlistId: "p1", playlistName: "Testi", sameNameExistsOnUsb: true, locksReorder: true }
      ]
    }),
    renderCurrentPlaylistTracksFromState: async () => { renderOpenPlaylist += 1; }
  });
  ctx.state.usbRoot = "/USB";
  await runUsbDiagnostics(ctx);
  assert.equal(renderOpenPlaylist, 1);
  assert.equal(ctx.state.playlistUsbExportStatusById.get("p1").locksReorder, true);
  assert.equal(ctx.el.diagOverallStatus.textContent, "PASS");
});

test("refreshUsb re-renders the open playlist after replacing the export status map", async () => {
  let renderOpenPlaylist = 0;
  const ctx = usbCtx({
    command: async () => ({ items: [], warnings: [], playlistUsbExportStatus: [] }),
    renderCurrentPlaylistTracksFromState: async () => { renderOpenPlaylist += 1; }
  });
  ctx.state.usbRoot = "/USB";
  await refreshUsb(ctx);
  assert.equal(renderOpenPlaylist, 1);
});

test("refreshUsb renders the backend-computed playlistTrackTotal, not a client sum", async () => {
  const ctx = usbCtx({
    command: async () => ({
      // per-playlist trackCount sums to 5, but the header must use the
      // backend's playlistTrackTotal verbatim
      items: [{ id: "a", name: "A", trackCount: 2, tracks: [] }, { id: "b", name: "B", trackCount: 3, tracks: [] }],
      playlistTrackTotal: 5,
      warnings: [],
      playlistUsbExportStatus: []
    })
  });
  ctx.state.usbRoot = "/USB";
  await refreshUsb(ctx);
  assert.equal(ctx.el.usbCountsText.textContent, "2 playlists, 5 tracks");
  assert.equal(ctx.el.usbPlaylists.querySelectorAll("[data-usb-playlist-index]").length, 2);
});

test("diagnostics guard, history refresh, and tracklist filename sanitizing stay stable", async () => {
  let status = "";
  const guarded = usbCtx({ emitStatus: (text) => { status = text; } });
  await runUsbDiagnostics(guarded);
  assert.equal(status, "Select USB folder first");

  const ctx = usbCtx({
    emitStatus: (text) => { status = text; },
    command: async () => ({
      items: [{ name: "H1", tracks: [{ id: "1", title: "A" }] }],
      counts: { importedPlaylists: 1, importedTracks: 1 },
      warnings: ["warn"]
    }),
    normalizeTrack: (track) => ({ ...track, normalized: true }),
    countWarningsForStatus: () => 1
  });
  ctx.state.usbRoot = "/USB";
  await refreshHistory(ctx);

  assert.equal(ctx.state.histories[0].tracks[0].normalized, true);
  assert.equal(ctx.el.historyCountsText.textContent, "1 sessions, 1 tracks");
  assert.equal(ctx.el.historyList.querySelectorAll("[data-history-index]").length, 1);
  assert.match(status, /USB histories loaded: 1 \| \(1 warning\(s\)\)/);

  assert.equal(sanitizeTracklistFileName("HISTORY 003"), "HISTORY 003.txt");
  assert.equal(sanitizeTracklistFileName('Set: A/B "Live"?'), "Set- A-B -Live-.txt");
  assert.equal(sanitizeTracklistFileName("   "), "tracklist.txt");
});

test("exportHistoryTracklist guards, cancels, saves sliced tracks, and reports save dismissal", async () => {
  let status = "";
  let invokeCalls = 0;
  await exportHistoryTracklist({
    state: { histories: [], selectedHistoryIndex: null, historyTracks: [] },
    emitStatus: (text) => { status = text; },
    invoke: async () => {
      invokeCalls += 1;
      return true;
    }
  });
  assert.equal(status, "Select a history session first");
  assert.equal(invokeCalls, 0);

  const baseState = {
    histories: [{ name: "HISTORY 001" }],
    selectedHistoryIndex: 0,
    historyTracks: [
      { artist: "A", title: "One", durationMs: 1000 },
      { artist: "B", title: "Two", durationMs: 1000 },
      { artist: "C", title: "Three", durationMs: 1000 }
    ]
  };
  const exportCtx = (dialogChoice, overrides = {}) => ({
    state: baseState,
    emitStatus: (text) => { status = text; },
    invoke: async () => {
      invokeCalls += 1;
      return true;
    },
    tracklistExportDialog: { open: async () => dialogChoice },
    ...overrides
  });

  await exportHistoryTracklist(exportCtx(null));
  assert.equal(invokeCalls, 0, "a cancelled dialog saves nothing");

  let invokeArgs = null;
  let openArgs = null;
  await exportHistoryTracklist(exportCtx(null, {
    invoke: async (cmd, payload) => {
      invokeArgs = { cmd, payload };
      return true;
    },
    tracklistExportDialog: {
      open: async (opts) => {
        openArgs = opts;
        return { timeMode: "before", startIndex: 1 };
      }
    }
  }));
  assert.deepEqual(openArgs.tracks, baseState.historyTracks);
  assert.equal(invokeArgs.cmd, "save_text_file");
  assert.equal(invokeArgs.payload.suggestedFileName, "HISTORY 001.txt");
  assert.equal(invokeArgs.payload.contents, buildTracklistText(baseState.historyTracks.slice(1), "before"));
  assert.match(status, /Tracklist exported: HISTORY 001/);

  await exportHistoryTracklist(exportCtx({ timeMode: "off", startIndex: 99 }, {
    invoke: async (_cmd, payload) => {
      invokeArgs = { payload };
      return true;
    }
  }));
  assert.equal(invokeArgs.payload.contents, "C - Three", "an out-of-range start clamps to the last track");

  await exportHistoryTracklist(exportCtx({ timeMode: "off", startIndex: 0 }, { invoke: async () => false }));
  assert.equal(status, "Tracklist export cancelled");
});

function makeExportState(overrides = {}) {
  return {
    sourceRoots: [],
    missingSourceRoots: new Set(),
    playlists: [{ id: "p1", name: "Set", tracks: [{ id: "t1", filePath: "/music/track.mp3" }] }],
    usbRoot: "/USB",
    usbRootValid: true,
    usbWritable: true,
    exportPruneStale: true,
    exportBackup: false,
    activeJobId: null,
    currentPlaylistId: null,
    usbPlaylists: [],
    ...overrides
  };
}

// An app ctx showing a stale diagnostics report, with export collaborators
// stubbed; `overrides` replaces any of them.
function exportCtx(state, overrides = {}) {
  const ctx = makeTestCtx({
    state,
    setProgress: () => {},
    startProgressHeartbeat: () => {},
    stopProgressHeartbeat: () => {},
    command: async () => ({ exportedTracks: 1, skippedTracks: 0, warnings: [] }),
    countWarningsForStatus: () => 0,
    warningEntryLevel: () => "info",
    logWarnings: () => {},
    emitMessage: () => {},
    loadPlaylists: async () => {},
    updateModeText: () => {},
    switchView: async () => {},
    commitActivePlaylistSort: async () => {},
    ...overrides
  });
  ctx.el.diagOverallStatus.textContent = "WARN";
  return ctx;
}

test("exportPlaylistToUsb reports local blockers and generic command failures", async () => {
  let status = "";
  const logged = [];

  await assert.rejects(
    exportPlaylistToUsb(exportCtx(makeExportState(), {
      emitStatus: (text) => { status = text; },
      command: async () => { throw new Error("boom"); },
      emitMessage: (message) => logged.push(message)
    }), "p1")
  );
  assert.match(status, /Export failed: boom/);
  assert.equal(logged[0].code, "export.failure");

  // A missing source folder is now a backend export-gate rejection
  // (validationType "source_root_missing") -- the frontend renders it, no
  // client-side path matching.
  await assert.rejects(
    exportPlaylistToUsb(
      exportCtx(makeExportState({
        playlists: [{ id: "p1", name: "Set", tracks: [{ id: "t1", filePath: "/music/missing/Artist - Track.mp3" }] }]
      }), {
        emitStatus: (text) => { status = text; },
        command: async () => {
          const err = new Error("export blocked: source folder is missing: /music/missing. Relocate or remove it first.");
          err.details = { validationType: "source_root_missing", missingRoots: ["/music/missing"] };
          throw err;
        }
      }),
      "p1"
    )
  );
  assert.match(status, /Export blocked: source folder is missing: \/music\/missing/);
});

test("exportPlaylistToUsb clears diagnostics after success and forwards backup option", async () => {
  for (const exportBackup of [true, false]) {
    let capturedOptions = null;
    const ctx = exportCtx(makeExportState({ exportBackup }), {
      command: async (_cmd, args) => {
        capturedOptions = args?.options;
        return { exportedTracks: 1, skippedTracks: 0, warnings: [] };
      }
    });
    await exportPlaylistToUsb(ctx, "p1");

    assert.equal(ctx.el.diagOverallStatus.textContent, "", "the stale diagnostics report is cleared");
    assert.equal(capturedOptions?.backupBeforeExport, exportBackup);
  }
});

test("exportPlaylistToUsb commits an active sort before exporting", async () => {
  const calls = [];
  await exportPlaylistToUsb(exportCtx(makeExportState(), {
    commitActivePlaylistSort: async (playlistId) => { calls.push(`commit:${playlistId}`); },
    command: async (cmd) => { calls.push(cmd); return { exportedTracks: 1, skippedTracks: 0, warnings: [] }; }
  }), "p1");

  assert.deepEqual(calls, ["commit:p1", "export_to_usb"]);
});

test("exportPlaylistToUsb blocks export when the sort commit fails, without calling export_to_usb", async () => {
  let status = "";
  let exportCalled = false;

  await exportPlaylistToUsb(exportCtx(makeExportState(), {
    emitStatus: (text) => { status = text; },
    commitActivePlaylistSort: async () => { throw new Error("disk full"); },
    command: async () => { exportCalled = true; return {}; }
  }), "p1");

  assert.equal(exportCalled, false);
  assert.match(status, /Export blocked: couldn't save the current sort order \(disk full\)/);
});

test("player menu single-select clears opposite list and enables proper actions", () => {
  const ctx = makeTestCtx();
  const { state, el } = ctx;
  Object.assign(state, {
    usbRoot: "/USB",
    usbRootValid: true,
    usbPlayerMenuAvailable: [
      { kind: 133, name: "BPM", origin: "both" },
      { kind: 134, name: "RATING", origin: "both" },
    ],
    usbPlayerMenuCurrent: [
      { kind: 132, name: "PLAYLIST", origin: "both", removable: false },
      { kind: 139, name: "KEY", origin: "both", removable: true },
    ],
    usbPlayerMenuAvailableSelectedKind: null,
    usbPlayerMenuCurrentSelectedKind: null,
  });

  renderUsbPlayerMenuEditor(ctx);
  handleUsbPlayerMenuListClick(ctx, "available", {
    target: el.usbPlayerMenuAvailable.querySelector(".player-menu-item[data-menu-kind='133']")
  });
  syncUsbPlayerMenuEditorControls(ctx);
  assert.equal(el.usbPlayerMenuAddBtn.disabled, false);
  assert.equal(el.usbPlayerMenuRemoveBtn.disabled, true);
  assert.equal(state.usbPlayerMenuCurrentSelectedKind, null);

  handleUsbPlayerMenuListClick(ctx, "current", {
    target: el.usbPlayerMenuCurrent.querySelector(".player-menu-item[data-menu-kind='139']")
  });
  syncUsbPlayerMenuEditorControls(ctx);
  assert.equal(el.usbPlayerMenuAddBtn.disabled, true);
  assert.equal(el.usbPlayerMenuRemoveBtn.disabled, false);
  assert.equal(el.usbPlayerMenuUpBtn.disabled, false);
  assert.equal(el.usbPlayerMenuDownBtn.disabled, true);
  assert.equal(state.usbPlayerMenuAvailableSelectedKind, null);
  assert.equal(
    el.usbPlayerMenuCurrent.querySelector(".player-menu-item[data-menu-kind='139']")?.classList.contains("is-selected"),
    true,
  );

  // Selecting a backend-flagged non-removable item disables the Remove button.
  handleUsbPlayerMenuListClick(ctx, "current", {
    target: el.usbPlayerMenuCurrent.querySelector(".player-menu-item[data-menu-kind='132']")
  });
  syncUsbPlayerMenuEditorControls(ctx);
  assert.equal(el.usbPlayerMenuRemoveBtn.disabled, true);
});
