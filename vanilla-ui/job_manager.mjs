// Job progress bar management: set/dismiss/heartbeat/withProgress.

export function setProgress(state, el, active, percent = 0, text = "", opts = {}) {
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
  el.progressText.textContent = state.progressBaseText;
}

export function dismissProgress(state, el) {
  setProgress(state, el, false, 0, "Idle");
}

export function startProgressHeartbeat(state, el) {
  if (state.progressHeartbeatTimer) return;
  state.progressStartedAtMs = Date.now();
  state.lastJobEventAtMs = Date.now();
  state.progressHeartbeatTimer = window.setInterval(() => {
    if (!el.progressFooter.classList.contains("active")) return;
    const now = Date.now();
    const totalSecs = Math.max(0, Math.floor((now - state.progressStartedAtMs) / 1000));
    const idleSecs = Math.max(0, Math.floor((now - state.lastJobEventAtMs) / 1000));
    const suffix = ` (${totalSecs}s)`;
    el.progressText.textContent = `${state.progressBaseText}${suffix}`;
  }, 1000);
}

export function stopProgressHeartbeat(state) {
  if (!state.progressHeartbeatTimer) return;
  window.clearInterval(state.progressHeartbeatTimer);
  state.progressHeartbeatTimer = null;
}

export function nextPaint() {
  return new Promise((resolve) => {
    requestAnimationFrame(() => resolve());
  });
}

export async function withProgress(state, el, label, fn) {
  setProgress(state, el, true, 10, `${label}...`);
  startProgressHeartbeat(state, el);
  await nextPaint();
  try {
    const result = await fn((percent, text) => setProgress(state, el, true, percent, text || `${label}...`));
    setProgress(state, el, true, 100, `${label} done`);
    setTimeout(() => {
      setProgress(state, el, false, 0, "Idle");
      stopProgressHeartbeat(state);
    }, 350);
    return result;
  } catch (error) {
    stopProgressHeartbeat(state);
    setProgress(state, el, true, 100, `${label} failed`, { error: true, dismissable: true });
    throw error;
  }
}

const JOB_STAGE_STATUS_RULES = {
  "usb_read:fetch_usb_playlists": ({ message }) => message || "Importing USB playlists...",
  "usb_read:fetch_usb_histories": ({ message }) => message || "Importing USB histories...",
};

export function formatJobStatusText(jobType, stage, message) {
  const normalizedJobType = String(jobType || "job");
  const normalizedStage = String(stage || "");
  const normalizedMessage = String(message || "");
  if (normalizedMessage) {
    return normalizedMessage;
  }
  const ruleKey = normalizedStage ? `${normalizedJobType}:${normalizedStage}` : "";
  const formatter = JOB_STAGE_STATUS_RULES[ruleKey];
  if (typeof formatter === "function") {
    return formatter({
      jobType: normalizedJobType,
      stage: normalizedStage,
      message: normalizedMessage
    });
  }
  return "";
}

function createEmitMessage(deps = {}) {
  if (typeof deps.emitMessage === "function") {
    return deps.emitMessage;
  }
  const setStatus = typeof deps.setStatus === "function" ? deps.setStatus : () => {};
  const pushEventLog = typeof deps.pushEventLog === "function" ? deps.pushEventLog : () => {};
  return (msg = {}) => {
    if (msg.status?.text) {
      setStatus(msg.status.text);
    }
    if (msg.eventLog?.text) {
      pushEventLog({
        level: msg.level,
        source: msg.source,
        code: msg.code,
        message: msg.eventLog.text,
        details: msg.eventLog.details,
        coalesceKey: msg.eventLog.coalesceKey,
        ts: msg.ts
      });
    }
  };
}

export function handleJobEvent(state, el, payload, deps = {}) {
  const {
    debugFrontendLog,
    applyRealtimeAnalyzedTrackUpdate,
  } = deps;
  const emitMessage = createEmitMessage(deps);
  const trackInfo = payload?.trackTitle || payload?.trackId;
  console.log(
    "[job-event]",
    payload?.event,
    payload?.stage,
    ...(trackInfo ? ["track:", trackInfo] : [payload?.message || ""])
  );
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
  }

  if (eventName === "job.started" && jobId) {
    state.activeJobId = jobId;
    state.lastJobEventAtMs = Date.now();
    setProgress(state, el, true, percent, message || "Working...");
    startProgressHeartbeat(state, el);
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

  if (jobId && state.activeJobId && jobId !== state.activeJobId) {
    return;
  }

  if (eventName === "job.progress") {
    const isPartialAnalysisPiece = stage === "analyze_new_tracks"
      && payload.trackId
      && payload.trackReady === false;
    if (isPartialAnalysisPiece) {
      return;
    }
    state.lastJobEventAtMs = Date.now();
    setProgress(state, el, true, percent, message || "Working...");
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
    state.lastJobEventAtMs = Date.now();
    setProgress(state, el, true, 100, message || "Done");
    if (status) {
      emitMessage({
        level: "info",
        source: "job",
        code: `${jobType}.completed`,
        status
      });
    }
    state.activeJobId = null;
    setTimeout(() => {
      setProgress(state, el, false, 0, "Idle");
      stopProgressHeartbeat(state);
    }, 350);
    return;
  }

  if (eventName === "job.failed") {
    state.lastJobEventAtMs = Date.now();
    setProgress(state, el, true, 100, message || "Failed");
    emitMessage({
      level: "error",
      source: "job",
      code: `${jobType}.failed`,
      ...(status ? { status } : {}),
      eventLog: {
        text: statusText || message || "Job failed",
        coalesceKey: `${jobType}.failed.${stage || "unknown"}`
      }
    });
    state.activeJobId = null;
    setTimeout(() => {
      setProgress(state, el, false, 0, "Idle");
      stopProgressHeartbeat(state);
    }, 500);
  }
}
