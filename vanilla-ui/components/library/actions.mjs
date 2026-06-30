export function trackHasRenderableWaveform(track) {
  const hasPreview = Array.isArray(track?.waveformPreview)
    && track.waveformPreview.length > 0
    && track.waveformPreview.some((v) => Number(v) > 0);
  const hasWaveformPath = typeof track?.waveformPeaksPath === "string"
    && track.waveformPeaksPath.trim().length > 0;
  return hasPreview || hasWaveformPath;
}

function resolveEmitStatus(deps = {}) {
  if (typeof deps.emitStatus === "function") return deps.emitStatus;
  if (typeof deps.setStatus === "function") return deps.setStatus;
  return () => {};
}

function findAnalysisAutoLimitWarning(warnings) {
  if (!Array.isArray(warnings)) return null;
  for (const warning of warnings) {
    const text = String(warning || "").trim();
    if (text.startsWith("Auto analysis limit reached:")) {
      return text;
    }
  }
  return null;
}

export function trackHasArtwork(track) {
  return !!(track?.artworkDataUrl || track?.artworkPath || track?.artworkUrl);
}

export function trackArtworkChecked(track) {
  return track?.artworkChecked === true || track?.artwork_checked === true;
}

export function trackHasBpm(track) {
  return Number.isFinite(Number(track?.bpm)) && Number(track.bpm) > 0;
}

export function trackHasKey(track) {
  return typeof track?.key === "string" && track.key.trim().length > 0;
}

export function trackHasCoreAnalysis(track, deps = {}) {
  const hasWaveform = deps.trackHasRenderableWaveform || trackHasRenderableWaveform;
  const hasBpm = deps.trackHasBpm || trackHasBpm;
  const durationMs = Number(track?.durationMs);
  return hasWaveform(track)
    && hasBpm(track)
    && Number.isFinite(durationMs)
    && durationMs > 0;
}

export function isUsbOriginTrack(track, deps = {}) {
  const {
    usbRoot = "",
    normalizePath = (value) => String(value || "").replace(/\\/g, "/").trim().toLowerCase()
  } = deps;
  if (!track) return false;
  const usbAnalysisPath = String(track.usbAnalysisPath || "").trim();
  if (usbAnalysisPath) return true;
  const normUsbRoot = normalizePath(usbRoot || "");
  const filePath = normalizePath(track.filePath || "");
  const waveformPath = normalizePath(track.waveformPeaksPath || "");
  if (normUsbRoot && filePath && filePath.startsWith(normUsbRoot)) return true;
  if (normUsbRoot && waveformPath && waveformPath.startsWith(normUsbRoot)) return true;
  return false;
}

export function resolveMissingAnalysisPieces(track, deps = {}) {
  const hasArtwork = deps.trackHasArtwork || trackHasArtwork;
  const artworkChecked = deps.trackArtworkChecked || trackArtworkChecked;
  const hasWaveform = deps.trackHasRenderableWaveform || trackHasRenderableWaveform;
  const hasBpm = deps.trackHasBpm || trackHasBpm;

  const pieces = [];
  const durationMs = Number(track?.durationMs);
  if (!Number.isFinite(durationMs) || durationMs <= 0) pieces.push("duration");
  if (!hasArtwork(track) && !artworkChecked(track)) pieces.push("artwork");
  if (!hasWaveform(track)) pieces.push("waveform");
  if (!hasBpm(track)) pieces.push("bpm_key");
  return pieces;
}

export function usbTrackNeedsHydration(track, deps = {}) {
  const hasWaveform = deps.trackHasRenderableWaveform || trackHasRenderableWaveform;
  const hasArtwork = deps.trackHasArtwork || trackHasArtwork;
  const artworkChecked = deps.trackArtworkChecked || trackArtworkChecked;
  const hasBpm = deps.trackHasBpm || trackHasBpm;
  const hasKey = deps.trackHasKey || trackHasKey;
  const needsPreviewHydration = deps.trackNeedsPreviewHydration || trackNeedsPreviewHydration;
  if (!track) return false;
  const artworkResolved = hasArtwork(track) || artworkChecked(track);
  return needsPreviewHydration(track) || !(hasWaveform(track) && artworkResolved && hasBpm(track) && hasKey(track));
}
function inferFormatFromPath(filePath) {
  if (!filePath) return "";
  const match = filePath.match(/\.([a-zA-Z0-9]+)$/);
  return match ? String(match[1] || "").toLowerCase() : "";
}

function clampWaveformPreview(value) {
  if (!Array.isArray(value)) return [];
  return value.map((v) => Math.max(0, Math.min(100, Number(v) || 0)));
}

function toFiniteOrNull(value) {
  const n = Number(value);
  return Number.isFinite(n) ? n : null;
}

export function normalizeTrack(track, fallbackIdPrefix = "t", deps = {}) {
  const {
    toPlayableUrl = () => null,
    appendUrlRevision = (url) => url,
    normalizeDurationMs = () => null,
    randomId = () => Math.random().toString(36).slice(2, 9)
  } = deps;

  const rawArtwork = track?.artworkDataUrl || track?.artworkUrl || track?.artworkPath || "";
  const artworkRevision = track?.updatedAt || track?.updated_at || "";
  const convertedArtwork = appendUrlRevision(toPlayableUrl(rawArtwork) || "", artworkRevision);
  const filePath = String(track?.filePath || "").trim();
  const inferredFormat = inferFormatFromPath(filePath);
  const waveformPreview = clampWaveformPreview(track?.waveformPreview);
  const title = track?.title || "Unknown Title";
  const artist = track?.artist || "Unknown Artist";
  const album = track?.album || "";
  const durationMs = normalizeDurationMs(track);

  return {
    id: track?.id || `${fallbackIdPrefix}-${randomId()}`,
    localTrackId: track?.localTrackId || track?.local_track_id || null,
    title,
    artist,
    album,
    trackNumber: toFiniteOrNull(track?.trackNumber),
    bpm: track?.bpm || "",
    bpmAnalyzer: track?.bpmAnalyzer || track?.bpm_analyzer || "",
    key: track?.key || "",
    artworkUrl: convertedArtwork,
    artworkDataUrl: track?.artworkDataUrl || "",
    artworkPath: track?.artworkPath || "",
    artworkChecked: trackArtworkChecked(track),
    filePath,
    durationMs,
    waveformPeaksPath: track?.waveformPeaksPath || "",
    usbAnalysisPath: track?.usbAnalysisPath || "",
    formatExt: track?.formatExt || track?.format_ext || inferredFormat,
    sampleRateHz: toFiniteOrNull(track?.sampleRateHz ?? track?.sample_rate_hz),
    bitDepth: toFiniteOrNull(track?.bitDepth ?? track?.bit_depth),
    bitrateKbps: toFiniteOrNull(track?.bitrateKbps ?? track?.bitrate_kbps),
    waveformPreview,
    waveformColorData: Array.isArray(track?.waveformColorData) ? track.waveformColorData : null,
    createdAt: track?.createdAt || track?.created_at || "",
    updatedAt: track?.updatedAt || track?.updated_at || "",
    searchText: `${title} ${artist} ${album}`.toLowerCase(),
    masterDbSource: !!(track?.masterDbSource ?? track?.master_db_source)
  };
}

export function normalizeUsbPlaylist(playlist, deps = {}) {
  const normalizeTrackFn = deps.normalizeTrack || ((track, prefix) => normalizeTrack(track, prefix, deps));
  const rawTracks = Array.isArray(playlist?.tracks)
    ? playlist.tracks
    : Array.isArray(playlist?.items)
      ? playlist.items
      : [];
  const tracks = rawTracks.map((track) => normalizeTrackFn(track, "usb"));
  const declared = Number(
    playlist?.trackCount ?? playlist?.track_count ?? playlist?.count ?? 0
  );
  return {
    ...playlist,
    source: String(playlist?.source || "unknown"),
    tracks,
    trackCount: Math.max(Number.isFinite(declared) ? declared : 0, tracks.length)
  };
}
function mergeTrackPreservingBestFields(existing, normalized) {
  const merged = { ...existing, ...normalized };
  if ((!Array.isArray(normalized.waveformPreview) || normalized.waveformPreview.length === 0)
    && Array.isArray(existing.waveformPreview)
    && existing.waveformPreview.length > 0) {
    merged.waveformPreview = [...existing.waveformPreview];
  }
  if ((!Array.isArray(normalized.waveformColorData) || normalized.waveformColorData.length === 0)
    && Array.isArray(existing.waveformColorData)
    && existing.waveformColorData.length > 0) {
    merged.waveformColorData = existing.waveformColorData;
  }
  if (!normalized.artworkDataUrl && existing.artworkDataUrl) merged.artworkDataUrl = existing.artworkDataUrl;
  if (!normalized.artworkUrl && existing.artworkUrl) merged.artworkUrl = existing.artworkUrl;
  if (!normalized.artworkPath && existing.artworkPath) merged.artworkPath = existing.artworkPath;
  if (normalized.artworkChecked || existing.artworkChecked) merged.artworkChecked = true;
  if (!normalized.waveformPeaksPath && existing.waveformPeaksPath) merged.waveformPeaksPath = existing.waveformPeaksPath;
  if (!normalized.bpm && existing.bpm) merged.bpm = existing.bpm;
  if (!normalized.key && existing.key) merged.key = existing.key;
  return merged;
}

export function scheduleRealtimeTrackRender(state, deps) {
  const {
    clearTimeoutFn,
    setTimeoutFn,
    applySearchLocalFilter,
    renderCurrentPlaylistTracksFromState,
    delayMs = 60
  } = deps;
  if (state.realtimeRenderQueued) return;
  state.realtimeRenderQueued = true;
  if (state.realtimeRenderTimer) {
    clearTimeoutFn(state.realtimeRenderTimer);
  }
  state.realtimeRenderTimer = setTimeoutFn(() => {
    state.realtimeRenderTimer = null;
    state.realtimeRenderQueued = false;
    applySearchLocalFilter();
    renderCurrentPlaylistTracksFromState();
  }, delayMs);
}

export function trackNeedsPreviewHydration(track) {
  if (!track) return false;
  const missingWaveformPreview = !Array.isArray(track.waveformPreview) || track.waveformPreview.length === 0;
  const missingColorData = !Array.isArray(track.waveformColorData) || track.waveformColorData.length === 0;
  const hasWaveformPath = typeof track.waveformPeaksPath === "string" && track.waveformPeaksPath.trim().length > 0;
  return (missingWaveformPreview && missingColorData) && hasWaveformPath;
}

export function mergeHydratedTrackIntoState(state, rawTrack, deps) {
  const { normalizeTrack } = deps;
  const normalized = normalizeTrack(rawTrack, "lib");
  const trackId = String(normalized.id || "").trim();
  if (!trackId) return false;
  let changed = false;

  for (let i = 0; i < state.tracks.length; i += 1) {
    const existing = state.tracks[i];
    if (String(existing?.id || "") !== trackId) continue;
    state.tracks[i] = mergeTrackPreservingBestFields(existing, normalized);
    changed = true;
    break;
  }

  for (const playlist of state.playlists) {
    for (let i = 0; i < (playlist.tracks || []).length; i += 1) {
      const existing = playlist.tracks[i];
      const localTrackId = String(existing?.localTrackId || existing?.id || "").trim();
      if (localTrackId !== trackId) continue;
      playlist.tracks[i] = mergeTrackPreservingBestFields(existing, normalized);
      changed = true;
    }
  }

  return changed;
}

export async function applyRealtimeAnalyzedTrackUpdate(state, payload, deps) {
  const {
    patchTrackAnalysisFields,
    debugFrontendLog,
    log,
    warn,
    patchLibraryRowByTrackId,
    scheduleRealtimeTrackRender,
    hydrateTrackPreviewFromBackend
  } = deps;

  const trackId = String(payload?.trackId || "").trim();
  if (!trackId) return;

  let libraryChanged = false;
  let patchedTrack = null;
  for (const track of state.tracks) {
    if (String(track.id) !== trackId) continue;
    libraryChanged = patchTrackAnalysisFields(track, payload) || libraryChanged;
    patchedTrack = track;
  }
  if (libraryChanged) {
    debugFrontendLog("row-update", {
      trackId,
      bpm: payload?.bpm ?? null,
      key: payload?.key ?? null
    });
    const label = patchedTrack
      ? [patchedTrack.artist, patchedTrack.title].filter(Boolean).join(" - ") || trackId
      : trackId;
    log("[analysis-ui] patched state for", label, "bpm:", payload?.bpm, "key:", payload?.key);
    patchLibraryRowByTrackId(trackId);
  } else {
    const bpm = Number(payload?.bpm);
    const hasBpm = payload?.bpm !== undefined
      && payload?.bpm !== null
      && Number.isFinite(bpm)
      && bpm > 0;
    const hasKey = typeof payload?.key === "string" && payload.key.trim();
    const hasAnalyzer = typeof payload?.bpmAnalyzer === "string" && payload.bpmAnalyzer.trim();
    const isBpmKeyPayload = hasBpm || hasKey || hasAnalyzer;
    if (isBpmKeyPayload) {
      const foundTrack = state.tracks.find((t) => String(t.id) === trackId);
      const warnLabel = foundTrack
        ? [foundTrack.artist, foundTrack.title].filter(Boolean).join(" - ") || trackId
        : trackId;
      warn("[analysis-ui] no state change for", warnLabel);
    }
  }

  let playlistChanged = false;
  for (const playlist of state.playlists) {
    for (const track of playlist.tracks || []) {
      const localTrackId = String(track.localTrackId || track.id || "").trim();
      if (localTrackId !== trackId) continue;
      playlistChanged = patchTrackAnalysisFields(track, payload) || playlistChanged;
    }
  }
  if (playlistChanged) {
    scheduleRealtimeTrackRender();
  }

  const payloadHasPreview = Array.isArray(payload?.waveformPreview) && payload.waveformPreview.length > 0;
  const payloadHasWaveformPath = typeof payload?.waveformPeaksPath === "string"
    && payload.waveformPeaksPath.trim().length > 0;
  if (!payloadHasPreview && payloadHasWaveformPath) {
    hydrateTrackPreviewFromBackend(trackId).catch(() => {});
  }
}

export async function hydrateTrackPreviewFromBackend(state, trackId, options = {}, deps) {
  const {
    command,
    mergeHydratedTrackIntoState,
    patchLibraryRowByTrackId,
    nextPaint,
    getLibraryVisibleTracks,
    updateLibraryDurationSummary,
    scheduleRealtimeTrackRender,
    renderSourceChips
  } = deps;
  const id = String(trackId || "").trim();
  if (!id) return;
  if (state.trackPreviewHydrateInFlight.has(id)) return;
  const updateSummary = options.updateSummary !== false;
  state.trackPreviewHydrateInFlight.add(id);
  try {
    const data = await command("get_tracks_by_ids_with_previews", { trackIds: [id] });
    let changed = false;
    for (const item of data?.items || []) {
      changed = mergeHydratedTrackIntoState(item) || changed;
    }
    if (changed) {
      patchLibraryRowByTrackId(id);
      if (updateSummary && !state.analyzingTrackIds.has(id) && state.analyzingTrackIds.size > 0) {
        await nextPaint();
        const visibleTracks = getLibraryVisibleTracks();
        updateLibraryDurationSummary(visibleTracks);
      }
      scheduleRealtimeTrackRender();
      renderSourceChips();
    }
  } finally {
    state.trackPreviewHydrateInFlight.delete(id);
  }
}

export async function hydrateLoadedTracksPreviewsInBackground(state, deps) {
  const {
    getLibraryVisibleTracks,
    command,
    mergeHydratedTrackIntoState,
    patchLibraryRowByTrackId,
    nextPaint,
    updateLibraryDurationSummary,
    scheduleRealtimeTrackRender,
    renderSourceChips,
    batchSize = 48
  } = deps;

  const hydrationSeq = ++state.loadedPreviewHydrationSeq;
  const targetTracks = getLibraryVisibleTracks();
  const pendingIds = targetTracks
    .filter((track) => trackNeedsPreviewHydration(track))
    .map((track) => String(track.id || "").trim())
    .filter(Boolean);
  if (!pendingIds.length) return;

  let anyChanged = false;
  for (let i = 0; i < pendingIds.length; i += batchSize) {
    if (hydrationSeq !== state.loadedPreviewHydrationSeq) return;
    const batch = pendingIds.slice(i, i + batchSize);
    try {
      const data = await command("get_tracks_by_ids_with_previews", { trackIds: batch });
      const changedIds = [];
      for (const item of data?.items || []) {
        const changed = mergeHydratedTrackIntoState(item);
        if (!changed) continue;
        anyChanged = true;
        const id = String(item?.id || "").trim();
        if (id) changedIds.push(id);
      }
      for (const id of changedIds) {
        patchLibraryRowByTrackId(id);
      }
      await nextPaint();
    } catch (_) {
      return;
    }
  }

  if (hydrationSeq !== state.loadedPreviewHydrationSeq) return;
  if (anyChanged) {
    const visibleTracks = getLibraryVisibleTracks();
    updateLibraryDurationSummary(visibleTracks);
    scheduleRealtimeTrackRender();
    renderSourceChips();
  }
}
export function getLibraryVisibleTracks(state) {
  return state.filteredTracks;
}

export function applySearchLocalFilter(state, el, deps = {}) {
  const {
    renderLibraryRows = () => {},
    updateSelectionCount = () => {}
  } = deps;

  const noSources = !state.sourceRoots.length && !state.masterDbEnabled;
  if (noSources) {
    state.filteredTracks = [];
    state.selectedTrackIds = new Set();
    renderLibraryRows();
    updateSelectionCount();
    return;
  }

  const query = String(el.librarySearch?.value || state.libraryQuery || "").trim().toLowerCase();
  state.filteredTracks = query
    ? state.tracks.filter((track) => {
      if (typeof track.searchText === "string" && track.searchText.length > 0) {
        return track.searchText.includes(query);
      }
      const fallback = `${track.title || ""} ${track.artist || ""} ${track.album || ""}`.toLowerCase();
      return fallback.includes(query);
    })
    : [...state.tracks];

  const visibleIds = new Set(state.filteredTracks.map((track) => track.id));
  state.selectedTrackIds = new Set(
    Array.from(state.selectedTrackIds).filter((id) => visibleIds.has(id))
  );
  renderLibraryRows();
  updateSelectionCount();
}

export function renderSourceChips(state, el, deps = {}) {
  const {
    documentObj = typeof document !== "undefined" ? document : null,
    escapeHtml = (value) => String(value || ""),
    trackPathMatchesAnyRoot = () => false,
    trackHasCoreAnalysis = () => false,
    persistSourceRootEnabled = () => {},
    updateScanLibraryButtonLabel = () => {},
    updateSourceFilterIndicator = () => {}
  } = deps;

  if (!documentObj) return;
  el.sourceChipsContainer.innerHTML = "";

  // master.db chip - shown when detected, positioned before filesystem chips
  if (state.externalMasterDbPath) {
    const chip = documentObj.createElement("span");
    chip.className = "source-chip source-chip-master-db";
    chip.innerHTML = `<input class="source-chip-toggle" type="checkbox" data-master-db="true" ${state.masterDbEnabled ? "checked" : ""} aria-label="Toggle desktop library" /><span class="source-chip-path">master.db</span>`;
    el.sourceChipsContainer.appendChild(chip);
  }

  state.sourceRoots.forEach((path, index) => {
    if (state.sourceRootEnabled[path] === undefined) {
      state.sourceRootEnabled[path] = true;
    }
    const scopedTracks = state.tracks.filter((track) => trackPathMatchesAnyRoot(track.filePath, [path]));
    const fullyAnalyzed = scopedTracks.length > 0 && scopedTracks.every((track) => {
      const durationMs = Number(track?.durationMs || 0);
      return trackHasCoreAnalysis(track) && Number.isFinite(durationMs) && durationMs > 0;
    });

    const chip = documentObj.createElement("span");
    chip.className = `source-chip${fullyAnalyzed ? " source-chip-analyzed" : ""}`;
    chip.innerHTML = `<input class="source-chip-toggle" type="checkbox" data-source-toggle-index="${index}" ${state.sourceRootEnabled[path] !== false ? "checked" : ""} aria-label="Filter source" /><span class="source-chip-path" title="${escapeHtml(path)}">${escapeHtml(path)}</span><button class="source-chip-remove" data-source-index="${index}" aria-label="Remove">&times;</button>`;
    el.sourceChipsContainer.appendChild(chip);
  });

  persistSourceRootEnabled(state.sourceRootEnabled);
  if (el.importMasterDbBtn) {
    el.importMasterDbBtn.classList.toggle("hidden", !state.externalMasterDbPath);
  }
  updateScanLibraryButtonLabel();
  updateSourceFilterIndicator();
}
export function renderCurrentPlaylistTracksFromState(state, el, deps = {}) {
  const {
    getCurrentPlaylist = () => null,
    filterTracksByQuery = (tracks) => tracks,
    renderEmptyState = () => {},
    applySortToTracks = (tracks) => tracks,
    renderTrackTable = () => {},
    cssEscape = (value) => String(value || ""),
    updateTrackListDurationSummary = () => {}
  } = deps;

  const playlist = getCurrentPlaylist();
  if (!playlist) return;
  state.currentPlaylistTracksView = filterTracksByQuery(playlist.tracks, state.playlistTrackSearch);

  const playlistEmpty = !playlist.tracks.length;
  if (el.playlistEmptyState) {
    el.playlistEmptyState.innerHTML = "";
    if (playlistEmpty) {
      renderEmptyState(el.playlistEmptyState, {
        icon: "♫",
        heading: "Browse Library or USB to add tracks"
      });
    }
  }
  if (el.playlistTableWrap) {
    el.playlistTableWrap.classList.toggle("hidden", playlistEmpty);
  }
  el.playlistSearchInput?.closest(".search-row")?.classList.toggle("hidden", playlistEmpty);
  el.playlistTotalDuration?.classList.toggle("hidden", playlistEmpty);
  el.exportPlaylistBtn?.closest(".playlist-actions")?.classList.toggle("hidden", playlistEmpty);

  const sortedPlaylist = applySortToTracks(state.currentPlaylistTracksView, "playlistTracksBody");
  renderTrackTable(el.playlistTracksBody, sortedPlaylist, {
    withCheckbox: false,
    origin: "local",
    secondaryActionLabel: "Play",
    secondaryActionType: "play-library",
    enableAnalyzeActions: false,
    actionLabel: "×",
    actionType: "remove-playlist-track",
    compactAddButton: true
  });
  for (const id of state.analyzingTrackIds) {
    const selector = `.track-grid-row[data-track-id="${cssEscape(id)}"][data-track-origin="local"]`;
    const row = el.playlistTracksBody.querySelector(selector);
    if (row) row.classList.add("is-analyzing");
  }
  updateTrackListDurationSummary(el.playlistTotalDuration, state.currentPlaylistTracksView);
}

export function updateLibraryDurationSummary(el, tracks, deps = {}) {
  const {
    trackHasCoreAnalysis = () => false,
    updateTrackListDurationSummary = () => {}
  } = deps;
  const summaryTracks = Array.isArray(tracks)
    ? tracks.map((track) => {
        const hasDuration = Number.isFinite(Number(track?.durationMs)) && Number(track?.durationMs) > 0;
        const countable = trackHasCoreAnalysis(track) || (track?.masterDbSource && hasDuration);
        return countable ? track : { ...track, durationMs: null };
      })
    : [];
  updateTrackListDurationSummary(el.libraryTotalDuration, summaryTracks);
}

export function renderLibraryRows(state, el, deps = {}) {
  const {
    getLibraryVisibleTracks = () => [],
    renderEmptyState = () => {},
    syncLibraryOnboardingMode = () => {},
    applySortToTracks = (tracks) => tracks,
    renderTrackTable = () => {},
    cssEscape = (value) => String(value || ""),
    updateLibraryDurationSummary = () => {}
  } = deps;

  const noSources = !state.sourcesEverConfigured;
  const visibleTracks = getLibraryVisibleTracks();
  const empty = noSources;

  if (el.libraryEmptyState) {
    el.libraryEmptyState.innerHTML = "";
    if (noSources) {
      const extraActions = state.externalMasterDbPath
        ? [{ label: "RB master.db", onAction: () => deps.onEnableMasterDb?.() }]
        : [];
      renderEmptyState(el.libraryEmptyState, {
        icon: "♫",
        heading: "Add a music folder to get started",
        actionLabel: "Add Folder",
        onAction: () => el.addSourceBtn?.click(),
        extraActions
      });
    }
  }
  if (el.libraryContent) {
    el.libraryContent.classList.toggle("hidden", empty);
  }
  syncLibraryOnboardingMode();

  const sortedLibrary = applySortToTracks(visibleTracks, "libraryTableBody");
  renderTrackTable(el.libraryTableBody, sortedLibrary, {
    withCheckbox: true,
    selectedIds: state.selectedTrackIds,
    actionLabel: "+",
    actionType: "add-library",
    compactAddButton: true,
    enableAnalyzeActions: true,
    origin: "local",
    secondaryActionLabel: "Play",
    secondaryActionType: "play-library"
  });
  for (const id of state.analyzingTrackIds) {
    const selector = `.track-grid-row[data-track-id="${cssEscape(id)}"][data-track-origin="local"]`;
    const row = el.libraryTableBody.querySelector(selector);
    if (row) row.classList.add("is-analyzing");
  }
  updateLibraryDurationSummary(visibleTracks);
}
// Library loading and analysis workflows extracted from main.js.

export async function loadTracks(state, query, limit, cursor, options = {}, deps) {
  const {
    command,
    normalizeTrack,
    readLibraryPagination,
    renderSourceChips,
    applySearchLocalFilter,
    hydrateLoadedTracksPreviewsInBackground
  } = deps;
  const requestSeq = Number(options.requestSeq || 0);
  const append = options.append === true;
  const previousById = new Map(
    (state.tracks || []).map((track) => [String(track.id), track])
  );
  const trimmed = String(query || "").trim();
  state.libraryLoading = true;
  try {
    const enabledRoots = (state.sourceRoots || []).filter(
      (root) => state.sourceRootEnabled?.[root] !== false
    );
    const includeMasterDb = state.masterDbEnabled === true;
    const hasEnabledSources = enabledRoots.length > 0 || includeMasterDb;
    const data = hasEnabledSources
      ? await command("browse_source_files", {
        sourceRoots: enabledRoots,
        includeMasterDb,
        query: trimmed,
        limit,
        cursor
      })
      : { total: 0, items: [], nextCursor: null, hasMore: false };
    const rawItems = data.items || [];

    if (requestSeq && requestSeq !== state.libraryRequestSeq) {
      return;
    }
    const normalizedItems = rawItems.map((t) => {
      const normalized = normalizeTrack(t, "lib");
      const prev = previousById.get(String(normalized.id));
      return prev ? mergeTrackPreservingBestFields(prev, normalized) : normalized;
    });
    if (append) {
      const mergedById = new Map(
        (state.tracks || []).map((track) => [String(track.id), track])
      );
      for (const track of normalizedItems) {
        mergedById.set(String(track.id), track);
      }
      state.tracks = Array.from(mergedById.values());
    } else {
      state.tracks = normalizedItems;
    }
    state.libraryQuery = trimmed;
    state.libraryLoadedTotal = Number(data.total || state.tracks.length || 0);
    const paging = readLibraryPagination(data);
    state.libraryNextCursor = paging.nextCursor;
    state.libraryHasMore = paging.hasMore;
    renderSourceChips();
    applySearchLocalFilter();
    void hydrateLoadedTracksPreviewsInBackground();
  } finally {
    if (!requestSeq || requestSeq === state.libraryRequestSeq) {
      state.libraryLoading = false;
    }
  }
}

export async function resetAndLoadLibraryTracks(state, query, limit, deps) {
  const { renderLibraryRows, loadTracks, ensureLibraryContainerFilled } = deps;
  state.libraryRequestSeq += 1;
  const requestSeq = state.libraryRequestSeq;
  state.libraryQuery = String(query || "").trim();
  state.libraryLoadedTotal = 0;
  state.libraryNextCursor = null;
  state.libraryHasMore = false;
  state.libraryLoading = true;
  const prevPreviews = new Map(
    (state.tracks || []).map((t) => [String(t.id || ""), { p: t.waveformPreview, c: t.waveformColorData }])
  );
  state.tracks = [];
  state.filteredTracks = [];
  renderLibraryRows();
  await loadTracks(state.libraryQuery, limit, null, { append: false, requestSeq });
  for (const track of state.tracks) {
    const id = String(track.id || "");
    const prev = prevPreviews.get(id);
    if (prev) {
      if ((!Array.isArray(track.waveformPreview) || track.waveformPreview.length === 0)
        && Array.isArray(prev.p) && prev.p.length > 0) {
        track.waveformPreview = prev.p;
      }
      if ((!Array.isArray(track.waveformColorData) || track.waveformColorData.length === 0)
        && Array.isArray(prev.c) && prev.c.length > 0) {
        track.waveformColorData = prev.c;
      }
    }
  }
  await ensureLibraryContainerFilled(limit);
}

export async function loadMoreLibraryTracks(state, limit, deps) {
  const { loadTracks } = deps;
  if (state.libraryLoading || !state.libraryHasMore) return;
  await loadTracks(state.libraryQuery, limit, state.libraryNextCursor, {
    append: true,
    requestSeq: state.libraryRequestSeq
  });
}

export async function ensureLibraryContainerFilled(state, el, limit, deps) {
  const { loadMoreLibraryTracks, LIBRARY_AUTOFILL_MAX_PAGES } = deps;
  const wrap = el.libraryTableWrap;
  if (!wrap) return;
  let guard = 0;
  while (
    state.libraryHasMore
    && !state.libraryLoading
    && wrap.scrollHeight <= wrap.clientHeight + 4
    && guard < LIBRARY_AUTOFILL_MAX_PAGES
  ) {
    await loadMoreLibraryTracks(limit);
    guard += 1;
  }
}

export async function refreshLoadedLibraryTracksFromBackend(state, deps) {
  const {
    LIBRARY_LOAD_LIMIT_DEFAULT,
    resetAndLoadLibraryTracks,
    loadMoreLibraryTracks
  } = deps;
  if (!state.sourceRoots.length && !state.masterDbEnabled) return;
  const currentCount = Number(state.tracks?.length || 0);
  await resetAndLoadLibraryTracks(state.libraryQuery, LIBRARY_LOAD_LIMIT_DEFAULT);
  while (state.tracks.length < currentCount && state.libraryHasMore) {
    await loadMoreLibraryTracks(LIBRARY_LOAD_LIMIT_DEFAULT);
  }
}

export function handleLibraryTableWrapScroll(state, el, deps) {
  const { LIBRARY_SCROLL_FETCH_THRESHOLD_PX, LIBRARY_LOAD_LIMIT_DEFAULT, loadMoreLibraryTracks, setStatus } = deps;
  const emitStatus = resolveEmitStatus(deps);
  const wrap = el.libraryTableWrap;
  if (!wrap || state.libraryLoading || !state.libraryHasMore) return;
  const remaining = wrap.scrollHeight - wrap.scrollTop - wrap.clientHeight;
  if (remaining > LIBRARY_SCROLL_FETCH_THRESHOLD_PX) return;
  loadMoreLibraryTracks(LIBRARY_LOAD_LIMIT_DEFAULT).catch((err) => {
    console.error(err);
    emitStatus(err.message || String(err));
  });
}

export function handleWindowLibraryScroll(state, el, windowObj, deps) {
  const { LIBRARY_SCROLL_FETCH_THRESHOLD_PX, LIBRARY_LOAD_LIMIT_DEFAULT, loadMoreLibraryTracks, setStatus } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (state.activeTab !== "library") return;
  if (state.libraryLoading || !state.libraryHasMore) return;
  const wrap = el.libraryTableWrap;
  if (!wrap) return;
  const rect = wrap.getBoundingClientRect();
  const remaining = rect.bottom - windowObj.innerHeight;
  if (remaining > LIBRARY_SCROLL_FETCH_THRESHOLD_PX) return;
  loadMoreLibraryTracks(LIBRARY_LOAD_LIMIT_DEFAULT).catch((err) => {
    console.error(err);
    emitStatus(err.message || String(err));
  });
}

export async function scanLibrary(state, deps) {
  const {
    setStatus,
    command,
    persistSourceRoots,
    resetAndLoadLibraryTracks,
    LIBRARY_LOAD_LIMIT_POST_SCAN,
    trackPathIsInsideSelectedRoots,
    trackHasCoreAnalysis,
    analyzeTrackIds,
    refreshCurrentPlaylistTracks,
    countWarningsForStatus
  } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (!state.sourceRoots.length) {
    emitStatus("Set at least one source root path before scanning");
    return;
  }

  persistSourceRoots(state.sourceRoots);
  emitStatus("Scanning library files...");
  const activeScanRoots = enabledSourceRoots(state.sourceRoots, state.sourceRootEnabled);
  const result = await command("scan_library", {
    sourceRoots: activeScanRoots,
    incremental: true
  });

  await resetAndLoadLibraryTracks("", LIBRARY_LOAD_LIMIT_POST_SCAN);
  const scopedTracks = state.tracks
    .filter((track) => trackPathIsInsideSelectedRoots(track.filePath));
  const albumCount = new Set(
    scopedTracks
      .map((track) => String(track.album || "").trim().toLowerCase())
      .filter(Boolean)
  ).size;
  const pendingTrackIds = scopedTracks
    .filter((track) => !(state.masterDbEnabled && track.masterDbSource) && !trackHasCoreAnalysis(track))
    .map((track) => track.id)
    .filter(Boolean);

  emitStatus(
    `Library scan: ${scopedTracks.length} tracks across ${albumCount} albums (local DB rows indexed ${result.indexed}, updated ${result.updated}, removed ${result.removed}). Resolving waveform/BPM/key...`
  );

  const analysis = pendingTrackIds.length > 0
    ? await analyzeTrackIds(pendingTrackIds, "Scan analysis", { pieceMode: "missing", batchMode: false })
    : { analyzed: 0, failed: 0, warnings: [] };
  const analyzed = Number(analysis?.analyzed || 0);
  const failed = Number(analysis?.failed || 0);
  const warnings = Array.isArray(analysis?.warnings) ? analysis.warnings : [];
  await refreshCurrentPlaylistTracks();

  const warningCount = countWarningsForStatus(warnings);
  const warningSuffix = warningCount ? ` (${warningCount} warning(s))` : "";
  const autoLimitWarning = findAnalysisAutoLimitWarning(warnings);
  const autoLimitSuffix = autoLimitWarning ? ` | ${autoLimitWarning}` : "";
  emitStatus(
    `Scan done: ${scopedTracks.length} tracks / ${albumCount} albums | analyzed ${analyzed}, failed ${failed}${warningSuffix}${autoLimitSuffix}`
  );
}

export async function scanMasterDb(state, deps) {
  const {
    command,
    resetAndLoadLibraryTracks,
    LIBRARY_LOAD_LIMIT_POST_SCAN,
    refreshCurrentPlaylistTracks,
    persistMasterDbEnabled,
    persistSourcesEverConfigured,
    renderSourceChips,
    logWarnings,
  } = deps;
  const emitStatus = resolveEmitStatus(deps);
  const path = state.externalMasterDbPath || undefined;

  emitStatus("Importing from desktop library...");
  let result;
  try {
    result = await command("scan_master_db", { path });
  } catch (err) {
    emitStatus(`Desktop library import failed: ${err?.message || err}`);
    return;
  }

  // Mark master.db as enabled and configured so loadTracks includes it in the
  // same browse path used for folder sources.
  state.masterDbEnabled = true;
  state.sourcesEverConfigured = true;
  persistMasterDbEnabled?.(true);
  persistSourcesEverConfigured?.(true);
  renderSourceChips?.();

  await resetAndLoadLibraryTracks("", LIBRARY_LOAD_LIMIT_POST_SCAN);

  await refreshCurrentPlaylistTracks();

  const notFound = Array.isArray(result.notFound) ? result.notFound : [];
  if (notFound.length > 0) {
    logWarnings?.(
      "master.db",
      notFound.map((p) => ({ level: "warn", message: p, code: "master_db.file_not_found" })),
      "desktop library import"
    );
  }

  const scanWarnings = Array.isArray(result.warnings) ? result.warnings : [];
  if (scanWarnings.length > 0) {
    logWarnings?.(
      "master.db",
      scanWarnings.map((msg) => ({ level: "info", message: msg, code: "master_db.scan_diag" })),
      "desktop library import"
    );
  }

  const suffix = notFound.length > 0 ? ` | ${notFound.length} file(s) not found (see event log)` : "";
  emitStatus(`Desktop library import done: ${result.indexed} new, ${result.updated} updated${suffix}`);
}

export async function analyzeTrackIds(state, trackIds, modeLabel = "Analyze", options = {}, deps) {
  const {
    shouldUseBatchAnalysis,
    parseAnalysisBpmRange,
    command,
    setStatus,
    resolveMissingAnalysisPieces,
    setTrackAnalyzingState,
    applyRealtimeAnalyzedTrackUpdate,
    nextPaint,
    mergeHydratedTrackIntoState,
    hydrateTrackPreviewFromBackend,
    patchLibraryRowByTrackId,
    patchPlaylistRowByTrackId,
    updateLibraryDurationSummary,
    renderSourceChips,
    refreshCurrentPlaylistTracks,
    countWarningsForStatus,
    logWarnings
  } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (state.analysisEnginePersistPromise) {
    try {
      await state.analysisEnginePersistPromise;
    } catch {
      // If persistence fails, proceed with current backend setting.
    }
  }
  const ids = Array.isArray(trackIds) ? trackIds.filter(Boolean) : [];
  if (!ids.length) return;
  const fullPieces = ["duration", "artwork", "waveform", "bpm_key"];
  const pieceMode = String(options?.pieceMode || "full").toLowerCase();
  const useBatchMode = shouldUseBatchAnalysis(ids.length, options);
  const bpmRange = parseAnalysisBpmRange(state.analysisBpmRange);
  let workers = 1;
  try {
    const parallelism = await command("get_system_parallelism");
    const cores = Math.max(1, Number(parallelism?.workers || 1));
    workers = Math.max(1, cores - 2);
  } catch (_) {
    workers = 1;
  }

  let analyzed = 0;
  let failed = 0;
  const warnings = [];
  const successfulIds = new Set();
  let completed = 0;
  let nextIndex = 0;

  emitStatus(`${modeLabel}: 0/${ids.length} track(s) ready...`);

  async function processTrack(trackId) {
    const id = String(trackId || "").trim();
    if (!id) return false;
    const currentTrack = state.tracks.find((track) => String(track.id) === id) || null;
    const pieces = pieceMode === "missing"
      ? resolveMissingAnalysisPieces(currentTrack)
      : fullPieces.slice();
    if (!pieces.length) return true;
    if (currentTrack) {
      for (const piece of pieces) {
        if (piece === "bpm_key") {
          currentTrack.bpm = null;
          currentTrack.key = null;
          currentTrack.bpmAnalyzer = "";
        }
        if (piece === "waveform") { currentTrack.waveformPeaksPath = null; currentTrack.waveformPreview = null; }
        if (piece === "duration") { currentTrack.durationMs = null; }
      }
    }
    setTrackAnalyzingState(id, true);
    let bpmKeyObserved = null;
    for (const piece of pieces) {
      try {
        const trackReady = piece === pieces[pieces.length - 1];
        const pieceData = await command("analyze_track_piece", {
          trackId: id,
          piece,
          bpmMin: bpmRange.min,
          bpmMax: bpmRange.max,
          analysisEngine: state.analysisEngine
        });
        await applyRealtimeAnalyzedTrackUpdate({
          trackId: id,
          bpm: pieceData?.bpm ?? null,
          bpmAnalyzer: pieceData?.bpmAnalyzer ?? null,
          key: pieceData?.key ?? null,
          durationMs: pieceData?.durationMs ?? null,
          artworkPath: pieceData?.artworkPath ?? null,
          waveformPeaksPath: pieceData?.waveformPeaksPath ?? null,
          waveformPreview: pieceData?.waveformPreview ?? null,
          artworkChecked: piece === "artwork",
          trackReady
        });
        if (piece === "bpm_key") {
          bpmKeyObserved = {
            bpm: pieceData?.bpm ?? null,
            key: pieceData?.key ?? null
          };
        }
        await nextPaint();
      } catch (err) {
        const engineLabel = String(state.analysisEngine || "stratum");
        warnings.push(`${id} ${piece} (${engineLabel}): ${err.message || err}`);
        setTrackAnalyzingState(id, false);
        return false;
      }
    }
    if (pieces.includes("bpm_key")) {
      const bpmNum = Number(bpmKeyObserved?.bpm);
      const hasBpm = Number.isFinite(bpmNum) && bpmNum > 0;
      const hasKey = typeof bpmKeyObserved?.key === "string" && bpmKeyObserved.key.trim().length > 0;
      if (!hasBpm && !hasKey) {
        const engineLabel = String(state.analysisEngine || "stratum");
        warnings.push(`${id} bpm_key (${engineLabel}): no BPM/key result`);
        setTrackAnalyzingState(id, false);
        return false;
      }
    }
    const hydratedTrack = state.tracks.find((track) => String(track.id) === id) || null;
    if (hydratedTrack) {
      const needsPreviewHydration = !Array.isArray(hydratedTrack.waveformPreview)
        || hydratedTrack.waveformPreview.length === 0;
      const hasWaveformPath = typeof hydratedTrack.waveformPeaksPath === "string"
        && hydratedTrack.waveformPeaksPath.trim().length > 0;
      if (needsPreviewHydration && hasWaveformPath) {
        await hydrateTrackPreviewFromBackend(id, { updateSummary: false });
      }
    }
    setTrackAnalyzingState(id, false);
    return true;
  }

  async function workerLoop() {
    while (true) {
      const idx = nextIndex;
      nextIndex += 1;
      if (idx >= ids.length) return;
      const ok = await processTrack(ids[idx]);
      if (ok) {
        analyzed += 1;
        successfulIds.add(String(ids[idx] || "").trim());
      } else {
        failed += 1;
      }
      completed += 1;
      if (ok && ids.length > 1 && completed < ids.length) {
        updateLibraryDurationSummary();
        renderSourceChips();
      }
      emitStatus(`${modeLabel}: ${completed}/${ids.length} track(s) ready...`);
    }
  }

  if (useBatchMode) {
    for (const id of ids) {
      setTrackAnalyzingState(String(id), true);
    }
    try {
      const batch = await command("analyze_new_tracks", {
        trackIds: ids,
        analysisEngine: state.analysisEngine
      });
      analyzed = Math.max(0, Number(batch?.analyzed || 0));
      failed = Math.max(0, Number(batch?.failed || 0));
      const batchWarnings = Array.isArray(batch?.warnings) ? batch.warnings : [];
      warnings.push(...batchWarnings.map((w) => String(w)));
      completed = ids.length;
      emitStatus(`${modeLabel}: ${completed}/${ids.length} track(s) ready...`);
    } catch (err) {
      warnings.push(`batch analysis failed; falling back to piece mode: ${err.message || err}`);
      const activeWorkers = Math.max(1, Math.min(workers, ids.length));
      await Promise.all(Array.from({ length: activeWorkers }, () => workerLoop()));
    } finally {
      for (const id of ids) {
        setTrackAnalyzingState(String(id), false);
      }
    }
  } else {
    const activeWorkers = Math.max(1, Math.min(workers, ids.length));
    await Promise.all(Array.from({ length: activeWorkers }, () => workerLoop()));
  }
  try {
    const hydrateIds = useBatchMode
      ? ids
      : ids.filter((id) => successfulIds.has(String(id || "").trim()));
    const hydrated = await command("get_tracks_by_ids_with_previews", { trackIds: hydrateIds });
    const changedIds = [];
    for (const item of hydrated?.items || []) {
      const changed = mergeHydratedTrackIntoState(item);
      if (changed) {
        const id = String(item?.id || "").trim();
        if (id) changedIds.push(id);
      }
    }
    if (changedIds.length) {
      if (ids.length === 1) {
        await nextPaint();
        await nextPaint();
      }
      for (const id of changedIds) {
        patchLibraryRowByTrackId(id);
        patchPlaylistRowByTrackId(id);
      }
      updateLibraryDurationSummary();
      renderSourceChips();
    }
  } catch (_) {
    // final sync pass only
  }

  await refreshCurrentPlaylistTracks();
  if (typeof logWarnings === "function" && warnings.length) {
    logWarnings("analysis", warnings, modeLabel);
  }
  const warningCount = countWarningsForStatus(warnings);
  const warningSuffix = warningCount ? ` (${warningCount} warning(s))` : "";
  const autoLimitWarning = findAnalysisAutoLimitWarning(warnings);
  const autoLimitSuffix = autoLimitWarning ? ` | ${autoLimitWarning}` : "";
  emitStatus(`${modeLabel} done: analyzed ${analyzed}, failed ${failed}${warningSuffix}${autoLimitSuffix}`);
  return { analyzed, failed, warnings };
}

export async function analyzeSingleTrack(state, track, modeLabel = null, deps) {
  const {
    resolveLocalTrackId,
    resolveLocalTrackIdAsync,
    setStatus,
    trackHasCoreAnalysis,
    analyzeTrackIds
  } = deps;
  const emitStatus = resolveEmitStatus(deps);
  let localId = resolveLocalTrackId(track);
  if (!localId) {
    localId = await resolveLocalTrackIdAsync(track);
  }
  if (!localId) {
    emitStatus("Track is not in local library yet. Scan library first, then analyze.");
    return;
  }
  const localTrack = state.tracks.find((t) => t.id === localId) || track;
  const label = modeLabel || (trackHasCoreAnalysis(localTrack) ? "Reanalyze" : "Analyze missing");
  await analyzeTrackIds([localId], label);
}

export function scheduleApplySearchLocalFilter(state, el, deps = {}) {
  const {
    clearTimeoutFn = (id) => clearTimeout(id),
    setTimeoutFn = (cb, ms) => setTimeout(cb, ms),
    resetAndLoadLibraryTracks = async () => {},
    setStatus = () => {},
    logError = () => {},
    debounceMs = 180
  } = deps;
  const emitStatus = resolveEmitStatus(deps);

  if (state.librarySearchDebounceTimer) {
    clearTimeoutFn(state.librarySearchDebounceTimer);
  }
  state.librarySearchDebounceTimer = setTimeoutFn(() => {
    state.librarySearchDebounceTimer = null;
    resetAndLoadLibraryTracks(el.librarySearch?.value || "").catch((err) => {
      logError(err);
      emitStatus(err.message || String(err));
    });
  }, debounceMs);
}

export function patchLibraryRowByTrackId(state, el, trackId, deps) {
  const {
    cssEscape,
    patchLibraryRowCells
  } = deps;
  const id = String(trackId || "").trim();
  if (!id) return false;
  const selector = `.track-grid-row[data-track-id="${cssEscape(id)}"][data-track-origin="local"]`;
  const row = el.libraryTableBody?.querySelector(selector);
  if (!row) return false;
  const track = state.tracks.find((t) => String(t.id) === id);
  if (!track) return false;
  const patched = patchLibraryRowCells(row, track);
  const analyzing = state.analyzingTrackIds.has(id);
  row.classList.toggle("is-analyzing", analyzing);
  const analyzeBtn = row.querySelector("[data-action='analyze-track']");
  if (analyzeBtn) analyzeBtn.disabled = analyzing;
  return patched;
}

export function patchPlaylistRowByTrackId(state, el, trackId, deps) {
  const {
    cssEscape,
    getCurrentPlaylist,
    patchLibraryRowCells
  } = deps;
  const id = String(trackId || "").trim();
  if (!id) return false;
  const selector = `.track-grid-row[data-track-id="${cssEscape(id)}"][data-track-origin="local"]`;
  const row = el.playlistTracksBody?.querySelector(selector);
  if (!row) return false;
  const playlist = getCurrentPlaylist();
  const track = (playlist?.tracks || []).find((t) => {
    const localTrackId = String(t?.localTrackId || t?.id || "").trim();
    return localTrackId === id;
  });
  if (!track) return false;
  const patched = patchLibraryRowCells(row, track);
  const analyzing = state.analyzingTrackIds.has(id);
  row.classList.toggle("is-analyzing", analyzing);
  return patched;
}

export function setTrackAnalyzingState(state, trackId, active, deps) {
  const {
    patchLibraryRowByTrackId,
    patchPlaylistRowByTrackId,
    trackHasCoreAnalysis,
    trackNeedsPreviewHydration,
    getLibraryVisibleTracks,
    updateLibraryDurationSummary,
    renderSourceChips
  } = deps;
  const id = String(trackId || "").trim();
  if (!id) return;
  if (active) state.analyzingTrackIds.add(id);
  else state.analyzingTrackIds.delete(id);
  patchLibraryRowByTrackId(id);
  patchPlaylistRowByTrackId(id);
  if (!active && state.analyzingTrackIds.size > 0) {
    const localTrack = state.tracks.find((track) => String(track?.id || "") === id);
    if (localTrack && trackHasCoreAnalysis(localTrack) && !trackNeedsPreviewHydration(localTrack)) {
      const visibleTracks = getLibraryVisibleTracks();
      updateLibraryDurationSummary(visibleTracks);
      renderSourceChips();
    }
  }
}

export function promoteTrackIdentity(state, el, oldId, newId, deps) {
  const { cssEscape } = deps;
  const fromId = String(oldId || "").trim();
  const toId = String(newId || "").trim();
  if (!fromId || !toId || fromId === toId) return;

  for (const track of state.tracks) {
    if (String(track?.id || "") !== fromId) continue;
    track.id = toId;
    track.localTrackId = toId;
  }

  if (state.selectedTrackIds.has(fromId)) {
    state.selectedTrackIds.delete(fromId);
    state.selectedTrackIds.add(toId);
  }

  for (const playlist of state.playlists) {
    for (const track of playlist.tracks || []) {
      const localTrackId = String(track?.localTrackId || track?.id || "").trim();
      if (localTrackId !== fromId) continue;
      track.localTrackId = toId;
      if (String(track.id || "") === fromId) {
        track.id = toId;
      }
    }
  }

  const row = el.libraryTableBody?.querySelector(
    `.track-grid-row[data-track-id="${cssEscape(fromId)}"][data-track-origin="local"]`
  );
  if (row) {
    row.dataset.trackId = toId;
    row.querySelectorAll("[data-id]").forEach((node) => {
      if (String(node.dataset.id || "") === fromId) {
        node.dataset.id = toId;
      }
    });
  }
}

// --- analysis_patch.mjs ---

export function parseProgressWaveformPreview(value) {
  if (!Array.isArray(value)) return null;
  return value
    .map((v) => Math.max(0, Math.min(100, Number(v) || 0)))
    .filter((v) => Number.isFinite(v));
}

export function patchTrackAnalysisFields(track, payload, deps) {
  if (!track || !payload || typeof payload !== "object") return false;
  const { toPlayableUrl } = deps;
  let changed = false;
  const setIfChanged = (key, next) => {
    if (next === undefined) return;
    if (track[key] === next) return;
    track[key] = next;
    changed = true;
  };
  const bpm = Number(payload.bpm);
  if (payload.bpm !== undefined && payload.bpm !== null && Number.isFinite(bpm) && bpm > 0) {
    setIfChanged("bpm", bpm.toFixed(2).replace(/\.00$/, ""));
  }
  if (typeof payload.bpmAnalyzer === "string" && payload.bpmAnalyzer.trim()) {
    setIfChanged("bpmAnalyzer", payload.bpmAnalyzer.trim());
  }
  if (typeof payload.key === "string" && payload.key.trim()) {
    setIfChanged("key", payload.key.trim());
  }
  if (typeof payload.filePath === "string" && payload.filePath.trim()) {
    setIfChanged("filePath", payload.filePath.trim());
  }
  if (typeof payload.artworkPath === "string" && payload.artworkPath.trim()) {
    const artworkPath = payload.artworkPath.trim();
    setIfChanged("artworkPath", artworkPath);
    setIfChanged("artworkUrl", appendUrlRevision(toPlayableUrl(artworkPath) || "", payload.updatedAt || Date.now()));
    setIfChanged("artworkChecked", true);
  } else if (payload.artworkChecked === true) {
    setIfChanged("artworkChecked", true);
  }
  if (typeof payload.waveformPeaksPath === "string" && payload.waveformPeaksPath.trim()) {
    setIfChanged("waveformPeaksPath", payload.waveformPeaksPath.trim());
  }
  const durationMs = Number(payload.durationMs);
  if (payload.durationMs !== undefined && payload.durationMs !== null
      && Number.isFinite(durationMs) && durationMs > 0) {
    setIfChanged("durationMs", Math.round(durationMs));
  }
  const waveformPreview = parseProgressWaveformPreview(payload.waveformPreview);
  if (waveformPreview && waveformPreview.length) {
    const samePreview = Array.isArray(track.waveformPreview)
      && track.waveformPreview.length === waveformPreview.length
      && track.waveformPreview.every((value, index) => Number(value) === Number(waveformPreview[index]));
    if (!samePreview) {
      track.waveformPreview = waveformPreview;
      changed = true;
    }
  }
  return changed;
}

export function patchLibraryRowCells(row, track, deps) {
  if (!row || !track) return false;
  const { escapeHtml, getKeyHue, buildCoverSrcCandidates, attachCoverFallbackHandlers, drawWaveformCanvas, trackHasCoreAnalysis, invalidateWaveformCache, setWaveformColorData } = deps;

  const cells = row.querySelectorAll('[role="cell"]');
  if (cells.length < 3) return false;

  const durationTd = row.querySelector(".td-length");
  if (durationTd) {
    durationTd.textContent = formatDurationMsInternal(track.durationMs);
  }

  const bpmTd = row.querySelector(".td-bpm");
  if (bpmTd) {
    const bpmTitle = track.bpmAnalyzer ? ` title="${escapeHtml(`Analyzed with: ${track.bpmAnalyzer}`)}"` : "";
    bpmTd.innerHTML = track.bpm
      ? `<span class="bpm-pill"${bpmTitle}>${escapeHtml(track.bpm)}</span>`
      : "-";
  }

  const keyTd = row.querySelector(".td-key");
  if (keyTd) {
    const keyHue = getKeyHue(track.key);
    const keyHueClass = `key-pill--h${((Math.round(Number(keyHue) / 30) % 12) + 12) % 12}`;
    keyTd.innerHTML = track.key
      ? `<span class="key-pill ${keyHueClass}">${escapeHtml(track.key)}</span>`
      : "-";
  }

  const coverTd = row.querySelector(".td-cover");
  if (coverTd) {
    const coverCandidates = buildCoverSrcCandidates(track);
    if (coverCandidates.length) {
      const img = coverTd.querySelector("img.cover-thumb");
      if (img) {
        img.src = coverCandidates[0];
        img.dataset.fallbacks = coverCandidates.slice(1).join("|");
      } else {
        coverTd.innerHTML = `<img class="cover-thumb" alt="cover" loading="lazy" src="${escapeHtml(coverCandidates[0])}" data-fallbacks="${escapeHtml(coverCandidates.slice(1).join("|"))}" />`;
        attachCoverFallbackHandlers(coverTd);
      }
    } else if (coverTd.querySelector("img.cover-thumb")) {
      coverTd.innerHTML = `<div class="cover-thumb" aria-hidden="true"></div>`;
    }
  }

  const waveformTd = row.querySelector(".td-waveform");
  if (waveformTd) {
    const waveformDiv = waveformTd.querySelector(".waveform");
    if (waveformDiv) {
      const colorData = Array.isArray(track.waveformColorData) && track.waveformColorData.length >= 6
        ? track.waveformColorData : null;
      const peaks = Array.isArray(track.waveformPreview)
        ? track.waveformPreview.map((v) => Math.max(0, Math.min(100, Number(v) || 0))).filter((v) => Number.isFinite(v))
        : [];
      const hasRenderableWaveform = colorData !== null || (peaks.length > 0 && peaks.some((v) => v > 0));
      if (hasRenderableWaveform) {
        if (invalidateWaveformCache) invalidateWaveformCache(waveformDiv);
        if (colorData) {
          if (setWaveformColorData) setWaveformColorData(waveformDiv, colorData);
          delete waveformDiv.dataset.peaks;
        } else {
          waveformDiv.dataset.peaks = peaks.join(",");
        }
        if (!waveformDiv.classList.contains("waveform-canvas")) {
          waveformDiv.classList.add("waveform-canvas");
          waveformDiv.insertAdjacentHTML("afterbegin", `<canvas class="waveform-canvas-el" aria-hidden="true"></canvas>`);
        }
        drawWaveformCanvas(waveformDiv);
      } else {
        delete waveformDiv.dataset.peaks;
        if (setWaveformColorData) setWaveformColorData(waveformDiv, null);
        if (invalidateWaveformCache) invalidateWaveformCache(waveformDiv);
        waveformDiv.classList.remove("waveform-canvas");
        const canvas = waveformDiv.querySelector("canvas");
        if (canvas) canvas.remove();
      }
    }
  }

  const actionTd = row.querySelector(".td-action");
  if (actionTd) {
    const analyzeBtn = actionTd.querySelector("[data-action='analyze-track']");
    if (analyzeBtn && trackHasCoreAnalysis(track)) {
      analyzeBtn.textContent = "Reanalyze";
      analyzeBtn.title = "Recompute waveform/BPM/key";
    }
  }

  return true;
}

export function createAnalysisPatchQueue() {
  const pending = new Set();
  let rafId = null;
  let patchFn = null;
  let fallbackFn = null;

  function flush() {
    rafId = null;
    const ids = Array.from(pending);
    pending.clear();
    let anyMissed = false;
    let patched = 0;
    for (const id of ids) {
      if (!patchFn(id)) {
        anyMissed = true;
      } else {
        patched++;
      }
    }
    if (ids.length) {
      console.log(`[analysis-ui] flush: ${patched}/${ids.length} rows patched, missed: ${anyMissed}`);
    }
    if (anyMissed && fallbackFn) {
      fallbackFn();
    }
  }

  return {
    get pending() { return pending; },
    get scheduled() { return rafId !== null; },

    init(patchRowById, scheduleFullRender, requestAnimationFrameFn) {
      patchFn = patchRowById;
      fallbackFn = scheduleFullRender;
      this._raf = requestAnimationFrameFn || globalThis.requestAnimationFrame?.bind(globalThis);
    },

    queue(trackId) {
      pending.add(trackId);
      if (rafId !== null) return;
      rafId = this._raf(flush);
    },

    flush,

    cancel() {
      pending.clear();
      rafId = null;
    }
  };
}

function formatDurationMsInternal(value) {
  const rawMs = Number(value);
  if (!Number.isFinite(rawMs) || rawMs <= 0) return "-";
  const ms = Math.max(0, Math.round(rawMs));
  const totalSeconds = Math.floor(ms / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

// --- analysis_settings.mjs ---

export const DEFAULT_ANALYSIS_BPM_RANGE = "70-180";

export const ANALYSIS_BPM_RANGE_PRESETS = [
  "70-180",
  "48-95",
  "58-115",
  "68-135",
  "78-155",
  "88-175",
  "98-195",
  "108-215",
  "118-235",
  "128-255"
];

export function parseAnalysisBpmRange(range) {
  const raw = String(range || "").trim();
  const m = /^(\d{1,3})\s*-\s*(\d{1,3})$/.exec(raw);
  if (!m) return parseAnalysisBpmRange(DEFAULT_ANALYSIS_BPM_RANGE);
  const min = Number(m[1]);
  const max = Number(m[2]);
  if (!Number.isFinite(min) || !Number.isFinite(max) || min <= 0 || min >= max) {
    return parseAnalysisBpmRange(DEFAULT_ANALYSIS_BPM_RANGE);
  }
  return { label: `${min}-${max}`, min, max };
}

export function normalizeAnalysisBpmRange(range) {
  const parsed = parseAnalysisBpmRange(range);
  return ANALYSIS_BPM_RANGE_PRESETS.includes(parsed.label)
    ? parsed.label
    : DEFAULT_ANALYSIS_BPM_RANGE;
}

// --- analysis_flow.mjs ---

export function shouldUseBatchAnalysis(trackCount, options = {}) {
  const count = Math.max(0, Number(trackCount) || 0);
  return options?.batchMode !== false && count > 1;
}

// --- library_pagination.mjs ---

export function readLibraryPagination(data) {
  const nextCursor = data?.nextCursor ?? data?.next_cursor ?? null;
  if (typeof data?.hasMore === "boolean") {
    return { nextCursor, hasMore: data.hasMore };
  }
  if (typeof data?.has_more === "boolean") {
    return { nextCursor, hasMore: data.has_more };
  }
  return { nextCursor, hasMore: !!nextCursor };
}

// --- source_root_filter.mjs ---

export function normalizePath(value) {
  return String(value || "").replace(/\\/g, "/").trim().toLowerCase();
}

export function trackPathMatchesAnyRoot(filePath, roots) {
  const fp = normalizePath(filePath);
  if (!fp) return false;
  return roots.some((root) => {
    const r = normalizePath(root).replace(/\/+$/, "");
    if (!r) return false;
    return fp === r || fp.startsWith(`${r}/`);
  });
}

export function enabledSourceRoots(sourceRoots, sourceRootEnabled = {}) {
  const roots = Array.isArray(sourceRoots) ? sourceRoots : [];
  return roots.filter((root) => sourceRootEnabled[root] !== false);
}

export function filterTracksBySourceRoots(tracks, sourceRoots, sourceRootEnabled = {}) {
  const list = Array.isArray(tracks) ? tracks : [];
  const enabled = enabledSourceRoots(sourceRoots, sourceRootEnabled);
  if (!enabled.length) return [];
  return list.filter((track) => trackPathMatchesAnyRoot(track?.filePath, enabled));
}

export function scanLibraryButtonLabel(sourceRoots) {
  const count = Array.isArray(sourceRoots) ? sourceRoots.length : 0;
  return count > 1 ? "Scan Libraries" : "Scan Library";
}

export function trackPathIsInsideSelectedRoots(filePath, sourceRoots) {
  return trackPathMatchesAnyRoot(filePath, sourceRoots);
}

// --- cover_url.mjs ---

export function convertFileSrcLocal(filePath) {
  const normalized = String(filePath || "").replace(/\\/g, "/").trim();
  if (!normalized) return null;
  if (/^(?:asset|tauri|https?|blob|data|file):/i.test(normalized)) return normalized;
  const encoded = normalized.split("/").map(encodeURIComponent).join("/");
  return `asset://localhost${encoded}`;
}

export function appendUrlRevision(url, revision) {
  const normalized = String(url || "").trim();
  if (!normalized) return "";
  if (normalized.startsWith("data:")) return normalized;
  const rev = String(revision || "").trim();
  if (!rev) return normalized;
  const joiner = normalized.includes("?") ? "&" : "?";
  return `${normalized}${joiner}rev=${encodeURIComponent(rev)}`;
}

export function buildCoverSrcCandidates(track, deps = {}) {
  const { toPlayableUrl } = deps;
  const out = [];
  const seen = new Set();
  const addPathVariants = (value) => {
    const raw = String(value || "").trim();
    if (!raw) return;
    if (/^(?:asset|tauri|https?|blob|data|file):/i.test(raw)) {
      push(raw);
      return;
    }
    if (typeof toPlayableUrl === "function") {
      push(toPlayableUrl(raw));
      const normalizedPath = raw.replace(/^file:\/\//i, "");
      if (normalizedPath && normalizedPath !== raw) {
        push(toPlayableUrl(normalizedPath));
      }
      if (raw.startsWith("/")) {
        push(toPlayableUrl(raw.replace(/^\/+/, "")));
      } else {
        push(toPlayableUrl(`/${raw}`));
      }
    }
    push(convertFileSrcLocal(raw));
  };
  const push = (value) => {
    const v = String(value || "").trim();
    if (!v || seen.has(v)) return;
    seen.add(v);
    out.push(v);
  };

  push(track?.artworkDataUrl);
  addPathVariants(track?.artworkPath);
  push(track?.artworkUrl);
  return out;
}

// --- cover_fallback.mjs ---

export function attachCoverFallbackHandlers(root = document, deps = {}) {
  const doc = deps.document || document;
  root.querySelectorAll("img.cover-thumb").forEach((img) => {
    if (img.dataset.fallbackBound === "1") return;
    img.dataset.fallbackBound = "1";
    img.addEventListener("error", () => {
      const queue = String(img.dataset.fallbacks || "")
        .split("|")
        .filter(Boolean);
      const next = queue.shift();
      if (next) {
        img.dataset.fallbacks = queue.join("|");
        img.src = next;
        return;
      }
      const placeholder = doc.createElement("div");
      placeholder.className = "cover-thumb";
      placeholder.setAttribute("aria-hidden", "true");
      img.replaceWith(placeholder);
    });
  });
}
