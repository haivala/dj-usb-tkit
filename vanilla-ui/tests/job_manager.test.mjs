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
  dismissProgress,
  formatJobStatusText,
  handleJobEvent,
  pauseProgressHeartbeat,
  resumeProgressHeartbeat,
  setProgress,
  startProgressHeartbeat,
  stopProgressHeartbeat
} from "../job_manager.mjs";
import { makeClassList } from "./fixtures/dom.mjs";

function makeButton() {
  return {
    hidden: true,
    innerHTML: "",
    attrs: {},
    setAttribute(name, value) {
      this.attrs[name] = value;
    }
  };
}

function makeEl() {
  return {
    progressFooter: {
      classList: makeClassList(),
      querySelector: () => ({ setAttribute: () => {} })
    },
    progressFill: { style: { width: "" } },
    progressText: { textContent: "" },
    progressPauseBtn: makeButton(),
    progressCancelAnalysisBtn: makeButton()
  };
}

function makeState() {
  return {
    progressPercent: 0,
    progressBaseText: "Idle",
    progressHeartbeatTimer: null,
    progressStartedAtMs: 0,
    progressPausedAtMs: null,
    lastJobEventAtMs: 0,
    activeJobId: null,
    activeJobType: null
  };
}

function withTimerStubs(fn) {
  const original = {
    setInterval: window.setInterval,
    clearInterval: window.clearInterval,
    setTimeout: window.setTimeout,
    globalSetTimeout: globalThis.setTimeout
  };
  const timers = [];
  const cleared = [];
  window.setInterval = (tick, ms) => {
    const id = timers.length + 1;
    timers.push({ id, tick, ms });
    return id;
  };
  window.clearInterval = (id) => {
    cleared.push(id);
  };
  window.setTimeout = (callback) => {
    callback();
    return 1;
  };
  globalThis.setTimeout = window.setTimeout;
  try {
    return fn({ timers, cleared });
  } finally {
    window.setInterval = original.setInterval;
    window.clearInterval = original.clearInterval;
    window.setTimeout = original.setTimeout;
    globalThis.setTimeout = original.globalSetTimeout;
  }
}

test("progress primitives clamp, classify, dismiss, and pause/resume heartbeat", () => {
  withTimerStubs(({ timers, cleared }) => {
    const state = makeState();
    const el = makeEl();

    setProgress(state, el, true, 150, "", { error: true, dismissable: true });
    assert.equal(state.progressPercent, 100);
    assert.equal(state.progressBaseText, "Working...");
    assert.equal(el.progressFill.style.width, "100%");
    assert.equal(el.progressFooter.classList.contains("active"), true);
    assert.equal(el.progressFooter.classList.contains("error"), true);
    assert.equal(el.progressFooter.classList.contains("dismissable"), true);

    setProgress(state, el, true, -10, "Under");
    assert.equal(state.progressPercent, 0);
    dismissProgress(state, el);
    assert.equal(state.progressBaseText, "Idle");
    assert.equal(el.progressFooter.classList.contains("active"), false);

    startProgressHeartbeat(state, el);
    startProgressHeartbeat(state, el);
    assert.equal(state.progressHeartbeatTimer, 1);
    assert.equal(timers.length, 1);
    assert.equal(timers[0].ms, 1000);

    el.progressFooter.classList.add("active");
    state.progressBaseText = "Analyzing";
    pauseProgressHeartbeat(state, el);
    timers[0].tick();
    assert.equal(el.progressText.textContent, "Analyzing (paused)");

    resumeProgressHeartbeat(state, el);
    assert.equal(state.progressPausedAtMs, null);
    assert.match(el.progressText.textContent, /Analyzing \(\d+s\)$/);

    pauseProgressHeartbeat(state, el);
    stopProgressHeartbeat(state);
    assert.deepEqual(cleared, [1]);
    assert.equal(state.progressHeartbeatTimer, null);
    assert.equal(state.progressPausedAtMs, null);
  });
});

test("formatJobStatusText is a passthrough of the backend message", () => {
  // Every job:event carries a non-empty message now (the backend substitutes
  // the job's started-message for a blank progress update).
  assert.equal(formatJobStatusText("usb_read", "fetch_usb_playlists", "USB: Reading playlists"), "USB: Reading playlists");
  assert.equal(formatJobStatusText("scan", "unknown_stage", "Running"), "Running");
  assert.equal(formatJobStatusText("scan", "unknown_stage", ""), "");
});

test("handleJobEvent tracks lifecycle, ignores stale jobs, and toggles USB locks", () => {
  withTimerStubs(() => {
    const state = makeState();
    const el = makeEl();
    const messages = [];
    const usbLocks = [];
    const deps = {
      debugFrontendLog: () => {},
      applyRealtimeAnalyzedTrackUpdate: () => Promise.resolve(),
      emitMessage: (message) => messages.push(message),
      setUsbRootControlsLocked: (locked) => usbLocks.push(locked)
    };

    handleJobEvent(state, el, {
      event: "job.started",
      jobId: "export-1",
      jobType: "export",
      stage: "copy",
      message: "Copying files",
      percent: 5
    }, deps);
    assert.equal(state.activeJobId, "export-1");
    assert.equal(state.activeJobType, "export");
    assert.equal(state.progressPercent, 5);
    assert.deepEqual(usbLocks, [true]);
    assert.equal(messages.at(-1).status.text, "Copying files");

    handleJobEvent(state, el, {
      event: "job.progress",
      jobId: "other-job",
      jobType: "export",
      message: "Wrong job",
      percent: 80
    }, deps);
    assert.equal(state.progressPercent, 5);

    handleJobEvent(state, el, {
      event: "job.completed",
      jobId: "export-1",
      jobType: "export",
      message: "Done"
    }, deps);
    assert.equal(state.activeJobId, null);
    assert.equal(state.activeJobType, null);
    assert.deepEqual(usbLocks, [true, false]);
    assert.equal(el.progressFooter.classList.contains("active"), false);

    handleJobEvent(state, el, {
      event: "job.started",
      jobId: "analysis-1",
      jobType: "analysis",
      stage: "analyze_new_tracks",
      message: "Analyzing",
      percent: 0
    }, deps);
    assert.equal(el.progressPauseBtn.hidden, false);
    assert.equal(el.progressCancelAnalysisBtn.hidden, false);

    handleJobEvent(state, el, {
      event: "job.completed",
      jobId: "analysis-1",
      jobType: "analysis",
      message: "Analysis done"
    }, deps);
    assert.equal(el.progressPauseBtn.hidden, true);
    assert.equal(el.progressCancelAnalysisBtn.hidden, true);
  });
});

test("handleJobEvent applies analysis progress, duration totals, and delayed pause freeze", () => {
  withTimerStubs(() => {
    const state = makeState();
    state.activeJobId = "analysis-1";
    state.analysisPaused = true;
    state.analyzingTrackIds = new Set(["track-1", "track-2"]);
    const el = makeEl();
    const realtimePayloads = [];
    const durationSummaries = [];
    const deps = {
      debugFrontendLog: () => {},
      applyRealtimeAnalyzedTrackUpdate: (payload) => {
        realtimePayloads.push(payload);
        return Promise.resolve();
      },
      setTrackAnalyzingState: (trackId, active) => {
        if (active) state.analyzingTrackIds.add(trackId);
        else state.analyzingTrackIds.delete(trackId);
      },
      applyLibraryDurationSummary: (totalMs, unknownCount) => {
        durationSummaries.push([totalMs, unknownCount]);
      }
    };

    handleJobEvent(state, el, {
      event: "job.progress",
      jobId: "analysis-1",
      jobType: "analysis",
      stage: "analyze_new_tracks",
      trackId: "track-1",
      trackReady: true
    }, deps);
    assert.equal(realtimePayloads.length, 1);
    assert.equal(state.analyzingTrackIds.size, 1);
    assert.equal(state.progressPausedAtMs, null);
    assert.deepEqual(durationSummaries, []);

    handleJobEvent(state, el, {
      event: "job.progress",
      jobId: "analysis-1",
      jobType: "analysis",
      stage: "analyze_new_tracks",
      trackId: "track-2",
      trackReady: true,
      libraryTotalDurationMs: 3000,
      libraryDurationUnknownCount: 1
    }, deps);
    assert.equal(state.analyzingTrackIds.size, 0);
    assert.ok(state.progressPausedAtMs > 0);
    assert.match(el.progressText.textContent, /\(paused\)$/);
    assert.deepEqual(durationSummaries, [[3000, 1]]);
  });
});

test("handleJobEvent routes analysis and job failures to the event log", () => {
  withTimerStubs(() => {
    const state = makeState();
    const el = makeEl();
    const messages = [];
    const deps = {
      debugFrontendLog: () => {},
      applyRealtimeAnalyzedTrackUpdate: () => Promise.resolve(),
      emitMessage: (message) => messages.push(message)
    };

    handleJobEvent(state, el, {
      event: "job.progress",
      stage: "analyze_new_tracks",
      trackId: "t6",
      trackTitle: "Bad Track",
      failed: true,
      errorMessage: "decode failed",
      filePath: "/music/bad.mp3"
    }, deps);
    assert.equal(messages[0].level, "error");
    assert.equal(messages[0].source, "analysis");
    assert.match(messages[0].eventLog.text, /Bad Track.*decode failed/);
    assert.equal(messages[0].eventLog.details, "/music/bad.mp3");

    state.activeJobId = "export-1";
    handleJobEvent(state, el, {
      event: "job.failed",
      jobId: "export-1",
      jobType: "export",
      stage: "copy",
      message: "Export failed"
    }, deps);
    assert.equal(state.activeJobId, null);
    assert.equal(messages.at(-1).level, "error");
    assert.equal(messages.at(-1).eventLog.text, "Export failed");
  });
});
