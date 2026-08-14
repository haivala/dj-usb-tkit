import test from "node:test";
import assert from "node:assert/strict";

import { restoreUsbBackup, deleteUsbBackup } from "../components/backups/actions.mjs";

test("restoreUsbBackup discards the on-screen diagnostics report after a successful restore", async () => {
  const state = { usbRoot: "/usb", usbBackups: [] };
  let hideCalls = 0;
  await restoreUsbBackup(state, "2020-01-01_00-00-00", {
    command: async () => ({}),
    openConfirmDialog: async () => true,
    hideUsbDiagnostics: () => { hideCalls += 1; },
    reload: async () => {}
  });
  assert.equal(hideCalls, 1, "a successful restore must discard the stale diagnostics report");
});

test("restoreUsbBackup does not discard diagnostics when the restore command fails", async () => {
  const state = { usbRoot: "/usb", usbBackups: [] };
  let hideCalls = 0;
  await restoreUsbBackup(state, "2020-01-01_00-00-00", {
    command: async () => { throw new Error("boom"); },
    openConfirmDialog: async () => true,
    hideUsbDiagnostics: () => { hideCalls += 1; },
    reload: async () => {}
  });
  assert.equal(hideCalls, 0, "a failed restore left the live files untouched, so the report is still valid");
});

test("restoreUsbBackup does nothing when the user cancels the confirm dialog", async () => {
  const state = { usbRoot: "/usb", usbBackups: [] };
  let commandCalls = 0;
  let hideCalls = 0;
  await restoreUsbBackup(state, "2020-01-01_00-00-00", {
    command: async () => { commandCalls += 1; return {}; },
    openConfirmDialog: async () => false,
    hideUsbDiagnostics: () => { hideCalls += 1; },
    reload: async () => {}
  });
  assert.equal(commandCalls, 0);
  assert.equal(hideCalls, 0);
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
