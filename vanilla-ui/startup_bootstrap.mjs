import {
  registerBackendJobEvents as registerBackendJobEventsCore,
  unregisterBackendJobEvents as unregisterBackendJobEventsCore
} from "./components/playback/actions.mjs";

export async function hydrateAppVersionLabel(el, deps = {}) {
  const {
    appVersionFallback = "0.1.0",
    tauriIsTauri = () => false,
    tauriGetVersion = async () => appVersionFallback
  } = deps;

  if (!el.settingsVersionText) return;
  let version = appVersionFallback;
  if (tauriIsTauri()) {
    try {
      const resolved = await tauriGetVersion();
      if (resolved && String(resolved).trim()) {
        version = String(resolved).trim();
      }
    } catch (_) {
      // Keep fallback in browser/test mode.
    }
  }
  el.settingsVersionText.textContent = `Version ${version}`;
}

export async function checkForUpdate(state, el, deps = {}) {
  const {
    resolveVersion = async () => null,
    fetchUpdateInfo = async () => null,
    renderUpdateNotice = () => {},
    renderCriticalUpdateBanner = () => {}
  } = deps;
  try {
    const version = await resolveVersion();
    if (!version) return;
    const info = await fetchUpdateInfo(version);
    if (!info) return;
    state.updateCheck = info;
    renderUpdateNotice(state, el);
    renderCriticalUpdateBanner(state, el);
  } catch {
    // An update check must never disrupt startup.
  }
}

export function restoreStoredUiPrefs(state, el, deps = {}) {
  const {
    localStorageObj = typeof localStorage !== "undefined" ? localStorage : null,
    constants,
    normalizeAnalysisBpmRange = (v) => v,
    defaultAnalysisBpmRange = "full"
  } = deps;

  try {
    const stored = localStorageObj?.getItem?.(constants.STORAGE_KEY_EXPORT_PRUNE_STALE);
    state.exportPruneStale = stored === null ? true : stored === "1";
  } catch {
    state.exportPruneStale = true;
  }
  if (el.exportSyncModeMirror && el.exportSyncModeAdditive) {
    el.exportSyncModeMirror.checked = !!state.exportPruneStale;
    el.exportSyncModeAdditive.checked = !state.exportPruneStale;
  }

  try {
    const stored = localStorageObj?.getItem?.(constants.STORAGE_KEY_EXPORT_BACKUP);
    state.exportBackup = stored === null ? true : stored === "1";
  } catch {
    state.exportBackup = true;
  }
  if (el.exportBackupCheckbox) {
    el.exportBackupCheckbox.checked = !!state.exportBackup;
  }

  try {
    const stored = localStorageObj?.getItem?.(constants.STORAGE_KEY_ANALYSIS_BPM_RANGE);
    state.analysisBpmRange = normalizeAnalysisBpmRange(stored || defaultAnalysisBpmRange);
  } catch {
    state.analysisBpmRange = defaultAnalysisBpmRange;
  }
  if (el.analysisBpmRangeSelect) {
    el.analysisBpmRangeSelect.value = state.analysisBpmRange;
  }

  try {
    const storedEngine = localStorageObj?.getItem?.(constants.STORAGE_KEY_ANALYSIS_ENGINE);
    state.analysisEngine = storedEngine === "essentia" ? "essentia" : "stratum";
  } catch {
    state.analysisEngine = "stratum";
  }
  if (el.analysisEngineSelect) {
    el.analysisEngineSelect.value = state.analysisEngine;
  }
  if (el.essentiaInstallRow) {
    const show = state.analysisEngine === "essentia";
    el.essentiaInstallRow.classList.toggle("hidden", !show);
  }

  try {
    state.sidebarCollapsed = localStorageObj?.getItem?.(constants.STORAGE_KEY_SIDEBAR_COLLAPSED) === "1";
  } catch {
    state.sidebarCollapsed = false;
  }
}

export function applySidebarCollapsedUi(state, el, deps = {}) {
  const { sidebarExpandBtn = null } = deps;
  if (state.sidebarCollapsed) {
    el.navSidebar.classList.add("collapsed");
    sidebarExpandBtn?.classList.add("visible");
  }
}

export function showHelpOnFirstVisit(el, deps = {}) {
  const {
    localStorageObj = typeof localStorage !== "undefined" ? localStorage : null,
    storageKeyHelpSeen = "helpSeen"
  } = deps;
  try {
    if (!localStorageObj?.getItem?.(storageKeyHelpSeen) && el.helpOverlay) {
      el.helpOverlay.classList.remove("hidden");
    }
  } catch {}
}

export function runDeferredInitialLoad(state, deps = {}) {
  const {
    setTimeoutFn = (cb) => setTimeout(cb, 0),
    withProgress = async (_label, fn) => fn(() => {}),
    loadPlaylists = async () => {},
    resetAndLoadLibraryTracks = async () => {},
    libraryLoadLimitInit = 200,
    updateModeText = () => {},
    updateSelectionCount = () => {},
    renderUsbPlaylistTracks = () => {},
    renderWaveformsIn = () => {},
    documentObj = typeof document !== "undefined" ? document : null,
    setStatus = () => {},
    logError = () => {}
  } = deps;

  setTimeoutFn(() => {
    withProgress("Initializing", async (progress) => {
      progress(35, "Loading playlists...");
      await loadPlaylists();
      progress(70, "Loading tracks...");
      await resetAndLoadLibraryTracks("", libraryLoadLimitInit);

      if (state.playlists.length > 0) {
        const hasCurrent = state.playlists.some((playlist) => playlist.id === state.currentPlaylistId);
        if (!hasCurrent) {
          state.currentPlaylistId = state.playlists[0].id;
        }
      }
      updateModeText();
      updateSelectionCount();
      renderUsbPlaylistTracks();
      renderWaveformsIn(documentObj);
    }).then(() => {
      state.startupPhase = false;
    }).catch((error) => {
      state.startupPhase = false;
      logError(error);
      setStatus(`Initialization failed: ${error.message}`);
    });
  }, 0);
}

export async function initApp(state, deps = {}) {
  const {
    el,
    constants,
    hydrateLocalStorageFromFrontendSettingsDb,
    themeInit,
    accentInit,
    hydrateAppVersionLabel,
    checkForUpdate = () => {},
    setupConsoleFileLogging,
    setupRuntimeErrorLogging,
    pushEventLog,
    setProgress,
    loadSourceRootsFromStorage,
    loadSourceRootEnabledFromStorage,
    loadMasterDbEnabledFromStorage,
    loadSourcesEverConfiguredFromStorage,
    loadUsbRecentRootsFromStorage,
    renderUsbRecentRoots,
    persistSourceRootEnabled,
    syncAssetScopePaths,
    loadUsbRootFromStorage,
    restoreStoredUiPrefs,
    applySidebarCollapsedUi,
    checkSourceRoots = async () => {},
    renderSourceChips,
    detectExternalMasterDb,
    bindEvents,
    switchView,
    showHelpOnFirstVisit,
    invoke,
    registerBackendJobEvents,
    handleBackendLogEvent,
    updateUsbRootText,
    runDeferredInitialLoad,
    logInfo = () => {},
    logError = () => {},
    warn = () => {}
  } = deps;

  pushEventLog({ level: "info", source: "startup", message: "App init started" });
  await hydrateLocalStorageFromFrontendSettingsDb();
  themeInit();
  accentInit();
  await hydrateAppVersionLabel();
  checkForUpdate();
  await setupConsoleFileLogging();
  logInfo("Frontend console bridge initialized");
  setupRuntimeErrorLogging();
  pushEventLog({ level: "info", source: "startup", message: "Console/event logging ready" });

  setProgress(false, 0, "Idle");
  loadSourceRootsFromStorage();
  loadSourceRootEnabledFromStorage();
  loadMasterDbEnabledFromStorage();
  loadSourcesEverConfiguredFromStorage();
  loadUsbRecentRootsFromStorage();
  renderUsbRecentRoots();

  for (const root of state.sourceRoots || []) {
    if (state.sourceRootEnabled[root] === undefined) {
      state.sourceRootEnabled[root] = true;
    }
  }
  persistSourceRootEnabled(state.sourceRootEnabled);
  await syncAssetScopePaths();
  await checkSourceRoots({ silent: true });
  loadUsbRootFromStorage();

  restoreStoredUiPrefs();
  applySidebarCollapsedUi();

  renderSourceChips();
  await detectExternalMasterDb();
  bindEvents();
  await switchView("library");
  pushEventLog({ level: "info", source: "startup", message: "Initial view ready" });

  showHelpOnFirstVisit();
  invoke("show_window").catch(() => {});

  try {
    await registerBackendJobEvents();
    try {
      const startupLogs = await invoke("get_backend_log_buffer");
      if (Array.isArray(startupLogs)) {
        for (const item of startupLogs) {
          handleBackendLogEvent(item);
        }
      }
    } catch {}
    pushEventLog({ level: "info", source: "startup", message: "Backend event listeners registered" });
  } catch (error) {
    logError("Backend event listener registration failed:", error);
    pushEventLog({
      level: "warn",
      source: "startup",
      message: `Backend event listeners unavailable: ${error?.message || String(error)}`
    });
  }

  updateUsbRootText(null, false);
  runDeferredInitialLoad();
}

export function debugFrontendLog(message, meta = null, deps = {}) {
  const {
    isTauriRuntime = () => false,
    invoke = async () => {}
  } = deps;
  if (!isTauriRuntime()) return;
  const suffix = meta == null
    ? ""
    : ` ${typeof meta === "string" ? meta : JSON.stringify(meta)}`;
  invoke("append_frontend_log", {
    level: "info",
    message: `[analysis-ui] ${message}${suffix}`
  }).catch(() => {});
}

export function handleBackendLogEvent(payload, deps = {}) {
  const { pushEventLog = () => {} } = deps;
  if (!payload || typeof payload !== "object") return;
  pushEventLog({
    level: String(payload.level || "info"),
    source: String(payload.source || "backend"),
    code: String(payload.code || "").trim(),
    message: String(payload.message || "").trim(),
    details: typeof payload.details === "string" ? payload.details : null
  });
}

export async function registerBackendJobEvents(state, deps = {}) {
  const {
    isTauriRuntime,
    unregisterBackendJobEvents,
    getTauriEventListen,
    handleJobEvent,
    handlePlaybackEvent,
    handleBackendLogEvent
  } = deps;

  return registerBackendJobEventsCore(state, {
    isTauriRuntime,
    unregisterBackendJobEvents,
    getTauriEventListen,
    handleJobEvent,
    handlePlaybackEvent,
    handleBackendLogEvent
  });
}

export async function unregisterBackendJobEvents(state, deps = {}) {
  const { warn = () => {} } = deps;
  return unregisterBackendJobEventsCore(state, { warn });
}

export async function switchView(state, el, viewId, deps = {}) {
  const {
    staticTabs = [],
    stopPlaybackIfActive = async () => {},
    syncLibraryOnboardingMode = () => {},
    updateModeText = () => {},
    populatePlaylistPanel = () => {},
    refreshCurrentPlaylistTracks = async () => {},
    renderEventLog = () => {},
    requestAnimationFrameFn = (cb) => cb(),
    documentObj = typeof document !== "undefined" ? document : null,
    renderWaveformsIn = () => {}
  } = deps;

  if (viewId !== state.activeTab) {
    await stopPlaybackIfActive();
  }
  state.activeTab = viewId;

  el.navSidebar.querySelectorAll(".nav-item[data-view]").forEach((btn) => {
    const active = btn.dataset.view === viewId;
    btn.classList.toggle("active", active);
    if (active) btn.setAttribute("aria-current", "true");
    else btn.removeAttribute("aria-current");
  });

  el.navPlaylistList.querySelectorAll(".nav-playlist-item").forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.playlistId === viewId);
  });

  const isStaticView = staticTabs.includes(viewId);
  const isPlaylist = !isStaticView;
  const panelKey = isPlaylist ? "playlist" : viewId;

  Object.entries(el.panels).forEach(([name, panel]) => {
    const active = name === panelKey;
    panel.classList.toggle("active", active);
    panel.setAttribute("aria-hidden", String(!active));
  });
  syncLibraryOnboardingMode();

  if (isPlaylist) {
    const selectedPlaylist = state.playlists.find((p) => p.id === viewId);
    if (selectedPlaylist) {
      state.currentPlaylistId = selectedPlaylist.id;
      updateModeText();
      populatePlaylistPanel(selectedPlaylist);
      await refreshCurrentPlaylistTracks();
    }
  } else if (viewId === "event-log") {
    renderEventLog();
  }

  requestAnimationFrameFn(() => {
    const activePanel = documentObj?.querySelector?.(".panel.active");
    renderWaveformsIn(activePanel || documentObj);
  });
}

export async function switchTab(state, el, tab, deps = {}) {
  return switchView(state, el, tab, deps);
}
