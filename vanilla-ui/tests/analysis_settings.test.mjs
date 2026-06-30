import test from "node:test";
import assert from "node:assert/strict";

import {
  DEFAULT_ANALYSIS_BPM_RANGE,
  normalizeAnalysisBpmRange,
  parseAnalysisBpmRange
} from "../components/library/actions.mjs";

test("parseAnalysisBpmRange parses valid range", () => {
  const parsed = parseAnalysisBpmRange("88-175");
  assert.equal(parsed.label, "88-175");
  assert.equal(parsed.min, 88);
  assert.equal(parsed.max, 175);
});

test("parseAnalysisBpmRange falls back to default for invalid values", () => {
  const parsed = parseAnalysisBpmRange("180-70");
  assert.equal(parsed.label, DEFAULT_ANALYSIS_BPM_RANGE);
});

test("normalizeAnalysisBpmRange keeps only known presets", () => {
  assert.equal(normalizeAnalysisBpmRange("70-180"), "70-180");
  assert.equal(normalizeAnalysisBpmRange("71-181"), DEFAULT_ANALYSIS_BPM_RANGE);
});
