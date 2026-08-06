import test from "node:test";
import assert from "node:assert/strict";

// job_manager.mjs references window.setInterval/clearInterval
if (typeof globalThis.window === "undefined") {
  globalThis.window = {
    setInterval: globalThis.setInterval,
    clearInterval: globalThis.clearInterval,
    setTimeout: globalThis.setTimeout
  };
}

import {
  USB_ROOT_LOCKING_JOB_TYPES,
  isUsbRootChangeBlocked,
  setUsbRootControlsLocked,
  pickUsbFolder,
  validateAndSetUsbRoot,
  renderUsbRecentRoots,
  loadUsbDevices,
  pruneUsbDevice
} from "../components/usb/actions.mjs";
import { updatePlaylistExportButtons } from "../components/playlist/actions.mjs";
import { handleJobEvent } from "../job_manager.mjs";
import { makeClassList } from "./fixtures/dom.mjs";

test("pickUsbFolder rejects while any locking job type is active, without opening the picker", async (t) => {
  for (const jobType of USB_ROOT_LOCKING_JOB_TYPES) {
    await t.test(`blocked for jobType=${jobType}`, async () => {
      const state = { activeJobId: "job-1", activeJobType: jobType };
      let invokeCalled = false;
      let lastStatus = "";
      const result = await pickUsbFolder({
        invoke: async () => { invokeCalled = true; return "/tmp/usb"; },
        validateAndSetUsbRoot: async () => {},
        state,
        emitStatus: (text) => { lastStatus = text; }
      });
      assert.equal(result, null);
      assert.equal(invokeCalled, false);
      assert.match(lastStatus, /wait/i);
    });
  }
});

test("pickUsbFolder proceeds when no locking job is active", async () => {
  const state = { activeJobId: null, activeJobType: null };
  let invokeCalled = false;
  let validateCalled = false;
  await pickUsbFolder({
    invoke: async () => { invokeCalled = true; return "/tmp/usb"; },
    validateAndSetUsbRoot: async () => { validateCalled = true; },
    state,
    emitStatus: () => {}
  });
  assert.equal(invokeCalled, true);
  assert.equal(validateCalled, true);
});

test("pickUsbFolder proceeds for unrelated job types (e.g. analysis)", async () => {
  const state = { activeJobId: "job-1", activeJobType: "analysis" };
  let invokeCalled = false;
  await pickUsbFolder({
    invoke: async () => { invokeCalled = true; return "/tmp/usb"; },
    validateAndSetUsbRoot: async () => {},
    state,
    emitStatus: () => {}
  });
  assert.equal(invokeCalled, true);
});

test("validateAndSetUsbRoot rejects while any locking job type is active, without calling the backend", async (t) => {
  for (const jobType of USB_ROOT_LOCKING_JOB_TYPES) {
    await t.test(`blocked for jobType=${jobType}`, async () => {
      const state = { activeJobId: "job-1", activeJobType: jobType, usbRoot: null };
      let commandCalled = false;
      let lastStatus = "";
      const valid = await validateAndSetUsbRoot(state, {}, "/tmp/new-usb", false, {
        command: async () => { commandCalled = true; return {}; },
        setStatus: (text) => { lastStatus = text; }
      });
      assert.equal(valid, false);
      assert.equal(commandCalled, false);
      assert.match(lastStatus, /wait/i);
    });
  }
});

test("setUsbRootControlsLocked disables the select button, every recent-root button, and the export button", () => {
  const state = {};
  const el = {
    selectUsbFolderBtn: { disabled: false, title: "" },
    usbRecentList: {
      querySelectorAll: () => [{ disabled: false }, { disabled: false }]
    },
    exportPlaylistBtn: { disabled: false }
  };
  setUsbRootControlsLocked(state, el, true, {});
  assert.equal(el.selectUsbFolderBtn.disabled, true);
  assert.match(el.selectUsbFolderBtn.title, /wait/i);
  assert.equal(el.exportPlaylistBtn.disabled, true);
});

test("setUsbRootControlsLocked unlocking calls updatePlaylistExportButtons rather than force-enabling export", () => {
  const state = {};
  const el = {
    selectUsbFolderBtn: { disabled: true, title: "wait" },
    usbRecentList: { querySelectorAll: () => [] },
    exportPlaylistBtn: { disabled: true }
  };
  let updateCalled = false;
  setUsbRootControlsLocked(state, el, false, {
    updatePlaylistExportButtons: () => { updateCalled = true; }
  });
  assert.equal(el.selectUsbFolderBtn.disabled, false);
  assert.equal(updateCalled, true);
  // Export button disabled state is left to updatePlaylistExportButtons, not force-set here.
  assert.equal(el.exportPlaylistBtn.disabled, true);
});

test("renderUsbRecentRoots disables recent-root buttons while locked", () => {
  const el = {
    usbRecentRow: { classList: makeClassList() },
    usbRecentList: { innerHTML: "", appendChild(btn) { this._btn = btn; } }
  };
  const btns = [];
  const documentStub = {
    createElement: (tag) => {
      const el = {
        tag,
        classList: makeClassList(),
        dataset: {},
        style: {},
        _children: [],
        appendChild(child) { this._children.push(child); }
      };
      if (tag === "button") btns.push(el);
      return el;
    }
  };
  renderUsbRecentRoots(el, ["/tmp/usb1"], documentStub, { activeJobId: "job-1", activeJobType: "diagnostics" });
  assert.equal(btns[0].disabled, true);
});

test("updatePlaylistExportButtons keeps the export button disabled while isUsbRootChangeBlocked is true, regardless of playlist selection", () => {
  const state = { activeJobId: "job-1", activeJobType: "export" };
  const el = { exportPlaylistBtn: { disabled: false, textContent: "", dataset: {} } };
  updatePlaylistExportButtons(state, el, {
    getCurrentPlaylist: () => ({ name: "My Playlist", tracks: [] }),
    computeExportButtonState: () => ({ text: "Export", title: "" }),
    isUsbOriginTrack: () => false,
    trackHasCoreAnalysis: () => true,
    isUsbRootChangeBlocked
  });
  assert.equal(el.exportPlaylistBtn.disabled, true);
});

test("updatePlaylistExportButtons enables the export button when nothing is blocking", () => {
  const state = { activeJobId: null, activeJobType: null };
  const el = { exportPlaylistBtn: { disabled: true, textContent: "", dataset: {} } };
  updatePlaylistExportButtons(state, el, {
    getCurrentPlaylist: () => ({ name: "My Playlist", tracks: [] }),
    computeExportButtonState: () => ({ text: "Export", title: "" }),
    isUsbOriginTrack: () => false,
    trackHasCoreAnalysis: () => true,
    isUsbRootChangeBlocked
  });
  assert.equal(el.exportPlaylistBtn.disabled, false);
});

function makeJobManagerHarness() {
  const state = { activeJobId: null, activeJobType: null };
  const el = {
    progressFooter: { classList: makeClassList() },
    progressFill: { style: {} },
    progressText: {},
    progressPauseBtn: { setAttribute() {}, },
    progressCancelAnalysisBtn: {},
  };
  el.progressFooter.querySelector = () => null;
  return { state, el };
}

test("handleJobEvent locks USB controls only for locking job types", () => {
  const { state, el } = makeJobManagerHarness();
  let lockedCalls = [];
  const deps = {
    debugFrontendLog: () => {},
    setUsbRootControlsLocked: (locked) => lockedCalls.push(locked)
  };

  handleJobEvent(state, el, { event: "job.started", jobId: "job-1", jobType: "diagnostics" }, deps);
  assert.deepEqual(lockedCalls, [true]);

  handleJobEvent(state, el, { event: "job.completed", jobId: "job-1", jobType: "diagnostics" }, deps);
  assert.deepEqual(lockedCalls, [true, false]);
});

test("handleJobEvent does not lock USB controls for unrelated job types", () => {
  const { state, el } = makeJobManagerHarness();
  let lockedCalls = [];
  const deps = {
    debugFrontendLog: () => {},
    setUsbRootControlsLocked: (locked) => lockedCalls.push(locked)
  };

  handleJobEvent(state, el, { event: "job.started", jobId: "job-1", jobType: "analysis" }, deps);
  handleJobEvent(state, el, { event: "job.completed", jobId: "job-1", jobType: "analysis" }, deps);
  assert.deepEqual(lockedCalls, []);
});

test("handleJobEvent unlocks USB controls on job.failed for locking job types", () => {
  const { state, el } = makeJobManagerHarness();
  let lockedCalls = [];
  const deps = {
    debugFrontendLog: () => {},
    setUsbRootControlsLocked: (locked) => lockedCalls.push(locked)
  };

  handleJobEvent(state, el, { event: "job.started", jobId: "job-1", jobType: "usb_write" }, deps);
  handleJobEvent(state, el, { event: "job.failed", jobId: "job-1", jobType: "usb_write" }, deps);
  assert.deepEqual(lockedCalls, [true, false]);
});

test("loadUsbDevices maps items[].rootPath into state.usbRecentRoots in backend order", async () => {
  const state = {};
  const items = [
    { id: "dev-1", rootPath: "/mnt/usbA", mounted: true },
    { id: "dev-2", rootPath: "/mnt/usbB", mounted: false }
  ];
  const command = async (name) => {
    assert.equal(name, "list_usb_devices");
    return { items };
  };
  const rows = await loadUsbDevices(state, command);
  assert.deepEqual(rows, ["/mnt/usbA", "/mnt/usbB"]);
  assert.deepEqual(state.usbRecentRoots, ["/mnt/usbA", "/mnt/usbB"]);
  assert.deepEqual(state.usbDevices, items);
});

test("loadUsbDevices recovers to an empty list on command failure", async () => {
  const state = {};
  const rows = await loadUsbDevices(state, async () => { throw new Error("boom"); });
  assert.deepEqual(rows, []);
  assert.deepEqual(state.usbRecentRoots, []);
});

test("pruneUsbDevice calls command('prune_usb_device', { id }) and re-loads the list", async () => {
  const state = {};
  const calls = [];
  let reloaded = false;
  await pruneUsbDevice(state, "dev-1", {
    command: async (name, payload) => { calls.push({ name, payload }); },
    reload: async () => { reloaded = true; }
  });
  assert.deepEqual(calls, [{ name: "prune_usb_device", payload: { id: "dev-1" } }]);
  assert.equal(reloaded, true);
});

test("pruneUsbDevice is a no-op without an id", async () => {
  const calls = [];
  await pruneUsbDevice({}, null, { command: async (...a) => calls.push(a) });
  assert.equal(calls.length, 0);
});
