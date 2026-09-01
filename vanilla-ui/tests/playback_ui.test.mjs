import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import {
  clearAllWaveformPlayheads,
  getPlaybackUiStateHelpers,
  scrubRatioFromPointer,
  setWaveformPlayhead,
  startPlayheadInterpolation,
  stopPlaybackFromUi,
  stopPlayheadInterpolation,
  updateTransportButtonsInDom
} from "../components/playback/actions.mjs";

test("playback UI globals and transport buttons reflect playing state", () => {
  const dom = new JSDOM(`
    <!doctype html><body>
      <button class="transport-btn" data-id="t1" data-row-key="row:1"></button>
      <button class="transport-btn" data-id="t2" data-row-key="row:2"></button>
    </body>
  `);
  const original = globalThis.playbackUiState;
  try {
    globalThis.playbackUiState = {
      ok: true,
      isTransportButtonPlaying: (state, meta) => state.playbackRowKey === meta.rowKey
    };
    assert.equal(getPlaybackUiStateHelpers().ok, true);

    updateTransportButtonsInDom({
      playbackActive: true,
      playbackRowKey: "row:1",
      playbackTrackId: null
    }, dom.window.document);

    const buttons = dom.window.document.querySelectorAll(".transport-btn");
    assert.equal(buttons[0].classList.contains("is-playing"), true);
    assert.equal(buttons[0].getAttribute("aria-label"), "Stop");
    assert.equal(buttons[1].classList.contains("is-playing"), false);
    assert.equal(buttons[1].getAttribute("aria-label"), "Play");
  } finally {
    globalThis.playbackUiState = original;
  }
});

test("waveform helpers set, clear, and clamp pointer scrub ratios", () => {
  const dom = new JSDOM(`<!doctype html><body><div class="waveform"></div><div class="waveform"></div></body>`);
  const document = dom.window.document;
  const wf = document.querySelector(".waveform");
  // jsdom does no layout, so clientWidth is 0 by default; pin it so the
  // pixel-offset the playhead transform uses is exercised.
  Object.defineProperty(wf, "clientWidth", { value: 120, configurable: true });

  setWaveformPlayhead(wf, 0.25, true);
  assert.equal(wf.style.getPropertyValue("--playhead-x"), "30px");
  assert.equal(wf.style.getPropertyValue("--playhead-position"), "25%");
  assert.equal(wf.classList.contains("is-playing"), true);

  clearAllWaveformPlayheads(document);
  document.querySelectorAll(".waveform").forEach((item) => {
    assert.equal(item.style.getPropertyValue("--playhead-x"), "0px");
    assert.equal(item.style.getPropertyValue("--playhead-position"), "0%");
    assert.equal(item.classList.contains("is-playing"), false);
  });

  const waveform = { getBoundingClientRect: () => ({ left: 10, width: 100 }) };
  assert.equal(scrubRatioFromPointer({ clientX: 60 }, waveform), 0.5);
  assert.equal(scrubRatioFromPointer({ clientX: -50 }, waveform), 0);
  assert.equal(scrubRatioFromPointer({ clientX: 1000 }, waveform), 1);
});

test("stopPlaybackFromUi clears playback state and updates UI", async () => {
  const state = {
    playbackStopPromise: null,
    playbackActive: true,
    playbackTrackId: "t1",
    playbackPath: "/music/t1.mp3",
    playbackRowKey: "row:1",
    activeWaveform: {},
    playbackLabelContext: { sourceLabel: "Library", title: "Artist - Track" }
  };
  const calls = [];

  await stopPlaybackFromUi(state, {
    command: async (name) => { calls.push(name); },
    clearAllWaveformPlayheads: () => { calls.push("clear"); },
    updateTransportButtonsInDom: () => { calls.push("transport"); },
    setStatus: (text) => { calls.push(text); }
  });

  assert.deepEqual(calls, ["transport", "stop_playback_native", "clear", "transport", "Idle"]);
  assert.equal(state.playbackActive, false);
  assert.equal(state.playbackTrackId, null);
  assert.equal(state.playbackStopPromise, null);
  // A stale label context must not survive a stop -- a later stray
  // playback.started would otherwise reprint a label for a track that ended.
  assert.equal(state.playbackLabelContext, null);
});

function startHarness({ waveformEl = { id: "wf" }, durationMs = 20000 } = {}) {
  const state = { activeWaveform: waveformEl };
  const ticks = [];
  let scheduled = null;
  startPlayheadInterpolation(state, {
    waveformEl,
    initialPositionMs: 5000,
    durationMs,
    setWaveformPlayhead: (_wf, fraction, playing) => { ticks.push({ fraction, playing }); },
    requestAnimationFrameFn: (fn) => { scheduled = fn; return 7; },
    cancelAnimationFrameFn: () => {},
    nowFn: () => 0
  });
  return { state, ticks, scheduled };
}

test("startPlayheadInterpolation schedules active waveforms and ignores invalid or superseded ones", () => {
  const started = startHarness();
  assert.equal(started.ticks.length, 1);
  assert.equal(started.ticks[0].fraction, 0.25);
  assert.equal(started.ticks[0].playing, true);
  assert.equal(started.state.playheadAnimationHandle, 7);
  assert.equal(typeof started.scheduled, "function");

  const superseded = startHarness({ durationMs: 10000 });
  superseded.state.activeWaveform = { id: "other" };
  superseded.scheduled();
  assert.equal(superseded.ticks.length, 1, "tick should no-op once superseded");

  for (const invalid of [{ waveformEl: null, durationMs: 10000 }, { waveformEl: { id: "wf" }, durationMs: 0 }]) {
    let ticked = false;
    startPlayheadInterpolation({}, {
      ...invalid,
      initialPositionMs: 0,
      setWaveformPlayhead: () => { ticked = true; },
      requestAnimationFrameFn: () => 1,
      cancelAnimationFrameFn: () => {}
    });
    assert.equal(ticked, false);
  }
});

test("stopPlayheadInterpolation cancels a scheduled frame and no-ops without one", () => {
  const state = { playheadAnimationHandle: 42 };
  const cancelled = [];
  stopPlayheadInterpolation(state, { cancelAnimationFrameFn: (handle) => cancelled.push(handle) });
  assert.deepEqual(cancelled, [42]);
  assert.equal(state.playheadAnimationHandle, null);

  stopPlayheadInterpolation(state, { cancelAnimationFrameFn: () => cancelled.push("unexpected") });
  assert.deepEqual(cancelled, [42]);
  assert.equal(state.playheadAnimationHandle, null);
});
