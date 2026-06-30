import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import {
  getPlaybackUiStateHelpers,
  updateTransportButtonsInDom,
  setWaveformPlayhead,
  clearAllWaveformPlayheads,
  scrubRatioFromPointer,
  stopPlaybackFromUi
} from "../components/playback/actions.mjs";

test("getPlaybackUiStateHelpers reads global playbackUiState", () => {
  const original = globalThis.playbackUiState;
  try {
    globalThis.playbackUiState = { ok: true };
    assert.deepEqual(getPlaybackUiStateHelpers(), { ok: true });
  } finally {
    globalThis.playbackUiState = original;
  }
});

test("updateTransportButtonsInDom updates button visual state", () => {
  const dom = new JSDOM(`
    <!doctype html>
    <body>
      <button class="transport-btn" data-id="t1" data-row-key="row:1"></button>
      <button class="transport-btn" data-id="t2" data-row-key="row:2"></button>
    </body>
  `);
  const original = globalThis.playbackUiState;
  try {
    globalThis.playbackUiState = {
      isTransportButtonPlaying: (state, meta) => state.playbackRowKey === meta.rowKey
    };
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

test("waveform helpers set and clear playhead state", () => {
  const dom = new JSDOM(`<!doctype html><body><div class="waveform"></div><div class="waveform"></div></body>`);
  const document = dom.window.document;
  const wf = document.querySelector(".waveform");

  setWaveformPlayhead(wf, 0.25, true);
  assert.equal(wf.style.getPropertyValue("--playhead-position"), "25%");
  assert.equal(wf.classList.contains("is-playing"), true);

  clearAllWaveformPlayheads(document);
  document.querySelectorAll(".waveform").forEach((item) => {
    assert.equal(item.style.getPropertyValue("--playhead-position"), "0%");
    assert.equal(item.classList.contains("is-playing"), false);
  });
});

test("scrubRatioFromPointer clamps against waveform bounds", () => {
  const waveform = {
    getBoundingClientRect: () => ({ left: 10, width: 100 })
  };
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
    activeWaveform: {}
  };
  const calls = [];

  await stopPlaybackFromUi(state, {
    command: async (name) => { calls.push(name); },
    clearAllWaveformPlayheads: () => { calls.push("clear"); },
    updateTransportButtonsInDom: () => { calls.push("transport"); },
    setStatus: (text) => { calls.push(text); }
  });

  assert.deepEqual(calls, ["stop_playback_native", "clear", "transport", "Idle"]);
  assert.equal(state.playbackActive, false);
  assert.equal(state.playbackTrackId, null);
  assert.equal(state.playbackStopPromise, null);
});
