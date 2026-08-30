// App state and static UI state defaults.

import { DEFAULT_ANALYSIS_BPM_RANGE } from "./components/library/actions.mjs";
import { createEventLogStore } from "./event_log.mjs";

export const STATIC_TABS = ["library", "usb", "usb-playlists", "usb-history", "usb-player-menu", "event-log", "backups"];
export const EVENT_LOG_MAX = 1000;

export function createInitialState() {
  return {
    sourceRoots: [],
    sourceRootEnabled: {},
    missingSourceRoots: new Set(),
    sourceRootAnalysisStatus: {},
    usbRoot: null,
    usbRecentRoots: [],
    usbRootValid: false,
    usbDeviceName: null,
    usbNeedsInit: false,
    usbWritable: true,
    exportPruneStale: true,
    exportBackup: true,
    analysisBpmRange: DEFAULT_ANALYSIS_BPM_RANGE,
    analysisEngine: "stratum",
    analysisEnginePersistPromise: null,
    nodeAvailable: false,
    essentiaInstalled: false,
    essentiaDownloading: false,
    updateCheck: null,
    tracks: [],
    filteredTracks: [],
    libraryQuery: "",
    libraryLoadedTotal: 0,
    libraryNextCursor: null,
    libraryHasMore: false,
    libraryLoading: false,
    libraryRequestSeq: 0,
    selectedTrackIds: new Set(),
    playlists: [],
    currentPlaylistId: null,
    playlistTrackSearch: "",
    usbPlaylists: [],
    // Keyed by local playlist id -- { sameNameExistsOnUsb, locksReorder },
    // computed server-side (see backend's PlaylistUsbExportStatus) whenever a
    // command scans the connected USB's playlist names, so the frontend never
    // re-derives the "does this collide with an existing USB playlist, and
    // would an additive export leave its order untouched" rule itself.
    playlistUsbExportStatusById: new Map(),
    // The USB playlist/history track tables' fetch/pagination/search/sort state
    // lives in their TrackListControllers (main.js), not here.
    histories: [],
    selectedHistoryIndex: null,
    // Whole selected session, kept only for the "Export Tracklist" text feature.
    historyTracks: [],
    usbPlayerMenuCurrent: [],
    usbPlayerMenuAvailable: [],
    usbPlayerMenuCurrentSelectedKind: null,
    usbPlayerMenuAvailableSelectedKind: null,
    usbPlayerMenuDivergence: { inEdbVisibleOnly: [], inPdbOnly: [], orderMismatch: false },
    activeTab: "library",
    sidebarCollapsed: false,
    externalMasterDbPath: null,
    masterDbEnabled: false,
    sourcesEverConfigured: false,
    activeJobId: null,
    activeJobType: null,
    analysisPaused: false,
    unlistenJobEvent: null,
    unlistenPlaybackEvent: null,
    unlistenBackendLogEvent: null,
    activeWaveform: null,
    playbackRowKey: null,
    playbackTrackId: null,
    playbackPath: null,
    playbackActive: false,
    playbackStartPromise: null,
    playbackStopPromise: null,
    playbackGeneration: 0,
    playbackPendingKind: null,
    playbackPendingRowKey: null,
    playbackPendingTrackId: null,
    playbackBackendQueue: null,
    playheadAnimationHandle: null,
    progressPercent: 0,
    progressBaseText: "Idle",
    progressHeartbeatTimer: null,
    progressStartedAtMs: 0,
    progressPausedAtMs: null,
    lastJobEventAtMs: 0,
    librarySearchDebounceTimer: null,
    trackPreviewHydrateInFlight: new Set(),
    loadedPreviewHydrationSeq: 0,
    analyzingTrackIds: new Set(),
    deletingPlaylistId: null,
    selectedRepairFixIds: new Set(),
    eventLogEntries: [],
    startupPhase: true,
    mockPlayback: {
      path: null,
      playing: false,
      startedAtMs: 0,
      startOffsetMs: 0,
      durationMs: 240000
    }
  };
}

export function createTableSortState() {
  return {};
}

export function createEventLogState() {
  return createEventLogStore({ maxEntries: EVENT_LOG_MAX });
}
