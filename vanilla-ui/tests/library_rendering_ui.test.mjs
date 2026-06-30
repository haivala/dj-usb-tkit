import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  renderCurrentPlaylistTracksFromState,
  updateLibraryDurationSummary,
  renderLibraryRows
} from "../components/library/actions.mjs";

test("renderCurrentPlaylistTracksFromState renders playlist tracks and empty state", () => {
  const dom = new JSDOM(`<!doctype html><body><div id="empty"></div><div id="wrap"></div><table><tbody id="body"></tbody></table><div id="dur"></div></body>`);
  const state = {
    playlistTrackSearch: "",
    currentPlaylistTracksView: [],
    analyzingTrackIds: new Set(["t1"])
  };
  const el = {
    playlistEmptyState: dom.window.document.querySelector("#empty"),
    playlistTableWrap: dom.window.document.querySelector("#wrap"),
    playlistTracksBody: dom.window.document.querySelector("#body"),
    playlistTotalDuration: dom.window.document.querySelector("#dur")
  };
  let rendered = 0;
  renderCurrentPlaylistTracksFromState(state, el, {
    getCurrentPlaylist: () => ({ tracks: [{ id: "t1" }] }),
    filterTracksByQuery: (tracks) => tracks,
    renderEmptyState: () => {},
    applySortToTracks: (tracks) => tracks,
    renderTrackTable: (tbody) => {
      rendered += 1;
      tbody.innerHTML = '<tr class="track-grid-row" data-track-id="t1" data-track-origin="local"></tr>';
    },
    cssEscape: (v) => v,
    updateTrackListDurationSummary: () => {}
  });
  assert.equal(rendered, 1);
  assert.equal(state.currentPlaylistTracksView.length, 1);
  assert.equal(el.playlistTracksBody.querySelector(".track-grid-row").classList.contains("is-analyzing"), true);
});

test("updateLibraryDurationSummary nulls duration for non-core-analysis tracks", () => {
  const el = { libraryTotalDuration: {} };
  let captured = null;
  updateLibraryDurationSummary(el, [{ id: "a", durationMs: 100 }, { id: "b", durationMs: 200 }], {
    trackHasCoreAnalysis: (t) => t.id === "a",
    updateTrackListDurationSummary: (_target, tracks) => { captured = tracks; }
  });
  assert.equal(captured[0].durationMs, 100);
  assert.equal(captured[1].durationMs, null);
});

test("renderLibraryRows renders empty onboarding and tracks table", () => {
  const dom = new JSDOM(`<!doctype html><body><div id="empty"></div><div id="content"></div><button id="add"></button><table><tbody id="body"></tbody></table></body>`);
  const state = {
    sourceRoots: ["/music"],
    selectedTrackIds: new Set(),
    analyzingTrackIds: new Set(["1"])
  };
  const el = {
    libraryEmptyState: dom.window.document.querySelector("#empty"),
    libraryContent: dom.window.document.querySelector("#content"),
    addSourceBtn: dom.window.document.querySelector("#add"),
    libraryTableBody: dom.window.document.querySelector("#body")
  };
  let rendered = 0;
  let summarized = null;
  renderLibraryRows(state, el, {
    getLibraryVisibleTracks: () => [{ id: "1" }],
    renderEmptyState: () => {},
    syncLibraryOnboardingMode: () => {},
    applySortToTracks: (tracks) => tracks,
    renderTrackTable: (tbody) => {
      rendered += 1;
      tbody.innerHTML = '<tr class="track-grid-row" data-track-id="1" data-track-origin="local"></tr>';
    },
    cssEscape: (v) => v,
    updateLibraryDurationSummary: (tracks) => { summarized = tracks; }
  });
  assert.equal(rendered, 1);
  assert.equal(Array.isArray(summarized), true);
  assert.equal(el.libraryTableBody.querySelector(".track-grid-row").classList.contains("is-analyzing"), true);
});
