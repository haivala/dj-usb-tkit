import test from "node:test";
import assert from "node:assert/strict";

import { formatParityIssues } from "../components/usb/actions.mjs";

// The "Issues" badges are computed in Rust now
// (service::diagnostics::parity_issue_labels, tested there). The frontend just
// passes `UsbParityPlaylistDetail.issueLabels` straight through.

test("formatParityIssues returns the backend's issueLabels verbatim", () => {
  assert.deepEqual(
    formatParityIssues({ issueLabels: ["+PDB 4", "order mismatch"] }),
    ["+PDB 4", "order mismatch"]
  );
});

test("formatParityIssues returns an empty array when there are no labels", () => {
  assert.deepEqual(formatParityIssues({ issueLabels: [] }), []);
  assert.deepEqual(formatParityIssues({}), []);
  assert.deepEqual(formatParityIssues(null), []);
});
