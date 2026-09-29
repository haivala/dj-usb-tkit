import test from "node:test";
import assert from "node:assert/strict";
import { normalizeTrack, trackArtworkChecked } from "../components/library/actions.mjs";

// "Needs a deeper USB metadata fetch" is computed in Rust now
// (service::usb::hydrate_usb_track_in_place, tested there). The frontend just
// carries the `needsHydration` flag through normalizeTrack.

const ctx = { toPlayableUrl: (v) => v };

test("normalizeTrack carries the backend needsHydration flag (USB rows)", () => {
  assert.equal(normalizeTrack(ctx, { id: "1", needsHydration: true }, "usb").needsHydration, true);
  assert.equal(normalizeTrack(ctx, { id: "2", needsHydration: false }, "usb").needsHydration, false);
  // absent / non-boolean -> false
  assert.equal(normalizeTrack(ctx, { id: "3" }, "lib").needsHydration, false);
});

test("trackArtworkChecked reflects the frontend runtime flag", () => {
  assert.equal(trackArtworkChecked({ artworkChecked: true }), true);
  assert.equal(trackArtworkChecked({}), false);
});
