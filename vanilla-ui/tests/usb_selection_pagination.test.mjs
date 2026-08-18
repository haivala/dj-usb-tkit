import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { bindUsbEvents } from "../components/usb/events.mjs";
import { renderUsbPlaylistTracks, loadMoreUsbPlaylistTracks } from "../components/usb/actions.mjs";

// Selecting a very large (well above the ~80-track typical case) USB
// playlist used to render and hydrate every track at once, which produced a
// total UI freeze (including the table becoming unresponsive just from
// moving the mouse over it -- a DOM-size problem, not a data-fetching one).
// Selections over LARGE_USB_SELECTION_THRESHOLD now render/hydrate one page
// at a time, loading more as the user scrolls near the bottom of the
// rendered content -- these tests exercise that directly, using the real
// pagination logic in components/usb/actions.mjs (not stubbed), only
// stubbing renderTrackTable itself to avoid needing a full DOM row pipeline.

function makeTracks(prefix, count) {
  return Array.from({ length: count }, (_, i) => ({ id: `${prefix}-${i}` }));
}

function makeDom() {
  const dom = new JSDOM(`<!doctype html><body>
    <button id="refreshUsbBtn"></button>
    <button id="initializeUsbBtn"></button>
    <button id="runUsbParityBtn"></button>
    <button id="refreshHistoryBtn"></button>
    <ul id="usbPlaylists">
      <li><button data-usb-playlist-index="0" data-usb-playlist="pl-a">A</button></li>
    </ul>
    <div class="table-wrap"><div id="usbPlaylistTracks"></div></div>
    <ul id="historyList"></ul>
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
  const wrap = el.usbPlaylistTracks.closest(".table-wrap");
  // jsdom doesn't do real layout -- fake a container tall enough for the
  // first page's rows and not yet scrolled; tests move scrollTop to
  // simulate the user scrolling near the bottom.
  Object.defineProperty(wrap, "clientHeight", { value: 500, configurable: true });
  Object.defineProperty(wrap, "scrollHeight", { value: 5000, configurable: true });
  Object.defineProperty(wrap, "scrollTop", { value: 0, writable: true, configurable: true });
  return { window: dom.window, document, el, wrap };
}

function baseCtx({ state, el, renderTrackTable }) {
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
    renderUsbPlaylistTracks: () => renderUsbPlaylistTracks(state, el, {
      filterTracksByQuery: (t) => t,
      applySortToTracks: (t) => t,
      renderTrackTable
    }),
    loadMoreUsbPlaylistTracks: (pageSize) => loadMoreUsbPlaylistTracks(state, el, pageSize, { renderTrackTable }),
    renderHistoryTracks: () => {},
    loadMoreHistoryTracks: async () => [],
    removeUsbPlaylist: async () => {},
    stopPlaybackIfActive: async () => {},
    hydrateUsbTrackMetadata: async () => {},
    hydrateUsbTrackMetadataBatch: async () => {},
    setActiveListItem: () => {},
    getHistoryDateDisplay: () => "",
    addTracksToCurrentPlaylist: async () => {},
    patchUsbTrackRow: () => true,
    patchHistoryTrackRow: () => true,
    applyUsbDurationSummary: () => {},
    formatDurationMs: () => ""
  };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

test("selecting a large playlist renders only the first page", async () => {
  const { el } = makeDom();
  const tracks = makeTracks("a", 320); // over LARGE_USB_SELECTION_THRESHOLD (300)
  const state = { usbPlaylists: [{ id: "pl-a", tracks, trackCount: 320, totalDurationMs: 0, durationKnownCount: 0 }] };

  const renderCalls = [];
  const renderTrackTable = async (_tbody, pageTracks, options) => {
    renderCalls.push({ count: pageTracks.length, append: !!options.append, indexOffset: options.indexOffset || 0 });
  };

  bindUsbEvents(baseCtx({ state, el, renderTrackTable }));
  el.usbPlaylists.querySelector('[data-usb-playlist-index="0"]').click();
  await flush();

  assert.equal(renderCalls.length, 1);
  assert.equal(renderCalls[0].count, 150, "only the first page (150 tracks) should render initially");
  assert.equal(renderCalls[0].append, false);
  assert.equal(state.usbPlaylistPagedCount, 150);
});

test("scrolling near the bottom of a paginated playlist loads and appends the next page", async () => {
  const { window, el, wrap } = makeDom();
  const tracks = makeTracks("a", 320);
  const state = { usbPlaylists: [{ id: "pl-a", tracks, trackCount: 320, totalDurationMs: 0, durationKnownCount: 0 }] };

  const renderCalls = [];
  const renderTrackTable = async (_tbody, pageTracks, options) => {
    renderCalls.push({ count: pageTracks.length, append: !!options.append, indexOffset: options.indexOffset || 0 });
  };

  bindUsbEvents(baseCtx({ state, el, renderTrackTable }));
  el.usbPlaylists.querySelector('[data-usb-playlist-index="0"]').click();
  await flush();
  assert.equal(renderCalls.length, 1);

  Object.defineProperty(wrap, "scrollTop", { value: 4600, configurable: true });
  wrap.dispatchEvent(new window.Event("scroll"));
  await flush();

  assert.equal(renderCalls.length, 2);
  assert.equal(renderCalls[1].count, 150, "second page is also 150 tracks (320 - 150 - 150 = 20 left for a third)");
  assert.equal(renderCalls[1].append, true);
  assert.equal(renderCalls[1].indexOffset, 150, "second page continues the index sequence after the first page's 150 rows");
  assert.equal(state.usbPlaylistPagedCount, 300);

  // A third scroll near the bottom should load the final, partial page.
  wrap.dispatchEvent(new window.Event("scroll"));
  await flush();
  assert.equal(renderCalls.length, 3);
  assert.equal(renderCalls[2].count, 20);
  assert.equal(state.usbPlaylistPagedCount, 320);

  // No more pages left -- another scroll event should be a no-op.
  wrap.dispatchEvent(new window.Event("scroll"));
  await flush();
  assert.equal(renderCalls.length, 3, "no further page load once everything is rendered");
});

test("a normal (~80-track) playlist selection is not paginated", async () => {
  const { el } = makeDom();
  const tracks = makeTracks("a", 80);
  const state = { usbPlaylists: [{ id: "pl-a", tracks, trackCount: 80, totalDurationMs: 0, durationKnownCount: 0 }] };

  const renderCalls = [];
  const renderTrackTable = async (_tbody, pageTracks, options) => {
    renderCalls.push({ count: pageTracks.length, append: !!options.append });
  };

  bindUsbEvents(baseCtx({ state, el, renderTrackTable }));
  el.usbPlaylists.querySelector('[data-usb-playlist-index="0"]').click();
  await flush();

  assert.equal(renderCalls.length, 1);
  assert.equal(renderCalls[0].count, 80, "all 80 tracks render in one pass, same as before pagination existed");
  assert.equal(state.usbPlaylistPagedCount, 0, "pagination should not engage for a selection at/under the threshold");
});
