import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  scheduleApplySearchLocalFilter
} from "../components/library/actions.mjs";
import {
  initializeUsb,
  pickUsbFolder
} from "../components/usb/actions.mjs";

test("scheduleApplySearchLocalFilter debounces and triggers reload", async () => {
  const dom = new JSDOM(`<!doctype html><body><input id="q" value="house" /></body>`);
  const state = { librarySearchDebounceTimer: null };
  const el = { librarySearch: dom.window.document.querySelector("#q") };
  let loaded = 0;
  scheduleApplySearchLocalFilter(state, el, {
    clearTimeoutFn: () => {},
    setTimeoutFn: (cb) => {
      cb();
      return 1;
    },
    resetAndLoadLibraryTracks: async (q) => {
      assert.equal(q, "house");
      loaded += 1;
    },
    setStatus: () => {},
    logError: () => {},
    debounceMs: 1
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(loaded, 1);
});

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
