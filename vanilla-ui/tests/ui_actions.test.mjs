import test from "node:test";
import assert from "node:assert/strict";
import {
  initializeUsb,
  pickUsbFolder
} from "../components/usb/actions.mjs";

// NOTE: library search debouncing moved to a main.js closure over
// libraryTracksCtl.setSearch (shared TrackListController) -- see
// tests/track_list_controller.test.mjs and the e2e library specs.

test("initializeUsb initializes and revalidates root", async () => {
  const state = { usbRoot: "/usb" };
  const el = { usbInitRow: { classList: { add: () => {} } } };
  const calls = [];
  await initializeUsb(state, el, {
    command: async (name, payload) => {
      calls.push([name, payload]);
    },
    setStatus: (text) => calls.push(["status", text]),
    validateAndSetUsbRoot: async (path, silent) => calls.push(["validate", path, silent]),
    logError: () => {}
  });
  assert.equal(calls[0][0], "initialize_usb");
  assert.equal(calls[1][0], "status");
  assert.equal(calls[2][0], "validate");
});

test("pickUsbFolder invokes picker and validates selected path", async () => {
  const calls = [];
  const selected = await pickUsbFolder({
    invoke: async (name) => {
      calls.push(name);
      return "/usb";
    },
    validateAndSetUsbRoot: async (path, silent) => calls.push([path, silent])
  });
  assert.equal(selected, "/usb");
  assert.equal(calls[0], "pick_usb_folder");
  assert.deepEqual(calls[1], ["/usb", false]);
});
