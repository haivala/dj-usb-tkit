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
  const constants = {
    LIBRARY_LOAD_LIMIT_DEFAULT: 200,
    LIBRARY_LOAD_LIMIT_POST_SCAN: 1000,
    ...overrides.constants
  };
  return {
    client: createApiClient({
      tauriInvoke: overrides.tauriInvoke || (() => { throw new Error("not tauri"); }),
      tauriIsTauri: overrides.tauriIsTauri || (() => false),
      tauriListen: overrides.tauriListen || (() => {}),
      state,
      normalizePath: overrides.normalizePath || ((value) => String(value || "").trim().toLowerCase().replace(/\\/g, "/")),
      constants
    }),
    state
  };
}

test("runtime detection handles false, true, thrown checks, and delegates in Tauri", async () => {
  for (const [tauriIsTauri, expected] of [
    [() => false, false],
    [() => true, true],
    [() => { throw new Error("no window"); }, false]
  ]) {
    assert.equal(makeClient({ tauriIsTauri }).client.isTauriRuntime(), expected);
  }

  const calls = [];
  const { client } = makeClient({
    tauriIsTauri: () => true,
    tauriInvoke: (cmd, payload) => {
      calls.push({ cmd, payload });
      return { ok: true, data: "real" };
    }
  });
  assert.equal((await client.invoke("scan_library", { foo: 1 })).data, "real");
  assert.deepEqual(calls, [{ cmd: "scan_library", payload: { foo: 1 } }]);
});

test("mock library commands scan, search, filter, and scope source roots", async () => {
  const { client } = makeClient();
  assert.equal((await client.invoke("scan_library")).data.jobId, "mock-scan");

  const allTracks = await client.invoke("search_tracks", { request: { query: "", limit: 10 } });
  assert.equal(allTracks.ok, true);
  assert.equal(allTracks.data.items.length, 3);

  const filtered = await client.invoke("search_tracks", { request: { query: "Track A", limit: 10 } });
  assert.equal(filtered.data.items.length, 1);
  assert.equal(filtered.data.items[0].title, "Track A");

  const browse = async (payload) => (await client.invoke("browse_source_files", payload)).data;
  assert.equal((await browse({ sourceRoots: [], includeMasterDb: false, query: "", limit: 10 })).items.length, 0);

  const folderOnly = await browse({ sourceRoots: ["/music"], includeMasterDb: false, query: "", limit: 10 });
  assert.equal(folderOnly.items.length, 3);
  assert.equal(folderOnly.items.some((track) => track.masterDbSource), false);
  assert.deepEqual(folderOnly.sourceRootAnalysis, [
    { sourceRoot: "/music", total: 3, analyzed: 0, fullyAnalyzed: false }
  ]);

  const masterOnly = await browse({ sourceRoots: [], includeMasterDb: true, query: "", limit: 10 });
  assert.deepEqual(masterOnly.items.map((track) => track.id), ["db-1"]);
  assert.equal(masterOnly.items[0].masterDbSource, true);
});

test("mock playlist, USB, playback, and removal commands mutate state consistently", async () => {
  const playlist = { id: "p1", name: "Set", tracks: [{ id: "t1" }, { id: "t2" }, { id: "t3" }] };
  const { client, state } = makeClient({
    state: { playlists: [playlist] }
  });

  const created = await client.invoke("create_playlist", { request: { name: "My Set" } });
  assert.equal(created.ok, true);
  assert.equal(created.data.name, "My Set");
  assert.equal(state.playlists.length, 2);

  const deleted = await client.invoke("delete_playlist", { request: { playlistId: "p1" } });
  assert.equal(deleted.data.deleted, true);
  assert.deepEqual(state.playlists.map((item) => item.name), ["My Set"]);

  const validRoot = await client.invoke("validate_usb_root", { request: { path: "/media/usb" } });
  const emptyRoot = await client.invoke("validate_usb_root", { request: { path: "" } });
  assert.equal(validRoot.data.valid, true);
  assert.equal(validRoot.data.normalizedRoot, "/media/usb");
  assert.equal(emptyRoot.data.valid, false);

  const play = await client.invoke("play_track_native", { request: { path: "/music/track.mp3" } });
  assert.equal(play.data.playing, true);
  assert.equal(state.mockPlayback.path, "/music/track.mp3");
  assert.equal(state.mockPlayback.playing, true);

  const stop = await client.invoke("stop_playback_native", {});
  assert.equal(stop.data.stopped, true);
  assert.equal(state.mockPlayback.path, null);
  assert.equal(state.mockPlayback.playing, false);

  const removePlaylist = { id: "p2", name: "Set", tracks: [{ id: "t1" }, { id: "t2" }, { id: "t3" }] };
  state.playlists.push(removePlaylist);
  const removed = await client.invoke("remove_tracks_from_playlist", {
    request: { playlistId: "p2", trackIds: ["t1", "t3"] }
  });
  assert.equal(removed.data.removed, 2);
  assert.deepEqual(removePlaylist.tracks, [{ id: "t2" }]);
});

test("mock command helper unwraps successes and throws mock command errors", async () => {
  const { client } = makeClient();
  const unknown = await client.invoke("nonexistent_command");
  assert.equal(unknown.ok, false);
  assert.equal(unknown.error.code, "INTERNAL_ERROR");

  const data = await client.command("search_tracks", { query: "", limit: 10 });
  assert.equal(data.items.length, 3);

  await assert.rejects(
    () => client.command("nonexistent_command"),
    /Unknown mock command/
  );
});
