import test from "node:test";
import assert from "node:assert/strict";
import {
  loadUsbRootFromStorage,
  resetUsbStateViews,
  syncAssetScopePaths,
  pickSourceFolders
} from "../components/usb/actions.mjs";
import { STORAGE_KEY_USB_ROOT } from "../settings_keys.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

test("loadUsbRootFromStorage hydrates usb root and updates controls", () => {
  let exportUpdates = 0;
  const ctx = makeTestCtx({ updatePlaylistExportButtons: () => { exportUpdates += 1; } });
  ctx.localStorage.setItem(STORAGE_KEY_USB_ROOT, " /usb ");
  Object.assign(ctx.state, { usbRoot: null, usbRootValid: true, usbNeedsInit: true });

  loadUsbRootFromStorage(ctx);

  assert.equal(ctx.state.usbRoot, "/usb");
  assert.equal(ctx.state.usbRootValid, false);
  assert.equal(ctx.state.usbNeedsInit, false);
  assert.equal(ctx.el.usbRootPathText.textContent, "No USB selected", "a stored root is not shown until validated");
  assert.equal(ctx.el.usbSelectedControls.classList.contains("hidden"), true);
  assert.equal(exportUpdates, 1);
  assert.equal(ctx.el.usbInitRow.classList.contains("hidden"), true);
});

test("resetUsbStateViews clears lists and rerenders", () => {
  const ctx = makeTestCtx();
  Object.assign(ctx.state, {
    usbPlaylists: [{ id: 1, name: "One" }],
    playlistUsbExportStatusById: new Map([["p1", { sameNameExistsOnUsb: true, locksReorder: false }]]),
    histories: [{ id: 1, name: "Session" }],
    historyTracks: [{ id: 1 }]
  });
  ctx.el.usbCountsText.textContent = "x";

  resetUsbStateViews(ctx);

  assert.equal(ctx.state.usbPlaylists.length, 0);
  assert.equal(ctx.state.histories.length, 0);
  assert.equal(ctx.state.playlistUsbExportStatusById.size, 0);
  assert.equal(ctx.el.usbCountsText.textContent, "");
  assert.match(ctx.el.usbPlaylists.textContent, /No playlists imported yet/);
  assert.match(ctx.el.historyList.textContent, /No history imported yet/);
});

test("syncAssetScopePaths calls allow_asset_paths with roots and usb root", async () => {
  const state = { sourceRoots: ["/music"], usbRoot: "/usb" };
  let called = null;
  await syncAssetScopePaths({
    state,
    invoke: async (name, payload) => { called = { name, payload }; },
    warn: () => {}
  });
  assert.equal(called.name, "allow_asset_paths");
  assert.deepEqual(called.payload.paths, ["/music", "/usb"]);
});

test("pickSourceFolders normalizes mixed picker payload", async () => {
  const folders = await pickSourceFolders({
    invoke: async () => [
      "/a",
      { path: "/b" },
      { Path: "/c" },
      { url: "/d" },
      { Url: "/e" },
      { filePath: "/f" },
      null
    ]
  });
  assert.deepEqual(folders, ["/a", "/b", "/c", "/d", "/e", "/f"]);
});
