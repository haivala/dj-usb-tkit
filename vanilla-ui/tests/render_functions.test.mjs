import test from "node:test";
import assert from "node:assert/strict";
import { renderTrackTable } from "../track_table.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

const baseTrack = {
  id: "t-1",
  title: "Title",
  artist: "Artist",
  album: "Album",
  bpm: "",
  key: "",
  waveformPreview: [],
  waveformPeaksPath: "",
  usbAnalysisPath: ""
};

const baseRowOptions = {
  origin: "lib",
  withCheckbox: false,
  actionLabel: "+",
  actionType: "add-library",
  compactAddButton: true,
  enableAnalyzeActions: false,
  secondaryActionLabel: "Play",
  secondaryActionType: "play-library"
};

// Renders one track through the real table renderer and returns its row.
async function renderRow(track = {}, options = {}, state = { currentPlaylistId: "playlist-1", playlists: [{ id: "playlist-1", name: "P1" }] }) {
  const ctx = makeTestCtx();
  Object.assign(ctx.state, state);
  const tbody = ctx.el.libraryTableBody;
  await renderTrackTable(ctx, tbody, [{ ...baseTrack, ...track }], { ...baseRowOptions, ...options });
  return tbody.querySelector(".track-grid-row");
}

function formatBadgeRow(track) {
  return renderRow(track, {
    origin: "usb",
    actionType: "add-usb",
    secondaryActionLabel: undefined,
    secondaryActionType: undefined
  }, {});
}

test("createTrackRow disables add with a helpful tooltip when no playlist is active", async () => {
  const row = await renderRow({ id: "t-3" }, {}, { currentPlaylistId: "", playlists: [] });
  const add = row.querySelector('[data-action="add-library"]');

  assert.equal(add.dataset.tooltip, "Create and activate a playlist first, then add tracks to it.");
  assert.equal(add.disabled, true);
});

test("createTrackRow analyze button reflects backend track.analysisReady", async () => {
  const notReady = (await renderRow({ id: "a-1", analysisReady: false }, { enableAnalyzeActions: true }))
    .querySelector('[data-action="analyze-track"]');
  assert.equal(notReady.textContent, "Analyze");
  assert.equal(notReady.dataset.tooltip, "Analyze missing waveform/BPM/key");

  const ready = (await renderRow({ id: "a-2", analysisReady: true }, { enableAnalyzeActions: true }))
    .querySelector('[data-action="analyze-track"]');
  assert.equal(ready.textContent, "Reanalyze");
  assert.equal(ready.dataset.tooltip, "Recompute waveform/BPM/key");
});

test("createTrackRow omits the analyze button on USB rows but keeps the cue editor", async () => {
  const row = await renderRow(
    { id: "u-1", analysisReady: true, usbAnalysisPath: "/PIONEER/USBANLZ/P001/0000A1B2/ANLZ0000.DAT" },
    { enableAnalyzeActions: true, origin: "usb" }
  );
  assert.equal(row.querySelector('[data-action="analyze-track"]'), null);
  assert.ok(row.querySelector('[data-action="edit-track-detail"]'));
});

test("createTrackRow renders a canvas for PWV4-only waveform data", async () => {
  const row = await renderRow({
    id: "t-pwv4",
    waveformColorData: [1, 2, 3, 4, 5, 6],
    waveformPeaksPath: "/tmp/ANLZ0000.EXT"
  });

  assert.ok(row.querySelector(".waveform.waveform-canvas"));
  assert.ok(row.querySelector(".waveform-canvas-el"));
});

test("renderTrackTable empty states use one full-width grid empty cell", async () => {
  for (const withCheckbox of [true, false]) {
    const ctx = makeTestCtx();
    const tbody = ctx.el.libraryTableBody;
    await renderTrackTable(ctx, tbody, [], { withCheckbox });
    const rows = tbody.querySelectorAll(".track-grid-row.track-grid-row-empty");
    assert.equal(rows.length, 1);
    assert.equal(tbody.querySelectorAll('[role="cell"]').length, 1);
    assert.equal(tbody.querySelector(".track-grid-cell.track-grid-empty").textContent, "No tracks available.");
  }
});

test("createTrackRow renders the format badge variant the backend's formatCompat asks for", async () => {
  const autofix = (await formatBadgeRow({
    filePath: "/media/track.wav",
    formatExt: "wav",
    formatCompat: {
      severity: "autofix",
      warning: "Uses an extended WAV header (WAVE_FORMAT_EXTENSIBLE) that some CDJs reject. Will be automatically converted to standard PCM on export."
    }
  })).querySelector(".format-badge");
  assert.equal(autofix.className, "format-badge autofix");
  assert.match(autofix.dataset.tooltip, /Will be automatically converted to standard PCM on export/);

  const warning = (await formatBadgeRow({
    filePath: "/media/track.wav",
    formatExt: "wav",
    formatCompat: {
      severity: "warn",
      warning: "Uses an extended WAV header with a non-standard subformat - cannot be safely converted and may not play on CDJ hardware."
    }
  })).querySelector(".format-badge");
  assert.equal(warning.className, "format-badge warn");
  assert.match(warning.dataset.tooltip, /cannot be safely converted/);

  const plain = (await formatBadgeRow({
    filePath: "/media/track.wav",
    formatExt: "wav",
    formatCompat: { severity: "ok", warning: null },
    sampleRateHz: 44100,
    bitDepth: 16
  })).querySelector(".format-badge");
  assert.equal(plain.className, "format-badge");
  assert.equal(plain.dataset.tooltip, "44.1 kHz · 16-bit");
  assert.equal(plain.textContent, "WAV");
});
