import test from "node:test";
import assert from "node:assert/strict";

import { buildTracklistText, formatDurationMs } from "../track_utils.mjs";

test("buildTracklistText with timeMode off is plain Artist - Title lines", () => {
  const tracks = [
    { artist: "Artist A", title: "Title A", durationMs: 180000 },
    { artist: "Artist B", title: "Title B", durationMs: 200000 }
  ];
  assert.equal(buildTracklistText(tracks, "off"), "Artist A - Title A\nArtist B - Title B");
});

test("buildTracklistText with timeMode before prefixes cumulative time", () => {
  const tracks = [
    { artist: "Artist A", title: "Title A", durationMs: 180000 }, // 3:00
    { artist: "Artist B", title: "Title B", durationMs: 200000 }
  ];
  assert.equal(
    buildTracklistText(tracks, "before"),
    "0:00 Artist A - Title A\n3:00 Artist B - Title B"
  );
});

test("buildTracklistText with timeMode after suffixes cumulative time", () => {
  const tracks = [
    { artist: "Artist A", title: "Title A", durationMs: 180000 },
    { artist: "Artist B", title: "Title B", durationMs: 200000 }
  ];
  assert.equal(
    buildTracklistText(tracks, "after"),
    "Artist A - Title A - 0:00\nArtist B - Title B - 3:00"
  );
});

test("buildTracklistText treats missing duration as contributing zero to the running clock", () => {
  const tracks = [
    { artist: "Artist A", title: "Title A", durationMs: null },
    { artist: "Artist B", title: "Title B", durationMs: 120000 }
  ];
  assert.equal(
    buildTracklistText(tracks, "before"),
    "0:00 Artist A - Title A\n0:00 Artist B - Title B"
  );
});

test("buildTracklistText returns empty string for empty input", () => {
  assert.equal(buildTracklistText([], "before"), "");
  assert.equal(buildTracklistText(undefined, "off"), "");
});

test("formatDurationMs still rolls over to H:MM:SS past an hour (sanity check for reused formatter)", () => {
  assert.equal(formatDurationMs(3661000), "1:01:01");
});
