import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  loadUsbRootFromStorage,
  resetUsbStateViews,
  syncAssetScopePaths,
  pickSourceFolders
} from "../components/usb/actions.mjs";

test("loadUsbRootFromStorage hydrates usb root and updates controls", () => {
  const dom = new JSDOM(`<!doctype html><body><div id="row"></div></body>`);
  const state = { usbRoot: null, usbRootValid: true, usbNeedsInit: true };
  const el = { usbInitRow: dom.window.document.querySelector("#row") };
  let rootText = null;
  let configUpdates = 0;
  let exportUpdates = 0;

  loadUsbRootFromStorage(state, el, {
    localStorageObj: { getItem: () => " /usb " },
    storageKeyUsbRoot: "k",
    updateUsbRootText: (path, valid) => { rootText = { path, valid }; },
    updateUsbConfigControlsVisibility: () => { configUpdates += 1; },
    updatePlaylistExportButtons: () => { exportUpdates += 1; }
  });

  assert.equal(state.usbRoot, "/usb");
  assert.equal(state.usbRootValid, false);
  assert.equal(state.usbNeedsInit, false);
  assert.deepEqual(rootText, { path: "/usb", valid: false });
  assert.equal(configUpdates, 1);
  assert.equal(exportUpdates, 1);
  assert.equal(el.usbInitRow.classList.contains("hidden"), true);
});

test("resetUsbStateViews clears lists and rerenders", () => {
  const state = {
    usbPlaylists: [{ id: 1 }],
    usbKnownPlaylistNames: new Set(["a"]),
    usbPlaylistTracks: [{ id: 1 }],
    usbPlaylistTracksView: [{ id: 1 }],
    histories: [{ id: 1 }],
    historyTracks: [{ id: 1 }],
    historyTracksView: [{ id: 1 }]
  };
  const el = {
    usbCountsText: { textContent: "x" },
    historyCountsText: { textContent: "y" },
    usbSelectedPlaylistText: { textContent: "z" },
    selectedHistoryText: { textContent: "w" }
  };
  let renders = 0;
  resetUsbStateViews(state, el, {
    renderUsbPlaylists: () => { renders += 1; },
    renderUsbPlaylistTracks: () => { renders += 1; },
    renderHistoryList: () => { renders += 1; },
    renderHistoryTracks: () => { renders += 1; }
  });
  assert.equal(state.usbPlaylists.length, 0);
  assert.equal(state.histories.length, 0);
  assert.equal(state.usbKnownPlaylistNames.size, 0);
  assert.equal(renders, 4);
});

test("syncAssetScopePaths calls allow_asset_paths with roots and usb root", async () => {
  const state = { sourceRoots: ["/music"], usbRoot: "/usb" };
  let called = null;
  await syncAssetScopePaths(state, {
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
