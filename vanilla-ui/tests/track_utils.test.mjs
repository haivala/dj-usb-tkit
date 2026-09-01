import test from "node:test";
import assert from "node:assert/strict";

import { buildTracklistText, formatBpm, formatDurationMs, renderTrackListDurationSummary } from "../track_utils.mjs";

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

test("formatBpm renders a numeric bpm without forcing decimals and drops float noise", () => {
  assert.equal(formatBpm(128), "128");
  assert.equal(formatBpm(128.0), "128");
  assert.equal(formatBpm(128.5), "128.5");
  assert.equal(formatBpm(128.499999), "128.5");
  assert.equal(formatBpm("174"), "174");
});

test("formatBpm returns an empty string for missing or non-positive values", () => {
  assert.equal(formatBpm(null), "");
  assert.equal(formatBpm(undefined), "");
  assert.equal(formatBpm(0), "");
  assert.equal(formatBpm("not a number"), "");
});

test("renderTrackListDurationSummary renders the backend total verbatim", () => {
  const target = { textContent: "" };
  renderTrackListDurationSummary(target, { totalDurationMs: 3661000, durationKnownCount: 3, trackCount: 3 });
  assert.equal(target.textContent, "Total time: 1:01:01");
});

test("renderTrackListDurationSummary appends a 'without length' suffix when some durations are unknown", () => {
  const target = { textContent: "" };
  renderTrackListDurationSummary(target, { totalDurationMs: 180000, durationKnownCount: 1, trackCount: 3 });
  assert.equal(target.textContent, "Total time: 3:00 (2 without length)");
});

test("renderTrackListDurationSummary is a no-op with a missing target and tolerates missing fields", () => {
  assert.doesNotThrow(() => renderTrackListDurationSummary(null, { totalDurationMs: 1000 }));
  const target = { textContent: "unchanged" };
  renderTrackListDurationSummary(target);
  assert.equal(target.textContent, "Total time: 0:00");
});
