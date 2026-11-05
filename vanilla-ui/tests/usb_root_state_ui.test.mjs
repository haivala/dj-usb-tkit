import test from "node:test";
import assert from "node:assert/strict";
import {
  loadUsbRootFromStorage,
  syncAssetScopePaths,
  pickSourceFolders
} from "../components/usb/actions.mjs";
import { STORAGE_KEY_USB_ROOT } from "../settings_keys.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

test("loadUsbRootFromStorage hydrates usb root and updates controls", () => {
  let exportUpdates = 0;
  const ctx = makeTestCtx({ updatePlaylistExportButtons: () => { exportUpdates += 1; } });
  ctx.localStorage.setItem(STORAGE_KEY_USB_ROOT, " /usb ");
  Object.assign(ctx.state, { usbRoot: null, usbRootValid: true });

  loadUsbRootFromStorage(ctx);

  assert.equal(ctx.state.usbRoot, "/usb");
  assert.equal(ctx.state.usbRootValid, false);
  assert.equal(ctx.el.usbRootPathText.textContent, "No USB selected", "a stored root is not shown until validated");
  assert.equal(ctx.el.usbSelectedControls.classList.contains("hidden"), true);
  assert.equal(exportUpdates, 1);
  assert.equal(ctx.el.usbInitRow.classList.contains("hidden"), true);
});

test("syncAssetScopePaths calls allow_asset_paths with the source folders and usb root", async () => {
  const state = { librarySources: { folders: [{ path: "/music" }] }, usbRoot: "/usb" };
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
