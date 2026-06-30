import test from "node:test";
import assert from "node:assert/strict";
import { sortTracks } from "../track_table.mjs";

test("sortTracks sorts BPM numerically", () => {
  const tracks = [
    { id: "a", bpm: "128.5" },
    { id: "b", bpm: "90" },
    { id: "c", bpm: "140" }
  ];
  const sorted = sortTracks(tracks, "bpm", "asc");
  assert.deepEqual(sorted.map((t) => t.id), ["b", "a", "c"]);
});

test("sortTracks sorts key lexicographically (case-insensitive locale)", () => {
  const tracks = [
    { id: "a", key: "Fm" },
    { id: "b", key: "Am" },
    { id: "c", key: "C#m" }
  ];
  const sorted = sortTracks(tracks, "key", "asc");
  assert.deepEqual(sorted.map((t) => t.id), ["b", "c", "a"]);
});

test("sortTracks sorts durationMs numerically and keeps null/undefined as zero", () => {
  const tracks = [
    { id: "a", durationMs: 180000 },
    { id: "b", durationMs: null },
    { id: "c", durationMs: 30000 },
    { id: "d" }
  ];
  const sorted = sortTracks(tracks, "durationMs", "asc");
  assert.deepEqual(sorted.map((t) => t.id), ["b", "d", "c", "a"]);
});

test("sortTracks artist sort uses title as stable tiebreak via original order preservation", () => {
  const tracks = [
    { id: "a", artist: "Same Artist", title: "Beta" },
    { id: "b", artist: "Same Artist", title: "Alpha" },
    { id: "c", artist: "Zed", title: "Track" }
  ];
  const sorted = sortTracks(tracks, "artist", "asc");
  assert.deepEqual(sorted.map((t) => t.id), ["b", "a", "c"]);
});
