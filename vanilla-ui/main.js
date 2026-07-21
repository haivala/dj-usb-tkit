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
import * as library from "./components/library/actions.mjs";
import * as settings from "./components/settings/actions.mjs";
import * as shell from "./components/shell/actions.mjs";
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
  STORAGE_KEY_ANALYSIS_BPM_RANGE,
  STORAGE_KEY_ANALYSIS_ENGINE,
  STORAGE_KEY_SIDEBAR_COLLAPSED,
  STORAGE_KEY_HELP_SEEN,
  FRONTEND_DB_KEY_THEME,
  FRONTEND_DB_KEY_ACCENT_HUE,
  FRONTEND_DB_KEY_EXPORT_PRUNE_STALE,
  FRONTEND_DB_KEY_EXPORT_BACKUP,
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
  updateTrackListDurationSummary,
  getHistoryDateValue,
  getHistoryDateDisplay,
  formatTimestampLocal,
  filterTracksByQuery,
} from "./track_utils.mjs";
import { createApiClient } from "./api_client.mjs";
import * as jobMgr from "./job_manager.mjs";
import * as bootstrap from "./startup_bootstrap.mjs";
import * as uiCtrl from "./ui_controller.mjs";
import { createMessageBus } from "./message_bus.mjs";
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
const LIBRARY_AUTOFILL_MAX_PAGES = 0;
const APP_VERSION_FALLBACK = "Not set";

const state = createInitialState();
const tableSortState = createTableSortState();
const eventLogStore = createEventLogState();

const { invoke, command, isTauriRuntime, getTauriEventListen } =
  createApiClient({
    tauriInvoke,
    tauriIsTauri,
    tauriListen,
    state,
    normalizePath: library.normalizePath,
    constants: { LIBRARY_LOAD_LIMIT_DEFAULT, LIBRARY_LOAD_LIMIT_POST_SCAN },
  });

let ThemeManager, AccentManager;

const el = {
  statusText: document.getElementById("statusText"),
  playlistBadge: document.getElementById("playlistBadge"),
  badgeLabel: document.getElementById("badgeLabel"),
  navSidebar: document.getElementById("navSidebar"),
  sidebarCollapseBtn: document.getElementById("sidebarCollapseBtn"),
  donateBtn: document.getElementById("donateBtn"),
  navPlaylistList: document.getElementById("navPlaylistList"),
  addPlaylistBtn: document.getElementById("addPlaylistBtn"),
  panels: {
    library: document.getElementById("panel-library"),
    usb: document.getElementById("panel-usb"),
    "usb-playlists": document.getElementById("panel-usb-playlists"),
    "usb-history": document.getElementById("panel-usb-history"),
    "usb-player-menu": document.getElementById("panel-usb-player-menu"),
    "event-log": document.getElementById("panel-event-log"),
    playlist: document.getElementById("panel-playlist"),
  },
  playlistPanelTitle: document.getElementById("playlistPanelTitle"),
  playlistSearchInput: document.getElementById("playlistSearchInput"),
  playlistTracksBody: document.getElementById("playlistTracksBody"),
  playlistTableWrap: document.getElementById("playlistTableWrap"),
  playlistEmptyState: document.getElementById("playlistEmptyState"),
  playlistTotalDuration: document.getElementById("playlistTotalDuration"),
  playlistExportStatus: document.getElementById("playlistExportStatus"),
  analyzePlaylistMissingBtn: document.getElementById(
    "analyzePlaylistMissingBtn",
  ),
  exportPlaylistBtn: document.getElementById("exportPlaylistBtn"),
  settingsBtn: document.getElementById("settingsBtn"),
  settingsDrawer: document.getElementById("settingsDrawer"),
  settingsBackdrop: document.getElementById("settingsBackdrop"),
  settingsCloseBtn: document.getElementById("settingsCloseBtn"),
  settingsVersionText: document.getElementById("settingsVersionText"),
  settingsUpdateNote: document.getElementById("settingsUpdateNote"),
  criticalUpdateBanner: document.getElementById("criticalUpdateBanner"),
  criticalUpdateText: document.getElementById("criticalUpdateText"),
  criticalUpdateDismissBtn: document.getElementById("criticalUpdateDismissBtn"),
  openEventLogBtn: document.getElementById("openEventLogBtn"),
  accentHueSlider: document.getElementById("accentHueSlider"),
  accentSwatch: document.getElementById("accentSwatch"),
  accentResetBtn: document.getElementById("accentResetBtn"),
  sourceFilterIndicator: document.getElementById("sourceFilterIndicator"),
  selectionActions: document.getElementById("selectionActions"),
  usbConnectionBar: document.getElementById("usbConnectionBar"),
  usbSelectedControls: document.getElementById("usbSelectedControls"),
  usbInitRow: document.getElementById("usbInitRow"),
  usbInitHint: document.getElementById("usbInitHint"),
  usbHealthDot: document.getElementById("usbHealthDot"),
  initializeUsbBtn: document.getElementById("initializeUsbBtn"),
  sourceChipsContainer: document.getElementById("sourceChipsContainer"),
  sourceBar: document.getElementById("sourceBar"),
  sourceFilterHeader: document.getElementById("sourceFilterHeader"),
  addSourceBtn: document.getElementById("addSourceBtn"),
  importMasterDbBtn: document.getElementById("importMasterDbBtn"),
  librarySearch: document.getElementById("librarySearch"),
  libraryTableBody: document.getElementById("libraryTableBody"),
  libraryTableWrap: document.getElementById("libraryTableWrap"),
  libraryEmptyState: document.getElementById("libraryEmptyState"),
  libraryContent: document.getElementById("libraryContent"),
  selectAllTracks: document.getElementById("selectAllTracks"),
  selectionCount: document.getElementById("selectionCount"),
  usbPlaylists: document.getElementById("usbPlaylists"),

  usbTrackSearch: document.getElementById("usbTrackSearch"),
  usbPlaylistTracks: document.getElementById("usbPlaylistTracks"),
  usbPlaylistTotalDuration: document.getElementById("usbPlaylistTotalDuration"),
  historyList: document.getElementById("historyList"),

  historyTrackSearch: document.getElementById("historyTrackSearch"),
  historyTracks: document.getElementById("historyTracks"),
  historyTotalDuration: document.getElementById("historyTotalDuration"),

  usbPlayerMenuAvailable: document.getElementById("usbPlayerMenuAvailable"),
  usbPlayerMenuCurrent: document.getElementById("usbPlayerMenuCurrent"),
  usbPlayerMenuAddBtn: document.getElementById("usbPlayerMenuAddBtn"),
  usbPlayerMenuRemoveBtn: document.getElementById("usbPlayerMenuRemoveBtn"),
  usbPlayerMenuUpBtn: document.getElementById("usbPlayerMenuUpBtn"),
  usbPlayerMenuDownBtn: document.getElementById("usbPlayerMenuDownBtn"),
  usbPlayerMenuDivergence: document.getElementById("usbPlayerMenuDivergence"),
  usbPlayerMenuDivergenceMessage: document.getElementById("usbPlayerMenuDivergenceMessage"),
  usbPlayerMenuSyncBtn: document.getElementById("usbPlayerMenuSyncBtn"),
  usbPlayerMenuRestoreBtn: document.getElementById("usbPlayerMenuRestoreBtn"),
  libraryTotalDuration: document.getElementById("libraryTotalDuration"),
  scanLibraryBtn: document.getElementById("scanLibraryBtn"),
  addSelectedBtn: document.getElementById("addSelectedBtn"),
  refreshUsbBtn: document.getElementById("refreshUsbBtn"),
  refreshHistoryBtn: document.getElementById("refreshHistoryBtn"),
  runUsbParityBtn: document.getElementById("runUsbParityBtn"),
  exportSyncModeGroup: document.getElementById("exportSyncModeGroup"),
  exportSyncModeMirror: document.getElementById("exportSyncModeMirror"),
  exportSyncModeAdditive: document.getElementById("exportSyncModeAdditive"),
  exportBackupCheckbox: document.getElementById("exportBackupCheckbox"),
  analysisBpmRangeSelect: document.getElementById("analysisBpmRangeSelect"),
  analysisEngineSelect: document.getElementById("analysisEngineSelect"),
  analysisEngineStatus: document.getElementById("analysisEngineStatus"),
  essentiaInstallRow: document.getElementById("essentiaInstallRow"),
  essentiaNodeStatus: document.getElementById("essentiaNodeStatus"),
  essentiaDownloadBtn: document.getElementById("essentiaDownloadBtn"),
  essentiaCancelBtn: document.getElementById("essentiaCancelBtn"),
  essentiaRemoveBtn: document.getElementById("essentiaRemoveBtn"),
  selectUsbFolderBtn: document.getElementById("selectUsbFolderBtn"),
  usbRecentRow: document.getElementById("usbRecentRow"),
  usbRecentList: document.getElementById("usbRecentList"),
  usbRootPathText: document.getElementById("usbRootPathText"),
  externalMasterDbToggle: document.getElementById("externalMasterDbToggle"),
  externalMasterDbCheckbox: document.getElementById("externalMasterDbCheckbox"),
  externalMasterDbPath: document.getElementById("externalMasterDbPath"),
  usbCountsText: document.getElementById("usbCountsText"),
  historyCountsText: document.getElementById("historyCountsText"),
  progressFooter: document.getElementById("progressFooter"),
  progressText: document.getElementById("progressText"),
  progressFill: document.getElementById("progressFill"),
  progressDismiss: document.getElementById("progressDismiss"),
  usbDiagnosticsCard: document.getElementById("usbDiagnosticsCard"),
  diagOverallStatus: document.getElementById("diagOverallStatus"),
  diagDuration: document.getElementById("diagDuration"),
  diagSections: document.getElementById("diagSections"),
  diagReportView: document.getElementById("diagReportView"),
  diagRepairPanel: document.getElementById("diagRepairPanel"),
  diagRepairSummary: document.getElementById("diagRepairSummary"),
  diagRepairFixes: document.getElementById("diagRepairFixes"),
  diagBackToReportBtn: document.getElementById("diagBackToReportBtn"),
  diagPlaylistDetails: document.getElementById("diagPlaylistDetails"),
  diagPlaylistTableBody: document.getElementById("diagPlaylistTableBody"),
  reDiagnoseBtn: document.getElementById("reDiagnoseBtn"),
  previewRepairsBtn: document.getElementById("previewRepairsBtn"),
  applyRepairsBtn: document.getElementById("applyRepairsBtn"),
  confirmOverlay: document.getElementById("confirmOverlay"),
  confirmTitle: document.getElementById("confirmTitle"),
  confirmMessage: document.getElementById("confirmMessage"),
  confirmOkBtn: document.getElementById("confirmOkBtn"),
  confirmCancelBtn: document.getElementById("confirmCancelBtn"),
  helpBtn: document.getElementById("helpBtn"),
  helpOverlay: document.getElementById("helpOverlay"),
  helpCloseBtn: document.getElementById("helpCloseBtn"),
  eventLogLevelFilter: document.getElementById("eventLogLevelFilter"),
  eventLogSourceFilter: document.getElementById("eventLogSourceFilter"),
  eventLogClearBtn: document.getElementById("eventLogClearBtn"),
  eventLogSummary: document.getElementById("eventLogSummary"),
  eventLogList: document.getElementById("eventLogList"),
};

const confirmDialog = uiCtrl.createConfirmDialogController(el);

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

function setProgress(active, percent = 0, text = "", opts = {}) {
  jobMgr.setProgress(state, el, active, percent, text, opts);
}
function dismissProgress() {
  jobMgr.dismissProgress(state, el);
}
function startProgressHeartbeat() {
  jobMgr.startProgressHeartbeat(state, el);
}
function stopProgressHeartbeat() {
  jobMgr.stopProgressHeartbeat(state);
}
function withProgress(label, fn) {
  return jobMgr.withProgress(state, el, label, fn);
}

const messageBus = createMessageBus({
  setStatusText: (text) => uiCtrl.setStatusText(el, text),
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

function emitMessage(input = {}) {
  return messageBus.emitMessage(input);
}

function setStatus(text, meta = {}) {
  const statusText = String(text || "");
  const eventLog = state.startupPhase
    ? {
      text: statusText,
      coalesceKey: "startup.status"
    }
    : null;
  emitMessage({
    level: meta.level || "info",
    source: meta.source || "ui",
    code: meta.code || null,
    status: { text: statusText },
    eventLog,
  });
}

function emitStatus(text, meta = {}) {
  return setStatus(text, meta);
}

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

function toPlayableUrl(path) {
  return playback.toPlayableUrl(path, {
    isTauriRuntime,
    tauriConvertFileSrc,
    windowObj: window,
  });
}
function normalizePath(value) {
  return library.normalizePath(value);
}

function normalizeTrack(track, fallbackIdPrefix = "t") {
  return library.normalizeTrack(track, fallbackIdPrefix, {
    toPlayableUrl,
    appendUrlRevision: library.appendUrlRevision,
    normalizeDurationMs,
  });
}

function buildCoverSrcCandidates(track) {
  return library.buildCoverSrcCandidates(track, { toPlayableUrl });
}
function attachCoverFallbackHandlers(root = document) {
  return library.attachCoverFallbackHandlers(root, { document });
}

function trackHasCoreAnalysis(track) {
  return library.trackHasCoreAnalysis(track, {
    trackHasRenderableWaveform: library.trackHasRenderableWaveform,
    trackHasBpm: library.trackHasBpm,
  });
}

function isUsbOriginTrack(track) {
  return library.isUsbOriginTrack(track, {
    usbRoot: state.usbRoot || "",
    normalizePath,
  });
}

function resolveMissingAnalysisPieces(track) {
  return library.resolveMissingAnalysisPieces(track, {
    trackHasArtwork: library.trackHasArtwork,
    trackArtworkChecked: library.trackArtworkChecked,
    trackHasRenderableWaveform: library.trackHasRenderableWaveform,
    trackHasBpm: library.trackHasBpm,
  });
}

function usbTrackNeedsHydration(track) {
  return library.usbTrackNeedsHydration(track, {
    trackHasRenderableWaveform: library.trackHasRenderableWaveform,
    trackHasArtwork: library.trackHasArtwork,
    trackArtworkChecked: library.trackArtworkChecked,
    trackHasBpm: library.trackHasBpm,
    trackHasKey: library.trackHasKey,
  });
}

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
function updateAddToPlaylistButtons() {
  uiCtrl.updateAddToPlaylistButtons(state, document);
}
function updateSelectionCount() {
  uiCtrl.updateSelectionCount(state, el);
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
  usb.renderUsbRecentRoots(el, state.usbRecentRoots, document);
}

function patchTrackAnalysisFields(track, payload) {
  return library.patchTrackAnalysisFields(track, payload, { toPlayableUrl });
}

function normalizeUsbPlaylist(p) {
  return library.normalizeUsbPlaylist(p, { normalizeTrack });
}

function renderPlaylistSidebarItemContent(p) {
  return playlist.renderPlaylistSidebarItemContent(p, { escapeHtml });
}
function updatePlaylistPanelTitle(p) {
  playlist.updatePlaylistPanelTitle(el, p, { formatDurationMs });
}
function formatPlaylistExportStatus(p) {
  return playlist.formatPlaylistExportStatus(p, { formatTimestampLocal });
}
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
function renderPlaylistTabsAndPanels() {
  renderPlaylistList();
}

function updatePlaylistExportButtons() {
  playlist.updatePlaylistExportButtons(state, el, {
    getCurrentPlaylist,
    computeExportButtonState: usb.computeExportButtonState,
    isUsbOriginTrack,
    trackHasCoreAnalysis,
  });
}

function createTrackRow(track, options) {
  return trackTable.createTrackRow(track, options, {
    state,
    buildCoverSrcCandidates,
    isTrackCurrentlyPlaying,
    escapeHtml,
    trackHasCoreAnalysis,
    getKeyHue,
  });
}

function renderTrackTable(tbody, tracks, options = {}) {
  trackTable.renderTrackTable(tbody, tracks, options, {
    createTrackRow,
    attachCoverFallbackHandlers,
    renderWaveformsIn,
    setWaveformColorData,
    updateTransportButtonsInDom,
    escapeHtml,
    setStatus,
  });
}

function applySortToTracks(tracks, tbodyId) {
  return shell.applySortToTracks(tableSortState, tracks, tbodyId, {
    sortTracks: trackTable.sortTracks,
  });
}

function handleSortHeaderClick(e) {
  shell.handleSortHeaderClick(tableSortState, e, {
    renderMap: {
      renderLibraryRows,
      renderUsbPlaylistTracks,
      renderHistoryTracks,
      renderCurrentPlaylistTracksFromState,
    },
    bodyToRendererMap: {
      libraryTableBody: "renderLibraryRows",
      usbPlaylistTracks: "renderUsbPlaylistTracks",
      historyTracks: "renderHistoryTracks",
      playlistTracksBody: "renderCurrentPlaylistTracksFromState",
    },
    doc: document,
  });
}

// --- Playback closures ---

function updateTransportButtonsInDom() {
  playback.updateTransportButtonsInDom(state, document);
}
function clearAllWaveformPlayheads() {
  playback.clearAllWaveformPlayheads(document);
}
function setWaveformPlayhead(element, fraction, playing) {
  playback.setWaveformPlayhead(element, fraction, playing);
}
function resolveLocalTrackId(track) {
  return playback.resolveLocalTrackId(track, state, { normalizePath });
}
function resolveLocalTrack(track) {
  return playback.resolveLocalTrack(track, state);
}
function shouldAllowResolvedFallback(track) {
  return playback.shouldAllowResolvedFallback(track, state, { normalizePath });
}
function getTrackPlaybackPath(track) {
  return playback.getTrackPlaybackPath(track, { resolveLocalTrack });
}
function isTrackCurrentlyPlaying(track) {
  return playback.isTrackCurrentlyPlaying(track, state, {
    normalizePath,
    getTrackPlaybackPath,
  });
}

async function resolveLocalTrackIdAsync(track) {
  return playback.resolveLocalTrackIdAsync(track, state, {
    command,
    normalizePath,
    promoteTrackIdentity,
    resolveLocalTrackId,
    shouldAllowResolvedFallback,
  });
}
function promoteTrackIdentity(oldId, newId) {
  library.promoteTrackIdentity(state, el, oldId, newId, { cssEscape });
}

async function resolveLocalTrackForPlayback(track) {
  return playback.resolveLocalTrackForPlayback(track, state, {
    command,
    normalizeTrack,
    resolveLocalTrack,
    scoreLocalTrackCandidate: playback.scoreLocalTrackCandidate,
  });
}

async function stopPlaybackIfActive() {
  return playback.stopPlaybackIfActive(state, {
    command,
    clearAllWaveformPlayheads,
    updateTransportButtonsInDom,
    setStatus,
    warn: (...a) => console.warn(...a),
  });
}
async function stopPlaybackFromUi() {
  return playback.stopPlaybackFromUi(state, {
    command,
    clearAllWaveformPlayheads,
    updateTransportButtonsInDom,
    setStatus,
  });
}
async function playTrackFromOrigin(track, origin, options = {}) {
  return playback.playTrackFromOriginController(state, track, origin, options, {
    playTrackFromOriginCore: playback.playTrackFromOrigin,
    command,
    resolveLocalTrackForPlayback,
    trackPathMatchesAnyRoot: library.trackPathMatchesAnyRoot,
    clearAllWaveformPlayheads,
    setWaveformPlayhead,
    updateTransportButtonsInDom,
    setStatus,
    warn: (...a) => console.warn(...a),
  });
}

function handlePlaybackEvent(payload) {
  playback.handlePlaybackEvent(state, payload, {
    setWaveformPlayhead,
    updateTransportButtonsInDom,
    clearAllWaveformPlayheads,
    setStatus,
  });
}

// --- Analysis patch queue ---

const analysisPatchQueue = library.createAnalysisPatchQueue();
analysisPatchQueue.init(
  (id) => patchLibraryRowByTrackId(id),
  () => scheduleRealtimeTrackRender(),
  (cb) => setTimeout(cb, 0),
);

function patchLibraryRowByTrackId(trackId) {
  return library.patchLibraryRowByTrackId(state, el, trackId, {
    cssEscape,
    patchLibraryRowCells: (row, track) =>
      library.patchLibraryRowCells(row, track, {
        escapeHtml,
        getKeyHue,
        buildCoverSrcCandidates,
        attachCoverFallbackHandlers,
        drawWaveformCanvas,
        trackHasCoreAnalysis,
        invalidateWaveformCache,
        setWaveformColorData,
      }),
  });
}
function patchPlaylistRowByTrackId(trackId) {
  return library.patchPlaylistRowByTrackId(state, el, trackId, {
    cssEscape,
    getCurrentPlaylist,
    patchLibraryRowCells: (row, track) =>
      library.patchLibraryRowCells(row, track, {
        escapeHtml,
        getKeyHue,
        buildCoverSrcCandidates,
        attachCoverFallbackHandlers,
        drawWaveformCanvas,
        trackHasCoreAnalysis,
        invalidateWaveformCache,
        setWaveformColorData,
      }),
  });
}
function patchUsbTrackRow(track) {
  const trackId = String(track?.id || "").trim();
  if (!trackId) return false;
  const selector = `.track-grid-row[data-track-origin="usb"][data-track-id="${cssEscape(trackId)}"]`;
  const rows = el.usbPlaylistTracks?.querySelectorAll?.(selector) || [];
  if (!rows.length) return false;
  let patched = false;
  rows.forEach((row) => {
    if (library.patchLibraryRowCells(row, track, {
      escapeHtml,
      getKeyHue,
      buildCoverSrcCandidates,
      attachCoverFallbackHandlers,
      drawWaveformCanvas,
      trackHasCoreAnalysis,
      invalidateWaveformCache,
    })) {
      patched = true;
    }
  });
  return patched;
}
function patchHistoryTrackRow(track) {
  const trackId = String(track?.id || "").trim();
  if (!trackId) return false;
  const selector = `.track-grid-row[data-track-origin="usb"][data-track-id="${cssEscape(trackId)}"]`;
  const rows = el.historyTracks?.querySelectorAll?.(selector) || [];
  if (!rows.length) return false;
  let patched = false;
  rows.forEach((row) => {
    if (library.patchLibraryRowCells(row, track, {
      escapeHtml,
      getKeyHue,
      buildCoverSrcCandidates,
      attachCoverFallbackHandlers,
      drawWaveformCanvas,
      trackHasCoreAnalysis,
      invalidateWaveformCache,
    })) {
      patched = true;
    }
  });
  return patched;
}
function setTrackAnalyzingState(trackId, active) {
  library.setTrackAnalyzingState(state, trackId, active, {
    patchLibraryRowByTrackId,
    patchPlaylistRowByTrackId,
    trackHasCoreAnalysis,
    trackNeedsPreviewHydration: library.trackNeedsPreviewHydration,
    getLibraryVisibleTracks,
    updateLibraryDurationSummary,
    renderSourceChips,
  });
}

// --- Library closures ---

function getLibraryVisibleTracks() {
  return library.getLibraryVisibleTracks(state);
}

function renderLibraryRows() {
  library.renderLibraryRows(state, el, {
    getLibraryVisibleTracks,
    renderEmptyState,
    syncLibraryOnboardingMode,
    applySortToTracks,
    renderTrackTable,
    cssEscape,
    updateLibraryDurationSummary,
    onEnableMasterDb: () => scanMasterDb(),
  });
}
function renderSourceChips() {
  library.renderSourceChips(state, el, {
    documentObj: document,
    escapeHtml,
    trackPathMatchesAnyRoot: library.trackPathMatchesAnyRoot,
    trackHasCoreAnalysis,
    persistSourceRootEnabled,
    updateScanLibraryButtonLabel,
    updateSourceFilterIndicator,
  });
}
async function checkSourceRoots(options = {}) {
  return library.refreshMissingSourceRoots(state, {
    command,
    renderSourceChips,
    emitStatus,
    silent: options?.silent !== false,
  });
}
function applySearchLocalFilter() {
  library.applySearchLocalFilter(state, el, {
    renderLibraryRows,
    updateSelectionCount,
  });
}
function scheduleApplySearchLocalFilter() {
  library.scheduleApplySearchLocalFilter(state, el, {
    clearTimeoutFn: window.clearTimeout.bind(window),
    setTimeoutFn: window.setTimeout.bind(window),
    resetAndLoadLibraryTracks,
    setStatus,
    emitStatus,
    logError: (e) => console.error(e),
    debounceMs: LIBRARY_SEARCH_DEBOUNCE_MS,
  });
}
function updateLibraryDurationSummary(tracks) {
  library.updateLibraryDurationSummary(el, tracks, {
    trackHasCoreAnalysis,
    updateTrackListDurationSummary,
  });
}
function renderCurrentPlaylistTracksFromState() {
  library.renderCurrentPlaylistTracksFromState(state, el, {
    getCurrentPlaylist,
    filterTracksByQuery,
    renderEmptyState,
    applySortToTracks,
    renderTrackTable,
    cssEscape,
    updateTrackListDurationSummary,
  });
}
function scheduleRealtimeTrackRender() {
  library.scheduleRealtimeTrackRender(state, {
    clearTimeoutFn: window.clearTimeout.bind(window),
    setTimeoutFn: window.setTimeout.bind(window),
    applySearchLocalFilter,
    renderCurrentPlaylistTracksFromState,
    delayMs: 60,
  });
}
function mergeHydratedTrackIntoState(rawTrack) {
  return library.mergeHydratedTrackIntoState(state, rawTrack, {
    normalizeTrack,
  });
}

async function applyRealtimeAnalyzedTrackUpdate(payload) {
  return library.applyRealtimeAnalyzedTrackUpdate(state, payload, {
    patchTrackAnalysisFields,
    debugFrontendLog,
    log: (...a) => console.log(...a),
    warn: (...a) => console.warn(...a),
    patchLibraryRowByTrackId,
    scheduleRealtimeTrackRender,
    hydrateTrackPreviewFromBackend,
  });
}
async function hydrateTrackPreviewFromBackend(trackId, options = {}) {
  return library.hydrateTrackPreviewFromBackend(state, trackId, options, {
    command,
    mergeHydratedTrackIntoState,
    patchLibraryRowByTrackId,
    nextPaint: jobMgr.nextPaint,
    getLibraryVisibleTracks,
    updateLibraryDurationSummary,
    scheduleRealtimeTrackRender,
    renderSourceChips,
  });
}
async function hydrateLoadedTracksPreviewsInBackground() {
  return library.hydrateLoadedTracksPreviewsInBackground(state, {
    getLibraryVisibleTracks,
    command,
    mergeHydratedTrackIntoState,
    patchLibraryRowByTrackId,
    nextPaint: jobMgr.nextPaint,
    updateLibraryDurationSummary,
    scheduleRealtimeTrackRender,
    renderSourceChips,
    batchSize: 48,
  });
}
async function loadTracks(
  query = "",
  limit = LIBRARY_LOAD_LIMIT_DEFAULT,
  cursor = null,
  options = {},
) {
  return library.loadTracks(state, query, limit, cursor, options, {
    command,
    normalizeTrack,
    readLibraryPagination: library.readLibraryPagination,
    renderSourceChips,
    applySearchLocalFilter,
    hydrateLoadedTracksPreviewsInBackground,
  });
}
async function resetAndLoadLibraryTracks(
  query = "",
  limit = LIBRARY_LOAD_LIMIT_DEFAULT,
  options = {},
) {
  return library.resetAndLoadLibraryTracks(state, query, limit, {
    renderLibraryRows,
    loadTracks,
    ensureLibraryContainerFilled,
  }, options);
}
async function loadMoreLibraryTracks(limit = LIBRARY_LOAD_LIMIT_DEFAULT) {
  return library.loadMoreLibraryTracks(state, limit, { loadTracks });
}
async function ensureLibraryContainerFilled(
  limit = LIBRARY_LOAD_LIMIT_DEFAULT,
) {
  return library.ensureLibraryContainerFilled(state, el, limit, {
    loadMoreLibraryTracks,
    LIBRARY_AUTOFILL_MAX_PAGES,
  });
}
function handleLibraryTableWrapScroll() {
  library.handleLibraryTableWrapScroll(state, el, {
    LIBRARY_SCROLL_FETCH_THRESHOLD_PX,
    LIBRARY_LOAD_LIMIT_DEFAULT,
    loadMoreLibraryTracks,
    setStatus,
    emitStatus,
  });
}
function handleWindowLibraryScroll() {
  library.handleWindowLibraryScroll(state, el, window, {
    LIBRARY_SCROLL_FETCH_THRESHOLD_PX,
    LIBRARY_LOAD_LIMIT_DEFAULT,
    loadMoreLibraryTracks,
    setStatus,
    emitStatus,
  });
}
async function scanLibrary() {
  return library.scanLibrary(state, {
    setStatus,
    emitStatus,
    command,
    persistSourceRoots,
    resetAndLoadLibraryTracks,
    LIBRARY_LOAD_LIMIT_POST_SCAN,
    trackPathIsInsideSelectedRoots: (fp) =>
      library.trackPathIsInsideSelectedRoots(
        fp,
        library.enabledSourceRoots(state.sourceRoots, state.sourceRootEnabled, state.missingSourceRoots)
      ),
    trackHasCoreAnalysis,
    analyzeTrackIds,
    refreshCurrentPlaylistTracks,
    countWarningsForStatus: eventLog.countWarningsForStatus,
    renderSourceChips,
  });
}
async function relocateSourceRoot(oldRoot) {
  return library.relocateSourceRoot(state, oldRoot, {
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
}
async function scanMasterDb() {
  return library.scanMasterDb(state, {
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
}
async function analyzeTrackIds(trackIds, modeLabel = "Analyze", options = {}) {
  return library.analyzeTrackIds(state, trackIds, modeLabel, options, {
    shouldUseBatchAnalysis: library.shouldUseBatchAnalysis,
    parseAnalysisBpmRange: library.parseAnalysisBpmRange,
    command,
    setStatus,
    emitStatus,
    resolveMissingAnalysisPieces,
    setTrackAnalyzingState,
    applyRealtimeAnalyzedTrackUpdate,
    nextPaint: jobMgr.nextPaint,
    mergeHydratedTrackIntoState,
    hydrateTrackPreviewFromBackend,
    patchLibraryRowByTrackId,
    patchPlaylistRowByTrackId,
    updateLibraryDurationSummary: () =>
      updateLibraryDurationSummary(getLibraryVisibleTracks()),
    renderSourceChips,
    refreshCurrentPlaylistTracks,
    countWarningsForStatus: eventLog.countWarningsForStatus,
    logWarnings,
  });
}
async function analyzeSingleTrack(track, modeLabel = null) {
  return library.analyzeSingleTrack(state, track, modeLabel, {
    resolveLocalTrackId,
    resolveLocalTrackIdAsync,
    setStatus,
    emitStatus,
    trackHasCoreAnalysis,
    analyzeTrackIds,
  });
}

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
function loadUsbRecentRootsFromStorage() {
  settings.loadUsbRecentRootsFromStorage(state);
}
function persistMasterDbEnabled(enabled) {
  settings.persistMasterDbEnabled(command, enabled);
}
function persistSourcesEverConfigured(value) {
  settings.persistSourcesEverConfigured(command, value);
}
function rememberUsbRecentRoot(path) {
  settings.rememberUsbRecentRoot(state, command, path, renderUsbRecentRoots);
}

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
async function loadPlaylists() {
  return playlist.loadPlaylists(state, {
    command,
    renderPlaylistTabsAndPanels,
    updatePlaylistExportButtons,
  });
}
async function refreshCurrentPlaylistTracks() {
  playlist.refreshCurrentPlaylistTracks(state, el, {
    getCurrentPlaylist,
    command,
    normalizeTrack,
    filterTracksByQuery,
    renderEmptyState,
    applySortToTracks,
    renderTrackTable,
    updateTrackListDurationSummary,
    updatePlaylistPanelTitle,
    updatePlaylistExportButtons,
    renderPlaylistList,
  });
}
async function createPlaylist(name) {
  return playlist.createPlaylist(name, {
    setStatus,
    emitStatus,
    withProgress,
    command,
    loadPlaylists,
    state,
    updateModeText,
    switchTab,
  });
}
async function deletePlaylist(playlistId) {
  return playlist.deletePlaylist(playlistId, {
    state,
    openConfirmDialog: (opts) => confirmDialog.open(opts),
    command,
    loadPlaylists,
    updateModeText,
    switchTab,
    setStatus,
    emitStatus,
  });
}
async function addTracksToCurrentPlaylist(tracks) {
  return playlist.addTracksToCurrentPlaylist(tracks, {
    requireCurrentPlaylist,
    resolveLocalTrackId,
    resolveLocalTrackIdAsync,
    shouldAllowResolvedFallback,
    pushEventLog,
    setStatus,
    emitStatus,
    withProgress,
    command,
    refreshCurrentPlaylistTracks,
  });
}

// --- USB closures ---

function renderUsbPlaylists() {
  usb.renderUsbPlaylists(state, el, { escapeHtml });
}
function renderUsbPlaylistTracks() {
  usb.renderUsbPlaylistTracks(state, el, {
    filterTracksByQuery,
    applySortToTracks,
    renderTrackTable,
    updateTrackListDurationSummary,
  });
}
function renderHistoryList() {
  usb.renderHistoryList(state, el, { escapeHtml, getHistoryDateValue });
}
function renderHistoryTracks() {
  usb.renderHistoryTracks(state, el, {
    filterTracksByQuery,
    applySortToTracks,
    renderTrackTable,
    updateTrackListDurationSummary,
  });
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
function rebuildKnownUsbPlaylistNames() {
  usb.rebuildKnownUsbPlaylistNames(state);
}
function resetUsbStateViews() {
  usb.resetUsbStateViews(state, el, {
    renderUsbPlaylists,
    renderUsbPlaylistTracks,
    renderHistoryList,
    renderHistoryTracks,
    renderUsbPlayerMenuEditor,
  });
}
function showDiagReportView() {
  usb.showDiagReportView(el);
}
function showDiagRepairView() {
  usb.showDiagRepairView(el);
}
function hydrateUsbTrackMetadata(track) {
  return usb.hydrateUsbTrackMetadata(state, track, {
    usbTrackNeedsHydration,
    command,
    normalizeTrack,
  });
}

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
    warn: (...a) => console.warn(...a),
    scheduler: (fn, ms) => window.setTimeout(fn, ms),
  });
  if (state.usbRoot) rememberUsbRecentRoot(state.usbRoot);
  await syncAssetScopePaths();
  return result;
}
async function initializeUsb() {
  return usb.initializeUsb(state, el, {
    command,
    setStatus,
    emitStatus,
    validateAndSetUsbRoot,
    logError: (...a) => console.error(...a),
  });
}
async function pickUsbFolder() {
  return usb.pickUsbFolder({ invoke, validateAndSetUsbRoot });
}
async function syncAssetScopePaths() {
  return usb.syncAssetScopePaths(state, {
    invoke,
    warn: (...a) => console.warn(...a),
  });
}
async function detectExternalMasterDb() {
  return usb.detectExternalMasterDb(state, el, {
    command,
    warn: (...a) => console.warn(...a),
    renderSourceChips,
  });
}
async function pickSourceFolders() {
  return usb.pickSourceFolders({ invoke });
}

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

async function refreshUsb() {
  return usb.refreshUsb(state, el, {
    setStatus,
    emitStatus,
    command,
    setProgress,
    startProgressHeartbeat,
    stopProgressHeartbeat,
    normalizeUsbPlaylist,
    rebuildKnownUsbPlaylistNames,
    renderUsbPlaylists,
    renderUsbPlaylistTracks,
    updatePlaylistExportButtons,
    countWarningsForStatus: eventLog.countWarningsForStatus,
    logWarnings,
  });
}
async function removeUsbPlaylist(p) {
  return usb.removeUsbPlaylist(state, p, {
    setStatus,
    emitStatus,
    openConfirmDialog: (opts) => confirmDialog.open(opts),
    command,
    refreshUsb,
    countWarningsForStatus: eventLog.countWarningsForStatus,
  });
}
async function runUsbDiagnostics() {
  return usb.runUsbDiagnostics(state, {
    setStatus,
    emitStatus,
    command,
    setProgress,
    startProgressHeartbeat,
    stopProgressHeartbeat,
    normalizePlaylistNameForCompare: usb.normalizePlaylistNameForCompare,
    updatePlaylistExportButtons,
    renderDiagnosticsReport,
    logWarnings,
  });
}
async function runUsbParityReport() {
  return usb.runUsbParityReport(state, {
    setStatus,
    emitStatus,
    command,
    setProgress,
    startProgressHeartbeat,
    stopProgressHeartbeat,
    renderParityReport,
    logWarnings,
  });
}
async function previewUsbRepairs() {
  return usb.previewUsbRepairs(state, {
    setStatus,
    emitStatus,
    command,
    setProgress,
    startProgressHeartbeat,
    stopProgressHeartbeat,
    renderRepairPreview,
    logWarnings,
  });
}
async function applyUsbRepairs() {
  return usb.applyUsbRepairs(state, {
    setStatus,
    emitStatus,
    command,
    setProgress,
    startProgressHeartbeat,
    stopProgressHeartbeat,
    logWarnings,
    runUsbDiagnostics,
  });
}
async function refreshHistory() {
  return usb.refreshHistory(state, el, {
    setStatus,
    emitStatus,
    command,
    normalizeTrack,
    countWarningsForStatus: eventLog.countWarningsForStatus,
    logWarnings,
    renderHistoryList,
    renderHistoryTracks,
  });
}
async function loadUsbPlayerMenuConfig() {
  return usb.loadUsbPlayerMenuConfig(state, el, {
    setStatus,
    emitStatus,
    command,
    documentObj: document,
  });
}
async function addUsbPlayerMenuItems() {
  return usb.addUsbPlayerMenuItems(state, el, {
    setStatus,
    emitStatus,
    command,
    documentObj: document,
  });
}
async function removeUsbPlayerMenuItems() {
  return usb.removeUsbPlayerMenuItems(state, el, {
    setStatus,
    emitStatus,
    command,
    documentObj: document,
  });
}
async function moveUsbPlayerMenuItems(direction) {
  return usb.moveUsbPlayerMenuItems(state, el, {
    setStatus,
    emitStatus,
    command,
    documentObj: document,
  }, direction);
}
async function syncUsbPlayerMenusEdbToPdb() {
  return usb.syncUsbPlayerMenusEdbToPdb(state, el, {
    setStatus,
    emitStatus,
    command,
    documentObj: document,
  });
}
async function exportPlaylistToUsb(playlistId) {
  return usb.exportPlaylistToUsb(state, el, playlistId, {
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
    renderUsbPlaylistTracks,
    refreshMissingSourceRoots: checkSourceRoots,
  });
}

// --- Bootstrap closures ---

function handleJobEvent(payload) {
  jobMgr.handleJobEvent(state, el, payload, {
    debugFrontendLog,
    pushEventLog,
    applyRealtimeAnalyzedTrackUpdate,
    setStatus,
    emitMessage,
  });
}
function handleBackendLogEvent(payload) {
  bootstrap.handleBackendLogEvent(payload, { pushEventLog });
}

async function registerBackendJobEvents() {
  return bootstrap.registerBackendJobEvents(state, {
    isTauriRuntime,
    unregisterBackendJobEvents,
    getTauriEventListen,
    handleJobEvent,
    handlePlaybackEvent,
    handleBackendLogEvent,
  });
}
async function unregisterBackendJobEvents() {
  return bootstrap.unregisterBackendJobEvents(state, {
    warn: (...a) => console.warn(...a),
  });
}

async function switchView(viewId) {
  const switched = await bootstrap.switchView(state, el, viewId, {
    staticTabs: STATIC_TABS,
    stopPlaybackIfActive,
    syncLibraryOnboardingMode,
    updateModeText,
    populatePlaylistPanel,
    refreshCurrentPlaylistTracks,
    renderEventLog,
    requestAnimationFrameFn: window.requestAnimationFrame.bind(window),
    documentObj: document,
    renderWaveformsIn,
  });
  if (viewId === "usb-player-menu") {
    renderUsbPlayerMenuEditor();
    await loadUsbPlayerMenuConfig();
  }
  return switched;
}
async function switchTab(tab) {
  return bootstrap.switchTab(state, el, tab, {
    staticTabs: STATIC_TABS,
    stopPlaybackIfActive,
    syncLibraryOnboardingMode,
    updateModeText,
    populatePlaylistPanel,
    refreshCurrentPlaylistTracks,
    renderEventLog,
    requestAnimationFrameFn: window.requestAnimationFrame.bind(window),
    documentObj: document,
    renderWaveformsIn,
  });
}
function hydrateAppVersionLabel() {
  return bootstrap.hydrateAppVersionLabel(el, {
    appVersionFallback: APP_VERSION_FALLBACK,
    tauriIsTauri,
    tauriGetVersion,
  });
}
function checkForUpdate() {
  return bootstrap.checkForUpdate(state, el, {
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
}
function restoreStoredUiPrefs() {
  bootstrap.restoreStoredUiPrefs(state, el, {
    localStorageObj: localStorage,
    constants: {
      STORAGE_KEY_EXPORT_PRUNE_STALE,
      STORAGE_KEY_EXPORT_BACKUP,
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
function runDeferredInitialLoad() {
  return bootstrap.runDeferredInitialLoad(state, {
    setTimeoutFn: (cb) => setTimeout(cb, 0),
    withProgress,
    loadPlaylists,
    resetAndLoadLibraryTracks,
    libraryLoadLimitInit: LIBRARY_LOAD_LIMIT_INIT,
    updateModeText,
    updateSelectionCount,
    renderUsbPlaylistTracks,
    renderWaveformsIn,
    documentObj: document,
    setStatus,
    logError: (e) => console.error(e),
  });
}

// --- Sidebar expand button ---

const sidebarExpandBtn = document.createElement("button");
sidebarExpandBtn.className = "sidebar-expand-btn";
sidebarExpandBtn.textContent = "\u25B8";
sidebarExpandBtn.title = "Expand sidebar";
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
    constants: {
      STORAGE_KEY_SIDEBAR_COLLAPSED,
      FRONTEND_DB_KEY_SIDEBAR_COLLAPSED,
      STORAGE_KEY_HELP_SEEN,
      FRONTEND_DB_KEY_HELP_SEEN,
      STORAGE_KEY_EXPORT_PRUNE_STALE,
      FRONTEND_DB_KEY_EXPORT_PRUNE_STALE,
      STORAGE_KEY_EXPORT_BACKUP,
      FRONTEND_DB_KEY_EXPORT_BACKUP,
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
    LIBRARY_LOAD_LIMIT_DEFAULT,
    dismissProgress,
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
    loadUsbPlayerMenuConfig,
    renderUsbPlayerMenuEditor,
    syncUsbPlayerMenuEditorControls,
    handleUsbPlayerMenuListClick,
    addUsbPlayerMenuItems,
    removeUsbPlayerMenuItems,
    moveUsbPlayerMenuItems,
    syncUsbPlayerMenusEdbToPdb,
    scheduleApplySearchLocalFilter,
    renderUsbPlaylistTracks,
    renderHistoryTracks,
    patchUsbTrackRow,
    patchHistoryTrackRow,
    addTracksToCurrentPlaylist,
    getLibraryVisibleTracks,
    analyzeSingleTrack,
    getPlaybackUiStateHelpers: playback.getPlaybackUiStateHelpers,
    isTrackCurrentlyPlaying,
    stopPlaybackFromUi,
    playTrackFromOrigin,
    scrubRatioFromPointer: playback.scrubRatioFromPointer,
    removeUsbPlaylist,
    stopPlaybackIfActive,
    hydrateUsbTrackMetadata,
    setActiveListItem: shell.setActiveListItem,
    getHistoryDateDisplay,
    getCurrentPlaylist,
    loadPlaylists,
    updateModeText,
    exportPlaylistToUsb,
    isUsbOriginTrack,
    trackHasCoreAnalysis,
    analyzeTrackIds,
    resolveLocalTrackId,
    handleSortHeaderClick,
    handleLibraryTableWrapScroll,
    handleWindowLibraryScroll,
    renderLibraryRows,
    hydrateLoadedTracksPreviewsInBackground,
  });
  return uiCtrl.bindEvents(bindCtx);
}

async function init() {
  return bootstrap.initApp(state, {
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
    loadUsbRecentRootsFromStorage,
    renderUsbRecentRoots,
    persistSourceRootEnabled,
    syncAssetScopePaths,
    loadUsbRootFromStorage,
    restoreStoredUiPrefs,
    applySidebarCollapsedUi,
    checkSourceRoots,
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
    logInfo: (...a) => console.info(...a),
    logError: (...a) => console.error(...a),
    warn: (...a) => console.warn(...a),
  });
}

window.addEventListener("resize", () => {
  renderWaveformsIn(document);
});
playback.bindBeforeUnloadCleanup(window, unregisterBackendJobEvents);

init().catch((error) => {
  state.startupPhase = false;
  console.error(error);
  setStatus(`Initialization failed: ${error.message}`);
});
