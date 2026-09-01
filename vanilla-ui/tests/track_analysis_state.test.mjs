import test from "node:test";
import assert from "node:assert/strict";
import { normalizeTrack, trackArtworkChecked } from "../components/library/actions.mjs";

// "Needs a deeper USB metadata fetch" is computed in Rust now
// (service::usb::hydrate_usb_track_in_place, tested there). The frontend just
// carries the `needsHydration` flag through normalizeTrack.

const deps = { toPlayableUrl: (v) => v, appendUrlRevision: (u) => u, normalizeDurationMs: () => null };

test("normalizeTrack carries the backend needsHydration flag (USB rows)", () => {
  assert.equal(normalizeTrack({ id: "1", needsHydration: true }, "usb", deps).needsHydration, true);
  assert.equal(normalizeTrack({ id: "2", needsHydration: false }, "usb", deps).needsHydration, false);
  // absent / non-boolean -> false
  assert.equal(normalizeTrack({ id: "3" }, "lib", deps).needsHydration, false);
});

test("trackArtworkChecked reflects the frontend runtime flag", () => {
  assert.equal(trackArtworkChecked({ artworkChecked: true }), true);
  assert.equal(trackArtworkChecked({}), false);
});
