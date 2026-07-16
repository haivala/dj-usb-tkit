import test from "node:test";
import assert from "node:assert/strict";
import { createTrackRow, renderTrackTable } from "../track_table.mjs";
import { escapeHtml } from "../ui_utils.mjs";

test("createTrackRow escapes HTML-sensitive track fields", () => {
  const html = createTrackRow(
    {
      id: "t-1",
      title: `<script>alert("x")</script>`,
      artist: "A&B",
      album: `\'"<>`,
      bpm: "",
      key: "",
      waveformPreview: [],
      waveformPeaksPath: "",
      usbAnalysisPath: ""
    },
    {
      origin: "usb",
      index: 0,
      withCheckbox: false,
      actionLabel: "+",
      actionType: "add-usb",
      compactAddButton: true,
      enableAnalyzeActions: false,
      secondaryActionLabel: "Play",
      secondaryActionType: "play-usb"
    },
    {
      state: { currentPlaylistId: "playlist-1" },
      buildCoverSrcCandidates: () => [],
      isTrackCurrentlyPlaying: () => false,
      escapeHtml,
      trackHasCoreAnalysis: () => false,
      getKeyHue: () => 270,
    }
  );

  assert.ok(html.includes("&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"));
  assert.ok(html.includes("A&amp;B"));
  assert.ok(html.includes("&#39;&quot;&lt;&gt;"));
  assert.equal(html.includes("<script>alert("), false);
});

test("createTrackRow does not emit inline style attributes", () => {
  const html = createTrackRow(
    {
      id: "t-2",
      title: "Title",
      artist: "Artist",
      album: "Album",
      bpm: "128",
      key: "Am",
      waveformPreview: [],
      waveformPeaksPath: "",
      usbAnalysisPath: ""
    },
    {
      origin: "lib",
      index: 0,
      withCheckbox: true,
      selectedIds: new Set(),
      actionLabel: "+",
      actionType: "add-library",
      compactAddButton: true,
      enableAnalyzeActions: true,
      secondaryActionLabel: "Play",
      secondaryActionType: "play-library"
    },
    {
      state: { currentPlaylistId: "playlist-1", playlists: [{ id: "playlist-1", name: "P1" }] },
      buildCoverSrcCandidates: () => [],
      isTrackCurrentlyPlaying: () => false,
      escapeHtml,
      trackHasCoreAnalysis: () => true,
      getKeyHue: () => 180,
    }
  );
  assert.equal(/style=/.test(html), false);
});

test("createTrackRow uses helpful add-button tooltip when no playlist is active", () => {
  const html = createTrackRow(
    {
      id: "t-3",
      title: "Title",
      artist: "Artist",
      album: "Album",
      bpm: "",
      key: "",
      waveformPreview: [],
      waveformPeaksPath: "",
      usbAnalysisPath: ""
    },
    {
      origin: "lib",
      index: 0,
      withCheckbox: false,
      actionLabel: "+",
      actionType: "add-library",
      compactAddButton: true,
      enableAnalyzeActions: false,
      secondaryActionLabel: "Play",
      secondaryActionType: "play-library"
    },
    {
      state: { currentPlaylistId: "", playlists: [] },
      buildCoverSrcCandidates: () => [],
      isTrackCurrentlyPlaying: () => false,
      escapeHtml,
      trackHasCoreAnalysis: () => false,
      getKeyHue: () => 180,
    }
  );

  assert.ok(html.includes("title=\"Create and activate a playlist first, then add tracks to it.\""));
  assert.ok(html.includes("disabled"));
});

test("createTrackRow renders a canvas for PWV4-only waveform data", () => {
  const html = createTrackRow(
    {
      id: "t-pwv4",
      title: "Title",
      artist: "Artist",
      album: "Album",
      bpm: "",
      key: "",
      waveformPreview: [],
      waveformColorData: [1, 2, 3, 4, 5, 6],
      waveformPeaksPath: "/tmp/ANLZ0000.EXT",
      usbAnalysisPath: ""
    },
    {
      origin: "lib",
      index: 0,
      withCheckbox: false,
      actionLabel: "+",
      actionType: "add-library",
      compactAddButton: true,
      enableAnalyzeActions: false,
      secondaryActionLabel: "Play",
      secondaryActionType: "play-library"
    },
    {
      state: { currentPlaylistId: "playlist-1", playlists: [{ id: "playlist-1", name: "P1" }] },
      buildCoverSrcCandidates: () => [],
      isTrackCurrentlyPlaying: () => false,
      escapeHtml,
      trackHasCoreAnalysis: () => true,
      getKeyHue: () => 180,
    }
  );

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
    id: "t-pwv4",
    title: "Title",
    artist: "Artist",
    album: "Album",
    bpm: "",
    key: "",
    waveformPreview: [],
    waveformColorData: [1, 2, 3, 4, 5, 6],
    waveformPeaksPath: "/tmp/ANLZ0000.EXT",
    usbAnalysisPath: ""
  };

  renderTrackTable(tbody, [track], { origin: "lib" }, {
    createTrackRow: (row, options) => createTrackRow(row, options, {
      state: { currentPlaylistId: "playlist-1", playlists: [{ id: "playlist-1", name: "P1" }] },
      buildCoverSrcCandidates: () => [],
      isTrackCurrentlyPlaying: () => false,
      escapeHtml,
      trackHasCoreAnalysis: () => true,
      getKeyHue: () => 180,
    }),
    attachCoverFallbackHandlers: () => {},
    renderWaveformsIn: () => { rendered = true; },
    setWaveformColorData: (element, data) => { attached = { element, data }; },
    updateTransportButtonsInDom: () => {},
    escapeHtml,
    setStatus: () => {}
  });

  assert.equal(attached.element, waveformEl);
  assert.deepEqual(attached.data, [1, 2, 3, 4, 5, 6]);
  assert.equal(rendered, true);
});

test("renderTrackTable empty state renders grid empty row in both selection modes", () => {
  const deps = {
    createTrackRow: () => "",
    attachCoverFallbackHandlers: () => {},
    renderWaveformsIn: () => {},
    updateTransportButtonsInDom: () => {},
    escapeHtml,
    setStatus: () => {}
  };

  const tbodyA = { innerHTML: "" };
  renderTrackTable(tbodyA, [], { withCheckbox: true }, deps);
  assert.ok(tbodyA.innerHTML.includes('class="track-grid-row track-grid-row-empty"'));
  assert.ok(tbodyA.innerHTML.includes('class="track-grid-cell track-grid-empty"'));
  assert.ok(tbodyA.innerHTML.includes("No tracks available."));

  const tbodyB = { innerHTML: "" };
  renderTrackTable(tbodyB, [], { withCheckbox: false }, deps);
  assert.ok(tbodyB.innerHTML.includes('class="track-grid-row track-grid-row-empty"'));
  assert.ok(tbodyB.innerHTML.includes('class="track-grid-cell track-grid-empty"'));
  assert.ok(tbodyB.innerHTML.includes("No tracks available."));
});

test("renderTrackTable empty row keeps a single full-width cell regardless of checkbox mode", () => {
  const deps = {
    createTrackRow: () => "",
    attachCoverFallbackHandlers: () => {},
    renderWaveformsIn: () => {},
    updateTransportButtonsInDom: () => {},
    escapeHtml,
    setStatus: () => {}
  };

  const withCheckbox = { innerHTML: "" };
  renderTrackTable(withCheckbox, [], { withCheckbox: true }, deps);
  const cellsA = (withCheckbox.innerHTML.match(/role="cell"/g) || []).length;
  assert.equal(cellsA, 1);

  const withoutCheckbox = { innerHTML: "" };
  renderTrackTable(withoutCheckbox, [], { withCheckbox: false }, deps);
  const cellsB = (withoutCheckbox.innerHTML.match(/role="cell"/g) || []).length;
  assert.equal(cellsB, 1);
});

function renderFormatBadgeRow(track) {
  return createTrackRow(
    {
      id: "t-format",
      title: "Title",
      artist: "Artist",
      album: "Album",
      bpm: "",
      key: "",
      waveformPreview: [],
      waveformPeaksPath: "",
      usbAnalysisPath: "",
      ...track
    },
    {
      origin: "usb",
      index: 0,
      withCheckbox: false,
      actionLabel: "+",
      actionType: "add-usb",
      compactAddButton: true,
      enableAnalyzeActions: false
    },
    {
      state: {},
      buildCoverSrcCandidates: () => [],
      isTrackCurrentlyPlaying: () => false,
      escapeHtml,
      trackHasCoreAnalysis: () => false,
      getKeyHue: () => 0
    }
  );
}

test("createTrackRow shows an autofix badge for WAVE_FORMAT_EXTENSIBLE PCM wav tracks", () => {
  const html = renderFormatBadgeRow({
    filePath: "/media/track.wav",
    formatExt: "wav",
    wavExtensibleKind: "extensible_pcm"
  });

  assert.ok(html.includes('class="format-badge autofix"'));
  assert.ok(!html.includes('class="format-badge warn"'));
  assert.ok(html.includes("Will be automatically converted to standard PCM on export"));
});

test("createTrackRow keeps a hard warning badge for extensible wav with an unsafe subformat", () => {
  const html = renderFormatBadgeRow({
    filePath: "/media/track.wav",
    formatExt: "wav",
    wavExtensibleKind: "extensible_other"
  });

  assert.ok(html.includes('class="format-badge warn"'));
  assert.ok(!html.includes('class="format-badge autofix"'));
  assert.ok(html.includes("cannot be safely converted"));
});

test("createTrackRow shows a plain badge for a wav with no extensible header issue", () => {
  const html = renderFormatBadgeRow({
    filePath: "/media/track.wav",
    formatExt: "wav",
    wavExtensibleKind: null,
    sampleRateHz: 44100,
    bitDepth: 16
  });

  assert.ok(html.includes('class="format-badge">WAV</span>'));
  assert.ok(!html.includes('class="format-badge warn"'));
  assert.ok(!html.includes('class="format-badge autofix"'));
});
