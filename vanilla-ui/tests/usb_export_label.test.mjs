import test from "node:test";
import assert from "node:assert/strict";

import {
  computeExportButtonState,
  playlistUsbExportStatusById
} from "../components/usb/actions.mjs";

test("computeExportButtonState shows append text when the backend reports a reorder-locking collision", () => {
  const statusById = playlistUsbExportStatusById([
    { playlistId: "p1", playlistName: "Testi", sameNameExistsOnUsb: true, locksReorder: true }
  ]);

  const state = computeExportButtonState({
    usbRoot: "/tmp/USB",
    usbRootValid: true,
    currentPlaylistId: "p1",
    currentPlaylistName: "Testi",
    playlistUsbExportStatusById: statusById
  });

  assert.equal(state.enabled, true);
  assert.equal(state.text, "Append to (Testi) on USB: (USB)");
  assert.equal(state.title, 'Append current playlist tracks to existing USB playlist "Testi"');
});

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

test("computeExportButtonState keeps export text when the backend reports no reorder lock (mirror mode)", () => {
  const statusById = playlistUsbExportStatusById([
    { playlistId: "p1", playlistName: "Testi", sameNameExistsOnUsb: true, locksReorder: false }
  ]);

  const state = computeExportButtonState({
    usbRoot: "/tmp/USB",
    usbRootValid: true,
    currentPlaylistId: "p1",
    currentPlaylistName: "Testi",
    playlistUsbExportStatusById: statusById
  });

  assert.equal(state.enabled, true);
  assert.equal(state.text, "Export to USB: USB");
  assert.equal(state.title, "Export current playlist to selected USB");
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
