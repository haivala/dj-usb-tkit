import test from "node:test";
import assert from "node:assert/strict";
import {
  clearUsbDiagnostics,
  hideUsbDiagnostics,
  renderRepairPreview
} from "../components/usb/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";
import { repairData, repairFix } from "./fixtures/backend_data.mjs";

function renderPreview(payload) {
  const ctx = makeTestCtx();
  ctx.el.applyRepairsBtn.disabled = false;
  ctx.el.previewRepairsBtn.disabled = false;
  renderRepairPreview(ctx, payload);
  const fixes = [...ctx.el.diagRepairFixes.querySelectorAll("li")];
  return { ctx, el: ctx.el, fixes };
}

const fixTitle = (li) => li.querySelector(".diag-repair-fix-title").textContent;
const fixDesc = (li) => li.querySelector(".diag-repair-fix-desc").textContent;
const fixCheck = (li) => li.querySelector(".diag-repair-fix-check");

test("renderRepairPreview handles no fixes and supported fix selection", () => {
  const empty = renderPreview(repairData({ summary: "No issues found." })).el;
  assert.equal(empty.applyRepairsBtn.disabled, true);
  assert.equal(empty.previewRepairsBtn.disabled, true);
  assert.equal(empty.diagRepairSummary.textContent, "No issues found.");
  assert.match(empty.diagRepairSummary.className, /diag-repair-summary-clean/);
  assert.equal(empty.diagRepairTechnical.classList.contains("hidden"), true);

  const { ctx, el, fixes } = renderPreview(repairData({
    summary: "2 fixes",
    overview: [{ title: "Playlists", status: "WARN", detail: "2 fixes" }],
    detectedIssues: ["a"],
    proposedFixes: [repairFix("fix_a"), repairFix("fix_b")],
    estimatedFileWrites: 2
  }));
  const selected = ctx.state.selectedRepairFixIds;
  assert.deepEqual(Array.from(selected).sort(), ["fix_a", "fix_b"]);
  assert.equal(el.applyRepairsBtn.disabled, false);

  for (const li of fixes) {
    fixCheck(li).checked = false;
    fixCheck(li).dispatchEvent(new ctx.window.Event("change"));
  }
  assert.equal(selected.size, 0);
  assert.equal(el.applyRepairsBtn.disabled, true);
});

test("renderRepairPreview renders each backend fix description verbatim and appends unsupported items", () => {
  // The backend now bakes the full "why this is manual-only" text into the
  // fix's `description` and doesn't emit a duplicate unsupportedItem for it.
  const { el, fixes } = renderPreview({
    summary: "Nothing here can be fixed automatically.",
    overview: [
      { title: "Needs your attention", status: "FAIL", detail: "2 problems can't be fixed automatically." }
    ],
    detectedIssues: ["unindexed", "missing-audio"],
    proposedFixes: [
      {
        id: "remove_missing_audio_references",
        title: "Remove Missing Audio References",
        description: "9 missing-audio references require manual review. Automatic removal is disabled while 13 canonical-path unindexed audio files are present.",
        supported: false,
        destructive: false
      }
    ],
    estimatedFileWrites: 0,
    estimatedFileDeletes: 0,
    unsupportedItems: [
      { issue: "2 malformed USBANLZ entry/entries", reason: "Inspect Event Log entries." }
    ]
  });

  // Headline and overview rows are backend text, rendered verbatim; the fix
  // list sits in the collapsed technical details.
  assert.equal(el.diagRepairSummary.textContent, "Nothing here can be fixed automatically.");
  const rows = [...el.diagRepairOverview.querySelectorAll(".diag-overview-item")];
  assert.equal(rows.length, 1);
  assert.ok(rows[0].classList.contains("diag-check-fail"));
  assert.equal(rows[0].querySelector(".diag-overview-title").textContent, "Needs your attention");
  assert.equal(el.diagRepairTechnical.open, false);
  assert.equal(el.diagRepairTechnical.classList.contains("hidden"), false);
  assert.equal(fixes.length, 2);
  assert.equal(fixTitle(fixes[0]), "Remove Missing Audio References");
  assert.match(
    fixDesc(fixes[0]),
    /9 missing-audio references require manual review.*13 canonical-path unindexed audio files/
  );
  // The standalone unsupported item renders as its own row, unmodified.
  assert.equal(fixTitle(fixes[1]), "2 malformed USBANLZ entry/entries");
});

// The real app context, with a stale report and an open, loading health card.
function makeDiagnosticsCtx() {
  const ctx = makeTestCtx();
  const { el } = ctx;
  el.usbHealthCard.setAttribute("open", "");
  el.usbHealthCard.classList.add("is-loading");
  el.usbHealthDot.classList.add("health-warn");
  el.diagSections.innerHTML = "<div>stale</div>";
  el.diagRepairFixes.innerHTML = "<div>stale fix</div>";
  el.diagOverallStatus.textContent = "WARN";
  el.diagDuration.textContent = "Completed in 1ms";
  el.diagRepairSummary.textContent = "stale summary";
  el.previewRepairsBtn.disabled = false;
  el.applyRepairsBtn.disabled = false;
  el.diagReportView.classList.add("hidden");
  el.diagRepairPanel.classList.remove("hidden");
  return ctx;
}

function assertDiagnosticsContentCleared(el) {
  for (const key of ["diagSections", "diagRepairFixes"]) {
    assert.equal(el[key].innerHTML, "");
  }
  for (const key of ["diagOverallStatus", "diagDuration", "diagRepairSummary"]) {
    assert.equal(el[key].textContent, "");
  }
  assert.equal(el.previewRepairsBtn.disabled, true);
  assert.equal(el.applyRepairsBtn.disabled, true);
  assert.equal(el.diagReportView.classList.contains("hidden"), false);
  assert.equal(el.diagRepairPanel.classList.contains("hidden"), true);
  assert.equal(el.usbHealthDot.classList.contains("health-warn"), false);
  assert.equal(el.usbHealthDot.dataset.tooltip, "USB health: unknown");
}

test("clearUsbDiagnostics and hideUsbDiagnostics blank content with different visibility effects", () => {
  for (const [action, cardHidden, cardOpen, loading] of [
    [clearUsbDiagnostics, false, true, true],
    [hideUsbDiagnostics, true, false, false]
  ]) {
    const ctx = makeDiagnosticsCtx();
    const { el } = ctx;
    el.usbDiagnosticsCard.classList.remove("hidden");
    action(ctx);
    assertDiagnosticsContentCleared(el);
    assert.equal(el.usbDiagnosticsCard.classList.contains("hidden"), cardHidden);
    assert.equal(el.usbHealthCard.open, cardOpen);
    assert.equal(el.usbHealthCard.classList.contains("is-loading"), loading);
  }
});
