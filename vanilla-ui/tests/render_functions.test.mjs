import test from "node:test";
import assert from "node:assert/strict";
import { buildTrackTableRow, finishTrackRows, renderEmptyTrackTable, setAddButtonState } from "../track_table.mjs";
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
  secondaryActionType: "play-library"
};

// Builds one track's row through the real table renderer and returns it.
async function renderRow(track = {}, options = {}, state = { currentPlaylistId: "playlist-1", playlists: [{ id: "playlist-1", name: "P1" }] }) {
  const ctx = makeTestCtx();
  Object.assign(ctx.state, state);
  const row = buildTrackTableRow(ctx, { ...baseTrack, ...track }, { ...baseRowOptions, ...options, index: 0 });
  ctx.el.libraryTableBody.append(row);
  await finishTrackRows(ctx, [row]);
  return row;
}

function formatBadgeRow(track) {
  return renderRow(track, {
    origin: "usb",
    actionType: "add-usb",
    secondaryActionType: undefined
  }, {});
}

test("createTrackRow disables add with a helpful tooltip when no playlist is active", async () => {
  const row = await renderRow({ id: "t-3" }, {}, { currentPlaylistId: "", playlists: [] });
  const add = row.querySelector('[data-action="add-library"]');

  assert.equal(add.dataset.tooltip, "Create and activate a playlist first, then add tracks to it.");
  assert.equal(add.disabled, true);
});

test("a row drawn before the playlists load gets its + once the active playlist is known", async () => {
  // The active playlist's id is restored at startup before its list loads.
  const state = { currentPlaylistId: "playlist-1", playlists: [] };
  const row = await renderRow({ id: "t-4" }, {}, state);
  const add = row.querySelector('[data-action="add-library"]');
  assert.equal(add.disabled, true);
  assert.equal(add.dataset.tooltip, "Create and activate a playlist first, then add tracks to it.");

  // The list loads (updateAddToPlaylistButtons redraws every row's + this way).
  setAddButtonState(add, { id: "playlist-1", name: "mauste" });
  assert.equal(add.disabled, false);
  assert.equal(add.dataset.tooltip, "Add to mauste");
});

test("createTrackRow analyze button reflects backend track.analysisReady", async () => {
  const notReady = (await renderRow({ id: "a-1", analysisReady: false }, { enableAnalyzeActions: true }))
    .querySelector('[data-action="analyze-track"]');
  assert.equal(notReady.textContent, "Analyze");
  assert.equal(notReady.dataset.tooltip, "Add the missing waveform, BPM, key and first beat; a BPM or key you edited or imported is kept");

  const ready = (await renderRow({ id: "a-2", analysisReady: true }, { enableAnalyzeActions: true }))
    .querySelector('[data-action="analyze-track"]');
  assert.equal(ready.textContent, "Reanalyze");
  assert.equal(ready.dataset.tooltip, "Reset waveform, BPM, key and first beat to a fresh analysis, replacing imported or edited values (cues are kept)");
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

test("renderEmptyTrackTable uses one full-width grid empty cell", () => {
  const ctx = makeTestCtx();
  const tbody = ctx.el.libraryTableBody;
  renderEmptyTrackTable(ctx, tbody);
  const rows = tbody.querySelectorAll(".track-grid-row.track-grid-row-empty");
  assert.equal(rows.length, 1);
  assert.equal(tbody.querySelectorAll('[role="cell"]').length, 1);
  assert.equal(tbody.querySelector(".track-grid-cell.track-grid-empty").textContent, "No tracks available.");
});

test("createTrackRow renders the format badge the backend's formatCompat describes", async () => {
  const autofix = (await formatBadgeRow({
    filePath: "/media/track.wav",
    formatExt: "wav",
    formatCompat: {
      severity: "autofix",
      warning: "Will be automatically converted to standard PCM on export.",
      badge: "WAV",
      tooltip: "Will be automatically converted to standard PCM on export."
    }
  })).querySelector(".format-badge");
  assert.equal(autofix.className, "format-badge autofix");
  assert.equal(autofix.textContent, "WAV ⟳");
  assert.match(autofix.dataset.tooltip, /Will be automatically converted to standard PCM on export/);

  const warning = (await formatBadgeRow({
    filePath: "/media/track.wav",
    formatExt: "wav",
    formatCompat: {
      severity: "warn",
      warning: "cannot be safely converted",
      badge: "WAV",
      tooltip: "44.1 kHz · 16-bit — cannot be safely converted"
    }
  })).querySelector(".format-badge");
  assert.equal(warning.className, "format-badge warn");
  assert.equal(warning.textContent, "WAV ⚠");
  assert.equal(warning.dataset.tooltip, "44.1 kHz · 16-bit — cannot be safely converted");

  const plain = (await formatBadgeRow({
    filePath: "/media/track.wav",
    formatExt: "wav",
    formatCompat: { severity: "ok", badge: "WAV" }
  })).querySelector(".format-badge");
  assert.equal(plain.className, "format-badge");
  assert.equal(plain.dataset.tooltip, undefined);
  assert.equal(plain.textContent, "WAV");
});
