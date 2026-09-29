import test from "node:test";
import assert from "node:assert/strict";
import {
  playTrackFromOrigin,
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

// A playback ctx whose DOM has no rows -- the transport/playhead UI updates run
// against an empty document.
function playbackCtx(state, overrides = {}) {
  return {
    state,
    document: { querySelectorAll: () => [] },
    command: async () => ({}),
    setStatus: () => {},
    warn: () => {},
    requestAnimationFrameFn: () => 0,
    cancelAnimationFrameFn: () => {},
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
    activeWaveform: null
  });

  await stopPlaybackIfActive(playbackCtx(state, {
    command: async (name) => { calls.push(name); },
    setStatus: (text) => calls.push(`status:${text}`)
  }));

  assert.equal(state.playbackActive, false);
  assert.equal(state.playbackTrackId, null);
  assert.equal(state.playbackPath, null);
  assert.equal(state.playbackRowKey, null);
  assert.equal(state.activeWaveform, null);
  assert.equal(state.playbackStopPromise, null);
  assert.deepEqual(calls, ["stop_playback_native", "status:Idle"]);
});

test("playTrackFromOrigin dedupes concurrent starts", async () => {
  const state = playbackState();
  let starts = 0;
  const ctx = playbackCtx(state, {
    command: async () => {
      starts += 1;
      await new Promise((resolve) => setTimeout(resolve, 15));
      return { trackId: "t1", sourceLabel: "Library" };
    }
  });

  await Promise.all([
    playTrackFromOrigin(ctx, { id: "t1" }, "local", {}),
    playTrackFromOrigin(ctx, { id: "t1" }, "local", {})
  ]);
  assert.equal(starts, 1);
  assert.equal(state.playbackStartPromise, null);
});

test("switching tracks while a start is pending supersedes it; only the newer start commits", async () => {
  const state = playbackState();
  const startedTracks = [];
  let resolveA;
  const pendingA = new Promise((resolve) => { resolveA = resolve; });
  const ctx = playbackCtx(state, {
    command: async (_name, request) => {
      startedTracks.push(request.trackId);
      if (request.trackId === "A") await pendingA;
      return { trackId: request.trackId, sourceLabel: "Library" };
    }
  });

  const resultA = playTrackFromOrigin(ctx, { id: "A" }, "local", { rowKey: "row-A" });
  const resultB = playTrackFromOrigin(ctx, { id: "B" }, "local", { rowKey: "row-B" });

  assert.equal(state.playbackPendingRowKey, "row-B");
  assert.equal(state.playbackPendingTrackId, "B");

  resolveA();
  await Promise.all([resultA, resultB]);
  // A was superseded before its queued job ran, so it never reached the backend.
  assert.deepEqual(startedTracks, ["B"]);
  assert.equal(state.playbackTrackId, "B");
  assert.equal(state.playbackRowKey, "row-B");
  assert.equal(state.playbackPendingKind, null);
});

test("stop supersedes a pending start; the stale start's success is not committed", async () => {
  const state = playbackState();
  const calls = [];
  let resolvePlay;
  const pendingPlay = new Promise((resolve) => { resolvePlay = resolve; });
  const ctx = playbackCtx(state, {
    command: async (name) => {
      calls.push(name);
      if (name === "play_resolved_track") {
        await pendingPlay;
        return {
          path: "/music/Track.mp3",
          playing: true,
          durationMs: 1000,
          positionMs: 0,
          trackId: "t1",
          matchedBy: "self",
          source: "library",
          sourceLabel: "Library",
          libraryResolved: true,
          hasUsbContext: false
        };
      }
      if (name === "stop_playback_native") return { stopped: true };
      throw new Error(`unexpected command ${name}`);
    }
  });

  const startPromise = playTrackFromOrigin(ctx, {
    id: "t1",
    title: "Track",
    filePath: "/music/Track.mp3"
  }, "library", { rowKey: "row-1" });

  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(calls, ["play_resolved_track"]);

  const stopPromise = stopPlaybackFromUi(ctx);
  assert.equal(state.playbackPendingKind, "stop");
  assert.equal(state.playbackActive, false);

  resolvePlay();
  await Promise.all([startPromise, stopPromise]);

  assert.deepEqual(calls, ["play_resolved_track", "stop_playback_native"]);
  assert.equal(state.playbackActive, false);
  assert.equal(state.playbackTrackId, null);
  assert.equal(state.playbackPath, null);
  assert.equal(state.playbackRowKey, null);
  assert.equal(state.playbackPendingKind, null);
});
