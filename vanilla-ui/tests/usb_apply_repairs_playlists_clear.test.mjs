import test from "node:test";
import assert from "node:assert/strict";
import { applyUsbRepairs } from "../components/usb/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

test("applyUsbRepairs does not clear playlists when nothing was applied", async () => {
  let renderOpenPlaylist = 0;
  const ctx = makeTestCtx({
    command: async () => ({
      appliedFixes: [],
      failedFixes: ["some_fix"],
      warnings: [],
      durationMs: 1,
      diagnostics: { overallStatus: "WARN", playlistDetails: [], warnings: [], durationMs: 1 }
    }),
    logWarnings: () => {},
    renderCurrentPlaylistTracksFromState: async () => { renderOpenPlaylist += 1; }
  });
  Object.assign(ctx.state, {
    usbRoot: "/tmp/usb",
    selectedRepairFixIds: new Set(["some_fix"]),
    usbPlaylists: [{ id: "u1", name: "Loaded" }]
  });

  await applyUsbRepairs(ctx);

  assert.deepEqual(ctx.state.usbPlaylists.map((p) => p.id), ["u1"], "loaded USB playlists are kept");
  assert.equal(ctx.el.diagOverallStatus.textContent, "WARN", "the refreshed diagnostics report is rendered");
  assert.equal(renderOpenPlaylist, 1);
});
