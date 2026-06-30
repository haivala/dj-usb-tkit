import test from "node:test";
import assert from "node:assert/strict";

if (typeof globalThis.window === "undefined") {
  globalThis.window = {};
}

import { createApiClient } from "../api_client.mjs";

function makeClient(overrides = {}) {
  const state = {
    playlists: [],
    tracks: [],
    libraryLoadedTotal: 0,
    mockPlayback: { path: null, playing: false, startedAtMs: 0, startOffsetMs: 0, durationMs: 240000 },
    ...overrides.state
  };
  const tauriInvoke = overrides.tauriInvoke || (() => { throw new Error("not tauri"); });
  const tauriIsTauri = overrides.tauriIsTauri || (() => false);
  const tauriListen = overrides.tauriListen || (() => {});
  const normalizePath = overrides.normalizePath || ((v) => String(v || "").trim().toLowerCase().replace(/\\/g, "/"));
  const constants = {
    LIBRARY_LOAD_LIMIT_DEFAULT: 200,
    LIBRARY_LOAD_LIMIT_POST_SCAN: 1000,
    ...overrides.constants
  };
  return { client: createApiClient({ tauriInvoke, tauriIsTauri, tauriListen, state, normalizePath, constants }), state };
}

test("isTauriRuntime returns false when tauriIsTauri returns false", () => {
  const { client } = makeClient({ tauriIsTauri: () => false });
  assert.equal(client.isTauriRuntime(), false);
});

test("isTauriRuntime returns true when tauriIsTauri returns true", () => {
  const { client } = makeClient({ tauriIsTauri: () => true });
  assert.equal(client.isTauriRuntime(), true);
});

test("isTauriRuntime returns false when tauriIsTauri throws", () => {
  const { client } = makeClient({ tauriIsTauri: () => { throw new Error("no window"); } });
  assert.equal(client.isTauriRuntime(), false);
});

test("invoke delegates to tauriInvoke in tauri runtime", async () => {
  const calls = [];
  const { client } = makeClient({
    tauriIsTauri: () => true,
    tauriInvoke: (cmd, payload) => { calls.push({ cmd, payload }); return { ok: true, data: "real" }; }
  });
  const result = await client.invoke("scan_library", { foo: 1 });
  assert.equal(result.data, "real");
  assert.equal(calls.length, 1);
  assert.equal(calls[0].cmd, "scan_library");
});

test("invoke returns mock for scan_library in non-tauri", async () => {
  const { client } = makeClient();
  const result = await client.invoke("scan_library");
  assert.equal(result.ok, true);
  assert.equal(result.data.jobId, "mock-scan");
});

test("invoke returns mock for search_tracks in non-tauri", async () => {
  const { client } = makeClient();
  const result = await client.invoke("search_tracks", { request: { query: "", limit: 10 } });
  assert.equal(result.ok, true);
  assert.equal(result.data.items.length, 3);
});

test("invoke search_tracks filters by query", async () => {
  const { client } = makeClient();
  const result = await client.invoke("search_tracks", { request: { query: "Track A", limit: 10 } });
  assert.equal(result.ok, true);
  assert.equal(result.data.items.length, 1);
  assert.equal(result.data.items[0].title, "Track A");
});

test("invoke mock browse_source_files scopes folder and master.db rows independently", async () => {
  const { client } = makeClient();

  const empty = await client.invoke("browse_source_files", {
    sourceRoots: [],
    includeMasterDb: false,
    query: "",
    limit: 10
  });
  assert.equal(empty.ok, true);
  assert.equal(empty.data.items.length, 0);

  const folderOnly = await client.invoke("browse_source_files", {
    sourceRoots: ["/music"],
    includeMasterDb: false,
    query: "",
    limit: 10
  });
  assert.equal(folderOnly.data.items.length, 3);
  assert.equal(folderOnly.data.items.some((track) => track.masterDbSource), false);

  const masterOnly = await client.invoke("browse_source_files", {
    sourceRoots: [],
    includeMasterDb: true,
    query: "",
    limit: 10
  });
  assert.deepEqual(masterOnly.data.items.map((track) => track.id), ["db-1"]);
  assert.equal(masterOnly.data.items[0].masterDbSource, true);
});

test("invoke returns mock for create_playlist and pushes to state", async () => {
  const { client, state } = makeClient();
  assert.equal(state.playlists.length, 0);
  const result = await client.invoke("create_playlist", { request: { name: "My Set" } });
  assert.equal(result.ok, true);
  assert.equal(result.data.name, "My Set");
  assert.equal(state.playlists.length, 1);
});

test("invoke returns mock for delete_playlist and removes from state", async () => {
  const { client, state } = makeClient({
    state: { playlists: [{ id: "p1", name: "A" }, { id: "p2", name: "B" }] }
  });
  const result = await client.invoke("delete_playlist", { request: { playlistId: "p1" } });
  assert.equal(result.ok, true);
  assert.equal(result.data.deleted, true);
  assert.equal(state.playlists.length, 1);
  assert.equal(state.playlists[0].id, "p2");
});

test("invoke returns mock for validate_usb_root", async () => {
  const { client } = makeClient();
  const valid = await client.invoke("validate_usb_root", { request: { path: "/media/usb" } });
  assert.equal(valid.ok, true);
  assert.equal(valid.data.valid, true);
  assert.equal(valid.data.normalizedRoot, "/media/usb");

  const empty = await client.invoke("validate_usb_root", { request: { path: "" } });
  assert.equal(empty.data.valid, false);
});

test("invoke mock play_track_native updates mockPlayback state", async () => {
  const { client, state } = makeClient();
  const result = await client.invoke("play_track_native", { request: { path: "/music/track.mp3" } });
  assert.equal(result.ok, true);
  assert.equal(result.data.playing, true);
  assert.equal(state.mockPlayback.playing, true);
  assert.equal(state.mockPlayback.path, "/music/track.mp3");
});

test("invoke mock stop_playback_native resets mockPlayback state", async () => {
  const { client, state } = makeClient({
    state: { mockPlayback: { path: "/music/track.mp3", playing: true, startedAtMs: 1000, startOffsetMs: 0, durationMs: 240000 } }
  });
  const result = await client.invoke("stop_playback_native", {});
  assert.equal(result.ok, true);
  assert.equal(result.data.stopped, true);
  assert.equal(state.mockPlayback.playing, false);
  assert.equal(state.mockPlayback.path, null);
});

test("invoke returns error for unknown mock command", async () => {
  const { client } = makeClient();
  const result = await client.invoke("nonexistent_command");
  assert.equal(result.ok, false);
  assert.equal(result.error.code, "INTERNAL_ERROR");
});

test("command unwraps successful response", async () => {
  const { client } = makeClient();
  const data = await client.command("search_tracks", { query: "", limit: 10 });
  assert.ok(Array.isArray(data.items));
  assert.equal(data.items.length, 3);
});

test("command throws on error response", async () => {
  const { client } = makeClient();
  await assert.rejects(
    () => client.command("nonexistent_command"),
    (err) => {
      assert.ok(err.message.includes("Unknown mock command"));
      return true;
    }
  );
});

test("invoke mock get_system_parallelism returns workers", async () => {
  const { client } = makeClient();
  const result = await client.invoke("get_system_parallelism");
  assert.equal(result.ok, true);
  assert.ok(result.data.workers >= 1);
});

test("invoke mock analyze_track_piece returns piece-specific data", async () => {
  const { client } = makeClient();
  const bpm = await client.invoke("analyze_track_piece", { request: { trackId: "t1", piece: "bpm_key" } });
  assert.equal(bpm.data.bpm, 124);
  assert.equal(bpm.data.key, "8A");

  const wf = await client.invoke("analyze_track_piece", { request: { trackId: "t1", piece: "waveform" } });
  assert.ok(Array.isArray(wf.data.waveformPreview));
  assert.ok(wf.data.waveformPeaksPath.includes("t1"));
});

test("invoke mock remove_tracks_from_playlist removes matching tracks", async () => {
  const playlist = { id: "p1", name: "Set", tracks: [{ id: "t1" }, { id: "t2" }, { id: "t3" }] };
  const { client } = makeClient({ state: { playlists: [playlist] } });
  const result = await client.invoke("remove_tracks_from_playlist", {
    request: { playlistId: "p1", trackIds: ["t1", "t3"] }
  });
  assert.equal(result.data.removed, 2);
  assert.equal(playlist.tracks.length, 1);
  assert.equal(playlist.tracks[0].id, "t2");
});
