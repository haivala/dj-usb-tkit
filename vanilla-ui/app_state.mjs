// App state and static UI state defaults.

import { DEFAULT_ANALYSIS_BPM_RANGE } from "./components/library/actions.mjs";
import { createEventLogStore } from "./event_log.mjs";

export const STATIC_TABS = ["library", "usb", "usb-playlists", "usb-history", "usb-player-menu", "event-log", "backups"];
export const EVENT_LOG_MAX = 500;

export function createInitialState() {
  const state = {
    // The backend's `get_library_sources`: the source folders
    // ({ path, enabled, missing, fullyAnalyzed }) and the imported libraries'
    // switches.
    librarySources: { folders: [], rekordboxDbEnabled: false, mixxxDbEnabled: false, everConfigured: false },
    usbRoot: null,
    usbRecentRoots: [],
    usbRootValid: false,
    usbDeviceName: null,
    // The name a connected drive carries that belongs to another drive.
    usbNameTakenBy: null,
    usbWritable: true,
    // An export was clicked and hasn't finished (see exportPlaylistToUsb).
    exportStarting: false,
    exportPruneStale: true,
    exportBackup: true,
    // Rebuild the Engine DJ library whenever the app changes a USB. Off until
    // turned on in Settings or by the diagnostics fix.
    exportEngineLibrary: false,
    // Cue editor: start playback on the first beat (memory-only start cue)
    // rather than on the first cue point. Applied when a track gets its first cue.
    cueStartOnFirstBeat: false,
    // Cue editor: how strongly the beat grid shows over the waveform (0-100).
    cueBeatgridLevel: 35,
    // Cue editor: snap new and dragged cues to the beat grid (Shift inverts).
    cueQuantize: true,
    cueFollowGrid: false,
    // Cue editor: metronome music/click balance (0 music only, 50 both full, 100 clicks only).
    cueMetronomeMix: 50,
    analysisBpmRange: DEFAULT_ANALYSIS_BPM_RANGE,
    analysisEngine: "stratum",
    analysisEnginePersistPromise: null,
    nodeAvailable: false,
    essentiaInstalled: false,
    essentiaDownloading: false,
    updateCheck: null,
    updateChecking: false,
    // The last check was the user's (re-check button): show "Up to date".
    updateCheckManual: false,
    updateInstalling: false,
    updateInstallFailed: false,
    // Pending progress-footer hide (job_manager.mjs scheduleProgressIdle).
    progressIdleTimer: null,
    // Resolvers waiting for the running USB job to end (waitForUsbJobIdle).
    usbJobIdleWaiters: [],
    // The loaded library page(s); owned by the library TrackListController
    // (main.js) via getItems/setItems, but read directly by playback
    // resolution, analysis patching, and selection.
    tracks: [],
    libraryQuery: "",
    // Each track list's selection (components/shared/track_selection.mjs):
    // the ticked track ids, plus -- for the views whose bar actions need the
    // row itself (USB playlist / history) -- the rows by id. The library's
    // ids are also `selectedTrackIds`.
    trackSelections: {
      library: { ids: new Set(), rows: new Map() },
      playlist: { ids: new Set(), rows: new Map() },
      usbPlaylist: { ids: new Set(), rows: new Map() },
      usbHistory: { ids: new Set(), rows: new Map() },
    },
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
    usbPlayerMenuDivergence: null,
    activeTab: "library",
    // Errors logged since the Event Log was last open (header badge).
    eventLogUnseenErrors: 0,
    // A USB report pointed at Event Log details not yet looked at (header dot).
    eventLogHasLinkedDetails: false,
    sidebarCollapsed: false,
    sidebarWidth: 220,
    externalRekordboxDbPath: null,
    rekordboxDbImported: false,
    externalMixxxDbPath: null,
    mixxxDbImported: false,
    // The job whose progress the footer bar shows: the oldest running one.
    activeJobId: null,
    activeJobType: null,
    // Every running backend job, in start order: jobId -> { jobType,
    // startedAtMs, percent, message }.
    runningJobs: new Map(),
    analysisPaused: false,
    unlistenJobEvent: null,
    unlistenPlaybackEvent: null,
    unlistenBackendLogEvent: null,
    activeWaveform: null,
    playbackRowKey: null,
    // The row the cue editor was last opened from: marked in the lists so
    // the track is easy to find again after the editor closes.
    lastEditedRowKey: null,
    playbackTrackId: null,
    playbackPath: null,
    playbackActive: false,
    playbackPaused: false,
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
    librarySearchDebounceTimer: null,
    loadedPreviewHydrationSeq: 0,
    analyzingTrackIds: new Set(),
    deletingPlaylistId: null,
    selectedRepairFixIds: new Set(),
    startupPhase: true
  };
  Object.defineProperty(state, "selectedTrackIds", {
    get: () => state.trackSelections.library.ids,
    set: (ids) => { state.trackSelections.library.ids = ids; },
    enumerable: true,
  });
  return state;
}

export function createTableSortState() {
  return {};
}

export function createEventLogState() {
  return createEventLogStore({ maxEntries: EVENT_LOG_MAX });
}
