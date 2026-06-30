import test from "node:test";
import assert from "node:assert/strict";
import { deletePlaylist, addTracksToCurrentPlaylist } from "../components/playlist/actions.mjs";

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

test("deletePlaylist respects cancel from confirm dialog", async () => {
  let commandCalled = false;
  const state = {
    playlists: [{ id: "pl-1", name: "Set", lastExportedAt: null }],
    deletingPlaylistId: null
  };

  await deletePlaylist("pl-1", {
    state,
    openConfirmDialog: async () => false,
    command: async () => { commandCalled = true; return { deleted: true }; },
    loadPlaylists: async () => {},
    updateModeText: () => {},
    switchTab: async () => {},
    setStatus: () => {}
  });

  assert.ok(!commandCalled, "should abort when user cancels");
  assert.equal(state.deletingPlaylistId, null,
    "should not leave deletingPlaylistId set after cancel");
});

test("deletePlaylist reloads and switches tab on success", async () => {
  const calls = [];
  const state = {
    playlists: [{ id: "pl-1", name: "Set", lastExportedAt: null }],
    deletingPlaylistId: null,
    currentPlaylistId: "pl-1"
  };

  await deletePlaylist("pl-1", {
    state,
    openConfirmDialog: async () => true,
    command: async () => { calls.push("command"); return { deleted: true }; },
    loadPlaylists: async () => {
      calls.push("loadPlaylists");
      state.playlists = [];
    },
    updateModeText: () => { calls.push("updateModeText"); },
    switchTab: async (tab) => { calls.push(`switchTab:${tab}`); },
    setStatus: (msg) => { calls.push("setStatus"); }
  });

  assert.ok(calls.includes("command"));
  assert.ok(calls.includes("loadPlaylists"));
  assert.ok(calls.includes("updateModeText"));
  assert.ok(calls.some((c) => c.startsWith("switchTab:")),
    "should switch to a different tab after deletion");
  assert.equal(state.deletingPlaylistId, null,
    "deletingPlaylistId should be cleared in finally block");
});

test("addTracksToCurrentPlaylist reports status with added/skipped counts", async () => {
  let statusMsg = "";
  const playlist = { id: "pl-1", name: "Set" };

  await addTracksToCurrentPlaylist(
    [{ id: "t1" }, { id: "t2" }, { id: "t3" }],
    {
      requireCurrentPlaylist: () => playlist,
      resolveLocalTrackId: (track) => track.id,
      setStatus: (msg) => { statusMsg = msg; },
      withProgress: async (_label, fn) => {
        return fn(() => {});
      },
      command: async () => ({ added: 2, skipped: 1 }),
      refreshCurrentPlaylistTracks: async () => {}
    }
  );

  assert.match(statusMsg, /2/, "should mention added count");
  assert.match(statusMsg, /1/, "should mention skipped count");
  assert.match(statusMsg, /Set/, "should mention playlist name");
});

test("addTracksToCurrentPlaylist bails when no playlist is active", async () => {
  let commandCalled = false;

  await addTracksToCurrentPlaylist(
    [{ id: "t1" }],
    {
      requireCurrentPlaylist: () => null,
      resolveLocalTrackId: (track) => track.id,
      setStatus: () => {},
      withProgress: async (_label, fn) => fn(() => {}),
      command: async () => { commandCalled = true; return { added: 0, skipped: 0 }; },
      refreshCurrentPlaylistTracks: async () => {}
    }
  );

  assert.ok(!commandCalled, "should not invoke backend without active playlist");
});
