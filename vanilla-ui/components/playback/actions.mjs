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

export function updateTransportButtonsInDom(state, document) {
  const helpers = getPlaybackUiStateHelpers();
  document.querySelectorAll(".transport-btn").forEach((btn) => {
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
    btn.setAttribute("title", isPlaying ? "Stop" : "Play");
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

export async function stopPlaybackFromUi(state, deps) {
  const {
    command,
    clearAllWaveformPlayheads,
    updateTransportButtonsInDom,
    setStatus
  } = deps;
  if (state.playbackStopPromise) return state.playbackStopPromise;
  if (!state.playbackActive) {
    setStatus("Idle");
    return;
  }
  state.playbackStopPromise = (async () => {
    await command("stop_playback_native");
    state.playbackActive = false;
    state.playbackTrackId = null;
    state.playbackPath = null;
    state.playbackRowKey = null;
    state.activeWaveform = null;
    clearAllWaveformPlayheads();
    updateTransportButtonsInDom();
    setStatus("Idle");
  })();
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

export function scoreLocalTrackCandidate(candidate, sourceTrack) {
  const matcher = globalThis?.playbackMatch;
  if (matcher && typeof matcher.scoreLocalTrackCandidate === "function") {
    return matcher.scoreLocalTrackCandidate(candidate, sourceTrack);
  }
  if (!candidate?.filePath) return -1;

  const normalize = (v) => String(v || "").trim().toLowerCase();
  const normPath = (v) => String(v || "").replace(/\\/g, "/").trim().toLowerCase();
  const fileName = (v) => {
    const p = normPath(v);
    const idx = p.lastIndexOf("/");
    return idx >= 0 ? p.slice(idx + 1) : p;
  };
  const stem = (v) => {
    const f = fileName(v);
    const idx = f.lastIndexOf(".");
    return idx > 0 ? f.slice(0, idx) : f;
  };

  const srcTitle = normalize(sourceTrack?.title);
  const srcArtist = normalize(sourceTrack?.artist);
  const srcAlbum = normalize(sourceTrack?.album);
  const srcPath = normPath(sourceTrack?.filePath);
  const srcFile = fileName(sourceTrack?.filePath);
  const srcStem = stem(sourceTrack?.filePath);

  const candTitle = normalize(candidate.title);
  const candArtist = normalize(candidate.artist);
  const candAlbum = normalize(candidate.album);
  const candPath = normPath(candidate.filePath);
  const candFile = fileName(candidate.filePath);
  const candStem = stem(candidate.filePath);

  let score = 0;
  if (srcTitle && candTitle === srcTitle) score += 12;
  if (srcArtist && candArtist === srcArtist) score += 12;
  if (srcAlbum && candAlbum === srcAlbum) score += 8;
  if (srcPath && candPath === srcPath) score += 24;
  if (srcFile && candFile === srcFile) score += 16;
  if (srcStem && candStem === srcStem) score += 8;

  const srcBpm = Number(sourceTrack?.bpm);
  const candBpm = Number(candidate?.bpm);
  if (Number.isFinite(srcBpm) && Number.isFinite(candBpm) && Math.abs(srcBpm - candBpm) <= 0.15) {
    score += 4;
  }

  return score;
}

export async function resolveLocalTrackForPlayback(track, state, deps) {
  const {
    command,
    normalizeTrack
  } = deps;
  const resolveLocalTrackFn = deps.resolveLocalTrack || ((value) => resolveLocalTrack(value, state));
  const scoreLocalTrackCandidateFn = deps.scoreLocalTrackCandidate || scoreLocalTrackCandidate;

  const local = resolveLocalTrackFn(track);
  if (local?.filePath) return local;

  const query = String(track?.title || "").trim();
  if (!query) return null;

  try {
    const data = await command("search_tracks", {
      query,
      limit: 250,
      cursor: null
    });
    const remote = (data?.items || []).map((t) => normalizeTrack(t, "lib-srch"));
    if (!remote.length) return null;

    const matcher = globalThis?.playbackMatch;
    if (matcher && typeof matcher.selectBestLocalMatch === "function") {
      return matcher.selectBestLocalMatch(track, remote, 16);
    }

    let best = null;
    let bestScore = -1;
    for (const candidate of remote) {
      const score = scoreLocalTrackCandidateFn(candidate, track);
      if (score > bestScore) {
        bestScore = score;
        best = candidate;
      }
    }
    return bestScore >= 16 ? best : null;
  } catch (err) {
    console.warn("Fallback local lookup failed:", err);
    return null;
  }
}

export function getTrackPlaybackPath(track, deps) {
  const { resolveLocalTrack } = deps;
  const localTrack = resolveLocalTrack(track);
  return localTrack?.filePath || track?.filePath || "";
}

export function isTrackCurrentlyPlaying(track, state, deps) {
  const { normalizePath, getTrackPlaybackPath } = deps;
  if (!state.playbackActive) return false;
  if (state.playbackTrackId && track?.id && state.playbackTrackId === track.id) return true;
  const a = normalizePath(getTrackPlaybackPath(track));
  const b = normalizePath(state.playbackPath || "");
  return !!a && !!b && a === b;
}
export async function playTrackFromOrigin(state, track, origin, options = {}, deps) {
  const {
    command,
    resolveLocalTrackForPlayback,
    trackPathMatchesAnyRoot,
    clearAllWaveformPlayheads,
    setWaveformPlayhead,
    updateTransportButtonsInDom,
    setStatus,
    warn
  } = deps;

  let localTrack = null;
  const trackPath = String(track?.filePath || "").trim();
  const isLibraryOrigin = String(origin || "").toLowerCase() === "library";
  if (isLibraryOrigin && trackPath) {
    localTrack = {
      id: track?.id || null,
      title: track?.title || "Unknown Title",
      filePath: trackPath
    };
  } else {
    try {
      const resolved = await command("resolve_playback_source", {
        title: track?.title || "",
        artist: track?.artist || "",
        album: track?.album || null,
        bpm: Number.isFinite(Number(track?.bpm)) ? Number(track.bpm) : null,
        filePath: track?.filePath || null,
        fileSizeBytes: Number.isFinite(Number(track?.fileSizeBytes)) ? Number(track.fileSizeBytes) : null
      });
      if (resolved?.resolvedPath) {
        localTrack = {
          id: resolved?.trackId || null,
          title: track?.title || "Unknown Title",
          filePath: resolved.resolvedPath
        };
      }
    } catch (err) {
      warn("resolve_playback_source failed, using frontend matcher fallback:", err);
    }
  }
  if (!localTrack) {
    localTrack = await resolveLocalTrackForPlayback(track);
  }
  const artist = String(track?.artist || "").trim();
  const titlePart = localTrack?.title || track?.title || "Unknown Title";
  const title = artist ? `${artist} - ${titlePart}` : titlePart;
  const startRatio = Math.max(0, Math.min(1, Number(options.startRatio) || 0));
  const waveformEl = options.waveformEl || null;

  const hasUsbContext = !!state.usbRoot && !!state.usbRootValid;
  const resolvedPath = String(localTrack?.filePath || "").trim();
  const libraryPath = trackPathMatchesAnyRoot(resolvedPath, state.sourceRoots || [])
    ? resolvedPath
    : (trackPathMatchesAnyRoot(trackPath, state.sourceRoots || []) ? trackPath : "");
  // master.db tracks live anywhere — not necessarily under a source root
  const masterDbPath = (!libraryPath && track?.masterDbSource) ? (resolvedPath || trackPath) : "";
  const usbPath = hasUsbContext && trackPathMatchesAnyRoot(trackPath, [state.usbRoot])
    ? trackPath
    : "";

  const isLibraryResolved = !!(libraryPath || masterDbPath);
  const playPath = libraryPath || masterDbPath || usbPath;
  const playId = isLibraryResolved
    ? (localTrack?.id || track?.id || null)
    : (track?.id || localTrack?.id || null);
  const sourceLabel = isLibraryResolved ? "Library" : (usbPath ? "USB" : "Unavailable");

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

  if (playPath && sourceLabel !== "Unavailable") {
    try {
      const playback = await playNativeWithRecovery(playPath);
      if (waveformEl) {
        clearAllWaveformPlayheads();
        state.activeWaveform = waveformEl;
        const duration = Number(playback?.durationMs || 0);
        const position = Number(playback?.positionMs || 0);
        setWaveformPlayhead(waveformEl, duration > 0 ? position / duration : startRatio, true);
      }
      state.playbackActive = true;
      state.playbackTrackId = playId;
      state.playbackPath = playback?.path || playPath;
      state.playbackRowKey = options.rowKey || null;
      updateTransportButtonsInDom();
      setStatus(`Playing from ${sourceLabel}: ${title}`);
      return;
    } catch (err) {
      if (libraryPath && usbPath && usbPath !== playPath) {
        try {
          const playback = await playNativeWithRecovery(usbPath);
          if (waveformEl) {
            clearAllWaveformPlayheads();
            state.activeWaveform = waveformEl;
            const duration = Number(playback?.durationMs || 0);
            const position = Number(playback?.positionMs || 0);
            setWaveformPlayhead(waveformEl, duration > 0 ? position / duration : startRatio, true);
          }
          state.playbackActive = true;
          state.playbackTrackId = track?.id || null;
          state.playbackPath = playback?.path || usbPath;
          state.playbackRowKey = options.rowKey || null;
          updateTransportButtonsInDom();
          setStatus(`Playing from USB (library unavailable): ${title}`);
          return;
        } catch (fallbackErr) {
          const message = fallbackErr?.message || String(fallbackErr);
          setStatus(`Playback failed (${sourceLabel}): ${message}`);
          return;
        }
      }
      const message = err?.message || String(err);
      setStatus(`Playback failed (${sourceLabel}): ${message}`);
      return;
    }
  }

  setStatus("Cannot play: track not found in Library or selected USB.");
}
export async function stopPlaybackIfActive(state, deps) {
  const {
    command,
    clearAllWaveformPlayheads,
    updateTransportButtonsInDom,
    setStatus,
    warn
  } = deps;
  if (state.playbackStopPromise) return state.playbackStopPromise;
  if (!state.playbackActive) return;
  state.playbackStopPromise = (async () => {
    try {
      await command("stop_playback_native");
    } catch (err) {
      warn("Failed to stop playback on context change:", err);
    }
    state.playbackActive = false;
    state.playbackTrackId = null;
    state.playbackPath = null;
    state.playbackRowKey = null;
    state.activeWaveform = null;
    clearAllWaveformPlayheads();
    updateTransportButtonsInDom();
    setStatus("Idle");
  })();
  try {
    await state.playbackStopPromise;
  } finally {
    state.playbackStopPromise = null;
  }
}

export async function playTrackFromOriginController(state, track, origin, options = {}, deps) {
  const { playTrackFromOriginCore } = deps;
  if (state.playbackStartPromise) {
    return state.playbackStartPromise;
  }
  const run = (async () => {
    if (state.playbackStopPromise) {
      await state.playbackStopPromise;
    }
    return playTrackFromOriginCore(state, track, origin, options, deps);
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
    setStatus
  } = deps;

  if (!payload || typeof payload !== "object") return;
  const eventName = String(payload.event || "");
  const path = payload.path ? String(payload.path) : null;
  const playing = !!payload.playing;
  const position = Number(payload.positionMs || 0);
  const duration = Number(payload.durationMs || 0);

  if (
    eventName === "playback.started"
    || eventName === "playback.seeked"
    || eventName === "playback.progress"
  ) {
    state.playbackActive = playing;
    state.playbackPath = path;
    if (state.activeWaveform) {
      const fraction = duration > 0 ? position / duration : 0;
      setWaveformPlayhead(state.activeWaveform, fraction, playing);
    }
    updateTransportButtonsInDom();
    return;
  }

  if (eventName === "playback.stopped") {
    state.playbackActive = false;
    state.playbackPath = null;
    state.playbackTrackId = null;
    state.playbackRowKey = null;
    state.activeWaveform = null;
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
