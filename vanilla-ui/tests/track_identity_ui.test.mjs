import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  setTrackAnalyzingState,
  showTrackRow
} from "../components/library/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

test("setTrackAnalyzingState updates the set and the rendered library row", async () => {
  const ctx = makeTestCtx();
  ctx.state.tracks = [{ id: "x", title: "X", durationMs: 1000, bpm: 120, waveformPreview: [1] }];
  ctx.el.libraryTableBody.append(ctx.buildTrackTableRow(ctx.state.tracks[0], { origin: "local", index: 0 }));
  const row = ctx.el.libraryTableBody.querySelector('.track-grid-row[data-track-id="x"]');

  setTrackAnalyzingState(ctx, "x", true);
  assert.equal(ctx.state.analyzingTrackIds.has("x"), true);
  assert.equal(row.classList.contains("is-analyzing"), true);

  setTrackAnalyzingState(ctx, "x", false);
  assert.equal(ctx.state.analyzingTrackIds.has("x"), false);
  assert.equal(row.classList.contains("is-analyzing"), false);
});

test("showTrackRow with replaces shows a listed file's new library row in place of its own", () => {
  const ctx = makeTestCtx();
  const path = "/music/a.mp3";
  ctx.state.tracks = [{ id: path, title: "A", filePath: path, durationMs: 1000, waveformPreview: [1] }];
  ctx.state.selectedTrackIds = new Set([path]);
  ctx.state.playlists = [{ tracks: [{ id: path, title: "A", filePath: path }] }];
  ctx.el.libraryTableBody.append(ctx.buildTrackTableRow(ctx.state.tracks[0], { origin: "local", index: 0 }));
  const copy = ctx.state.tracks[0];

  showTrackRow(ctx, { id: "lib-1", title: "A", filePath: path, durationMs: 1000, bpm: 128, waveformPreview: [1] },
    { replaces: path });

  // The same object, now the library row.
  assert.equal(ctx.state.tracks[0], copy);
  assert.equal(copy.id, "lib-1");
  assert.equal(copy.bpm, 128);
  assert.equal(ctx.state.playlists[0].tracks[0].id, "lib-1");
  assert.equal(ctx.state.selectedTrackIds.has("lib-1"), true);
  assert.equal(ctx.state.selectedTrackIds.has(path), false);
  const row = ctx.el.libraryTableBody.querySelector(".track-grid-row");
  assert.equal(row.dataset.trackId, "lib-1");
  for (const node of row.querySelectorAll("[data-id]")) assert.equal(node.dataset.id, "lib-1");
});
