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

// A ctx whose document counts the transport-button refreshes and playhead
// clears it's asked for (it has no rows, so both find nothing).
function eventDeps(overrides = {}) {
  const calls = { transport: 0, clear: 0, status: "", cancelled: [] };
  return {
    calls,
    deps: {
      document: {
        querySelectorAll: (selector) => {
          if (selector === ".transport-btn") calls.transport += 1;
          if (selector === ".waveform") calls.clear += 1;
          return [];
        }
      },
      emitStatus: (text) => { calls.status = text; },
      requestAnimationFrameFn: () => 0,
      cancelAnimationFrameFn: (handle) => { calls.cancelled.push(handle); },
      ...overrides
    }
  };
}

// A waveform element that records the playhead it's given.
function fakeWaveform() {
  const wf = {
    clientWidth: 100,
    count: 0,
    fraction: null,
    playing: null,
    style: {
      setProperty(name, value) {
        if (name !== "--playhead-position") return;
        wf.count += 1;
        wf.fraction = parseFloat(value) / 100;
      }
    },
    classList: {
      toggle(name, on) {
        if (name === "is-playing") wf.playing = on;
      }
    }
  };
  return wf;
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
    activeWaveform: fakeWaveform()
  };
  const wf = state.activeWaveform;
  const raf = fakeRaf();
  const { calls, deps } = eventDeps({
    requestAnimationFrameFn: raf.requestAnimationFrameFn,
    cancelAnimationFrameFn: raf.cancelAnimationFrameFn
  });

  handlePlaybackEvent({ state, ...deps }, started());

  assert.equal(state.playbackActive, true);
  assert.equal(state.playbackPath, "/music/a.mp3");
  assert.equal(wf.count, 1);
  assert.equal(calls.transport, 1);
  assert.ok(Math.abs(wf.fraction - 0.25) < 0.001);
  assert.equal(wf.playing, true);
});

test("handlePlaybackEvent resets playback state on stop and cancels interpolation", () => {
  const state = {
    playbackActive: true,
    playbackPath: "/music/a.mp3",
    playbackTrackId: "t1",
    playbackRowKey: "row1",
    activeWaveform: fakeWaveform(),
    playheadAnimationHandle: 42
  };
  const { calls, deps } = eventDeps();

  handlePlaybackEvent({ state, ...deps }, { event: "playback.stopped" });

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

test("handlePlaybackEvent applies the backend-resolved trackId and clears rowKey on a path change", () => {
  const changed = {
    playbackActive: true,
    playbackPath: "/music/a.mp3",
    playbackTrackId: "old-id",
    playbackRowKey: "row-old",
    activeWaveform: null
  };
  handlePlaybackEvent(
    { state: changed, ...eventDeps().deps },
    started("/music/b.mp3", { positionMs: 0, durationMs: 0, trackId: "new-id" })
  );
  assert.equal(changed.playbackPath, "/music/b.mp3");
  assert.equal(changed.playbackTrackId, "new-id");
  assert.equal(changed.playbackRowKey, null);
});

test("handlePlaybackEvent leaves playbackTrackId untouched when the event omits trackId", () => {
  const seeked = {
    playbackActive: true,
    playbackPath: "/music/a.mp3",
    playbackTrackId: "id-1",
    playbackRowKey: "row-1",
    activeWaveform: null
  };
  handlePlaybackEvent({ state: seeked, ...eventDeps().deps }, started("/music/a.mp3", { event: "playback.seeked" }));
  assert.equal(seeked.playbackTrackId, "id-1");
  assert.equal(seeked.playbackRowKey, "row-1");
});

test("handlePlaybackEvent clears playbackTrackId when the backend sends trackId null", () => {
  const state = {
    playbackActive: true,
    playbackPath: "/music/a.mp3",
    playbackTrackId: "id-1",
    playbackRowKey: "row-1",
    activeWaveform: null
  };
  handlePlaybackEvent({ state, ...eventDeps().deps }, started("/music/b.mp3", { trackId: null }));
  assert.equal(state.playbackTrackId, null);
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
  handlePlaybackEvent({ state, ...deps }, started("/music/a.mp3", { event: "playback.seeked" }));
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
  handlePlaybackEvent({ state: stray, ...strayHarness.deps }, started());
  assert.equal(stray.playbackActive, false);
  assert.equal(stray.playbackPath, null);

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
  handlePlaybackEvent({ state: pending, ...pendingHarness.deps }, started("/music/a.mp3", { positionMs: 0 }));
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
    activeWaveform: fakeWaveform()
  };
  const staleHarness = eventDeps();
  handlePlaybackEvent({ state: stale, ...staleHarness.deps }, { event: "playback.stopped", path: "/music/a.mp3" });
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
    activeWaveform: fakeWaveform()
  };
  const currentHarness = eventDeps();
  handlePlaybackEvent({ state: current, ...currentHarness.deps }, { event: "playback.stopped", path: "/music/a.mp3" });
  assert.equal(current.playbackActive, false);
  assert.equal(current.playbackPath, null);
  assert.equal(currentHarness.calls.clear, 1);
  assert.equal(currentHarness.calls.transport, 1);
  assert.equal(currentHarness.calls.status, "Idle");
});

test("handlePlaybackEvent surfaces playback errors", () => {
  const { calls, deps } = eventDeps();
  handlePlaybackEvent({ state: { activeWaveform: null }, ...deps }, {
    event: "playback.error",
    message: "Audio device busy"
  });

  assert.equal(calls.status, "Audio device busy");
});
