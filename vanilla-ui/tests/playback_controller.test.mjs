import test from "node:test";
import assert from "node:assert/strict";
import {
  stopPlaybackIfActive,
  playTrackFromOriginController as playTrackFromOrigin
} from "../components/playback/actions.mjs";

test("stopPlaybackIfActive clears playback state and UI", async () => {
  const calls = [];
  const state = {
    playbackActive: true,
    playbackTrackId: "t1",
    playbackPath: "/music/a.mp3",
    playbackRowKey: "row-1",
    activeWaveform: { id: "wf" },
    playbackStopPromise: null
  };

  await stopPlaybackIfActive(state, {
    command: async (name) => {
      calls.push(name);
      assert.equal(name, "stop_playback_native");
    },
    clearAllWaveformPlayheads: () => calls.push("clear"),
    updateTransportButtonsInDom: () => calls.push("transport"),
    setStatus: (text) => calls.push(`status:${text}`),
    warn: () => {}
  });

  assert.equal(state.playbackActive, false);
  assert.equal(state.playbackTrackId, null);
  assert.equal(state.playbackPath, null);
  assert.equal(state.playbackRowKey, null);
  assert.equal(state.activeWaveform, null);
  assert.equal(state.playbackStopPromise, null);
  assert.deepEqual(calls, [
    "stop_playback_native",
    "clear",
    "transport",
    "status:Idle"
  ]);
});

test("playTrackFromOrigin dedupes concurrent starts", async () => {
  const state = {
    playbackStartPromise: null,
    playbackStopPromise: null
  };
  let starts = 0;
  const result = await Promise.all([
    playTrackFromOrigin(state, { id: "t1" }, "local", {}, {
      playTrackFromOriginCore: async () => {
        starts += 1;
        await new Promise((resolve) => setTimeout(resolve, 15));
        return "ok";
      }
    }),
    playTrackFromOrigin(state, { id: "t1" }, "local", {}, {
      playTrackFromOriginCore: async () => {
        starts += 1;
        return "ok";
      }
    })
  ]);

  assert.deepEqual(result, ["ok", "ok"]);
  assert.equal(starts, 1);
  assert.equal(state.playbackStartPromise, null);
});
