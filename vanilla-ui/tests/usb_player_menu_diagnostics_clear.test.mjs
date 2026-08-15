import test from "node:test";
import assert from "node:assert/strict";
import { syncUsbPlayerMenusEdbToPdb, updateUsbPlayerMenuConfig } from "../components/usb/actions.mjs";

test("syncUsbPlayerMenusEdbToPdb clears diagnostics only when the PDB was actually updated", async () => {
  const state = { usbRoot: "/tmp/usb", usbRootValid: true };
  let clearCalls = 0;

  await syncUsbPlayerMenusEdbToPdb(state, {}, {
    setStatus: () => {},
    command: async () => ({ updated: true, currentItems: [], availableItems: [] }),
    clearUsbDiagnostics: () => { clearCalls += 1; },
    documentObj: {}
  });

  assert.equal(clearCalls, 1);
});

test("syncUsbPlayerMenusEdbToPdb does not clear diagnostics when nothing changed", async () => {
  const state = { usbRoot: "/tmp/usb", usbRootValid: true };
  let clearCalls = 0;

  await syncUsbPlayerMenusEdbToPdb(state, {}, {
    setStatus: () => {},
    command: async () => ({ updated: false, currentItems: [], availableItems: [] }),
    clearUsbDiagnostics: () => { clearCalls += 1; },
    documentObj: {}
  });

  assert.equal(clearCalls, 0);
});

test("updateUsbPlayerMenuConfig clears diagnostics only when the config was actually updated", async () => {
  const state = { usbRoot: "/tmp/usb", usbRootValid: true };
  let clearCalls = 0;

  await updateUsbPlayerMenuConfig(state, {}, {
    setStatus: () => {},
    command: async () => ({ updated: true, currentItems: [], availableItems: [] }),
    clearUsbDiagnostics: () => { clearCalls += 1; },
    documentObj: {}
  }, [131, 132]);

  assert.equal(clearCalls, 1);
});

test("updateUsbPlayerMenuConfig does not clear diagnostics when nothing changed", async () => {
  const state = { usbRoot: "/tmp/usb", usbRootValid: true };
  let clearCalls = 0;

  await updateUsbPlayerMenuConfig(state, {}, {
    setStatus: () => {},
    command: async () => ({ updated: false, currentItems: [], availableItems: [] }),
    clearUsbDiagnostics: () => { clearCalls += 1; },
    documentObj: {}
  }, [131, 132]);

  assert.equal(clearCalls, 0);
});
