import test from "node:test";
import assert from "node:assert/strict";

test("export-block status can be derived from structured missing-analysis details", () => {
  const details = {
    validationType: "missing_analysis",
    missingTrackCount: 2,
    totalTrackCount: 5
  };
  const status = `Export blocked: ${details.missingTrackCount}/${details.totalTrackCount} track(s) need analysis. Use Analyze Missing Tracks.`;
  assert.equal(
    status,
    "Export blocked: 2/5 track(s) need analysis. Use Analyze Missing Tracks."
  );
});
