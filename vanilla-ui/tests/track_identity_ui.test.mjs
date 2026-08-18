import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  setTrackAnalyzingState,
  promoteTrackIdentity
} from "../components/library/actions.mjs";

test("setTrackAnalyzingState updates set and patches library/playlist rows", () => {
  const state = {
    analyzingTrackIds: new Set(["x"]),
    tracks: [{ id: "x", durationMs: 1000, bpm: 120, waveformPreview: [1] }]
  };
  const calls = [];
  setTrackAnalyzingState(state, "x", false, {
    patchLibraryRowByTrackId: (id) => calls.push(`lib:${id}`),
    patchPlaylistRowByTrackId: (id) => calls.push(`pl:${id}`)
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
