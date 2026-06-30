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
  setProgress,
  dismissProgress,
  startProgressHeartbeat,
  stopProgressHeartbeat,
  handleJobEvent,
  formatJobStatusText
} from "../job_manager.mjs";

function makeClassList() {
  const classes = new Set();
  return {
    add(name) { classes.add(name); },
    remove(name) { classes.delete(name); },
    toggle(name, force) {
      if (force) classes.add(name);
      else classes.delete(name);
    },
    contains(name) { return classes.has(name); },
    entries() { return classes; }
  };
}

function makeEl() {
  return {
    progressFooter: {
      classList: makeClassList(),
      querySelector: () => ({ setAttribute: () => {} })
    },
    progressFill: { style: { width: "" } },
    progressText: { textContent: "" }
  };
}

function makeState() {
  return {
    progressPercent: 0,
    progressBaseText: "Idle",
    progressHeartbeatTimer: null,
    progressStartedAtMs: 0,
    lastJobEventAtMs: 0,
    activeJobId: null
  };
}

test("setProgress sets clamped percent and text on state and el", () => {
  const state = makeState();
  const el = makeEl();
  setProgress(state, el, true, 42, "Scanning...");
  assert.equal(state.progressPercent, 42);
  assert.equal(state.progressBaseText, "Scanning...");
  assert.equal(el.progressFill.style.width, "42%");
  assert.equal(el.progressText.textContent, "Scanning...");
  assert.ok(el.progressFooter.classList.contains("active"));
});

test("setProgress clamps percent to 0-100", () => {
  const state = makeState();
  const el = makeEl();
  setProgress(state, el, true, 150, "Over");
  assert.equal(state.progressPercent, 100);
  setProgress(state, el, true, -10, "Under");
  assert.equal(state.progressPercent, 0);
});

test("setProgress uses default text when empty", () => {
  const state = makeState();
  const el = makeEl();
  setProgress(state, el, true, 50, "");
  assert.equal(state.progressBaseText, "Working...");
  setProgress(state, el, false, 0, "");
  assert.equal(state.progressBaseText, "Idle");
});

test("setProgress toggles error and dismissable classes", () => {
  const state = makeState();
  const el = makeEl();
  setProgress(state, el, true, 100, "Failed", { error: true, dismissable: true });
  assert.ok(el.progressFooter.classList.contains("error"));
  assert.ok(el.progressFooter.classList.contains("dismissable"));
  setProgress(state, el, false, 0, "Idle");
  assert.ok(!el.progressFooter.classList.contains("error"));
  assert.ok(!el.progressFooter.classList.contains("dismissable"));
});

test("dismissProgress resets to idle", () => {
  const state = makeState();
  const el = makeEl();
  setProgress(state, el, true, 75, "Working...");
  dismissProgress(state, el);
  assert.equal(state.progressPercent, 0);
  assert.equal(state.progressBaseText, "Idle");
  assert.ok(!el.progressFooter.classList.contains("active"));
});

test("startProgressHeartbeat sets timer and timestamps", () => {
  const state = makeState();
  const el = makeEl();
  const timers = [];
  const origSetInterval = window.setInterval;
  window.setInterval = (fn, ms) => { const id = 42; timers.push({ fn, ms }); return id; };
  try {
    startProgressHeartbeat(state, el);
    assert.equal(state.progressHeartbeatTimer, 42);
    assert.ok(state.progressStartedAtMs > 0);
    assert.ok(state.lastJobEventAtMs > 0);
    assert.equal(timers.length, 1);
    assert.equal(timers[0].ms, 1000);
  } finally {
    window.setInterval = origSetInterval;
  }
});

test("startProgressHeartbeat is no-op when timer already exists", () => {
  const state = makeState();
  state.progressHeartbeatTimer = 99;
  const el = makeEl();
  const origSetInterval = window.setInterval;
  let called = false;
  window.setInterval = () => { called = true; return 100; };
  try {
    startProgressHeartbeat(state, el);
    assert.ok(!called);
    assert.equal(state.progressHeartbeatTimer, 99);
  } finally {
    window.setInterval = origSetInterval;
  }
});

test("stopProgressHeartbeat clears timer", () => {
  const state = makeState();
  state.progressHeartbeatTimer = 77;
  let clearedId = null;
  const origClearInterval = window.clearInterval;
  window.clearInterval = (id) => { clearedId = id; };
  try {
    stopProgressHeartbeat(state);
    assert.equal(clearedId, 77);
    assert.equal(state.progressHeartbeatTimer, null);
  } finally {
    window.clearInterval = origClearInterval;
  }
});

test("stopProgressHeartbeat is no-op without timer", () => {
  const state = makeState();
  let called = false;
  const origClearInterval = window.clearInterval;
  window.clearInterval = () => { called = true; };
  try {
    stopProgressHeartbeat(state);
    assert.ok(!called);
  } finally {
    window.clearInterval = origClearInterval;
  }
});

test("handleJobEvent job.started sets active job and progress", () => {
  const state = makeState();
  const el = makeEl();
  let statusText = "";
  const origSetInterval = window.setInterval;
  window.setInterval = (fn, ms) => 42;
  try {
    handleJobEvent(state, el, {
      event: "job.started",
      jobId: "j1",
      jobType: "export",
      stage: "copy",
      message: "Copying files",
      percent: 5
    }, {
      debugFrontendLog: () => {},
      pushEventLog: () => {},
      applyRealtimeAnalyzedTrackUpdate: () => Promise.resolve(),
      setStatus: (t) => { statusText = t; }
    });
    assert.equal(state.activeJobId, "j1");
    assert.equal(state.progressPercent, 5);
    assert.equal(statusText, "Copying files");
  } finally {
    window.setInterval = origSetInterval;
  }
});

test("formatJobStatusText applies usb import stage-specific override", () => {
  assert.equal(
    formatJobStatusText("usb_read", "fetch_usb_playlists", "Importing USB playlists..."),
    "Importing USB playlists..."
  );
  assert.equal(
    formatJobStatusText("usb_read", "fetch_usb_histories", "Importing USB histories..."),
    "Importing USB histories..."
  );
});

test("formatJobStatusText uses fallback template for unknown stage", () => {
  assert.equal(
    formatJobStatusText("scan", "unknown_stage", "Running"),
    "Running"
  );
  assert.equal(formatJobStatusText("scan", "unknown_stage", ""), "");
});

test("handleJobEvent omits usb_read fetch status prefix for playlist and history imports", () => {
  const state = makeState();
  const el = makeEl();
  const seen = [];
  const origSetInterval = window.setInterval;
  window.setInterval = () => 42;
  try {
    handleJobEvent(state, el, {
      event: "job.started",
      jobId: "j-usb-playlists",
      jobType: "usb_read",
      stage: "fetch_usb_playlists",
      message: "Importing USB playlists...",
      percent: 10
    }, {
      debugFrontendLog: () => {},
      pushEventLog: () => {},
      applyRealtimeAnalyzedTrackUpdate: () => Promise.resolve(),
      setStatus: (t) => { seen.push(t); }
    });
    assert.equal(seen.at(-1), "Importing USB playlists...");

    handleJobEvent(state, el, {
      event: "job.progress",
      jobId: "j-usb-playlists",
      jobType: "usb_read",
      stage: "fetch_usb_histories",
      message: "Importing USB histories...",
      percent: 60
    }, {
      debugFrontendLog: () => {},
      pushEventLog: () => {},
      applyRealtimeAnalyzedTrackUpdate: () => Promise.resolve(),
      setStatus: (t) => { seen.push(t); }
    });
    assert.equal(seen.at(-1), "Importing USB histories...");
  } finally {
    window.setInterval = origSetInterval;
  }
});

test("handleJobEvent job.completed resets active job", () => {
  const state = makeState();
  state.activeJobId = "j1";
  const el = makeEl();
  const origSetTimeout = window.setTimeout;
  const origClearInterval = window.clearInterval;
  window.setTimeout = (fn) => fn();
  window.clearInterval = () => {};
  try {
    handleJobEvent(state, el, {
      event: "job.completed",
      jobId: "j1",
      jobType: "export",
      message: "Done"
    }, {
      debugFrontendLog: () => {},
      pushEventLog: () => {},
      applyRealtimeAnalyzedTrackUpdate: () => Promise.resolve(),
      setStatus: () => {}
    });
    assert.equal(state.activeJobId, null);
  } finally {
    window.setTimeout = origSetTimeout;
    window.clearInterval = origClearInterval;
  }
});

test("handleJobEvent ignores events for different job id", () => {
  const state = makeState();
  state.activeJobId = "j1";
  const el = makeEl();
  let statusCalled = false;
  handleJobEvent(state, el, {
    event: "job.progress",
    jobId: "j-other",
    percent: 50,
    message: "Other job"
  }, {
    debugFrontendLog: () => {},
    pushEventLog: () => {},
    applyRealtimeAnalyzedTrackUpdate: () => Promise.resolve(),
    setStatus: () => { statusCalled = true; }
  });
  assert.ok(!statusCalled);
});

test("handleJobEvent ignores null/non-object payload", () => {
  const state = makeState();
  const el = makeEl();
  const deps = {
    debugFrontendLog: () => {},
    pushEventLog: () => {},
    applyRealtimeAnalyzedTrackUpdate: () => Promise.resolve(),
    setStatus: () => {}
  };
  // Should not throw
  handleJobEvent(state, el, null, deps);
  handleJobEvent(state, el, "string", deps);
  handleJobEvent(state, el, undefined, deps);
});

test("handleJobEvent analysis progress calls applyRealtimeAnalyzedTrackUpdate", () => {
  const state = makeState();
  const el = makeEl();
  let realtimeCalled = false;
  handleJobEvent(state, el, {
    event: "job.progress",
    jobId: "j1",
    jobType: "analysis",
    stage: "analyze_new_tracks",
    trackId: "t5",
    trackTitle: "Track E",
    percent: 40,
    message: "Analyzing"
  }, {
    debugFrontendLog: () => {},
    pushEventLog: () => {},
    applyRealtimeAnalyzedTrackUpdate: () => { realtimeCalled = true; return Promise.resolve(); },
    setStatus: () => {}
  });
  assert.ok(realtimeCalled);
});

test("handleJobEvent analysis failure pushes error to event log", () => {
  const state = makeState();
  const el = makeEl();
  const logged = [];
  handleJobEvent(state, el, {
    event: "job.progress",
    stage: "analyze_new_tracks",
    trackId: "t6",
    trackTitle: "Bad Track",
    failed: true,
    errorMessage: "decode failed",
    filePath: "/music/bad.mp3"
  }, {
    debugFrontendLog: () => {},
    pushEventLog: (entry) => { logged.push(entry); },
    applyRealtimeAnalyzedTrackUpdate: () => Promise.resolve(),
    setStatus: () => {}
  });
  assert.equal(logged.length, 1);
  assert.equal(logged[0].level, "error");
  assert.ok(logged[0].message.includes("Bad Track"));
  assert.ok(logged[0].message.includes("decode failed"));
  assert.equal(logged[0].details, "/music/bad.mp3");
});

test("handleJobEvent job.failed resets active job", () => {
  const state = makeState();
  state.activeJobId = "j1";
  const el = makeEl();
  const origSetTimeout = window.setTimeout;
  const origClearInterval = window.clearInterval;
  window.setTimeout = (fn) => fn();
  window.clearInterval = () => {};
  try {
    handleJobEvent(state, el, {
      event: "job.failed",
      jobId: "j1",
      message: "Export failed"
    }, {
      debugFrontendLog: () => {},
      pushEventLog: () => {},
      applyRealtimeAnalyzedTrackUpdate: () => Promise.resolve(),
      setStatus: () => {}
    });
    assert.equal(state.activeJobId, null);
  } finally {
    window.setTimeout = origSetTimeout;
    window.clearInterval = origClearInterval;
  }
});
