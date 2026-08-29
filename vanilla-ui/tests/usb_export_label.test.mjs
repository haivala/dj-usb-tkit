import test from "node:test";
import assert from "node:assert/strict";

import {
  computeExportButtonState,
  playlistUsbExportStatusById
} from "../components/usb/actions.mjs";
import {
  playlistLocksReorderOnExport,
  recomputeReorderLocks
} from "../components/shared/export_reorder_lock.mjs";

// Append-vs-export text for a real reorder-locking / non-colliding playlist
// is covered end-to-end via the Tauri mock in
// tests/e2e/playlist_analysis_actions.spec.mjs ("...disabled with a tooltip
// when additive export won't reorder it on the USB" and "...stays enabled in
// additive mode when no same-name USB playlist exists"), which also proves
// the backend-computed field actually reaches the DOM. The `locksReorder`
// boolean itself is unit- and functional-tested backend-side (see
// backend/src/service/export.rs).

test("playlistUsbExportStatusById indexes the backend's per-playlist status by playlist id", () => {
  const statusById = playlistUsbExportStatusById([
    { playlistId: "p1", playlistName: "Testi", sameNameExistsOnUsb: true, locksReorder: true },
    { playlistId: "p2", playlistName: "House", sameNameExistsOnUsb: false, locksReorder: false },
    { playlistId: "", playlistName: "Unnamed", sameNameExistsOnUsb: false, locksReorder: false }
  ]);

  assert.equal(statusById.size, 2);
  assert.equal(statusById.get("p1").sameNameExistsOnUsb, true);
  assert.equal(statusById.get("p2").locksReorder, false);
  assert.equal(statusById.get("missing"), undefined);
});

// Counterpart of the backend test
// `playlist_locks_reorder_on_export_only_when_additive_and_same_name_exists`
// in backend/src/service/export.rs -- if the rule changes on one side, one of
// these two tests fails.
test("playlistLocksReorderOnExport locks only in additive mode with a same-named USB playlist", () => {
  assert.equal(playlistLocksReorderOnExport(true, true), false); // mirror
  assert.equal(playlistLocksReorderOnExport(false, true), true); // additive + collision
  assert.equal(playlistLocksReorderOnExport(false, false), false); // additive, no collision
  assert.equal(playlistLocksReorderOnExport(true, false), false);
  assert.equal(playlistLocksReorderOnExport(false, undefined), false);
});

test("recomputeReorderLocks re-derives locksReorder from cached sameNameExistsOnUsb", () => {
  const input = new Map([
    ["p1", { playlistId: "p1", playlistName: "Testi", sameNameExistsOnUsb: true, locksReorder: false }],
    ["p2", { playlistId: "p2", playlistName: "House", sameNameExistsOnUsb: false, locksReorder: false }]
  ]);

  const additive = recomputeReorderLocks(input, false);
  assert.notEqual(additive, input);
  assert.equal(input.get("p1").locksReorder, false, "input entry not mutated");
  assert.equal(additive.get("p1").locksReorder, true);
  assert.equal(additive.get("p1").playlistName, "Testi");
  assert.equal(additive.get("p1").sameNameExistsOnUsb, true);
  assert.equal(additive.get("p2").locksReorder, false);

  const mirror = recomputeReorderLocks(additive, true);
  assert.equal(mirror.get("p1").locksReorder, false);
  assert.equal(mirror.get("p2").locksReorder, false);

  assert.equal(recomputeReorderLocks(null, false).size, 0);
  assert.equal(recomputeReorderLocks(undefined, true).size, 0);
});

test("computeExportButtonState appends last path segment to export text", () => {
  const state = computeExportButtonState({
    usbRoot: "/media/user/USB_TRY",
    usbRootValid: true,
    currentPlaylistId: "p1",
    currentPlaylistName: "Testi",
    playlistUsbExportStatusById: new Map()
  });

  assert.equal(state.enabled, true);
  assert.equal(state.text, "Export to USB: USB_TRY");
});
