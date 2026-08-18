import test from "node:test";
import assert from "node:assert/strict";
import { reorderUsbPlaylists, moveArrayItem } from "../components/usb/actions.mjs";

test("moveArrayItem moves an item from the start to the end", () => {
  assert.deepEqual(moveArrayItem(["a", "b", "c"], 0, 2), ["b", "c", "a"]);
});

test("moveArrayItem moves an item from the end to the start", () => {
  assert.deepEqual(moveArrayItem(["a", "b", "c"], 2, 0), ["c", "a", "b"]);
});

test("moveArrayItem moves an item by one position", () => {
  assert.deepEqual(moveArrayItem(["a", "b", "c"], 0, 1), ["b", "a", "c"]);
});

test("moveArrayItem is a no-op when fromIndex equals toIndex", () => {
  assert.deepEqual(moveArrayItem(["a", "b", "c"], 1, 1), ["a", "b", "c"]);
});

test("moveArrayItem handles a single-item list", () => {
  assert.deepEqual(moveArrayItem(["only"], 0, 0), ["only"]);
});

test("moveArrayItem does not mutate the input list", () => {
  const input = ["a", "b", "c"];
  moveArrayItem(input, 0, 2);
  assert.deepEqual(input, ["a", "b", "c"]);
});

test("reorderUsbPlaylists requires selected usb root", async () => {
  const state = { usbRoot: null, usbPlaylists: [{ id: "pl-1" }] };
  let status = "";
  let commandCalls = 0;

  await reorderUsbPlaylists(state, {}, {
    setStatus: (text) => { status = text; },
    command: async () => { commandCalls += 1; },
    refreshUsb: async () => {}
  });

  assert.equal(status, "Select USB folder first");
  assert.equal(commandCalls, 0);
});

test("reorderUsbPlaylists surfaces an error status and still refreshes", async () => {
  const state = {
    usbRoot: "/tmp/usb",
    usbRootValid: true,
    usbPlaylists: [{ id: "usb-pl-1" }, { id: "usb-pl-2" }]
  };
  let status = "";
  let refreshed = 0;

  await reorderUsbPlaylists(state, {}, {
    setStatus: (text) => { status = text; },
    command: async () => { throw new Error("write failed"); },
    refreshUsb: async () => { refreshed += 1; }
  });

  assert.match(status, /Failed to save playlist order: write failed/);
  assert.equal(refreshed, 1);
});

test("reorderUsbPlaylists does not clear diagnostics when the command fails", async () => {
  const state = {
    usbRoot: "/tmp/usb",
    usbRootValid: true,
    usbPlaylists: [{ id: "usb-pl-1" }, { id: "usb-pl-2" }]
  };
  let clearCalls = 0;

  await reorderUsbPlaylists(state, {}, {
    setStatus: () => {},
    command: async () => { throw new Error("write failed"); },
    refreshUsb: async () => {},
    clearUsbDiagnostics: () => { clearCalls += 1; }
  });

  assert.equal(clearCalls, 0);
});
