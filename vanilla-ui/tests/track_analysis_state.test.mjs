import test from "node:test";
import assert from "node:assert/strict";
import {
  trackHasRenderableWaveform,
  trackHasArtwork,
  trackArtworkChecked,
  trackHasBpm,
  trackHasKey,
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
  assert.equal(trackArtworkChecked({}), false);

  assert.equal(trackHasBpm({ bpm: 120 }), true);
  assert.equal(trackHasBpm({ bpm: 0 }), false);

  assert.equal(trackHasKey({ key: "8A" }), true);
  assert.equal(trackHasKey({ key: "" }), false);
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
