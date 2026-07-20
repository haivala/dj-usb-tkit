import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { bindUsbEvents } from "../components/usb/events.mjs";

function makeCtx(usbTrack, historyTrack) {
  const dom = new JSDOM(`<!doctype html><body>
    <button id="refreshUsbBtn"></button>
    <button id="initializeUsbBtn"></button>
    <button id="runUsbParityBtn"></button>
    <button id="refreshHistoryBtn"></button>
    <ul id="usbPlaylists"></ul>
    <div id="usbPlaylistTracks">
      <div class="track-grid-row" data-track-index="0" data-track-id="${usbTrack.id}" data-track-origin="usb" data-playback-row="usb:${usbTrack.id}"></div>
    </div>
    <ul id="historyList"></ul>
    <div id="historyTracks">
      <div class="track-grid-row" data-track-index="0" data-track-id="${historyTrack.id}" data-track-origin="usb" data-playback-row="usb:${historyTrack.id}"></div>
    </div>
  </body>`);
  const document = dom.window.document;
  const state = {
    usbPlaylists: [],
    usbPlaylistTracks: [usbTrack],
    usbPlaylistTracksView: [usbTrack],
    histories: [],
    historyTracks: [historyTrack],
    historyTracksView: [historyTrack]
  };
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
  return { dom, document, state, el };
}

function baseCtx({
  state,
  el,
  hydrateUsbTrackMetadata,
  patchUsbTrackRow = () => false,
  renderUsbPlaylistTracks = () => {},
  patchHistoryTrackRow = () => false,
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
    hydrateUsbTrackMetadata,
    setActiveListItem: () => {},
    getHistoryDateDisplay: () => "",
    addTracksToCurrentPlaylist: async () => {},
    patchUsbTrackRow,
    patchHistoryTrackRow
  };
}

test("clicking a USB playlist track row patches that row in place instead of re-rendering the whole table", async () => {
  const track = { id: "t-1", bpm: "", key: "" };
  const { state, el } = makeCtx(track, { id: "h-unused", bpm: "", key: "" });

  let renderCalls = 0;
  let patchCalls = 0;
  const ctx = baseCtx({
    state,
    el,
    hydrateUsbTrackMetadata: async (t) => {
      // Simulate hydration filling in BPM (changes the meta fingerprint).
      t.bpm = "128";
    },
    patchUsbTrackRow: (t) => {
      patchCalls += 1;
      assert.equal(t.id, track.id);
      return true;
    },
    renderUsbPlaylistTracks: () => { renderCalls += 1; }
  });

  bindUsbEvents(ctx);
  el.usbPlaylistTracks.querySelector(".track-grid-row").click();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(patchCalls, 1, "patchUsbTrackRow should be called once");
  assert.equal(renderCalls, 0, "renderUsbPlaylistTracks must not run when the patch succeeds");
});

test("falls back to a full re-render only when the row patch fails", async () => {
  const track = { id: "t-2", bpm: "", key: "" };
  const { state, el } = makeCtx(track, { id: "h-unused", bpm: "", key: "" });

  let renderCalls = 0;
  const ctx = baseCtx({
    state,
    el,
    hydrateUsbTrackMetadata: async (t) => { t.bpm = "128"; },
    patchUsbTrackRow: () => false,
    renderUsbPlaylistTracks: () => { renderCalls += 1; }
  });

  bindUsbEvents(ctx);
  el.usbPlaylistTracks.querySelector(".track-grid-row").click();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(renderCalls, 1, "renderUsbPlaylistTracks is the required fallback when patching fails");
});

test("does not patch or re-render when hydration changes nothing", async () => {
  const track = { id: "t-3", bpm: "128", key: "Am" };
  const { state, el } = makeCtx(track, { id: "h-unused", bpm: "", key: "" });

  let renderCalls = 0;
  let patchCalls = 0;
  const ctx = baseCtx({
    state,
    el,
    hydrateUsbTrackMetadata: async () => {
      // Track already fully hydrated: nothing changes.
    },
    patchUsbTrackRow: () => { patchCalls += 1; return true; },
    renderUsbPlaylistTracks: () => { renderCalls += 1; }
  });

  bindUsbEvents(ctx);
  el.usbPlaylistTracks.querySelector(".track-grid-row").click();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(patchCalls, 0, "no need to patch a row whose data didn't change");
  assert.equal(renderCalls, 0, "no need to re-render a row whose data didn't change");
});

// History playlists are USB playlists too (same browsing/hydration path) and
// share the identical patch-or-fallback fix.

test("clicking a history track row patches that row in place instead of re-rendering the whole list", async () => {
  const historyTrack = { id: "h-1", bpm: "", key: "" };
  const { state, el } = makeCtx({ id: "t-unused", bpm: "", key: "" }, historyTrack);

  let renderCalls = 0;
  let patchCalls = 0;
  const ctx = baseCtx({
    state,
    el,
    hydrateUsbTrackMetadata: async (t) => { t.bpm = "128"; },
    patchHistoryTrackRow: (t) => {
      patchCalls += 1;
      assert.equal(t.id, historyTrack.id);
      return true;
    },
    renderHistoryTracks: () => { renderCalls += 1; }
  });

  bindUsbEvents(ctx);
  el.historyTracks.querySelector(".track-grid-row").click();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(patchCalls, 1, "patchHistoryTrackRow should be called once");
  assert.equal(renderCalls, 0, "renderHistoryTracks must not run when the patch succeeds");
});

test("history row click falls back to a full re-render only when the row patch fails", async () => {
  const historyTrack = { id: "h-2", bpm: "", key: "" };
  const { state, el } = makeCtx({ id: "t-unused", bpm: "", key: "" }, historyTrack);

  let renderCalls = 0;
  const ctx = baseCtx({
    state,
    el,
    hydrateUsbTrackMetadata: async (t) => { t.bpm = "128"; },
    patchHistoryTrackRow: () => false,
    renderHistoryTracks: () => { renderCalls += 1; }
  });

  bindUsbEvents(ctx);
  el.historyTracks.querySelector(".track-grid-row").click();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(renderCalls, 1, "renderHistoryTracks is the required fallback when patching fails");
});

test("history row click does not patch or re-render when hydration changes nothing", async () => {
  const historyTrack = { id: "h-3", bpm: "128", key: "Am" };
  const { state, el } = makeCtx({ id: "t-unused", bpm: "", key: "" }, historyTrack);

  let renderCalls = 0;
  let patchCalls = 0;
  const ctx = baseCtx({
    state,
    el,
    hydrateUsbTrackMetadata: async () => {},
    patchHistoryTrackRow: () => { patchCalls += 1; return true; },
    renderHistoryTracks: () => { renderCalls += 1; }
  });

  bindUsbEvents(ctx);
  el.historyTracks.querySelector(".track-grid-row").click();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(patchCalls, 0, "no need to patch a row whose data didn't change");
  assert.equal(renderCalls, 0, "no need to re-render a row whose data didn't change");
});
