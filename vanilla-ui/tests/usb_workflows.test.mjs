import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

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

test("runUsbDiagnostics refreshes the open playlist's reorder lock from the new status", async () => {
  const state = { usbRoot: "/USB", playlistUsbExportStatusById: new Map() };
  let renderOpenPlaylist = 0;
  await runUsbDiagnostics(state, {
    setStatus: () => {},
    command: async () => ({
      durationMs: 1,
      warnings: [],
      playlistUsbExportStatus: [
        { playlistId: "p1", playlistName: "Testi", sameNameExistsOnUsb: true, locksReorder: true }
      ]
    }),
    updatePlaylistExportButtons: () => {},
    renderCurrentPlaylistTracksFromState: async () => { renderOpenPlaylist += 1; },
    renderDiagnosticsReport: () => {},
    logWarnings: () => {}
  });
  assert.equal(renderOpenPlaylist, 1);
  assert.equal(state.playlistUsbExportStatusById.get("p1").locksReorder, true);
});

test("refreshUsb re-renders the open playlist after replacing the export status map", async () => {
  const state = { usbRoot: "/USB", usbPlaylists: [], usbPlaylistTracks: [] };
  const el = { usbCountsText: { textContent: "" } };
  let renderOpenPlaylist = 0;
  await refreshUsb(state, el, {
    setStatus: () => {},
    command: async () => ({ items: [], warnings: [], playlistUsbExportStatus: [] }),
    setProgress: () => {},
    startProgressHeartbeat: () => {},
    stopProgressHeartbeat: () => {},
    normalizeUsbPlaylist: (p) => p,
    renderUsbPlaylists: () => {},
    clearUsbPlaylistTracks: () => {},
    renderCurrentPlaylistTracksFromState: async () => { renderOpenPlaylist += 1; },
    updatePlaylistExportButtons: () => {},
    countWarningsForStatus: () => 0,
    logWarnings: () => {}
  });
  assert.equal(renderOpenPlaylist, 1);
});

test("diagnostics guard, history refresh, and tracklist filename sanitizing stay stable", async () => {
  let status = "";
  await runUsbDiagnostics({ usbRoot: null }, {
    setStatus: (text) => { status = text; }
  });
  assert.equal(status, "Select USB folder first");

  const state = { usbRoot: "/USB", histories: [], historyTracks: [] };
  const el = { historyCountsText: { textContent: "" } };
  let renderedLists = 0;
  let renderedTracks = 0;

  await refreshHistory(state, el, {
    setStatus: (text) => { status = text; },
    command: async () => ({
      items: [{ name: "H1", tracks: [{ id: "1", title: "A" }] }],
      counts: { importedPlaylists: 1, importedTracks: 1 },
      warnings: ["warn"]
    }),
    normalizeTrack: (track, prefix) => ({ ...track, normalizedWith: prefix }),
    countWarningsForStatus: () => 1,
    logWarnings: () => {},
    renderHistoryList: () => { renderedLists += 1; },
    clearHistoryTracks: () => { renderedTracks += 1; }
  });

  assert.equal(state.histories[0].tracks[0].normalizedWith, "hist");
  assert.equal(el.historyCountsText.textContent, "1 sessions, 1 tracks");
  assert.equal(renderedLists, 1);
  assert.equal(renderedTracks, 1);
  assert.match(status, /USB histories loaded: 1 \| \(1 warning\(s\)\)/);

  assert.equal(sanitizeTracklistFileName("HISTORY 003"), "HISTORY 003.txt");
  assert.equal(sanitizeTracklistFileName('Set: A/B "Live"?'), "Set- A-B -Live-.txt");
  assert.equal(sanitizeTracklistFileName("   "), "tracklist.txt");
});

test("exportHistoryTracklist guards, cancels, saves sliced tracks, and reports save dismissal", async () => {
  let status = "";
  let invokeCalls = 0;
  await exportHistoryTracklist({ histories: [], selectedHistoryIndex: null, historyTracks: [] }, {}, {
    setStatus: (text) => { status = text; },
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
  let buildCalls = 0;
  await exportHistoryTracklist(baseState, {}, {
    setStatus: () => {},
    invoke: async () => {
      invokeCalls += 1;
      return true;
    },
    buildTracklistText: () => {
      buildCalls += 1;
      return "";
    },
    tracklistExportDialog: { open: async () => null }
  });
  assert.equal(buildCalls, 0);

  let invokeArgs = null;
  let buildArgs = null;
  let openArgs = null;
  await exportHistoryTracklist(baseState, {}, {
    setStatus: (text) => { status = text; },
    invoke: async (cmd, payload) => {
      invokeArgs = { cmd, payload };
      return true;
    },
    buildTracklistText: (tracks, timeMode) => {
      buildArgs = { tracks, timeMode };
      return "B - Two\nC - Three";
    },
    tracklistExportDialog: {
      open: async (opts) => {
        openArgs = opts;
        return { timeMode: "before", startIndex: 1 };
      }
    }
  });
  assert.deepEqual(openArgs.tracks, baseState.historyTracks);
  assert.deepEqual(buildArgs.tracks, baseState.historyTracks.slice(1));
  assert.equal(buildArgs.timeMode, "before");
  assert.equal(invokeArgs.cmd, "save_text_file");
  assert.equal(invokeArgs.payload.suggestedFileName, "HISTORY 001.txt");
  assert.equal(invokeArgs.payload.contents, "B - Two\nC - Three");
  assert.match(status, /Tracklist exported: HISTORY 001/);

  await exportHistoryTracklist(baseState, {}, {
    setStatus: () => {},
    invoke: async () => true,
    buildTracklistText: (tracks) => {
      buildArgs = { tracks };
      return "";
    },
    tracklistExportDialog: { open: async () => ({ timeMode: "off", startIndex: 99 }) }
  });
  assert.deepEqual(buildArgs.tracks, [baseState.historyTracks.at(-1)]);

  await exportHistoryTracklist(baseState, {}, {
    setStatus: (text) => { status = text; },
    invoke: async () => false,
    buildTracklistText: () => "",
    tracklistExportDialog: { open: async () => ({ timeMode: "off", startIndex: 0 }) }
  });
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
    usbPlaylistTracks: [],
    ...overrides
  };
}

function makeExportDeps(overrides = {}) {
  return {
    setStatus: () => {},
    setProgress: () => {},
    startProgressHeartbeat: () => {},
    nextPaint: async () => {},
    command: async () => ({ exportedTracks: 1, skippedTracks: 0, warnings: [] }),
    stopProgressHeartbeat: () => {},
    countWarningsForStatus: () => 0,
    warningEntryLevel: () => "info",
    logWarnings: () => {},
    pushEventLog: () => {},
    loadPlaylists: async () => {},
    updateModeText: () => {},
    switchView: async () => {},
    renderUsbPlaylists: () => {},
    clearUsbPlaylistTracks: () => {},
    refreshMissingSourceRoots: async () => [],
    clearUsbDiagnostics: () => {},
    ...overrides
  };
}

test("exportPlaylistToUsb reports local blockers and generic command failures", async () => {
  let status = "";
  const logged = [];

  await assert.rejects(
    exportPlaylistToUsb(makeExportState(), {}, "p1", makeExportDeps({
      setStatus: (text) => { status = text; },
      command: async () => { throw new Error("boom"); },
      pushEventLog: (entry) => logged.push(entry)
    }))
  );
  assert.match(status, /Export failed: boom/);
  assert.equal(logged[0].code, "export.failure");

  let exportCalled = false;
  await exportPlaylistToUsb(
    makeExportState({
      sourceRoots: ["/music/missing"],
      missingSourceRoots: new Set(["/music/missing"]),
      playlists: [{ id: "p1", name: "Set", tracks: [{ id: "t1", filePath: "/music/missing/Artist - Track.mp3" }] }]
    }),
    {},
    "p1",
    makeExportDeps({
      setStatus: (text) => { status = text; },
      command: async () => {
        exportCalled = true;
        return {};
      },
      refreshMissingSourceRoots: async () => ["/music/missing"]
    })
  );
  assert.equal(exportCalled, false);
  assert.match(status, /Export blocked: source folder is missing/);
});

test("exportPlaylistToUsb clears diagnostics after success and forwards backup option", async () => {
  for (const exportBackup of [true, false]) {
    let clearCalls = 0;
    let capturedOptions = null;
    await exportPlaylistToUsb(
      makeExportState({ exportBackup }),
      {},
      "p1",
      makeExportDeps({
        command: async (_cmd, args) => {
          capturedOptions = args?.options;
          return { exportedTracks: 1, skippedTracks: 0, warnings: [] };
        },
        clearUsbDiagnostics: () => { clearCalls += 1; }
      })
    );

    assert.equal(clearCalls, 1);
    assert.equal(capturedOptions?.backupBeforeExport, exportBackup);
  }
});

test("exportPlaylistToUsb commits an active sort before exporting", async () => {
  const calls = [];
  await exportPlaylistToUsb(
    makeExportState(),
    {},
    "p1",
    makeExportDeps({
      commitActivePlaylistSort: async (playlistId) => { calls.push(`commit:${playlistId}`); },
      command: async (cmd) => { calls.push(cmd); return { exportedTracks: 1, skippedTracks: 0, warnings: [] }; }
    })
  );

  assert.deepEqual(calls, ["commit:p1", "export_to_usb"]);
});

test("exportPlaylistToUsb blocks export when the sort commit fails, without calling export_to_usb", async () => {
  let status = "";
  let exportCalled = false;

  await exportPlaylistToUsb(
    makeExportState(),
    {},
    "p1",
    makeExportDeps({
      setStatus: (text) => { status = text; },
      commitActivePlaylistSort: async () => { throw new Error("disk full"); },
      command: async () => { exportCalled = true; return {}; }
    })
  );

  assert.equal(exportCalled, false);
  assert.match(status, /Export blocked: couldn't save the current sort order \(disk full\)/);
});

test("player menu single-select clears opposite list and enables proper actions", () => {
  const dom = new JSDOM(`<!doctype html><body>
    <button id="add"></button>
    <button id="remove"></button>
    <button id="up"></button>
    <button id="down"></button>
    <div id="available"></div>
    <div id="current"></div>
  </body>`);
  const document = dom.window.document;
  const state = {
    usbRoot: "/USB",
    usbRootValid: true,
    usbPlayerMenuAvailable: [
      { kind: 133, name: "BPM", origin: "both" },
      { kind: 134, name: "RATING", origin: "both" },
    ],
    usbPlayerMenuCurrent: [
      { kind: 132, name: "PLAYLIST", origin: "both" },
      { kind: 139, name: "KEY", origin: "both" },
    ],
    usbPlayerMenuAvailableSelectedKind: null,
    usbPlayerMenuCurrentSelectedKind: null,
  };
  const el = {
    usbPlayerMenuAddBtn: document.getElementById("add"),
    usbPlayerMenuRemoveBtn: document.getElementById("remove"),
    usbPlayerMenuUpBtn: document.getElementById("up"),
    usbPlayerMenuDownBtn: document.getElementById("down"),
    usbPlayerMenuAvailable: document.getElementById("available"),
    usbPlayerMenuCurrent: document.getElementById("current"),
  };

  renderUsbPlayerMenuEditor(state, el, { documentObj: document });
  handleUsbPlayerMenuListClick(
    state,
    el,
    { documentObj: document },
    "available",
    { target: el.usbPlayerMenuAvailable.querySelector(".player-menu-item[data-menu-kind='133']") }
  );
  syncUsbPlayerMenuEditorControls(state, el);
  assert.equal(el.usbPlayerMenuAddBtn.disabled, false);
  assert.equal(el.usbPlayerMenuRemoveBtn.disabled, true);
  assert.equal(state.usbPlayerMenuCurrentSelectedKind, null);

  handleUsbPlayerMenuListClick(
    state,
    el,
    { documentObj: document },
    "current",
    { target: el.usbPlayerMenuCurrent.querySelector(".player-menu-item[data-menu-kind='139']") }
  );
  syncUsbPlayerMenuEditorControls(state, el);
  assert.equal(el.usbPlayerMenuAddBtn.disabled, true);
  assert.equal(el.usbPlayerMenuRemoveBtn.disabled, false);
  assert.equal(el.usbPlayerMenuUpBtn.disabled, false);
  assert.equal(el.usbPlayerMenuDownBtn.disabled, true);
  assert.equal(state.usbPlayerMenuAvailableSelectedKind, null);
  assert.equal(
    el.usbPlayerMenuCurrent.querySelector(".player-menu-item[data-menu-kind='139']")?.classList.contains("is-selected"),
    true,
  );
});
