import test from "node:test";
import assert from "node:assert/strict";
import { applyUsbRepairs } from "../components/usb/actions.mjs";

test("applyUsbRepairs does not clear playlists when nothing was applied", async () => {
  const state = { usbRoot: "/tmp/usb", selectedRepairFixIds: new Set(["some_fix"]) };
  let resetCalls = 0;
  let renderCalls = 0;
  let renderOpenPlaylist = 0;

  await applyUsbRepairs(state, {
    setStatus: () => {},
    command: async () => ({
      appliedFixes: [],
      failedFixes: ["some_fix"],
      warnings: [],
      durationMs: 1,
      diagnostics: { playlistDetails: [], warnings: [], durationMs: 1 }
    }),
    logWarnings: () => {},
    resetUsbStateViews: () => { resetCalls += 1; },
    updatePlaylistExportButtons: () => {},
    renderCurrentPlaylistTracksFromState: async () => { renderOpenPlaylist += 1; },
    renderDiagnosticsReport: () => { renderCalls += 1; }
  });

  assert.equal(resetCalls, 0);
  assert.equal(renderCalls, 1);
  assert.equal(renderOpenPlaylist, 1);
});
