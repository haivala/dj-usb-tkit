// Job progress bar management: set/dismiss/heartbeat/withProgress.

import { USB_ROOT_LOCKING_JOB_TYPES, isUsbRootChangeBlocked } from "./components/usb/actions.mjs";

export function setProgress(ctx, active, percent = 0, text = "", opts = {}) {
  const { state, el } = ctx;
  el.progressFooter.classList.toggle("active", active);
  el.progressFooter.classList.toggle("error", !!opts.error);
  el.progressFooter.classList.toggle("dismissable", !!opts.dismissable);
  const clamped = Math.max(0, Math.min(100, Number(percent) || 0));
  state.progressPercent = clamped;
  state.progressBaseText = text || (active ? "Working..." : "Idle");
  el.progressFill.style.width = `${clamped}%`;
  el.progressFooter
    .querySelector(".progress-track")
    ?.setAttribute("aria-valuenow", String(clamped));
  // The elapsed time lives in its own span, so per-track text updates never
  // blank it between heartbeat ticks.
  el.progressText.textContent = state.progressBaseText;
}

function elapsedSecs(state) {
  return Math.max(0, Math.floor((Date.now() - state.progressStartedAtMs) / 1000));
}

function renderProgressTime(ctx) {
  const { state, el } = ctx;
  if (state.progressPausedAtMs) el.progressTime.textContent = "(paused)";
  else if (!state.progressHeartbeatTimer) el.progressTime.textContent = "";
  else el.progressTime.textContent = `(${elapsedSecs(state)}s)`;
}

export function dismissProgress(ctx) {
  setProgress(ctx, false, 0, "Idle");
}

// Hide the footer `delayMs` after a job finished, so its final text stays
// readable. Only one hide is ever pending; a new job cancels it.
export function scheduleProgressIdle(ctx, delayMs) {
  const { state } = ctx;
  clearTimeout(state.progressIdleTimer);
  state.progressIdleTimer = setTimeout(() => {
    state.progressIdleTimer = null;
    setProgress(ctx, false, 0, "Idle");
    stopProgressHeartbeat(ctx);
  }, delayMs);
}

export function startProgressHeartbeat(ctx) {
  const { state, el } = ctx;
  // A new job inside the previous one's hide delay: drop that hide (it would
  // blank this job's footer) and restart the elapsed-time clock.
  if (state.progressIdleTimer) {
    clearTimeout(state.progressIdleTimer);
    state.progressIdleTimer = null;
    stopProgressHeartbeat(ctx);
  }
  if (state.progressHeartbeatTimer) return;
  state.progressStartedAtMs = Date.now();
  state.progressPausedAtMs = null;
  state.progressHeartbeatTimer = window.setInterval(() => {
    if (!el.progressFooter.classList.contains("active")) return;
    renderProgressTime(ctx);
  }, 1000);
  renderProgressTime(ctx);
}

export function stopProgressHeartbeat(ctx) {
  const { state } = ctx;
  if (!state.progressHeartbeatTimer) return;
  window.clearInterval(state.progressHeartbeatTimer);
  state.progressHeartbeatTimer = null;
  state.progressPausedAtMs = null;
  renderProgressTime(ctx);
}

export function pauseProgressHeartbeat(ctx) {
  const { state } = ctx;
  if (state.progressPausedAtMs) return;
  state.progressPausedAtMs = Date.now();
  renderProgressTime(ctx);
}

export function resumeProgressHeartbeat(ctx) {
  const { state } = ctx;
  if (!state.progressPausedAtMs) return;
  // Shift the start time forward by however long we were paused, so the
  // displayed elapsed time picks back up from where it left off instead of
  // counting the paused interval.
  state.progressStartedAtMs += Date.now() - state.progressPausedAtMs;
  state.progressPausedAtMs = null;
  renderProgressTime(ctx);
}

// Undo a freeze that turned out to be premature: pause was clicked in the
// gap after a worker had already picked up its next track but before that
// track's first piece event arrived, so it looked like nothing was in
// flight. The frozen interval was real work time, so unlike a resume the
// start time isn't shifted.
export function unfreezeProgressHeartbeat(ctx) {
  const { state } = ctx;
  if (!state.progressPausedAtMs) return;
  state.progressPausedAtMs = null;
  renderProgressTime(ctx);
}

export function nextPaint() {
  return new Promise((resolve) => {
    requestAnimationFrame(() => resolve());
  });
}

export async function withProgress(ctx, label, fn) {
  setProgress(ctx, true, 10, `${label}...`);
  startProgressHeartbeat(ctx);
  await nextPaint();
  try {
    const result = await fn((percent, text) => setProgress(ctx, true, percent, text || `${label}...`));
    setProgress(ctx, true, 100, `${label} done`);
    scheduleProgressIdle(ctx, 350);
    return result;
  } catch (error) {
    stopProgressHeartbeat(ctx);
    setProgress(ctx, true, 100, `${label} failed`, { error: true, dismissable: true });
    throw error;
  }
}

export function toggleAnalysisPause(ctx) {
  const { state, el } = ctx;
  const paused = !state.analysisPaused;
  state.analysisPaused = paused;
  updateAnalysisPauseButtonAppearance(el, paused);
  if (paused) {
    // Only tracks not yet picked up are held back -- anything already in
    // flight keeps running, so the timer keeps counting until that settles.
    // handleJobEvent freezes it once analyzingTrackIds empties out. If
    // nothing is in flight right now, it's already effectively stopped, so
    // reflect that immediately.
    if (state.analyzingTrackIds.size === 0 && analysisOwnsProgressBar(state)) {
      pauseProgressHeartbeat(ctx);
    }
  } else if (analysisOwnsProgressBar(state)) {
    resumeProgressHeartbeat(ctx);
  }
  ctx.command("set_analysis_paused", { paused }).catch((err) => {
    console.error("[analysis-ui] set_analysis_paused failed:", err);
  });
}

export function cancelAnalysis(ctx) {
  const { state, el } = ctx;
  el.progressPauseBtn.hidden = true;
  el.progressCancelAnalysisBtn.hidden = true;
  if (state.analysisPaused) {
    state.analysisPaused = false;
    if (analysisOwnsProgressBar(state)) resumeProgressHeartbeat(ctx);
  }
  ctx.command("cancel_analysis").catch((err) => {
    console.error("[analysis-ui] cancel_analysis failed:", err);
  });
}

export function updateAnalysisPauseButtonAppearance(el, paused) {
  const label = paused ? "Resume analysis" : "Pause analysis";
  el.progressPauseBtn.setAttribute("aria-pressed", paused ? "true" : "false");
  el.progressPauseBtn.setAttribute("aria-label", label);
  el.progressPauseBtn.setAttribute("title", label);
  el.progressPauseBtn.textContent = paused ? "\u25B6" : "\u275A\u275A";
}

// The pause/cancel buttons follow the analysis job, not the bar: they show
// while an analysis runs even when an export started first owns the bar. The
// bar's clock only pauses with the analysis while the analysis owns it.
function analysisOwnsProgressBar(state) {
  return state.activeJobType === "analysis";
}

function setAnalysisControlsVisible(state, el, visible) {
  el.progressPauseBtn.hidden = !visible;
  el.progressCancelAnalysisBtn.hidden = !visible;
  state.analysisPaused = false;
  updateAnalysisPauseButtonAppearance(el, false);
}

// Backend-owned: every `job:event` carries a non-empty `message`
// (run_usb_job_with_progress substitutes the job's started-message for a blank
// progress update), so there's nothing to fall back to here.
export function formatJobStatusText(_jobType, _stage, message) {
  return String(message || "");
}

export function handleJobEvent(ctx, payload) {
  const {
    state,
    el,
    debugFrontendLog,
    emitMessage,
    applyRealtimeAnalyzedTrackUpdate,
    refreshSourceRootAnalysisStatus,
    applyLibraryDurationSummary,
    setTrackAnalyzingState,
    setUsbRootControlsLocked,
  } = ctx;
  if (!payload || typeof payload !== "object") return;

  const eventName = String(payload.event || "");
  const jobId = payload.jobId ? String(payload.jobId) : null;
  const jobType = String(payload.jobType || "job");
  const stage = String(payload.stage || "");
  const message = String(payload.message || "");
  const percent = Number(payload.percent ?? 0);
  const statusText = formatJobStatusText(jobType, stage, message);
  const status = statusText ? { text: statusText } : null;

  if (eventName === "job.progress" && stage === "analyze_new_tracks" && payload.trackId) {
    debugFrontendLog("progress", {
      trackId: String(payload.trackId),
      trackTitle: payload.trackTitle ? String(payload.trackTitle) : null,
      current: Number(payload.current || 0),
      total: Number(payload.total || 0),
      bpm: payload.bpm ?? null,
      key: payload.key ?? null,
      hasWaveformPreview: Array.isArray(payload.waveformPreview) && payload.waveformPreview.length > 0,
      hasWaveformPath: typeof payload.waveformPeaksPath === "string" && payload.waveformPeaksPath.trim().length > 0,
      hasArtworkPath: typeof payload.artworkPath === "string" && payload.artworkPath.trim().length > 0,
      failed: payload.failed === true,
      errorMessage: payload.errorMessage ? String(payload.errorMessage) : null
    });
    if (payload.failed === true) {
      const trackLabel = payload.trackTitle
        ? `${String(payload.trackTitle)} (${String(payload.trackId)})`
        : String(payload.trackId);
      const details = typeof payload.filePath === "string" && payload.filePath.trim()
        ? payload.filePath.trim()
        : null;
      emitMessage({
        level: "error",
        source: "analysis",
        code: "analyze.track_failed",
        eventLog: {
          text: `Track analysis failed: ${trackLabel} - ${payload.errorMessage || "unknown analysis error"}`,
          details,
          coalesceKey: `analysis.track_failed.${String(payload.trackId)}`
        }
      });
    }
    applyRealtimeAnalyzedTrackUpdate(payload).catch((err) => {
      console.error("[analysis-ui] realtime update failed:", err);
    });
    // Mark this specific track as analyzing only once the backend actually
    // starts producing pieces for it, and clear it once its own pieces are
    // all done -- this reflects the real, capped worker count (only as many
    // rows show "analyzing" at once as there are active workers) rather than
    // marking the whole submitted batch as analyzing for the entire call.
    setTrackAnalyzingState(String(payload.trackId), payload.trackReady !== true);
    if (payload.trackReady === true && typeof payload.libraryTotalDurationMs === "number") {
      applyLibraryDurationSummary(payload.libraryTotalDurationMs, payload.libraryDurationUnknownCount);
    }
    // A pause click only stops workers from picking up a *new* track --
    // whichever track(s) were already in flight keep going, so the elapsed
    // timer should keep counting during that window instead of jumping to
    // "(paused)" immediately. Only once every in-flight track has actually
    // finished (no rows left in the "analyzing" state) has the batch really
    // stopped. Conversely, a track still turning up while frozen means the
    // freeze was premature (see unfreezeProgressHeartbeat).
    if (state.analysisPaused && state.analyzingTrackIds && analysisOwnsProgressBar(state)) {
      if (state.analyzingTrackIds.size === 0) {
        pauseProgressHeartbeat(ctx);
      } else {
        unfreezeProgressHeartbeat(ctx);
      }
    }
  }

  if (eventName === "job.started" && jobId) {
    state.runningJobs.set(jobId, { jobType, startedAtMs: Date.now(), percent, message });
    if (USB_ROOT_LOCKING_JOB_TYPES.has(jobType)) {
      setUsbRootControlsLocked(true);
    }
    if (jobType === "analysis") {
      setAnalysisControlsVisible(state, el, true);
    }
    // A job started while another runs (an analysis or an update during an
    // export) only reports in the status line; the bar stays with the
    // running job until it ends.
    if (!state.activeJobId) {
      showJobOnProgressBar(ctx, jobId);
    }
    if (status) {
      emitMessage({
        level: "info",
        source: "job",
        code: `${jobType}.started`,
        status
      });
    }
    return;
  }

  const ownsProgressBar = !jobId || jobId === state.activeJobId;

  if (eventName === "job.progress") {
    const isPartialAnalysisPiece = stage === "analyze_new_tracks"
      && payload.trackId
      && payload.trackReady === false;
    if (isPartialAnalysisPiece) {
      return;
    }
    const job = jobId ? state.runningJobs.get(jobId) : null;
    if (job) {
      job.percent = percent;
      job.message = message;
    }
    if (ownsProgressBar) {
      setProgress(ctx, true, percent, message || "Working...");
    }
    if (status) {
      emitMessage({
        level: "info",
        source: "job",
        code: `${jobType}.progress`,
        status
      });
    }
    return;
  }

  if (eventName === "job.completed") {
    if (ownsProgressBar) {
      setProgress(ctx, true, 100, message || "Done");
    }
    if (status) {
      emitMessage({
        level: "info",
        source: "job",
        code: `${jobType}.completed`,
        status
      });
    }
    Promise.resolve(refreshSourceRootAnalysisStatus()).catch(() => {});
    finishJob(ctx, jobId, jobType, ownsProgressBar, 350);
    return;
  }

  if (eventName === "job.failed") {
    if (ownsProgressBar) {
      setProgress(ctx, true, 100, message || "Failed");
    }
    // No Event Log entry: every job fails through a failed command, and the
    // backend logs each failed command's reason once (ApiResponse::failure).
    if (status) {
      emitMessage({
        level: "error",
        source: "job",
        code: `${jobType}.failed`,
        status
      });
    }
    finishJob(ctx, jobId, jobType, ownsProgressBar, 500);
  }
}

// Hand the footer bar to a running job: its own text, percent and elapsed
// time, counted from when it started rather than when it got the bar.
function showJobOnProgressBar(ctx, jobId) {
  const { state } = ctx;
  const job = state.runningJobs.get(jobId);
  state.activeJobId = jobId;
  state.activeJobType = job.jobType;
  setProgress(ctx, true, job.percent, job.message || "Working...");
  stopProgressHeartbeat(ctx);
  startProgressHeartbeat(ctx);
  state.progressStartedAtMs = job.startedAtMs;
  // An analysis paused while the export held the bar arrives paused.
  if (analysisOwnsProgressBar(state) && state.analysisPaused && state.analyzingTrackIds?.size === 0) {
    pauseProgressHeartbeat(ctx);
  }
  renderProgressTime(ctx);
}

function finishJob(ctx, jobId, jobType, ownsProgressBar, idleDelayMs) {
  const { state, el, setUsbRootControlsLocked } = ctx;
  if (jobId) state.runningJobs.delete(jobId);
  if (USB_ROOT_LOCKING_JOB_TYPES.has(jobType) && !isUsbRootChangeBlocked(state)) {
    setUsbRootControlsLocked(false);
    const waiters = state.usbJobIdleWaiters.splice(0);
    waiters.forEach((resolve) => resolve());
  }
  if (jobType === "analysis") {
    setAnalysisControlsVisible(state, el, false);
  }
  if (!ownsProgressBar) return;
  state.activeJobId = null;
  state.activeJobType = null;
  const [nextJobId] = state.runningJobs.keys();
  if (nextJobId) {
    showJobOnProgressBar(ctx, nextJobId);
  } else {
    scheduleProgressIdle(ctx, idleDelayMs);
  }
}

// Resolves once no USB job holds the drive: at once when none is running,
// otherwise when the running one completes or fails.
export function waitForUsbJobIdle(ctx) {
  const { state } = ctx;
  if (!isUsbRootChangeBlocked(state)) return Promise.resolve();
  return new Promise((resolve) => state.usbJobIdleWaiters.push(resolve)).then(() => waitForUsbJobIdle(ctx));
}
