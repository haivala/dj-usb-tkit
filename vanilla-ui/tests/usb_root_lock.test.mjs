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
  isUsbRootChangeBlocked,
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

function jobHarness() {
  const el = {
    progressFooter: { classList: makeClassList() },
    progressFill: { style: {} },
    progressText: {},
    progressPauseBtn: { setAttribute() {} },
    progressCancelAnalysisBtn: {}
  };
  el.progressFooter.querySelector = () => null;
  return { state: { activeJobId: null, activeJobType: null }, el };
}

function recentRootDocumentStub(buttons) {
  return {
    createElement: (tag) => {
      const element = {
        tag,
        classList: makeClassList(),
        dataset: {},
        style: {},
        _children: [],
        appendChild(child) { this._children.push(child); }
      };
      if (tag === "button") buttons.push(element);
      return element;
    }
  };
}

test("pickUsbFolder blocks every USB-locking job type and allows unlocked or unrelated jobs", async (t) => {
  for (const jobType of USB_ROOT_LOCKING_JOB_TYPES) {
    await t.test(`blocked for jobType=${jobType}`, async () => {
      let invokeCalled = false;
      let lastStatus = "";
      const result = await pickUsbFolder({
        invoke: async () => { invokeCalled = true; return "/tmp/usb"; },
        validateAndSetUsbRoot: async () => {},
        state: { activeJobId: "job-1", activeJobType: jobType },
        emitStatus: (text) => { lastStatus = text; }
      });
      assert.equal(result, null);
      assert.equal(invokeCalled, false);
      assert.match(lastStatus, /wait/i);
    });
  }

  for (const [activeJobId, activeJobType] of [[null, null], ["job-1", "analysis"]]) {
    let invokeCalled = false;
    let validateCalled = false;
    await pickUsbFolder({
      invoke: async () => { invokeCalled = true; return "/tmp/usb"; },
      validateAndSetUsbRoot: async () => { validateCalled = true; },
      state: { activeJobId, activeJobType },
      emitStatus: () => {}
    });
    assert.equal(invokeCalled, true);
    assert.equal(validateCalled, true);
  }
});

test("validateAndSetUsbRoot blocks every USB-locking job type before backend validation", async (t) => {
  for (const jobType of USB_ROOT_LOCKING_JOB_TYPES) {
    await t.test(`blocked for jobType=${jobType}`, async () => {
      let commandCalled = false;
      let lastStatus = "";
      const valid = await validateAndSetUsbRoot(
        { activeJobId: "job-1", activeJobType: jobType, usbRoot: null },
        {},
        "/tmp/new-usb",
        false,
        {
          command: async () => { commandCalled = true; return {}; },
          setStatus: (text) => { lastStatus = text; }
        }
      );
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
  setUsbRootControlsLocked({}, lockedEl, true, {});
  assert.equal(lockedEl.selectUsbFolderBtn.disabled, true);
  assert.match(lockedEl.selectUsbFolderBtn.title, /wait/i);
  assert.equal(lockedEl.exportPlaylistBtn.disabled, true);

  const unlockedEl = {
    selectUsbFolderBtn: { disabled: true, title: "wait" },
    usbRecentList: { querySelectorAll: () => [] },
    exportPlaylistBtn: { disabled: true }
  };
  let updateCalled = false;
  setUsbRootControlsLocked({}, unlockedEl, false, {
    updatePlaylistExportButtons: () => { updateCalled = true; }
  });
  assert.equal(unlockedEl.selectUsbFolderBtn.disabled, false);
  assert.equal(updateCalled, true);
  assert.equal(unlockedEl.exportPlaylistBtn.disabled, true);

  const buttons = [];
  renderUsbRecentRoots({
    usbRecentRow: { classList: makeClassList() },
    usbRecentList: { innerHTML: "", appendChild(btn) { this._btn = btn; } }
  }, ["/tmp/usb1"], recentRootDocumentStub(buttons), {
    activeJobId: "job-1",
    activeJobType: "diagnostics"
  });
  assert.equal(buttons[0].disabled, true);
});

test("updatePlaylistExportButtons respects USB-root lock state", () => {
  for (const [state, expectedDisabled] of [
    [{ activeJobId: "job-1", activeJobType: "export" }, true],
    [{ activeJobId: null, activeJobType: null }, false]
  ]) {
    const el = { exportPlaylistBtn: { disabled: !expectedDisabled, textContent: "", dataset: {} } };
    updatePlaylistExportButtons(state, el, {
      getCurrentPlaylist: () => ({ name: "My Playlist", tracks: [] }),
      computeExportButtonState: () => ({ text: "Export", title: "" }),
      isUsbOriginTrack: () => false,
      trackHasCoreAnalysis: () => true,
      isUsbRootChangeBlocked
    });
    assert.equal(el.exportPlaylistBtn.disabled, expectedDisabled);
  }
});

test("handleJobEvent locks USB controls only for USB-locking job lifecycles", () => {
  const locking = jobHarness();
  const lockingCalls = [];
  const deps = {
    debugFrontendLog: () => {},
    setUsbRootControlsLocked: (locked) => lockingCalls.push(locked)
  };
  handleJobEvent(locking.state, locking.el, { event: "job.started", jobId: "job-1", jobType: "diagnostics" }, deps);
  handleJobEvent(locking.state, locking.el, { event: "job.completed", jobId: "job-1", jobType: "diagnostics" }, deps);
  assert.deepEqual(lockingCalls, [true, false]);

  const analysis = jobHarness();
  const analysisCalls = [];
  const analysisDeps = {
    debugFrontendLog: () => {},
    setUsbRootControlsLocked: (locked) => analysisCalls.push(locked)
  };
  handleJobEvent(analysis.state, analysis.el, { event: "job.started", jobId: "job-1", jobType: "analysis" }, analysisDeps);
  handleJobEvent(analysis.state, analysis.el, { event: "job.completed", jobId: "job-1", jobType: "analysis" }, analysisDeps);
  assert.deepEqual(analysisCalls, []);

  const failed = jobHarness();
  const failedCalls = [];
  const failedDeps = {
    debugFrontendLog: () => {},
    setUsbRootControlsLocked: (locked) => failedCalls.push(locked)
  };
  handleJobEvent(failed.state, failed.el, { event: "job.started", jobId: "job-1", jobType: "usb_write" }, failedDeps);
  handleJobEvent(failed.state, failed.el, { event: "job.failed", jobId: "job-1", jobType: "usb_write" }, failedDeps);
  assert.deepEqual(failedCalls, [true, false]);
});

test("loadUsbDevices maps backend devices and recovers to an empty list on failure", async () => {
  const state = {};
  const items = [
    { id: "dev-1", rootPath: "/mnt/usbA", mounted: true },
    { id: "dev-2", rootPath: "/mnt/usbB", mounted: false }
  ];
  const rows = await loadUsbDevices(state, async (name) => {
    assert.equal(name, "list_usb_devices");
    return { items };
  });
  assert.deepEqual(rows, ["/mnt/usbA", "/mnt/usbB"]);
  assert.deepEqual(state.usbRecentRoots, ["/mnt/usbA", "/mnt/usbB"]);
  assert.deepEqual(state.usbDevices, items);

  const failed = {};
  assert.deepEqual(await loadUsbDevices(failed, async () => { throw new Error("boom"); }), []);
  assert.deepEqual(failed.usbRecentRoots, []);
});

test("pruneUsbDevice calls the backend and reloads only when an id is provided", async () => {
  const calls = [];
  let reloaded = false;
  await pruneUsbDevice({}, "dev-1", {
    command: async (name, payload) => { calls.push({ name, payload }); },
    reload: async () => { reloaded = true; }
  });
  assert.deepEqual(calls, [{ name: "prune_usb_device", payload: { id: "dev-1" } }]);
  assert.equal(reloaded, true);

  await pruneUsbDevice({}, null, { command: async (...args) => calls.push(args) });
  assert.equal(calls.length, 1);
});
