import * as trackTable from "./track_table.mjs";
import * as playback from "./components/playback/actions.mjs";
import * as playlist from "./components/playlist/actions.mjs";
import * as usb from "./components/usb/actions.mjs";
import * as eventLog from "./components/event-log/actions.mjs";
import * as backupsUi from "./components/backups/actions.mjs";
import * as library from "./components/library/actions.mjs";
import * as settings from "./components/settings/actions.mjs";
import * as shell from "./components/shell/actions.mjs";
import * as trackDetail from "./components/track-detail/actions.mjs";
import * as jobMgr from "./job_manager.mjs";
import * as bootstrap from "./startup_bootstrap.mjs";
import * as uiCtrl from "./ui_controller.mjs";
import * as messages from "./message_bus.mjs";
import { createInitialState, createTableSortState, createEventLogState } from "./app_state.mjs";
import { renderWaveformsIn } from "./waveform.mjs";

const ELEMENT_IDS = [
  "statusText", "playlistBadge", "badgeLabel", "usbNameBadge", "usbNameBadgeLabel", "usbHeaderHealthDot", "navSidebar",
  "sidebarCollapseBtn", "sidebarExpandBtn", "donateBtn", "navPlaylistList", "addPlaylistBtn",
  "playlistPanelTitle", "playlistSearchInput", "playlistTracksBody", "playlistTableWrap",
  "playlistEmptyState", "playlistTotalDuration", "playlistExportStatus", "analyzePlaylistMissingBtn",
  "exportPlaylistBtn", "settingsBtn", "settingsDrawer", "settingsBackdrop",
  "settingsCloseBtn", "settingsVersionText", "settingsUpdateNote", "updateBanner",
  "updateBannerText", "updateBannerDismissBtn", "openEventLogBtn", "accentHueSlider",
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
  "analysisEngineSelect", "keyNotationSelect", "analysisEngineStatus", "essentiaInstallRow", "essentiaNodeStatus",
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
  "trackDetailOverlay", "trackDetailTitle", "trackDetailCloseBtn", "trackDetailWaveform",
  "trackDetailBeatgrid", "trackDetailPreStart", "trackDetailCueMarkers", "trackDetailPlayhead", "trackDetailFirstBeatMs",
  "trackDetailFirstBeatMinus", "trackDetailFirstBeatPlus", "trackDetailStartFirstCue", "trackDetailStartFirstBeat", "trackDetailStartNote", "trackDetailAddCue",
  "trackDetailBpm", "trackDetailBpmMinus", "trackDetailBpmPlus", "trackDetailBpmHalf", "trackDetailBpmDouble",
  "trackDetailQuantize", "trackDetailMetronome", "trackDetailUndo", "trackDetailRedo",
  "trackDetailKey", "trackDetailKeyMinus", "trackDetailKeyPlus",
  "trackDetailPlayPause", "trackDetailHint", "trackDetailHintBtn",
  "trackDetailOverview", "trackDetailOverviewCues", "trackDetailOverviewWindow", "trackDetailZoomOut", "trackDetailZoomIn", "trackDetailZoomFit", "trackDetailGridLevel", "trackDetailMetronomeMix", "trackDetailZoomRange", "trackDetailTotalTime",
  "trackDetailCueList", "trackDetailStartCue", "trackDetailCancelBtn", "trackDetailSaveBtn", "trackDetailColorPopover",
];

function collectElements(document) {
  return {
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
}

// Everything the UI modules share: app state, DOM refs, platform services
// (`env`: document, window, localStorage, the API client, Tauri helpers) and
// every module action, bound so each is callable as `ctx.action(...args)`.
export function createAppContext(env) {
  const { document, window } = env;
  const ctx = {
    state: createInitialState(),
    el: collectElements(document),
    tableSortState: createTableSortState(),
    eventLogStore: createEventLogState(),
    requestAnimationFrameFn: (cb) => window.requestAnimationFrame(cb),
    cancelAnimationFrameFn: (handle) => window.cancelAnimationFrame(handle),
    // Late-bound: setupConsoleFileLogging replaces the console methods at startup.
    logInfo: (...a) => console.info(...a),
    warn: (...a) => console.warn(...a),
    logError: (...a) => console.error(...a),
    nextPaint: jobMgr.nextPaint,
    countWarningsForStatus: eventLog.countWarningsForStatus,
    warningEntryLevel: eventLog.warningEntryLevel,
    renderWaveformsIn,
    ...env,
  };

  function bindActions(mod, names) {
    for (const name of names.trim().split(/\s+/)) {
      if (typeof mod[name] !== "function") throw new Error(`bindActions: ${name} is not exported`);
      if (name in ctx) throw new Error(`bindActions: ${name} bound twice`);
      ctx[name] = (...args) => mod[name](ctx, ...args);
    }
  }

  bindActions(messages, "emitMessage emitStatus pushEventLog");
  bindActions(eventLog, `storeEventLogEntry renderEventLog logWarnings setupConsoleFileLogging
    setupRuntimeErrorLogging`);
  bindActions(backupsUi, "renderBackups restoreUsbBackup deleteUsbBackup");
  bindActions(jobMgr, `setProgress dismissProgress startProgressHeartbeat stopProgressHeartbeat
    withProgress handleJobEvent toggleAnalysisPause cancelAnalysis scheduleProgressIdle waitForUsbJobIdle`);
  bindActions(uiCtrl, `updateModeText updateActivePlaylistIndicators updateAddToPlaylistButtons
    updateUsbNameBadge updateSelectionCount updateUsbSubNavDisabledState updateUsbEmptyState
    updateSourceFilterIndicator updateScanLibraryButtonLabel closeSettingsDrawer updateUsbHealthDot
    syncLibraryOnboardingMode bindEvents`);
  bindActions(settings, `persistSetting hydrateLocalStorageFromFrontendSettingsDb persistSourceRoots
    persistUsbRoot loadSourceRootsFromStorage loadSourceRootEnabledFromStorage persistSourceRootEnabled
    persistMasterDbEnabled loadMasterDbEnabledFromStorage persistSourcesEverConfigured
    loadSourcesEverConfiguredFromStorage`);
  bindActions(bootstrap, "switchView debugFrontendLog handleBackendLogEvent");
  bindActions(shell, "handleSortHeaderClick");
  bindActions(playback, `toPlayableUrl updateTransportButtonsInDom clearAllWaveformPlayheads
    pausePlaybackFromUi resumePlaybackFromUi moveActiveWaveform stopPlaybackFromUi stopPlaybackIfActive
    resolveLocalTrackIdAsync isTrackCurrentlyPlaying playTrackFromOrigin handlePlaybackEvent
    registerBackendJobEvents unregisterBackendJobEvents`);
  bindActions(playlist, `renderPlaylistList promptNewPlaylist startPlaylistRename loadPlaylists
    commitActivePlaylistSort updatePlaylistExportButtons createPlaylist deletePlaylist
    addTracksToCurrentPlaylist addLibrarySelectionToCurrentPlaylist populatePlaylistPanel
    getCurrentPlaylist isPlaylistSortActive renderCurrentPlaylistTracksFromState
    refreshCurrentPlaylistTracks`);
  bindActions(library, `normalizeTrack normalizeUsbPlaylist refreshSourceRootAnalysisStatus
    refreshMissingSourceRoots applyRealtimeAnalyzedTrackUpdate hydrateLoadedTracksPreviewsInBackground
    relocateSourceRoot renderSourceChips applyLibraryDurationSummary renderLibraryRows
    scheduleLibrarySearch resetAndLoadLibraryTracks handleLibraryTableWrapScroll
    reloadTrackListsForKeyNotation enabledLibrarySourceRoots scanLibrary analyzeSelectedTracks
    scanMasterDb analyzeTrackIds analyzeSingleTrack setTrackAnalyzingState promoteTrackIdentity
    patchTrackAnalysisFields`);
  bindActions(usb, `setUsbRootControlsLocked refreshPlaylistExportStatus showDiagReportView
    clearUsbDiagnostics hideUsbDiagnostics loadUsbRootFromStorage resetUsbStateViews
    syncAssetScopePaths pickSourceFolders detectExternalMasterDb validateAndSetUsbRoot
    removeUsbPlaylist reorderUsbPlaylists refreshUsb runUsbDiagnostics runUsbParityReport
    previewUsbRepairs applyUsbRepairs refreshHistory exportHistoryTracklist
    handleUsbPlayerMenuListClick renderUsbPlayerMenuEditor syncUsbPlayerMenusEdbToPdb
    syncUsbPlayerMenuEditorControls loadUsbPlayerMenuConfig addUsbPlayerMenuItems
    removeUsbPlayerMenuItems moveUsbPlayerMenuItems exportPlaylistToUsb patchUsbTrackRow
    patchHistoryTrackRow initializeUsb pickUsbFolder hydrateUsbTrackMetadata loadUsbDevices
    pruneUsbDevice updateUsbRootText`);
  bindActions(trackTable, "renderTrackTable");
  bindActions(trackDetail, "openTrackDetail");

  ctx.confirmDialog = uiCtrl.createConfirmDialogController(ctx.el);
  ctx.openConfirmDialog = (opts) => ctx.confirmDialog.open(opts);
  ctx.tracklistExportDialog = uiCtrl.createTracklistExportDialogController(ctx.el);
  ctx.trackDetailDialog = trackDetail.createAppTrackDetailController(ctx);
  ctx.libraryTracksCtl = library.createLibraryTracksController(ctx);
  ctx.playlistTracksCtl = playlist.createPlaylistTracksController(ctx);
  ctx.usbPlaylistTracksCtl = usb.createUsbPlaylistTracksController(ctx);
  ctx.usbHistoryTracksCtl = usb.createUsbHistoryTracksController(ctx);
  return ctx;
}
