import test from "node:test";
import assert from "node:assert/strict";

import { deleteUsbBackup } from "../components/backups/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

// The real app context with every collaborator delete touches as a no-op; the
// post-action list reload renders into the real backups list.
function backupCtx(state, overrides = {}) {
  return makeTestCtx({
    state,
    emitStatus: () => {},
    ...overrides
  });
}

test("deleteUsbBackup removes the entry without touching the diagnostics report", async () => {
  const state = { usbRoot: "/usb", usbBackups: [{ timestamp: "2020-01-01_00-00-00", files: [] }] };
  const calls = [];
  await deleteUsbBackup(backupCtx(state, {
    command: async (name, args) => { calls.push({ name, args }); return {}; },
    openConfirmDialog: async () => true
  }), "2020-01-01_00-00-00");
  // The delete, then the list reload.
  assert.deepEqual(calls.map((c) => c.name), ["delete_usb_backup", "list_usb_backups"]);
  assert.equal(calls[0].args.timestamp, "2020-01-01_00-00-00");
});
