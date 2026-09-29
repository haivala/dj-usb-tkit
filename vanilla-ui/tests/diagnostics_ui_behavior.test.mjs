import test from "node:test";
import assert from "node:assert/strict";
import {
  clearUsbDiagnostics,
  hideUsbDiagnostics,
  renderRepairPreview
} from "../components/usb/actions.mjs";
import { makeClassList } from "./fixtures/dom.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

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
  const empty = renderPreview({
    detectedIssues: [],
    proposedFixes: [],
    estimatedFileWrites: 0,
    estimatedFileDeletes: 0,
    unsupportedItems: []
  }).el;
  assert.equal(empty.applyRepairsBtn.disabled, true);
  assert.equal(empty.previewRepairsBtn.disabled, true);
  assert.equal(empty.diagRepairSummary.textContent, "No issues found.");

  const { ctx, el, fixes } = renderPreview({
    detectedIssues: ["a"],
    proposedFixes: [
      { id: "fix_a", title: "Fix A", description: "desc", supported: true, destructive: false },
      { id: "fix_b", title: "Fix B", description: "desc", supported: true, destructive: false }
    ],
    estimatedFileWrites: 2,
    estimatedFileDeletes: 0,
    unsupportedItems: []
  });
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
    detectedIssues: ["unindexed", "missing-audio"],
    proposedFixes: [
      {
        id: "remove_missing_audio_references",
        title: "Remove Missing Audio References",
        description: "9 missing-audio reference(s) require manual review. Automatic removal is disabled while 13 canonical-path unindexed audio file(s) are present.",
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

  assert.match(el.diagRepairSummary.textContent, /2 issue\(s\).*0 fixable/);
  assert.equal(fixes.length, 2);
  assert.equal(fixTitle(fixes[0]), "Remove Missing Audio References");
  assert.match(
    fixDesc(fixes[0]),
    /9 missing-audio reference\(s\) require manual review.*13 canonical-path unindexed audio file\(s\)/
  );
  // The standalone unsupported item renders as its own row, unmodified.
  assert.equal(fixTitle(fixes[1]), "2 malformed USBANLZ entry/entries");
});

function makeHealthDot() {
  return {
    classList: makeClassList(),
    dataset: {},
    ariaLabel: "",
    setAttribute(name, value) {
      if (name === "aria-label") this.ariaLabel = value;
    }
  };
}

// A stand-in container: `replaceChildren()` empties its markup.
function container(innerHTML) {
  return { innerHTML, replaceChildren() { this.innerHTML = ""; } };
}

function makeDiagnosticsEl() {
  const healthCard = {
    classList: makeClassList(),
    open: true,
    removeAttribute(name) {
      if (name === "open") this.open = false;
    }
  };
  healthCard.classList.add("is-loading");
  const el = {
    usbHealthDot: makeHealthDot(),
    usbHeaderHealthDot: makeHealthDot(),
    usbDiagnosticsCard: {
      classList: makeClassList(),
      closest: (selector) => selector === "#usbHealthCard" ? healthCard : null
    },
    diagSections: container("<div>stale</div>"),
    diagOverallStatus: { textContent: "WARN", className: "diag-badge diag-warn" },
    diagDuration: { textContent: "Completed in 1ms" },
    diagPlaylistDetails: { classList: makeClassList() },
    diagPlaylistTableBody: container("<tr></tr>"),
    diagRepairSummary: { textContent: "stale summary", className: "diag-repair-summary is-bad" },
    diagRepairFixes: container("<div>stale fix</div>"),
    previewRepairsBtn: { disabled: false },
    applyRepairsBtn: { disabled: false },
    diagReportView: { classList: makeClassList() },
    diagRepairPanel: { classList: makeClassList() },
    _healthCard: healthCard
  };
  el.usbHealthDot.classList.add("health-warn");
  return el;
}

function assertDiagnosticsContentCleared(el) {
  for (const key of ["diagSections", "diagPlaylistTableBody", "diagRepairFixes"]) {
    assert.equal(el[key].innerHTML, "");
  }
  for (const key of ["diagOverallStatus", "diagDuration", "diagRepairSummary"]) {
    assert.equal(el[key].textContent, "");
  }
  assert.equal(el.previewRepairsBtn.disabled, true);
  assert.equal(el.applyRepairsBtn.disabled, true);
  assert.equal(el.diagPlaylistDetails.classList.contains("hidden"), true);
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
    const el = makeDiagnosticsEl();
    action({ el });
    assertDiagnosticsContentCleared(el);
    assert.equal(el.usbDiagnosticsCard.classList.contains("hidden"), cardHidden);
    assert.equal(el._healthCard.open, cardOpen);
    assert.equal(el._healthCard.classList.contains("is-loading"), loading);
  }
});
