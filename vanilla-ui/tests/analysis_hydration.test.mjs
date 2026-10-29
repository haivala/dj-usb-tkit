import test from "node:test";
import assert from "node:assert/strict";
import {
  trackNeedsPreviewHydration,
  mergeHydratedTrackIntoState,
  hydrateLoadedTracksPreviewsInBackground
} from "../components/library/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

test("trackNeedsPreviewHydration requires waveform path and missing preview", () => {
  assert.equal(trackNeedsPreviewHydration({ waveformPreview: [], waveformPeaksPath: "/a.DAT" }), true);
  assert.equal(trackNeedsPreviewHydration({ waveformPreview: [1], waveformPeaksPath: "/a.DAT" }), false);
  assert.equal(trackNeedsPreviewHydration({ waveformPreview: [], waveformPeaksPath: "" }), false);
});

test("mergeHydratedTrackIntoState merges into library and playlist", () => {
  const state = {
    tracks: [{ id: "1", title: "Old", waveformPreview: [20], artworkUrl: "old.jpg" }],
    playlists: [{ tracks: [{ id: "x", localTrackId: "1", title: "Old P", waveformPreview: [30] }] }]
  };
  const changed = mergeHydratedTrackIntoState(makeTestCtx({ state }), { id: "1", title: "New", waveformPreview: [] });
  assert.equal(changed, true);
  assert.equal(state.tracks[0].title, "New");
  assert.deepEqual(state.tracks[0].waveformPreview, [20]);
  assert.equal(state.playlists[0].tracks[0].title, "New");
  assert.deepEqual(state.playlists[0].tracks[0].waveformPreview, [30]);
});

test("hydrateLoadedTracksPreviewsInBackground fetches every track still missing a preview", async () => {
  const ctx = makeTestCtx({
    command: async (name, request) => (name === "get_tracks_by_ids_with_previews"
      ? { items: request.trackIds.map((id) => ({ id, title: id, waveformPreview: [7] })) }
      : {}),
    renderCurrentPlaylistTracksFromState: async () => {},
  });
  ctx.state.tracks = [
    { id: "1", waveformPreview: [], waveformPeaksPath: "/a" },
    { id: "2", waveformPreview: [], waveformPeaksPath: "/b" }
  ];
  await hydrateLoadedTracksPreviewsInBackground(ctx);
  assert.deepEqual(ctx.state.tracks.map((t) => t.waveformPreview), [[7], [7]]);
});
