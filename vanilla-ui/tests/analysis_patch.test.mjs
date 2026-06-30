import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  parseProgressWaveformPreview,
  patchTrackAnalysisFields,
  patchLibraryRowCells,
  createAnalysisPatchQueue
} from "../components/library/actions.mjs";

// ---------------------------------------------------------------------------
// parseProgressWaveformPreview
// ---------------------------------------------------------------------------

test("parseProgressWaveformPreview returns null for non-array input", () => {
  assert.equal(parseProgressWaveformPreview(null), null);
  assert.equal(parseProgressWaveformPreview(undefined), null);
  assert.equal(parseProgressWaveformPreview("peaks"), null);
  assert.equal(parseProgressWaveformPreview(42), null);
});

test("parseProgressWaveformPreview clamps values to 0-100", () => {
  const result = parseProgressWaveformPreview([-5, 0, 50, 100, 150]);
  assert.deepEqual(result, [0, 0, 50, 100, 100]);
});

test("parseProgressWaveformPreview clamps NaN to 0 and Infinity to 100", () => {
  const result = parseProgressWaveformPreview([10, NaN, 20, Infinity, 30]);
  assert.deepEqual(result, [10, 0, 20, 100, 30]);
});

test("parseProgressWaveformPreview returns empty array for empty input", () => {
  assert.deepEqual(parseProgressWaveformPreview([]), []);
});

// ---------------------------------------------------------------------------
// patchTrackAnalysisFields
// ---------------------------------------------------------------------------

test("patchTrackAnalysisFields returns false for null/missing inputs", () => {
  assert.equal(patchTrackAnalysisFields(null, {}, { toPlayableUrl: () => null }), false);
  assert.equal(patchTrackAnalysisFields({}, null, { toPlayableUrl: () => null }), false);
  assert.equal(patchTrackAnalysisFields({}, "string", { toPlayableUrl: () => null }), false);
});

test("patchTrackAnalysisFields patches bpm as formatted string", () => {
  const track = { bpm: "" };
  const deps = { toPlayableUrl: () => null };
  const changed = patchTrackAnalysisFields(track, { bpm: 128.0 }, deps);
  assert.equal(changed, true);
  assert.equal(track.bpm, "128");
});

test("patchTrackAnalysisFields formats bpm with decimals", () => {
  const track = { bpm: "" };
  const deps = { toPlayableUrl: () => null };
  patchTrackAnalysisFields(track, { bpm: 174.53 }, deps);
  assert.equal(track.bpm, "174.53");
});

test("patchTrackAnalysisFields ignores zero/negative/NaN bpm", () => {
  const track = { bpm: "120" };
  const deps = { toPlayableUrl: () => null };
  assert.equal(patchTrackAnalysisFields(track, { bpm: 0 }, deps), false);
  assert.equal(patchTrackAnalysisFields(track, { bpm: -5 }, deps), false);
  assert.equal(patchTrackAnalysisFields(track, { bpm: NaN }, deps), false);
  assert.equal(track.bpm, "120");
});

test("patchTrackAnalysisFields updating bpm does not clobber existing key", () => {
  const track = { bpm: "120", key: "C#m" };
  const deps = { toPlayableUrl: () => null };
  const changed = patchTrackAnalysisFields(track, { bpm: 128.4 }, deps);
  assert.equal(changed, true);
  assert.equal(track.bpm, "128.40");
  assert.equal(track.key, "C#m");
});

test("patchTrackAnalysisFields patches key", () => {
  const track = { key: "" };
  const deps = { toPlayableUrl: () => null };
  patchTrackAnalysisFields(track, { key: " Am " }, deps);
  assert.equal(track.key, "Am");
});

test("patchTrackAnalysisFields ignores blank key", () => {
  const track = { key: "Cm" };
  const deps = { toPlayableUrl: () => null };
  assert.equal(patchTrackAnalysisFields(track, { key: "  " }, deps), false);
  assert.equal(track.key, "Cm");
});

test("patchTrackAnalysisFields patches artworkPath and artworkUrl", () => {
  const track = { artworkPath: "", artworkUrl: "" };
  const deps = { toPlayableUrl: (p) => `asset://${p}` };
  patchTrackAnalysisFields(track, { artworkPath: "/covers/art.jpg" }, deps);
  assert.equal(track.artworkPath, "/covers/art.jpg");
  assert.match(track.artworkUrl, /^asset:\/\/\/covers\/art\.jpg\?rev=/);
  assert.equal(track.artworkChecked, true);
});

test("patchTrackAnalysisFields marks checked artwork as terminal without artwork", () => {
  const track = { artworkPath: "", artworkUrl: "", artworkChecked: false };
  const deps = { toPlayableUrl: () => null };
  const changed = patchTrackAnalysisFields(track, { artworkChecked: true }, deps);
  assert.equal(changed, true);
  assert.equal(track.artworkChecked, true);
  assert.equal(track.artworkPath, "");
  assert.equal(track.artworkUrl, "");
});

test("patchTrackAnalysisFields patches waveformPeaksPath", () => {
  const track = { waveformPeaksPath: "" };
  const deps = { toPlayableUrl: () => null };
  patchTrackAnalysisFields(track, { waveformPeaksPath: "/waves/t.bin" }, deps);
  assert.equal(track.waveformPeaksPath, "/waves/t.bin");
});

test("patchTrackAnalysisFields patches waveformPreview", () => {
  const track = { waveformPreview: [] };
  const deps = { toPlayableUrl: () => null };
  const changed = patchTrackAnalysisFields(track, { waveformPreview: [10, 50, 90] }, deps);
  assert.equal(changed, true);
  assert.deepEqual(track.waveformPreview, [10, 50, 90]);
});

test("patchTrackAnalysisFields returns false when preview is identical", () => {
  const track = { waveformPreview: [10, 50, 90] };
  const deps = { toPlayableUrl: () => null };
  const changed = patchTrackAnalysisFields(track, { waveformPreview: [10, 50, 90] }, deps);
  assert.equal(changed, false);
});

test("patchTrackAnalysisFields returns false when nothing changes", () => {
  const track = { bpm: "128", key: "Am" };
  const deps = { toPlayableUrl: () => null };
  const changed = patchTrackAnalysisFields(track, { bpm: 128.0, key: "Am" }, deps);
  assert.equal(changed, false);
});

test("patchTrackAnalysisFields patches multiple fields at once", () => {
  const track = { bpm: "", key: "", filePath: "", waveformPreview: [] };
  const deps = { toPlayableUrl: () => null };
  const changed = patchTrackAnalysisFields(track, {
    bpm: 140,
    key: "Dm",
    filePath: "/music/song.flac",
    waveformPreview: [5, 10, 15]
  }, deps);
  assert.equal(changed, true);
  assert.equal(track.bpm, "140");
  assert.equal(track.key, "Dm");
  assert.equal(track.filePath, "/music/song.flac");
  assert.deepEqual(track.waveformPreview, [5, 10, 15]);
});

// ---------------------------------------------------------------------------
// patchLibraryRowCells — JSDOM tests
// ---------------------------------------------------------------------------

function makeRowHtml(trackId, opts = {}) {
  const checkbox = opts.withCheckbox
    ? `<div role="cell" class="track-grid-cell td-select"><input type="checkbox" data-id="${trackId}" /></div>`
    : "";
  const cover = opts.coverSrc
    ? `<div role="cell" class="track-grid-cell td-cover"><img class="cover-thumb" alt="cover" src="${opts.coverSrc}" data-fallbacks="" /></div>`
    : `<div role="cell" class="track-grid-cell td-cover"><div class="cover-thumb" aria-hidden="true"></div></div>`;
  const waveformClass = opts.hasWaveform ? "waveform waveform-canvas" : "waveform";
  const canvas = opts.hasWaveform ? `<canvas class="waveform-canvas-el" aria-hidden="true"></canvas>` : "";
  const waveform = `<div role="cell" class="track-grid-cell td-waveform"><div class="waveform-cell"><div class="${waveformClass}" data-peaks="${opts.peaks || ""}">${canvas}<i class="waveform-playhead" aria-hidden="true"></i></div></div></div>`;
  const analyzeLabel = opts.analyzed ? "Reanalyze" : "Analyze";
  const action = `<div role="cell" class="track-grid-cell td-action"><div class="action-buttons"><button data-action="analyze-track" data-id="${trackId}">${analyzeLabel}</button></div></div>`;
  return `<div role="row" class="track-grid-row" data-track-id="${trackId}" data-track-origin="local">
    ${checkbox}${cover}${waveform}
    <div role="cell" class="track-grid-cell td-track">Title</div>
    <div role="cell" class="track-grid-cell td-album">Album</div>
    <div role="cell" class="track-grid-cell td-format">-</div>
    <div role="cell" class="track-grid-cell td-length">-</div>
    <div role="cell" class="track-grid-cell td-bpm">-</div>
    <div role="cell" class="track-grid-cell td-key">-</div>
    ${action}
  </div>`;
}

function makeDeps(overrides = {}) {
  return {
    escapeHtml: (v) => String(v ?? ""),
    getKeyHue: () => 270,
    buildCoverSrcCandidates: () => [],
    attachCoverFallbackHandlers: () => {},
    drawWaveformCanvas: () => {},
    trackHasCoreAnalysis: () => false,
    ...overrides
  };
}

test("jsdom: patchLibraryRowCells updates BPM cell", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "132", key: "", waveformPreview: [] };
  patchLibraryRowCells(row, track, makeDeps());
  const bpmTd = row.querySelector(".td-bpm");
  assert.ok(bpmTd.innerHTML.includes("132"));
  assert.ok(bpmTd.querySelector(".bpm-pill"));
});

test("jsdom: patchLibraryRowCells updates duration cell", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { durationMs: 180000, bpm: "", key: "", waveformPreview: [] };
  patchLibraryRowCells(row, track, makeDeps());
  const durationTd = row.querySelector(".td-length");
  assert.equal(durationTd.textContent, "3:00");
});

test("jsdom: patchLibraryRowCells duration formatting supports hours", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { durationMs: 3661000, bpm: "", key: "", waveformPreview: [] };
  patchLibraryRowCells(row, track, makeDeps());
  const durationTd = row.querySelector(".td-length");
  assert.equal(durationTd.textContent, "1:01:01");
});

test("jsdom: patchLibraryRowCells duration formatting renders '-' for invalid values", () => {
  const values = [0, -1, NaN];
  for (const value of values) {
    const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
    const row = dom.window.document.querySelector(".track-grid-row");
    const track = { durationMs: value, bpm: "", key: "", waveformPreview: [] };
    patchLibraryRowCells(row, track, makeDeps());
    const durationTd = row.querySelector(".td-length");
    assert.equal(durationTd.textContent, "-");
  }
});

test("jsdom: patchLibraryRowCells duration rounds sub-second values", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { durationMs: 1499, bpm: "", key: "", waveformPreview: [] };
  patchLibraryRowCells(row, track, makeDeps());
  const durationTd = row.querySelector(".td-length");
  assert.equal(durationTd.textContent, "0:01");
});

test("jsdom: patchLibraryRowCells shows dash for empty BPM", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "", key: "", waveformPreview: [] };
  patchLibraryRowCells(row, track, makeDeps());
  assert.equal(row.querySelector(".td-bpm").textContent, "-");
});

test("jsdom: patchLibraryRowCells updates key cell with hue", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "", key: "Am", waveformPreview: [] };
  patchLibraryRowCells(row, track, makeDeps({ getKeyHue: () => 180 }));
  const keyTd = row.querySelector(".td-key");
  const pill = keyTd.querySelector(".key-pill");
  assert.ok(pill);
  assert.equal(pill.textContent, "Am");
  assert.ok(pill.classList.contains("key-pill--h6"));
});

test("jsdom: patchLibraryRowCells updates existing cover img src", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1", { coverSrc: "/old.jpg" })}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "", key: "", waveformPreview: [] };
  patchLibraryRowCells(row, track, makeDeps({
    buildCoverSrcCandidates: () => ["/new.jpg", "/fallback.jpg"]
  }));
  const img = row.querySelector("img.cover-thumb");
  assert.equal(img.src, "/new.jpg");
  assert.equal(img.dataset.fallbacks, "/fallback.jpg");
});

test("jsdom: patchLibraryRowCells creates cover img when placeholder exists", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "", key: "", waveformPreview: [] };
  let fallbackCalls = 0;
  patchLibraryRowCells(row, track, makeDeps({
    buildCoverSrcCandidates: () => ["/art.png"],
    attachCoverFallbackHandlers: () => { fallbackCalls += 1; }
  }));
  const img = row.querySelector("img.cover-thumb");
  assert.ok(img);
  assert.equal(img.getAttribute("src"), "/art.png");
  assert.equal(fallbackCalls, 1);
});

test("jsdom: patchLibraryRowCells replaces stale cover img with placeholder when artwork is absent", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1", { coverSrc: "/old.jpg" })}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "", key: "", waveformPreview: [], artworkChecked: true };
  patchLibraryRowCells(row, track, makeDeps({
    buildCoverSrcCandidates: () => []
  }));
  assert.equal(row.querySelector("img.cover-thumb"), null);
  assert.ok(row.querySelector(".td-cover .cover-thumb"));
});

test("jsdom: patchLibraryRowCells updates waveform peaks and adds canvas class", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "", key: "", waveformPreview: [10, 50, 80] };
  let drawCalls = 0;
  patchLibraryRowCells(row, track, makeDeps({
    drawWaveformCanvas: () => { drawCalls += 1; }
  }));
  const waveform = row.querySelector(".waveform");
  assert.ok(waveform.classList.contains("waveform-canvas"));
  assert.equal(waveform.dataset.peaks, "10,50,80");
  assert.ok(waveform.querySelector("canvas.waveform-canvas-el"));
  assert.equal(drawCalls, 1);
});

test("jsdom: patchLibraryRowCells invalidates waveform cache before drawing", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "", key: "", waveformPreview: [10, 50, 80] };
  const invalidated = [];
  patchLibraryRowCells(row, track, makeDeps({
    invalidateWaveformCache: (el) => { invalidated.push(el); }
  }));
  const waveform = row.querySelector(".waveform");
  assert.equal(invalidated.length, 1);
  assert.equal(invalidated[0], waveform);
});

test("jsdom: patchLibraryRowCells does not add duplicate canvas on already-canvas waveform", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1", { hasWaveform: true, peaks: "1,2,3" })}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "", key: "", waveformPreview: [20, 40, 60] };
  patchLibraryRowCells(row, track, makeDeps());
  const canvases = row.querySelectorAll("canvas.waveform-canvas-el");
  assert.equal(canvases.length, 1);
  assert.equal(row.querySelector(".waveform").dataset.peaks, "20,40,60");
});

test("jsdom: patchLibraryRowCells skips waveform draw when peaks are empty", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "", key: "", waveformPreview: [] };
  let drawCalls = 0;
  patchLibraryRowCells(row, track, makeDeps({
    drawWaveformCanvas: () => { drawCalls += 1; }
  }));
  assert.equal(drawCalls, 0);
});

test("jsdom: patchLibraryRowCells skips waveform draw when all peaks are zero", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "", key: "", waveformPreview: [0, 0, 0] };
  let drawCalls = 0;
  patchLibraryRowCells(row, track, makeDeps({
    drawWaveformCanvas: () => { drawCalls += 1; }
  }));
  assert.equal(drawCalls, 0);
});

test("jsdom: patchLibraryRowCells updates Analyze to Reanalyze", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "120", key: "Am", waveformPreview: [10] };
  patchLibraryRowCells(row, track, makeDeps({ trackHasCoreAnalysis: () => true }));
  const btn = row.querySelector("[data-action='analyze-track']");
  assert.equal(btn.textContent, "Reanalyze");
  assert.equal(btn.title, "Recompute waveform/BPM/key");
});

test("jsdom: patchLibraryRowCells leaves Analyze label when not fully analyzed", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "120", key: "", waveformPreview: [] };
  patchLibraryRowCells(row, track, makeDeps({ trackHasCoreAnalysis: () => false }));
  const btn = row.querySelector("[data-action='analyze-track']");
  assert.equal(btn.textContent, "Analyze");
});

test("jsdom: patchLibraryRowCells handles checkbox row offset correctly", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1", { withCheckbox: true })}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "140", key: "Bb", waveformPreview: [50] };
  let drawCalls = 0;
  patchLibraryRowCells(row, track, makeDeps({
    buildCoverSrcCandidates: () => ["/cover.jpg"],
    drawWaveformCanvas: () => { drawCalls += 1; }
  }));
  // Cover is in cell[1] (after checkbox)
  const coverImg = row.querySelector(".td-cover img.cover-thumb");
  assert.ok(coverImg);
  assert.equal(coverImg.getAttribute("src"), "/cover.jpg");
  assert.equal(drawCalls, 1);
  assert.ok(row.querySelector(".td-bpm").innerHTML.includes("140"));
});

test("jsdom: patchLibraryRowCells returns false for null row", () => {
  assert.equal(patchLibraryRowCells(null, { bpm: "120" }, makeDeps()), false);
});

test("jsdom: patchLibraryRowCells returns false for row with fewer than 3 cells", () => {
  const dom = new JSDOM(`<div><div role="row"><div role="cell">a</div><div role="cell">b</div></div></div>`);
  const row = dom.window.document.querySelector('[role="row"]');
  assert.equal(patchLibraryRowCells(row, { bpm: "120" }, makeDeps()), false);
});

// ---------------------------------------------------------------------------
// createAnalysisPatchQueue
// ---------------------------------------------------------------------------

test("queue coalesces multiple track IDs into a single flush", () => {
  const patched = [];
  let fallbackCalled = false;
  let rafCallback = null;
  const fakeRaf = (cb) => { rafCallback = cb; return 1; };

  const queue = createAnalysisPatchQueue();
  queue.init(
    (id) => { patched.push(id); return true; },
    () => { fallbackCalled = true; },
    fakeRaf
  );

  queue.queue("t1");
  queue.queue("t2");
  queue.queue("t3");
  queue.queue("t1"); // duplicate — should be deduplicated by Set

  assert.equal(queue.pending.size, 3);
  assert.equal(queue.scheduled, true);

  // Simulate rAF callback
  rafCallback();

  assert.deepEqual(patched, ["t1", "t2", "t3"]);
  assert.equal(queue.pending.size, 0);
  assert.equal(queue.scheduled, false);
  assert.equal(fallbackCalled, false);
});

test("queue triggers fallback when patch returns false for any ID", () => {
  let fallbackCalled = false;
  let rafCallback = null;
  const fakeRaf = (cb) => { rafCallback = cb; return 1; };

  const queue = createAnalysisPatchQueue();
  queue.init(
    (id) => id !== "t2", // t2 is not in DOM
    () => { fallbackCalled = true; },
    fakeRaf
  );

  queue.queue("t1");
  queue.queue("t2");
  rafCallback();

  assert.equal(fallbackCalled, true);
});

test("queue does not schedule duplicate rAF when already scheduled", () => {
  let rafCalls = 0;
  const fakeRaf = (cb) => { rafCalls += 1; return rafCalls; };

  const queue = createAnalysisPatchQueue();
  queue.init((id) => true, () => {}, fakeRaf);

  queue.queue("t1");
  queue.queue("t2");
  queue.queue("t3");

  assert.equal(rafCalls, 1, "only one rAF should be scheduled");
});

test("queue allows new scheduling after flush completes", () => {
  let rafCalls = 0;
  let rafCallback = null;
  const fakeRaf = (cb) => { rafCalls += 1; rafCallback = cb; return rafCalls; };
  const patched = [];

  const queue = createAnalysisPatchQueue();
  queue.init((id) => { patched.push(id); return true; }, () => {}, fakeRaf);

  // First batch
  queue.queue("t1");
  assert.equal(rafCalls, 1);
  rafCallback();
  assert.deepEqual(patched, ["t1"]);

  // Second batch — should schedule a new rAF
  queue.queue("t2");
  assert.equal(rafCalls, 2);
  rafCallback();
  assert.deepEqual(patched, ["t1", "t2"]);
});

test("queue cancel clears pending and resets scheduled state", () => {
  let rafCallback = null;
  const fakeRaf = (cb) => { rafCallback = cb; return 1; };

  const queue = createAnalysisPatchQueue();
  queue.init((id) => true, () => {}, fakeRaf);

  queue.queue("t1");
  queue.queue("t2");
  assert.equal(queue.pending.size, 2);
  assert.equal(queue.scheduled, true);

  queue.cancel();
  assert.equal(queue.pending.size, 0);
  assert.equal(queue.scheduled, false);
});

test("queue flush can be called directly for synchronous testing", () => {
  const patched = [];
  const fakeRaf = (cb) => 1; // never call cb

  const queue = createAnalysisPatchQueue();
  queue.init((id) => { patched.push(id); return true; }, () => {}, fakeRaf);

  queue.queue("t1");
  queue.queue("t2");

  // Call flush manually
  queue.flush();
  assert.deepEqual(patched, ["t1", "t2"]);
  assert.equal(queue.pending.size, 0);
});

// ---------------------------------------------------------------------------
// Combined: patchTrackAnalysisFields + patchLibraryRowCells end-to-end
// ---------------------------------------------------------------------------

test("jsdom: simulated analysis event updates track state and DOM cells together", () => {
  const dom = new JSDOM(`<div>${makeRowHtml("t1")}</div>`);
  const row = dom.window.document.querySelector(".track-grid-row");
  const track = { bpm: "", key: "", waveformPreview: [], artworkPath: "", artworkUrl: "" };

  // Simulate an analysis event payload
  const eventPayload = {
    bpm: 128.5,
    key: "Am",
    waveformPreview: [10, 40, 80, 60, 30],
    artworkPath: "/covers/track1.jpg"
  };
  const deps = { toPlayableUrl: (p) => `asset://${p}` };

  // Step 1: patch track state
  const changed = patchTrackAnalysisFields(track, eventPayload, deps);
  assert.equal(changed, true);
  assert.equal(track.bpm, "128.50");
  assert.equal(track.key, "Am");
  assert.deepEqual(track.waveformPreview, [10, 40, 80, 60, 30]);
  assert.equal(track.artworkPath, "/covers/track1.jpg");

  // Step 2: patch DOM cells
  let drawCalls = 0;
  const domDeps = makeDeps({
    buildCoverSrcCandidates: () => ["/covers/track1.jpg"],
    drawWaveformCanvas: () => { drawCalls += 1; },
    trackHasCoreAnalysis: () => true
  });
  const result = patchLibraryRowCells(row, track, domDeps);
  assert.equal(result, true);

  assert.ok(row.querySelector(".td-bpm").innerHTML.includes("128.50"));
  const keyPill = row.querySelector(".td-key .key-pill");
  assert.ok(keyPill);
  assert.equal(keyPill.textContent, "Am");
  // Waveform drawn
  assert.equal(drawCalls, 1);
  // Button updated
  assert.equal(row.querySelector("[data-action='analyze-track']").textContent, "Reanalyze");
});
