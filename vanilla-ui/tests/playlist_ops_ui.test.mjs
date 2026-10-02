import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import {
  commitActivePlaylistSort,
  formatPlaylistExportStatus,
  renderPlaylistPanelChrome,
  renderPlaylistList
} from "../components/playlist/actions.mjs";
import { bindPlaylistEvents } from "../components/playlist/events.mjs";
import { APP_TEMPLATES } from "./test_helpers.mjs";

function makeDom() {
  return new JSDOM(`<!doctype html><body>
    <ul id="navPlaylistList"></ul>
    <button id="addPlaylistBtn"></button>
    <div id="playlistPanelTitle"></div>
    <div id="playlistExportStatus"></div>
    <div id="badgeLabel"></div>
    <input id="playlistSearchInput" value="" />
    <div id="playlistEmptyState"></div>
    <div id="playlistTableWrap"></div>
    <tbody id="playlistTracksBody"></tbody>
    <div id="playlistTotalDuration"></div>
    <button id="exportPlaylistBtn"></button>
    <button id="analyzePlaylistMissingBtn"></button>
    ${APP_TEMPLATES}
  </body>`, { pretendToBeVisual: true });
}

function elements(document, ids) {
  return Object.fromEntries(ids.map((id) => [id, document.getElementById(id)]));
}

function bindDeps(overrides) {
  return {
    emitStatus: () => {},
    switchView: async () => {},
    deletePlaylist: async () => {},
    startPlaylistRename: () => {},
    promptNewPlaylist: () => {},
    command: async () => ({}),
    getCurrentPlaylist: () => null,
    loadPlaylists: async () => {},
    updateModeText: () => {},
    isTrackCurrentlyPlaying: () => false,
    stopPlaybackFromUi: async () => {},
    playTrackFromOrigin: async () => {},
    scrubRatioFromPointer: () => 0,
    exportPlaylistToUsb: async () => {},
    analyzeTrackIds: async () => {},
    refreshCurrentPlaylistTracks: async () => {},
    commitActivePlaylistSort: async () => {},
    isPlaylistSortActive: () => false,
    playlistTracksCtl: {
      view: [], hasMore: false, setSearch: () => {}, rerender: async () => {},
      loadMore: async () => {}, attachScroll: () => {},
    },
    ...overrides
  };
}

test("renderPlaylistList marks active tabs and active playlist mode", () => {
  const { document } = makeDom().window;
  renderPlaylistList({
    state: {
      activeTab: "p2",
      currentPlaylistId: "p1",
      playlists: [{ id: "p1", name: "One" }, { id: "p2", name: "Two" }]
    },
    el: elements(document, ["navPlaylistList"]),
    document
  });

  const buttons = document.querySelectorAll(".nav-playlist-item");
  assert.equal(buttons.length, 2);
  assert.equal(buttons[0].classList.contains("active"), true);
  assert.equal(buttons[1].classList.contains("playlist-active-mode"), true);
});

test("formatPlaylistExportStatus describes the last export", () => {
  assert.equal(formatPlaylistExportStatus({}), "Not exported yet.");
  assert.match(formatPlaylistExportStatus({
    lastExportedAt: "2026-01-01T00:00:00Z",
    lastExportedUsbRoot: "/usb",
    lastExportedTrackCount: 5
  }), /^Last exported .+ to \/usb \(5 track\(s\)\)\.$/);
});

// The active playlist sort lives in tableSortState; committing always clears
// it (recorded as "clear") before deciding whether to persist it.
function commitCtx(state, sort = { key: "artist", dir: "desc" }) {
  const calls = [];
  const tableSortState = sort ? { playlistTracksBody: sort } : {};
  const ctx = {
    state,
    el: {},
    command: async (cmd, payload) => { calls.push([cmd, payload]); return {}; },
    tableSortState: new Proxy(tableSortState, {
      deleteProperty(target, key) {
        calls.push(["clear"]);
        return delete target[key];
      }
    })
  };
  return { calls, ctx };
}

test("commitActivePlaylistSort sends the active sort to the backend to persist as the new order", async () => {
  const state = {
    playlists: [{ id: "p1", name: "A", tracks: [{ id: "t2" }, { id: "t1" }] }],
    playlistUsbExportStatusById: new Map()
  };
  const { calls, ctx } = commitCtx(state);

  await commitActivePlaylistSort(ctx, "p1");

  assert.deepEqual(calls, [
    ["clear"],
    ["reorder_playlist_tracks", { playlistId: "p1", sortBy: "artist", sortDir: "desc" }]
  ]);
});

test("commitActivePlaylistSort no-ops (still clears) when no sort is active", async () => {
  const state = {
    playlists: [{ id: "p1", name: "A", tracks: [{ id: "t1" }] }],
    playlistUsbExportStatusById: new Map()
  };
  const { calls, ctx } = commitCtx(state, null);

  await commitActivePlaylistSort(ctx, "p1");

  assert.deepEqual(calls, [["clear"]]);
});

test("commitActivePlaylistSort no-ops when the playlist is now additive-export-locked", async () => {
  const state = {
    playlists: [{ id: "p1", name: "A", tracks: [{ id: "t1" }] }],
    playlistUsbExportStatusById: new Map([["p1", { locksReorder: true }]])
  };
  const { calls, ctx } = commitCtx(state);

  await commitActivePlaylistSort(ctx, "p1");

  assert.deepEqual(calls, [["clear"]]);
});

test("commitActivePlaylistSort no-ops for a missing/empty playlist id", async () => {
  const state = { playlists: [], playlistUsbExportStatusById: new Map() };
  const { calls, ctx } = commitCtx(state);

  await commitActivePlaylistSort(ctx, null);
  await commitActivePlaylistSort(ctx, "does-not-exist");

  assert.deepEqual(calls, [["clear"], ["clear"]]);
});

test("bindPlaylistEvents ignores playlist selection clicks while new playlist input is open", async () => {
  const dom = makeDom();
  const { document, Event } = dom.window;
  const el = {
    ...elements(document, ["navPlaylistList", "addPlaylistBtn", "playlistSearchInput", "exportPlaylistBtn"]),
    panels: { playlist: document.createElement("div") }
  };
  el.navPlaylistList.innerHTML = `
    <li><button class="nav-playlist-item" data-playlist-id="p1">One</button></li>
    <li class="nav-new-input-wrap"><input class="nav-new-input" /></li>
  `;
  const switched = [];

  bindPlaylistEvents(bindDeps({
    state: { currentPlaylistId: "p0", selectedTrackIds: new Set() },
    el,
    switchView: async (view) => { switched.push(view); }
  }));

  document.querySelector(".nav-playlist-item").dispatchEvent(new Event("click", { bubbles: true }));
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.deepEqual(switched, []);
});

test("empty playlist links to the Media Library and to USB, or to USB Playlists once a USB is selected", async () => {
  const { document } = makeDom().window;
  const views = [];
  const ctx = {
    state: {
      activeTab: "p1",
      currentPlaylistId: "p1",
      playlists: [{ id: "p1", name: "One" }],
      analyzingTrackIds: new Set(),
      selectedTrackIds: new Set(),
      usbRoot: null,
      usbRootValid: false,
    },
    el: elements(document, [
      "navPlaylistList", "playlistPanelTitle", "playlistExportStatus", "playlistSearchInput",
      "playlistEmptyState", "playlistTableWrap", "playlistTracksBody", "playlistTotalDuration",
      "exportPlaylistBtn", "analyzePlaylistMissingBtn",
    ]),
    document,
    playlistTracksCtl: { total: 0, loading: false },
    switchView: async (view) => { views.push(view); },
  };
  const buttons = () => [...document.querySelectorAll("#playlistEmptyState .empty-state-action")];

  renderPlaylistPanelChrome(ctx);
  assert.deepEqual(buttons().map((b) => b.textContent), ["Media Library", "USB"]);
  assert.equal(ctx.el.playlistExportStatus.classList.contains("hidden"), true);
  buttons()[0].click();
  buttons()[1].click();

  ctx.state.usbRoot = "/mnt/usb";
  ctx.state.usbRootValid = true;
  renderPlaylistPanelChrome(ctx);
  assert.deepEqual(buttons().map((b) => b.textContent), ["Media Library", "USB Playlists"]);
  buttons()[1].click();

  assert.deepEqual(views, ["library", "usb", "usb-playlists"]);
});
