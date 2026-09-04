import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import {
  base64ToBytes,
  drawDetailWaveform,
  computeWaveNorm,
} from "../components/track-detail/waveform_detail.mjs";

test("base64ToBytes decodes and is defensive", () => {
  const src = new Uint8Array([0, 255, 16, 32, 1, 2]);
  const b64 = btoa(String.fromCharCode(...src));
  assert.deepEqual([...base64ToBytes(b64)], [...src]);
  assert.equal(base64ToBytes("").length, 0);
  assert.equal(base64ToBytes(undefined).length, 0);
});

test("computeWaveNorm is a whole-track p05..p95 of entry heights", () => {
  // 1000 entries: 800 quiet (h≈4), 200 loud (h≈28).
  const n = 1000;
  const bytes = new Uint8Array(n * 2);
  for (let i = 0; i < n; i += 1) {
    const h = i < 800 ? 4 : 28;
    const v = h << 2;
    bytes[i * 2] = (v >> 8) & 0xff;
    bytes[i * 2 + 1] = v & 0xff;
  }
  const norm = computeWaveNorm(bytes);
  assert.equal(norm.lo, 4, "p05 sits in the quiet band");
  assert.equal(norm.hi, 28, "p95 reaches the loud band");
});

test("drawDetailWaveform does not throw and signals when the wrapper has no width", () => {
  const dom = new JSDOM(
    `<!doctype html><body><div id="w" class="waveform"><canvas class="waveform-canvas-el"></canvas></div></body>`,
    { pretendToBeVisual: true }
  );
  global.window = dom.window;
  const wrap = dom.window.document.getElementById("w");
  const bytes = new Uint8Array(200); // 100 entries
  const norm = computeWaveNorm(bytes);
  assert.equal(
    drawDetailWaveform(wrap, bytes, { startMs: 0, endMs: 60000, durationMs: 120000, norm }),
    false
  );
  wrap.getBoundingClientRect = () => ({ width: 800, height: 200, left: 0, top: 0 });
  assert.equal(
    drawDetailWaveform(wrap, bytes, { startMs: 0, endMs: 60000, durationMs: 120000, norm }),
    true
  );
});
