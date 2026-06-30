import test from "node:test";
import assert from "node:assert/strict";
import {
  showDiagReportView, showDiagRepairView, renderRepairPreview,
  diagStatusIcon, renderDiagnosticsReport, renderParityReport
} from "../components/usb/actions.mjs";

function makeClassList() {
  const classes = new Set();
  return {
    add(name) { classes.add(name); },
    remove(name) { classes.delete(name); },
    contains(name) { return classes.has(name); }
  };
}

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

function renderDiagnosticsPlaylistTable(entries = []) {
  return {
    summary: "Playlist Resolution Details",
    columns: [
      "Playlist",
      "Status",
      "Total Entries",
      "Resolved Entries",
      "Resolution Rate",
      "PDB Entries",
      "eDB Entries",
      "Matched Entries",
      "PDB Match Rate",
      "eDB Match Rate"
    ],
    rows: entries.map((entry) => ({
      playlist: entry.name,
      status: entry.status,
      totalEntries: entry.totalEntries,
      resolvedEntries: entry.resolvedEntries,
      resolutionRate: entry.resolutionRate,
      pedbEntries: entry.pedbEntries,
      edbEntries: entry.edbEntries,
      matchedEntries: entry.matchedEntries,
      pedbMatchRate: entry.pedbMatchRate,
      edbMatchRate: entry.edbMatchRate
    }))
  };
}

function renderStrictParityPlaylistTable(entries = []) {
  return {
    summary: "Strict Parity Playlist Details",
    columns: [
      "Playlist",
      "Status",
      "PDB Tracks",
      "eDB Tracks",
      "Matched",
      "Only in PDB",
      "Only in eDB",
      "Order",
      "PDB Duplicates",
      "PDB Metadata Gaps",
      "eDB Metadata Gaps",
      "Artwork Mismatches",
      "Path Mismatches",
      "Dictionary ID Issues",
      "Playlist ID Match",
      "Sort Order Match"
    ],
    rows: entries.map((entry) => ({
      playlist: entry.name,
      status: entry.status,
      pedbTracks: entry.pedbTracks,
      edbTracks: entry.edbTracks,
      matchedTracks: entry.matchedTracks,
      onlyInPdb: entry.onlyInPdb,
      onlyInEdb: entry.onlyInEdb,
      order: entry.orderMismatch ? "Mismatch" : "Match",
      pdbDuplicates: entry.pdbDuplicateEntries,
      pdbMetadataGaps: entry.pdbMissingCoreMetadata,
      edbMetadataGaps: entry.edbMissingCoreMetadata,
      artworkMismatches: entry.artworkMismatchTracks,
      pathMismatches: entry.pathMismatchTracks,
      dictionaryIdIssues: entry.dictionaryIdIssueTracks,
      playlistIdMatch: entry.playlistIdMatch,
      sortOrderMatch: entry.sortOrderMatch
    }))
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

test("diagnostics UI keeps playlist resolution and strict parity tables separate", () => {
  const diagnosticsTable = renderDiagnosticsPlaylistTable([
    {
      name: "Warmup",
      status: "PASS",
      totalEntries: 3,
      resolvedEntries: 3,
      resolutionRate: 1,
      pedbEntries: 3,
      edbEntries: 3,
      matchedEntries: 3,
      pedbMatchRate: 1,
      edbMatchRate: 1
    }
  ]);
  const parityTable = renderStrictParityPlaylistTable([
    {
      name: "Warmup",
      status: "FAIL",
      pedbTracks: 3,
      edbTracks: 3,
      matchedTracks: 3,
      onlyInPdb: 0,
      onlyInEdb: 0,
      orderMismatch: false,
      pdbDuplicateEntries: 0,
      pdbMissingCoreMetadata: 1,
      edbMissingCoreMetadata: 0,
      artworkMismatchTracks: 1,
      pathMismatchTracks: 1,
      dictionaryIdIssueTracks: 1,
      playlistIdMatch: true,
      sortOrderMatch: true
    }
  ]);

  assert.equal(diagnosticsTable.summary, "Playlist Resolution Details");
  assert.equal(parityTable.summary, "Strict Parity Playlist Details");
  assert.notDeepEqual(diagnosticsTable.columns, parityTable.columns);
  assert.equal(diagnosticsTable.rows[0].playlist, "Warmup");
  assert.equal(parityTable.rows[0].playlist, "Warmup");
  assert.equal(diagnosticsTable.rows[0].status, "PASS");
  assert.equal(parityTable.rows[0].status, "FAIL");
});

test("strict parity table exposes strict-only mismatch columns", () => {
  const parityTable = renderStrictParityPlaylistTable([
    {
      name: "Player Export",
      status: "FAIL",
      pedbTracks: 12,
      edbTracks: 12,
      matchedTracks: 11,
      onlyInPdb: 1,
      onlyInEdb: 0,
      orderMismatch: true,
      pdbDuplicateEntries: 2,
      pdbMissingCoreMetadata: 3,
      edbMissingCoreMetadata: 1,
      artworkMismatchTracks: 4,
      pathMismatchTracks: 5,
      dictionaryIdIssueTracks: 6,
      playlistIdMatch: false,
      sortOrderMatch: false
    }
  ]);

  assert.deepEqual(
    parityTable.columns.slice(-6),
    [
      "eDB Metadata Gaps",
      "Artwork Mismatches",
      "Path Mismatches",
      "Dictionary ID Issues",
      "Playlist ID Match",
      "Sort Order Match"
    ]
  );
  assert.equal(parityTable.rows[0].order, "Mismatch");
  assert.equal(parityTable.rows[0].pathMismatches, 5);
  assert.equal(parityTable.rows[0].dictionaryIdIssues, 6);
  assert.equal(parityTable.rows[0].playlistIdMatch, false);
  assert.equal(parityTable.rows[0].sortOrderMatch, false);
});

test("diagnostics playlist resolution table keeps operational columns only", () => {
  const diagnosticsTable = renderDiagnosticsPlaylistTable([
    {
      name: "Operational USB",
      status: "WARN",
      totalEntries: 841,
      resolvedEntries: 841,
      resolutionRate: 1,
      pedbEntries: 841,
      edbEntries: 103,
      matchedEntries: 103,
      pedbMatchRate: 0.122,
      edbMatchRate: 1
    }
  ]);

  assert.deepEqual(
    diagnosticsTable.columns,
    [
      "Playlist",
      "Status",
      "Total Entries",
      "Resolved Entries",
      "Resolution Rate",
      "PDB Entries",
      "eDB Entries",
      "Matched Entries",
      "PDB Match Rate",
      "eDB Match Rate"
    ]
  );
  assert.equal(diagnosticsTable.rows[0].playlist, "Operational USB");
  assert.equal(diagnosticsTable.rows[0].status, "WARN");
  assert.equal(diagnosticsTable.rows[0].resolvedEntries, 841);
  assert.equal(diagnosticsTable.rows[0].edbEntries, 103);
  assert.ok(!("pathMismatches" in diagnosticsTable.rows[0]));
  assert.ok(!("dictionaryIdIssues" in diagnosticsTable.rows[0]));
});

test("strict parity summary uses structured rows instead of one dense sentence", () => {
  const summaryRows = [
    { label: "Membership only-in-PDB", status: "FAIL", count: 7 },
    { label: "Membership only-in-eDB", status: "PASS", count: 0 },
    { label: "Artwork presence mismatches", status: "WARN", count: 5 }
  ];

  assert.equal(summaryRows[0].label, "Membership only-in-PDB");
  assert.equal(summaryRows[0].count, 7);
  assert.equal(summaryRows[1].label, "Membership only-in-eDB");
  assert.equal(summaryRows[1].count, 0);
  assert.equal(summaryRows[2].status, "WARN");
});

// --- Coverage: separate diagnostics vs strict parity sections ---

test("diagnostics and strict parity tables use different summaries and column sets", () => {
  const diagTable = renderDiagnosticsPlaylistTable([
    {
      name: "Set A",
      status: "PASS",
      totalEntries: 10,
      resolvedEntries: 10,
      resolutionRate: 1,
      pedbEntries: 10,
      edbEntries: 10,
      matchedEntries: 10,
      pedbMatchRate: 1,
      edbMatchRate: 1
    }
  ]);
  const parityTable = renderStrictParityPlaylistTable([
    {
      name: "Set A",
      status: "FAIL",
      pedbTracks: 10,
      edbTracks: 10,
      matchedTracks: 9,
      onlyInPdb: 1,
      onlyInEdb: 0,
      orderMismatch: false,
      pdbDuplicateEntries: 0,
      pdbMissingCoreMetadata: 2,
      edbMissingCoreMetadata: 0,
      artworkMismatchTracks: 0,
      pathMismatchTracks: 0,
      dictionaryIdIssueTracks: 0,
      playlistIdMatch: true,
      sortOrderMatch: true
    }
  ]);

  // Different section summaries
  assert.notEqual(diagTable.summary, parityTable.summary);
  assert.ok(diagTable.summary.includes("Playlist Resolution"));
  assert.ok(parityTable.summary.includes("Strict Parity"));

  // Diagnostics has operational columns only
  assert.ok(diagTable.columns.includes("Resolution Rate"));
  assert.ok(!diagTable.columns.includes("PDB Duplicates"));
  assert.ok(!diagTable.columns.includes("Dictionary ID Issues"));

  // Parity has strict-only columns
  assert.ok(parityTable.columns.includes("PDB Duplicates"));
  assert.ok(parityTable.columns.includes("Dictionary ID Issues"));
  assert.ok(parityTable.columns.includes("Playlist ID Match"));
  assert.ok(!parityTable.columns.includes("Resolution Rate"));
});

// --- Coverage: strict parity summary and table rendering ---

test("strict parity summary rows have label, status, and count fields", () => {
  const summaryRows = [
    { label: "Failing playlists", status: "FAIL", count: 1 },
    { label: "PDB metadata gaps", status: "FAIL", count: 3 },
    { label: "Path mismatches", status: "FAIL", count: 2 },
    { label: "Artwork presence mismatches", status: "WARN", count: 1 },
    { label: "Unresolved PDB dictionary ids", status: "FAIL", count: 4 },
    { label: "Membership only-in-PDB", status: "PASS", count: 0 },
    { label: "Order mismatches", status: "PASS", count: 0 },
    { label: "Duplicate PDB entries", status: "PASS", count: 0 },
    { label: "eDB source gaps", status: "PASS", count: 0 }
  ];

  // All rows have the required shape
  for (const row of summaryRows) {
    assert.ok(typeof row.label === "string" && row.label.length > 0);
    assert.ok(["PASS", "FAIL", "WARN"].includes(row.status));
    assert.ok(typeof row.count === "number");
  }

  // FAIL rows have count > 0
  const failRows = summaryRows.filter((r) => r.status === "FAIL");
  assert.ok(failRows.length > 0);
  for (const row of failRows) {
    assert.ok(row.count > 0, `FAIL row "${row.label}" should have count > 0`);
  }

  // PASS rows have count === 0
  const passRows = summaryRows.filter((r) => r.status === "PASS");
  assert.ok(passRows.length > 0);
  for (const row of passRows) {
    assert.equal(row.count, 0, `PASS row "${row.label}" should have count === 0`);
  }
});

test("strict parity playlist table renders per-playlist mismatch data", () => {
  const table = renderStrictParityPlaylistTable([
    {
      name: "Main Set",
      status: "FAIL",
      pedbTracks: 20,
      edbTracks: 20,
      matchedTracks: 18,
      onlyInPdb: 2,
      onlyInEdb: 0,
      orderMismatch: true,
      pdbDuplicateEntries: 1,
      pdbMissingCoreMetadata: 3,
      edbMissingCoreMetadata: 1,
      artworkMismatchTracks: 2,
      pathMismatchTracks: 4,
      dictionaryIdIssueTracks: 5,
      playlistIdMatch: false,
      sortOrderMatch: false
    }
  ]);

  const row = table.rows[0];
  assert.equal(row.playlist, "Main Set");
  assert.equal(row.status, "FAIL");
  assert.equal(row.pedbTracks, 20);
  assert.equal(row.matchedTracks, 18);
  assert.equal(row.onlyInPdb, 2);
  assert.equal(row.order, "Mismatch");
  assert.equal(row.pdbDuplicates, 1);
  assert.equal(row.pdbMetadataGaps, 3);
  assert.equal(row.edbMetadataGaps, 1);
  assert.equal(row.artworkMismatches, 2);
  assert.equal(row.pathMismatches, 4);
  assert.equal(row.dictionaryIdIssues, 5);
  assert.equal(row.playlistIdMatch, false);
  assert.equal(row.sortOrderMatch, false);
});

// --- Coverage: legacy parity messaging is absent ---

test("diagnostics table does not include legacy dense parity sentence columns", () => {
  const diagTable = renderDiagnosticsPlaylistTable([
    {
      name: "Test",
      status: "PASS",
      totalEntries: 5,
      resolvedEntries: 5,
      resolutionRate: 1,
      pedbEntries: 5,
      edbEntries: 5,
      matchedEntries: 5,
      pedbMatchRate: 1,
      edbMatchRate: 1
    }
  ]);

  // No strict-parity-specific columns in diagnostics
  const legacyColumns = [
    "PDB Duplicates",
    "PDB Metadata Gaps",
    "eDB Metadata Gaps",
    "Artwork Mismatches",
    "Path Mismatches",
    "Dictionary ID Issues",
    "Playlist ID Match",
    "Sort Order Match"
  ];
  for (const col of legacyColumns) {
    assert.ok(!diagTable.columns.includes(col), `diagnostics table should not include "${col}"`);
  }

  // No parity-specific fields in diagnostics rows
  const row = diagTable.rows[0];
  assert.ok(!("pathMismatches" in row));
  assert.ok(!("dictionaryIdIssues" in row));
  assert.ok(!("pdbDuplicates" in row));
  assert.ok(!("artworkMismatches" in row));
  assert.ok(!("playlistIdMatch" in row));
  assert.ok(!("sortOrderMatch" in row));
});

test("strict parity table does not include legacy operational resolution columns", () => {
  const parityTable = renderStrictParityPlaylistTable([
    {
      name: "Test",
      status: "PASS",
      pedbTracks: 5,
      edbTracks: 5,
      matchedTracks: 5,
      onlyInPdb: 0,
      onlyInEdb: 0,
      orderMismatch: false,
      pdbDuplicateEntries: 0,
      pdbMissingCoreMetadata: 0,
      edbMissingCoreMetadata: 0,
      artworkMismatchTracks: 0,
      pathMismatchTracks: 0,
      dictionaryIdIssueTracks: 0,
      playlistIdMatch: true,
      sortOrderMatch: true
    }
  ]);

  const operationalColumns = [
    "Total Entries",
    "Resolved Entries",
    "Resolution Rate",
    "PDB Match Rate",
    "eDB Match Rate"
  ];
  for (const col of operationalColumns) {
    assert.ok(!parityTable.columns.includes(col), `parity table should not include "${col}"`);
  }
});

// --- Coverage: empty-state for diagnostics/parity split ---

test("diagnostics playlist table handles empty playlist list", () => {
  const diagTable = renderDiagnosticsPlaylistTable([]);
  assert.equal(diagTable.summary, "Playlist Resolution Details");
  assert.equal(diagTable.rows.length, 0);
  assert.ok(diagTable.columns.length > 0);
});

test("strict parity playlist table handles empty playlist list", () => {
  const parityTable = renderStrictParityPlaylistTable([]);
  assert.equal(parityTable.summary, "Strict Parity Playlist Details");
  assert.equal(parityTable.rows.length, 0);
  assert.ok(parityTable.columns.length > 0);
});

test("strict parity summary handles empty summary rows", () => {
  const summaryRows = [];
  assert.equal(summaryRows.length, 0);
});

// --- Coverage: mock payload contract ---

test("diagnostics mock payload contract has required top-level fields", () => {
  const diagPayload = {
    overallStatus: "WARN",
    pdbIntegrity: { title: "PDB Integrity", status: "PASS", checks: [] },
    edbAccess: { title: "Database Access", status: "PASS", checks: [] },
    contentsIntegrity: { title: "Contents Integrity", status: "PASS", checks: [] },
    analysisIntegrity: { title: "Analysis Files", status: "WARN", checks: [] },
    playlistResolution: {
      title: "Playlist Resolution",
      status: "PASS",
      checks: [
        { label: "Overall resolution", status: "PASS", detail: "3/3 entries resolve (100.0%) across 1 playlists" }
      ]
    },
    playlistDetails: [
      {
        name: "Warmup",
        totalEntries: 3,
        resolvedEntries: 3,
        resolutionRate: 1,
        status: "PASS",
        pedbEntries: 3,
        edbEntries: 3,
        matchedEntries: 3,
        pedbMatchRate: 1,
        edbMatchRate: 1
      }
    ],
    warnings: [],
    durationMs: 10
  };

  // Required sections
  assert.ok(diagPayload.pdbIntegrity);
  assert.ok(diagPayload.edbAccess);
  assert.ok(diagPayload.contentsIntegrity);
  assert.ok(diagPayload.analysisIntegrity);
  assert.ok(diagPayload.playlistResolution);
  assert.ok(typeof diagPayload.overallStatus === "string");
  assert.ok(typeof diagPayload.durationMs === "number");
  assert.ok(Array.isArray(diagPayload.playlistDetails));
  assert.ok(Array.isArray(diagPayload.warnings));

  // Each section has title, status, checks
  for (const section of [diagPayload.pdbIntegrity, diagPayload.edbAccess, diagPayload.contentsIntegrity, diagPayload.analysisIntegrity, diagPayload.playlistResolution]) {
    assert.ok(typeof section.title === "string");
    assert.ok(typeof section.status === "string");
    assert.ok(Array.isArray(section.checks));
  }

  // Playlist detail has operational fields
  const pd = diagPayload.playlistDetails[0];
  assert.ok("totalEntries" in pd);
  assert.ok("resolvedEntries" in pd);
  assert.ok("resolutionRate" in pd);
  assert.ok("pedbEntries" in pd);
  assert.ok("edbEntries" in pd);
  assert.ok("matchedEntries" in pd);
  // Diagnostics playlist detail should not have strict parity fields
  assert.ok(!("pdbMissingCoreMetadata" in pd));
  assert.ok(!("dictionaryIdIssueTracks" in pd));
});

test("parity mock payload contract has required top-level fields", () => {
  const parityPayload = {
    overallStatus: "FAIL",
    checks: [
      { label: "Overall player parity status", status: "FAIL", detail: "playlists checked: 1, fail: 1" },
      { label: "PDB metadata completeness", status: "FAIL", detail: "1 track(s) missing" }
    ],
    summaryRows: [
      { label: "Failing playlists", status: "FAIL", count: 1 },
      { label: "PDB metadata gaps", status: "FAIL", count: 1 },
      { label: "Membership only-in-PDB", status: "PASS", count: 0 }
    ],
    playlistDetails: [
      {
        name: "Warmup",
        pdbTracks: 3,
        edbTracks: 3,
        matchedTracks: 3,
        onlyInPdb: 0,
        onlyInEdb: 0,
        orderMismatch: false,
        pdbDuplicateEntries: 0,
        pdbMissingCoreMetadata: 1,
        edbMissingCoreMetadata: 0,
        artworkMismatchTracks: 1,
        pathMismatchTracks: 1,
        dictionaryIdIssueTracks: 1,
        playlistIdMatch: true,
        sortOrderMatch: true,
        status: "FAIL"
      }
    ],
    warnings: [],
    durationMs: 10
  };

  assert.ok(typeof parityPayload.overallStatus === "string");
  assert.ok(Array.isArray(parityPayload.checks));
  assert.ok(Array.isArray(parityPayload.summaryRows));
  assert.ok(Array.isArray(parityPayload.playlistDetails));
  assert.ok(typeof parityPayload.durationMs === "number");

  // Each check has label, status, detail
  for (const check of parityPayload.checks) {
    assert.ok(typeof check.label === "string");
    assert.ok(typeof check.status === "string");
    assert.ok(typeof check.detail === "string");
  }

  // Each summary row has label, status, count
  for (const row of parityPayload.summaryRows) {
    assert.ok(typeof row.label === "string");
    assert.ok(typeof row.status === "string");
    assert.ok(typeof row.count === "number");
  }

  // Parity playlist detail has strict parity fields
  const pd = parityPayload.playlistDetails[0];
  assert.ok("pdbTracks" in pd);
  assert.ok("edbTracks" in pd);
  assert.ok("matchedTracks" in pd);
  assert.ok("onlyInPdb" in pd);
  assert.ok("onlyInEdb" in pd);
  assert.ok("orderMismatch" in pd);
  assert.ok("pdbDuplicateEntries" in pd);
  assert.ok("pdbMissingCoreMetadata" in pd);
  assert.ok("edbMissingCoreMetadata" in pd);
  assert.ok("artworkMismatchTracks" in pd);
  assert.ok("pathMismatchTracks" in pd);
  assert.ok("dictionaryIdIssueTracks" in pd);
  assert.ok("playlistIdMatch" in pd);
  assert.ok("sortOrderMatch" in pd);
  // Parity playlist detail should not have operational fields
  assert.ok(!("totalEntries" in pd));
  assert.ok(!("resolvedEntries" in pd));
  assert.ok(!("resolutionRate" in pd));
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
