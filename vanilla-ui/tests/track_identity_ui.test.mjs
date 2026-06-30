import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  patchLibraryRowByTrackId,
  patchPlaylistRowByTrackId,
  setTrackAnalyzingState,
  promoteTrackIdentity
} from "../components/library/actions.mjs";

test("patchLibraryRowByTrackId patches row and toggles analyzing UI", () => {
  const dom = new JSDOM(`<!doctype html><body><table><tbody id="lib"><tr class="track-grid-row" data-track-origin="local" data-track-id="t1"><td><button data-action="analyze-track"></button></td></tr></tbody></table></body>`);
  const state = {
    tracks: [{ id: "t1", title: "Track" }],
    analyzingTrackIds: new Set(["t1"])
  };
  const el = { libraryTableBody: dom.window.document.querySelector("#lib") };
  let patchedTrack = null;
  const ok = patchLibraryRowByTrackId(state, el, "t1", {
    cssEscape: (v) => v,
    patchLibraryRowCells: (_row, track) => {
      patchedTrack = track;
      return true;
    }
  });
  assert.equal(ok, true);
  assert.equal(patchedTrack?.id, "t1");
  const row = el.libraryTableBody.querySelector(".track-grid-row");
  assert.equal(row.classList.contains("is-analyzing"), true);
  assert.equal(row.querySelector("[data-action='analyze-track']").disabled, true);
});

test("patchPlaylistRowByTrackId resolves by localTrackId", () => {
  const dom = new JSDOM(`<!doctype html><body><table><tbody id="pl"><tr class="track-grid-row" data-track-origin="local" data-track-id="local-2"></tr></tbody></table></body>`);
  const state = {
    analyzingTrackIds: new Set(),
    playlists: []
  };
  const playlist = { tracks: [{ id: "usb-2", localTrackId: "local-2", title: "Song" }] };
  const el = { playlistTracksBody: dom.window.document.querySelector("#pl") };
  let patched = null;
  const ok = patchPlaylistRowByTrackId(state, el, "local-2", {
    cssEscape: (v) => v,
    getCurrentPlaylist: () => playlist,
    patchLibraryRowCells: (_row, track) => {
      patched = track;
      return true;
    }
  });
  assert.equal(ok, true);
  assert.equal(patched?.localTrackId, "local-2");
});

test("setTrackAnalyzingState updates set and triggers summary/chips when done", () => {
  const state = {
    analyzingTrackIds: new Set(["x"]),
    tracks: [{ id: "x", durationMs: 1000, bpm: 120, waveformPreview: [1] }]
  };
  const calls = [];
  setTrackAnalyzingState(state, "x", false, {
    patchLibraryRowByTrackId: (id) => calls.push(`lib:${id}`),
    patchPlaylistRowByTrackId: (id) => calls.push(`pl:${id}`),
    trackHasCoreAnalysis: () => true,
    trackNeedsPreviewHydration: () => false,
    getLibraryVisibleTracks: () => [{ id: "x" }],
    updateLibraryDurationSummary: () => calls.push("summary"),
    renderSourceChips: () => calls.push("chips")
  });
  assert.equal(state.analyzingTrackIds.has("x"), false);
  assert.deepEqual(calls, ["lib:x", "pl:x"]);
});

test("promoteTrackIdentity updates state ids and row dataset ids", () => {
  const dom = new JSDOM(`<!doctype html><body><table><tbody id="lib"><tr class="track-grid-row" data-track-origin="local" data-track-id="old"><td><button data-id="old"></button></td></tr></tbody></table></body>`);
  const state = {
    tracks: [{ id: "old", localTrackId: null }],
    selectedTrackIds: new Set(["old"]),
    playlists: [{ tracks: [{ id: "old", localTrackId: "old" }] }]
  };
  const el = { libraryTableBody: dom.window.document.querySelector("#lib") };
  promoteTrackIdentity(state, el, "old", "new", { cssEscape: (v) => v });

  assert.equal(state.tracks[0].id, "new");
  assert.equal(state.tracks[0].localTrackId, "new");
  assert.equal(state.selectedTrackIds.has("new"), true);
  assert.equal(state.selectedTrackIds.has("old"), false);
  assert.equal(state.playlists[0].tracks[0].id, "new");
  assert.equal(state.playlists[0].tracks[0].localTrackId, "new");
  const row = el.libraryTableBody.querySelector(".track-grid-row");
  assert.equal(row.dataset.trackId, "new");
  assert.equal(row.querySelector("[data-id]").dataset.id, "new");
});
