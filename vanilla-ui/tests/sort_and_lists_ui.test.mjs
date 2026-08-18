import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  applySortToTracks
} from "../components/shell/actions.mjs";
import {
  renderUsbPlaylists,
  renderUsbPlaylistTracks,
  renderHistoryList,
  renderHistoryTracks
} from "../components/usb/actions.mjs";

test("applySortToTracks applies configured state sorter", () => {
  const out = applySortToTracks({ body: { key: "title", dir: "asc" } }, [{ title: "b" }, { title: "a" }], "body", {
    sortTracks: (tracks) => [...tracks].sort((a, b) => a.title.localeCompare(b.title))
  });
  assert.deepEqual(out.map((t) => t.title), ["a", "b"]);
});

test("renderUsbPlaylists renders empty and populated states", () => {
  const dom = new JSDOM(`<!doctype html><body><div class="split"><ul id="usb"></ul><div class="right"></div></div></body>`);
  const el = { usbPlaylists: dom.window.document.querySelector("#usb") };
  const state = { usbPlaylists: [] };
  renderUsbPlaylists(state, el, { escapeHtml: (v) => String(v) });
  assert.equal(el.usbPlaylists.textContent.includes("No playlists imported yet"), true);

  state.usbPlaylists = [{ id: "1", name: "Set", source: "pdb", trackCount: 3 }];
  renderUsbPlaylists(state, el, { escapeHtml: (v) => String(v) });
  assert.equal(el.usbPlaylists.querySelectorAll("button").length, 1);
});

test("renderUsbPlaylistTracks computes view and renders table", () => {
  const state = { usbPlaylistTracks: [{ id: 1 }, { id: 2 }], usbTrackSearch: "", usbPlaylistTracksView: [] };
  const el = { usbPlaylistTracks: {}, usbPlaylistTotalDuration: {} };
  let renderedCount = 0;
  renderUsbPlaylistTracks(state, el, {
    filterTracksByQuery: (tracks) => tracks,
    applySortToTracks: (tracks) => tracks,
    renderTrackTable: (_tbody, tracks) => { renderedCount = tracks.length; },
    updateTrackListDurationSummary: () => {}
  });
  assert.equal(state.usbPlaylistTracksView.length, 2);
  assert.equal(renderedCount, 2);
});

test("renderHistoryList renders list entries with optional dates", () => {
  const dom = new JSDOM(`<!doctype html><body><div class="split"><ul id="hist"></ul><div class="right"></div></div></body>`);
  const el = { historyList: dom.window.document.querySelector("#hist") };
  const state = { histories: [{ name: "Session", date: "2026-04-07" }] };
  renderHistoryList(state, el, {
    escapeHtml: (v) => String(v),
    getHistoryDateValue: () => "2026-04-07"
  });
  assert.equal(el.historyList.querySelectorAll("button").length, 1);
});

test("renderHistoryTracks computes view and renders table", () => {
  const state = { historyTracks: [{ id: "a" }], historyTrackSearch: "", historyTracksView: [] };
  const el = { historyTracks: {}, historyTotalDuration: {} };
  let renderedCount = 0;
  renderHistoryTracks(state, el, {
    filterTracksByQuery: (tracks) => tracks,
    applySortToTracks: (tracks) => tracks,
    renderTrackTable: (_tbody, tracks) => { renderedCount = tracks.length; },
    updateTrackListDurationSummary: () => {}
  });
  assert.equal(state.historyTracksView.length, 1);
  assert.equal(renderedCount, 1);
});
