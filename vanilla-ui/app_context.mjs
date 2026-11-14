import * as trackTable from "./track_table.mjs";
import * as playback from "./components/playback/actions.mjs";
import * as playlist from "./components/playlist/actions.mjs";
import * as usb from "./components/usb/actions.mjs";
import * as usbAudioFormats from "./components/usb/audio_formats.mjs";
import * as exportPlaylists from "./components/usb/export_playlists.mjs";
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
import { createGroupEditDialogController } from "./components/shared/track_metadata_edits.mjs";
import { createAddToDialogController } from "./components/shared/add_to_playlists.mjs";
import { createConvertDialogController } from "./components/shared/transcodes.mjs";
import * as trackColumns from "./components/shared/track_columns.mjs";

// Every element with an `id` in index.html, by id (`el.statusText`, …). Ids
// inside <template>s aren't part of the document, so they're never collected.
function collectElements(document) {
  return {
    ...Object.fromEntries([...document.querySelectorAll("[id]")].map((node) => [node.id, node])),
    panels: {
      library: document.getElementById("panel-library"),
      usb: document.getElementById("panel-usb"),
      "usb-playlists": document.getElementById("panel-usb-playlists"),
      "usb-history": document.getElementById("panel-usb-history"),
      "usb-player-menu": document.getElementById("panel-usb-player-menu"),
      "export-playlists": document.getElementById("panel-export-playlists"),
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
  // The header rows go in first: they hold elements `el` collects.
  trackColumns.buildTrackGridHeads(document);
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
  bindActions(eventLog, `storeEventLogEntry renderEventLog renderEventLogBadge markEventLogLinkedDetails clearEventLogBadge logWarnings setupConsoleFileLogging openEventLogFile
    setupRuntimeErrorLogging logToFile`);
  bindActions(backupsUi, "renderBackups restoreUsbBackup deleteUsbBackup");
  bindActions(jobMgr, `setProgress dismissProgress startProgressHeartbeat stopProgressHeartbeat
    withProgress handleJobEvent toggleAnalysisPause cancelAnalysis cancelTranscode scheduleProgressIdle waitForUsbJobIdle`);
  bindActions(uiCtrl, `updateModeText updateActivePlaylistIndicators updateAddToPlaylistButtons
    updateUsbNameBadge updateSelectionCount updateUsbSubNavDisabledState updateUsbEmptyState
    updateAnalyzeLibraryButtonLabel closeSettingsDrawer updateUsbHealthDot
    syncLibraryOnboardingMode bindEvents`);
  bindActions(settings, "persistSetting hydrateLocalStorageFromFrontendSettingsDb");
  bindActions(bootstrap, "switchView debugFrontendLog handleBackendLogEvent");
  bindActions(shell, "handleSortHeaderClick");
  bindActions(playback, `toPlayableUrl updateTransportButtonsInDom clearAllWaveformPlayheads
    pausePlaybackFromUi resumePlaybackFromUi moveActiveWaveform stopPlaybackFromUi stopPlaybackIfActive
    resolveLocalTrackIdAsync isTrackCurrentlyPlaying playTrackFromOrigin handlePlaybackEvent
    registerBackendJobEvents unregisterBackendJobEvents`);
  bindActions(playlist, `renderPlaylistList promptNewPlaylist startPlaylistRename reorderPlaylists loadPlaylists applyPlaylistsState
    commitActivePlaylistSort updatePlaylistExportButtons createPlaylist deletePlaylist
    addTracksToCurrentPlaylist addLibrarySelectionToCurrentPlaylist populatePlaylistPanel
    getCurrentPlaylist isPlaylistSortActive clearPlaylistTrackSort renderCurrentPlaylistTracksFromState
    refreshCurrentPlaylistTracks importExternalPlaylist renderOpenPlaylistExportStatus
    refreshPlaylistExportSpace reloadPlaylistForUsb`);
  bindActions(library, `normalizeTrack applyLibrarySources
    refreshLibrarySources showExternalLibrary applyRealtimeAnalyzedTrackUpdate
    relocateSourceRoot renderSourceChips applyLibraryDurationSummary renderLibraryRows
    scheduleLibrarySearch resetAndLoadLibraryTracks
    reloadTrackListsForKeyNotation analyzeLibrary reanalyzeTracks
    scanRekordboxDb scanMixxxDb analyzeTrackIds analyzeSingleTrack setTrackAnalyzingState setTracksWaitingState
    showTrackRow`);
  bindActions(usb, `setUsbRootControlsLocked applyPlaylistUsbExportStatus showDiagReportView
    clearUsbDiagnostics hideUsbDiagnostics loadUsbRootFromStorage usbContentChanged
    pickSourceFolders detectExternalRekordboxDb detectExternalMixxxDb
    validateAndSetUsbRoot renameDrive
    removeUsbPlaylist refreshUsb ensureUsbListLoaded openUsbPlaylistRow openHistoryRow
    runUsbDiagnostics runUsbParityReport
    previewUsbRepairs applyUsbRepairs exportHistoryTracklist
    handleUsbPlayerMenuListClick renderUsbPlayerMenuEditor
    syncUsbPlayerMenuEditorControls loadUsbPlayerMenuConfig addUsbPlayerMenuItems
    loadUsbBackgroundColor renderUsbBackgroundColor setUsbBackgroundColor
    removeUsbPlayerMenuItems moveUsbPlayerMenuItems exportPlaylistToUsb
    initializeUsb pickUsbFolder hydrateUsbTrackMetadata loadUsbDevices
    pruneUsbDevice updateUsbRootText`);
  bindActions(usbAudioFormats, "applyUsbAudioFormats renderUsbAudioFormats");
  bindActions(exportPlaylists, `openExportPlaylistsPage renderExportPlaylistsControls selectExportPlaylists
    toggleExportPlaylist clickExportPlaylistRow runExportPlaylists refreshExportPlaylistsPage
    analyzeExportPlaylistsMissing`);
  bindActions(trackTable, "buildTrackTableRow renderEmptyTrackTable finishTrackRows");
  bindActions(trackDetail, "openTrackDetail");
  bindActions(trackColumns, "applyTrackColumns renderTrackColumnsButtons");

  ctx.confirmDialog = uiCtrl.createConfirmDialogController(ctx.el);
  ctx.openConfirmDialog = (opts) => ctx.confirmDialog.open(opts);
  ctx.tracklistExportDialog = uiCtrl.createTracklistExportDialogController(ctx.el);
  ctx.playlistImportDialog = uiCtrl.createPlaylistImportDialogController(ctx.el, ctx.document);
  ctx.groupEditDialog = createGroupEditDialogController(ctx.el, ctx.document);
  ctx.addToDialog = createAddToDialogController(ctx.el, ctx.document);
  ctx.convertDialog = createConvertDialogController(ctx.el, ctx.document);
  ctx.trackColumnsDialog = trackColumns.createTrackColumnsDialogController(ctx);
  ctx.trackDetailDialog = trackDetail.createAppTrackDetailController(ctx);
  ctx.libraryTracksCtl = library.createLibraryTracksController(ctx);
  ctx.playlistTracksCtl = playlist.createPlaylistTracksController(ctx);
  ctx.usbPlaylistTracksCtl = usb.createUsbPlaylistTracksController(ctx);
  ctx.usbHistoryTracksCtl = usb.createUsbHistoryTracksController(ctx);
  return ctx;
}
