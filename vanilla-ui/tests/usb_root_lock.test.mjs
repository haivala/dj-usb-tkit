import test from "node:test";
import assert from "node:assert/strict";

if (typeof globalThis.window === "undefined") {
  globalThis.window = {
    setInterval: globalThis.setInterval,
    clearInterval: globalThis.clearInterval,
    setTimeout: globalThis.setTimeout
  };
}

import {
  USB_ROOT_LOCKING_JOB_TYPES,
  pickUsbFolder,
  renderUsbRecentRoots,
  setUsbRootControlsLocked,
  validateAndSetUsbRoot
} from "../components/usb/actions.mjs";
import { updatePlaylistExportButtons } from "../components/playlist/actions.mjs";
import { handleJobEvent } from "../job_manager.mjs";
import { makeClassList } from "./fixtures/dom.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

// handleJobEvent's collaborators as no-ops, recording USB lock toggles.
function jobCtx(lockCalls) {
  const el = {
    progressFooter: { classList: makeClassList(), querySelector: () => null },
    progressFill: { style: {} },
    progressText: {},
    progressTime: {},
    progressPauseBtn: { setAttribute() {} },
    progressCancelAnalysisBtn: {}
  };
  return {
    state: { activeJobId: null, activeJobType: null, runningJobs: new Map(), usbJobIdleWaiters: [] },
    el,
    debugFrontendLog: () => {},
    emitMessage: () => {},
    applyRealtimeAnalyzedTrackUpdate: () => Promise.resolve(),
    refreshLibrarySources: () => {},
    applyLibraryDurationSummary: () => {},
    setTrackAnalyzingState: () => {},
    setUsbRootControlsLocked: (locked) => lockCalls.push(locked)
  };
}

function withJob(ctx, activeJobId, activeJobType) {
  const runningJobs = activeJobId ? new Map([[activeJobId, { jobType: activeJobType }]]) : new Map();
  Object.assign(ctx.state, { activeJobId, activeJobType, runningJobs });
  return ctx;
}

test("pickUsbFolder blocks every USB-locking job type", async (t) => {
  for (const jobType of USB_ROOT_LOCKING_JOB_TYPES) {
    await t.test(`blocked for jobType=${jobType}`, async () => {
      let invokeCalled = false;
      let lastStatus = "";
      const ctx = withJob(makeTestCtx({
        invoke: async () => { invokeCalled = true; return "/tmp/usb"; },
        emitStatus: (text) => { lastStatus = text; }
      }), "job-1", jobType);
      const result = await pickUsbFolder(ctx);
      assert.equal(result, null);
      assert.equal(invokeCalled, false);
      assert.match(lastStatus, /wait/i);
    });
  }
});

test("validateAndSetUsbRoot blocks every USB-locking job type before backend validation", async (t) => {
  for (const jobType of USB_ROOT_LOCKING_JOB_TYPES) {
    await t.test(`blocked for jobType=${jobType}`, async () => {
      let commandCalled = false;
      let lastStatus = "";
      const ctx = {
        state: { activeJobId: "job-1", activeJobType: jobType, runningJobs: new Map([["job-1", { jobType }]]), usbRoot: null },
        el: {},
        command: async () => { commandCalled = true; return {}; },
        emitStatus: (text) => { lastStatus = text; }
      };
      const valid = await validateAndSetUsbRoot(ctx, "/tmp/new-usb");
      assert.equal(valid, false);
      assert.equal(commandCalled, false);
      assert.match(lastStatus, /wait/i);
    });
  }
});

test("re-selecting the same USB clears its lists, so the player's changes show", async () => {
  const responses = {
    validate_usb_root: { valid: true, normalizedRoot: "/tmp/usb", hasWriteAccess: true,
      hasVendorRoot: true, hasContents: true, hasPdb: true, warnings: [] },
    // A named drive, or the naming prompt would wait for an answer.
    get_usb_device_name: { name: "Club Stick" },
    get_usb_audio_formats: { formats: [] },
    list_usb_devices: { items: [] },
  };
  const ctx = makeTestCtx({ command: async (name) => responses[name] ?? {} });
  Object.assign(ctx.state, {
    usbRoot: "/tmp/usb",
    usbPlaylists: [{ id: "p1", name: "Old" }],
    histories: [{ id: "h1", name: "Old session" }],
  });

  await validateAndSetUsbRoot(ctx, "/tmp/usb");

  assert.equal(ctx.state.usbPlaylists.length, 0);
  assert.equal(ctx.state.histories.length, 0);
});

test("USB root controls and recent-root buttons reflect lock state", () => {
  const lockedEl = {
    selectUsbFolderBtn: { disabled: false, title: "" },
    usbRecentList: { querySelectorAll: () => [{ disabled: false }, { disabled: false }] },
    exportPlaylistBtn: { disabled: false }
  };
  setUsbRootControlsLocked({ el: lockedEl }, true);
  assert.equal(lockedEl.selectUsbFolderBtn.disabled, true);
  assert.match(lockedEl.selectUsbFolderBtn.title, /wait/i);
  assert.equal(lockedEl.exportPlaylistBtn.disabled, true);

  const unlockedEl = {
    selectUsbFolderBtn: { disabled: true, title: "wait" },
    usbRecentList: { querySelectorAll: () => [] },
    exportPlaylistBtn: { disabled: true }
  };
  let updateCalled = false;
  setUsbRootControlsLocked({
    el: unlockedEl,
    updatePlaylistExportButtons: () => { updateCalled = true; }
  }, false);
  assert.equal(unlockedEl.selectUsbFolderBtn.disabled, false);
  assert.equal(updateCalled, true);
  assert.equal(unlockedEl.exportPlaylistBtn.disabled, true);

  const ctx = withJob(makeTestCtx(), "job-1", "diagnostics");
  ctx.state.usbRecentRoots = ["/tmp/usb1"];
  renderUsbRecentRoots(ctx);
  assert.equal(ctx.el.usbRecentList.querySelector("button").disabled, true);
});

test("updatePlaylistExportButtons respects USB-root lock state", () => {
  for (const [activeJobId, activeJobType, expectedDisabled] of [
    ["job-1", "export", true],
    [null, null, false]
  ]) {
    const ctx = withJob(makeTestCtx(), activeJobId, activeJobType);
    ctx.state.playlists = [{ id: "p1", name: "My Playlist", tracks: [] }];
    ctx.state.currentPlaylistId = "p1";
    ctx.el.exportPlaylistBtn.disabled = !expectedDisabled;
    updatePlaylistExportButtons(ctx);
    assert.equal(ctx.el.exportPlaylistBtn.disabled, expectedDisabled);
  }
});

test("handleJobEvent locks USB controls only for USB-locking job lifecycles", () => {
  for (const [jobType, endEvent, expected] of [
    ["diagnostics", "job.completed", [true, false]],
    ["analysis", "job.completed", []],
    ["usb_write", "job.failed", [true, false]]
  ]) {
    const lockCalls = [];
    const ctx = jobCtx(lockCalls);
    handleJobEvent(ctx, { event: "job.started", jobId: "job-1", jobType });
    handleJobEvent(ctx, { event: endEvent, jobId: "job-1", jobType });
    assert.deepEqual(lockCalls, expected, jobType);
  }
});
