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

test("deletePlaylist confirms, deletes, refreshes and switches tab", async () => {
  let seenDialog = null;
  let deleteCalls = 0;
  let refreshed = 0;
  let switchedTo = null;
  let status = "";
  const state = {
    deletingPlaylistId: null,
    currentPlaylistId: "pl-1",
    playlists: [
      { id: "pl-1", name: "Main", lastExportedAt: "2026-01-01T00:00:00Z" },
      { id: "pl-2", name: "Other", lastExportedAt: null }
    ]
  };

  await deletePlaylist("pl-1", {
    state,
    openConfirmDialog: async (dialog) => {
      seenDialog = dialog;
      return true;
    },
    command: async (name, payload) => {
      assert.equal(name, "delete_playlist");
      assert.equal(payload.playlistId, "pl-1");
      deleteCalls += 1;
      return { deleted: true };
    },
    loadPlaylists: async () => {
      refreshed += 1;
      state.playlists = [{ id: "pl-2", name: "Other" }];
    },
    updateModeText: () => {},
    switchTab: async (tab) => { switchedTo = tab; },
    setStatus: (text) => { status = text; }
  });

  assert.equal(deleteCalls, 1);
  assert.equal(refreshed, 1);
  assert.equal(state.currentPlaylistId, "pl-2");
  assert.equal(switchedTo, "pl-2");
  assert.match(status, /Playlist deleted: Main/);
  assert.equal(seenDialog.confirmLabel, "Delete");
  assert.match(seenDialog.message, /easy to recreate by importing playlists from USB/i);
  assert.equal(state.deletingPlaylistId, null);
});
