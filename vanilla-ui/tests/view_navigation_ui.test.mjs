import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { switchView, switchTab } from "../startup_bootstrap.mjs";

function makeFixture() {
  const dom = new JSDOM(`<!doctype html><body>
    <div id="navSidebar">
      <button class="nav-item" data-view="library"></button>
      <button class="nav-item" data-view="event-log"></button>
    </div>
    <div id="navPlaylistList">
      <button class="nav-playlist-item" data-playlist-id="pl-1"></button>
    </div>
    <section id="panel-library"></section>
    <section id="panel-playlist"></section>
    <section id="panel-event-log"></section>
  </body>`);
  const doc = dom.window.document;
  const el = {
    navSidebar: doc.querySelector("#navSidebar"),
    navPlaylistList: doc.querySelector("#navPlaylistList"),
    panels: {
      library: doc.querySelector("#panel-library"),
      playlist: doc.querySelector("#panel-playlist"),
      "event-log": doc.querySelector("#panel-event-log")
    }
  };
  return { dom, doc, el };
}

test("switchView navigates to static view and renders event log", async () => {
  const { doc, el } = makeFixture();
  const state = { activeTab: "library", playlists: [], currentPlaylistId: null };
  let renderedEventLog = 0;
  let waveformCalls = 0;
  await switchView(state, el, "event-log", {
    staticTabs: ["library", "event-log"],
    stopPlaybackIfActive: async () => {},
    syncLibraryOnboardingMode: () => {},
    updateModeText: () => {},
    populatePlaylistPanel: () => {},
    refreshCurrentPlaylistTracks: async () => {},
    renderEventLog: () => { renderedEventLog += 1; },
    requestAnimationFrameFn: (cb) => cb(),
    documentObj: doc,
    renderWaveformsIn: () => { waveformCalls += 1; }
  });
  assert.equal(state.activeTab, "event-log");
  assert.equal(renderedEventLog, 1);
  assert.equal(waveformCalls, 1);
});

test("switchView for playlist updates current playlist and refreshes tracks", async () => {
  const { doc, el } = makeFixture();
  const state = { activeTab: "library", playlists: [{ id: "pl-1" }], currentPlaylistId: null };
  let refreshed = 0;
  await switchView(state, el, "pl-1", {
    staticTabs: ["library", "event-log"],
    stopPlaybackIfActive: async () => {},
    syncLibraryOnboardingMode: () => {},
    updateModeText: () => {},
    populatePlaylistPanel: () => {},
    refreshCurrentPlaylistTracks: async () => { refreshed += 1; },
    renderEventLog: () => {},
    requestAnimationFrameFn: (cb) => cb(),
    documentObj: doc,
    renderWaveformsIn: () => {}
  });
  assert.equal(state.currentPlaylistId, "pl-1");
  assert.equal(refreshed, 1);
});

test("switchTab delegates to switchView behavior", async () => {
  const { doc, el } = makeFixture();
  const state = { activeTab: "library", playlists: [], currentPlaylistId: null };
  await switchTab(state, el, "library", {
    staticTabs: ["library", "event-log"],
    stopPlaybackIfActive: async () => {},
    syncLibraryOnboardingMode: () => {},
    updateModeText: () => {},
    populatePlaylistPanel: () => {},
    refreshCurrentPlaylistTracks: async () => {},
    renderEventLog: () => {},
    requestAnimationFrameFn: (cb) => cb(),
    documentObj: doc,
    renderWaveformsIn: () => {}
  });
  assert.equal(state.activeTab, "library");
});
