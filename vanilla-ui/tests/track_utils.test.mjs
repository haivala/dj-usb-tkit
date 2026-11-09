import test from "node:test";
import assert from "node:assert/strict";

import { formatBpm, formatDurationMs } from "../track_utils.mjs";

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
