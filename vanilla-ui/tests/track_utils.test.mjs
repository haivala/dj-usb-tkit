import test from "node:test";
import assert from "node:assert/strict";

import { formatBpm, formatDurationMs, renderTrackListDurationSummary } from "../track_utils.mjs";

test("formatDurationMs still rolls over to H:MM:SS past an hour (sanity check for reused formatter)", () => {
  assert.equal(formatDurationMs(3661000), "1:01:01");
});

test("formatBpm always renders two decimals and drops float noise", () => {
  assert.equal(formatBpm(128), "128.00");
  assert.equal(formatBpm(128.5), "128.50");
  assert.equal(formatBpm(128.499999), "128.50");
  assert.equal(formatBpm(87.25), "87.25");
  assert.equal(formatBpm("174"), "174.00");
});

test("formatBpm returns an empty string for missing or non-positive values", () => {
  assert.equal(formatBpm(null), "");
  assert.equal(formatBpm(undefined), "");
  assert.equal(formatBpm(0), "");
  assert.equal(formatBpm("not a number"), "");
});

test("renderTrackListDurationSummary renders the backend total verbatim", () => {
  const target = { textContent: "" };
  renderTrackListDurationSummary(target, { totalDurationMs: 3661000, durationUnknownCount: 0 });
  assert.equal(target.textContent, "Total time: 1:01:01");
});

test("renderTrackListDurationSummary appends a 'without length' suffix when some durations are unknown", () => {
  const target = { textContent: "" };
  renderTrackListDurationSummary(target, { totalDurationMs: 180000, durationUnknownCount: 2 });
  assert.equal(target.textContent, "Total time: 3:00 (2 without length)");
});

test("renderTrackListDurationSummary is a no-op with a missing target and tolerates missing fields", () => {
  assert.doesNotThrow(() => renderTrackListDurationSummary(null, { totalDurationMs: 1000 }));
  const target = { textContent: "unchanged" };
  renderTrackListDurationSummary(target);
  assert.equal(target.textContent, "Total time: 0:00");
});
