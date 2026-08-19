import test from "node:test";
import assert from "node:assert/strict";
import { createTrackRow, renderTrackTable } from "../track_table.mjs";
import { escapeHtml } from "../ui_utils.mjs";

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
  index: 0,
  withCheckbox: false,
  actionLabel: "+",
  actionType: "add-library",
  compactAddButton: true,
  enableAnalyzeActions: false,
  secondaryActionLabel: "Play",
  secondaryActionType: "play-library"
};

function rowDeps(overrides = {}) {
  return {
    state: { currentPlaylistId: "playlist-1", playlists: [{ id: "playlist-1", name: "P1" }] },
    buildCoverSrcCandidates: () => [],
    isTrackCurrentlyPlaying: () => false,
    escapeHtml,
    trackHasCoreAnalysis: () => false,
    getKeyHue: () => 180,
    ...overrides
  };
}

function renderRow(track = {}, options = {}, deps = {}) {
  return createTrackRow(
    { ...baseTrack, ...track },
    { ...baseRowOptions, ...options },
    rowDeps(deps)
  );
}

function tableDeps(overrides = {}) {
  return {
    createTrackRow: () => "",
    attachCoverFallbackHandlers: () => {},
    renderWaveformsIn: () => {},
    updateTransportButtonsInDom: () => {},
    escapeHtml,
    setStatus: () => {},
    ...overrides
  };
}

function formatBadgeRow(track) {
  return renderRow(track, {
    origin: "usb",
    actionType: "add-usb",
    secondaryActionLabel: undefined,
    secondaryActionType: undefined
  }, {
    state: {},
    trackHasCoreAnalysis: () => false,
    getKeyHue: () => 0
  });
}

test("createTrackRow disables add with a helpful tooltip when no playlist is active", () => {
  const html = renderRow({ id: "t-3" }, {}, { state: { currentPlaylistId: "", playlists: [] } });

  assert.ok(html.includes("data-tooltip=\"Create and activate a playlist first, then add tracks to it.\""));
  assert.ok(html.includes("disabled"));
});

test("createTrackRow renders a canvas for PWV4-only waveform data", () => {
  const html = renderRow({
    id: "t-pwv4",
    waveformColorData: [1, 2, 3, 4, 5, 6],
    waveformPeaksPath: "/tmp/ANLZ0000.EXT"
  }, {}, {
    trackHasCoreAnalysis: () => true
  });

  assert.ok(html.includes("waveform waveform-canvas"));
  assert.ok(html.includes("waveform-canvas-el"));
});

test("renderTrackTable attaches PWV4 color data before drawing", () => {
  const waveformEl = {};
  const tbody = {
    innerHTML: "",
    lastElementChild: null,
    insertAdjacentHTML(_position, html) {
      this.innerHTML += html;
      this.lastElementChild = {
        querySelector: (selector) => selector === ".waveform" ? waveformEl : null
      };
    }
  };
  let attached = null;
  let rendered = false;
  const track = {
    ...baseTrack,
    id: "t-pwv4",
    waveformColorData: [1, 2, 3, 4, 5, 6],
    waveformPeaksPath: "/tmp/ANLZ0000.EXT"
  };

  renderTrackTable(tbody, [track], { origin: "lib" }, tableDeps({
    createTrackRow: (row, options) => createTrackRow(row, options, rowDeps({
      trackHasCoreAnalysis: () => true
    })),
    renderWaveformsIn: () => { rendered = true; },
    setWaveformColorData: (element, data) => { attached = { element, data }; }
  }));

  assert.equal(attached.element, waveformEl);
  assert.deepEqual(attached.data, [1, 2, 3, 4, 5, 6]);
  assert.equal(rendered, true);
});

test("renderTrackTable empty states use one full-width grid empty cell", () => {
  const playlistView = { innerHTML: "" };
  renderTrackTable(playlistView, [], { withCheckbox: false }, tableDeps());
  assert.ok(playlistView.innerHTML.includes('class="track-grid-row track-grid-row-empty"'));
  assert.ok(playlistView.innerHTML.includes('class="track-grid-cell track-grid-empty"'));
  assert.ok(playlistView.innerHTML.includes("No tracks available."));

  for (const withCheckbox of [true, false]) {
    const tbody = { innerHTML: "" };
    renderTrackTable(tbody, [], { withCheckbox }, tableDeps());
    assert.equal((tbody.innerHTML.match(/role="cell"/g) || []).length, 1);
  }
});

test("createTrackRow renders the expected wav format badge variants", () => {
  const autofix = formatBadgeRow({
    filePath: "/media/track.wav",
    formatExt: "wav",
    wavExtensibleKind: "extensible_pcm"
  });
  assert.ok(autofix.includes('class="format-badge autofix"'));
  assert.ok(!autofix.includes('class="format-badge warn"'));
  assert.ok(autofix.includes("Will be automatically converted to standard PCM on export"));

  const warning = formatBadgeRow({
    filePath: "/media/track.wav",
    formatExt: "wav",
    wavExtensibleKind: "extensible_other"
  });
  assert.ok(warning.includes('class="format-badge warn"'));
  assert.ok(!warning.includes('class="format-badge autofix"'));
  assert.ok(warning.includes("cannot be safely converted"));

  const plain = formatBadgeRow({
    filePath: "/media/track.wav",
    formatExt: "wav",
    wavExtensibleKind: null,
    sampleRateHz: 44100,
    bitDepth: 16
  });
  assert.ok(plain.includes('class="format-badge" data-tooltip="44.1 kHz \u00b7 16-bit">WAV</span>'));
  assert.ok(!plain.includes('class="format-badge warn"'));
  assert.ok(!plain.includes('class="format-badge autofix"'));
});
