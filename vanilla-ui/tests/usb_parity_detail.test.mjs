import test from "node:test";
import assert from "node:assert/strict";

import { renderParityReport } from "../components/usb/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

// The "Issues" badges are computed in Rust now
// (service::diagnostics::parity_issue_labels, tested there). The frontend just
// renders `UsbParityPlaylistDetail.issueLabels` straight through.

function renderIssues(playlistDetails) {
  const ctx = makeTestCtx();
  renderParityReport(ctx, { overallStatus: "WARN", durationMs: 3, checks: [], playlistDetails });
  return [...ctx.el.diagPlaylistTableBody.querySelectorAll("tr")].map((tr) => tr.lastElementChild.textContent);
}

test("renderParityReport shows the backend's issueLabels verbatim", () => {
  assert.deepEqual(
    renderIssues([{ name: "A", status: "WARN", matchedTracks: 1, issueLabels: ["+PDB 4", "order mismatch"] }]),
    ["+PDB 4, order mismatch"]
  );
});

test("renderParityReport leaves the issues cell empty when there are no labels", () => {
  assert.deepEqual(
    renderIssues([
      { name: "A", status: "PASS", matchedTracks: 1, issueLabels: [] },
      { name: "B", status: "PASS", matchedTracks: 1 }
    ]),
    ["", ""]
  );
});
