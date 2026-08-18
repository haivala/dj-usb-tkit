import test from "node:test";
import assert from "node:assert/strict";
import {
  trackNeedsPreviewHydration,
  mergeHydratedTrackIntoState,
  hydrateTrackPreviewFromBackend,
  hydrateLoadedTracksPreviewsInBackground
} from "../components/library/actions.mjs";

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
  const changed = mergeHydratedTrackIntoState(state, { id: "1", title: "New", waveformPreview: [] }, {
    normalizeTrack: (t) => t
  });
  assert.equal(changed, true);
  assert.equal(state.tracks[0].title, "New");
  assert.deepEqual(state.tracks[0].waveformPreview, [20]);
  assert.equal(state.playlists[0].tracks[0].title, "New");
  assert.deepEqual(state.playlists[0].tracks[0].waveformPreview, [30]);
});

test("hydrateTrackPreviewFromBackend applies updates and patches the row", async () => {
  const state = {
    trackPreviewHydrateInFlight: new Set(),
    analyzingTrackIds: new Set(),
    loadedPreviewHydrationSeq: 0
  };
  const calls = [];
  await hydrateTrackPreviewFromBackend(state, "1", {
    command: async () => ({ items: [{ id: "1" }] }),
    mergeHydratedTrackIntoState: () => true,
    patchLibraryRowByTrackId: (id) => calls.push(`patch:${id}`)
  });
  assert.deepEqual(calls, ["patch:1"]);
  assert.equal(state.trackPreviewHydrateInFlight.size, 0);
});

test("hydrateLoadedTracksPreviewsInBackground batches and patches", async () => {
  const state = { loadedPreviewHydrationSeq: 0 };
  const patched = [];
  await hydrateLoadedTracksPreviewsInBackground(state, {
    getLibraryVisibleTracks: () => [
      { id: "1", waveformPreview: [], waveformPeaksPath: "/a" },
      { id: "2", waveformPreview: [], waveformPeaksPath: "/b" }
    ],
    command: async () => ({ items: [{ id: "1" }, { id: "2" }] }),
    mergeHydratedTrackIntoState: () => true,
    patchLibraryRowByTrackId: (id) => patched.push(id),
    nextPaint: async () => {},
    applySearchLocalFilter: () => {},
    renderCurrentPlaylistTracksFromState: () => {},
    renderSourceChips: () => {},
    batchSize: 10
  });
  assert.deepEqual(patched.sort(), ["1", "2"]);
});
