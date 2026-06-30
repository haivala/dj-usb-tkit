import test from "node:test";
import assert from "node:assert/strict";
import { hydrateUsbTrackMetadata, validateAndSetUsbRoot } from "../components/usb/actions.mjs";

function makeClassList() {
  const classes = new Set();
  return {
    add(name) { classes.add(name); },
    remove(name) { classes.delete(name); },
    toggle(name, force) {
      if (typeof force === "boolean") {
        if (force) classes.add(name);
        else classes.delete(name);
        return force;
      }
      if (classes.has(name)) {
        classes.delete(name);
        return false;
      }
      classes.add(name);
      return true;
    },
    contains(name) { return classes.has(name); }
  };
}

test("validateAndSetUsbRoot exposes one-click init state for writable missing-structure USB", async () => {
  const initRow = { classList: makeClassList() };
  const state = {
    usbRoot: null,
    usbRootValid: false,
    usbNeedsInit: false,
    usbWritable: false
  };
  const el = {
    usbInitRow: initRow,
    usbInitHint: { textContent: "" },
    initializeUsbBtn: { disabled: true }
  };

  let lastStatus = "";
  let diagCalled = false;
  const valid = await validateAndSetUsbRoot(state, el, "/tmp/usb", false, {
    command: async (name) => {
      assert.equal(name, "validate_usb_root");
      return {
        valid: false,
        hasWriteAccess: true,
        normalizedRoot: "/tmp/usb",
        hasVendorRoot: false,
        hasContents: false,
        hasPdb: false,
        warnings: ["Missing vendor root folder", "Missing Contents directory"]
      };
    },
    persistUsbRoot: () => {},
    updateUsbRootText: () => {},
    resetUsbStateViews: () => {},
    updateUsbConfigControlsVisibility: () => {},
    updateUsbSubNavDisabledState: () => {},
    updatePlaylistExportButtons: () => {},
    setStatus: (text) => { lastStatus = text; },
    runUsbDiagnostics: async () => { diagCalled = true; },
    warn: () => {},
    scheduler: () => {}
  });

  assert.equal(valid, false);
  assert.equal(state.usbRootValid, false);
  assert.equal(state.usbNeedsInit, true);
  assert.equal(state.usbWritable, true);
  assert.equal(state.usbRoot, "/tmp/usb");
  assert.equal(el.initializeUsbBtn.disabled, false);
  assert.equal(el.usbInitRow.classList.contains("hidden"), false);
  assert.match(el.usbInitHint.textContent, /missing External library structure/i);
  assert.match(lastStatus, /Click "Initialize USB Structure"/);
  assert.equal(diagCalled, false);
});

test("validateAndSetUsbRoot valid USB triggers diagnostics path and hides init controls", async () => {
  const initRow = { classList: makeClassList() };
  initRow.classList.add("hidden");
  const scheduled = [];
  const state = {
    usbRoot: null,
    usbRootValid: false,
    usbNeedsInit: false,
    usbWritable: false
  };
  const el = {
    usbInitRow: initRow,
    usbInitHint: { textContent: "" },
    initializeUsbBtn: { disabled: true }
  };

  let lastStatus = "";
  let diagCalled = false;
  const valid = await validateAndSetUsbRoot(state, el, "/tmp/usb", false, {
    command: async () => ({
      valid: true,
      hasWriteAccess: true,
      normalizedRoot: "/tmp/usb",
      hasVendorRoot: true,
      hasContents: true,
      hasPdb: true,
      warnings: []
    }),
    persistUsbRoot: () => {},
    updateUsbRootText: () => {},
    resetUsbStateViews: () => {},
    updateUsbConfigControlsVisibility: () => {},
    updateUsbSubNavDisabledState: () => {},
    updatePlaylistExportButtons: () => {},
    setStatus: (text) => { lastStatus = text; },
    runUsbDiagnostics: async () => { diagCalled = true; },
    warn: () => {},
    scheduler: (fn) => { scheduled.push(fn); }
  });

  assert.equal(valid, true);
  assert.equal(state.usbRootValid, true);
  assert.equal(state.usbNeedsInit, false);
  assert.equal(el.initializeUsbBtn.disabled, true);
  assert.equal(el.usbInitRow.classList.contains("hidden"), true);
  assert.match(lastStatus, /Running diagnostics/i);
  assert.equal(scheduled.length, 1, "valid USB should schedule diagnostics");

  await scheduled[0]();
  assert.equal(diagCalled, true);
});

test("validateAndSetUsbRoot hides stale diagnostics before reading a different USB", async () => {
  const initRow = { classList: makeClassList() };
  const diagnosticsCard = { classList: makeClassList(), closest: () => null };
  const diagPlaylistDetails = { classList: makeClassList() };
  const previewRepairsBtn = { disabled: false };
  const applyRepairsBtn = { disabled: false };
  const diagReportView = { classList: makeClassList() };
  const diagRepairPanel = { classList: makeClassList() };
  diagnosticsCard.classList.remove("hidden");
  diagRepairPanel.classList.remove("hidden");
  const state = {
    usbRoot: "/tmp/old-usb",
    usbRootValid: true,
    usbNeedsInit: false,
    usbWritable: true
  };
  const el = {
    usbInitRow: initRow,
    usbInitHint: { textContent: "" },
    initializeUsbBtn: { disabled: false },
    usbDiagnosticsCard: diagnosticsCard,
    diagSections: { innerHTML: "stale diagnostics" },
    diagOverallStatus: { textContent: "WARN", className: "diag-badge diag-warn" },
    diagDuration: { textContent: "Completed in 10ms" },
    diagPlaylistDetails,
    diagPlaylistTableBody: { innerHTML: "<tr><td>stale</td></tr>" },
    diagRepairSummary: { textContent: "stale repair", className: "diag-repair-summary" },
    diagRepairFixes: { innerHTML: "<li>stale</li>" },
    previewRepairsBtn,
    applyRepairsBtn,
    diagReportView,
    diagRepairPanel
  };

  let commandSawDiagnosticsHidden = false;
  await validateAndSetUsbRoot(state, el, "/tmp/new-usb", false, {
    command: async () => {
      commandSawDiagnosticsHidden = diagnosticsCard.classList.contains("hidden")
        && el.diagSections.innerHTML === ""
        && previewRepairsBtn.disabled
        && applyRepairsBtn.disabled
        && !diagReportView.classList.contains("hidden")
        && diagRepairPanel.classList.contains("hidden");
      return {
        valid: true,
        hasWriteAccess: true,
        normalizedRoot: "/tmp/new-usb",
        hasVendorRoot: true,
        hasContents: true,
        hasPdb: true,
        warnings: []
      };
    },
    persistUsbRoot: () => {},
    updateUsbRootText: () => {},
    resetUsbStateViews: () => {},
    updateUsbConfigControlsVisibility: () => {},
    updateUsbSubNavDisabledState: () => {},
    updatePlaylistExportButtons: () => {},
    setStatus: () => {},
    runUsbDiagnostics: async () => {},
    warn: () => {},
    scheduler: () => {}
  });

  assert.equal(commandSawDiagnosticsHidden, true);
});

test("hydrateUsbTrackMetadata marks inspected no-artwork tracks as checked", async () => {
  const state = { usbRoot: "/tmp/usb" };
  const track = {
    id: "123",
    filePath: "/tmp/usb/Contents/track.mp3",
    title: "Track",
    artist: "Artist",
    waveformPreview: [10],
    bpm: 120,
    key: "8A",
    artworkPath: "",
    artworkUrl: ""
  };
  let inspectCalls = 0;

  const result = await hydrateUsbTrackMetadata(state, track, {
    usbTrackNeedsHydration: (candidate) => {
      assert.equal(candidate, track);
      return true;
    },
    command: async (name, payload) => {
      inspectCalls += 1;
      assert.equal(name, "inspect_usb_track");
      assert.equal(payload.trackId, "123");
      assert.equal(payload.usbRoot, "/tmp/usb");
      return {
        track: {
          id: "123",
          title: "Track",
          artist: "Artist",
          waveformPreview: [10],
          bpm: 120,
          key: "8A",
          artworkPath: "",
          artworkUrl: ""
        }
      };
    },
    normalizeTrack: (candidate) => ({ ...candidate })
  });

  assert.equal(result, track);
  assert.equal(inspectCalls, 1);
  assert.equal(track.artworkChecked, true);
  assert.equal(track.artworkPath, "");
  assert.equal(track.artworkUrl, "");
});
