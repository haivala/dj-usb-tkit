import test from "node:test";
import assert from "node:assert/strict";
import {
  normalizeTrack,
  normalizeUsbPlaylist
} from "../components/library/actions.mjs";

test("normalizeTrack maps snake_case and clamps waveform preview", () => {
  const normalized = normalizeTrack({
    id: "1",
    local_track_id: "local-1",
    title: "Song",
    artist: "Artist",
    album: "Album",
    format_ext: "mp3",
    sample_rate_hz: "44100",
    bit_depth: "16",
    bitrate_kbps: "320",
    bpm_analyzer: "stratum",
    waveformPreview: [-10, 40, 500],
    filePath: "/music/song.mp3",
    updated_at: "2024-01-01T00:00:00Z"
  }, "x", {
    toPlayableUrl: (v) => v,
    appendUrlRevision: (url, rev) => `${url}?rev=${rev}`,
    normalizeDurationMs: () => 12345
  });

  assert.equal(normalized.id, "1");
  assert.equal(normalized.localTrackId, "local-1");
  assert.equal(normalized.sampleRateHz, 44100);
  assert.equal(normalized.bitDepth, 16);
  assert.equal(normalized.bitrateKbps, 320);
  assert.equal(normalized.bpmAnalyzer, "stratum");
  assert.deepEqual(normalized.waveformPreview, [0, 40, 100]);
  assert.equal(normalized.durationMs, 12345);
  assert.equal(normalized.searchText, "song artist album");
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

test("normalizeTrack creates fallback id when missing", () => {
  const normalized = normalizeTrack({
    title: "Song",
    artist: "Artist",
    filePath: "/music/song.flac"
  }, "lib", {
    randomId: () => "abc1234",
    normalizeDurationMs: () => null
  });
  assert.equal(normalized.id, "lib-abc1234");
  assert.equal(normalized.formatExt, "flac");
});

test("normalizeUsbPlaylist normalizes tracks and keeps max trackCount", () => {
  const playlist = normalizeUsbPlaylist({
    name: "USB Set",
    source: "pdb",
    track_count: 1,
    items: [{ id: "t1", title: "A", artist: "B", filePath: "/usb/a.mp3" }, { id: "t2", title: "C", artist: "D", filePath: "/usb/c.mp3" }]
  }, {
    normalizeTrack: (track) => ({ ...track, normalized: true })
  });

  assert.equal(playlist.source, "pdb");
  assert.equal(playlist.tracks.length, 2);
  assert.equal(playlist.trackCount, 2);
  assert.equal(playlist.tracks[0].normalized, true);
});
