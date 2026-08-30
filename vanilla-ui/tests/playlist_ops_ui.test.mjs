import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import {
  commitActivePlaylistSort,
  createPlaylist,
  formatPlaylistExportStatus,
  loadPlaylists,
  renderPlaylistList
} from "../components/playlist/actions.mjs";
import { bindPlaylistEvents } from "../components/playlist/events.mjs";

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
  </body>`, { pretendToBeVisual: true });
}

function elements(document, ids) {
  return Object.fromEntries(ids.map((id) => [id, document.getElementById(id)]));
}

function bindDeps(overrides) {
  return {
    setStatus: () => {},
    switchView: async () => {},
    deletePlaylist: async () => {},
    startPlaylistRename: () => {},
    promptNewPlaylist: () => {},
    command: async () => ({}),
    getCurrentPlaylist: () => null,
    loadPlaylists: async () => {},
    updateModeText: () => {},
    getPlaybackUiStateHelpers: () => null,
    isTrackCurrentlyPlaying: () => false,
    stopPlaybackFromUi: async () => {},
    playTrackFromOrigin: async () => {},
    scrubRatioFromPointer: () => 0,
    exportPlaylistToUsb: async () => {},
    analyzeTrackIds: async () => {},
    resolveLocalTrackId: () => null,
    refreshCurrentPlaylistTracks: async () => {},
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
    activeTab: "p2",
    currentPlaylistId: "p1",
    playlists: [{ id: "p1", name: "One" }, { id: "p2", name: "Two" }]
  }, elements(document, ["navPlaylistList"]), {
    document,
    renderPlaylistSidebarItemContent: (playlist) => playlist.name
  });

  const buttons = document.querySelectorAll(".nav-playlist-item");
  assert.equal(buttons.length, 2);
  assert.equal(buttons[0].classList.contains("active"), true);
  assert.equal(buttons[1].classList.contains("playlist-active-mode"), true);
});

test("playlist commands format export status, load lists, and select newly loaded playlists", async () => {
  assert.equal(formatPlaylistExportStatus({
    lastExportedAt: "2026-01-01T00:00:00Z",
    lastExportedUsbRoot: "/usb",
    lastExportedTrackCount: 5
  }, { formatTimestampLocal: () => "Jan 1" }), "Last exported Jan 1 to /usb (5 track(s)).");

  const loaded = { playlists: [] };
  const loadCalls = [];
  await loadPlaylists(loaded, {
    command: async () => ({ items: [{ id: "p1", name: "One" }] }),
    renderPlaylistTabsAndPanels: () => loadCalls.push("render"),
    updatePlaylistExportButtons: () => loadCalls.push("buttons")
  });
  assert.deepEqual(loaded.playlists, [{ id: "p1", name: "One", tracks: [] }]);
  assert.deepEqual(loadCalls, ["render", "buttons"]);

  const state = { currentPlaylistId: "p1", playlists: [{ id: "p1", name: "Old" }] };
  const createCalls = [];
  await createPlaylist("Fresh", {
    setStatus: (text) => createCalls.push(`status:${text}`),
    withProgress: async (_label, fn) => fn(() => {}),
    command: async () => ({ playlistId: "missing-id", name: "Fresh" }),
    loadPlaylists: async () => {
      state.playlists = [{ id: "p1", name: "Old" }, { id: "p2", name: "Fresh" }];
      createCalls.push("load");
    },
    state,
    updateModeText: () => createCalls.push("mode"),
    switchTab: async (tab) => createCalls.push(`tab:${tab}`)
  });
  assert.equal(state.currentPlaylistId, "p2");
  assert.deepEqual(createCalls, ["load", "mode", "tab:p2", "status:Playlist created: Fresh"]);
});

function commitDeps(overrides = {}) {
  const calls = [];
  return {
    calls,
    deps: {
      command: async (cmd, payload) => { calls.push([cmd, payload]); return {}; },
      getActiveSort: () => ({ key: "artist", dir: "desc" }),
      clearPlaylistTrackSort: () => { calls.push(["clear"]); },
      ...overrides
    }
  };
}

test("commitActivePlaylistSort sends the active sort to the backend to persist as the new order", async () => {
  const state = {
    playlists: [{ id: "p1", name: "A", tracks: [{ id: "t2" }, { id: "t1" }] }],
    playlistUsbExportStatusById: new Map()
  };
  const { calls, deps } = commitDeps();

  await commitActivePlaylistSort(state, "p1", deps);

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
  const { calls, deps } = commitDeps({ getActiveSort: () => null });

  await commitActivePlaylistSort(state, "p1", deps);

  assert.deepEqual(calls, [["clear"]]);
});

test("commitActivePlaylistSort no-ops when the playlist is now additive-export-locked", async () => {
  const state = {
    playlists: [{ id: "p1", name: "A", tracks: [{ id: "t1" }] }],
    playlistUsbExportStatusById: new Map([["p1", { locksReorder: true }]])
  };
  const { calls, deps } = commitDeps();

  await commitActivePlaylistSort(state, "p1", deps);

  assert.deepEqual(calls, [["clear"]]);
});

test("commitActivePlaylistSort no-ops for a missing/empty playlist id", async () => {
  const state = { playlists: [], playlistUsbExportStatusById: new Map() };
  const { calls, deps } = commitDeps();

  await commitActivePlaylistSort(state, null, deps);
  await commitActivePlaylistSort(state, "does-not-exist", deps);

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
