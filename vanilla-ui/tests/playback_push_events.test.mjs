import test from "node:test";
import assert from "node:assert/strict";
import { handlePlaybackEvent } from "../components/playback/actions.mjs";

test("handlePlaybackEvent applies push progress updates", () => {
  const state = { playbackActive: false, playbackPath: null, activeWaveform: { id: "wf" } };
  const calls = { waveform: 0, transport: 0 };

  handlePlaybackEvent(state, {
    event: "playback.progress",
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
    setStatus: () => {}
  });

  assert.equal(state.playbackActive, true);
  assert.equal(state.playbackPath, "/music/a.mp3");
  assert.equal(calls.waveform, 1);
  assert.equal(calls.transport, 1);
  assert.equal(state._fraction, 0.25);
  assert.equal(state._playing, true);
});

test("handlePlaybackEvent resets playback state on stop", () => {
  const state = {
    playbackActive: true,
    playbackPath: "/music/a.mp3",
    playbackTrackId: "t1",
    playbackRowKey: "row1",
    activeWaveform: { id: "wf" }
  };
  const calls = { clear: 0, transport: 0, status: "" };

  handlePlaybackEvent(state, { event: "playback.stopped" }, {
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => { calls.transport += 1; },
    clearAllWaveformPlayheads: () => { calls.clear += 1; },
    setStatus: (text) => { calls.status = text; }
  });

  assert.equal(state.playbackActive, false);
  assert.equal(state.playbackPath, null);
  assert.equal(state.playbackTrackId, null);
  assert.equal(state.playbackRowKey, null);
  assert.equal(state.activeWaveform, null);
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
