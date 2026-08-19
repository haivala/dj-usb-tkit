import test from "node:test";
import assert from "node:assert/strict";
import {
  clearUsbDiagnostics,
  hideUsbDiagnostics,
  renderRepairPreview,
  showDiagRepairView
} from "../components/usb/actions.mjs";
import { makeClassList } from "./fixtures/dom.mjs";

function makeElement(tag = "div") {
  return {
    tagName: tag.toUpperCase(),
    className: "",
    classList: makeClassList(),
    textContent: "",
    innerHTML: "",
    dataset: {},
    type: "",
    checked: false,
    disabled: false,
    title: "",
    children: [],
    _listeners: {},
    appendChild(node) { this.children.push(node); },
    addEventListener(event, handler) { this._listeners[event] = handler; },
    trigger(event, payload) { this._listeners[event]?.(payload); }
  };
}

function makeRepairEl() {
  return {
    usbDiagnosticsCard: { classList: makeClassList() },
    diagRepairPanel: { classList: makeClassList() },
    diagReportView: { classList: makeClassList() },
    diagRepairSummary: { textContent: "", className: "" },
    diagRepairFixes: { innerHTML: "", children: [], appendChild(node) { this.children.push(node); } },
    applyRepairsBtn: { disabled: false },
    previewRepairsBtn: { disabled: false }
  };
}

function renderPreview(payload, deps = {}) {
  const el = makeRepairEl();
  const selection = deps.selection || new Set();
  renderRepairPreview(el, payload, {
    documentObj: { createElement: (tag) => makeElement(tag) },
    showDiagRepairView: () => showDiagRepairView(el),
    getSelectedFixIds: () => selection,
    setSelectedFixIds: (ids) => {
      selection.clear();
      for (const id of ids) selection.add(id);
    },
    ...deps
  });
  return { el, selection };
}

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

  const selected = new Set();
  let withFixes = null;
  withFixes = renderPreview({
    detectedIssues: ["a"],
    proposedFixes: [
      { id: "fix_a", title: "Fix A", description: "desc", supported: true, destructive: false },
      { id: "fix_b", title: "Fix B", description: "desc", supported: true, destructive: false }
    ],
    estimatedFileWrites: 2,
    estimatedFileDeletes: 0,
    unsupportedItems: []
  }, {
    selection: selected,
    onToggleFixSelection: (id, checked) => {
      if (checked) selected.add(id);
      else selected.delete(id);
      withFixes.el.applyRepairsBtn.disabled = selected.size === 0;
    }
  });
  assert.deepEqual(Array.from(selected).sort(), ["fix_a", "fix_b"]);
  assert.equal(withFixes.el.applyRepairsBtn.disabled, false);

  withFixes.el.diagRepairFixes.children[0].children[0].trigger("change", { target: { checked: false } });
  withFixes.el.diagRepairFixes.children[1].children[0].trigger("change", { target: { checked: false } });
  assert.equal(selected.size, 0);
  assert.equal(withFixes.el.applyRepairsBtn.disabled, true);
});

test("renderRepairPreview merges preview-only missing-audio manual review into one item", () => {
  const { el } = renderPreview({
    detectedIssues: ["unindexed", "missing-audio"],
    proposedFixes: [
      { title: "Manual Re-import Unindexed Audio", description: "placeholder", supported: false, destructive: false },
      { title: "Remove Missing Audio References", description: "placeholder", supported: false, destructive: false }
    ],
    estimatedFileWrites: 0,
    estimatedFileDeletes: 0,
    unsupportedItems: [
      { issue: "13 unindexed audio file(s) under Contents", reason: "Automatic deletion is intentionally disabled." },
      {
        issue: "9 missing-audio reference(s) require manual review",
        reason: "Automatic removal is disabled while 13 unindexed audio file(s) are present."
      }
    ]
  });

  assert.match(el.diagRepairSummary.textContent, /2 issue\(s\).*0 fixable/);
  assert.equal(el.diagRepairFixes.children.length, 2);
  assert.equal(el.diagRepairFixes.children[1].children[0].children[0].children[0].textContent, "Remove Missing Audio References");
  assert.match(
    el.diagRepairFixes.children[1].children[0].children[1].textContent,
    /9 missing-audio reference\(s\) require manual review.*13 unindexed audio file\(s\)/
  );
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
    diagSections: { innerHTML: "<div>stale</div>" },
    diagOverallStatus: { textContent: "WARN", className: "diag-badge diag-warn" },
    diagDuration: { textContent: "Completed in 1ms" },
    diagPlaylistDetails: { classList: makeClassList() },
    diagPlaylistTableBody: { innerHTML: "<tr></tr>" },
    diagRepairSummary: { textContent: "stale summary", className: "diag-repair-summary is-bad" },
    diagRepairFixes: { innerHTML: "<div>stale fix</div>" },
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
    action(el);
    assertDiagnosticsContentCleared(el);
    assert.equal(el.usbDiagnosticsCard.classList.contains("hidden"), cardHidden);
    assert.equal(el._healthCard.open, cardOpen);
    assert.equal(el._healthCard.classList.contains("is-loading"), loading);
  }
});
