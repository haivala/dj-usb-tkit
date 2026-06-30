import test from "node:test";
import assert from "node:assert/strict";
import { removeUsbPlaylist } from "../components/usb/actions.mjs";

test("removeUsbPlaylist requires selected usb root", async () => {
  const state = { usbRoot: null };
  let status = "";

  await removeUsbPlaylist(state, { id: "pl-1", name: "USB Set" }, {
    setStatus: (text) => { status = text; },
    openConfirmDialog: async () => true,
    command: async () => ({ removedFromEdb: 0, removedFromPdb: 0, warnings: [] }),
    refreshUsb: async () => {}
  });

  assert.equal(status, "Select USB folder first");
});

test("removeUsbPlaylist stops when dialog is cancelled", async () => {
  const state = { usbRoot: "/tmp/usb" };
  let commandCalls = 0;

  await removeUsbPlaylist(state, { id: "pl-1", name: "USB Set" }, {
    setStatus: () => {},
    openConfirmDialog: async ({ title, confirmLabel }) => {
      assert.equal(title, "Remove USB Playlist");
      assert.equal(confirmLabel, "Remove");
      return false;
    },
    command: async () => {
      commandCalls += 1;
      return { removedFromEdb: 0, removedFromPdb: 0, warnings: [] };
    },
    refreshUsb: async () => {}
  });

  assert.equal(commandCalls, 0);
});

test("removeUsbPlaylist confirms, executes remove and reports status", async () => {
  const state = { usbRoot: "/tmp/usb" };
  let refreshed = 0;
  let status = "";
  let payload = null;

  await removeUsbPlaylist(state, { id: "pl-9", name: "Night Set" }, {
    setStatus: (text) => { status = text; },
    openConfirmDialog: async () => true,
    command: async (name, data) => {
      assert.equal(name, "remove_usb_playlist");
      payload = data;
      return {
        removedFromEdb: 2,
        removedFromPdb: 2,
        warnings: ["one warning"]
      };
    },
    refreshUsb: async () => { refreshed += 1; }
  });

  assert.deepEqual(payload, {
    usbRoot: "/tmp/usb",
    playlistId: "pl-9",
    playlistName: "Night Set"
  });
  assert.equal(refreshed, 1);
  assert.match(status, /Removed USB playlist: Night Set/);
  assert.match(status, /\(1 warning\(s\)\)/);
});
