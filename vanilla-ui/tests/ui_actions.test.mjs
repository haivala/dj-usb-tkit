import test from "node:test";
import assert from "node:assert/strict";
import {
  initializeUsb,
  pickUsbFolder
} from "../components/usb/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

// A ctx whose backend accepts `/usb` as a valid, named root; records every
// command/invoke. Auto-diagnostics scheduling is disabled.
function usbCtx(calls, overrides = {}) {
  return makeTestCtx({
    runUsbDiagnostics: async () => {},
    command: async (name, payload) => {
      calls.push([name, payload]);
      if (name === "validate_usb_root") {
        return { valid: true, normalizedRoot: payload.path, hasWriteAccess: true, hasVendorRoot: true, hasContents: true, hasPdb: true };
      }
      if (name === "get_usb_device_name") return { name: "Stick" };
      return {};
    },
    ...overrides
  });
}

test("initializeUsb initializes and revalidates root", async () => {
  const calls = [];
  const ctx = usbCtx(calls, { emitStatus: (text) => calls.push(["status", text]) });
  ctx.state.usbRoot = "/usb";
  await initializeUsb(ctx);
  assert.deepEqual(calls[0], ["initialize_usb", { usbRoot: "/usb" }]);
  assert.deepEqual(calls[1], ["status", "USB initialized"]);
  assert.deepEqual(calls[2], ["validate_usb_root", { path: "/usb" }]);
  assert.equal(ctx.state.usbRootValid, true);
});

test("pickUsbFolder invokes picker and validates selected path", async () => {
  const calls = [];
  const ctx = usbCtx(calls, {
    invoke: async (name) => {
      calls.push([name]);
      return name === "pick_usb_folder" ? "/usb" : null;
    }
  });
  const selected = await pickUsbFolder(ctx);
  assert.equal(selected, "/usb");
  assert.deepEqual(calls[0], ["pick_usb_folder"]);
  assert.deepEqual(calls[1], ["validate_usb_root", { path: "/usb" }]);
  assert.equal(ctx.state.usbRoot, "/usb");
});
