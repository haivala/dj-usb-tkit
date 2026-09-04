import test from "node:test";
import assert from "node:assert/strict";

import {
  computeExportButtonState,
  playlistUsbExportStatusById,
  refreshPlaylistExportStatus
} from "../components/usb/actions.mjs";

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
    { playlistId: "p1", playlistName: "Testi", sameNameExistsOnUsb: true, locksReorder: true },
    { playlistId: "p2", playlistName: "House", sameNameExistsOnUsb: false, locksReorder: false },
    { playlistId: "", playlistName: "Unnamed", sameNameExistsOnUsb: false, locksReorder: false }
  ]);

  assert.equal(statusById.size, 2);
  assert.equal(statusById.get("p1").sameNameExistsOnUsb, true);
  assert.equal(statusById.get("p2").locksReorder, false);
  assert.equal(statusById.get("missing"), undefined);
});

test("refreshPlaylistExportStatus re-indexes state from the backend command's response", async () => {
  const state = { usbRoot: "/media/usb", playlistUsbExportStatusById: new Map() };
  const calls = [];
  const command = async (name, payload) => {
    calls.push({ name, payload });
    return {
      playlistUsbExportStatus: [
        { playlistId: "p1", playlistName: "Testi", sameNameExistsOnUsb: true, locksReorder: true },
        { playlistId: "p2", playlistName: "House", sameNameExistsOnUsb: false, locksReorder: false }
      ]
    };
  };

  const byId = await refreshPlaylistExportStatus(state, { command });

  assert.deepEqual(calls, [{ name: "refresh_playlist_export_status", payload: { usbRoot: "/media/usb" } }]);
  assert.equal(byId, state.playlistUsbExportStatusById);
  assert.equal(state.playlistUsbExportStatusById.get("p1").locksReorder, true);
  assert.equal(state.playlistUsbExportStatusById.get("p2").locksReorder, false);
});

test("refreshPlaylistExportStatus passes null usbRoot when none is connected", async () => {
  const state = { usbRoot: null, playlistUsbExportStatusById: new Map([["p1", {}]]) };
  const command = async () => ({ playlistUsbExportStatus: [] });

  await refreshPlaylistExportStatus(state, { command });
  assert.equal(state.playlistUsbExportStatusById.size, 0);
});

test("computeExportButtonState renders the backend-computed label for the current playlist", () => {
  const statusById = playlistUsbExportStatusById([
    {
      playlistId: "p1",
      playlistName: "Testi",
      sameNameExistsOnUsb: false,
      locksReorder: false,
      exportButtonText: "Export to USB: USB_TRY",
      exportButtonTitle: "Export current playlist to selected USB"
    }
  ]);

  const state = computeExportButtonState({
    usbRoot: "/media/user/USB_TRY",
    usbRootValid: true,
    currentPlaylistId: "p1",
    playlistUsbExportStatusById: statusById
  });

  assert.equal(state.enabled, true);
  assert.equal(state.text, "Export to USB: USB_TRY");
  assert.equal(state.title, "Export current playlist to selected USB");
});

test("computeExportButtonState shows the select-USB prompt when no valid USB is set", () => {
  const state = computeExportButtonState({
    usbRoot: "",
    usbRootValid: false,
    currentPlaylistId: "p1",
    playlistUsbExportStatusById: new Map()
  });

  assert.equal(state.enabled, false);
  assert.equal(state.text, "Select USB first");
});
