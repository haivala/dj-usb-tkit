import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  validateAndSetUsbRoot
} from "../components/usb/actions.mjs";

test("jsdom: validate usb root wiring exposes initialize UI then enables controls after success", async () => {
  const dom = new JSDOM(`
    <div id="usbSelectedControls" class="hidden"></div>
    <div id="usbDiagnosticsCard" class="hidden"></div>
    <div id="usbInitRow" class="hidden"></div>
    <div id="usbInitHint"></div>
    <button id="initializeUsbBtn"></button>
  `);
  const { document } = dom.window;
  const state = { usbRoot: null, usbRootValid: false, usbNeedsInit: false, usbWritable: false };
  const el = {
    usbSelectedControls: document.getElementById("usbSelectedControls"),
    usbDiagnosticsCard: document.getElementById("usbDiagnosticsCard"),
    usbInitRow: document.getElementById("usbInitRow"),
    usbInitHint: document.getElementById("usbInitHint"),
    initializeUsbBtn: document.getElementById("initializeUsbBtn")
  };

  let lastStatus = "";
  let diagnosticsRan = false;

  const deps = {
    command: async () => ({
      valid: false,
      hasWriteAccess: true,
      normalizedRoot: "/USB",
      hasVendorRoot: false,
      hasContents: false,
      hasPdb: false,
      warnings: ["missing External library structure"]
    }),
    persistUsbRoot: () => {},
    updateUsbRootText: () => {},
    resetUsbStateViews: () => {},
    updateUsbConfigControlsVisibility: () => {
      const hasValid = !!state.usbRoot && !!state.usbRootValid;
      el.usbSelectedControls.classList.toggle("hidden", !hasValid);
    },
    updateUsbSubNavDisabledState: () => {},
    updatePlaylistExportButtons: () => {},
    setStatus: (text) => { lastStatus = text; },
    runUsbDiagnostics: async () => { diagnosticsRan = true; },
    warn: () => {},
    scheduler: (fn) => fn()
  };

  // Phase 1: writable but uninitialized USB
  await validateAndSetUsbRoot(state, el, "/USB", false, deps);

  assert.ok(state.usbNeedsInit, "should flag as needing init");
  assert.ok(!el.usbInitRow.classList.contains("hidden"),
    "init row should be visible for writable uninit USB");
  assert.ok(!el.initializeUsbBtn.disabled,
    "init button should be enabled");
  assert.match(lastStatus, /not initialized/i,
    "status should mention initialization needed");
  assert.ok(!diagnosticsRan, "diagnostics should not run for invalid USB");

  // Phase 2: now pretend USB is valid
  deps.command = async () => ({
    valid: true,
    hasWriteAccess: true,
    normalizedRoot: "/USB",
    hasVendorRoot: true,
    hasContents: true,
    hasPdb: true,
    hasEdb: true,
    warnings: []
  });

  await validateAndSetUsbRoot(state, el, "/USB", false, deps);

  assert.ok(state.usbRootValid, "USB should now be valid");
  assert.ok(el.usbInitRow.classList.contains("hidden"),
    "init row should hide after successful validation");
  assert.ok(!el.usbSelectedControls.classList.contains("hidden"),
    "controls should be visible for valid USB");
  assert.ok(diagnosticsRan, "diagnostics should auto-run for valid USB");
});
