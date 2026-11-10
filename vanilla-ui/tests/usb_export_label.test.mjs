import test from "node:test";
import assert from "node:assert/strict";

import { playlistUsbExportStatusById } from "../components/usb/actions.mjs";

// Append-vs-export text for a real reorder-locking / non-colliding playlist
// is covered end-to-end via the Tauri stubs in
// tests/e2e/playlist_analysis_actions.spec.mjs ("...disabled with a tooltip
// when additive export won't reorder it on the USB" and "...stays enabled in
// additive mode when no same-name USB playlist exists"), which also proves
// the backend-computed field actually reaches the DOM. The `locksReorder`
// boolean itself is now owned entirely by the backend -- unit- and
// functional-tested in backend/src/service/export.rs.

test("playlistUsbExportStatusById indexes the backend's per-playlist status by playlist id", () => {
  const statusById = playlistUsbExportStatusById([
    { playlistId: "p1", playlistName: "Testi", usbSelected: true, sameNameExistsOnUsb: true, locksReorder: true },
    { playlistId: "p2", playlistName: "House", usbSelected: true, sameNameExistsOnUsb: false, locksReorder: false }
  ]);

  assert.equal(statusById.size, 2);
  assert.equal(statusById.get("p1").sameNameExistsOnUsb, true);
  assert.equal(statusById.get("p2").locksReorder, false);
  assert.equal(statusById.get("missing"), undefined);
});
