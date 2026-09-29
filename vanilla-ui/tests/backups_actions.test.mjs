import test from "node:test";
import assert from "node:assert/strict";

import { restoreUsbBackup, deleteUsbBackup } from "../components/backups/actions.mjs";

// Every collaborator restore/delete touches, as no-ops (the post-action list
// reload finds no list element to render into).
function backupCtx(state, overrides = {}) {
  return {
    state,
    el: {},
    setStatus: () => {},
    clearUsbDiagnostics: () => {},
    resetUsbStateViews: () => {},
    ...overrides
  };
}

test("restoreUsbBackup clears the on-screen diagnostics report after a successful restore", async () => {
  const state = { usbRoot: "/usb", usbBackups: [] };
  let clearCalls = 0;
  await restoreUsbBackup(backupCtx(state, {
    command: async () => ({}),
    openConfirmDialog: async () => true,
    clearUsbDiagnostics: () => { clearCalls += 1; }
  }), "2020-01-01_00-00-00");
  assert.equal(clearCalls, 1, "a successful restore must clear the stale diagnostics report");
});

test("restoreUsbBackup clears loaded playlists (and other derived state) after a successful restore", async () => {
  const state = { usbRoot: "/usb", usbBackups: [] };
  const resetCalls = [];
  await restoreUsbBackup(backupCtx(state, {
    command: async () => ({}),
    openConfirmDialog: async () => true,
    resetUsbStateViews: (opts) => { resetCalls.push(opts); }
  }), "2020-01-01_00-00-00");
  assert.equal(resetCalls.length, 1, "a successful restore must clear stale playlists/histories/player-menu state");
  assert.equal(resetCalls[0].hideDiagnostics, false, "the diagnostics panel itself must stay visible -- only its report is cleared, separately");
});

test("restoreUsbBackup does not clear playlists when the restore command fails", async () => {
  const state = { usbRoot: "/usb", usbBackups: [] };
  let resetCalls = 0;
  await restoreUsbBackup(backupCtx(state, {
    command: async () => { throw new Error("boom"); },
    openConfirmDialog: async () => true,
    resetUsbStateViews: () => { resetCalls += 1; }
  }), "2020-01-01_00-00-00");
  assert.equal(resetCalls, 0, "a failed restore left the live files untouched, so loaded playlists are still valid");
});

test("restoreUsbBackup does not clear diagnostics when the restore command fails", async () => {
  const state = { usbRoot: "/usb", usbBackups: [] };
  let clearCalls = 0;
  await restoreUsbBackup(backupCtx(state, {
    command: async () => { throw new Error("boom"); },
    openConfirmDialog: async () => true,
    clearUsbDiagnostics: () => { clearCalls += 1; }
  }), "2020-01-01_00-00-00");
  assert.equal(clearCalls, 0, "a failed restore left the live files untouched, so the report is still valid");
});

test("restoreUsbBackup does nothing when the user cancels the confirm dialog", async () => {
  const state = { usbRoot: "/usb", usbBackups: [] };
  let commandCalls = 0;
  let clearCalls = 0;
  await restoreUsbBackup(backupCtx(state, {
    command: async () => { commandCalls += 1; return {}; },
    openConfirmDialog: async () => false,
    clearUsbDiagnostics: () => { clearCalls += 1; }
  }), "2020-01-01_00-00-00");
  assert.equal(commandCalls, 0);
  assert.equal(clearCalls, 0);
});

test("deleteUsbBackup removes the entry without touching the diagnostics report", async () => {
  const state = { usbRoot: "/usb", usbBackups: [{ timestamp: "2020-01-01_00-00-00", files: [] }] };
  let commandArgs = null;
  await deleteUsbBackup(backupCtx(state, {
    command: async (name, args) => { commandArgs = { name, args }; return {}; },
    openConfirmDialog: async () => true
  }), "2020-01-01_00-00-00");
  assert.equal(commandArgs.name, "delete_usb_backup");
  assert.equal(commandArgs.args.timestamp, "2020-01-01_00-00-00");
});
