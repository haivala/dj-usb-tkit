import {
  convertFileSrc as tauriConvertFileSrc,
  invoke as tauriInvoke,
  isTauri as tauriIsTauri,
} from "@tauri-apps/api/core";
import { getVersion as tauriGetVersion } from "@tauri-apps/api/app";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { escapeHtml, getPlaylistTabDomId } from "./ui_utils.mjs";
import * as trackTable from "./track_table.mjs";
import * as playback from "./components/playback/actions.mjs";
import * as playlist from "./components/playlist/actions.mjs";
import * as usb from "./components/usb/actions.mjs";
import * as eventLog from "./components/event-log/actions.mjs";
import * as backupsUi from "./components/backups/actions.mjs";
import * as library from "./components/library/actions.mjs";
import * as settings from "./components/settings/actions.mjs";
import * as shell from "./components/shell/actions.mjs";
import { createTrackListController } from "./components/shared/track_list_controller.mjs";
import { applyPlaylistReorderLockToGrid } from "./components/shared/export_reorder_lock.mjs";
import {
  createInitialState,
  createTableSortState,
  createEventLogState,
  STATIC_TABS,
} from "./app_state.mjs";
import {
  STORAGE_KEY_THEME,
  STORAGE_KEY_ACCENT_HUE,
  STORAGE_KEY_USB_ROOT,
  STORAGE_KEY_EXPORT_PRUNE_STALE,
  STORAGE_KEY_EXPORT_BACKUP,
  STORAGE_KEY_BACKUP_RETENTION_COUNT,
  STORAGE_KEY_ANALYSIS_BPM_RANGE,
  STORAGE_KEY_ANALYSIS_ENGINE,
  STORAGE_KEY_SIDEBAR_COLLAPSED,
  STORAGE_KEY_HELP_SEEN,
  FRONTEND_DB_KEY_THEME,
  FRONTEND_DB_KEY_ACCENT_HUE,
  FRONTEND_DB_KEY_EXPORT_PRUNE_STALE,
  FRONTEND_DB_KEY_EXPORT_BACKUP,
  FRONTEND_DB_KEY_BACKUP_RETENTION_COUNT,
  FRONTEND_DB_KEY_ANALYSIS_BPM_RANGE,
  FRONTEND_DB_KEY_ANALYSIS_ENGINE,
  FRONTEND_DB_KEY_SIDEBAR_COLLAPSED,
  FRONTEND_DB_KEY_HELP_SEEN,
} from "./settings_keys.mjs";
import {
  WAVEFORM_COLORS,
  deriveWaveformColors,
  drawWaveformCanvas,
  renderWaveformsIn,
  invalidateWaveformCache,
  setWaveformColorData,
} from "./waveform.mjs";
import { getKeyHue } from "./key_hue.mjs";
import {
  normalizeDurationMs,
  formatDurationMs,
  renderTrackListDurationSummary,
  getHistoryDateValue,
  getHistoryDateDisplay,
  formatTimestampLocal,
  filterTracksByQuery,
  buildTracklistText,
  loadMoreIfNearBottom,
} from "./track_utils.mjs";
import { createApiClient } from "./api_client.mjs";
import * as jobMgr from "./job_manager.mjs";
import * as bootstrap from "./startup_bootstrap.mjs";
import * as uiCtrl from "./ui_controller.mjs";
import { createMessageBus, shouldPersistStatusToEventLog } from "./message_bus.mjs";
import { openExternalUrl } from "./components/settings/events.mjs";
import {
  fetchUpdateInfo as fetchUpdateInfoRemote,
  renderUpdateNotice,
  renderCriticalUpdateBanner,
} from "./update_check.mjs";

const LIBRARY_SEARCH_DEBOUNCE_MS = 180;
const LIBRARY_LOAD_LIMIT_INIT = 200;
const LIBRARY_LOAD_LIMIT_DEFAULT = 200;
const LIBRARY_LOAD_LIMIT_POST_SCAN = 1000;
const LIBRARY_SCROLL_FETCH_THRESHOLD_PX = 120;
const APP_VERSION_FALLBACK = "Not set";
const MOCK_API_CLIENT_MODULE = "./mock_api_client.mjs";

const state = createInitialState();
const tableSortState = createTableSortState();
const eventLogStore = createEventLogState();

const { invoke, command, isTauriRuntime, getTauriEventListen } =
  createApiClient({
    tauriInvoke,
    tauriIsTauri,
    tauriListen,
    loadMockInvoke: async () => {
      const { createMockInvoke } = await import(MOCK_API_CLIENT_MODULE);
      return createMockInvoke({
        state,
        normalizePath: library.normalizePath,
        constants: { LIBRARY_LOAD_LIMIT_DEFAULT, LIBRARY_LOAD_LIMIT_POST_SCAN },
      });
    },
  });

let ThemeManager, AccentManager;

const ELEMENT_IDS = [
  "statusText", "playlistBadge", "badgeLabel", "usbNameBadge", "usbNameBadgeLabel", "usbHeaderHealthDot", "navSidebar",
  "sidebarCollapseBtn", "donateBtn", "navPlaylistList", "addPlaylistBtn",
  "playlistPanelTitle", "playlistSearchInput", "playlistTracksBody", "playlistTableWrap",
  "playlistEmptyState", "playlistTotalDuration", "playlistExportStatus", "analyzePlaylistMissingBtn",
  "exportPlaylistBtn", "settingsBtn", "settingsDrawer", "settingsBackdrop",
  "settingsCloseBtn", "settingsVersionText", "settingsUpdateNote", "criticalUpdateBanner",
  "criticalUpdateText", "criticalUpdateDismissBtn", "openEventLogBtn", "accentHueSlider",
  "accentSwatch", "accentResetBtn", "sourceFilterIndicator", "selectionActions",
  "usbConnectionBar", "usbSelectedControls", "usbInitRow", "usbInitHint",
  "usbHealthDot", "initializeUsbBtn", "sourceChipsContainer", "sourceBar",
  "sourceFilterHeader", "addSourceBtn", "importMasterDbBtn", "librarySearch",
  "libraryTableBody", "libraryTableWrap", "libraryEmptyState", "libraryContent",
  "selectAllTracks", "selectionCount", "usbPlaylists", "usbTrackSearch",
  "usbPlaylistTracks", "usbPlaylistTotalDuration", "historyList", "historyTrackSearch",
  "historyTracks", "historyTotalDuration", "usbPlayerMenuAvailable", "usbPlayerMenuCurrent",
  "usbPlayerMenuAddBtn", "usbPlayerMenuRemoveBtn", "usbPlayerMenuUpBtn", "usbPlayerMenuDownBtn",
  "usbPlayerMenuDivergence", "usbPlayerMenuDivergenceMessage", "usbPlayerMenuSyncBtn", "usbPlayerMenuRestoreBtn",
  "libraryTotalDuration", "scanLibraryBtn", "addSelectedBtn", "refreshUsbBtn",
  "refreshHistoryBtn", "exportHistoryTracklistBtn", "runUsbParityBtn", "exportSyncModeGroup",
  "exportSyncModeMirror", "exportSyncModeAdditive", "exportBackupCheckbox", "backupRetentionCountInput",
  "openBackupsBtn", "backupsList", "backupsSummary", "backupsRefreshBtn",
  "driveNameOverlay", "driveNameInput", "driveNameError", "driveNameOkBtn", "driveNameSkipBtn", "analysisBpmRangeSelect",
  "analysisEngineSelect", "analysisEngineStatus", "essentiaInstallRow", "essentiaNodeStatus",
  "essentiaDownloadBtn", "essentiaCancelBtn", "essentiaRemoveBtn", "selectUsbFolderBtn",
  "usbRecentRow", "usbRecentList", "usbRootPathText", "externalMasterDbToggle",
  "externalMasterDbCheckbox", "externalMasterDbPath", "usbCountsText", "historyCountsText",
  "progressFooter", "progressText", "progressFill", "progressDismiss",
  "progressPauseBtn", "progressCancelAnalysisBtn", "usbDiagnosticsCard", "diagOverallStatus",
  "diagDuration", "diagSections", "diagReportView", "diagRepairPanel",
  "diagRepairSummary", "diagRepairFixes", "diagBackToReportBtn", "diagPlaylistDetails",
  "diagPlaylistTableBody", "reDiagnoseBtn", "previewRepairsBtn", "applyRepairsBtn",
  "confirmOverlay", "confirmTitle", "confirmMessage", "confirmOkBtn",
  "confirmCancelBtn", "tracklistExportOverlay", "tracklistExportTitle", "tracklistExportStartTrack",
  "tracklistExportTimesToggle", "tracklistExportPlacementRow", "tracklistExportPlacement", "tracklistExportOkBtn",
  "tracklistExportCancelBtn", "helpBtn", "helpOverlay", "helpCloseBtn",
  "eventLogLevelFilter", "eventLogSourceFilter", "eventLogClearBtn", "eventLogSummary",
  "eventLogList",
];

const el = {
  ...Object.fromEntries(ELEMENT_IDS.map((id) => [id, document.getElementById(id)])),
  panels: {
    library: document.getElementById("panel-library"),
    usb: document.getElementById("panel-usb"),
    "usb-playlists": document.getElementById("panel-usb-playlists"),
    "usb-history": document.getElementById("panel-usb-history"),
    "usb-player-menu": document.getElementById("panel-usb-player-menu"),
    "event-log": document.getElementById("panel-event-log"),
    backups: document.getElementById("panel-backups"),
    playlist: document.getElementById("panel-playlist"),
  },
};

const confirmDialog = uiCtrl.createConfirmDialogController(el);
const tracklistExportDialog = uiCtrl.createTracklistExportDialogController(el);

// --- Closures that bind state/el/deps ---

function persistSetting(storageKey, dbKey, value) {
  settings.persistSetting(command, storageKey, dbKey, value);
}

ThemeManager = settings.createThemeManager({
  persistSetting: (sk, dk, v) => persistSetting(sk, dk, v),
  invoke,
  deriveWaveformColors,
  WAVEFORM_COLORS,
  renderWaveformsIn,
  STORAGE_KEY_THEME,
  FRONTEND_DB_KEY_THEME,
  STORAGE_KEY_ACCENT_HUE,
});
AccentManager = settings.createAccentManager({
  el,
  persistSetting: (sk, dk, v) => persistSetting(sk, dk, v),
  themeManager: ThemeManager,
  STORAGE_KEY_ACCENT_HUE,
  FRONTEND_DB_KEY_ACCENT_HUE,
});
ThemeManager.setAccentManager(AccentManager);

function pushEventLogRaw(entry = {}) {
  eventLog.pushEventLog(state, eventLogStore, renderEventLog, entry);
}
function logWarnings(source, warnings, context = "") {
  eventLog.logWarnings(pushEventLog, source, warnings, context);
}
function renderEventLog() {
  eventLog.renderEventLog(state, el, document, {
    ensureEventLogSourceOptions: () =>
      eventLog.ensureEventLogSourceOptions(state, el, document),
    escapeHtml,
  });
}

async function renderBackups() {
  await backupsUi.renderBackups(state, el, document, { command, escapeHtml });
}

function setProgress(active, percent = 0, text = "", opts = {}) {
  jobMgr.setProgress(state, el, active, percent, text, opts);
}
function dismissProgress() {
  jobMgr.dismissProgress(state, el);
}
function toggleAnalysisPause() {
  const paused = !state.analysisPaused;
  state.analysisPaused = paused;
  jobMgr.updateAnalysisPauseButtonAppearance(el, paused);
  if (paused) {
    // Only tracks not yet picked up are held back -- anything already in
    // flight keeps running, so the timer keeps counting until that settles.
    // job_manager's handleJobEvent freezes it once analyzingTrackIds empties
    // out. If nothing is in flight right now, it's already effectively
    // stopped, so reflect that immediately.
    if (state.analyzingTrackIds.size === 0) {
      jobMgr.pauseProgressHeartbeat(state, el);
    }
  } else {
    jobMgr.resumeProgressHeartbeat(state, el);
  }
  command("set_analysis_paused", { paused }).catch((err) => {
    console.error("[analysis-ui] set_analysis_paused failed:", err);
  });
}
function cancelAnalysis() {
  el.progressPauseBtn.hidden = true;
  el.progressCancelAnalysisBtn.hidden = true;
  if (state.analysisPaused) {
    state.analysisPaused = false;
    jobMgr.resumeProgressHeartbeat(state, el);
  }
  command("cancel_analysis").catch((err) => {
    console.error("[analysis-ui] cancel_analysis failed:", err);
  });
}
function startProgressHeartbeat() {
  jobMgr.startProgressHeartbeat(state, el);
}
function stopProgressHeartbeat() {
  jobMgr.stopProgressHeartbeat(state);
}
const withProgress = (label, fn) => jobMgr.withProgress(state, el, label, fn);

const messageBus = createMessageBus({
  setStatusText: (text, warningCount) => uiCtrl.setStatusText(el, text, warningCount),
  setProgressText: (progress) => {
    if (!progress?.text) return;
    const percent = Number(progress.percent);
    if (Number.isFinite(percent)) {
      setProgress(true, percent, progress.text);
      return;
    }
    setProgress(true, state.progressPercent, progress.text);
  },
  pushEventLog: pushEventLogRaw,
});

const emitMessage = (input = {}) => messageBus.emitMessage(input);

function setStatus(text, meta = {}) {
  const statusText = String(text || "");
  const level = meta.level || "info";
  const eventLog = shouldPersistStatusToEventLog(level, state.startupPhase)
    ? {
      text: statusText,
      details: meta.details ?? null,
      coalesceKey: meta.coalesceKey ?? (state.startupPhase ? "startup.status" : null)
    }
    : null;
  emitMessage({
    level,
    source: meta.source || "ui",
    code: meta.code || null,
    status: { text: statusText, warningCount: meta.warningCount || 0 },
    eventLog,
  });
}

const emitStatus = (text, meta = {}) => setStatus(text, meta);

function pushEventLog(entry = {}) {
  const text = String(entry.message ?? entry.text ?? "").trim();
  if (!text) return null;
  return emitMessage({
    level: entry.level,
    source: entry.source,
    code: entry.code,
    ts: entry.ts,
    eventLog: {
      text,
      details: entry.details ?? null,
      coalesceKey: entry.coalesceKey ?? null
    }
  });
}

function debugFrontendLog(message, meta = null) {
  bootstrap.debugFrontendLog(message, meta, { isTauriRuntime, invoke });
}

const toPlayableUrl = (path) => playback.toPlayableUrl(path, {
    isTauriRuntime,
    tauriConvertFileSrc,
    windowObj: window,
  });
const normalizePath = (value) => library.normalizePath(value);

const normalizeTrack = (track, fallbackIdPrefix = "t") => library.normalizeTrack(track, fallbackIdPrefix, {
    toPlayableUrl,
    appendUrlRevision: library.appendUrlRevision,
    normalizeDurationMs,
  });

const buildCoverSrcCandidates = (track) => library.buildCoverSrcCandidates(track, { toPlayableUrl });
const attachCoverFallbackHandlers = (root = document) => library.attachCoverFallbackHandlers(root, { document });

const usbTrackNeedsHydration = (track) => library.usbTrackNeedsHydration(track, {
    trackHasRenderableWaveform: library.trackHasRenderableWaveform,
    trackHasArtwork: library.trackHasArtwork,
    trackArtworkChecked: library.trackArtworkChecked,
    trackHasBpm: library.trackHasBpm,
    trackHasKey: library.trackHasKey,
  });

function getCurrentPlaylist() {
  return state.playlists.find((p) => p.id === state.currentPlaylistId) || null;
}
function requireCurrentPlaylist() {
  const p = getCurrentPlaylist();
  if (p) return p;
  setStatus("Create and activate a playlist first");
  return null;
}

function cssEscape(value) {
  const text = String(value || "");
  return typeof window.CSS?.escape === "function"
    ? window.CSS.escape(text)
    : text.replace(/["\\]/g, "\\$&");
}

function updateModeText() {
  uiCtrl.updateModeText(state, el, {
    getCurrentPlaylist,
    updateAddToPlaylistButtons,
    updateActivePlaylistIndicators,
  });
}
function updateActivePlaylistIndicators() {
  uiCtrl.updateActivePlaylistIndicators(state, el);
}
function updateUsbNameBadge() {
  uiCtrl.updateUsbNameBadge(state, el);
}
function updateAddToPlaylistButtons() {
  uiCtrl.updateAddToPlaylistButtons(state, document);
}
function updateSelectionCount() {
  uiCtrl.updateSelectionCount(state, el);
  updateScanLibraryButtonLabel();
}
function updateUsbSubNavDisabledState() {
  uiCtrl.updateUsbSubNavDisabledState(state, el, { switchView });
}
function closeSettingsDrawer() {
  uiCtrl.closeSettingsDrawer(el);
}
function updateUsbHealthDot(status) {
  uiCtrl.updateUsbHealthDot(el, status);
}
function syncLibraryOnboardingMode() {
  uiCtrl.syncLibraryOnboardingMode(state, document);
}
function updateSourceFilterIndicator() {
  uiCtrl.updateSourceFilterIndicator(state, el);
}
function updateScanLibraryButtonLabel() {
  uiCtrl.updateScanLibraryButtonLabel(state, el, {
    scanLibraryButtonLabel: library.scanLibraryButtonLabel,
  });
}
function updateUsbEmptyState() {
  uiCtrl.updateUsbEmptyState(state, document, { renderEmptyState });
}
function renderEmptyState(container, opts) {
  shell.renderEmptyState(document, container, opts);
}

function updateUsbConfigControlsVisibility() {
  usb.updateUsbConfigControlsVisibility(state, el);
  updateUsbEmptyState();
}
function updateUsbRootText(path, valid = false) {
  usb.updateUsbRootText(el, path, valid);
}
function renderUsbRecentRoots() {
  usb.renderUsbRecentRoots(el, state.usbRecentRoots, document, state);
}

const patchTrackAnalysisFields = (track, payload) => library.patchTrackAnalysisFields(track, payload, { toPlayableUrl });

const normalizeUsbPlaylist = (p) => library.normalizeUsbPlaylist(p, { normalizeTrack });

const renderPlaylistSidebarItemContent = (p) => playlist.renderPlaylistSidebarItemContent(p, { escapeHtml });
function updatePlaylistPanelTitle(p) {
  playlist.updatePlaylistPanelTitle(el, p, { formatDurationMs });
}
const formatPlaylistExportStatus = (p) => playlist.formatPlaylistExportStatus(p, { formatTimestampLocal });
function populatePlaylistPanel(p) {
  playlist.populatePlaylistPanel(el, state, p, {
    updatePlaylistPanelTitle,
    formatPlaylistExportStatus,
    updatePlaylistExportButtons,
  });
}

function renderPlaylistList() {
  playlist.renderPlaylistList(state, el, {
    document,
    renderPlaylistSidebarItemContent,
  });
}
function updatePlaylistExportButtons() {
  playlist.updatePlaylistExportButtons(state, el, {
    getCurrentPlaylist,
    computeExportButtonState: usb.computeExportButtonState,
    isUsbRootChangeBlocked: usb.isUsbRootChangeBlocked,
  });
}

const createTrackRow = (track, options) => trackTable.createTrackRow(track, options, {
    state,
    buildCoverSrcCandidates,
    isTrackCurrentlyPlaying,
    escapeHtml,
    getKeyHue,
  });

async function renderTrackTable(tbody, tracks, options = {}) {
  await trackTable.renderTrackTable(tbody, tracks, options, {
    createTrackRow,
    attachCoverFallbackHandlers,
    renderWaveformsIn,
    setWaveformColorData,
    updateTransportButtonsInDom,
    escapeHtml,
    setStatus,
  });
}

const applySortToTracks = (tracks, tbodyId) => shell.applySortToTracks(tableSortState, tracks, tbodyId, {
    sortTracks: trackTable.sortTracks,
  });

const clearPlaylistTrackSort = () => shell.clearTrackSort(
  tableSortState,
  "playlistTracksBody",
  el.playlistTracksBody?.closest("[data-track-grid]")
);

const commitActivePlaylistSort = (playlistId) => playlist.commitActivePlaylistSort(state, playlistId, {
  command,
  isPlaylistSortActive: () => !!tableSortState.playlistTracksBody,
  clearPlaylistTrackSort,
  applySortToTracks,
});

// The app-playlist track table's data layer. Unlike the other three views its
// list is not paginated (get_playlist_tracks returns the whole playlist) and
// its column sort + search are *client-side view ops* -- the sort only becomes
// the playlist's real order when committed on navigate-away/export
// (commitActivePlaylistSort), and search must not shrink `playlist.tracks`
// (which drag-reorder and the commit both operate on in full). So this runs in
// sortMode "client": `ctl.items` is the whole playlist (backed by
// `getCurrentPlaylist().tracks`), `ctl.view` is the filtered+sorted render.
const playlistTracksCtl = createTrackListController({
  bodyId: "playlistTracksBody",
  pageSize: 0,
  getElements: () => ({
    body: el.playlistTracksBody,
    wrap: el.playlistTableWrap,
    durationTarget: el.playlistTotalDuration,
  }),
  fetchPage: ({ scopeId, cursor, limit }) =>
    command("get_playlist_tracks", { playlistId: scopeId, cursor: cursor || null, limit }),
  normalize: (track) => normalizeTrack(track, "plt"),
  getItems: () => getCurrentPlaylist()?.tracks || [],
  setItems: (value) => {
    const p = getCurrentPlaylist();
    if (p) p.tracks = value;
  },
  sortMode: "client",
  sortItems: (items) => applySortToTracks(items, "playlistTracksBody"),
  filterItems: (items) => filterTracksByQuery(items, playlistTracksCtl.query),
  rowOptions: () => {
    const playlist = getCurrentPlaylist();
    const lock = playlist
      ? applyPlaylistReorderLockToGrid(
        el,
        playlist,
        { searchActive: !!playlistTracksCtl.query },
        state.playlistUsbExportStatusById,
      )
      : {};
    return {
      withCheckbox: false,
      origin: "local",
      secondaryActionLabel: "Play",
      secondaryActionType: "play-library",
      enableAnalyzeActions: false,
      actionLabel: "×",
      actionType: "remove-playlist-track",
      compactAddButton: true,
      reservesDragColumn: true,
      enableDragReorder: lock.enableDragReorder,
      dragDisabledTooltip: lock.dragDisabledTooltip,
    };
  },
  renderTrackTable,
  renderDurationSummary: (target, summary) =>
    renderTrackListDurationSummary(target, summary, formatDurationMs),
  getTableSortState: () => tableSortState,
  onResponse: (data) => {
    const p = getCurrentPlaylist();
    if (!p) return;
    p.totalDurationMs = Number(data.totalDurationMs) || 0;
    p.durationKnownCount = Number(data.durationKnownCount) || 0;
  },
});

// Panel glue around the playlist track table that the controller does not own:
// empty state, section visibility, search-input restore, is-analyzing pulse,
// title, export buttons, sidebar. Run after every (re)render of the list.
function renderPlaylistPanelChrome() {
  const playlist = getCurrentPlaylist();
  if (!playlist) return;
  const empty = !(playlist.tracks || []).length;
  if (el.playlistEmptyState) {
    el.playlistEmptyState.innerHTML = "";
    if (empty) {
      renderEmptyState(el.playlistEmptyState, {
        icon: "♫",
        heading: "Browse Library or USB to add tracks",
      });
    }
  }
  el.playlistTableWrap?.classList.toggle("hidden", empty);
  el.playlistTotalDuration?.classList.toggle("hidden", empty);
  el.playlistSearchInput?.closest(".search-row")?.classList.toggle("hidden", empty);
  el.exportPlaylistBtn?.closest(".playlist-actions")?.classList.toggle("hidden", empty);
  if (el.playlistSearchInput && el.playlistSearchInput.value !== (state.playlistTrackSearch || "")) {
    el.playlistSearchInput.value = state.playlistTrackSearch || "";
  }
  for (const id of state.analyzingTrackIds) {
    const row = el.playlistTracksBody?.querySelector(
      `.track-grid-row[data-track-id="${cssEscape(id)}"][data-track-origin="local"]`,
    );
    if (row) row.classList.add("is-analyzing");
  }
  updatePlaylistPanelTitle(playlist);
  updatePlaylistExportButtons();
  renderPlaylistList();
}

function handleSortHeaderClick(e) {
  shell.handleSortHeaderClick(tableSortState, e, {
    renderMap: {
      applyLibrarySort,
      renderUsbPlaylistTracks,
      renderHistoryTracks,
      renderCurrentPlaylistTracksFromState,
    },
    bodyToRendererMap: {
      libraryTableBody: "applyLibrarySort",
      usbPlaylistTracks: "renderUsbPlaylistTracks",
      historyTracks: "renderHistoryTracks",
      playlistTracksBody: "renderCurrentPlaylistTracksFromState",
    },
    doc: document,
  });
}

// --- Playback closures ---

function updateTransportButtonsInDom(root) {
  playback.updateTransportButtonsInDom(state, root || document);
}
function clearAllWaveformPlayheads() {
  playback.clearAllWaveformPlayheads(document);
}
function setWaveformPlayhead(element, fraction, playing) {
  playback.setWaveformPlayhead(element, fraction, playing);
}
const resolveLocalTrackId = (track) => playback.resolveLocalTrackId(track, state, { normalizePath });
const resolveLocalTrack = (track) => playback.resolveLocalTrack(track, state);
const getTrackPlaybackPath = (track) => playback.getTrackPlaybackPath(track, { resolveLocalTrack });
const isTrackCurrentlyPlaying = (track) => playback.isTrackCurrentlyPlaying(track, state, {
    normalizePath,
    getTrackPlaybackPath,
  });

const resolveLocalTrackIdAsync = async (track) => playback.resolveLocalTrackIdAsync(track, state, {
    command,
    normalizePath,
    promoteTrackIdentity,
    resolveLocalTrackId,
  });
function promoteTrackIdentity(oldId, newId) {
  library.promoteTrackIdentity(state, el, oldId, newId, { cssEscape });
}

const stopPlaybackIfActive = async () => playback.stopPlaybackIfActive(state, {
    command,
    clearAllWaveformPlayheads,
    updateTransportButtonsInDom,
    setStatus,
    warn: (...a) => console.warn(...a),
    cancelAnimationFrameFn: window.cancelAnimationFrame.bind(window),
  });
const stopPlaybackFromUi = async () => playback.stopPlaybackFromUi(state, {
    command,
    clearAllWaveformPlayheads,
    updateTransportButtonsInDom,
    setStatus,
    cancelAnimationFrameFn: window.cancelAnimationFrame.bind(window),
  });
const playTrackFromOrigin = async (track, origin, options = {}) => playback.playTrackFromOriginController(state, track, origin, options, {
    playTrackFromOriginCore: playback.playTrackFromOrigin,
    command,
    clearAllWaveformPlayheads,
    setWaveformPlayhead,
    updateTransportButtonsInDom,
    setStatus,
    requestAnimationFrameFn: window.requestAnimationFrame.bind(window),
    cancelAnimationFrameFn: window.cancelAnimationFrame.bind(window),
  });

function handlePlaybackEvent(payload) {
  playback.handlePlaybackEvent(state, payload, {
    setWaveformPlayhead,
    updateTransportButtonsInDom,
    clearAllWaveformPlayheads,
    setStatus,
    resolveTrackIdForPath: (path) => playback.findTrackIdByPath(state, path, { normalizePath }),
    requestAnimationFrameFn: window.requestAnimationFrame.bind(window),
    cancelAnimationFrameFn: window.cancelAnimationFrame.bind(window),
  });
}

const patchRowCellDeps = {
  escapeHtml,
  getKeyHue,
  buildCoverSrcCandidates,
  attachCoverFallbackHandlers,
  drawWaveformCanvas,
  invalidateWaveformCache,
  setWaveformColorData,
};

const patchLibraryRowByTrackId = (trackId) => library.patchLibraryRowByTrackId(state, el, trackId, {
    cssEscape,
    patchLibraryRowCells: (row, track) =>
      library.patchLibraryRowCells(row, track, patchRowCellDeps),
  });
const patchPlaylistRowByTrackId = (trackId) => library.patchPlaylistRowByTrackId(state, el, trackId, {
    cssEscape,
    getCurrentPlaylist,
    patchLibraryRowCells: (row, track) =>
      library.patchLibraryRowCells(row, track, patchRowCellDeps),
  });
function patchTrackRowInContainer(container, track) {
  const trackId = String(track?.id || "").trim();
  if (!trackId) return false;
  const selector = `.track-grid-row[data-track-origin="usb"][data-track-id="${cssEscape(trackId)}"]`;
  const rows = container?.querySelectorAll?.(selector) || [];
  if (!rows.length) return false;
  let patched = false;
  rows.forEach((row) => {
    if (library.patchLibraryRowCells(row, track, patchRowCellDeps)) {
      patched = true;
    }
  });
  return patched;
}
const patchUsbTrackRow = (track) => patchTrackRowInContainer(el.usbPlaylistTracks, track);
const patchHistoryTrackRow = (track) => patchTrackRowInContainer(el.historyTracks, track);
function setTrackAnalyzingState(trackId, active) {
  library.setTrackAnalyzingState(state, trackId, active, {
    patchLibraryRowByTrackId,
    patchPlaylistRowByTrackId,
  });
}

// --- Library closures ---

// The library track table's data layer: server-paginated + searched + sorted
// via `browse_source_files`, rendered through the shared TrackListController.
// `ctl.items` is backed by `state.tracks` (read by playback resolution,
// analysis patching, and selection) so those stay consistent as pages load.
// `libraryPrevById` is rebuilt before every page's normalize() so a lazily
// hydrated waveform preview survives a reload / filter / sort change.
let libraryPrevById = new Map();

const libraryTracksCtl = createTrackListController({
  bodyId: "libraryTableBody",
  pageSize: LIBRARY_LOAD_LIMIT_DEFAULT,
  getElements: () => ({
    body: el.libraryTableBody,
    wrap: el.libraryTableWrap,
    durationTarget: el.libraryTotalDuration,
  }),
  fetchPage: ({ query, sortBy, sortDir, cursor, limit }) => {
    const enabledRoots = (state.sourceRoots || []).filter(
      (root) => state.sourceRootEnabled?.[root] !== false && !library.sourceRootIsMissing(state, root),
    );
    const includeMasterDb = state.masterDbEnabled === true;
    state.libraryQuery = String(query || "").trim();
    if (!enabledRoots.length && !includeMasterDb) {
      return { total: 0, items: [], nextCursor: null, hasMore: false, totalDurationMs: 0, durationKnownCount: 0 };
    }
    return command("browse_source_files", {
      sourceRoots: enabledRoots,
      includeMasterDb,
      query: state.libraryQuery,
      sortBy: sortBy || null,
      sortDir: sortDir || null,
      cursor: cursor || null,
      limit,
    });
  },
  normalize: (track) => {
    const normalized = normalizeTrack(track, "lib");
    const prev = libraryPrevById.get(String(normalized.id));
    return prev ? library.mergeTrackPreservingBestFields(prev, normalized) : normalized;
  },
  getItems: () => state.tracks,
  setItems: (value) => { state.tracks = value; },
  rowOptions: () => ({
    withCheckbox: true,
    selectedIds: state.selectedTrackIds,
    actionLabel: "+",
    actionType: "add-library",
    compactAddButton: true,
    enableAnalyzeActions: true,
    origin: "local",
    secondaryActionLabel: "Play",
    secondaryActionType: "play-library",
  }),
  renderTrackTable,
  renderDurationSummary: (_target, summary) =>
    applyLibraryDurationSummary(
      summary.totalDurationMs,
      Number(summary.trackCount || 0) - Number(summary.durationKnownCount || 0),
    ),
  getTableSortState: () => tableSortState,
  onResponse: (data) => {
    // On a re-query (search / sort) state.tracks still holds the previous list;
    // snapshot it so surviving tracks keep their lazily hydrated previews. On a
    // full reload (source-filter change) state.tracks is already cleared and
    // resetAndLoadLibraryTracks took the snapshot before clearing.
    if ((state.tracks || []).length) {
      libraryPrevById = new Map(state.tracks.map((t) => [String(t.id), t]));
    }
    library.applySourceRootAnalysisFromBrowseData(state, data);
    renderSourceChips();
  },
  onPage: (_page, { first }) => {
    if (first) {
      const loadedIds = new Set((state.tracks || []).map((t) => t.id));
      state.selectedTrackIds = new Set(
        [...state.selectedTrackIds].filter((id) => loadedIds.has(id)),
      );
      updateSelectionCount();
    }
    renderLibraryChrome();
    void hydrateLoadedTracksPreviewsInBackground();
  },
});

function renderLibraryChrome() {
  library.renderLibraryChrome(state, el, {
    renderEmptyState,
    syncLibraryOnboardingMode,
    cssEscape,
    onEnableMasterDb: () => scanMasterDb(),
  });
}

// Re-render the loaded library rows (no fetch) + chrome -- used after an
// in-place mutation of `state.tracks` (analysis patch, bg preview hydration)
// or a selection change.
async function renderLibraryRows() {
  await libraryTracksCtl.rerender();
  renderLibraryChrome();
}
const getLibraryVisibleTracks = () => libraryTracksCtl.view;
// Kept name: callers use this to "re-render the library after mutating
// state.tracks in place" -- search itself is now a backend query.
const applySearchLocalFilter = () => renderLibraryRows();
// Library column-header sort -> re-query page 1 with the new sortBy (spans the
// whole library, not just the loaded rows). Wired via bodyToRendererMap.
const applyLibrarySort = async () => {
  await libraryTracksCtl.applyHeaderSort();
  renderLibraryChrome();
};

function renderSourceChips() {
  library.renderSourceChips(state, el, {
    documentObj: document,
    escapeHtml,
    persistSourceRootEnabled,
    updateScanLibraryButtonLabel,
    updateSourceFilterIndicator,
  });
}
const refreshSourceRootAnalysisStatus = async () => library.refreshSourceRootAnalysisStatus(state, {
    command,
    renderSourceChips,
  });
const checkSourceRoots = async (options = {}) => library.refreshMissingSourceRoots(state, {
    command,
    renderSourceChips,
    emitStatus,
    silent: options?.silent !== false,
  });
// Debounced library search. Goes through the controller's setSearch (a
// re-query of page 1 that does NOT pre-clear state.tracks), so lazily hydrated
// waveform previews on tracks that survive the filter aren't flashed away.
let librarySearchDebounceTimer = null;
function scheduleApplySearchLocalFilter() {
  if (librarySearchDebounceTimer) window.clearTimeout(librarySearchDebounceTimer);
  librarySearchDebounceTimer = window.setTimeout(() => {
    librarySearchDebounceTimer = null;
    Promise.resolve(libraryTracksCtl.setSearch(el.librarySearch?.value || ""))
      .then(renderLibraryChrome)
      .catch((err) => {
        console.error(err);
        emitStatus(err.message || String(err));
      });
  }, LIBRARY_SEARCH_DEBOUNCE_MS);
}
function applyLibraryDurationSummary(totalMs, unknownCount) {
  library.applyLibraryDurationSummary(el, state, totalMs, unknownCount, { formatDurationMs });
}
// Re-render the open playlist's track table from the already-loaded
// `playlist.tracks` (no fetch) -- used when only the view changed: a column
// sort, a reorder-lock flip after a USB scan, a cross-view analysis patch.
async function renderCurrentPlaylistTracksFromState() {
  if (!getCurrentPlaylist()) return;
  await playlistTracksCtl.rerender();
  renderPlaylistPanelChrome();
}
const mergeHydratedTrackIntoState = (rawTrack) => library.mergeHydratedTrackIntoState(state, rawTrack, {
    normalizeTrack,
  });

const applyRealtimeAnalyzedTrackUpdate = async (payload) => library.applyRealtimeAnalyzedTrackUpdate(state, payload, {
    patchTrackAnalysisFields,
    debugFrontendLog,
    log: (...a) => console.log(...a),
    warn: (...a) => console.warn(...a),
    patchLibraryRowByTrackId,
    hydrateTrackPreviewFromBackend,
  });
const hydrateTrackPreviewFromBackend = async (trackId) => library.hydrateTrackPreviewFromBackend(state, trackId, {
    command,
    mergeHydratedTrackIntoState,
    patchLibraryRowByTrackId,
  });
const hydrateLoadedTracksPreviewsInBackground = async () => library.hydrateLoadedTracksPreviewsInBackground(state, {
    getLoadedTracks: () => state.tracks,
    command,
    mergeHydratedTrackIntoState,
    patchLibraryRowByTrackId,
    nextPaint: jobMgr.nextPaint,
    rerenderLibrary: renderLibraryRows,
    renderCurrentPlaylistTracksFromState,
    renderSourceChips,
    batchSize: 48,
  });
// Fetch the library from page 1 (source-filter / search / post-scan reload).
// `state.libraryQuery` is the persisted search text; a bigger `limit` (post
// scan) is a one-shot override, otherwise the controller's page size is used.
async function resetAndLoadLibraryTracks(
  query = "",
  limit = LIBRARY_LOAD_LIMIT_DEFAULT,
) {
  state.libraryQuery = String(query || "").trim();
  libraryTracksCtl.query = state.libraryQuery;
  // Snapshot for waveform-preview preservation before ctl.load() clears state.tracks.
  libraryPrevById = new Map((state.tracks || []).map((t) => [String(t.id), t]));
  const opts = Number.isFinite(limit) && limit > LIBRARY_LOAD_LIMIT_DEFAULT ? { limit } : {};
  await libraryTracksCtl.load(opts);
}
const loadMoreLibraryTracks = async () => libraryTracksCtl.loadMore();
function handleLibraryTableWrapScroll() {
  loadMoreIfNearBottom(
    el.libraryTableWrap,
    LIBRARY_SCROLL_FETCH_THRESHOLD_PX,
    () => libraryTracksCtl.loading,
    () => libraryTracksCtl.hasMore,
    () => libraryTracksCtl.loadMore().catch((err) => {
      console.error(err);
      emitStatus(err.message || String(err));
    }),
  );
}
const scanLibrary = async () => library.scanLibrary(state, {
    setStatus,
    emitStatus,
    command,
    persistSourceRoots,
    resetAndLoadLibraryTracks,
    LIBRARY_LOAD_LIMIT_POST_SCAN,
    trackPathIsInsideSelectedRoots: (fp) =>
      library.trackPathMatchesAnyRoot(
        fp,
        library.enabledSourceRoots(state.sourceRoots, state.sourceRootEnabled, state.missingSourceRoots)
      ),
    analyzeTrackIds,
    refreshCurrentPlaylistTracks,
    countWarningsForStatus: eventLog.countWarningsForStatus,
    renderSourceChips,
  });
const relocateSourceRoot = async (oldRoot) => library.relocateSourceRoot(state, oldRoot, {
    pickSourceFolders,
    command,
    persistSourceRoots,
    persistSourceRootEnabled,
    syncAssetScopePaths,
    renderSourceChips,
    resetAndLoadLibraryTracks,
    refreshCurrentPlaylistTracks,
    refreshMissingSourceRoots: checkSourceRoots,
    LIBRARY_LOAD_LIMIT_DEFAULT,
    emitStatus,
  });
const scanMasterDb = async () => library.scanMasterDb(state, {
    setStatus,
    emitStatus,
    command,
    resetAndLoadLibraryTracks,
    LIBRARY_LOAD_LIMIT_POST_SCAN,
    refreshCurrentPlaylistTracks,
    persistMasterDbEnabled,
    persistSourcesEverConfigured,
    renderSourceChips,
    logWarnings,
  });
const analyzeTrackIds = async (trackIds, modeLabel = "Analyze", options = {}) => library.analyzeTrackIds(state, trackIds, modeLabel, options, {
    parseAnalysisBpmRange: library.parseAnalysisBpmRange,
    command,
    setStatus,
    emitStatus,
    setTrackAnalyzingState,
    nextPaint: jobMgr.nextPaint,
    mergeHydratedTrackIntoState,
    patchLibraryRowByTrackId,
    patchPlaylistRowByTrackId,
    applySearchLocalFilter,
    renderSourceChips,
    refreshSourceRootAnalysisStatus,
    refreshCurrentPlaylistTracks,
    countWarningsForStatus: eventLog.countWarningsForStatus,
    logWarnings,
  });
const analyzeSelectedTracks = async () => library.analyzeSelectedTracks(state, {
    setStatus,
    emitStatus,
    analyzeTrackIds,
    refreshCurrentPlaylistTracks,
  });
const analyzeSingleTrack = async (track, modeLabel = null) => library.analyzeSingleTrack(state, track, modeLabel, {
    resolveLocalTrackId,
    resolveLocalTrackIdAsync,
    setStatus,
    emitStatus,
    analyzeTrackIds,
  });

// --- Settings closures ---

function persistSourceRoots(roots) {
  settings.persistSourceRoots(command, roots);
}
function persistUsbRoot(path) {
  settings.persistUsbRoot(command, path);
}
function persistSourceRootEnabled(enabledMap) {
  settings.persistSourceRootEnabled(command, enabledMap);
}
function loadSourceRootsFromStorage() {
  settings.loadSourceRootsFromStorage(state);
}
function loadSourceRootEnabledFromStorage() {
  settings.loadSourceRootEnabledFromStorage(state);
}
function loadMasterDbEnabledFromStorage() {
  settings.loadMasterDbEnabledFromStorage(state);
}
function loadSourcesEverConfiguredFromStorage() {
  settings.loadSourcesEverConfiguredFromStorage(state);
}
async function loadUsbDevices() {
  const rows = await usb.loadUsbDevices(state, command);
  renderUsbRecentRoots();
  return rows;
}
function persistMasterDbEnabled(enabled) {
  settings.persistMasterDbEnabled(command, enabled);
}
function persistSourcesEverConfigured(value) {
  settings.persistSourcesEverConfigured(command, value);
}
const pruneUsbDevice = async (id) => usb.pruneUsbDevice(state, id, { command, reload: loadUsbDevices });

// --- Playlist closures ---

function promptNewPlaylist() {
  playlist.promptNewPlaylist(el, {
    document,
    requestAnimationFrame: window.requestAnimationFrame.bind(window),
    createPlaylist,
    setStatus,
    emitStatus,
  });
}
function startPlaylistRename(playlistId) {
  playlist.startPlaylistRename(playlistId, state, el, {
    document,
    requestAnimationFrame: window.requestAnimationFrame.bind(window),
    command,
    setStatus,
    emitStatus,
    renderPlaylistSidebarItemContent,
    getCurrentPlaylist,
    formatPlaylistExportStatus,
  });
}
const loadPlaylists = async () => playlist.loadPlaylists(state, {
    command,
    renderPlaylistTabsAndPanels: renderPlaylistList,
    updatePlaylistExportButtons,
  });
// Fetch the open playlist's tracks from the backend and render. `ctl.load`
// replaces `getCurrentPlaylist().tracks` via setItems.
async function refreshCurrentPlaylistTracks() {
  const playlist = getCurrentPlaylist();
  if (!playlist) return;
  playlistTracksCtl.query = String(state.playlistTrackSearch || "");
  await playlistTracksCtl.load({ scopeId: playlist.id });
  renderPlaylistPanelChrome();
}
const createPlaylist = async (name) => playlist.createPlaylist(name, {
    setStatus,
    emitStatus,
    withProgress,
    command,
    loadPlaylists,
    state,
    updateModeText,
    switchTab,
  });
const deletePlaylist = async (playlistId) => playlist.deletePlaylist(playlistId, {
    state,
    openConfirmDialog: (opts) => confirmDialog.open(opts),
    command,
    loadPlaylists,
    updateModeText,
    switchTab,
    setStatus,
    emitStatus,
  });
const addTracksToCurrentPlaylist = async (tracks) => playlist.addTracksToCurrentPlaylist(tracks, {
    requireCurrentPlaylist,
    pushEventLog,
    setStatus,
    emitStatus,
    withProgress,
    command,
    refreshCurrentPlaylistTracks,
    promoteTrackIdentity,
    usbRoot: state.usbRoot,
    usbRootValid: state.usbRootValid,
  });

// --- USB closures ---

function renderUsbPlaylists() {
  usb.renderUsbPlaylists(state, el, { escapeHtml });
}

// The USB-playlist track table's data layer: paginated + searched + sorted +
// per-page-hydrated by the backend (fetch_usb_playlist_tracks), rendered via
// the shared controller. Selection/search/sort/scroll all go through it.
const usbPlaylistTracksCtl = createTrackListController({
  bodyId: "usbPlaylistTracks",
  getElements: () => ({
    body: el.usbPlaylistTracks,
    wrap: el.usbPlaylistTracks?.closest?.(".table-wrap"),
    durationTarget: el.usbPlaylistTotalDuration,
  }),
  fetchPage: ({ scopeId, query, sortBy, sortDir, cursor, limit }) =>
    command("fetch_usb_playlist_tracks", {
      usbRoot: state.usbRoot || null,
      id: scopeId,
      query,
      sortBy: sortBy || null,
      sortDir: sortDir || null,
      cursor: cursor || null,
      limit,
    }),
  normalize: (track) => normalizeTrack(track, "usb"),
  rowOptions: usb.usbPlaylistRowOptions,
  renderTrackTable,
  renderDurationSummary: (target, summary) =>
    renderTrackListDurationSummary(target, summary, formatDurationMs),
  getTableSortState: () => tableSortState,
});
// Sort-header clicks route here via bodyToRendererMap.
async function renderUsbPlaylistTracks() {
  await usbPlaylistTracksCtl.applyHeaderSort();
}
function renderHistoryList() {
  usb.renderHistoryList(state, el, { escapeHtml, getHistoryDateValue });
}

// USB-history track table -- same shared controller over fetch_usb_history_tracks.
const usbHistoryTracksCtl = createTrackListController({
  bodyId: "historyTracks",
  getElements: () => ({
    body: el.historyTracks,
    wrap: el.historyTracks?.closest?.(".table-wrap"),
    durationTarget: el.historyTotalDuration,
  }),
  fetchPage: ({ scopeId, query, sortBy, sortDir, cursor, limit }) =>
    command("fetch_usb_history_tracks", {
      usbRoot: state.usbRoot || null,
      id: scopeId,
      query,
      sortBy: sortBy || null,
      sortDir: sortDir || null,
      cursor: cursor || null,
      limit,
    }),
  normalize: (track) => normalizeTrack(track, "hist"),
  rowOptions: usb.usbHistoryRowOptions,
  renderTrackTable,
  renderDurationSummary: (target, summary) =>
    renderTrackListDurationSummary(target, summary, formatDurationMs),
  getTableSortState: () => tableSortState,
});
async function renderHistoryTracks() {
  await usbHistoryTracksCtl.applyHeaderSort();
}
function renderUsbPlayerMenuEditor() {
  usb.renderUsbPlayerMenuEditor(state, el, { documentObj: document });
}
function syncUsbPlayerMenuEditorControls() {
  usb.syncUsbPlayerMenuEditorControls(state, el);
}
function handleUsbPlayerMenuListClick(side, event) {
  usb.handleUsbPlayerMenuListClick(state, el, { documentObj: document }, side, event);
}
function resetUsbStateViews({ hideDiagnostics = true } = {}) {
  usb.resetUsbStateViews(state, el, {
    renderUsbPlaylists,
    clearUsbPlaylistTracks: () => usbPlaylistTracksCtl.clear(),
    renderHistoryList,
    clearHistoryTracks: () => usbHistoryTracksCtl.clear(),
    renderUsbPlayerMenuEditor,
    hideDiagnostics,
  });
}
function showDiagReportView() {
  usb.showDiagReportView(el);
}
function showDiagRepairView() {
  usb.showDiagRepairView(el);
}
const hydrateUsbTrackMetadata = (track) => usb.hydrateUsbTrackMetadata(state, track, {
    usbTrackNeedsHydration,
    command,
    normalizeTrack,
  });

function loadUsbRootFromStorage() {
  usb.loadUsbRootFromStorage(state, el, {
    localStorageObj: localStorage,
    storageKeyUsbRoot: STORAGE_KEY_USB_ROOT,
    updateUsbRootText,
    updateUsbConfigControlsVisibility,
    updatePlaylistExportButtons,
  });
}
async function validateAndSetUsbRoot(path, silent = false) {
  const result = await usb.validateAndSetUsbRoot(state, el, path, silent, {
    command,
    persistUsbRoot,
    updateUsbRootText,
    resetUsbStateViews,
    updateUsbConfigControlsVisibility,
    updateUsbSubNavDisabledState,
    updatePlaylistExportButtons,
    setStatus,
    emitStatus,
    runUsbDiagnostics,
    updateUsbNameBadge,
    warn: (...a) => console.warn(...a),
    scheduler: (fn, ms) => window.setTimeout(fn, ms),
  });
  if (state.usbRoot) await loadUsbDevices();
  await syncAssetScopePaths();
  return result;
}
const initializeUsb = async () => usb.initializeUsb(state, el, {
    command,
    setStatus,
    emitStatus,
    validateAndSetUsbRoot,
    logError: (...a) => console.error(...a),
  });
const pickUsbFolder = async () => usb.pickUsbFolder({ invoke, validateAndSetUsbRoot, state, emitStatus });
const syncAssetScopePaths = async () => usb.syncAssetScopePaths(state, {
    invoke,
    warn: (...a) => console.warn(...a),
  });
const detectExternalMasterDb = async () => usb.detectExternalMasterDb(state, el, {
    command,
    warn: (...a) => console.warn(...a),
    renderSourceChips,
  });
const pickSourceFolders = async () => usb.pickSourceFolders({ invoke });

function renderDiagnosticsReport(data) {
  usb.renderDiagnosticsReport(el, data, {
    escapeHtml,
    showDiagReportView,
    updateUsbHealthDot,
    switchView,
    documentObj: document,
  });
}
function renderParityReport(data) {
  usb.renderParityReport(el, data, {
    escapeHtml,
    showDiagReportView,
    formatParityIssues: usb.formatParityIssues,
    documentObj: document,
  });
}
function renderRepairPreview(data) {
  usb.renderRepairPreview(el, data, {
    documentObj: document,
    showDiagRepairView: () => showDiagRepairView(),
    getSelectedFixIds: () => state.selectedRepairFixIds,
    setSelectedFixIds: (ids) => {
      state.selectedRepairFixIds = new Set(ids);
    },
    onToggleFixSelection: (id, checked) => {
      const fixId = String(id || "");
      if (!fixId) return;
      if (checked) state.selectedRepairFixIds.add(fixId);
      else state.selectedRepairFixIds.delete(fixId);
      el.applyRepairsBtn.disabled = state.selectedRepairFixIds.size === 0;
    },
  });
}

const refreshUsb = async () => usb.refreshUsb(state, el, {
    setStatus,
    emitStatus,
    command,
    setProgress,
    startProgressHeartbeat,
    stopProgressHeartbeat,
    normalizeUsbPlaylist,
    renderUsbPlaylists,
    clearUsbPlaylistTracks: () => usbPlaylistTracksCtl.clear(),
    renderCurrentPlaylistTracksFromState,
    updatePlaylistExportButtons,
    countWarningsForStatus: eventLog.countWarningsForStatus,
    logWarnings,
  });
const removeUsbPlaylist = async (p) => usb.removeUsbPlaylist(state, p, {
    setStatus,
    emitStatus,
    openConfirmDialog: (opts) => confirmDialog.open(opts),
    command,
    refreshUsb,
    countWarningsForStatus: eventLog.countWarningsForStatus,
    clearUsbDiagnostics: () => usb.clearUsbDiagnostics(el),
  });
const reorderUsbPlaylists = async () => usb.reorderUsbPlaylists(state, el, {
    setStatus,
    emitStatus,
    command,
    refreshUsb,
    clearUsbDiagnostics: () => usb.clearUsbDiagnostics(el),
  });
const usbJobBaseDeps = {
  setStatus,
  emitStatus,
  command,
  setProgress,
  startProgressHeartbeat,
  stopProgressHeartbeat,
  logWarnings,
};

const runUsbDiagnostics = async () => usb.runUsbDiagnostics(state, {
    ...usbJobBaseDeps,
    updatePlaylistExportButtons,
    renderCurrentPlaylistTracksFromState,
    renderDiagnosticsReport,
  });
const runUsbParityReport = async () => usb.runUsbParityReport(state, {
    ...usbJobBaseDeps,
    renderParityReport,
  });
const previewUsbRepairs = async () => usb.previewUsbRepairs(state, {
    ...usbJobBaseDeps,
    renderRepairPreview,
  });
const applyUsbRepairs = async () => usb.applyUsbRepairs(state, {
    ...usbJobBaseDeps,
    resetUsbStateViews,
    updatePlaylistExportButtons,
    renderCurrentPlaylistTracksFromState,
    renderDiagnosticsReport,
  });
const refreshHistory = async () => usb.refreshHistory(state, el, {
    setStatus,
    emitStatus,
    command,
    normalizeTrack,
    countWarningsForStatus: eventLog.countWarningsForStatus,
    logWarnings,
    renderHistoryList,
    clearHistoryTracks: () => usbHistoryTracksCtl.clear(),
  });
const exportHistoryTracklist = async () => usb.exportHistoryTracklist(state, el, {
    setStatus,
    emitStatus,
    invoke,
    tracklistExportDialog,
    buildTracklistText,
  });
const usbPlayerMenuDeps = {
  setStatus,
  emitStatus,
  command,
  documentObj: document,
  clearUsbDiagnostics: () => usb.clearUsbDiagnostics(el),
};

const loadUsbPlayerMenuConfig = async () => usb.loadUsbPlayerMenuConfig(state, el, usbPlayerMenuDeps);
const addUsbPlayerMenuItems = async () => usb.addUsbPlayerMenuItems(state, el, usbPlayerMenuDeps);
const removeUsbPlayerMenuItems = async () => usb.removeUsbPlayerMenuItems(state, el, usbPlayerMenuDeps);
const moveUsbPlayerMenuItems = async (direction) => usb.moveUsbPlayerMenuItems(state, el, usbPlayerMenuDeps, direction);
const syncUsbPlayerMenusEdbToPdb = async () => usb.syncUsbPlayerMenusEdbToPdb(state, el, usbPlayerMenuDeps);
const exportPlaylistToUsb = async (playlistId) => usb.exportPlaylistToUsb(state, el, playlistId, {
    setStatus,
    emitStatus,
    emitMessage,
    setProgress,
    startProgressHeartbeat,
    nextPaint: jobMgr.nextPaint,
    command,
    stopProgressHeartbeat,
    countWarningsForStatus: eventLog.countWarningsForStatus,
    warningEntryLevel: eventLog.warningEntryLevel,
    logWarnings,
    pushEventLog,
    loadPlaylists,
    updateModeText,
    switchView,
    renderUsbPlaylists,
    clearUsbPlaylistTracks: () => usbPlaylistTracksCtl.clear(),
    refreshMissingSourceRoots: checkSourceRoots,
    clearUsbDiagnostics: () => usb.clearUsbDiagnostics(el),
    commitActivePlaylistSort,
  });

// --- Bootstrap closures ---

function setUsbRootControlsLocked(locked) {
  usb.setUsbRootControlsLocked(state, el, locked, { updatePlaylistExportButtons });
}
function handleJobEvent(payload) {
  jobMgr.handleJobEvent(state, el, payload, {
    debugFrontendLog,
    pushEventLog,
    applyRealtimeAnalyzedTrackUpdate,
    setStatus,
    emitMessage,
    refreshSourceRootAnalysisStatus,
    applyLibraryDurationSummary,
    setTrackAnalyzingState,
    setUsbRootControlsLocked,
  });
}
function handleBackendLogEvent(payload) {
  bootstrap.handleBackendLogEvent(payload, { pushEventLog });
}

const registerBackendJobEvents = async () => bootstrap.registerBackendJobEvents(state, {
    isTauriRuntime,
    unregisterBackendJobEvents,
    getTauriEventListen,
    handleJobEvent,
    handlePlaybackEvent,
    handleBackendLogEvent,
  });
const unregisterBackendJobEvents = async () => bootstrap.unregisterBackendJobEvents(state, {
    warn: (...a) => console.warn(...a),
  });

async function switchView(viewId) {
  const switched = await bootstrap.switchView(state, el, viewId, {
    staticTabs: STATIC_TABS,
    stopPlaybackIfActive,
    syncLibraryOnboardingMode,
    updateModeText,
    populatePlaylistPanel,
    refreshCurrentPlaylistTracks,
    renderEventLog,
    renderBackups,
    requestAnimationFrameFn: window.requestAnimationFrame.bind(window),
    documentObj: document,
    renderWaveformsIn,
    commitActivePlaylistSort,
    emitStatus,
  });
  if (viewId === "usb-player-menu") {
    renderUsbPlayerMenuEditor();
    await loadUsbPlayerMenuConfig();
  }
  return switched;
}
const switchTab = async (tab) => bootstrap.switchTab(state, el, tab, {
    staticTabs: STATIC_TABS,
    stopPlaybackIfActive,
    syncLibraryOnboardingMode,
    updateModeText,
    populatePlaylistPanel,
    refreshCurrentPlaylistTracks,
    renderEventLog,
    renderBackups,
    requestAnimationFrameFn: window.requestAnimationFrame.bind(window),
    documentObj: document,
    renderWaveformsIn,
    commitActivePlaylistSort,
    emitStatus,
  });
const hydrateAppVersionLabel = () => bootstrap.hydrateAppVersionLabel(el, {
    appVersionFallback: APP_VERSION_FALLBACK,
    tauriIsTauri,
    tauriGetVersion,
  });
const checkForUpdate = () => bootstrap.checkForUpdate(state, el, {
    resolveVersion: async () => {
      if (!tauriIsTauri()) return null;
      try {
        const version = await tauriGetVersion();
        return version && String(version).trim() ? String(version).trim() : null;
      } catch {
        return null;
      }
    },
    fetchUpdateInfo: (version) =>
      fetchUpdateInfoRemote(version, {
        fetchFn: typeof fetch !== "undefined" ? fetch.bind(window) : null,
      }),
    renderUpdateNotice: (s, e) =>
      renderUpdateNotice(s, e, { openUrl: (url) => openExternalUrl(window, url) }),
    renderCriticalUpdateBanner: (s, e) =>
      renderCriticalUpdateBanner(s, e, {
        localStorageObj: localStorage,
        openUrl: (url) => openExternalUrl(window, url),
      }),
  });
function restoreStoredUiPrefs() {
  bootstrap.restoreStoredUiPrefs(state, el, {
    localStorageObj: localStorage,
    constants: {
      STORAGE_KEY_EXPORT_PRUNE_STALE,
      STORAGE_KEY_EXPORT_BACKUP,
      STORAGE_KEY_BACKUP_RETENTION_COUNT,
      STORAGE_KEY_ANALYSIS_BPM_RANGE,
      STORAGE_KEY_ANALYSIS_ENGINE,
      STORAGE_KEY_SIDEBAR_COLLAPSED,
    },
    normalizeAnalysisBpmRange: library.normalizeAnalysisBpmRange,
    defaultAnalysisBpmRange: library.DEFAULT_ANALYSIS_BPM_RANGE,
  });
}
function applySidebarCollapsedUi() {
  bootstrap.applySidebarCollapsedUi(state, el, { sidebarExpandBtn });
  document.body.classList.toggle("sidebar-collapsed", !!state.sidebarCollapsed);
}
function showHelpOnFirstVisit() {
  bootstrap.showHelpOnFirstVisit(el, {
    localStorageObj: localStorage,
    storageKeyHelpSeen: STORAGE_KEY_HELP_SEEN,
  });
}
const runDeferredInitialLoad = () => bootstrap.runDeferredInitialLoad(state, {
    setTimeoutFn: (cb) => setTimeout(cb, 0),
    withProgress,
    loadPlaylists,
    resetAndLoadLibraryTracks,
    libraryLoadLimitInit: LIBRARY_LOAD_LIMIT_INIT,
    updateModeText,
    updateSelectionCount,
    clearUsbPlaylistTracks: () => usbPlaylistTracksCtl.clear(),
    renderWaveformsIn,
    documentObj: document,
    setStatus,
    logError: (e) => console.error(e),
  });

// --- Sidebar expand button ---

const sidebarExpandBtn = document.createElement("button");
sidebarExpandBtn.className = "sidebar-expand-btn";
sidebarExpandBtn.textContent = "\u25B8";
sidebarExpandBtn.dataset.tooltip = "Expand sidebar";
sidebarExpandBtn.setAttribute("aria-label", "Expand sidebar");
document.body.appendChild(sidebarExpandBtn);

// --- Bind events & init ---

function bindEvents() {
  const bindCtx = uiCtrl.createBindEventsContext(state, el, {
    document,
    window,
    navigator,
    eventLogStore,
    sidebarExpandBtn,
    confirmDialog,
    tracklistExportDialog,
    constants: {
      STORAGE_KEY_SIDEBAR_COLLAPSED,
      FRONTEND_DB_KEY_SIDEBAR_COLLAPSED,
      STORAGE_KEY_HELP_SEEN,
      FRONTEND_DB_KEY_HELP_SEEN,
      STORAGE_KEY_EXPORT_PRUNE_STALE,
      FRONTEND_DB_KEY_EXPORT_PRUNE_STALE,
      STORAGE_KEY_EXPORT_BACKUP,
      FRONTEND_DB_KEY_EXPORT_BACKUP,
      STORAGE_KEY_BACKUP_RETENTION_COUNT,
      FRONTEND_DB_KEY_BACKUP_RETENTION_COUNT,
      STORAGE_KEY_ANALYSIS_BPM_RANGE,
      FRONTEND_DB_KEY_ANALYSIS_BPM_RANGE,
      STORAGE_KEY_ANALYSIS_ENGINE,
      FRONTEND_DB_KEY_ANALYSIS_ENGINE,
      LIBRARY_LOAD_LIMIT_DEFAULT,
    },
    setStatus,
    emitStatus,
    closeSettingsDrawer,
    renderEventLog,
    renderBackups,
    hideUsbDiagnostics: usb.hideUsbDiagnostics,
    clearUsbDiagnostics: usb.clearUsbDiagnostics,
    resetUsbStateViews,
    switchView,
    deletePlaylist,
    startPlaylistRename,
    promptNewPlaylist,
    persistSetting,
    openConfirmDialog: (opts) => confirmDialog.open(opts),
    renderSourceChips,
    syncAssetScopePaths,
    applySearchLocalFilter,
    updateSelectionCount,
    updateSourceFilterIndicator,
    command,
    getTauriEventListen,
    setProgress,
    resetAndLoadLibraryTracks,
    refreshCurrentPlaylistTracks,
    clearPlaylistTrackSort,
    withProgress,
    persistSourceRoots,
    persistSourceRootEnabled,
    persistMasterDbEnabled,
    persistSourcesEverConfigured,
    enabledSourceRoots: library.enabledSourceRoots,
    pickSourceFolders,
    relocateSourceRoot,
    scanLibrary,
    scanMasterDb,
    analyzeSelectedTracks,
    LIBRARY_LOAD_LIMIT_DEFAULT,
    dismissProgress,
    toggleAnalysisPause,
    cancelAnalysis,
    refreshUsb,
    pickUsbFolder,
    validateAndSetUsbRoot,
    initializeUsb,
    normalizeAnalysisBpmRange: library.normalizeAnalysisBpmRange,
    pushEventLog,
    updatePlaylistExportButtons,
    runUsbParityReport,
    runUsbDiagnostics,
    previewUsbRepairs,
    applyUsbRepairs,
    showDiagReportView,
    refreshHistory,
    exportHistoryTracklist,
    loadUsbPlayerMenuConfig,
    renderUsbPlayerMenuEditor,
    syncUsbPlayerMenuEditorControls,
    handleUsbPlayerMenuListClick,
    addUsbPlayerMenuItems,
    removeUsbPlayerMenuItems,
    moveUsbPlayerMenuItems,
    syncUsbPlayerMenusEdbToPdb,
    scheduleApplySearchLocalFilter,
    usbPlaylistTracksCtl,
    usbHistoryTracksCtl,
    playlistTracksCtl,
    patchUsbTrackRow,
    patchHistoryTrackRow,
    addTracksToCurrentPlaylist,
    loadMoreLibraryTracks,
    pruneUsbDevice,
    getLibraryVisibleTracks,
    analyzeSingleTrack,
    getPlaybackUiStateHelpers: playback.getPlaybackUiStateHelpers,
    isTrackCurrentlyPlaying,
    stopPlaybackFromUi,
    playTrackFromOrigin,
    scrubRatioFromPointer: playback.scrubRatioFromPointer,
    removeUsbPlaylist,
    reorderUsbPlaylists,
    moveArrayItem: usb.moveArrayItem,
    stopPlaybackIfActive,
    hydrateUsbTrackMetadata,
    formatDurationMs,
    setActiveListItem: shell.setActiveListItem,
    getHistoryDateDisplay,
    getCurrentPlaylist,
    renderCurrentPlaylistTracksFromState,
    commitActivePlaylistSort,
    isPlaylistSortActive: () => !!tableSortState.playlistTracksBody,
    loadPlaylists,
    updateModeText,
    exportPlaylistToUsb,
    analyzeTrackIds,
    resolveLocalTrackId,
    handleSortHeaderClick,
    handleLibraryTableWrapScroll,
    renderLibraryRows,
    libraryTracksCtl,
    hydrateLoadedTracksPreviewsInBackground,
  });
  return uiCtrl.bindEvents(bindCtx);
}

const init = async () => bootstrap.initApp(state, {
    el,
    constants: { STORAGE_KEY_HELP_SEEN },
    hydrateLocalStorageFromFrontendSettingsDb: () =>
      settings.hydrateLocalStorageFromFrontendSettingsDb(command, state),
    themeInit: () => ThemeManager.init(),
    accentInit: () => AccentManager.init(),
    hydrateAppVersionLabel,
    checkForUpdate,
    setupConsoleFileLogging: () =>
      eventLog.setupConsoleFileLogging({
        isTauriRuntime,
        invoke,
        pushEventLog,
      }),
    setupRuntimeErrorLogging: () =>
      eventLog.setupRuntimeErrorLogging({ pushEventLog }),
    pushEventLog,
    setProgress,
    loadSourceRootsFromStorage,
    loadSourceRootEnabledFromStorage,
    loadMasterDbEnabledFromStorage,
    loadSourcesEverConfiguredFromStorage,
    loadUsbDevices,
    renderUsbRecentRoots,
    persistSourceRootEnabled,
    syncAssetScopePaths,
    loadUsbRootFromStorage,
    restoreStoredUiPrefs,
    applySidebarCollapsedUi,
    checkSourceRoots,
    renderSourceChips,
    refreshSourceRootAnalysisStatus,
    detectExternalMasterDb,
    bindEvents,
    switchView,
    showHelpOnFirstVisit,
    invoke,
    registerBackendJobEvents,
    handleBackendLogEvent,
    updateUsbRootText,
    runDeferredInitialLoad,
    logInfo: (...a) => console.info(...a),
    logError: (...a) => console.error(...a),
    warn: (...a) => console.warn(...a),
  });

window.addEventListener("resize", () => {
  renderWaveformsIn(document);
});
playback.bindBeforeUnloadCleanup(window, unregisterBackendJobEvents);

init().catch((error) => {
  state.startupPhase = false;
  console.error(error);
  setStatus(`Initialization failed: ${error.message}`);
});
