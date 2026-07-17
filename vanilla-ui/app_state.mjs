// App state and static UI state defaults.

import { DEFAULT_ANALYSIS_BPM_RANGE } from "./components/library/actions.mjs";
import { createEventLogStore } from "./event_log.mjs";

export const STATIC_TABS = ["library", "usb", "usb-playlists", "usb-history", "usb-player-menu", "event-log"];
export const EVENT_LOG_MAX = 1000;

export function createInitialState() {
  return {
    sourceRoots: [],
    sourceRootEnabled: {},
    sourceRootAnalysisStatus: {},
    usbRoot: null,
    usbRecentRoots: [],
    usbRootValid: false,
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
    currentPlaylistTracksView: [],
    usbPlaylists: [],
    usbKnownPlaylistNames: new Set(),
    usbPlaylistTracks: [],
    usbPlaylistTracksView: [],
    usbTrackSearch: "",
    histories: [],
    historyTracks: [],
    historyTracksView: [],
    historyTrackSearch: "",
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
    progressPercent: 0,
    progressBaseText: "Idle",
    progressHeartbeatTimer: null,
    progressStartedAtMs: 0,
    lastJobEventAtMs: 0,
    librarySearchDebounceTimer: null,
    realtimeRenderTimer: null,
    realtimeRenderQueued: false,
    trackPreviewHydrateInFlight: new Set(),
    loadedPreviewHydrationSeq: 0,
    analyzingTrackIds: new Set(),
    analysisPatchQueue: new Set(),
    analysisPatchRafId: null,
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
