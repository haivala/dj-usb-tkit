import test from "node:test";
import assert from "node:assert/strict";
import {
  playTrackFromOrigin as playTrackFromOriginCore,
  playTrackFromOriginController as playTrackFromOrigin,
  stopPlaybackFromUi,
  stopPlaybackIfActive
} from "../components/playback/actions.mjs";

function playbackState(overrides = {}) {
  return {
    playbackStartPromise: null,
    playbackStopPromise: null,
    playbackActive: false,
    playbackTrackId: null,
    playbackPath: null,
    playbackRowKey: null,
    activeWaveform: null,
    playbackGeneration: 0,
    playbackPendingKind: null,
    playbackPendingRowKey: null,
    playbackPendingTrackId: null,
    playbackBackendQueue: null,
    sourceRoots: ["/music"],
    usbRoot: null,
    usbRootValid: false,
    ...overrides
  };
}

function stopDeps(calls = [], overrides = {}) {
  return {
    command: async (name) => {
      calls.push(name);
      assert.equal(name, "stop_playback_native");
    },
    clearAllWaveformPlayheads: () => calls.push("clear"),
    updateTransportButtonsInDom: () => calls.push("transport"),
    setStatus: (text) => calls.push(`status:${text}`),
    warn: () => {},
    ...overrides
  };
}

test("stopPlaybackIfActive clears playback state and UI", async () => {
  const calls = [];
  const state = playbackState({
    playbackActive: true,
    playbackTrackId: "t1",
    playbackPath: "/music/a.mp3",
    playbackRowKey: "row-1",
    activeWaveform: { id: "wf" }
  });

  await stopPlaybackIfActive(state, stopDeps(calls));

  assert.equal(state.playbackActive, false);
  assert.equal(state.playbackTrackId, null);
  assert.equal(state.playbackPath, null);
  assert.equal(state.playbackRowKey, null);
  assert.equal(state.activeWaveform, null);
  assert.equal(state.playbackStopPromise, null);
  assert.deepEqual(calls, ["transport", "stop_playback_native", "clear", "transport", "status:Idle"]);
});

test("playTrackFromOrigin dedupes concurrent starts", async () => {
  const state = playbackState();
  let starts = 0;
  const deps = {
    playTrackFromOriginCore: async () => {
      starts += 1;
      await new Promise((resolve) => setTimeout(resolve, 15));
      return "ok";
    }
  };

  assert.deepEqual(await Promise.all([
    playTrackFromOrigin(state, { id: "t1" }, "local", {}, deps),
    playTrackFromOrigin(state, { id: "t1" }, "local", {}, deps)
  ]), ["ok", "ok"]);
  assert.equal(starts, 1);
  assert.equal(state.playbackStartPromise, null);
});

test("switching tracks while a start is pending immediately supersedes rather than being dropped", async () => {
  const state = playbackState();
  const startedTracks = [];
  let resolveA;
  const pendingA = new Promise((resolve) => { resolveA = resolve; });

  const resultA = playTrackFromOrigin(state, { id: "A" }, "local", { rowKey: "row-A" }, {
    playTrackFromOriginCore: async () => {
      startedTracks.push("A");
      await pendingA;
      return "A-ok";
    }
  });
  const resultB = playTrackFromOrigin(state, { id: "B" }, "local", { rowKey: "row-B" }, {
    playTrackFromOriginCore: async () => {
      startedTracks.push("B");
      return "B-ok";
    }
  });

  assert.deepEqual(startedTracks, ["A", "B"]);
  assert.equal(state.playbackPendingRowKey, "row-B");
  assert.equal(state.playbackPendingTrackId, "B");

  resolveA();
  assert.deepEqual(await Promise.all([resultA, resultB]), ["A-ok", "B-ok"]);
  assert.equal(state.playbackPendingKind, null);
});

test("stop supersedes a pending start; the stale start's success is not committed", async () => {
  const state = playbackState();
  const calls = [];
  let resolvePlay;
  const pendingPlay = new Promise((resolve) => { resolvePlay = resolve; });
  const commonDeps = {
    command: async (name, payload) => {
      calls.push(name);
      if (name === "resolve_playback_source") {
        return { resolvedPath: "/music/Track.mp3", trackId: "t1", matchedBy: "self" };
      }
      if (name === "play_track_native") {
        await pendingPlay;
        return { path: payload.path, durationMs: 1000, positionMs: 0 };
      }
      if (name === "stop_playback_native") return { stopped: true };
      throw new Error(`unexpected command ${name}`);
    },
    trackPathMatchesAnyRoot: (path, roots) => roots.some((root) => String(path || "").startsWith(root)),
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus: () => {},
    warn: () => {}
  };

  const startPromise = playTrackFromOrigin(state, {
    id: "t1",
    title: "Track",
    filePath: "/music/Track.mp3"
  }, "library", { rowKey: "row-1" }, {
    playTrackFromOriginCore,
    ...commonDeps
  });

  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(calls, ["resolve_playback_source", "play_track_native"]);

  const stopPromise = stopPlaybackFromUi(state, commonDeps);
  assert.equal(state.playbackPendingKind, "stop");
  assert.equal(state.playbackActive, false);

  resolvePlay();
  await Promise.all([startPromise, stopPromise]);

  assert.deepEqual(calls, ["resolve_playback_source", "play_track_native", "stop_playback_native"]);
  assert.equal(state.playbackActive, false);
  assert.equal(state.playbackTrackId, null);
  assert.equal(state.playbackPath, null);
  assert.equal(state.playbackRowKey, null);
  assert.equal(state.playbackPendingKind, null);
});
