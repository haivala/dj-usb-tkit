import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { bindUsbEvents } from "../components/usb/events.mjs";

// Selecting a USB playlist/history used to hydrate every track with its own
// "inspect_usb_track" call, each re-parsing the PDB and re-opening the eDB
// SQLCipher connection. hydrateSelectionTracks now batches tracks into fixed
// -size chunks and calls hydrateUsbTrackMetadataBatch once per chunk, still
// bailing out (no further chunk calls, no row patches) as soon as the user
// selects a different playlist/history mid-hydration.

function makeTracks(prefix, count) {
  return Array.from({ length: count }, (_, i) => ({ id: `${prefix}-${i}`, bpm: "", key: "" }));
}

function makeDom() {
  const dom = new JSDOM(`<!doctype html><body>
    <button id="refreshUsbBtn"></button>
    <button id="initializeUsbBtn"></button>
    <button id="runUsbParityBtn"></button>
    <button id="refreshHistoryBtn"></button>
    <ul id="usbPlaylists">
      <li><button data-usb-playlist-index="0" data-usb-playlist="pl-a">A</button></li>
      <li><button data-usb-playlist-index="1" data-usb-playlist="pl-b">B</button></li>
    </ul>
    <div id="usbPlaylistTracks"></div>
    <ul id="historyList">
      <li><button data-history-index="0">H0</button></li>
    </ul>
    <div id="historyTracks"></div>
  </body>`);
  const document = dom.window.document;
  const el = {
    refreshUsbBtn: document.querySelector("#refreshUsbBtn"),
    initializeUsbBtn: document.querySelector("#initializeUsbBtn"),
    runUsbParityBtn: document.querySelector("#runUsbParityBtn"),
    refreshHistoryBtn: document.querySelector("#refreshHistoryBtn"),
    usbPlaylists: document.querySelector("#usbPlaylists"),
    usbPlaylistTracks: document.querySelector("#usbPlaylistTracks"),
    historyList: document.querySelector("#historyList"),
    historyTracks: document.querySelector("#historyTracks")
  };
  return { dom, document, el };
}

function baseCtx({
  state,
  el,
  hydrateUsbTrackMetadataBatch,
  patchUsbTrackRow = () => true,
  renderUsbPlaylistTracks = () => {},
  patchHistoryTrackRow = () => true,
  renderHistoryTracks = () => {}
}) {
  return {
    state,
    el,
    setStatus: () => {},
    refreshUsb: async () => {},
    pickUsbFolder: async () => {},
    validateAndSetUsbRoot: async () => {},
    initializeUsb: async () => {},
    runUsbParityReport: async () => {},
    runUsbDiagnostics: async () => {},
    previewUsbRepairs: async () => {},
    applyUsbRepairs: async () => {},
    showDiagReportView: () => {},
    refreshHistory: async () => {},
    loadUsbPlayerMenuConfig: async () => {},
    syncUsbPlayerMenuEditorControls: () => {},
    handleUsbPlayerMenuListClick: () => {},
    addUsbPlayerMenuItems: async () => {},
    removeUsbPlayerMenuItems: async () => {},
    moveUsbPlayerMenuItems: async () => {},
    syncUsbPlayerMenusEdbToPdb: async () => {},
    renderUsbPlaylistTracks,
    renderHistoryTracks,
    removeUsbPlaylist: async () => {},
    stopPlaybackIfActive: async () => {},
    hydrateUsbTrackMetadata: async () => {},
    hydrateUsbTrackMetadataBatch,
    setActiveListItem: () => {},
    getHistoryDateDisplay: () => "",
    addTracksToCurrentPlaylist: async () => {},
    patchUsbTrackRow,
    patchHistoryTrackRow
  };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

test("selecting a playlist with few tracks batches them into a single hydrateUsbTrackMetadataBatch call", async () => {
  const { el } = makeDom();
  const tracksA = makeTracks("a", 5);
  const state = { usbPlaylists: [{ id: "pl-a", tracks: tracksA }, { id: "pl-b", tracks: [] }] };

  const calls = [];
  const patched = [];
  const ctx = baseCtx({
    state,
    el,
    hydrateUsbTrackMetadataBatch: async (chunk) => {
      calls.push(chunk.map((t) => t.id));
      for (const t of chunk) t.bpm = "128";
    },
    patchUsbTrackRow: (t) => { patched.push(t.id); return true; }
  });

  bindUsbEvents(ctx);
  el.usbPlaylists.querySelector('[data-usb-playlist-index="0"]').click();
  await flush();

  assert.equal(calls.length, 1, "5 tracks should fit in a single chunk/call");
  assert.deepEqual(calls[0], tracksA.map((t) => t.id));
  assert.deepEqual(patched.sort(), tracksA.map((t) => t.id).sort());
});

test("selecting a playlist with more tracks than the chunk size issues multiple batch calls", async () => {
  const { el } = makeDom();
  const tracksA = makeTracks("a", 45);
  const state = { usbPlaylists: [{ id: "pl-a", tracks: tracksA }, { id: "pl-b", tracks: [] }] };

  const calls = [];
  const ctx = baseCtx({
    state,
    el,
    hydrateUsbTrackMetadataBatch: async (chunk) => {
      calls.push(chunk.length);
      for (const t of chunk) t.bpm = "128";
    }
  });

  bindUsbEvents(ctx);
  el.usbPlaylists.querySelector('[data-usb-playlist-index="0"]').click();
  await flush();

  assert.deepEqual(calls, [40, 5], "45 tracks should split into a 40-track chunk and a 5-track chunk");
});

test("switching playlists mid-hydration stops further batch calls and row patches for the stale selection", async () => {
  const { el } = makeDom();
  const tracksA = makeTracks("a", 45); // two chunks: 40 + 5
  const tracksB = makeTracks("b", 2);
  const state = { usbPlaylists: [{ id: "pl-a", tracks: tracksA }, { id: "pl-b", tracks: tracksB }] };

  const calls = [];
  const patched = [];
  let releaseFirstCall;
  const firstCallGate = new Promise((resolve) => { releaseFirstCall = resolve; });

  const ctx = baseCtx({
    state,
    el,
    hydrateUsbTrackMetadataBatch: async (chunk) => {
      const callIndex = calls.length;
      calls.push(chunk.map((t) => t.id));
      if (callIndex === 0) {
        // Pause the very first (playlist A, chunk 1) call so we can switch
        // to playlist B while it's still in flight.
        await firstCallGate;
      }
      for (const t of chunk) t.bpm = "128";
    },
    patchUsbTrackRow: (t) => { patched.push(t.id); return true; }
  });

  bindUsbEvents(ctx);
  el.usbPlaylists.querySelector('[data-usb-playlist-index="0"]').click();
  // The first chunk call for playlist A has been issued and is now paused.
  assert.equal(calls.length, 1);

  el.usbPlaylists.querySelector('[data-usb-playlist-index="1"]').click();
  await flush();
  // Playlist B's (single, small) chunk should have run to completion already.
  assert.equal(calls.length, 2);
  assert.deepEqual(patched, tracksB.map((t) => t.id));

  releaseFirstCall();
  await flush();
  await flush();

  assert.equal(
    calls.length,
    2,
    "playlist A's second chunk must never be requested once the selection went stale"
  );
  assert.deepEqual(
    patched,
    tracksB.map((t) => t.id),
    "no playlist A row should be patched after the user switched away from it"
  );
});

// History selection reuses the exact same hydrateSelectionTracks function as
// USB playlist selection (see events.mjs), so it gets the same batching and
// stale-selection cancellation for free. These tests pin that down directly
// against the history list rather than relying on it being "the same code".

test("selecting a history entry batches its tracks through hydrateUsbTrackMetadataBatch", async () => {
  const { el } = makeDom();
  const historyTracks = makeTracks("h", 6);
  const state = { histories: [{ id: "hist-0", tracks: historyTracks }] };

  const calls = [];
  const patched = [];
  const ctx = baseCtx({
    state,
    el,
    hydrateUsbTrackMetadataBatch: async (chunk) => {
      calls.push(chunk.map((t) => t.id));
      for (const t of chunk) t.bpm = "128";
    },
    patchHistoryTrackRow: (t) => { patched.push(t.id); return true; }
  });

  bindUsbEvents(ctx);
  el.historyList.querySelector('[data-history-index="0"]').click();
  await flush();

  assert.equal(calls.length, 1, "history tracks should batch into a single call like playlist tracks do");
  assert.deepEqual(calls[0], historyTracks.map((t) => t.id));
  assert.deepEqual(patched.sort(), historyTracks.map((t) => t.id).sort());
});
