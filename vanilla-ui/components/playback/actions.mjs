import { transportIcon } from "../../track_table.mjs";
import { isTransportButtonPlaying } from "../../playback_ui_state.mjs";

export function toPlayableUrl(ctx, path) {
  if (!path) return null;
  const raw = String(path).trim();
  if (!raw) return null;
  if (/^https?:\/\//i.test(raw) || /^blob:/i.test(raw) || /^data:/i.test(raw)) return raw;
  if (/^file:\/\//i.test(raw)) return raw;

  if (ctx.isTauriRuntime?.() && typeof ctx.tauriConvertFileSrc === "function") {
    try {
      const converted = ctx.tauriConvertFileSrc(raw);
      if (converted) return converted;
    } catch (_) {}
  }
  const windowObj = ctx.window || globalThis;
  if (windowObj?.__TAURI__?.core?.convertFileSrc) {
    try {
      const converted = windowObj.__TAURI__.core.convertFileSrc(raw);
      if (converted) return converted;
    } catch (_) {}
  }

  const normalized = raw.replace(/\\/g, "/");
  if (/^[a-zA-Z]:\//.test(normalized)) {
    return `file:///${encodeURI(normalized)}`;
  }
  if (normalized.startsWith("/")) {
    return `file://${encodeURI(normalized)}`;
  }
  return null;
}
// Playback UI helpers that coordinate DOM state with playback state.

export function updateTransportButtonsInDom(ctx, root = ctx.document) {
  const { state } = ctx;
  root.querySelectorAll(".transport-btn").forEach((btn) => {
    const isPlaying = isTransportButtonPlaying(state, { rowKey: btn.dataset.rowKey, trackId: btn.dataset.id });
    btn.classList.toggle("is-playing", isPlaying);
    btn.setAttribute("aria-label", isPlaying ? "Stop" : "Play");
    btn.dataset.tooltip = isPlaying ? "Stop" : "Play";
    btn.replaceChildren(transportIcon(btn.ownerDocument, isPlaying));
  });
}

export function setWaveformPlayhead(element, fraction, playing, paused = false) {
  if (!element) return;
  const clamped = Math.max(0, Math.min(1, Number(fraction) || 0));
  // Drive the playhead with a compositor-only `transform: translateX` (see
  // styles.css) instead of animating `left`. Animating `left` forces a
  // layout + repaint of the waveform region on every frame of the playback
  // rAF loop, which is brutal when WebKitGTK is software-compositing (the
  // AppImage path, where WEBKIT_DISABLE_DMABUF_RENDERER is forced on).
  // `--playhead-position` (a percentage) is still written so scrub math and
  // any width-less context keeps a usable value.
  const width = element.clientWidth || 0;
  element.style.setProperty("--playhead-x", `${clamped * width}px`);
  element.style.setProperty("--playhead-position", `${clamped * 100}%`);
  element.classList.toggle("is-playing", !!playing);
  element.classList.toggle("is-paused", !playing && !!paused);
}

export function clearAllWaveformPlayheads(ctx) {
  ctx.document.querySelectorAll(".waveform").forEach((wf) => {
    setWaveformPlayhead(wf, 0, false);
  });
}

export function scrubRatioFromPointer(event, waveformElement) {
  if (!waveformElement) return 0;
  const rect = waveformElement.getBoundingClientRect();
  if (!rect.width) return 0;
  const x = event.clientX - rect.left;
  return Math.max(0, Math.min(1, x / rect.width));
}

// Drives the waveform playhead by wall-clock interpolation from a single known
// position/duration snapshot, instead of depending on a stream of backend push events.
export function startPlayheadInterpolation(ctx, {
  waveformEl,
  initialPositionMs,
  durationMs,
  nowFn = () => Date.now()
}) {
  const { state, requestAnimationFrameFn } = ctx;
  stopPlayheadInterpolation(ctx);
  if (!waveformEl || !(durationMs > 0) || typeof requestAnimationFrameFn !== "function") return;

  const startWallClockMs = nowFn();
  const tick = () => {
    if (state.activeWaveform !== waveformEl) return;
    const elapsedMs = nowFn() - startWallClockMs;
    const positionMs = Math.min(durationMs, initialPositionMs + elapsedMs);
    setWaveformPlayhead(waveformEl, positionMs / durationMs, true);
    state.playheadAnimationHandle = requestAnimationFrameFn(tick);
  };
  tick();
}

export function stopPlayheadInterpolation(ctx) {
  const { state, cancelAnimationFrameFn } = ctx;
  if (state.playheadAnimationHandle != null && typeof cancelAnimationFrameFn === "function") {
    cancelAnimationFrameFn(state.playheadAnimationHandle);
  }
  state.playheadAnimationHandle = null;
}

export function beginPlaybackIntent(state, kind, target = {}) {
  state.playbackGeneration = (state.playbackGeneration || 0) + 1;
  state.playbackPendingKind = kind;
  state.playbackPendingRowKey = kind === "play" ? (target.rowKey || null) : null;
  state.playbackPendingTrackId = kind === "play" ? (target.trackId || null) : null;
  return state.playbackGeneration;
}

export function isGenerationCurrent(state, generation) {
  return generation === undefined || state.playbackGeneration === generation;
}

export function clearPlaybackIntentIfCurrent(state, generation) {
  if (!isGenerationCurrent(state, generation)) return;
  state.playbackPendingKind = null;
  state.playbackPendingRowKey = null;
  state.playbackPendingTrackId = null;
}

export function withBackendQueue(state, jobFn) {
  const prior = state.playbackBackendQueue || Promise.resolve();
  const run = prior.catch(() => {}).then(jobFn);
  state.playbackBackendQueue = run.catch(() => {});
  return run;
}

// Drop every trace of the current playback from state + the DOM.
function resetPlaybackState(ctx) {
  const { state } = ctx;
  state.playbackActive = false;
  state.playbackPaused = false;
  state.playbackTrackId = null;
  state.playbackPath = null;
  state.playbackRowKey = null;
  state.activeWaveform = null;
  state.playbackLabelContext = null;
  stopPlayheadInterpolation(ctx);
  clearAllWaveformPlayheads(ctx);
}

/// Apply a backend pause/resume status (a `pause_playback_native` /
/// `resume_playback_native` response or a `playback.paused` / `playback.resumed`
/// event). The backend owns the paused flag and the position; this only
/// projects them: paused freezes the playhead at `positionMs`, resumed restarts
/// the interpolation from there.
export function applyPauseStatus(ctx, status) {
  const { state, emitStatus } = ctx;
  if (!status || !(status.playing || status.paused)) return;
  state.playbackPaused = !!status.paused;
  const duration = Number(status.durationMs || 0);
  const position = Number(status.positionMs || 0);
  const waveformEl = state.activeWaveform;
  if (status.paused) {
    stopPlayheadInterpolation(ctx);
    if (waveformEl) {
      setWaveformPlayhead(waveformEl, duration > 0 ? position / duration : 0, false, true);
    }
    emitStatus("Paused");
  } else {
    if (waveformEl && duration > 0) {
      startPlayheadInterpolation(ctx, {
        waveformEl,
        initialPositionMs: position,
        durationMs: duration,
      });
    }
    if (state.playbackLabelContext) {
      const { sourceLabel, title } = state.playbackLabelContext;
      emitStatus(`Playing from ${sourceLabel}: ${title}`);
    }
  }
  updateTransportButtonsInDom(ctx);
}

async function runPauseChange(ctx, commandName, shouldRun) {
  const { state } = ctx;
  if (!state.playbackActive || !shouldRun()) return;
  // A play/stop issued while this was queued wins; don't apply a stale status.
  const generation = state.playbackGeneration;
  return withBackendQueue(state, async () => {
    const status = await ctx.command(commandName);
    if (!isGenerationCurrent(state, generation)) return;
    applyPauseStatus(ctx, status);
  });
}

export function pausePlaybackFromUi(ctx) {
  return runPauseChange(ctx, "pause_playback_native", () => !ctx.state.playbackPaused);
}

export function resumePlaybackFromUi(ctx) {
  return runPauseChange(ctx, "resume_playback_native", () => !!ctx.state.playbackPaused);
}

/// Hand the running playback's playhead over to another waveform (the cue
/// editor adopting a track already playing from a list row, and back on
/// close). Position and paused flag come from the backend's status; this only
/// re-targets the projection. Resolves to the waveform it replaced, or
/// `undefined` when nothing was moved (no playback, or a play/stop won).
export function moveActiveWaveform(ctx, waveformEl) {
  const { state } = ctx;
  if (!state.playbackActive) return Promise.resolve(undefined);
  const generation = state.playbackGeneration;
  return withBackendQueue(state, async () => {
    const status = await ctx.command("get_playback_status_native");
    if (!isGenerationCurrent(state, generation) || !state.playbackActive) return undefined;
    if (!status || !(status.playing || status.paused)) return undefined;
    const previous = state.activeWaveform;
    stopPlayheadInterpolation(ctx);
    if (previous && previous !== waveformEl) setWaveformPlayhead(previous, 0, false);
    state.activeWaveform = waveformEl || null;
    const duration = Number(status.durationMs || 0);
    const position = Number(status.positionMs || 0);
    if (waveformEl && status.paused) {
      setWaveformPlayhead(waveformEl, duration > 0 ? position / duration : 0, false, true);
    } else if (waveformEl && duration > 0) {
      startPlayheadInterpolation(ctx, {
        waveformEl,
        initialPositionMs: position,
        durationMs: duration,
      });
    }
    return previous || null;
  });
}

// `fromUi`: a Stop the user pressed -- reports "Idle" even when nothing was
// playing and lets a failed stop surface. Otherwise (a context change such as
// switching views) it's a no-op when idle and a failed stop is only logged.
async function stopPlayback(ctx, { fromUi }) {
  const { state, emitStatus } = ctx;
  if (state.playbackStopPromise) return state.playbackStopPromise;
  if (!state.playbackActive && state.playbackPendingKind !== "play") {
    if (fromUi) emitStatus("Idle");
    return;
  }
  const generation = beginPlaybackIntent(state, "stop");
  updateTransportButtonsInDom(ctx);
  state.playbackStopPromise = withBackendQueue(state, async () => {
    try {
      await ctx.command("stop_playback_native");
    } catch (err) {
      if (fromUi) throw err;
      ctx.warn("Failed to stop playback on context change:", err);
    }
    if (isGenerationCurrent(state, generation)) {
      resetPlaybackState(ctx);
      clearPlaybackIntentIfCurrent(state, generation);
    }
    updateTransportButtonsInDom(ctx);
    emitStatus("Idle");
  });
  try {
    await state.playbackStopPromise;
  } finally {
    state.playbackStopPromise = null;
  }
}

export function stopPlaybackFromUi(ctx) {
  return stopPlayback(ctx, { fromUi: true });
}

export function stopPlaybackIfActive(ctx) {
  return stopPlayback(ctx, { fromUi: false });
}

function toNumberOrNull(value) {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : null;
}

// Track-identity resolution is backend-owned: `resolve_track_identity` sees the
// whole library DB (the frontend only ever holds the loaded pages) and can
// materialize a local row for an on-disk file. No client-side matching here --
// not even as a fast path.
export async function resolveLocalTrackIdAsync(ctx, track) {
  const { state } = ctx;
  if (!track) return null;
  if (track.localTrackId) return track.localTrackId;

  const filePath = String(track.filePath || "").trim();
  try {
    const data = await ctx.command("resolve_track_identity", {
      trackId: track.id || null,
      title: track.title || "",
      artist: track.artist || "",
      album: track.album || null,
      bpm: toNumberOrNull(track.bpm),
      filePath: filePath || null,
      fileSizeBytes: toNumberOrNull(track.fileSizeBytes),
      trackNumber: toNumberOrNull(track.trackNumber),
      key: track.key || null,
      formatExt: track.formatExt || null,
      sampleRateHz: toNumberOrNull(track.sampleRateHz),
      bitDepth: toNumberOrNull(track.bitDepth),
      bitrateKbps: toNumberOrNull(track.bitrateKbps),
      usbRoot: state.usbRoot || null,
      usbRootValid: !!state.usbRootValid,
      usbAnalysisPath: track.usbAnalysisPath || null
    });
    if (data?.trackId) {
      const previousId = String(track.id || "").trim();
      track.localTrackId = data.trackId;
      ctx.promoteTrackIdentity(previousId, data.trackId);
      return data.trackId;
    }
  } catch (_) {
    return null;
  }
  return null;
}

// Synchronous, per-row hot path (rendered for every track in every list). Pure
// id compare against the backend-resolved id the playback events / USB rows
// carry -- no path or metadata scan over `state.tracks` (which is only the
// loaded pages).
export function isTrackCurrentlyPlaying(ctx, track) {
  const { state } = ctx;
  if (state.playbackPendingKind === "stop") return false;
  if (state.playbackPendingKind === "play") {
    return !!(state.playbackPendingTrackId && track?.id && state.playbackPendingTrackId === track.id);
  }
  if (!state.playbackActive) return false;
  const rowId = String(track?.localTrackId || track?.id || "");
  return !!rowId && !!state.playbackTrackId && state.playbackTrackId === rowId;
}

async function playResolvedTrack(ctx, track, origin, options, generation) {
  const { state, emitStatus } = ctx;
  const trackPath = String(track?.filePath || "").trim();
  const originLower = String(origin || "").toLowerCase();
  const artist = String(track?.artist || "").trim();
  const titlePart = track?.title || "Unknown Title";
  const title = artist ? `${artist} - ${titlePart}` : titlePart;
  const startRatio = Math.max(0, Math.min(1, Number(options.startRatio) || 0));
  const rawStartOffsetMs = toNumberOrNull(options.startOffsetMs);
  const startOffsetMs = rawStartOffsetMs === null ? null : Math.max(0, Math.round(rawStartOffsetMs));
  const waveformEl = options.waveformEl || null;

  return withBackendQueue(state, async () => {
    if (!isGenerationCurrent(state, generation)) return;
    try {
      const playback = await ctx.command("play_resolved_track", {
        title: track?.title || "",
        artist: track?.artist || "",
        album: track?.album || null,
        bpm: toNumberOrNull(track?.bpm),
        filePath: trackPath || null,
        fileSizeBytes: toNumberOrNull(track?.fileSizeBytes),
        trackId: track?.id || null,
        origin: originLower,
        usbRoot: state.usbRoot || null,
        usbRootValid: !!state.usbRootValid,
        startOffsetMs,
        startRatio,
        startPaused: !!options.startPaused
      });
      if (!isGenerationCurrent(state, generation)) return;
      if (waveformEl) {
        clearAllWaveformPlayheads(ctx);
        state.activeWaveform = waveformEl;
        const duration = Number(playback?.durationMs || 0);
        const position = Number(playback?.positionMs || 0);
        if (playback?.paused) {
          // A `startPaused` load/seek: parked at the position, no audio.
          stopPlayheadInterpolation(ctx);
          setWaveformPlayhead(waveformEl, duration > 0 ? position / duration : startRatio, false, true);
        } else if (duration > 0) {
          startPlayheadInterpolation(ctx, {
            waveformEl,
            initialPositionMs: position,
            durationMs: duration,
          });
        } else {
          setWaveformPlayhead(waveformEl, startRatio, true);
        }
      }
      // Backend-owned: `play_resolved_track` always returns the resolved
      // `sourceLabel` (see backend playback_source_label / mod.rs). Stash the
      // string so later playback events re-use it verbatim rather than
      // re-deriving a label the frontend can't always reproduce.
      const sourceLabel = playback?.sourceLabel || "";
      state.playbackActive = true;
      state.playbackPaused = !!playback?.paused;
      state.playbackTrackId = playback?.trackId || track?.id || null;
      state.playbackPath = playback?.path || trackPath;
      state.playbackRowKey = options.rowKey || null;
      state.playbackLabelContext = { sourceLabel, title };
      updateTransportButtonsInDom(ctx);
      emitStatus(playback?.paused ? "Paused" : `Playing from ${sourceLabel}: ${title}`);
    } catch (err) {
      if (!isGenerationCurrent(state, generation)) return;
      const message = err?.message || String(err);
      // `play_resolved_track` only raises NOT_FOUND when it can't resolve the
      // track to a playable path in the Library or the selected USB -- an
      // expected, soft outcome, not a failure.
      if (err?.code === "NOT_FOUND") {
        emitStatus("Cannot play: track not found in Library or selected USB.", { level: "warn", source: "playback" });
        return;
      }
      emitStatus(`Playback failed: ${message}`, { level: "error", source: "playback" });
    }
  });
}

export async function playTrackFromOrigin(ctx, track, origin, options = {}) {
  const { state } = ctx;
  const rowKey = options.rowKey || null;
  const trackId = track?.id || null;

  if (
    state.playbackStartPromise
    && state.playbackPendingKind === "play"
    && (state.playbackPendingRowKey || null) === rowKey
    && (state.playbackPendingTrackId || null) === trackId
  ) {
    return state.playbackStartPromise;
  }

  const generation = beginPlaybackIntent(state, "play", { rowKey, trackId });
  updateTransportButtonsInDom(ctx);

  const run = (async () => {
    try {
      return await playResolvedTrack(ctx, track, origin, options, generation);
    } finally {
      clearPlaybackIntentIfCurrent(state, generation);
      updateTransportButtonsInDom(ctx);
    }
  })();
  state.playbackStartPromise = run;
  try {
    return await run;
  } finally {
    if (state.playbackStartPromise === run) {
      state.playbackStartPromise = null;
    }
  }
}

export function handlePlaybackEvent(ctx, payload) {
  const { state, emitStatus } = ctx;
  if (!payload || typeof payload !== "object") return;
  const eventName = String(payload.event || "");
  const path = payload.path ? String(payload.path) : null;
  const playing = !!payload.playing;
  const position = Number(payload.positionMs || 0);
  const duration = Number(payload.durationMs || 0);

  if (eventName === "playback.started" || eventName === "playback.seeked") {
    // These are one-shot confirmations tied directly to our own playback start call
    // (unlike a continuous progress stream). If we have no active path and nothing
    // pending, this can't be a legitimate confirmation of anything we're waiting on —
    // treat a stray playing:true here as noise rather than reviving cleared state.
    const noActiveOrPendingContext = !state.playbackActive && !state.playbackPath && state.playbackPendingKind !== "play";
    if (playing && noActiveOrPendingContext) return;

    const pathChanged = path !== null && path !== state.playbackPath;
    state.playbackActive = playing;
    state.playbackPaused = false;
    state.playbackPath = path;
    // Backend-owned: `playback.started` / `playback.seeked` carry the resolved
    // local track id (omitted when the backend couldn't resolve one -- then we
    // keep whatever `play_resolved_track` already stashed).
    if (payload.trackId !== undefined) {
      state.playbackTrackId = payload.trackId || null;
    }
    if (pathChanged) {
      state.playbackRowKey = null;
    }
    if (state.activeWaveform) {
      if (playing && duration > 0) {
        startPlayheadInterpolation(ctx, {
          waveformEl: state.activeWaveform,
          initialPositionMs: position,
          durationMs: duration,
        });
      } else {
        setWaveformPlayhead(state.activeWaveform, duration > 0 ? position / duration : 0, playing);
      }
    }
    updateTransportButtonsInDom(ctx);
    // Keep the status line a live projection of playback state rather than a
    // one-shot string frozen at play-dispatch time -- reuse the backend-owned
    // label playTrackFromOrigin stashed, verbatim, so later events (e.g. a
    // seek) keep it accurate without re-deriving it here.
    if (playing && state.playbackLabelContext) {
      const { sourceLabel, title } = state.playbackLabelContext;
      emitStatus(`Playing from ${sourceLabel}: ${title}`);
    }
    return;
  }

  if (eventName === "playback.paused" || eventName === "playback.resumed") {
    // Stale if we've already moved on to another track (same guard as stopped).
    if (!state.playbackActive || (path !== null && path !== state.playbackPath)) return;
    applyPauseStatus(ctx, payload);
    return;
  }

  if (eventName === "playback.stopped") {
    // A natural end-of-track notification and a fresh explicit play for a different
    // track travel to us via independent threads with no ordering guarantee — if this
    // "stopped" is for a path we've already moved on from, it's stale; don't let it
    // blank out whatever is now actually playing.
    if (path !== null && path !== state.playbackPath) return;
    resetPlaybackState(ctx);
    updateTransportButtonsInDom(ctx);
    emitStatus("Idle");
    return;
  }

  if (eventName === "playback.error") {
    const message = payload.message ? String(payload.message) : "Playback failed";
    emitStatus(message);
  }
}

export async function unregisterBackendJobEvents(ctx) {
  const { state } = ctx;
  const unlistenFns = [state.unlistenJobEvent, state.unlistenPlaybackEvent, state.unlistenBackendLogEvent]
    .filter((fn) => typeof fn === "function");
  state.unlistenJobEvent = null;
  state.unlistenPlaybackEvent = null;
  state.unlistenBackendLogEvent = null;
  for (const fn of unlistenFns) {
    try {
      await Promise.resolve(fn());
    } catch (err) {
      ctx.warn("Failed to unlisten backend event:", err);
    }
  }
}

export async function registerBackendJobEvents(ctx) {
  const { state } = ctx;
  if (!ctx.isTauriRuntime()) return;
  await unregisterBackendJobEvents(ctx);
  const listen = await ctx.getTauriEventListen();
  if (!listen) return;

  const unlisten = await listen("job:event", (event) => {
    ctx.handleJobEvent(event?.payload);
  });
  if (typeof unlisten === "function") {
    state.unlistenJobEvent = unlisten;
  }

  const unlistenPlayback = await listen("playback:event", (event) => {
    handlePlaybackEvent(ctx, event?.payload);
  });
  if (typeof unlistenPlayback === "function") {
    state.unlistenPlaybackEvent = unlistenPlayback;
  }

  const unlistenBackendLog = await listen("backend:log", (event) => {
    ctx.handleBackendLogEvent(event?.payload);
  });
  if (typeof unlistenBackendLog === "function") {
    state.unlistenBackendLogEvent = unlistenBackendLog;
  }
}

export function bindBeforeUnloadCleanup(ctx) {
  ctx.window.addEventListener("beforeunload", () => {
    unregisterBackendJobEvents(ctx).catch(() => {});
  });
}
