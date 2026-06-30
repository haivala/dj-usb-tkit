import test from "node:test";
import assert from "node:assert/strict";
import { updateUsbConfigControlsVisibility } from "../components/usb/actions.mjs";

function makeClassList() {
  const classes = new Set();
  return {
    add(name) { classes.add(name); },
    remove(name) { classes.delete(name); },
    toggle(name, force) {
      if (force) classes.add(name);
      else classes.delete(name);
    },
    contains(name) { return classes.has(name); }
  };
}

test("updateUsbConfigControlsVisibility hides controls and diagnostics without valid root", () => {
  const state = { usbRoot: "/tmp/usb", usbRootValid: false };
  const el = {
    usbSelectedControls: { classList: makeClassList() },
    usbDiagnosticsCard: { classList: makeClassList() }
  };

  updateUsbConfigControlsVisibility(state, el);

  assert.equal(el.usbSelectedControls.classList.contains("hidden"), true);
  assert.equal(el.usbDiagnosticsCard.classList.contains("hidden"), true);
});

test("updateUsbConfigControlsVisibility shows controls with valid root", () => {
  const state = { usbRoot: "/tmp/usb", usbRootValid: true };
  const el = {
    usbSelectedControls: { classList: makeClassList() },
    usbDiagnosticsCard: { classList: makeClassList() }
  };

  el.usbSelectedControls.classList.add("hidden");
  updateUsbConfigControlsVisibility(state, el);

  assert.equal(el.usbSelectedControls.classList.contains("hidden"), false);
  assert.equal(el.usbDiagnosticsCard.classList.contains("hidden"), false);
});
