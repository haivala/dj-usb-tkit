import test from "node:test";
import assert from "node:assert/strict";
import { addUsbPlayerMenuItems, syncUsbPlayerMenusEdbToPdb } from "../components/usb/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

// A ctx showing a stale "WARN" diagnostics report; the player-menu command
// answers `{ updated }`.
function menuCtx(updated) {
  const ctx = makeTestCtx({
    command: async () => ({ updated, currentItems: [{ kind: 131 }], availableItems: [{ kind: 132 }] })
  });
  Object.assign(ctx.state, {
    usbRoot: "/tmp/usb",
    usbRootValid: true,
    usbPlayerMenuCurrent: [{ kind: 131 }],
    usbPlayerMenuAvailable: [{ kind: 132 }],
    usbPlayerMenuAvailableSelectedKind: 132
  });
  ctx.el.diagOverallStatus.textContent = "WARN";
  return ctx;
}

const reportCleared = (ctx) => ctx.el.diagOverallStatus.textContent === "";

test("syncUsbPlayerMenusEdbToPdb clears diagnostics only when the PDB was actually updated", async () => {
  const changed = menuCtx(true);
  await syncUsbPlayerMenusEdbToPdb(changed);
  assert.equal(reportCleared(changed), true);

  const unchanged = menuCtx(false);
  await syncUsbPlayerMenusEdbToPdb(unchanged);
  assert.equal(reportCleared(unchanged), false);
});

test("a player-menu config update clears diagnostics only when the config was actually updated", async () => {
  const changed = menuCtx(true);
  await addUsbPlayerMenuItems(changed);
  assert.equal(reportCleared(changed), true);

  const unchanged = menuCtx(false);
  await addUsbPlayerMenuItems(unchanged);
  assert.equal(reportCleared(unchanged), false);
});
