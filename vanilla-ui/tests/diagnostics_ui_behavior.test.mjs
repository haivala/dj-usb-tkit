import test from "node:test";
import assert from "node:assert/strict";
import {
  showDiagReportView, showDiagRepairView, renderRepairPreview,
  diagStatusIcon, renderDiagnosticsReport, renderParityReport
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
    children: [],
    _listeners: {},
    appendChild(node) { this.children.push(node); },
    addEventListener(event, handler) { this._listeners[event] = handler; },
    trigger(event, payload) {
      const fn = this._listeners[event];
      if (typeof fn === "function") fn(payload);
    }
  };
}

test("showDiagReportView and showDiagRepairView toggle diagnostic views", () => {
  const el = {
    diagReportView: { classList: makeClassList() },
    diagRepairPanel: { classList: makeClassList() }
  };

  showDiagRepairView(el);
  assert.equal(el.diagReportView.classList.contains("hidden"), true);
  assert.equal(el.diagRepairPanel.classList.contains("hidden"), false);

  showDiagReportView(el);
  assert.equal(el.diagReportView.classList.contains("hidden"), false);
  assert.equal(el.diagRepairPanel.classList.contains("hidden"), true);
});

test("renderRepairPreview enables apply when supported fixes exist", () => {
  const selection = new Set();
  const el = {
    usbDiagnosticsCard: { classList: makeClassList() },
    diagRepairPanel: { classList: makeClassList() },
    diagReportView: { classList: makeClassList() },
    diagRepairSummary: { textContent: "", className: "" },
    diagRepairFixes: { innerHTML: "", children: [], appendChild(node) { this.children.push(node); } },
    applyRepairsBtn: { disabled: true },
    previewRepairsBtn: { disabled: false }
  };

  renderRepairPreview(el, {
    detectedIssues: [{ issue: "a" }, { issue: "b" }],
    proposedFixes: [
      { id: "fix_a", title: "Fix A", description: "desc", supported: true, destructive: false, estimatedWrites: 2, estimatedDeletes: 0 },
      { id: "fix_b", title: "Fix B", description: "desc", supported: false, destructive: true, estimatedWrites: 0, estimatedDeletes: 1 }
    ],
    estimatedFileWrites: 2,
    estimatedFileDeletes: 1,
    unsupportedItems: [{ issue: "x", reason: "n/a" }]
  }, {
    documentObj: { createElement: (tag) => makeElement(tag) },
    showDiagRepairView: () => showDiagRepairView(el),
    getSelectedFixIds: () => selection,
    setSelectedFixIds: (ids) => {
      selection.clear();
      for (const id of ids) selection.add(id);
    }
  });

  assert.equal(el.usbDiagnosticsCard.classList.contains("hidden"), false);
  assert.match(el.diagRepairSummary.textContent, /2 issue\(s\)/);
  assert.equal(el.applyRepairsBtn.disabled, false);
  assert.equal(el.previewRepairsBtn.disabled, false);
  assert.equal(el.diagRepairFixes.children.length, 3);
});

test("renderRepairPreview disables apply when no supported fixes are selected", () => {
  const selection = new Set();
  const el = {
    usbDiagnosticsCard: { classList: makeClassList() },
    diagRepairPanel: { classList: makeClassList() },
    diagReportView: { classList: makeClassList() },
    diagRepairSummary: { textContent: "", className: "" },
    diagRepairFixes: { innerHTML: "", children: [], appendChild(node) { this.children.push(node); } },
    applyRepairsBtn: { disabled: true },
    previewRepairsBtn: { disabled: false }
  };

  renderRepairPreview(el, {
    detectedIssues: [{ issue: "a" }],
    proposedFixes: [
      { id: "fix_a", title: "Fix A", description: "desc", supported: true, destructive: false, estimatedWrites: 1, estimatedDeletes: 0 },
      { id: "fix_b", title: "Fix B", description: "desc", supported: true, destructive: false, estimatedWrites: 1, estimatedDeletes: 0 }
    ],
    estimatedFileWrites: 2,
    estimatedFileDeletes: 0,
    unsupportedItems: []
  }, {
    documentObj: { createElement: (tag) => makeElement(tag) },
    showDiagRepairView: () => showDiagRepairView(el),
    getSelectedFixIds: () => selection,
    setSelectedFixIds: (ids) => {
      selection.clear();
      for (const id of ids) selection.add(id);
    },
    onToggleFixSelection: (id, checked) => {
      if (checked) selection.add(id);
      else selection.delete(id);
      el.applyRepairsBtn.disabled = selection.size === 0;
    }
  });

  assert.equal(el.applyRepairsBtn.disabled, false);
  assert.deepEqual(Array.from(selection).sort(), ["fix_a", "fix_b"]);

  const firstFixInput = el.diagRepairFixes.children[0].children[0];
  const secondFixInput = el.diagRepairFixes.children[1].children[0];
  firstFixInput.trigger("change", { target: { checked: false } });
  secondFixInput.trigger("change", { target: { checked: false } });

  assert.equal(selection.size, 0);
  assert.equal(el.applyRepairsBtn.disabled, true);
});

test("renderRepairPreview disables apply and preview when there are no fixes", () => {
  const el = {
    usbDiagnosticsCard: { classList: makeClassList() },
    diagRepairPanel: { classList: makeClassList() },
    diagReportView: { classList: makeClassList() },
    diagRepairSummary: { textContent: "", className: "" },
    diagRepairFixes: { innerHTML: "", children: [], appendChild(node) { this.children.push(node); } },
    applyRepairsBtn: { disabled: false },
    previewRepairsBtn: { disabled: false }
  };

  renderRepairPreview(el, {
    detectedIssues: [],
    proposedFixes: [],
    estimatedFileWrites: 0,
    estimatedFileDeletes: 0,
    unsupportedItems: []
  }, {
    documentObj: { createElement: (tag) => makeElement(tag) },
    showDiagRepairView: () => showDiagRepairView(el)
  });

  assert.equal(el.applyRepairsBtn.disabled, true);
  assert.equal(el.previewRepairsBtn.disabled, true);
  assert.equal(el.diagRepairSummary.textContent, "No issues found.");
});

test("renderRepairPreview merges preview-only missing-audio manual review into one card", () => {
  const el = {
    usbDiagnosticsCard: { classList: makeClassList() },
    diagRepairPanel: { classList: makeClassList() },
    diagReportView: { classList: makeClassList() },
    diagRepairSummary: { textContent: "", className: "" },
    diagRepairFixes: { innerHTML: "", children: [], appendChild(node) { this.children.push(node); } },
    applyRepairsBtn: { disabled: true },
    previewRepairsBtn: { disabled: false }
  };

  renderRepairPreview(el, {
    detectedIssues: ["unindexed", "missing-audio"],
    proposedFixes: [
      {
        title: "Manual Re-import Unindexed Audio",
        description: "placeholder",
        supported: false,
        destructive: false,
        estimatedWrites: 0,
        estimatedDeletes: 0
      },
      {
        title: "Remove Missing Audio References",
        description: "placeholder",
        supported: false,
        destructive: false,
        estimatedWrites: 0,
        estimatedDeletes: 0
      }
    ],
    estimatedFileWrites: 0,
    estimatedFileDeletes: 0,
    unsupportedItems: [
      {
        issue: "13 unindexed audio file(s) under Contents",
        reason: "Automatic deletion is intentionally disabled."
      },
      {
        issue: "9 missing-audio reference(s) require manual review",
        reason: "Automatic removal is disabled while 13 unindexed audio file(s) are present."
      }
    ]
  }, {
    documentObj: { createElement: (tag) => makeElement(tag) },
    showDiagRepairView: () => showDiagRepairView(el)
  });

  assert.match(el.diagRepairSummary.textContent, /2 issue\(s\) · 0 fixable/);
  assert.equal(el.diagRepairFixes.children.length, 2);
  assert.equal(el.diagRepairFixes.children[1].children[0].children[0].children[0].textContent, "Remove Missing Audio References");
  assert.match(
    el.diagRepairFixes.children[1].children[0].children[1].textContent,
    /9 missing-audio reference\(s\) require manual review.*13 unindexed audio file\(s\)/
  );
});

// --- Coverage: diagStatusIcon ---

test("diagStatusIcon returns check for PASS", () => {
  assert.equal(diagStatusIcon("PASS"), "\u2713");
});

test("diagStatusIcon returns warning for WARN", () => {
  assert.equal(diagStatusIcon("WARN"), "\u26A0");
});

test("diagStatusIcon returns cross for FAIL", () => {
  assert.equal(diagStatusIcon("FAIL"), "\u2717");
});

test("diagStatusIcon returns cross for unknown status", () => {
  assert.equal(diagStatusIcon("UNKNOWN"), "\u2717");
});

// --- Coverage: renderDiagnosticsReport ---

function makeDiagEl() {
  const cl = () => {
    const classes = new Set();
    return {
      add(name) { classes.add(name); },
      remove(name) { classes.delete(name); },
      contains(name) { return classes.has(name); },
      toggle(name, force) { if (force) classes.add(name); else classes.delete(name); }
    };
  };
  return {
    usbDiagnosticsCard: { classList: cl() },
    diagReportView: { classList: cl() },
    diagRepairPanel: { classList: cl() },
    previewRepairsBtn: { disabled: true },
    diagOverallStatus: { textContent: "", className: "" },
    diagDuration: { textContent: "" },
    diagSections: { innerHTML: "", children: [], appendChild(n) { this.children.push(n); } },
    diagPlaylistDetails: {
      classList: cl(),
      querySelector: (sel) => {
        if (sel === "summary") return { textContent: "" };
        if (sel === "thead tr") return { innerHTML: "" };
        return null;
      }
    },
    diagPlaylistTableBody: { innerHTML: "", children: [], appendChild(n) { this.children.push(n); } }
  };
}

test("renderDiagnosticsReport populates overall status and sections", () => {
  const el = makeDiagEl();
  let healthDotStatus = null;
  renderDiagnosticsReport(el, {
    overallStatus: "WARN",
    durationMs: 55,
    pdbIntegrity: { title: "PDB Integrity", status: "PASS", checks: [{ label: "PDB exists", status: "PASS", detail: "Found" }] },
    edbAccess: { title: "Database Access", status: "PASS", checks: [] },
    contentsIntegrity: null,
    analysisIntegrity: null,
    playlistResolution: null,
    playlistDetails: [],
    warnings: []
  }, {
    escapeHtml: (s) => String(s),
    showDiagReportView: () => showDiagReportView(el),
    updateUsbHealthDot: (s) => { healthDotStatus = s; },
    documentObj: {
      getElementById: () => null,
      createElement: (tag) => makeElement(tag)
    }
  });

  assert.equal(el.diagOverallStatus.textContent, "WARN");
  assert.ok(el.diagOverallStatus.className.includes("diag-warn"));
  assert.ok(el.diagDuration.textContent.includes("55ms"));
  assert.equal(healthDotStatus, "WARN");
  assert.equal(el.previewRepairsBtn.disabled, false);
  assert.ok(!el.usbDiagnosticsCard.classList.contains("hidden"));
  // Two sections (pdbIntegrity, edbAccess) — contentsIntegrity/analysisIntegrity/playlistResolution are null
  assert.equal(el.diagSections.children.length, 2);
});

test("renderDiagnosticsReport renders playlist details table", () => {
  const el = makeDiagEl();
  renderDiagnosticsReport(el, {
    overallStatus: "PASS",
    durationMs: 10,
    pdbIntegrity: { title: "PDB", status: "PASS", checks: [] },
    playlistDetails: [
      { name: "Warmup", status: "PASS", resolvedEntries: 3, totalEntries: 3, resolutionRate: 1.0 }
    ],
    warnings: []
  }, {
    escapeHtml: (s) => String(s),
    showDiagReportView: () => showDiagReportView(el),
    updateUsbHealthDot: () => {},
    documentObj: {
      getElementById: () => null,
      createElement: (tag) => makeElement(tag)
    }
  });

  assert.ok(!el.diagPlaylistDetails.classList.contains("hidden"));
  assert.equal(el.diagPlaylistTableBody.children.length, 1);
});

test("renderDiagnosticsReport hides playlist details when empty", () => {
  const el = makeDiagEl();
  renderDiagnosticsReport(el, {
    overallStatus: "PASS",
    durationMs: 10,
    playlistDetails: [],
    warnings: []
  }, {
    escapeHtml: (s) => String(s),
    showDiagReportView: () => showDiagReportView(el),
    updateUsbHealthDot: () => {},
    documentObj: {
      getElementById: () => null,
      createElement: (tag) => makeElement(tag)
    }
  });

  assert.ok(el.diagPlaylistDetails.classList.contains("hidden"));
});

test("renderDiagnosticsReport renders player counter snapshot section when present", () => {
  const el = makeDiagEl();
  renderDiagnosticsReport(el, {
    overallStatus: "PASS",
    durationMs: 10,
    pdbIntegrity: { title: "PDB", status: "PASS", checks: [] },
    cdjCounterSnapshot: {
      confidence: "high",
      playlistCountCandidate: 2,
      songCountCandidate: 10,
      shapeMode: "additive",
      baselineInitLike: false,
      t00Tracks: 10,
      t08Entries: 12,
      t11: { first: 0, last: 0, ec: 0 },
      t12: { first: 0, last: 0, ec: 0 },
      t17: { first: 0, last: 0, ec: 0 },
      t18: { first: 0, last: 0, ec: 0 },
      t19: { ec: 1, chainLen: 1, dataPage: { page: 1, nrs: 1, numRl: 0, rowpf0: 0x0020, tranrf0: 0x0001 } }
    },
    playlistDetails: [],
    warnings: []
  }, {
    escapeHtml: (s) => String(s),
    showDiagReportView: () => showDiagReportView(el),
    updateUsbHealthDot: () => {},
    documentObj: {
      getElementById: () => null,
      createElement: (tag) => makeElement(tag)
    }
  });

  // pdbIntegrity + cdjCounterSnapshot = 2 sections
  assert.equal(el.diagSections.children.length, 2);
});

// --- Coverage: renderParityReport ---

test("renderParityReport populates overall status and checks", () => {
  const el = makeDiagEl();
  renderParityReport(el, {
    overallStatus: "FAIL",
    durationMs: 21,
    checks: [
      { label: "Overall player parity status", status: "FAIL", detail: "playlists checked: 1, fail: 1" }
    ],
    summaryRows: [
      { label: "Failing playlists", status: "FAIL", count: 1 },
      { label: "PDB metadata gaps", status: "FAIL", count: 1 }
    ],
    playlistDetails: [],
    warnings: []
  }, {
    escapeHtml: (s) => String(s),
    showDiagReportView: () => showDiagReportView(el),
    formatParityIssues: () => [],
    documentObj: { createElement: (tag) => makeElement(tag) }
  });

  assert.equal(el.diagOverallStatus.textContent, "FAIL");
  assert.ok(el.diagOverallStatus.className.includes("diag-fail"));
  assert.ok(el.diagDuration.textContent.includes("21ms"));
  assert.equal(el.diagSections.children.length, 1);
  assert.ok(el.diagPlaylistDetails.classList.contains("hidden"));
});

test("renderParityReport renders playlist detail rows with issues", () => {
  const el = makeDiagEl();
  renderParityReport(el, {
    overallStatus: "FAIL",
    durationMs: 10,
    checks: [],
    summaryRows: [],
    playlistDetails: [
      {
        name: "Warmup",
        status: "FAIL",
        pdbTracks: 3,
        edbTracks: 3,
        matchedTracks: 2,
        onlyInPdb: 1,
        onlyInEdb: 0,
        pdbMissingCoreMetadata: 1
      }
    ],
    warnings: []
  }, {
    escapeHtml: (s) => String(s),
    showDiagReportView: () => showDiagReportView(el),
    formatParityIssues: (pd) => pd.onlyInPdb ? ["+1 PDB only"] : [],
    documentObj: { createElement: (tag) => makeElement(tag) }
  });

  assert.ok(!el.diagPlaylistDetails.classList.contains("hidden"));
  assert.equal(el.diagPlaylistTableBody.children.length, 1);
});

test("renderParityReport handles missing summaryRows gracefully", () => {
  const el = makeDiagEl();
  renderParityReport(el, {
    overallStatus: "PASS",
    durationMs: 5,
    checks: [],
    playlistDetails: [],
    warnings: []
  }, {
    escapeHtml: (s) => String(s),
    showDiagReportView: () => showDiagReportView(el),
    formatParityIssues: () => [],
    documentObj: { createElement: (tag) => makeElement(tag) }
  });

  assert.equal(el.diagOverallStatus.textContent, "PASS");
  assert.equal(el.diagSections.children.length, 1);
});
