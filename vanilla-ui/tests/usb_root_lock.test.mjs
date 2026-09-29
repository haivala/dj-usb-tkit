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
  loadUsbDevices,
  pickUsbFolder,
  pruneUsbDevice,
  renderUsbRecentRoots,
  setUsbRootControlsLocked,
  validateAndSetUsbRoot
} from "../components/usb/actions.mjs";
import { updatePlaylistExportButtons } from "../components/playlist/actions.mjs";
import { handleJobEvent } from "../job_manager.mjs";
import { makeClassList } from "./fixtures/dom.mjs";
import { makeTestCtx, withSilencedConsole } from "./test_helpers.mjs";

// handleJobEvent's collaborators as no-ops, recording USB lock toggles.
function jobCtx(lockCalls) {
  const el = {
    progressFooter: { classList: makeClassList(), querySelector: () => null },
    progressFill: { style: {} },
    progressText: {},
    progressPauseBtn: { setAttribute() {} },
    progressCancelAnalysisBtn: {}
  };
  return {
    state: { activeJobId: null, activeJobType: null, usbJobIdleWaiters: [] },
    el,
    debugFrontendLog: () => {},
    emitMessage: () => {},
    applyRealtimeAnalyzedTrackUpdate: () => Promise.resolve(),
    refreshSourceRootAnalysisStatus: () => {},
    applyLibraryDurationSummary: () => {},
    setTrackAnalyzingState: () => {},
    setUsbRootControlsLocked: (locked) => lockCalls.push(locked)
  };
}

function withJob(ctx, activeJobId, activeJobType) {
  Object.assign(ctx.state, { activeJobId, activeJobType });
  return ctx;
}

test("pickUsbFolder blocks every USB-locking job type and allows unlocked or unrelated jobs", async (t) => {
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

  for (const [activeJobId, activeJobType] of [[null, null], ["job-1", "analysis"]]) {
    let invokeCalled = false;
    const commands = [];
    const ctx = withJob(makeTestCtx({
      runUsbDiagnostics: async () => {},
      invoke: async () => { invokeCalled = true; return "/tmp/usb"; },
      command: async (name) => { commands.push(name); return {}; },
      emitStatus: () => {}
    }), activeJobId, activeJobType);
    await pickUsbFolder(ctx);
    assert.equal(invokeCalled, true);
    assert.ok(commands.includes("validate_usb_root"));
  }
});

test("validateAndSetUsbRoot blocks every USB-locking job type before backend validation", async (t) => {
  for (const jobType of USB_ROOT_LOCKING_JOB_TYPES) {
    await t.test(`blocked for jobType=${jobType}`, async () => {
      let commandCalled = false;
      let lastStatus = "";
      const ctx = {
        state: { activeJobId: "job-1", activeJobType: jobType, usbRoot: null },
        el: {},
        command: async () => { commandCalled = true; return {}; },
        emitStatus: (text) => { lastStatus = text; }
      };
      const valid = await validateAndSetUsbRoot(ctx, "/tmp/new-usb", false);
      assert.equal(valid, false);
      assert.equal(commandCalled, false);
      assert.match(lastStatus, /wait/i);
    });
  }
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

test("handleJobEvent locks USB controls only for USB-locking job lifecycles", async () => {
  // handleJobEvent unconditionally console.logs a "[job-event]" trace line
  // for every event it handles -- expected here since we're calling it
  // directly and repeatedly, so silence it to keep the test run's terminal
  // output clean.
  await withSilencedConsole(() => {
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
});

test("loadUsbDevices maps backend devices and recovers to an empty list on failure", async () => {
  const items = [
    { id: "dev-1", rootPath: "/mnt/usbA", mounted: true },
    { id: "dev-2", rootPath: "/mnt/usbB", mounted: false }
  ];
  const ctx = makeTestCtx({
    command: async (name) => {
      assert.equal(name, "list_usb_devices");
      return { items };
    }
  });
  const rows = await loadUsbDevices(ctx);
  assert.deepEqual(rows, ["/mnt/usbA", "/mnt/usbB"]);
  assert.deepEqual(ctx.state.usbRecentRoots, ["/mnt/usbA", "/mnt/usbB"]);
  assert.deepEqual(ctx.state.usbDevices, items);
  assert.equal(ctx.el.usbRecentList.querySelectorAll(".usb-cfg-recent-btn").length, 2);

  const failed = makeTestCtx({ command: async () => { throw new Error("boom"); } });
  // loadUsbDevices logs the caught failure via console.warn -- expected
  // here since we're deliberately exercising that path.
  const failedRows = await withSilencedConsole(() => loadUsbDevices(failed));
  assert.deepEqual(failedRows, []);
  assert.deepEqual(failed.state.usbRecentRoots, []);
});

test("pruneUsbDevice calls the backend and reloads only when an id is provided", async () => {
  const calls = [];
  const ctx = makeTestCtx({
    command: async (name, payload) => { calls.push({ name, payload }); return { items: [] }; }
  });
  await pruneUsbDevice(ctx, "dev-1");
  assert.deepEqual(calls, [
    { name: "prune_usb_device", payload: { id: "dev-1" } },
    { name: "list_usb_devices", payload: undefined }
  ]);

  await pruneUsbDevice(ctx, null);
  assert.equal(calls.length, 2);
});
