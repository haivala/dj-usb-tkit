import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import {
  createTrackDetailController,
  openTrackDetail,
  MAX_CUES,
} from "../components/track-detail/actions.mjs";
import { base64ToBytes } from "../components/track-detail/waveform_detail.mjs";

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

test("renameCue mutates the name without rebuilding the row DOM (focus-loss regression)", () => {
  const el = makeEl();
  const c = createTrackDetailController(el);
  c.open({ track: { durationMs: 100000, bpm: 120, detailWaveform: pwv5Base64(200) }, firstBeatMs: 0, cues: [], durationMs: 100000, bpm: 120 });
  const cue = c.addCue();

  const nameInput = el.trackDetailCueList.querySelector(".cue-row-name");
  assert.ok(nameInput, "row renders a name input");

  c.renameCue(cue.tempId, "a");
  c.renameCue(cue.tempId, "ab");
  c.renameCue(cue.tempId, "abc");

  // Same DOM node instance — renderCueList (which wipes+rebuilds every row)
  // must not have run, or a real input would lose focus on every keystroke.
  assert.strictEqual(el.trackDetailCueList.querySelector(".cue-row-name"), nameInput);
  assert.equal(c.getWorking().cues[0].name, "abc");

  // A subsequently added cue doesn't renumber/overwrite the rename.
  const second = c.addCue();
  assert.equal(c.getWorking().cues[0].name, "abc", "first cue's rename survives a later add");
  assert.equal(second.name, "Cue 2");
});

test("addCueAtRatio drops a cue at an exact track position (double-click)", () => {
  const el = makeEl();
  const c = createTrackDetailController(el);
  c.open({ track: { durationMs: 200000, bpm: 120, detailWaveform: pwv5Base64(200) }, firstBeatMs: 0, cues: [], durationMs: 200000, bpm: 120 });

  const cue = c.addCueAtRatio(0.25);
  assert.equal(cue.positionMs, 50000);
  assert.equal(c.toSavePayload().cues[0].positionMs, 50000);

  // Explicit-position overload clamps to the track.
  assert.equal(c.addCue(999999).positionMs, 200000);
});

test("default view is ~2 minutes; fitView opens the whole track; zoom narrows it", () => {
  const el = makeEl();
  const c = createTrackDetailController(el);
  c.open({ track: { durationMs: 300000, bpm: 120, detailWaveform: pwv5Base64(200) }, firstBeatMs: 0, cues: [], durationMs: 300000, bpm: 120 });

  let v = c.getView();
  assert.equal(v.startMs, 0);
  assert.equal(v.endMs, 120000, "default span is 2 min");

  c.fitView();
  v = c.getView();
  assert.equal(v.endMs - v.startMs, 300000);

  c.zoomAt(0.5, 0.25); // zoom in 4x, anchored at centre
  v = c.getView();
  assert.equal(v.endMs - v.startMs, 75000);
  assert.equal(v.startMs, 112500);

  // viewRatioToTrackRatio maps a 0..1 within the view to a whole-track fraction.
  const trackRatio = c.viewRatioToTrackRatio(0); // left edge of view
  assert.ok(Math.abs(trackRatio - 112500 / 300000) < 1e-6);
});

test("markers are labelled A.. in position order", () => {
  const el = makeEl();
  const c = createTrackDetailController(el);
  c.open({ track: { durationMs: 100000, bpm: 120, detailWaveform: pwv5Base64(200) }, firstBeatMs: 0, cues: [
    { positionMs: 5000 }, { positionMs: 1000 }, { positionMs: 9000 },
  ], durationMs: 100000, bpm: 120 });
  const labels = [...el.trackDetailCueMarkers.querySelectorAll(".cue-marker")].map((m) => m.textContent);
  assert.deepEqual(labels, ["A", "B", "C"]);
});

test("nudgeFirstBeat moves by one beat interval", () => {
  const el = makeEl();
  const c = createTrackDetailController(el);
  c.open({ track: { durationMs: 60000, bpm: 120, detailWaveform: pwv5Base64(200) }, firstBeatMs: 1000, cues: [], durationMs: 60000, bpm: 120 });
  c.nudgeFirstBeat(1);
  assert.equal(c.getWorking().firstBeatMs, 1500);
  c.nudgeFirstBeat(-1);
  assert.equal(c.getWorking().firstBeatMs, 1000);
});

test("openTrackDetail resolves id, fetches detail, then saves the edited payload", async () => {
  makeEl();
  const calls = [];
  const command = async (name, args) => {
    calls.push([name, args]);
    if (name === "get_track_detail") {
      return {
        track: { id: "local-1", title: "T", durationMs: 10000, bpm: 128 },
        firstBeatMs: 50,
        cues: [],
        detailWaveform: pwv5Base64(20),
      };
    }
    if (name === "save_track_analysis_edits") {
      return { trackId: "local-1", firstBeatMs: args.firstBeatMs, cues: args.cues, anlzRegenerated: true };
    }
    throw new Error(`unexpected ${name}`);
  };
  const savedPayload = { firstBeatMs: 60, cues: [{ positionMs: 2000, colorId: 5, name: "Drop" }] };
  const trackDetailDialog = { open: async () => savedPayload };
  const emitted = [];
  await openTrackDetail(
    { id: "row-1", title: "T" },
    { command, resolveLocalTrackIdAsync: async () => "local-1", trackDetailDialog, emitStatus: (m) => emitted.push(m) }
  );

  assert.deepEqual(calls[0], ["get_track_detail", { trackId: "local-1" }]);
  assert.equal(calls[1][0], "save_track_analysis_edits");
  assert.deepEqual(calls[1][1], { trackId: "local-1", firstBeatMs: 60, cues: savedPayload.cues });
  assert.match(emitted.join(" "), /Saved 1 cue/);
});

test("openTrackDetail bails when the track has no analysis waveform", async () => {
  makeEl();
  const emitted = [];
  let opened = false;
  await openTrackDetail(
    { id: "row-x" },
    {
      command: async () => ({ track: { id: "local-x" }, cues: [], detailWaveform: null }),
      resolveLocalTrackIdAsync: async () => "local-x",
      trackDetailDialog: { open: async () => { opened = true; return null; } },
      emitStatus: (m) => emitted.push(m),
    }
  );
  assert.equal(opened, false);
  assert.match(emitted.join(" "), /Analyze this track first/);
});
