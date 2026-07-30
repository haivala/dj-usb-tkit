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

test("handlePlaybackEvent applies a playback.started confirmation and starts playhead interpolation", () => {
  const state = {
    playbackActive: false,
    playbackPath: null,
    playbackPendingKind: "play",
    activeWaveform: { id: "wf" }
  };
  const calls = { waveform: 0, transport: 0 };
  const raf = fakeRaf();

  handlePlaybackEvent(state, {
    event: "playback.started",
    path: "/music/a.mp3",
    playing: true,
    positionMs: 5000,
    durationMs: 20000
  }, {
    setWaveformPlayhead: (_wf, fraction, playing) => {
      calls.waveform += 1;
      state._fraction = fraction;
      state._playing = playing;
    },
    updateTransportButtonsInDom: () => { calls.transport += 1; },
    clearAllWaveformPlayheads: () => {},
    setStatus: () => {},
    requestAnimationFrameFn: raf.requestAnimationFrameFn,
    cancelAnimationFrameFn: raf.cancelAnimationFrameFn
  });

  assert.equal(state.playbackActive, true);
  assert.equal(state.playbackPath, "/music/a.mp3");
  assert.equal(calls.waveform, 1);
  assert.equal(calls.transport, 1);
  assert.ok(Math.abs(state._fraction - 0.25) < 0.001);
  assert.equal(state._playing, true);
});

test("handlePlaybackEvent resets playback state on stop and cancels playhead interpolation", () => {
  const state = {
    playbackActive: true,
    playbackPath: "/music/a.mp3",
    playbackTrackId: "t1",
    playbackRowKey: "row1",
    activeWaveform: { id: "wf" },
    playheadAnimationHandle: 42
  };
  const calls = { clear: 0, transport: 0, status: "", cancelled: [] };

  handlePlaybackEvent(state, { event: "playback.stopped" }, {
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => { calls.transport += 1; },
    clearAllWaveformPlayheads: () => { calls.clear += 1; },
    setStatus: (text) => { calls.status = text; },
    cancelAnimationFrameFn: (handle) => { calls.cancelled.push(handle); }
  });

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

test("handlePlaybackEvent reconciles playbackTrackId and clears playbackRowKey when path changes", () => {
  const state = {
    playbackActive: true,
    playbackPath: "/music/a.mp3",
    playbackTrackId: "old-id",
    playbackRowKey: "row-old",
    activeWaveform: null,
    tracks: [{ id: "new-id", filePath: "/music/b.mp3" }]
  };

  handlePlaybackEvent(state, {
    event: "playback.started",
    path: "/music/b.mp3",
    playing: true,
    positionMs: 0,
    durationMs: 0
  }, {
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    clearAllWaveformPlayheads: () => {},
    setStatus: () => {},
    resolveTrackIdForPath: (path) => state.tracks.find((t) => t.filePath === path)?.id || null
  });

  assert.equal(state.playbackPath, "/music/b.mp3");
  assert.equal(state.playbackTrackId, "new-id");
  assert.equal(state.playbackRowKey, null);
});

test("handlePlaybackEvent leaves playbackTrackId/playbackRowKey untouched when path is unchanged (seek)", () => {
  const state = {
    playbackActive: true,
    playbackPath: "/music/a.mp3",
    playbackTrackId: "id-1",
    playbackRowKey: "row-1",
    activeWaveform: null
  };
  let resolveCalls = 0;

  handlePlaybackEvent(state, {
    event: "playback.seeked",
    path: "/music/a.mp3",
    playing: true,
    positionMs: 5000,
    durationMs: 20000
  }, {
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    clearAllWaveformPlayheads: () => {},
    setStatus: () => {},
    resolveTrackIdForPath: () => { resolveCalls += 1; return null; }
  });

  assert.equal(state.playbackTrackId, "id-1");
  assert.equal(state.playbackRowKey, "row-1");
  assert.equal(resolveCalls, 0);
});

test("handlePlaybackEvent ignores a stray playing:true event after an explicit stop already completed", () => {
  const state = {
    playbackActive: false,
    playbackPath: null,
    playbackTrackId: null,
    playbackRowKey: null,
    playbackPendingKind: null,
    activeWaveform: null
  };
  const calls = { waveform: 0, transport: 0 };

  // Natural-end-of-track detection and a fresh explicit play travel to the frontend via
  // independent threads (the transition relay vs. the direct command's own emit) with no
  // ordering guarantee — a "started" confirmation that lands with no active path and
  // nothing pending can't be legitimate; treat it as noise rather than reviving cleared state.
  handlePlaybackEvent(state, {
    event: "playback.started",
    path: "/music/a.mp3",
    playing: true,
    positionMs: 6000,
    durationMs: 20000
  }, {
    setWaveformPlayhead: () => { calls.waveform += 1; },
    updateTransportButtonsInDom: () => { calls.transport += 1; },
    clearAllWaveformPlayheads: () => {},
    setStatus: () => {}
  });

  assert.equal(state.playbackActive, false);
  assert.equal(state.playbackPath, null);
  assert.equal(calls.waveform, 0);
  assert.equal(calls.transport, 0);
});

test("handlePlaybackEvent applies a playing:true event while our own play is still pending", () => {
  const state = {
    playbackActive: false,
    playbackPath: null,
    playbackPendingKind: "play",
    playbackPendingRowKey: "row-1",
    playbackPendingTrackId: "t1",
    activeWaveform: null
  };
  const calls = { transport: 0 };

  handlePlaybackEvent(state, {
    event: "playback.started",
    path: "/music/a.mp3",
    playing: true,
    positionMs: 0,
    durationMs: 20000
  }, {
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => { calls.transport += 1; },
    clearAllWaveformPlayheads: () => {},
    setStatus: () => {}
  });

  assert.equal(state.playbackActive, true);
  assert.equal(state.playbackPath, "/music/a.mp3");
  assert.equal(calls.transport, 1);
});

test("handlePlaybackEvent ignores a stopped event for a path we've already moved on from", () => {
  const state = {
    playbackActive: true,
    playbackPath: "/music/b.mp3",
    playbackTrackId: "t-b",
    playbackRowKey: "row-b",
    activeWaveform: { id: "wf" }
  };
  const calls = { clear: 0, transport: 0, status: "" };

  // Simulates a natural-stop notification for track A (which finished on its own) arriving
  // after the user already switched to and started track B — must not blank out B.
  handlePlaybackEvent(state, { event: "playback.stopped", path: "/music/a.mp3" }, {
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => { calls.transport += 1; },
    clearAllWaveformPlayheads: () => { calls.clear += 1; },
    setStatus: (text) => { calls.status = text; }
  });

  assert.equal(state.playbackActive, true);
  assert.equal(state.playbackPath, "/music/b.mp3");
  assert.equal(state.playbackTrackId, "t-b");
  assert.equal(state.playbackRowKey, "row-b");
  assert.equal(calls.clear, 0);
  assert.equal(calls.transport, 0);
  assert.equal(calls.status, "");
});

test("handlePlaybackEvent applies a stopped event matching the current path", () => {
  const state = {
    playbackActive: true,
    playbackPath: "/music/a.mp3",
    playbackTrackId: "t-a",
    playbackRowKey: "row-a",
    activeWaveform: { id: "wf" }
  };
  const calls = { clear: 0, transport: 0, status: "" };

  handlePlaybackEvent(state, { event: "playback.stopped", path: "/music/a.mp3" }, {
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => { calls.transport += 1; },
    clearAllWaveformPlayheads: () => { calls.clear += 1; },
    setStatus: (text) => { calls.status = text; }
  });

  assert.equal(state.playbackActive, false);
  assert.equal(state.playbackPath, null);
  assert.equal(calls.clear, 1);
  assert.equal(calls.transport, 1);
  assert.equal(calls.status, "Idle");
});

test("handlePlaybackEvent surfaces playback errors", () => {
  let status = "";
  handlePlaybackEvent({ activeWaveform: null }, { event: "playback.error", message: "Audio device busy" }, {
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    clearAllWaveformPlayheads: () => {},
    setStatus: (text) => { status = text; }
  });

  assert.equal(status, "Audio device busy");
});
