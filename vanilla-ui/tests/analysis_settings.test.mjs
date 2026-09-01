import test from "node:test";
import assert from "node:assert/strict";

import {
  DEFAULT_ANALYSIS_BPM_RANGE,
  normalizeAnalysisBpmRange
} from "../components/library/actions.mjs";

// The BPM-range string is parsed + validated in Rust
// (service::analysis::resolve_analysis_bpm_range, tested there). The frontend
// only guards the persisted setting's *format* so a corrupt localStorage value
// can't reach the dropdown.

test("normalizeAnalysisBpmRange keeps a well-formed range and rejects garbage", () => {
  assert.equal(normalizeAnalysisBpmRange("70-180"), "70-180");
  assert.equal(normalizeAnalysisBpmRange(" 88 - 175 "), "88 - 175");
  assert.equal(normalizeAnalysisBpmRange("not a range"), DEFAULT_ANALYSIS_BPM_RANGE);
  assert.equal(normalizeAnalysisBpmRange(""), DEFAULT_ANALYSIS_BPM_RANGE);
  assert.equal(normalizeAnalysisBpmRange(null), DEFAULT_ANALYSIS_BPM_RANGE);
});
