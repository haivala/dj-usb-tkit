import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import {
  runUsbDiagnostics,
  refreshHistory,
  exportPlaylistToUsb,
  handleUsbPlayerMenuListClick,
  renderUsbPlayerMenuEditor,
  syncUsbPlayerMenuEditorControls,
} from "../components/usb/actions.mjs";

test("runUsbDiagnostics requires usb root", async () => {
  const state = { usbRoot: null };
  let status = "";

  await runUsbDiagnostics(state, {
    setStatus: (text) => { status = text; }
  });

  assert.equal(status, "Select USB folder first");
});

test("refreshHistory populates state and summary counts", async () => {
  const state = { usbRoot: "/USB", histories: [], historyTracks: [] };
  const el = {
    historyCountsText: { textContent: "" }
  };
  let renderedLists = 0;
  let renderedTracks = 0;
  let status = "";

  await refreshHistory(state, el, {
    setStatus: (text) => { status = text; },
    command: async () => ({
      items: [{ name: "H1", tracks: [{ id: "1", title: "A" }] }],
      counts: { importedPlaylists: 1, importedTracks: 1, pdbT11Playlists: 2, pdbT12Entries: 3 },
      warnings: ["warn"]
    }),
    normalizeTrack: (track, prefix) => ({ ...track, normalizedWith: prefix }),
    countWarningsForStatus: () => 1,
    logWarnings: () => {},
    renderHistoryList: () => { renderedLists += 1; },
    renderHistoryTracks: () => { renderedTracks += 1; }
  });

  assert.equal(state.histories.length, 1);
  assert.equal(state.histories[0].tracks[0].normalizedWith, "hist");
  assert.match(el.historyCountsText.textContent, /1 sessions, 1 tracks/);
  assert.equal(renderedLists, 1);
  assert.equal(renderedTracks, 1);
  assert.match(status, /USB histories loaded: 1 \(1 warning\(s\)\)/);
});

test("exportPlaylistToUsb blocks when playlist tracks are missing and logs generic failures", async () => {
  const state = {
    playlists: [{ id: "p1", name: "Set", tracks: [{ id: "t1" }] }],
    usbRoot: "/USB",
    usbRootValid: true,
    usbWritable: true,
    exportPruneStale: true,
    activeJobId: null,
    currentPlaylistId: null,
    usbPlaylists: ["old"],
    usbPlaylistTracks: ["old"]
  };
  const el = {
    usbSelectedPlaylistText: { textContent: "" }
  };
  let status = "";
  const logged = [];

  await assert.rejects(
    exportPlaylistToUsb(state, el, "p1", {
      setStatus: (text) => { status = text; },
      setProgress: () => {},
      startProgressHeartbeat: () => {},
      nextPaint: async () => {},
      command: async () => { throw new Error("boom"); },
      stopProgressHeartbeat: () => {},
      countWarningsForStatus: () => 0,
      warningEntryLevel: () => "info",
      logWarnings: () => {},
      pushEventLog: (entry) => logged.push(entry),
      loadPlaylists: async () => {},
      updateModeText: () => {},
      switchView: async () => {},
      renderUsbPlaylists: () => {},
      renderUsbPlaylistTracks: () => {}
    })
  );

  assert.match(status, /Export failed: boom/);
  assert.equal(logged[0].code, "export.failure");
});

test("exportPlaylistToUsb blocks playlists affected by missing source roots", async () => {
  const state = {
    sourceRoots: ["/music/missing"],
    missingSourceRoots: new Set(["/music/missing"]),
    playlists: [
      {
        id: "p1",
        name: "Set",
        tracks: [{ id: "t1", filePath: "/music/missing/Artist - Track.mp3" }]
      }
    ],
    usbRoot: "/USB",
    usbRootValid: true,
    usbWritable: true,
    exportPruneStale: true,
    activeJobId: null
  };
  const el = {};
  let status = "";
  let exportCalled = false;

  await exportPlaylistToUsb(state, el, "p1", {
    setStatus: (text) => { status = text; },
    refreshMissingSourceRoots: async () => ["/music/missing"],
    command: async () => {
      exportCalled = true;
      return {};
    },
    setProgress: () => {},
    startProgressHeartbeat: () => {},
    nextPaint: async () => {},
    stopProgressHeartbeat: () => {},
    countWarningsForStatus: () => 0,
    warningEntryLevel: () => "info",
    logWarnings: () => {},
    pushEventLog: () => {},
    loadPlaylists: async () => {},
    updateModeText: () => {},
    switchView: async () => {},
    renderUsbPlaylists: () => {},
    renderUsbPlaylistTracks: () => {}
  });

  assert.equal(exportCalled, false);
  assert.match(status, /Export blocked: source folder is missing/);
});

test("player menu single-select clears opposite list and enables proper actions", () => {
  const dom = new JSDOM(`<!doctype html><body>
    <button id="refresh"></button>
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
      { menuItemId: 133, kind: 133, name: "BPM", origin: "both" },
      { menuItemId: 134, kind: 134, name: "RATING", origin: "both" },
    ],
    usbPlayerMenuCurrent: [
      { menuItemId: 17, kind: 132, name: "PLAYLIST", origin: "both" },
      { menuItemId: 11, kind: 139, name: "KEY", origin: "both" },
    ],
    usbPlayerMenuAvailableSelectedKind: null,
    usbPlayerMenuCurrentSelectedKind: null,
  };
  const el = {
    refreshUsbPlayerMenuBtn: document.getElementById("refresh"),
    usbPlayerMenuAddBtn: document.getElementById("add"),
    usbPlayerMenuRemoveBtn: document.getElementById("remove"),
    usbPlayerMenuUpBtn: document.getElementById("up"),
    usbPlayerMenuDownBtn: document.getElementById("down"),
    usbPlayerMenuAvailable: document.getElementById("available"),
    usbPlayerMenuCurrent: document.getElementById("current"),
  };

  renderUsbPlayerMenuEditor(state, el, { documentObj: document });

  const availableFirst = el.usbPlayerMenuAvailable.querySelector(".player-menu-item[data-menu-kind='133']");
  handleUsbPlayerMenuListClick(state, el, { documentObj: document }, "available", { target: availableFirst });
  syncUsbPlayerMenuEditorControls(state, el);

  assert.equal(el.usbPlayerMenuAddBtn.disabled, false);
  assert.equal(el.usbPlayerMenuRemoveBtn.disabled, true);
  assert.equal(state.usbPlayerMenuCurrentSelectedKind, null);

  const currentSecond = el.usbPlayerMenuCurrent.querySelector(".player-menu-item[data-menu-kind='139']");
  handleUsbPlayerMenuListClick(state, el, { documentObj: document }, "current", { target: currentSecond });
  syncUsbPlayerMenuEditorControls(state, el);

  assert.equal(el.usbPlayerMenuAddBtn.disabled, true);
  assert.equal(el.usbPlayerMenuRemoveBtn.disabled, false);
  assert.equal(el.usbPlayerMenuUpBtn.disabled, false);
  assert.equal(el.usbPlayerMenuDownBtn.disabled, true);
  assert.equal(state.usbPlayerMenuAvailableSelectedKind, null);

  renderUsbPlayerMenuEditor(state, el, { documentObj: document });
  assert.equal(
    el.usbPlayerMenuCurrent.querySelector(".player-menu-item[data-menu-kind='139']")?.classList.contains("is-selected"),
    true,
  );
});

test("exportPlaylistToUsb passes backupBeforeExport true when exportBackup is true", async () => {
  const state = {
    playlists: [{ id: "p1", name: "Set", tracks: [{ id: "t1" }] }],
    usbRoot: "/USB",
    usbRootValid: true,
    usbWritable: true,
    exportPruneStale: true,
    exportBackup: true,
    activeJobId: null,
    currentPlaylistId: null,
    usbPlaylists: [],
    usbPlaylistTracks: []
  };
  const el = { usbSelectedPlaylistText: { textContent: "" } };
  let capturedOptions = null;

  await assert.rejects(
    exportPlaylistToUsb(state, el, "p1", {
      setStatus: () => {},
      setProgress: () => {},
      startProgressHeartbeat: () => {},
      nextPaint: async () => {},
      command: async (_cmd, args) => {
        capturedOptions = args?.options;
        throw new Error("sentinel");
      },
      stopProgressHeartbeat: () => {},
      countWarningsForStatus: () => 0,
      warningEntryLevel: () => "info",
      logWarnings: () => {},
      pushEventLog: () => {},
      loadPlaylists: async () => {},
      updateModeText: () => {},
      switchView: async () => {},
      renderUsbPlaylists: () => {},
      renderUsbPlaylistTracks: () => {}
    })
  );

  assert.equal(capturedOptions?.backupBeforeExport, true);
});

test("exportPlaylistToUsb passes backupBeforeExport false when exportBackup is false", async () => {
  const state = {
    playlists: [{ id: "p1", name: "Set", tracks: [{ id: "t1" }] }],
    usbRoot: "/USB",
    usbRootValid: true,
    usbWritable: true,
    exportPruneStale: false,
    exportBackup: false,
    activeJobId: null,
    currentPlaylistId: null,
    usbPlaylists: [],
    usbPlaylistTracks: []
  };
  const el = { usbSelectedPlaylistText: { textContent: "" } };
  let capturedOptions = null;

  await assert.rejects(
    exportPlaylistToUsb(state, el, "p1", {
      setStatus: () => {},
      setProgress: () => {},
      startProgressHeartbeat: () => {},
      nextPaint: async () => {},
      command: async (_cmd, args) => {
        capturedOptions = args?.options;
        throw new Error("sentinel");
      },
      stopProgressHeartbeat: () => {},
      countWarningsForStatus: () => 0,
      warningEntryLevel: () => "info",
      logWarnings: () => {},
      pushEventLog: () => {},
      loadPlaylists: async () => {},
      updateModeText: () => {},
      switchView: async () => {},
      renderUsbPlaylists: () => {},
      renderUsbPlaylistTracks: () => {}
    })
  );

  assert.equal(capturedOptions?.backupBeforeExport, false);
});
