import test from "node:test";
import assert from "node:assert/strict";

import {
  computeExportButtonState,
  knownUsbPlaylistNamesFromPlaylists
} from "../components/usb/actions.mjs";

test("computeExportButtonState shows append text when additive mode targets playlist known from diagnostics", () => {
  const knownUsbPlaylistNames = new Set(["testi"]);

  const state = computeExportButtonState({
    usbRoot: "/tmp/USB",
    usbRootValid: true,
    exportPruneStale: false,
    currentPlaylistName: "Testi",
    knownUsbPlaylistNames
  });

  assert.equal(state.enabled, true);
  assert.equal(state.text, "Append to (Testi) on USB: (USB)");
  assert.equal(state.title, 'Append current playlist tracks to existing USB playlist "Testi"');
});

test("knownUsbPlaylistNamesFromPlaylists normalizes imported playlist names for append detection", () => {
  const knownUsbPlaylistNames = knownUsbPlaylistNamesFromPlaylists([
    { name: "  Testi " },
    { name: "House" }
  ]);

  const state = computeExportButtonState({
    usbRoot: "/tmp/USB",
    usbRootValid: true,
    exportPruneStale: false,
    currentPlaylistName: "testi",
    knownUsbPlaylistNames
  });

  assert.equal(knownUsbPlaylistNames.has("testi"), true);
  assert.equal(state.text, "Append to (testi) on USB: (USB)");
});

test("computeExportButtonState keeps export text in mirror mode even when same-name USB playlist exists", () => {
  const state = computeExportButtonState({
    usbRoot: "/tmp/USB",
    usbRootValid: true,
    exportPruneStale: true,
    currentPlaylistName: "Testi",
    knownUsbPlaylistNames: new Set(["testi"])
  });

  assert.equal(state.enabled, true);
  assert.equal(state.text, "Export to USB: USB");
  assert.equal(state.title, "Export current playlist to selected USB");
});

test("computeExportButtonState shows Select USB first when no USB root is set", () => {
  const state = computeExportButtonState({
    usbRoot: null,
    usbRootValid: false,
    exportPruneStale: false,
    currentPlaylistName: "Testi",
    knownUsbPlaylistNames: new Set()
  });

  assert.equal(state.enabled, false);
  assert.equal(state.text, "Select USB first");
  assert.equal(state.title, "Select a valid USB folder first");
});

test("computeExportButtonState appends last path segment to export text", () => {
  const state = computeExportButtonState({
    usbRoot: "/media/user/USB_TRY",
    usbRootValid: true,
    exportPruneStale: true,
    currentPlaylistName: "Testi",
    knownUsbPlaylistNames: new Set()
  });

  assert.equal(state.enabled, true);
  assert.equal(state.text, "Export to USB: USB_TRY");
});
