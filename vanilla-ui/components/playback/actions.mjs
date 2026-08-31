export function toPlayableUrl(path, deps = {}) {
  const {
    isTauriRuntime = () => false,
    tauriConvertFileSrc = null,
    windowObj = typeof window !== "undefined" ? window : globalThis
  } = deps;

  if (!path) return null;
  const raw = String(path).trim();
  if (!raw) return null;
  if (/^https?:\/\//i.test(raw) || /^blob:/i.test(raw) || /^data:/i.test(raw)) return raw;
  if (/^file:\/\//i.test(raw)) return raw;

  if (isTauriRuntime() && typeof tauriConvertFileSrc === "function") {
    try {
      const converted = tauriConvertFileSrc(raw);
      if (converted) return converted;
    } catch (_) {}
  }
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

export function getPlaybackUiStateHelpers() {
  return globalThis?.playbackUiState || null;
}

export function updateTransportButtonsInDom(state, root) {
  const helpers = getPlaybackUiStateHelpers();
  root.querySelectorAll(".transport-btn").forEach((btn) => {
    const id = btn.dataset.id || "";
    const rowKey = btn.dataset.rowKey || "";
    const isPlaying = helpers?.isTransportButtonPlaying
      ? helpers.isTransportButtonPlaying(state, { rowKey, trackId: id })
      : (
        !!(state.playbackActive && state.playbackRowKey && rowKey && state.playbackRowKey === rowKey)
        || !!(state.playbackActive && state.playbackTrackId && id === state.playbackTrackId)
      );
    btn.classList.toggle("is-playing", isPlaying);
    btn.setAttribute("aria-label", isPlaying ? "Stop" : "Play");
    btn.dataset.tooltip = isPlaying ? "Stop" : "Play";
    btn.innerHTML = isPlaying
      ? `<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="7" y="7" width="10" height="10" rx="1"></rect></svg>`
      : `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 6v12l10-6z"></path></svg>`;
  });
}

export function setWaveformPlayhead(element, fraction, playing) {
  if (!element) return;
  const clamped = Math.max(0, Math.min(1, Number(fraction) || 0));
  element.style.setProperty("--playhead-position", `${clamped * 100}%`);
  element.classList.toggle("is-playing", !!playing);
}

export function clearAllWaveformPlayheads(document) {
  document.querySelectorAll(".waveform").forEach((wf) => {
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
export function startPlayheadInterpolation(state, {
  waveformEl,
  initialPositionMs,
  durationMs,
  setWaveformPlayhead: setWaveformPlayheadFn,
  requestAnimationFrameFn,
  cancelAnimationFrameFn,
  nowFn = () => Date.now()
}) {
  stopPlayheadInterpolation(state, { cancelAnimationFrameFn });
  if (!waveformEl || !(durationMs > 0) || typeof requestAnimationFrameFn !== "function") return;

  const startWallClockMs = nowFn();
  const tick = () => {
    if (state.activeWaveform !== waveformEl) return;
    const elapsedMs = nowFn() - startWallClockMs;
    const positionMs = Math.min(durationMs, initialPositionMs + elapsedMs);
    setWaveformPlayheadFn(waveformEl, positionMs / durationMs, true);
    state.playheadAnimationHandle = requestAnimationFrameFn(tick);
  };
  tick();
}

export function stopPlayheadInterpolation(state, { cancelAnimationFrameFn } = {}) {
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

export async function stopPlaybackFromUi(state, deps) {
  const {
    command,
    clearAllWaveformPlayheads,
    updateTransportButtonsInDom,
    setStatus,
    cancelAnimationFrameFn
  } = deps;
  if (state.playbackStopPromise) return state.playbackStopPromise;
  if (!state.playbackActive && state.playbackPendingKind !== "play") {
    setStatus("Idle");
    return;
  }
  const generation = beginPlaybackIntent(state, "stop");
  updateTransportButtonsInDom();
  state.playbackStopPromise = withBackendQueue(state, async () => {
    await command("stop_playback_native");
    if (isGenerationCurrent(state, generation)) {
      state.playbackActive = false;
      state.playbackTrackId = null;
      state.playbackPath = null;
      state.playbackRowKey = null;
      state.activeWaveform = null;
      state.playbackLabelContext = null;
      stopPlayheadInterpolation(state, { cancelAnimationFrameFn });
      clearAllWaveformPlayheads();
      clearPlaybackIntentIfCurrent(state, generation);
    }
    updateTransportButtonsInDom();
    setStatus("Idle");
  });
  try {
    await state.playbackStopPromise;
  } finally {
    state.playbackStopPromise = null;
  }
}
function toNumberOrNull(value) {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : null;
}

// Track-identity resolution is backend-owned: `resolve_track_identity` sees the
// whole library DB (the frontend only ever holds the loaded pages) and can
// materialize a local row for an on-disk file. No client-side matching here --
// not even as a fast path.
export async function resolveLocalTrackIdAsync(track, state, deps) {
  const {
    command,
    promoteTrackIdentity
  } = deps;

  if (!track) return null;
  if (track.localTrackId) return track.localTrackId;

  const filePath = String(track.filePath || "").trim();
  try {
    const data = await command("resolve_track_identity", {
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
      if (typeof promoteTrackIdentity === "function") {
        promoteTrackIdentity(previousId, data.trackId);
      }
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
export function isTrackCurrentlyPlaying(track, state) {
  if (state.playbackPendingKind === "stop") return false;
  if (state.playbackPendingKind === "play") {
    return !!(state.playbackPendingTrackId && track?.id && state.playbackPendingTrackId === track.id);
  }
  if (!state.playbackActive) return false;
  const rowId = String(track?.localTrackId || track?.id || "");
  return !!rowId && !!state.playbackTrackId && state.playbackTrackId === rowId;
}
export async function playTrackFromOrigin(state, track, origin, options = {}, deps) {
  const {
    command,
    clearAllWaveformPlayheads,
    setWaveformPlayhead,
    updateTransportButtonsInDom,
    setStatus,
    generation,
    requestAnimationFrameFn,
    cancelAnimationFrameFn
  } = deps;

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
      const playback = await command("play_resolved_track", {
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
        startRatio
      });
      if (!isGenerationCurrent(state, generation)) return;
      if (waveformEl) {
        clearAllWaveformPlayheads();
        state.activeWaveform = waveformEl;
        const duration = Number(playback?.durationMs || 0);
        const position = Number(playback?.positionMs || 0);
        if (duration > 0) {
          startPlayheadInterpolation(state, {
            waveformEl,
            initialPositionMs: position,
            durationMs: duration,
            setWaveformPlayhead,
            requestAnimationFrameFn,
            cancelAnimationFrameFn
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
      state.playbackTrackId = playback?.trackId || track?.id || null;
      state.playbackPath = playback?.path || trackPath;
      state.playbackRowKey = options.rowKey || null;
      state.playbackLabelContext = { sourceLabel, title };
      updateTransportButtonsInDom();
      setStatus(`Playing from ${sourceLabel}: ${title}`);
    } catch (err) {
      if (!isGenerationCurrent(state, generation)) return;
      const message = err?.message || String(err);
      if (/track not found in Library or selected USB/i.test(message)) {
        setStatus("Cannot play: track not found in Library or selected USB.", { level: "warn", source: "playback" });
        return;
      }
      setStatus(`Playback failed: ${message}`, { level: "error", source: "playback" });
    }
  });
}
export async function stopPlaybackIfActive(state, deps) {
  const {
    command,
    clearAllWaveformPlayheads,
    updateTransportButtonsInDom,
    setStatus,
    warn,
    cancelAnimationFrameFn
  } = deps;
  if (state.playbackStopPromise) return state.playbackStopPromise;
  if (!state.playbackActive && state.playbackPendingKind !== "play") return;
  const generation = beginPlaybackIntent(state, "stop");
  updateTransportButtonsInDom();
  state.playbackStopPromise = withBackendQueue(state, async () => {
    try {
      await command("stop_playback_native");
    } catch (err) {
      warn("Failed to stop playback on context change:", err);
    }
    if (isGenerationCurrent(state, generation)) {
      state.playbackActive = false;
      state.playbackTrackId = null;
      state.playbackPath = null;
      state.playbackRowKey = null;
      state.activeWaveform = null;
      state.playbackLabelContext = null;
      stopPlayheadInterpolation(state, { cancelAnimationFrameFn });
      clearAllWaveformPlayheads();
      clearPlaybackIntentIfCurrent(state, generation);
    }
    updateTransportButtonsInDom();
    setStatus("Idle");
  });
  try {
    await state.playbackStopPromise;
  } finally {
    state.playbackStopPromise = null;
  }
}

export async function playTrackFromOriginController(state, track, origin, options = {}, deps) {
  const { playTrackFromOriginCore, updateTransportButtonsInDom } = deps;
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
  updateTransportButtonsInDom?.();

  const run = (async () => {
    try {
      return await playTrackFromOriginCore(state, track, origin, options, { ...deps, generation });
    } finally {
      clearPlaybackIntentIfCurrent(state, generation);
      updateTransportButtonsInDom?.();
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
export function handlePlaybackEvent(state, payload, deps) {
  const {
    setWaveformPlayhead,
    updateTransportButtonsInDom,
    clearAllWaveformPlayheads,
    setStatus,
    requestAnimationFrameFn,
    cancelAnimationFrameFn
  } = deps;

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
        startPlayheadInterpolation(state, {
          waveformEl: state.activeWaveform,
          initialPositionMs: position,
          durationMs: duration,
          setWaveformPlayhead,
          requestAnimationFrameFn,
          cancelAnimationFrameFn
        });
      } else {
        setWaveformPlayhead(state.activeWaveform, duration > 0 ? position / duration : 0, playing);
      }
    }
    updateTransportButtonsInDom();
    // Keep the status line a live projection of playback state rather than a
    // one-shot string frozen at play-dispatch time -- reuse the backend-owned
    // label playTrackFromOrigin stashed, verbatim, so later events (e.g. a
    // seek) keep it accurate without re-deriving it here.
    if (playing && state.playbackLabelContext) {
      const { sourceLabel, title } = state.playbackLabelContext;
      setStatus(`Playing from ${sourceLabel}: ${title}`);
    }
    return;
  }

  if (eventName === "playback.stopped") {
    // A natural end-of-track notification and a fresh explicit play for a different
    // track travel to us via independent threads with no ordering guarantee — if this
    // "stopped" is for a path we've already moved on from, it's stale; don't let it
    // blank out whatever is now actually playing.
    if (path !== null && path !== state.playbackPath) return;
    state.playbackActive = false;
    state.playbackPath = null;
    state.playbackTrackId = null;
    state.playbackRowKey = null;
    state.activeWaveform = null;
    state.playbackLabelContext = null;
    stopPlayheadInterpolation(state, { cancelAnimationFrameFn });
    clearAllWaveformPlayheads();
    updateTransportButtonsInDom();
    setStatus("Idle");
    return;
  }

  if (eventName === "playback.error") {
    const message = payload.message ? String(payload.message) : "Playback failed";
    setStatus(message);
  }
}

export async function unregisterBackendJobEvents(state, deps = {}) {
  const warn = deps.warn || (() => {});
  const unlistenFns = [state.unlistenJobEvent, state.unlistenPlaybackEvent, state.unlistenBackendLogEvent]
    .filter((fn) => typeof fn === "function");
  state.unlistenJobEvent = null;
  state.unlistenPlaybackEvent = null;
  state.unlistenBackendLogEvent = null;
  for (const fn of unlistenFns) {
    try {
      await Promise.resolve(fn());
    } catch (err) {
      warn("Failed to unlisten backend event:", err);
    }
  }
}

export async function registerBackendJobEvents(state, deps) {
  const {
    isTauriRuntime,
    unregisterBackendJobEvents,
    getTauriEventListen,
    handleJobEvent,
    handlePlaybackEvent,
    handleBackendLogEvent
  } = deps;

  if (!isTauriRuntime()) return;
  await unregisterBackendJobEvents();
  const listen = await getTauriEventListen();
  if (!listen) return;

  const unlisten = await listen("job:event", (event) => {
    handleJobEvent(event?.payload);
  });

  if (typeof unlisten === "function") {
    state.unlistenJobEvent = unlisten;
  }

  const unlistenPlayback = await listen("playback:event", (event) => {
    handlePlaybackEvent(event?.payload);
  });
  if (typeof unlistenPlayback === "function") {
    state.unlistenPlaybackEvent = unlistenPlayback;
  }

  if (typeof handleBackendLogEvent === "function") {
    const unlistenBackendLog = await listen("backend:log", (event) => {
      handleBackendLogEvent(event?.payload);
    });
    if (typeof unlistenBackendLog === "function") {
      state.unlistenBackendLogEvent = unlistenBackendLog;
    }
  }
}

export function bindBeforeUnloadCleanup(windowObj, unregisterBackendJobEvents) {
  windowObj.addEventListener("beforeunload", () => {
    unregisterBackendJobEvents().catch(() => {});
  });
}
