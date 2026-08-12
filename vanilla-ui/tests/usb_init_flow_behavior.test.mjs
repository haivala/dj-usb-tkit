import test from "node:test";
import assert from "node:assert/strict";
import { hydrateUsbTrackMetadata, hydrateUsbTrackMetadataBatch, validateAndSetUsbRoot } from "../components/usb/actions.mjs";
import { makeClassList } from "./fixtures/dom.mjs";

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

test("validateAndSetUsbRoot renders structured WarningEntry warnings as their message text, not [object Object]", async () => {
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
  const valid = await validateAndSetUsbRoot(state, el, "/tmp/usb", false, {
    command: async () => ({
      valid: false,
      hasWriteAccess: true,
      normalizedRoot: "/tmp/usb",
      hasVendorRoot: false,
      hasContents: false,
      hasPdb: false,
      warnings: [
        { level: "warn", code: "usb.validate.missing-vendor-root", message: "Missing vendor root folder", source: "usb-validate" },
        { level: "warn", code: "usb.validate.missing-contents", message: "Missing Contents directory", source: "usb-validate" }
      ]
    }),
    persistUsbRoot: () => {},
    updateUsbRootText: () => {},
    resetUsbStateViews: () => {},
    updateUsbConfigControlsVisibility: () => {},
    updateUsbSubNavDisabledState: () => {},
    updatePlaylistExportButtons: () => {},
    setStatus: (text) => { lastStatus = text; },
    runUsbDiagnostics: async () => {},
    warn: () => {},
    scheduler: () => {}
  });

  assert.equal(valid, false);
  assert.doesNotMatch(el.usbInitHint.textContent, /\[object Object\]/);
  assert.match(el.usbInitHint.textContent, /Missing vendor root folder \| Missing Contents directory/);
  assert.doesNotMatch(lastStatus, /\[object Object\]/);
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

test("hydrateUsbTrackMetadataBatch sends one inspect_usb_tracks call for multiple tracks and applies results by id", async () => {
  const state = { usbRoot: "/tmp/usb" };
  const trackA = { id: "1", filePath: "/a.mp3", title: "A", artist: "Artist A" };
  const trackB = { id: "2", filePath: "/b.mp3", title: "B", artist: "Artist B" };
  let commandCalls = 0;

  await hydrateUsbTrackMetadataBatch(state, [trackA, trackB], {
    usbTrackNeedsHydration: () => true,
    command: async (name, payload) => {
      commandCalls += 1;
      assert.equal(name, "inspect_usb_tracks");
      assert.equal(payload.usbRoot, "/tmp/usb");
      assert.deepEqual(
        payload.items.map((item) => item.trackId),
        ["1", "2"]
      );
      return {
        items: [
          { trackId: "1", source: "pdb", track: { id: "1", title: "A", artist: "Artist A", bpm: 120 } },
          { trackId: "2", source: "eDB", track: { id: "2", title: "B", artist: "Artist B", bpm: 128 } }
        ]
      };
    },
    normalizeTrack: (candidate) => ({ ...candidate })
  });

  assert.equal(commandCalls, 1, "should batch both tracks into a single command call");
  assert.equal(trackA.bpm, 120);
  assert.equal(trackA.artworkChecked, true);
  assert.equal(trackB.bpm, 128);
  assert.equal(trackB.artworkChecked, true);
});

test("hydrateUsbTrackMetadataBatch skips tracks that don't need hydration and never calls command for an empty candidate set", async () => {
  const state = { usbRoot: "/tmp/usb" };
  const track = { id: "1", title: "A", artist: "Artist A" };
  let commandCalls = 0;

  const tracks = await hydrateUsbTrackMetadataBatch(state, [track], {
    usbTrackNeedsHydration: () => false,
    command: async () => {
      commandCalls += 1;
      return { items: [] };
    },
    normalizeTrack: (candidate) => ({ ...candidate })
  });

  assert.equal(commandCalls, 0, "no candidates need hydration, so no IPC call should be made");
  assert.equal(tracks[0], track);
  assert.equal(track.artworkChecked, undefined);
});

test("hydrateUsbTrackMetadataBatch marks tracks missing from the response as checked without throwing", async () => {
  const state = { usbRoot: "/tmp/usb" };
  const trackA = { id: "1", title: "A", artist: "Artist A" };
  const trackB = { id: "999999", title: "Unresolved Title", artist: "Unresolved Artist" };

  await hydrateUsbTrackMetadataBatch(state, [trackA, trackB], {
    usbTrackNeedsHydration: () => true,
    command: async () => ({
      items: [
        { trackId: "1", source: "pdb", track: { id: "1", title: "A", artist: "Artist A", bpm: 120 } },
        { trackId: "999999", source: null, track: null }
      ]
    }),
    normalizeTrack: (candidate) => ({
      ...candidate,
      title: candidate.title || "Unknown Title",
      artist: candidate.artist || "Unknown Artist"
    })
  });

  assert.equal(trackA.bpm, 120);
  assert.equal(trackA.artworkChecked, true);
  assert.equal(trackB.artworkChecked, true, "unresolved tracks should still be marked checked so they aren't retried forever");
  assert.equal(trackB.title, "Unresolved Title");
  assert.equal(trackB.artist, "Unresolved Artist");
});

test("hydrateUsbTrackMetadataBatch preserves existing labels when inspection returns partial metadata", async () => {
  const state = { usbRoot: "/tmp/usb" };
  const track = { id: "1", title: "Existing Title", artist: "Existing Artist", durationMs: 180000 };

  await hydrateUsbTrackMetadataBatch(state, [track], {
    usbTrackNeedsHydration: () => true,
    command: async () => ({
      items: [
        { trackId: "1", source: "pdb", track: { id: "1", bpm: 120 } }
      ]
    }),
    normalizeTrack: (candidate) => ({
      ...candidate,
      title: candidate.title || "Unknown Title",
      artist: candidate.artist || "Unknown Artist",
      durationMs: candidate.durationMs ?? null
    })
  });

  assert.equal(track.title, "Existing Title");
  assert.equal(track.artist, "Existing Artist");
  assert.equal(track.durationMs, 180000);
  assert.equal(track.bpm, 120);
  assert.equal(track.artworkChecked, true);
});
