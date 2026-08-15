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

test("removeUsbPlaylist clears the diagnostics report after a successful removal", async () => {
  const state = { usbRoot: "/tmp/usb" };
  let clearCalls = 0;

  await removeUsbPlaylist(state, { id: "pl-1", name: "USB Set" }, {
    setStatus: () => {},
    openConfirmDialog: async () => true,
    command: async () => ({ removedFromEdb: 1, removedFromPdb: 1, warnings: [] }),
    refreshUsb: async () => {},
    clearUsbDiagnostics: () => { clearCalls += 1; }
  });

  assert.equal(clearCalls, 1);
});

