import test from "node:test";
import assert from "node:assert/strict";
import {
  applySortToTracks
} from "../components/shell/actions.mjs";
import {
  renderHistoryTracks
} from "../components/usb/actions.mjs";

test("applySortToTracks applies configured state sorter", () => {
  const out = applySortToTracks({ body: { key: "title", dir: "asc" } }, [{ title: "b" }, { title: "a" }], "body", {
    sortTracks: (tracks) => [...tracks].sort((a, b) => a.title.localeCompare(b.title))
  });
  assert.deepEqual(out.map((t) => t.title), ["a", "b"]);
});

// The USB-playlist track view is now driven by the shared TrackListController
// (see tests/track_list_controller.test.mjs); only USB history still uses this
// legacy self-hydrating render path.
test("renderHistoryTracks computes view and renders table", () => {
  const state = { historyTracks: [{ id: "a" }], historyTrackSearch: "", historyTracksView: [] };
  const el = { historyTracks: {}, historyTotalDuration: {} };
  let renderedCount = 0;
  renderHistoryTracks(state, el, {
    filterTracksByQuery: (tracks) => tracks,
    applySortToTracks: (tracks) => tracks,
    renderTrackTable: (_tbody, tracks) => { renderedCount = tracks.length; }
  });
  assert.equal(state.historyTracksView.length, 1);
  assert.equal(renderedCount, 1);
});
