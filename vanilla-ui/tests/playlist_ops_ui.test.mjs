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
import { APP_TEMPLATES, makeTestCtx } from "./test_helpers.mjs";

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

test("playlist commands format export status, load lists, and select newly loaded playlists", async () => {
  assert.match(formatPlaylistExportStatus({
    lastExportedAt: "2026-01-01T00:00:00Z",
    lastExportedUsbRoot: "/usb",
    lastExportedTrackCount: 5
  }), /^Last exported .+ to \/usb \(5 track\(s\)\)\.$/);

  const loaded = makeTestCtx({ command: async () => ({ items: [{ id: "p1", name: "One" }] }) });
  await loadPlaylists(loaded);
  assert.deepEqual(loaded.state.playlists, [{ id: "p1", name: "One", tracks: [] }]);
  assert.equal(loaded.el.navPlaylistList.querySelector(".nav-playlist-item").dataset.playlistId, "p1");

  const createCalls = [];
  const ctx = makeTestCtx({
    emitStatus: (text) => createCalls.push(`status:${text}`),
    withProgress: async (_label, fn) => fn(() => {}),
    command: async (name) => {
      if (name === "create_playlist") return { playlistId: "missing-id", name: "Fresh" };
      if (name === "list_playlists") return { items: [{ id: "p1", name: "Old" }, { id: "p2", name: "Fresh" }] };
      return {};
    },
    updateModeText: () => createCalls.push("mode"),
    switchView: async (tab) => createCalls.push(`tab:${tab}`)
  });
  ctx.state.currentPlaylistId = "p1";
  await createPlaylist(ctx, "Fresh");
  assert.equal(ctx.state.currentPlaylistId, "p2");
  assert.deepEqual(createCalls, ["mode", "tab:p2", "status:Playlist created: Fresh"]);
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

test("Analyze Missing Tracks delegates the whole playlist to the backend (no page force-load, no client id resolution)", async () => {
  const dom = makeDom();
  const { document, Event } = dom.window;
  const el = {
    ...elements(document, ["navPlaylistList", "addPlaylistBtn", "playlistSearchInput", "exportPlaylistBtn", "analyzePlaylistMissingBtn"]),
    panels: { playlist: document.createElement("div") }
  };
  const analyzeCalls = [];
  let loadMoreCalls = 0;

  bindPlaylistEvents(bindDeps({
    state: { currentPlaylistId: "pl-9", selectedTrackIds: new Set() },
    el,
    getCurrentPlaylist: () => ({ id: "pl-9", name: "Big", tracks: [{ id: "t1" }] }),
    analyzeTrackIds: async (ids, label, options) => { analyzeCalls.push({ ids, label, options }); },
    playlistTracksCtl: {
      view: [], hasMore: true, setSearch: () => {}, rerender: async () => {},
      loadMore: async () => { loadMoreCalls += 1; }, attachScroll: () => {},
    },
  }));

  el.analyzePlaylistMissingBtn.dispatchEvent(new Event("click", { bubbles: true }));
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(loadMoreCalls, 0, "must not force-load playlist pages");
  assert.equal(analyzeCalls.length, 1);
  assert.deepEqual(analyzeCalls[0].ids, []);
  assert.deepEqual(analyzeCalls[0].options, { playlistId: "pl-9" });
});
