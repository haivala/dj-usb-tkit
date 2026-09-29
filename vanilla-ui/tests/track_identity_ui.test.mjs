import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  setTrackAnalyzingState,
  promoteTrackIdentity
} from "../components/library/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

test("setTrackAnalyzingState updates the set and the rendered library row", async () => {
  const ctx = makeTestCtx();
  ctx.state.tracks = [{ id: "x", title: "X", durationMs: 1000, bpm: 120, waveformPreview: [1] }];
  await ctx.libraryTracksCtl.rerender();
  const row = ctx.el.libraryTableBody.querySelector('.track-grid-row[data-track-id="x"]');

  setTrackAnalyzingState(ctx, "x", true);
  assert.equal(ctx.state.analyzingTrackIds.has("x"), true);
  assert.equal(row.classList.contains("is-analyzing"), true);

  setTrackAnalyzingState(ctx, "x", false);
  assert.equal(ctx.state.analyzingTrackIds.has("x"), false);
  assert.equal(row.classList.contains("is-analyzing"), false);
});

test("promoteTrackIdentity updates state ids and row dataset ids", () => {
  const dom = new JSDOM(`<!doctype html><body><table><tbody id="lib"><tr class="track-grid-row" data-track-origin="local" data-track-id="old"><td><button data-id="old"></button></td></tr></tbody></table></body>`);
  const state = {
    tracks: [{ id: "old", localTrackId: null }],
    selectedTrackIds: new Set(["old"]),
    playlists: [{ tracks: [{ id: "old", localTrackId: "old" }] }]
  };
  const el = { libraryTableBody: dom.window.document.querySelector("#lib") };
  promoteTrackIdentity({ state, el }, "old", "new");

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
