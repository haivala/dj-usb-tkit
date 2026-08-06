import test from "node:test";
import assert from "node:assert/strict";
import { deletePlaylist } from "../components/playlist/actions.mjs";

test("deletePlaylist skips when playlist not found in state", async () => {
  let commandCalled = false;
  const state = { playlists: [], deletingPlaylistId: null };

  await deletePlaylist("missing-id", {
    state,
    openConfirmDialog: async () => true,
    command: async () => { commandCalled = true; return { deleted: true }; },
    loadPlaylists: async () => {},
    updateModeText: () => {},
    switchTab: async () => {},
    setStatus: () => {}
  });

  assert.ok(!commandCalled, "should not call backend for nonexistent playlist");
});

