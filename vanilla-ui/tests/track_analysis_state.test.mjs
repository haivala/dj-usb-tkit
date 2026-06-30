import test from "node:test";
import assert from "node:assert/strict";
import {
  trackHasRenderableWaveform,
  trackHasArtwork,
  trackArtworkChecked,
  trackHasBpm,
  trackHasKey,
  trackHasCoreAnalysis,
  isUsbOriginTrack,
  resolveMissingAnalysisPieces,
  usbTrackNeedsHydration
} from "../components/library/actions.mjs";

test("waveform/artwork/bpm/key checks classify correctly", () => {
  assert.equal(trackHasRenderableWaveform({ waveformPreview: [0, 12] }), true);
  assert.equal(trackHasRenderableWaveform({ waveformPreview: [], waveformPeaksPath: "/a" }), true);
  assert.equal(trackHasRenderableWaveform({ waveformPreview: [0, 0], waveformPeaksPath: "" }), false);

  assert.equal(trackHasArtwork({ artworkUrl: "x" }), true);
  assert.equal(trackHasArtwork({}), false);
  assert.equal(trackHasArtwork({ artworkChecked: true }), false);
  assert.equal(trackArtworkChecked({ artworkChecked: true }), true);
  assert.equal(trackArtworkChecked({ artwork_checked: true }), true);

  assert.equal(trackHasBpm({ bpm: 120 }), true);
  assert.equal(trackHasBpm({ bpm: 0 }), false);

  assert.equal(trackHasKey({ key: "8A" }), true);
  assert.equal(trackHasKey({ key: "" }), false);
});

test("trackHasCoreAnalysis requires waveform+bpm+duration", () => {
  assert.equal(trackHasCoreAnalysis({ waveformPreview: [1], bpm: 120, durationMs: 1000 }), true);
  assert.equal(trackHasCoreAnalysis({ waveformPreview: [1], bpm: 0, durationMs: 1000 }), false);
  assert.equal(trackHasCoreAnalysis({ waveformPreview: [1], bpm: 120, durationMs: 0 }), false);
});

test("isUsbOriginTrack detects usbAnalysisPath and usb-root paths", () => {
  assert.equal(isUsbOriginTrack({ usbAnalysisPath: "/USBANLZ/1.DAT" }, { usbRoot: "/usb" }), true);
  assert.equal(isUsbOriginTrack({ filePath: "/usb/Contents/a.mp3" }, { usbRoot: "/usb" }), true);
  assert.equal(isUsbOriginTrack({ waveformPeaksPath: "/usb/ANLZ/a.DAT" }, { usbRoot: "/usb" }), true);
  assert.equal(isUsbOriginTrack({ filePath: "/music/a.mp3" }, { usbRoot: "/usb" }), false);
});

test("resolveMissingAnalysisPieces returns missing parts list", () => {
  assert.deepEqual(resolveMissingAnalysisPieces({}), ["duration", "artwork", "waveform", "bpm_key"]);
  assert.deepEqual(resolveMissingAnalysisPieces({ durationMs: 1000, artworkUrl: "x", waveformPreview: [10], bpm: 120 }), []);
  assert.deepEqual(resolveMissingAnalysisPieces({ durationMs: 1000, artworkChecked: true, waveformPreview: [10], bpm: 120 }), []);
});

test("usbTrackNeedsHydration true until all core pieces exist", () => {
  assert.equal(usbTrackNeedsHydration({}), true);
  assert.equal(usbTrackNeedsHydration({
    waveformPeaksPath: "/USB/PIONEER/USBANLZ/P001/TEST/ANLZ0000.DAT",
    waveformPreview: [],
    artworkUrl: "x",
    bpm: 120,
    key: "8A"
  }), true);
  assert.equal(usbTrackNeedsHydration({
    waveformPreview: [1],
    artworkUrl: "x",
    bpm: 120,
    key: "8A"
  }), false);
  assert.equal(usbTrackNeedsHydration({
    waveformPreview: [1],
    artworkChecked: true,
    bpm: 120,
    key: "8A"
  }), false);
});
