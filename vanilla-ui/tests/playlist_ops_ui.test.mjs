import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import {
  renderPlaylistList,
  promptNewPlaylist,
  startPlaylistRename,
  formatPlaylistExportStatus,
  loadPlaylists,
  refreshCurrentPlaylistTracks,
  updatePlaylistExportButtons,
  createPlaylist
} from "../components/playlist/actions.mjs";
import { bindPlaylistEvents } from "../components/playlist/events.mjs";

function makeDom() {
  const dom = new JSDOM(`
    <!doctype html>
    <body>
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
    </body>
  `, { pretendToBeVisual: true });
  return dom;
}

test("renderPlaylistList renders active and active-mode playlist buttons", () => {
  const dom = makeDom();
  const { document } = dom.window;
  const state = {
    activeTab: "p2",
    currentPlaylistId: "p1",
    playlists: [
      { id: "p1", name: "One" },
      { id: "p2", name: "Two" }
    ]
  };
  const el = { navPlaylistList: document.getElementById("navPlaylistList") };

  renderPlaylistList(state, el, {
    document,
    renderPlaylistSidebarItemContent: (playlist) => playlist.name
  });

  const buttons = document.querySelectorAll(".nav-playlist-item");
  assert.equal(buttons.length, 2);
  // playlists are rendered newest-first (reversed), so p2 is first, p1 is second
  assert.equal(buttons[0].classList.contains("active"), true);
  assert.equal(buttons[1].classList.contains("playlist-active-mode"), true);
});

test("promptNewPlaylist guards double submit and restores button", async () => {
  const dom = makeDom();
  const { document, Event, KeyboardEvent } = dom.window;
  const el = {
    navPlaylistList: document.getElementById("navPlaylistList"),
    addPlaylistBtn: document.getElementById("addPlaylistBtn")
  };
  let created = 0;
  const statuses = [];

  promptNewPlaylist(el, {
    document,
    requestAnimationFrame: (cb) => cb(),
    createPlaylist: async () => { created += 1; },
    setStatus: (text) => statuses.push(text)
  });

  const input = document.querySelector(".nav-new-input");
  input.value = "My Playlist";
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  input.dispatchEvent(new Event("blur", { bubbles: true }));
  await new Promise((resolve) => setTimeout(resolve, 120));

  assert.equal(created, 1);
  assert.equal(el.addPlaylistBtn.classList.contains("hidden"), false);
  assert.equal(statuses.length, 0);
});

test("startPlaylistRename saves new name and updates panel + badge", async () => {
  const dom = makeDom();
  const { document, KeyboardEvent } = dom.window;
  const state = {
    activeTab: "p1",
    playlists: [{ id: "p1", name: "Old Name", lastExportedAt: "2026-01-01T00:00:00Z" }]
  };
  const navPlaylistList = document.getElementById("navPlaylistList");
  navPlaylistList.innerHTML = '<li><button class="nav-playlist-item" data-playlist-id="p1">Old Name</button></li>';
  const el = {
    navPlaylistList,
    playlistPanelTitle: document.getElementById("playlistPanelTitle"),
    playlistExportStatus: document.getElementById("playlistExportStatus"),
    badgeLabel: document.getElementById("badgeLabel")
  };

  startPlaylistRename("p1", state, el, {
    document,
    requestAnimationFrame: (cb) => cb(),
    command: async () => ({ name: "New Name" }),
    setStatus: () => {},
    renderPlaylistSidebarItemContent: (playlist) => playlist.name,
    getCurrentPlaylist: () => ({ id: "p1", name: "New Name" }),
    formatPlaylistExportStatus: () => "Not exported yet."
  });

  const input = document.querySelector(".nav-rename-input");
  input.value = "New Name";
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(state.playlists[0].name, "New Name");
  assert.equal(el.playlistPanelTitle.textContent, "New Name");
  assert.equal(el.playlistExportStatus.textContent, "Not exported yet.");
  assert.equal(el.badgeLabel.textContent, "New Name");
});

test("formatPlaylistExportStatus formats exported playlists", () => {
  const text = formatPlaylistExportStatus(
    {
      lastExportedAt: "2026-01-01T00:00:00Z",
      lastExportedUsbRoot: "/usb",
      lastExportedTrackCount: 5
    },
    { formatTimestampLocal: () => "Jan 1" }
  );

  assert.equal(text, "Last exported Jan 1 to /usb (5 track(s)).");
});

test("loadPlaylists normalizes items and triggers playlist refresh hooks", async () => {
  const state = { playlists: [] };
  const calls = [];

  await loadPlaylists(state, {
    command: async () => ({ items: [{ id: "p1", name: "One" }] }),
    renderPlaylistTabsAndPanels: () => calls.push("render"),
    updatePlaylistExportButtons: () => calls.push("buttons")
  });

  assert.deepEqual(state.playlists, [{ id: "p1", name: "One", tracks: [] }]);
  assert.deepEqual(calls, ["render", "buttons"]);
});

test("refreshCurrentPlaylistTracks updates playlist table state and empty state", async () => {
  const dom = makeDom();
  const { document } = dom.window;
  const playlist = { id: "p1", name: "One", tracks: [] };
  const state = {
    playlistTrackSearch: "",
    currentPlaylistTracksView: []
  };
  const el = {
    playlistSearchInput: document.getElementById("playlistSearchInput"),
    playlistEmptyState: document.getElementById("playlistEmptyState"),
    playlistTableWrap: document.getElementById("playlistTableWrap"),
    playlistTracksBody: document.getElementById("playlistTracksBody"),
    playlistTotalDuration: document.getElementById("playlistTotalDuration")
  };
  const calls = [];

  await refreshCurrentPlaylistTracks(state, el, {
    getCurrentPlaylist: () => playlist,
    command: async () => ({ items: [{ id: "t1", title: "Track" }] }),
    normalizeTrack: (track) => track,
    filterTracksByQuery: (tracks) => tracks,
    renderEmptyState: (_container, payload) => calls.push(payload.heading),
    applySortToTracks: (tracks) => tracks,
    renderTrackTable: (_tbody, tracks) => calls.push(`rows:${tracks.length}`),
    updateTrackListDurationSummary: (_node, tracks) => calls.push(`duration:${tracks.length}`),
    updatePlaylistPanelTitle: (item) => calls.push(`title:${item.id}`),
    updatePlaylistExportButtons: () => calls.push("buttons"),
    renderPlaylistList: () => calls.push("list")
  });

  assert.equal(playlist.tracks.length, 1);
  assert.equal(state.currentPlaylistTracksView.length, 1);
  assert.equal(el.playlistTableWrap.classList.contains("hidden"), false);
  assert.deepEqual(calls, ["rows:1", "duration:1", "title:p1", "buttons", "list"]);
});

test("updatePlaylistExportButtons hides export while analyze-missing is available", () => {
  const dom = makeDom();
  const { document } = dom.window;
  const state = {
    usbRoot: "/usb",
    usbRootValid: true,
    exportPruneStale: true,
    usbKnownPlaylistNames: new Set()
  };
  const el = {
    exportPlaylistBtn: document.getElementById("exportPlaylistBtn"),
    analyzePlaylistMissingBtn: document.getElementById("analyzePlaylistMissingBtn")
  };

  updatePlaylistExportButtons(state, el, {
    getCurrentPlaylist: () => ({
      name: "Set",
      tracks: [{ id: "t1", usbAnalysisPath: null, analyzed: false }]
    }),
    computeExportButtonState: () => ({ enabled: true, text: "Export", title: "Go" }),
    isUsbOriginTrack: () => false,
    trackHasCoreAnalysis: () => false
  });

  assert.equal(el.exportPlaylistBtn.hidden, true);
  assert.equal(el.analyzePlaylistMissingBtn.hidden, false);
  assert.match(el.analyzePlaylistMissingBtn.textContent, /1/);
});

test("createPlaylist validates name and switches to the new playlist", async () => {
  const state = { currentPlaylistId: null };
  const calls = [];

  await createPlaylist("Fresh", {
    setStatus: (text) => calls.push(`status:${text}`),
    withProgress: async (_label, fn) => fn(() => {}),
    command: async () => ({ playlistId: "p9", name: "Fresh" }),
    loadPlaylists: async () => calls.push("load"),
    state,
    updateModeText: () => calls.push("mode"),
    switchTab: async (tab) => calls.push(`tab:${tab}`)
  });

  assert.equal(state.currentPlaylistId, "p9");
  assert.deepEqual(calls, ["load", "mode", "tab:p9", "status:Playlist created: Fresh"]);
});

test("createPlaylist selects newly loaded playlist when command id is not present", async () => {
  const state = { currentPlaylistId: "p1", playlists: [{ id: "p1", name: "Old" }] };
  const calls = [];

  await createPlaylist("Fresh", {
    setStatus: (text) => calls.push(`status:${text}`),
    withProgress: async (_label, fn) => fn(() => {}),
    command: async () => ({ playlistId: "missing-id", name: "Fresh" }),
    loadPlaylists: async () => {
      state.playlists = [{ id: "p1", name: "Old" }, { id: "p2", name: "Fresh" }];
      calls.push("load");
    },
    state,
    updateModeText: () => calls.push("mode"),
    switchTab: async (tab) => calls.push(`tab:${tab}`)
  });

  assert.equal(state.currentPlaylistId, "p2");
  assert.deepEqual(calls, ["load", "mode", "tab:p2", "status:Playlist created: Fresh"]);
});

test("bindPlaylistEvents ignores playlist selection clicks while new playlist input is open", async () => {
  const dom = makeDom();
  const { document, Event } = dom.window;
  const el = {
    navPlaylistList: document.getElementById("navPlaylistList"),
    addPlaylistBtn: document.getElementById("addPlaylistBtn"),
    panels: { playlist: document.createElement("div") },
    playlistSearchInput: document.getElementById("playlistSearchInput"),
    exportPlaylistBtn: document.getElementById("exportPlaylistBtn")
  };
  el.navPlaylistList.innerHTML = `
    <li><button class="nav-playlist-item" data-playlist-id="p1">One</button></li>
    <li class="nav-new-input-wrap"><input class="nav-new-input" /></li>
  `;

  const switched = [];
  bindPlaylistEvents({
    state: { currentPlaylistId: "p0", currentPlaylistTracksView: [], selectedTrackIds: new Set() },
    el,
    setStatus: () => {},
    switchView: async (view) => { switched.push(view); },
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
    isUsbOriginTrack: () => false,
    trackHasCoreAnalysis: () => false,
    analyzeTrackIds: async () => {},
    resolveLocalTrackId: () => null,
    refreshCurrentPlaylistTracks: async () => {}
  });

  document.querySelector(".nav-playlist-item").dispatchEvent(new Event("click", { bubbles: true }));
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.deepEqual(switched, []);
});
