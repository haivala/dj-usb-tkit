import test from "node:test";
import assert from "node:assert/strict";

import { restoreUsbBackup, deleteUsbBackup } from "../components/backups/actions.mjs";

test("restoreUsbBackup clears the on-screen diagnostics report after a successful restore", async () => {
  const state = { usbRoot: "/usb", usbBackups: [] };
  let clearCalls = 0;
  await restoreUsbBackup(state, "2020-01-01_00-00-00", {
    command: async () => ({}),
    openConfirmDialog: async () => true,
    clearUsbDiagnostics: () => { clearCalls += 1; },
    reload: async () => {}
  });
  assert.equal(clearCalls, 1, "a successful restore must clear the stale diagnostics report");
});

test("restoreUsbBackup does not clear diagnostics when the restore command fails", async () => {
  const state = { usbRoot: "/usb", usbBackups: [] };
  let clearCalls = 0;
  await restoreUsbBackup(state, "2020-01-01_00-00-00", {
    command: async () => { throw new Error("boom"); },
    openConfirmDialog: async () => true,
    clearUsbDiagnostics: () => { clearCalls += 1; },
    reload: async () => {}
  });
  assert.equal(clearCalls, 0, "a failed restore left the live files untouched, so the report is still valid");
});

test("restoreUsbBackup does nothing when the user cancels the confirm dialog", async () => {
  const state = { usbRoot: "/usb", usbBackups: [] };
  let commandCalls = 0;
  let clearCalls = 0;
  await restoreUsbBackup(state, "2020-01-01_00-00-00", {
    command: async () => { commandCalls += 1; return {}; },
    openConfirmDialog: async () => false,
    clearUsbDiagnostics: () => { clearCalls += 1; },
    reload: async () => {}
  });
  assert.equal(commandCalls, 0);
  assert.equal(clearCalls, 0);
});

test("deleteUsbBackup removes the entry without touching the diagnostics report", async () => {
  const state = { usbRoot: "/usb", usbBackups: [{ timestamp: "2020-01-01_00-00-00", files: [] }] };
  let commandArgs = null;
  await deleteUsbBackup(state, "2020-01-01_00-00-00", {
    command: async (name, args) => { commandArgs = { name, args }; return {}; },
    openConfirmDialog: async () => true,
    reload: async () => {}
  });
  assert.equal(commandArgs.name, "delete_usb_backup");
  assert.equal(commandArgs.args.timestamp, "2020-01-01_00-00-00");
});
