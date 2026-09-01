import test from "node:test";
import assert from "node:assert/strict";
import {
  normalizeTrack,
  normalizeUsbPlaylist
} from "../components/library/actions.mjs";
import { normalizeDurationMs } from "../track_utils.mjs";

test("normalizeTrack maps the camelCase backend fields and clamps waveform preview", () => {
  const normalized = normalizeTrack({
    id: "1",
    localTrackId: "local-1",
    title: "Song",
    artist: "Artist",
    album: "Album",
    formatExt: "mp3",
    sampleRateHz: "44100",
    bitDepth: "16",
    bitrateKbps: "320",
    bpmAnalyzer: "stratum",
    durationMs: 12345,
    waveformPreview: [-10, 40, 500],
    filePath: "/music/song.mp3",
    updatedAt: "2024-01-01T00:00:00Z"
  }, "x", {
    toPlayableUrl: (v) => v,
    appendUrlRevision: (url, rev) => `${url}?rev=${rev}`,
    normalizeDurationMs
  });

  assert.equal(normalized.id, "1");
  assert.equal(normalized.localTrackId, "local-1");
  assert.equal(normalized.sampleRateHz, 44100);
  assert.equal(normalized.bitDepth, 16);
  assert.equal(normalized.bitrateKbps, 320);
  assert.equal(normalized.bpmAnalyzer, "stratum");
  assert.deepEqual(normalized.waveformPreview, [0, 40, 100]);
  assert.equal(normalized.durationMs, 12345);
});

test("normalizeDurationMs reads the canonical durationMs (ms) and rejects non-positive values", () => {
  assert.equal(normalizeDurationMs({ durationMs: 240000 }), 240000);
  assert.equal(normalizeDurationMs({ durationMs: 1234.6 }), 1235);
  assert.equal(normalizeDurationMs({ durationMs: 0 }), null);
  assert.equal(normalizeDurationMs({ durationMs: null }), null);
  assert.equal(normalizeDurationMs({}), null);
  assert.equal(normalizeDurationMs(null), null);
});

test("normalizeTrack maps camelCase bpmAnalyzer", () => {
  const normalized = normalizeTrack({
    id: "2",
    title: "Song B",
    artist: "Artist B",
    bpmAnalyzer: "essentia",
    filePath: "/music/song-b.wav"
  }, "x", {
    normalizeDurationMs: () => null
  });

  assert.equal(normalized.bpmAnalyzer, "essentia");
});

test("normalizeTrack creates fallback id when missing and passes formatExt through verbatim", () => {
  const normalized = normalizeTrack({
    title: "Song",
    artist: "Artist",
    filePath: "/music/song.flac",
    formatExt: "flac"
  }, "lib", {
    randomId: () => "abc1234",
    normalizeDurationMs: () => null
  });
  assert.equal(normalized.id, "lib-abc1234");
  assert.equal(normalized.formatExt, "flac");
  // The frontend no longer infers format from the path -- the backend always
  // populates formatExt, so an absent value stays empty.
  const noFormat = normalizeTrack({ title: "X", artist: "Y", filePath: "/a/b.mp3" }, "lib", {
    randomId: () => "z",
    normalizeDurationMs: () => null
  });
  assert.equal(noFormat.formatExt, "");
});

test("normalizeUsbPlaylist normalizes tracks and keeps max trackCount", () => {
  const playlist = normalizeUsbPlaylist({
    name: "USB Set",
    source: "pdb",
    trackCount: 1,
    items: [{ id: "t1", title: "A", artist: "B", filePath: "/usb/a.mp3" }, { id: "t2", title: "C", artist: "D", filePath: "/usb/c.mp3" }]
  }, {
    normalizeTrack: (track) => ({ ...track, normalized: true })
  });

  assert.equal(playlist.source, "pdb");
  assert.equal(playlist.tracks.length, 2);
  assert.equal(playlist.trackCount, 2);
  assert.equal(playlist.tracks[0].normalized, true);
});
