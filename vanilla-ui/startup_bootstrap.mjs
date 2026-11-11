import { settleUsbOrderEditBeforeLeaving } from "./components/usb/order_edit.mjs";
import { STATIC_TABS } from "./app_state.mjs";
import { loadUiTexts, t } from "./ui_texts.mjs";
import { createThemeManager, createAccentManager } from "./components/settings/actions.mjs";
import { refreshUpdateCheck, UPDATE_RECHECK_INTERVAL_MS } from "./update_check.mjs";
import { normalizeAnalysisBpmRange, DEFAULT_ANALYSIS_BPM_RANGE } from "./components/library/actions.mjs";
import { applySidebarWidth, clampSidebarWidth } from "./components/shell/actions.mjs";
import {
  STORAGE_KEY_EXPORT_PRUNE_STALE,
  STORAGE_KEY_EXPORT_BACKUP,
  STORAGE_KEY_EXPORT_ENGINE_LIBRARY,
  STORAGE_KEY_BACKUP_RETENTION_COUNT,
  STORAGE_KEY_ANALYSIS_BPM_RANGE,
  STORAGE_KEY_ANALYSIS_ENGINE,
  STORAGE_KEY_KEY_NOTATION,
  STORAGE_KEY_SIDEBAR_COLLAPSED,
  STORAGE_KEY_SIDEBAR_WIDTH,
  STORAGE_KEY_HELP_SEEN,
  STORAGE_KEY_CUE_START_ON_FIRST_BEAT,
  STORAGE_KEY_CUE_BEATGRID_LEVEL,
  STORAGE_KEY_CUE_QUANTIZE,
  STORAGE_KEY_CUE_FOLLOW_GRID,
  STORAGE_KEY_CUE_METRONOME_MIX,
} from "./settings_keys.mjs";

export async function hydrateAppVersionLabel(ctx) {
  const { el, tauriIsTauri, tauriGetVersion } = ctx;
  let version = t("ui-settings-version-not-set");
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
  el.settingsVersionText.textContent = t("ui-settings-version", { version });
}

export function checkForUpdate(ctx) {
  if (!ctx.isTauriRuntime()) return;
  refreshUpdateCheck(ctx);
  setInterval(() => refreshUpdateCheck(ctx), UPDATE_RECHECK_INTERVAL_MS);
}

export function restoreStoredUiPrefs(ctx) {
  const { state, el, localStorage } = ctx;

  try {
    const stored = localStorage.getItem(STORAGE_KEY_EXPORT_PRUNE_STALE);
    state.exportPruneStale = stored === null ? true : stored === "1";
  } catch {
    state.exportPruneStale = true;
  }
  el.exportSyncModeMirror.checked = !!state.exportPruneStale;
  el.exportSyncModeAdditive.checked = !state.exportPruneStale;

  try {
    const stored = localStorage.getItem(STORAGE_KEY_EXPORT_BACKUP);
    state.exportBackup = stored === null ? true : stored === "1";
  } catch {
    state.exportBackup = true;
  }
  el.exportBackupCheckbox.checked = !!state.exportBackup;

  try {
    state.exportEngineLibrary = localStorage.getItem(STORAGE_KEY_EXPORT_ENGINE_LIBRARY) === "1";
  } catch {
    state.exportEngineLibrary = false;
  }
  el.exportEngineLibraryCheckbox.checked = !!state.exportEngineLibrary;

  try {
    const stored = localStorage.getItem(STORAGE_KEY_BACKUP_RETENTION_COUNT);
    const parsed = stored === null ? NaN : Number.parseInt(stored, 10);
    state.backupRetentionCount = Number.isFinite(parsed) && parsed >= 1 ? parsed : 10;
  } catch {
    state.backupRetentionCount = 10;
  }
  el.backupRetentionCountInput.value = String(state.backupRetentionCount);

  try {
    const stored = localStorage.getItem(STORAGE_KEY_ANALYSIS_BPM_RANGE);
    state.analysisBpmRange = normalizeAnalysisBpmRange(stored || DEFAULT_ANALYSIS_BPM_RANGE);
  } catch {
    state.analysisBpmRange = DEFAULT_ANALYSIS_BPM_RANGE;
  }
  el.analysisBpmRangeSelect.value = state.analysisBpmRange;

  try {
    const storedEngine = localStorage.getItem(STORAGE_KEY_ANALYSIS_ENGINE);
    state.analysisEngine = storedEngine === "essentia" ? "essentia" : "stratum";
  } catch {
    state.analysisEngine = "stratum";
  }
  el.analysisEngineSelect.value = state.analysisEngine;

  // Display only: the backend renders key labels in this notation.
  try {
    const storedNotation = localStorage.getItem(STORAGE_KEY_KEY_NOTATION);
    state.keyNotation = storedNotation === "camelot" ? "camelot" : "classic";
  } catch {
    state.keyNotation = "classic";
  }
  el.keyNotationSelect.value = state.keyNotation;
  const show = state.analysisEngine === "essentia";
  el.essentiaInstallRow.classList.toggle("hidden", !show);

  try {
    state.cueStartOnFirstBeat =
      localStorage.getItem(STORAGE_KEY_CUE_START_ON_FIRST_BEAT) === "1";
  } catch {
    state.cueStartOnFirstBeat = false;
  }

  try {
    state.cueQuantize =
      localStorage.getItem(STORAGE_KEY_CUE_QUANTIZE) !== "0";
  } catch {
    state.cueQuantize = true;
  }

  try {
    state.cueFollowGrid =
      localStorage.getItem(STORAGE_KEY_CUE_FOLLOW_GRID) === "1";
  } catch {
    state.cueFollowGrid = false;
  }

  state.cueBeatgridLevel = 35;
  try {
    const raw = localStorage.getItem(STORAGE_KEY_CUE_BEATGRID_LEVEL);
    const level = raw == null || raw === "" ? NaN : Number(raw);
    if (Number.isFinite(level)) state.cueBeatgridLevel = Math.max(0, Math.min(100, Math.round(level)));
  } catch {
    // keep the default
  }

  state.cueMetronomeMix = 50;
  try {
    const raw = localStorage.getItem(STORAGE_KEY_CUE_METRONOME_MIX);
    const mix = raw == null || raw === "" ? NaN : Number(raw);
    if (Number.isFinite(mix)) state.cueMetronomeMix = Math.max(0, Math.min(100, Math.round(mix)));
  } catch {
    // keep the default
  }

  try {
    state.sidebarCollapsed = localStorage.getItem(STORAGE_KEY_SIDEBAR_COLLAPSED) === "1";
  } catch {
    state.sidebarCollapsed = false;
  }

  try {
    state.sidebarWidth = clampSidebarWidth(localStorage.getItem(STORAGE_KEY_SIDEBAR_WIDTH));
  } catch {
    state.sidebarWidth = clampSidebarWidth(null);
  }
}

export function applySidebarCollapsedUi(ctx) {
  const { state, el, document } = ctx;
  applySidebarWidth(document, state.sidebarWidth);
  if (state.sidebarCollapsed) {
    el.navSidebar.classList.add("collapsed");
    el.sidebarExpandBtn.classList.add("visible");
  }
  document.body.classList.toggle("sidebar-collapsed", !!state.sidebarCollapsed);
}

export function showHelpOnFirstVisit(ctx) {
  const { el, localStorage } = ctx;
  try {
    if (!localStorage.getItem(STORAGE_KEY_HELP_SEEN)) {
      el.helpOverlay.classList.remove("hidden");
    }
  } catch {}
}

// One-time upgrades of data older versions wrote (backend `migrations.rs`).
// The backend runs them as a progress-bar job, or returns at once when
// nothing is pending; a failure only gets logged, and they run again next
// launch.
export async function runDataMigrations(ctx) {
  try {
    return await ctx.command("run_data_migrations");
  } catch (error) {
    ctx.pushEventLog({
      level: "warn",
      source: "startup",
      message: error?.message || String(error)
    });
    return null;
  }
}

export function runDeferredInitialLoad(ctx) {
  const { state } = ctx;
  (ctx.setTimeoutFn || setTimeout)(() => {
    const texts = {
      working: t("ui-startup-progress"),
      done: t("ui-startup-progress-done"),
      failed: t("ui-startup-progress-failed")
    };
    ctx.withProgress(texts, async (progress) => {
      progress(35, t("ui-startup-loading-playlists"));
      await ctx.loadPlaylists();
      progress(70, t("ui-startup-loading-tracks"));
      await ctx.resetAndLoadLibraryTracks("");

      if (state.playlists.length > 0) {
        const hasCurrent = state.playlists.some((playlist) => playlist.id === state.currentPlaylistId);
        if (!hasCurrent) {
          // Default the selection to the top sidebar playlist (matches the
          // post-delete fallback in components/playlist/actions.mjs).
          state.currentPlaylistId = state.playlists[0].id;
        }
      }
      ctx.updateModeText();
      ctx.updateSelectionCount();
      ctx.usbPlaylistTracksCtl.clear();
      ctx.renderWaveformsIn(ctx.document);
    }).then(() => {
      state.startupPhase = false;
      return runDataMigrations(ctx);
    }).catch((error) => {
      state.startupPhase = false;
      ctx.logError(error);
      ctx.emitStatus(t("ui-startup-init-failed", { error: error.message }));
    });
  }, 0);
}

export async function initApp(ctx) {
  const { state, invoke, pushEventLog } = ctx;

  // Before anything renders: the page's text is the backend's. The window stays hidden
  // (tauri.conf.json `visible: false`) until `show_window` below.
  if (!(await loadUiTexts(ctx))) return;
  ctx.logToFile({ source: "startup", message: "App init started" });
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
  ctx.logToFile({ source: "startup", message: "Console/event logging ready" });

  ctx.setProgress(false, 0, t("ui-common-idle"));
  await ctx.loadUsbDevices();
  // The library's folders and their switches (draws the chips, lets the
  // folders into the asset scope).
  await ctx.refreshLibrarySources();
  ctx.loadUsbRootFromStorage();

  restoreStoredUiPrefs(ctx);
  applySidebarCollapsedUi(ctx);

  await ctx.detectExternalRekordboxDb();
  await ctx.detectExternalMixxxDb();
  // Sources starts folded (index.html) and opens once its chips are drawn:
  // opened earlier, its row keeps the height it had before they arrived.
  ctx.el.sourceBar.classList.remove("collapsed");
  ctx.bindEvents();
  await ctx.switchView("library");
  ctx.logToFile({ source: "startup", message: "Initial view ready" });

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
    ctx.logToFile({ source: "startup", message: "Backend event listeners registered" });
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

// An analysis debug line for the log file: `message` and its `meta` object.
export function debugFrontendLog(ctx, message, meta) {
  ctx.logToFile({ source: "analysis-ui", message: `${message} ${JSON.stringify(meta)}` });
}

// The desktop app already wrote the entry to the session log file, in English.
export function handleBackendLogEvent(ctx, payload) {
  if (!payload || typeof payload !== "object") return;
  ctx.storeEventLogEntry({
    level: payload.level,
    source: payload.source || "backend",
    code: payload.code,
    message: payload.message,
    inLogFile: true
  });
}

export async function switchView(ctx, viewId) {
  const { state, el, document } = ctx;

  const viewChanged = viewId !== state.activeTab;
  // Leaving the USB Playlists page with an order edit on: it's saved,
  // discarded or kept (staying here).
  if (viewChanged && !(await settleUsbOrderEditBeforeLeaving(ctx))) return;
  if (viewChanged) {
    await ctx.stopPlaybackIfActive();
    try {
      await ctx.commitActivePlaylistSort(state.activeTab);
    } catch (err) {
      console.error(err);
      ctx.emitStatus(err?.message || String(err));
    }
  }
  state.activeTab = viewId;

  // Backups opens from the USB page, so the USB entry stays highlighted.
  const navViewId = viewId === "backups" ? "usb" : viewId;
  el.navSidebar.querySelectorAll(".nav-item[data-view]").forEach((btn) => {
    const active = btn.dataset.view === navViewId;
    btn.classList.toggle("active", active);
    if (active) btn.setAttribute("aria-current", "true");
    else btn.removeAttribute("aria-current");
  });

  el.eventLogBtn.classList.toggle("active", viewId === "event-log");

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
    ctx.clearEventLogBadge();
    ctx.renderEventLog();
  } else if (viewId === "backups") {
    await ctx.renderBackups();
  } else if (viewId === "usb-playlists" || viewId === "usb-history") {
    ctx.ensureUsbListLoaded(viewId, { viewChanged });
  } else if (viewId === "usb-player-menu") {
    ctx.renderUsbPlayerMenuEditor();
    ctx.renderUsbBackgroundColor();
    await ctx.loadUsbPlayerMenuConfig();
    await ctx.loadUsbBackgroundColor();
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
