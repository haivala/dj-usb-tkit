import test from "node:test";
import assert from "node:assert/strict";
import {
  loadUsbRootFromStorage,
  clearUsbPlaylistsView,
  usbContentChanged,
  usbSelectionChanged,
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

test("usbSelectionChanged clears lists and rerenders", () => {
  const ctx = makeTestCtx();
  Object.assign(ctx.state, {
    usbPlaylists: [{ id: 1, name: "One" }],
    playlistUsbExportStatusById: new Map([["p1", { sameNameExistsOnUsb: true, locksReorder: false }]]),
    histories: [{ id: 1, name: "Session" }],
    historyTracks: [{ id: 1 }]
  });
  ctx.el.usbCountsText.textContent = "x";
  ctx.el.usbPlaylists.dataset.loaded = "";
  ctx.el.historyList.dataset.loaded = "";

  usbSelectionChanged(ctx);

  assert.equal(ctx.state.usbPlaylists.length, 0);
  assert.equal(ctx.state.histories.length, 0);
  assert.equal(ctx.state.playlistUsbExportStatusById.size, 0);
  assert.equal(ctx.el.usbCountsText.textContent, "");
  // Nothing is read yet, so the lists show nothing rather than "no playlists".
  assert.equal(ctx.el.usbPlaylists.textContent, "");
  assert.equal(ctx.el.historyList.textContent, "");
  assert.equal("loaded" in ctx.el.usbPlaylists.dataset, false);
  assert.equal("loaded" in ctx.el.historyList.dataset, false);
  assert.equal(ctx.el.usbDiagnosticsCard.classList.contains("hidden"), true);
});

test("usbContentChanged clears the lists but keeps the export status and the diagnostics panel", () => {
  const ctx = makeTestCtx();
  const status = new Map([["p1", { sameNameExistsOnUsb: true, locksReorder: true }]]);
  Object.assign(ctx.state, {
    usbPlaylists: [{ id: 1, name: "One" }],
    playlistUsbExportStatusById: status,
    histories: [{ id: 1, name: "Session" }],
    usbBackgroundColor: { color: "red" }
  });
  ctx.el.usbDiagnosticsCard.classList.remove("hidden");

  usbContentChanged(ctx);

  assert.equal(ctx.state.usbPlaylists.length, 0);
  assert.equal(ctx.state.histories.length, 0);
  assert.equal(ctx.state.usbBackgroundColor, null);
  assert.equal(ctx.state.playlistUsbExportStatusById, status);
  assert.equal(ctx.el.usbDiagnosticsCard.classList.contains("hidden"), false);
});

test("clearUsbPlaylistsView clears the playlists and their totals, and leaves the history", () => {
  const ctx = makeTestCtx();
  Object.assign(ctx.state, {
    usbPlaylists: [{ id: 1, name: "One" }],
    histories: [{ id: 1, name: "Session" }]
  });
  ctx.el.usbCountsText.textContent = "1 playlist, 3 tracks";
  ctx.el.historyCountsText.textContent = "1 session, 3 tracks";
  ctx.el.usbPlaylists.dataset.loaded = "";
  ctx.el.historyList.dataset.loaded = "";

  clearUsbPlaylistsView(ctx);

  assert.equal(ctx.state.usbPlaylists.length, 0);
  assert.equal(ctx.el.usbCountsText.textContent, "");
  assert.equal(ctx.state.histories.length, 1);
  assert.equal(ctx.el.historyCountsText.textContent, "1 session, 3 tracks");
  assert.equal("loaded" in ctx.el.usbPlaylists.dataset, false);
  assert.equal("loaded" in ctx.el.historyList.dataset, true);
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
