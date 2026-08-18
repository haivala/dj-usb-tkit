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

// The recent-USB pills (usb/events.mjs's usbRecentList click handler) and
// the folder picker (pickUsbFolder) both funnel through this same
// validateAndSetUsbRoot function with an identical signature -- there's no
// "came from a recent pill" flag for it to branch on. So a single test
// exercising validateAndSetUsbRoot's naming behavior covers both entry
// points; there's nothing recent-pill-specific left to wire up.
function makeFakeInteractiveElement(overrides = {}) {
  const listeners = new Map();
  return {
    value: "",
    hidden: true,
    focus() {},
    select() {},
    addEventListener(type, fn) {
      if (!listeners.has(type)) listeners.set(type, []);
      listeners.get(type).push(fn);
    },
    removeEventListener(type, fn) {
      const fns = listeners.get(type);
      if (!fns) return;
      const idx = fns.indexOf(fn);
      if (idx !== -1) fns.splice(idx, 1);
    },
    emit(type, event = {}) {
      for (const fn of listeners.get(type) || []) fn(event);
    },
    ...overrides
  };
}

async function waitFor(predicate, { timeoutMs = 1000 } = {}) {
  const start = Date.now();
  while (!predicate()) {
    if (Date.now() - start > timeoutMs) {
      throw new Error("waitFor: condition never became true");
    }
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

function makeValidateUsbRootDeps(command) {
  return {
    command,
    documentObj: makeFakeInteractiveElement(),
    persistUsbRoot: () => {},
    updateUsbRootText: () => {},
    resetUsbStateViews: () => {},
    updateUsbConfigControlsVisibility: () => {},
    updateUsbSubNavDisabledState: () => {},
    updatePlaylistExportButtons: () => {},
    setStatus: () => {},
    runUsbDiagnostics: async () => {},
    warn: () => {},
    scheduler: (fn) => { fn(); }
  };
}

test("validateAndSetUsbRoot updates the USB name badge as the drive connects, gets named, and disconnects", async () => {
  const state = { usbRoot: null, usbRootValid: false, usbNeedsInit: false, usbWritable: false };
  const el = {
    driveNameOverlay: makeFakeInteractiveElement({ hidden: true }),
    driveNameInput: makeFakeInteractiveElement(),
    driveNameOkBtn: makeFakeInteractiveElement(),
    driveNameSkipBtn: makeFakeInteractiveElement(),
    usbInitRow: { classList: makeClassList() }
  };

  const setNameCalls = [];
  const command = async (name, payload) => {
    if (name === "validate_usb_root") {
      return {
        valid: true,
        hasWriteAccess: true,
        normalizedRoot: "/media/user/CLUBSTICK",
        hasVendorRoot: true,
        hasContents: true,
        hasPdb: true,
        warnings: []
      };
    }
    if (name === "get_usb_device_name") {
      return { name: null, suggestedName: null };
    }
    if (name === "set_usb_device_name") {
      setNameCalls.push(payload);
      return { saved: true };
    }
    throw new Error(`unexpected command: ${name}`);
  };

  let badgeUpdateCount = 0;
  const deps = makeValidateUsbRootDeps(command);
  deps.updateUsbNameBadge = () => { badgeUpdateCount += 1; };

  const resultPromise = validateAndSetUsbRoot(state, el, "/media/user/CLUBSTICK", false, deps);
  await waitFor(() => el.driveNameOverlay.hidden === false);
  assert.equal(state.usbDeviceName, null, "must not show a name while the prompt is still open");
  el.driveNameInput.value = "Club Stick";
  el.driveNameOkBtn.emit("click");
  await resultPromise;

  assert.equal(state.usbDeviceName, "Club Stick");
  assert.ok(badgeUpdateCount > 0, "the badge must be refreshed after the name is saved");

  await validateAndSetUsbRoot(state, el, "", false, deps);
  assert.equal(state.usbDeviceName, null, "disconnecting the USB must clear the name badge");
});

test("validateAndSetUsbRoot surfaces a visible status message (not just a console warning) when checking the drive name fails, instead of silently skipping the prompt", async () => {
  const state = { usbRoot: null, usbRootValid: false, usbNeedsInit: false, usbWritable: false };
  const el = {
    driveNameOverlay: makeFakeInteractiveElement({ hidden: true }),
    driveNameInput: makeFakeInteractiveElement(),
    driveNameOkBtn: makeFakeInteractiveElement(),
    driveNameError: { hidden: true, textContent: "" }
  };

  const statuses = [];
  const command = async (name) => {
    if (name === "validate_usb_root") {
      return {
        valid: true,
        hasWriteAccess: true,
        normalizedRoot: "/media/user/CLUBSTICK",
        hasVendorRoot: true,
        hasContents: true,
        hasPdb: true,
        warnings: []
      };
    }
    if (name === "get_usb_device_name") {
      throw new Error("usb root not found");
    }
    throw new Error(`unexpected command: ${name}`);
  };

  const deps = makeValidateUsbRootDeps(command);
  deps.setStatus = (text) => statuses.push(text);

  const valid = await validateAndSetUsbRoot(state, el, "/media/user/CLUBSTICK", false, deps);

  assert.equal(valid, true, "validation itself still succeeds even if the naming check fails");
  assert.equal(el.driveNameOverlay.hidden, true, "prompt cannot open without knowing the current name");
  assert.ok(
    statuses.some((s) => s.includes("Could not check this drive's name")),
    `expected a visible status message explaining the naming check failed, got: ${JSON.stringify(statuses)}`
  );
});

test("validateAndSetUsbRoot does not open the naming prompt when get_usb_device_name returns a shape without a 'name' field (fails closed on an unexpected/mocked response instead of guessing 'unnamed')", async () => {
  // Regression test: a permissive test double or a future API change could
  // resolve with e.g. {} instead of a real GetUsbDeviceNameData. Reading a
  // missing field as falsy ("not named") used to pop the naming prompt --
  // which blocks every click in the app until resolved -- based on a guess
  // rather than a real answer.
  const state = { usbRoot: null, usbRootValid: false, usbNeedsInit: false, usbWritable: false };
  const el = {
    driveNameOverlay: makeFakeInteractiveElement({ hidden: true }),
    driveNameInput: makeFakeInteractiveElement(),
    driveNameOkBtn: makeFakeInteractiveElement(),
    driveNameSkipBtn: makeFakeInteractiveElement(),
    driveNameError: { hidden: true, textContent: "" }
  };

  const command = async (name) => {
    if (name === "validate_usb_root") {
      return {
        valid: true,
        hasWriteAccess: true,
        normalizedRoot: "/media/user/CLUBSTICK",
        hasVendorRoot: true,
        hasContents: true,
        hasPdb: true,
        warnings: []
      };
    }
    if (name === "get_usb_device_name") {
      return {}; // no "name" field at all -- not a real answer.
    }
    throw new Error(`unexpected command: ${name}`);
  };

  const valid = await validateAndSetUsbRoot(
    state,
    el,
    "/media/user/CLUBSTICK",
    false,
    makeValidateUsbRootDeps(command)
  );

  assert.equal(valid, true);
  assert.equal(el.driveNameOverlay.hidden, true, "an ambiguous response must never open the prompt");
});

test("the drive-naming prompt can always be dismissed without saving (Escape, backdrop click, and a 'Not now' button), never permanently blocking the UI", async () => {
  const state = { usbRoot: null, usbRootValid: false, usbNeedsInit: false, usbWritable: false };

  const command = async (name) => {
    if (name === "validate_usb_root") {
      return {
        valid: true,
        hasWriteAccess: true,
        normalizedRoot: "/media/user/CLUBSTICK",
        hasVendorRoot: true,
        hasContents: true,
        hasPdb: true,
        warnings: []
      };
    }
    if (name === "get_usb_device_name") {
      return { name: null, suggestedName: null };
    }
    if (name === "set_usb_device_name") {
      throw new Error("set_usb_device_name must not be called when the prompt is dismissed without saving");
    }
    throw new Error(`unexpected command: ${name}`);
  };

  // 1. Escape key (dispatched on the document, matching a real keydown that
  // can land while focus is anywhere in the dialog, not just the input).
  {
    const el = {
      driveNameOverlay: makeFakeInteractiveElement({ hidden: true }),
      driveNameInput: makeFakeInteractiveElement(),
      driveNameOkBtn: makeFakeInteractiveElement(),
      driveNameSkipBtn: makeFakeInteractiveElement()
    };
    const deps = makeValidateUsbRootDeps(command);
    const resultPromise = validateAndSetUsbRoot(state, el, "/media/user/CLUBSTICK", false, deps);
    await waitFor(() => el.driveNameOverlay.hidden === false);
    deps.documentObj.emit("keydown", { key: "Escape" });
    await resultPromise;
    assert.equal(el.driveNameOverlay.hidden, true);
  }

  // 2. Clicking the backdrop itself (not the dialog box inside it).
  {
    const el = {
      driveNameOverlay: makeFakeInteractiveElement({ hidden: true }),
      driveNameInput: makeFakeInteractiveElement(),
      driveNameOkBtn: makeFakeInteractiveElement(),
      driveNameSkipBtn: makeFakeInteractiveElement()
    };
    const resultPromise = validateAndSetUsbRoot(
      state,
      el,
      "/media/user/CLUBSTICK",
      false,
      makeValidateUsbRootDeps(command)
    );
    await waitFor(() => el.driveNameOverlay.hidden === false);
    el.driveNameOverlay.emit("click", { target: el.driveNameOverlay });
    await resultPromise;
    assert.equal(el.driveNameOverlay.hidden, true);
  }

  // 3. The explicit "Not now" button.
  {
    const el = {
      driveNameOverlay: makeFakeInteractiveElement({ hidden: true }),
      driveNameInput: makeFakeInteractiveElement(),
      driveNameOkBtn: makeFakeInteractiveElement(),
      driveNameSkipBtn: makeFakeInteractiveElement()
    };
    const resultPromise = validateAndSetUsbRoot(
      state,
      el,
      "/media/user/CLUBSTICK",
      false,
      makeValidateUsbRootDeps(command)
    );
    await waitFor(() => el.driveNameOverlay.hidden === false);
    el.driveNameSkipBtn.emit("click");
    await resultPromise;
    assert.equal(el.driveNameOverlay.hidden, true);
  }
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
