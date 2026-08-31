import test from "node:test";
import assert from "node:assert/strict";
import { handlePlaybackEvent } from "../components/playback/actions.mjs";

function fakeRaf() {
  const calls = [];
  return {
    requestAnimationFrameFn: (fn) => { calls.push(fn); return calls.length; },
    cancelAnimationFrameFn: () => {},
    calls
  };
}

function eventDeps(overrides = {}) {
  const calls = { waveform: 0, transport: 0, clear: 0, status: "", cancelled: [] };
  return {
    calls,
    deps: {
      setWaveformPlayhead: (_wf, fraction, playing) => {
        calls.waveform += 1;
        calls.fraction = fraction;
        calls.playing = playing;
      },
      updateTransportButtonsInDom: () => { calls.transport += 1; },
      clearAllWaveformPlayheads: () => { calls.clear += 1; },
      setStatus: (text) => { calls.status = text; },
      cancelAnimationFrameFn: (handle) => { calls.cancelled.push(handle); },
      ...overrides
    }
  };
}

function started(path = "/music/a.mp3", overrides = {}) {
  return {
    event: "playback.started",
    path,
    playing: true,
    positionMs: 5000,
    durationMs: 20000,
    ...overrides
  };
}

test("handlePlaybackEvent applies a started confirmation and starts playhead interpolation", () => {
  const state = {
    playbackActive: false,
    playbackPath: null,
    playbackPendingKind: "play",
    activeWaveform: { id: "wf" }
  };
  const raf = fakeRaf();
  const { calls, deps } = eventDeps({
    requestAnimationFrameFn: raf.requestAnimationFrameFn,
    cancelAnimationFrameFn: raf.cancelAnimationFrameFn
  });

  handlePlaybackEvent(state, started(), deps);

  assert.equal(state.playbackActive, true);
  assert.equal(state.playbackPath, "/music/a.mp3");
  assert.equal(calls.waveform, 1);
  assert.equal(calls.transport, 1);
  assert.ok(Math.abs(calls.fraction - 0.25) < 0.001);
  assert.equal(calls.playing, true);
});

test("handlePlaybackEvent resets playback state on stop and cancels interpolation", () => {
  const state = {
    playbackActive: true,
    playbackPath: "/music/a.mp3",
    playbackTrackId: "t1",
    playbackRowKey: "row1",
    activeWaveform: { id: "wf" },
    playheadAnimationHandle: 42
  };
  const { calls, deps } = eventDeps();

  handlePlaybackEvent(state, { event: "playback.stopped" }, deps);

  assert.equal(state.playbackActive, false);
  assert.equal(state.playbackPath, null);
  assert.equal(state.playbackTrackId, null);
  assert.equal(state.playbackRowKey, null);
  assert.equal(state.activeWaveform, null);
  assert.equal(state.playheadAnimationHandle, null);
  assert.deepEqual(calls.cancelled, [42]);
  assert.equal(calls.clear, 1);
  assert.equal(calls.transport, 1);
  assert.equal(calls.status, "Idle");
});

test("handlePlaybackEvent reconciles ids on path changes but leaves seek state intact", () => {
  const changed = {
    playbackActive: true,
    playbackPath: "/music/a.mp3",
    playbackTrackId: "old-id",
    playbackRowKey: "row-old",
    activeWaveform: null,
    tracks: [{ id: "new-id", filePath: "/music/b.mp3" }]
  };
  handlePlaybackEvent(changed, started("/music/b.mp3", { positionMs: 0, durationMs: 0 }), eventDeps({
    resolveTrackIdForPath: (path) => changed.tracks.find((track) => track.filePath === path)?.id || null
  }).deps);
  assert.equal(changed.playbackPath, "/music/b.mp3");
  assert.equal(changed.playbackTrackId, "new-id");
  assert.equal(changed.playbackRowKey, null);

  const seeked = {
    playbackActive: true,
    playbackPath: "/music/a.mp3",
    playbackTrackId: "id-1",
    playbackRowKey: "row-1",
    activeWaveform: null
  };
  let resolveCalls = 0;
  handlePlaybackEvent(seeked, started("/music/a.mp3", { event: "playback.seeked" }), eventDeps({
    resolveTrackIdForPath: () => { resolveCalls += 1; return null; }
  }).deps);
  assert.equal(seeked.playbackTrackId, "id-1");
  assert.equal(seeked.playbackRowKey, "row-1");
  assert.equal(resolveCalls, 0);
});

test("handlePlaybackEvent reuses the backend source label verbatim on a seek", () => {
  // Regression: the status line used to be re-derived on the frontend from the
  // play origin, so a seek on a USB-origin track that the backend actually
  // resolved to the library flipped the label from "Library" back to "USB".
  const state = {
    playbackActive: true,
    playbackPath: "/music/a.mp3",
    playbackTrackId: "id-1",
    playbackRowKey: "row-1",
    activeWaveform: null,
    playbackLabelContext: { sourceLabel: "Library", title: "Artist - Track" }
  };
  const { calls, deps } = eventDeps();
  handlePlaybackEvent(state, started("/music/a.mp3", { event: "playback.seeked" }), deps);
  assert.equal(calls.status, "Playing from Library: Artist - Track");
});

test("handlePlaybackEvent ignores stray started events but applies pending started events", () => {
  const stray = {
    playbackActive: false,
    playbackPath: null,
    playbackTrackId: null,
    playbackRowKey: null,
    playbackPendingKind: null,
    activeWaveform: null
  };
  const strayHarness = eventDeps();
  handlePlaybackEvent(stray, started(), strayHarness.deps);
  assert.equal(stray.playbackActive, false);
  assert.equal(stray.playbackPath, null);
  assert.equal(strayHarness.calls.waveform, 0);
  assert.equal(strayHarness.calls.transport, 0);

  const pending = {
    playbackActive: false,
    playbackPath: null,
    playbackPendingKind: "play",
    playbackPendingRowKey: "row-1",
    playbackPendingTrackId: "t1",
    activeWaveform: null
  };
  const pendingHarness = eventDeps();
  handlePlaybackEvent(pending, started("/music/a.mp3", { positionMs: 0 }), pendingHarness.deps);
  assert.equal(pending.playbackActive, true);
  assert.equal(pending.playbackPath, "/music/a.mp3");
  assert.equal(pendingHarness.calls.transport, 1);
});

test("handlePlaybackEvent ignores stale stopped paths and applies matching stopped paths", () => {
  const stale = {
    playbackActive: true,
    playbackPath: "/music/b.mp3",
    playbackTrackId: "t-b",
    playbackRowKey: "row-b",
    activeWaveform: { id: "wf" }
  };
  const staleHarness = eventDeps();
  handlePlaybackEvent(stale, { event: "playback.stopped", path: "/music/a.mp3" }, staleHarness.deps);
  assert.equal(stale.playbackActive, true);
  assert.equal(stale.playbackPath, "/music/b.mp3");
  assert.equal(stale.playbackTrackId, "t-b");
  assert.equal(stale.playbackRowKey, "row-b");
  assert.equal(staleHarness.calls.clear, 0);
  assert.equal(staleHarness.calls.transport, 0);
  assert.equal(staleHarness.calls.status, "");

  const current = {
    playbackActive: true,
    playbackPath: "/music/a.mp3",
    playbackTrackId: "t-a",
    playbackRowKey: "row-a",
    activeWaveform: { id: "wf" }
  };
  const currentHarness = eventDeps();
  handlePlaybackEvent(current, { event: "playback.stopped", path: "/music/a.mp3" }, currentHarness.deps);
  assert.equal(current.playbackActive, false);
  assert.equal(current.playbackPath, null);
  assert.equal(currentHarness.calls.clear, 1);
  assert.equal(currentHarness.calls.transport, 1);
  assert.equal(currentHarness.calls.status, "Idle");
});

test("handlePlaybackEvent surfaces playback errors", () => {
  const { calls, deps } = eventDeps();
  handlePlaybackEvent({ activeWaveform: null }, {
    event: "playback.error",
    message: "Audio device busy"
  }, deps);

  assert.equal(calls.status, "Audio device busy");
});
