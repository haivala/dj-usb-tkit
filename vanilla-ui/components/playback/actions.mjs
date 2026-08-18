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

function getPlaybackSourceLabelFn(deps = {}) {
  if (typeof deps.getPlaybackSourceLabel === "function") return deps.getPlaybackSourceLabel;
  const fallback = globalThis?.playbackSourceLabel?.getPlaybackSourceLabel;
  return typeof fallback === "function" ? fallback : () => "Local file";
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

function isMaterializedLocalId(candidate, normalizePath) {
  if (!candidate?.id) return false;
  const candidateId = normalizePath(candidate.id);
  const candidatePath = normalizePath(candidate.filePath || "");
  return !!candidateId && candidateId !== candidatePath;
}

export function resolveLocalTrackId(track, state, deps) {
  const { normalizePath } = deps;
  if (!track) return null;
  if (track.localTrackId) return track.localTrackId;

  if (track.id) {
    const byId = state.tracks.find((t) => t.id === track.id);
    if (byId && isMaterializedLocalId(byId, normalizePath)) {
      return track.id;
    }
  }

  const normTitle = String(track.title || "").trim().toLowerCase();
  const normArtist = String(track.artist || "").trim().toLowerCase();
  const normAlbum = String(track.album || "").trim().toLowerCase();
  const normFormat = String(track.formatExt || "").trim().toLowerCase();
  const normPath = normalizePath(track.filePath || "");

  if (normPath) {
    const byPath = state.tracks.find((t) => normalizePath(t.filePath || "") === normPath);
    if (byPath && isMaterializedLocalId(byPath, normalizePath)) {
      return byPath.id;
    }
  }

  const strictMatch = state.tracks.find(
    (t) => isMaterializedLocalId(t, normalizePath)
      && String(t.title || "").trim().toLowerCase() === normTitle
      && String(t.artist || "").trim().toLowerCase() === normArtist
      && (!normAlbum || String(t.album || "").trim().toLowerCase() === normAlbum)
      && (!normFormat || String(t.formatExt || "").trim().toLowerCase() === normFormat)
  );
  if (strictMatch?.id) return strictMatch.id;

  const looseMatch = state.tracks.find(
    (t) => isMaterializedLocalId(t, normalizePath)
      && String(t.title || "").trim().toLowerCase() === normTitle
      && String(t.artist || "").trim().toLowerCase() === normArtist
  );
  return looseMatch?.id || null;
}

export function shouldAllowResolvedFallback(track, state, deps) {
  const { normalizePath } = deps;
  if (!track) return false;
  const filePath = String(track.filePath || "").trim();
  const usbRoot = String(state.usbRoot || "").trim();
  const usbAnalysisPath = String(track.usbAnalysisPath || "").trim();
  if (usbAnalysisPath) return false;
  if (usbRoot && filePath && normalizePath(filePath).startsWith(normalizePath(usbRoot))) {
    return false;
  }
  return true;
}

export async function resolveLocalTrackIdAsync(track, state, deps) {
  const {
    command,
    normalizePath,
    promoteTrackIdentity
  } = deps;
  const resolveLocalTrackIdFn = deps.resolveLocalTrackId
    || ((value) => resolveLocalTrackId(value, state, { normalizePath }));
  const shouldAllowResolvedFallbackFn = deps.shouldAllowResolvedFallback
    || ((value) => shouldAllowResolvedFallback(value, state, { normalizePath }));

  const syncId = resolveLocalTrackIdFn(track);
  if (syncId) return syncId;
  if (!track) return null;

  const filePath = String(track.filePath || "").trim();
  const isUsbOrigin = !shouldAllowResolvedFallbackFn(track);
  if (filePath && !isUsbOrigin) {
    try {
      const data = await command("materialize_source_track", {
        filePath,
        title: track.title || "",
        artist: track.artist || "",
        album: track.album || null,
        trackNumber: toNumberOrNull(track.trackNumber),
        key: track.key || null,
        fileSizeBytes: toNumberOrNull(track.fileSizeBytes),
        formatExt: track.formatExt || null,
        sampleRateHz: toNumberOrNull(track.sampleRateHz),
        bitDepth: toNumberOrNull(track.bitDepth),
        bitrateKbps: toNumberOrNull(track.bitrateKbps)
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
  }

  const title = String(track.title || "").trim();
  const artist = String(track.artist || "").trim();
  if (!title || !artist) return null;
  try {
    const data = await command("resolve_playback_source", {
      title,
      artist,
      album: track.album || null,
      bpm: toNumberOrNull(track.bpm),
      filePath: filePath || null,
      fileSizeBytes: toNumberOrNull(track.fileSizeBytes)
    });
    return data?.trackId || null;
  } catch (_) {
    return null;
  }
}

function getFileName(value) {
  const normalized = String(value || "").replace(/\\/g, "/").trim().toLowerCase();
  if (!normalized) return "";
  const i = normalized.lastIndexOf("/");
  return i >= 0 ? normalized.slice(i + 1) : normalized;
}

function getStem(value) {
  const file = getFileName(value);
  const i = file.lastIndexOf(".");
  return i > 0 ? file.slice(0, i) : file;
}

export function resolveLocalTrack(track, state) {
  if (!track) return null;

  if (track.id) {
    const byId = state.tracks.find((t) => t.id === track.id);
    if (byId?.filePath) return byId;
  }

  const title = String(track.title || "").trim().toLowerCase();
  const artist = String(track.artist || "").trim().toLowerCase();
  if (!title) return null;

  const byMeta = state.tracks.find((t) => {
    const tTitle = String(t.title || "").trim().toLowerCase();
    const tArtist = String(t.artist || "").trim().toLowerCase();
    return tTitle === title && tArtist === artist && !!t.filePath;
  });
  if (byMeta) return byMeta;

  const sourcePath = String(track.filePath || "").replace(/\\/g, "/").trim().toLowerCase();
  if (!sourcePath) return null;

  const byExactPath = state.tracks.find(
    (t) => String(t.filePath || "").replace(/\\/g, "/").trim().toLowerCase() === sourcePath && !!t.filePath
  );
  if (byExactPath) return byExactPath;

  const sourceFile = getFileName(sourcePath);
  const sourceStem = getStem(sourcePath);
  if (!sourceFile && !sourceStem) return null;

  const byFile = state.tracks.filter((t) => getFileName(t.filePath) === sourceFile && !!t.filePath);
  if (byFile.length === 1) return byFile[0];
  if (byFile.length > 1) {
    const narrowed = byFile.find((t) => {
      const tTitle = String(t.title || "").trim().toLowerCase();
      const tArtist = String(t.artist || "").trim().toLowerCase();
      return (title && tTitle === title) || (artist && tArtist === artist);
    });
    if (narrowed) return narrowed;
  }

  const byStem = state.tracks.filter((t) => getStem(t.filePath) === sourceStem && !!t.filePath);
  if (byStem.length === 1) return byStem[0];
  if (byStem.length > 1) {
    const narrowed = byStem.find((t) => {
      const tTitle = String(t.title || "").trim().toLowerCase();
      const tArtist = String(t.artist || "").trim().toLowerCase();
      return (title && tTitle === title) || (artist && tArtist === artist);
    });
    if (narrowed) return narrowed;
  }

  return null;
}

export function getTrackPlaybackPath(track, deps) {
  const { resolveLocalTrack } = deps;
  const localTrack = resolveLocalTrack(track);
  return localTrack?.filePath || track?.filePath || "";
}

export function isTrackCurrentlyPlaying(track, state, deps) {
  const { normalizePath, getTrackPlaybackPath } = deps;
  if (state.playbackPendingKind === "stop") return false;
  if (state.playbackPendingKind === "play") {
    return !!(state.playbackPendingTrackId && track?.id && state.playbackPendingTrackId === track.id);
  }
  if (!state.playbackActive) return false;
  if (state.playbackTrackId && track?.id && state.playbackTrackId === track.id) return true;
  const a = normalizePath(getTrackPlaybackPath(track));
  const b = normalizePath(state.playbackPath || "");
  return !!a && !!b && a === b;
}
export async function playTrackFromOrigin(state, track, origin, options = {}, deps) {
  const {
    command,
    trackPathMatchesAnyRoot,
    clearAllWaveformPlayheads,
    setWaveformPlayhead,
    updateTransportButtonsInDom,
    setStatus,
    warn,
    generation,
    requestAnimationFrameFn,
    cancelAnimationFrameFn
  } = deps;

  const trackPath = String(track?.filePath || "").trim();
  const originLower = String(origin || "").toLowerCase();

  // Every origin (library, playlist, USB, history) now routes through the
  // same resolution call -- Steps 6/6c on the backend guarantee this only
  // ever returns a genuine local row (or a fast "self" match for the
  // already-fine common case), so a playlist entry that still references a
  // stale USB placeholder self-heals here with no migration required.
  let resolved = null;
  try {
    resolved = await command("resolve_playback_source", {
      title: track?.title || "",
      artist: track?.artist || "",
      album: track?.album || null,
      bpm: Number.isFinite(Number(track?.bpm)) ? Number(track.bpm) : null,
      filePath: track?.filePath || null,
      fileSizeBytes: Number.isFinite(Number(track?.fileSizeBytes)) ? Number(track.fileSizeBytes) : null,
      trackId: track?.id || null
    });
  } catch (err) {
    warn("resolve_playback_source failed:", err);
  }

  const artist = String(track?.artist || "").trim();
  const titlePart = track?.title || "Unknown Title";
  const title = artist ? `${artist} - ${titlePart}` : titlePart;
  const startRatio = Math.max(0, Math.min(1, Number(options.startRatio) || 0));
  const waveformEl = options.waveformEl || null;

  const hasUsbContext = !!state.usbRoot && !!state.usbRootValid;
  const isLibraryResolved = resolved?.matchedBy === "self"
    || resolved?.matchedBy === "hash"
    || resolved?.matchedBy === "metadata";
  const libraryPath = isLibraryResolved ? String(resolved?.resolvedPath || "").trim() : "";
  // "is this literally under the mounted USB root" -- unrelated to the backend fix,
  // this is just how the transient audio-file fallback below decides it has a USB copy to try.
  const usbPath = hasUsbContext && trackPathMatchesAnyRoot(trackPath, [state.usbRoot])
    ? trackPath
    : "";

  const playPath = libraryPath || usbPath;
  const playId = isLibraryResolved
    ? (resolved?.trackId || track?.id || null)
    : (track?.id || null);
  const sourceLabel = getPlaybackSourceLabelFn(deps)({
    origin: originLower,
    libraryResolved: isLibraryResolved,
    hasUsbContext
  });

  const playNativeWithRecovery = async (path) => {
    try {
      return await command("play_track_native", { path, startRatio });
    } catch (err) {
      const message = String(err?.message || err || "").toLowerCase();
      const recoverable = /busy|already|in use|device|stream|sink|playing/.test(message);
      if (!recoverable) throw err;
      try {
        await command("stop_playback_native");
      } catch (stopErr) {
        warn("stop_playback_native recovery attempt failed:", stopErr);
      }
      return command("play_track_native", { path, startRatio });
    }
  };

  if (playPath) {
    return withBackendQueue(state, async () => {
      if (!isGenerationCurrent(state, generation)) return;
      try {
        const playback = await playNativeWithRecovery(playPath);
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
        state.playbackActive = true;
        state.playbackTrackId = playId;
        state.playbackPath = playback?.path || playPath;
        state.playbackRowKey = options.rowKey || null;
        state.playbackLabelContext = { origin: originLower, libraryResolved: isLibraryResolved, hasUsbContext, title };
        updateTransportButtonsInDom();
        setStatus(`Playing from ${sourceLabel}: ${title}`);
        return;
      } catch (err) {
        if (libraryPath && usbPath && usbPath !== playPath) {
          try {
            const playback = await playNativeWithRecovery(usbPath);
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
            state.playbackActive = true;
            state.playbackTrackId = track?.id || null;
            state.playbackPath = playback?.path || usbPath;
            state.playbackRowKey = options.rowKey || null;
            state.playbackLabelContext = { origin: originLower, libraryResolved: false, hasUsbContext, title };
            updateTransportButtonsInDom();
            setStatus(`Playing from USB (library unavailable): ${title}`);
            return;
          } catch (fallbackErr) {
            if (!isGenerationCurrent(state, generation)) return;
            const message = fallbackErr?.message || String(fallbackErr);
            setStatus(`Playback failed (${sourceLabel}): ${message}`, { level: "error", source: "playback" });
            return;
          }
        }
        if (!isGenerationCurrent(state, generation)) return;
        const message = err?.message || String(err);
        setStatus(`Playback failed (${sourceLabel}): ${message}`, { level: "error", source: "playback" });
        return;
      }
    });
  }

  setStatus("Cannot play: track not found in Library or selected USB.", { level: "warn", source: "playback" });
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
export function findTrackIdByPath(state, path, deps) {
  const { normalizePath } = deps;
  const target = normalizePath(path || "");
  if (!target) return null;
  const match = (state.tracks || []).find((t) => normalizePath(t.filePath || "") === target);
  return match?.id || null;
}

export function handlePlaybackEvent(state, payload, deps) {
  const {
    setWaveformPlayhead,
    updateTransportButtonsInDom,
    clearAllWaveformPlayheads,
    setStatus,
    resolveTrackIdForPath,
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
    // These are one-shot confirmations tied directly to our own play_track_native call
    // (unlike a continuous progress stream). If we have no active path and nothing
    // pending, this can't be a legitimate confirmation of anything we're waiting on —
    // treat a stray playing:true here as noise rather than reviving cleared state.
    const noActiveOrPendingContext = !state.playbackActive && !state.playbackPath && state.playbackPendingKind !== "play";
    if (playing && noActiveOrPendingContext) return;

    const pathChanged = path !== null && path !== state.playbackPath;
    state.playbackActive = playing;
    state.playbackPath = path;
    if (pathChanged) {
      state.playbackTrackId = typeof resolveTrackIdForPath === "function" ? resolveTrackIdForPath(path) : null;
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
    // Make the status line a live projection of playback state rather than
    // a one-shot string frozen at play-dispatch time -- recompute the
    // label from the same context playTrackFromOrigin stashed, so later
    // events (e.g. a seek) keep it accurate.
    if (playing && state.playbackLabelContext) {
      const { origin, libraryResolved, hasUsbContext, title } = state.playbackLabelContext;
      const label = getPlaybackSourceLabelFn(deps)({ origin, libraryResolved, hasUsbContext });
      setStatus(`Playing from ${label}: ${title}`);
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
