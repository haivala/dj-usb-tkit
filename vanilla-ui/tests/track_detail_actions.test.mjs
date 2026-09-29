import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import {
  createTrackDetailController,
  openTrackDetail,
  MAX_CUES,
} from "../components/track-detail/actions.mjs";
import { base64ToBytes } from "../components/track-detail/waveform_detail.mjs";
import { APP_TEMPLATES } from "./test_helpers.mjs";

function pwv5Base64(entryCount) {
  const bytes = new Uint8Array(entryCount * 2);
  for (let i = 0; i < entryCount; i += 1) {
    const h = 8 + (i % 20);
    const v = (2 << 13) | (3 << 10) | (5 << 7) | (h << 2);
    bytes[i * 2] = (v >> 8) & 0xff;
    bytes[i * 2 + 1] = v & 0xff;
  }
  return btoa(String.fromCharCode(...bytes));
}

function makeEl() {
  const dom = new JSDOM(
    `<!doctype html><body>
      <div id="trackDetailOverlay" hidden>
        <h3 id="trackDetailTitle"></h3>
        <input id="trackDetailFirstBeatMs" />
        <div class="waveform waveform-canvas" id="trackDetailWaveform">
          <canvas class="waveform-canvas-el"></canvas>
          <i id="trackDetailPlayhead" hidden></i>
        </div>
        <div id="trackDetailBeatgrid"></div>
        <div id="trackDetailCueMarkers"></div>
        <button id="trackDetailAddCue"></button>
        <div id="trackDetailCueList"></div>
        <button id="trackDetailSaveBtn"></button>
        <div id="trackDetailColorPopover" hidden></div>
      </div>
      ${APP_TEMPLATES}
    </body>`,
    { pretendToBeVisual: true }
  );
  global.window = dom.window;
  global.document = dom.window.document;
  const ids = [
    "trackDetailOverlay", "trackDetailTitle", "trackDetailWaveform", "trackDetailBeatgrid",
    "trackDetailCueMarkers", "trackDetailPlayhead", "trackDetailFirstBeatMs", "trackDetailAddCue",
    "trackDetailCueList", "trackDetailSaveBtn", "trackDetailColorPopover",
  ];
  return Object.fromEntries(ids.map((id) => [id, dom.window.document.getElementById(id)]));
}

test("base64ToBytes round-trips a PWV5 payload", () => {
  const bytes = base64ToBytes(pwv5Base64(50));
  assert.equal(bytes.length, 100);
  assert.ok(bytes instanceof Uint8Array);
  assert.equal(base64ToBytes(null).length, 0);
  assert.equal(base64ToBytes("!!not base64!!").length, 0);
});

test("addCue adds a coloured, auto-named cue at the playhead and stops at 8", () => {
  const el = makeEl();
  const c = createTrackDetailController(el);
  c.open({
    track: { title: "T", artist: "A", durationMs: 100000, bpm: 120, detailWaveform: pwv5Base64(200) },
    firstBeatMs: 100,
    cues: [],
    durationMs: 100000,
    bpm: 120,
  });

  for (let i = 0; i < MAX_CUES; i += 1) assert.ok(c.addCue(), `cue ${i} added`);
  assert.equal(c.addCue(), null, "9th cue refused");
  assert.equal(el.trackDetailAddCue.disabled, true);

  // Default name + colour are assigned once, in add order, and cycle through
  // all 8 palette colours (MAX_CUES === palette size).
  const added = c.getWorking().cues;
  assert.deepEqual(added.map((x) => x.name), Array.from({ length: 8 }, (_, i) => `Cue ${i + 1}`));
  assert.deepEqual(added.map((x) => x.colorId), [1, 2, 3, 4, 5, 6, 7, 8]);

  const payload = c.toSavePayload();
  assert.equal(payload.cues.length, MAX_CUES);
  assert.equal(payload.firstBeatMs, 100);
});

test("openTrackDetail bails when the track has no analysis waveform", async () => {
  makeEl();
  const emitted = [];
  let opened = false;
  await openTrackDetail(
    {
      command: async () => ({ track: { id: "local-x" }, cues: [], detailWaveform: null }),
      resolveLocalTrackIdAsync: async () => "local-x",
      trackDetailDialog: { open: async () => { opened = true; return null; } },
      emitStatus: (m) => emitted.push(m),
    },
    { id: "row-x" }
  );
  assert.equal(opened, false);
  assert.match(emitted.join(" "), /Analyze this track first/);
});
