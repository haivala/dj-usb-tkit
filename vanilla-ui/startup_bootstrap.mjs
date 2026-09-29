import { STATIC_TABS } from "./app_state.mjs";
import { createThemeManager, createAccentManager } from "./components/settings/actions.mjs";
import { renderUpdateNotice, renderUpdateBanner } from "./update_check.mjs";
import { normalizeAnalysisBpmRange, DEFAULT_ANALYSIS_BPM_RANGE } from "./components/library/actions.mjs";
import {
  STORAGE_KEY_EXPORT_PRUNE_STALE,
  STORAGE_KEY_EXPORT_BACKUP,
  STORAGE_KEY_BACKUP_RETENTION_COUNT,
  STORAGE_KEY_ANALYSIS_BPM_RANGE,
  STORAGE_KEY_ANALYSIS_ENGINE,
  STORAGE_KEY_KEY_NOTATION,
  STORAGE_KEY_SIDEBAR_COLLAPSED,
  STORAGE_KEY_HELP_SEEN,
  STORAGE_KEY_CUE_START_ON_FIRST_BEAT,
  STORAGE_KEY_CUE_BEATGRID_LEVEL,
  STORAGE_KEY_CUE_QUANTIZE,
  STORAGE_KEY_CUE_METRONOME_MIX,
} from "./settings_keys.mjs";

const APP_VERSION_FALLBACK = "Not set";
const LIBRARY_LOAD_LIMIT_INIT = 200;

export async function hydrateAppVersionLabel(ctx) {
  const { el, tauriIsTauri, tauriGetVersion } = ctx;
  if (!el.settingsVersionText) return;
  let version = APP_VERSION_FALLBACK;
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

export async function checkForUpdate(ctx) {
  if (!ctx.isTauriRuntime()) return;
  try {
    // Backend-owned: `check_for_update` knows the running version and does the
    // GitHub fetch + version compare itself (see backend/src/service/update_check.rs).
    const info = await ctx.command("check_for_update");
    if (!info) return;
    ctx.state.updateCheck = info;
    renderUpdateNotice(ctx);
    renderUpdateBanner(ctx);
  } catch {
    // An update check must never disrupt startup.
  }
}

export function restoreStoredUiPrefs(ctx) {
  const { state, el, localStorage } = ctx;

  try {
    const stored = localStorage?.getItem?.(STORAGE_KEY_EXPORT_PRUNE_STALE);
    state.exportPruneStale = stored === null ? true : stored === "1";
  } catch {
    state.exportPruneStale = true;
  }
  if (el.exportSyncModeMirror && el.exportSyncModeAdditive) {
    el.exportSyncModeMirror.checked = !!state.exportPruneStale;
    el.exportSyncModeAdditive.checked = !state.exportPruneStale;
  }

  try {
    const stored = localStorage?.getItem?.(STORAGE_KEY_EXPORT_BACKUP);
    state.exportBackup = stored === null ? true : stored === "1";
  } catch {
    state.exportBackup = true;
  }
  if (el.exportBackupCheckbox) {
    el.exportBackupCheckbox.checked = !!state.exportBackup;
  }

  try {
    const stored = localStorage?.getItem?.(STORAGE_KEY_BACKUP_RETENTION_COUNT);
    const parsed = stored === null ? NaN : Number.parseInt(stored, 10);
    state.backupRetentionCount = Number.isFinite(parsed) && parsed >= 1 ? parsed : 10;
  } catch {
    state.backupRetentionCount = 10;
  }
  if (el.backupRetentionCountInput) {
    el.backupRetentionCountInput.value = String(state.backupRetentionCount);
  }

  try {
    const stored = localStorage?.getItem?.(STORAGE_KEY_ANALYSIS_BPM_RANGE);
    state.analysisBpmRange = normalizeAnalysisBpmRange(stored || DEFAULT_ANALYSIS_BPM_RANGE);
  } catch {
    state.analysisBpmRange = DEFAULT_ANALYSIS_BPM_RANGE;
  }
  if (el.analysisBpmRangeSelect) {
    el.analysisBpmRangeSelect.value = state.analysisBpmRange;
  }

  try {
    const storedEngine = localStorage?.getItem?.(STORAGE_KEY_ANALYSIS_ENGINE);
    state.analysisEngine = storedEngine === "essentia" ? "essentia" : "stratum";
  } catch {
    state.analysisEngine = "stratum";
  }
  if (el.analysisEngineSelect) {
    el.analysisEngineSelect.value = state.analysisEngine;
  }

  // Display only: the backend renders key labels in this notation.
  try {
    const storedNotation = localStorage?.getItem?.(STORAGE_KEY_KEY_NOTATION);
    state.keyNotation = storedNotation === "camelot" ? "camelot" : "classic";
  } catch {
    state.keyNotation = "classic";
  }
  if (el.keyNotationSelect) {
    el.keyNotationSelect.value = state.keyNotation;
  }
  if (el.essentiaInstallRow) {
    const show = state.analysisEngine === "essentia";
    el.essentiaInstallRow.classList.toggle("hidden", !show);
  }

  try {
    state.cueStartOnFirstBeat =
      localStorage?.getItem?.(STORAGE_KEY_CUE_START_ON_FIRST_BEAT) === "1";
  } catch {
    state.cueStartOnFirstBeat = false;
  }

  try {
    state.cueQuantize =
      localStorage?.getItem?.(STORAGE_KEY_CUE_QUANTIZE) !== "0";
  } catch {
    state.cueQuantize = true;
  }

  state.cueBeatgridLevel = 35;
  try {
    const raw = localStorage?.getItem?.(STORAGE_KEY_CUE_BEATGRID_LEVEL);
    const level = raw == null || raw === "" ? NaN : Number(raw);
    if (Number.isFinite(level)) state.cueBeatgridLevel = Math.max(0, Math.min(100, Math.round(level)));
  } catch {
    // keep the default
  }

  state.cueMetronomeMix = 50;
  try {
    const raw = localStorage?.getItem?.(STORAGE_KEY_CUE_METRONOME_MIX);
    const mix = raw == null || raw === "" ? NaN : Number(raw);
    if (Number.isFinite(mix)) state.cueMetronomeMix = Math.max(0, Math.min(100, Math.round(mix)));
  } catch {
    // keep the default
  }

  try {
    state.sidebarCollapsed = localStorage?.getItem?.(STORAGE_KEY_SIDEBAR_COLLAPSED) === "1";
  } catch {
    state.sidebarCollapsed = false;
  }
}

export function applySidebarCollapsedUi(ctx) {
  const { state, el, document } = ctx;
  if (state.sidebarCollapsed) {
    el.navSidebar.classList.add("collapsed");
    el.sidebarExpandBtn?.classList.add("visible");
  }
  document.body.classList.toggle("sidebar-collapsed", !!state.sidebarCollapsed);
}

export function showHelpOnFirstVisit(ctx) {
  const { el, localStorage } = ctx;
  try {
    if (!localStorage?.getItem?.(STORAGE_KEY_HELP_SEEN) && el.helpOverlay) {
      el.helpOverlay.classList.remove("hidden");
    }
  } catch {}
}

export function runDeferredInitialLoad(ctx) {
  const { state } = ctx;
  (ctx.setTimeoutFn || setTimeout)(() => {
    ctx.withProgress("Initializing", async (progress) => {
      progress(35, "Loading playlists...");
      await ctx.loadPlaylists();
      progress(70, "Loading tracks...");
      await ctx.resetAndLoadLibraryTracks("", LIBRARY_LOAD_LIMIT_INIT);

      if (state.playlists.length > 0) {
        const hasCurrent = state.playlists.some((playlist) => playlist.id === state.currentPlaylistId);
        if (!hasCurrent) {
          // list_playlists is ordered created_at ASC, so the newest playlist is
          // last -- default the selection to it (matches the post-delete fallback
          // in components/playlist/actions.mjs).
          state.currentPlaylistId = state.playlists.at(-1).id;
        }
      }
      ctx.updateModeText();
      ctx.updateSelectionCount();
      ctx.usbPlaylistTracksCtl.clear();
      ctx.renderWaveformsIn(ctx.document);
    }).then(() => {
      state.startupPhase = false;
    }).catch((error) => {
      state.startupPhase = false;
      ctx.logError(error);
      ctx.setStatus(`Initialization failed: ${error.message}`);
    });
  }, 0);
}

export async function initApp(ctx) {
  const { state, invoke, pushEventLog } = ctx;

  pushEventLog({ level: "info", source: "startup", message: "App init started" });
  await ctx.hydrateLocalStorageFromFrontendSettingsDb();
  const themeManager = createThemeManager(ctx);
  const accentManager = createAccentManager(ctx, themeManager);
  themeManager.setAccentManager(accentManager);
  themeManager.init();
  accentManager.init();
  await hydrateAppVersionLabel(ctx);
  checkForUpdate(ctx);
  await ctx.setupConsoleFileLogging();
  ctx.logInfo("Frontend console bridge initialized");
  ctx.setupRuntimeErrorLogging();
  pushEventLog({ level: "info", source: "startup", message: "Console/event logging ready" });

  ctx.setProgress(false, 0, "Idle");
  ctx.loadSourceRootsFromStorage();
  ctx.loadSourceRootEnabledFromStorage();
  ctx.loadMasterDbEnabledFromStorage();
  ctx.loadSourcesEverConfiguredFromStorage();
  await ctx.loadUsbDevices();

  for (const root of state.sourceRoots || []) {
    if (state.sourceRootEnabled[root] === undefined) {
      state.sourceRootEnabled[root] = true;
    }
  }
  ctx.persistSourceRootEnabled(state.sourceRootEnabled);
  await ctx.syncAssetScopePaths();
  await ctx.refreshMissingSourceRoots({ silent: true });
  ctx.loadUsbRootFromStorage();

  restoreStoredUiPrefs(ctx);
  applySidebarCollapsedUi(ctx);

  ctx.renderSourceChips();
  ctx.refreshSourceRootAnalysisStatus().catch(() => {});
  await ctx.detectExternalMasterDb();
  ctx.bindEvents();
  await ctx.switchView("library");
  pushEventLog({ level: "info", source: "startup", message: "Initial view ready" });

  showHelpOnFirstVisit(ctx);
  invoke("show_window").catch(() => {});

  try {
    await ctx.registerBackendJobEvents();
    try {
      const startupLogs = await invoke("get_backend_log_buffer");
      if (Array.isArray(startupLogs)) {
        for (const item of startupLogs) {
          handleBackendLogEvent(ctx, item);
        }
      }
    } catch {}
    pushEventLog({ level: "info", source: "startup", message: "Backend event listeners registered" });
  } catch (error) {
    ctx.logError("Backend event listener registration failed:", error);
    pushEventLog({
      level: "warn",
      source: "startup",
      message: `Backend event listeners unavailable: ${error?.message || String(error)}`
    });
  }

  ctx.updateUsbRootText(null, false);
  runDeferredInitialLoad(ctx);
}

export function debugFrontendLog(ctx, message, meta = null) {
  if (!ctx.isTauriRuntime()) return;
  const suffix = meta == null
    ? ""
    : ` ${typeof meta === "string" ? meta : JSON.stringify(meta)}`;
  ctx.invoke("append_frontend_log", {
    level: "info",
    message: `[analysis-ui] ${message}${suffix}`
  }).catch(() => {});
}

export function handleBackendLogEvent(ctx, payload) {
  if (!payload || typeof payload !== "object") return;
  ctx.pushEventLog({
    level: String(payload.level || "info"),
    source: String(payload.source || "backend"),
    code: String(payload.code || "").trim(),
    message: String(payload.message || "").trim(),
    details: typeof payload.details === "string" ? payload.details : null
  });
}

export async function switchView(ctx, viewId) {
  const { state, el, document } = ctx;

  if (viewId !== state.activeTab) {
    await ctx.stopPlaybackIfActive();
    try {
      await ctx.commitActivePlaylistSort(state.activeTab);
    } catch (err) {
      console.error(err);
      ctx.emitStatus(`Save track order failed: ${err.message || err}`);
    }
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

  const isPlaylist = !STATIC_TABS.includes(viewId);
  const panelKey = isPlaylist ? "playlist" : viewId;

  Object.entries(el.panels).forEach(([name, panel]) => {
    const active = name === panelKey;
    panel.classList.toggle("active", active);
    panel.setAttribute("aria-hidden", String(!active));
  });
  ctx.syncLibraryOnboardingMode();

  if (isPlaylist) {
    const selectedPlaylist = state.playlists.find((p) => p.id === viewId);
    if (selectedPlaylist) {
      state.currentPlaylistId = selectedPlaylist.id;
      ctx.updateModeText();
      ctx.populatePlaylistPanel(selectedPlaylist);
      await ctx.refreshCurrentPlaylistTracks();
    }
  } else if (viewId === "event-log") {
    ctx.renderEventLog();
  } else if (viewId === "backups") {
    await ctx.renderBackups();
  } else if (viewId === "usb-player-menu") {
    ctx.renderUsbPlayerMenuEditor();
    await ctx.loadUsbPlayerMenuConfig();
  }

  // refreshCurrentPlaylistTracks() already renders and draws waveforms for
  // the (already-visible) playlist panel, so redrawing here would just
  // repeat that work. Other views' content isn't necessarily re-rendered on
  // switch, so they still need this to redraw canvases that may have been
  // sized while hidden.
  if (!isPlaylist) {
    ctx.requestAnimationFrameFn(() => {
      const activePanel = document.querySelector(".panel.active");
      ctx.renderWaveformsIn(activePanel || document);
    });
  }
}
