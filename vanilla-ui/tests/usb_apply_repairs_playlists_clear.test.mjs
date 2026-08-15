import test from "node:test";
import assert from "node:assert/strict";
import { applyUsbRepairs } from "../components/usb/actions.mjs";

test("applyUsbRepairs clears loaded playlists only when a fix was actually applied", async () => {
  const state = { usbRoot: "/tmp/usb", selectedRepairFixIds: new Set(["repair_pdb_wrong_playlist_tree_shape"]) };
  const resetCalls = [];

  await applyUsbRepairs(state, {
    setStatus: () => {},
    command: async () => ({ appliedFixes: ["repair_pdb_wrong_playlist_tree_shape"], failedFixes: [], warnings: [], durationMs: 1 }),
    logWarnings: () => {},
    runUsbDiagnostics: async () => {},
    resetUsbStateViews: (opts) => { resetCalls.push(opts); }
  });

  assert.equal(resetCalls.length, 1, "an applied fix may have changed playlist data -- loaded playlists must be cleared");
  assert.equal(resetCalls[0].hideDiagnostics, false, "the diagnostics panel stays visible -- it's about to be repopulated by re-diagnosing");
});

test("applyUsbRepairs does not clear playlists when nothing was applied", async () => {
  const state = { usbRoot: "/tmp/usb", selectedRepairFixIds: new Set(["some_fix"]) };
  let resetCalls = 0;

  await applyUsbRepairs(state, {
    setStatus: () => {},
    command: async () => ({ appliedFixes: [], failedFixes: ["some_fix"], warnings: [], durationMs: 1 }),
    logWarnings: () => {},
    runUsbDiagnostics: async () => {},
    resetUsbStateViews: () => { resetCalls += 1; }
  });

  assert.equal(resetCalls, 0);
});
