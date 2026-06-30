import test from "node:test";
import assert from "node:assert/strict";

import { shouldUseBatchAnalysis } from "../components/library/actions.mjs";

test("shouldUseBatchAnalysis uses batch for multi-track by default", () => {
  assert.equal(shouldUseBatchAnalysis(2), true);
  assert.equal(shouldUseBatchAnalysis(10), true);
});

test("shouldUseBatchAnalysis uses piece mode for single-track", () => {
  assert.equal(shouldUseBatchAnalysis(1), false);
  assert.equal(shouldUseBatchAnalysis(0), false);
});

test("shouldUseBatchAnalysis respects explicit batchMode=false", () => {
  assert.equal(shouldUseBatchAnalysis(5, { batchMode: false }), false);
});
