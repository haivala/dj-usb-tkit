import test from "node:test";
import assert from "node:assert/strict";
import { deletePlaylist } from "../components/playlist/actions.mjs";

test("deletePlaylist aborts when confirmation is declined", async () => {
  let commandCalled = false;
  const state = {
    deletingPlaylistId: null,
    currentPlaylistId: "pl-1",
    playlists: [{ id: "pl-1", name: "Main", lastExportedAt: null }]
  };

  await deletePlaylist("pl-1", {
    state,
    openConfirmDialog: async ({ title }) => {
      assert.equal(title, "Delete App Playlist");
      return false;
    },
    command: async () => {
      commandCalled = true;
      return { deleted: true };
    },
    loadPlaylists: async () => {},
    updateModeText: () => {},
    switchTab: async () => {},
    setStatus: () => {}
  });

  assert.equal(commandCalled, false);
  assert.equal(state.deletingPlaylistId, null);
});
