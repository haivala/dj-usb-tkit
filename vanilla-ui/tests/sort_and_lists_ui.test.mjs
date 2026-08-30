import test from "node:test";
import assert from "node:assert/strict";
import {
  applySortToTracks
} from "../components/shell/actions.mjs";

test("applySortToTracks applies configured state sorter", () => {
  const out = applySortToTracks({ body: { key: "title", dir: "asc" } }, [{ title: "b" }, { title: "a" }], "body", {
    sortTracks: (tracks) => [...tracks].sort((a, b) => a.title.localeCompare(b.title))
  });
  assert.deepEqual(out.map((t) => t.title), ["a", "b"]);
});

// The USB playlist and history track views are driven by the shared
// TrackListController (see tests/track_list_controller.test.mjs +
// tests/e2e/usb_large_playlist_pagination.spec.mjs). Column-sort header clicks
// route into the controller via bodyToRendererMap -> applyHeaderSort.
