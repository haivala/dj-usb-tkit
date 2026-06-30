import test from "node:test";
import assert from "node:assert/strict";

import {
  handleLibraryTableWrapScroll,
  handleWindowLibraryScroll,
  analyzeSingleTrack,
  analyzeTrackIds,
  applyRealtimeAnalyzedTrackUpdate
} from "../components/library/actions.mjs";

test("handleLibraryTableWrapScroll requests more rows near the bottom", async () => {
  const state = { libraryLoading: false, libraryHasMore: true };
  const el = {
    libraryTableWrap: {
      scrollHeight: 1000,
      scrollTop: 850,
      clientHeight: 100
    }
  };
  let loaded = 0;

  handleLibraryTableWrapScroll(state, el, {
    LIBRARY_SCROLL_FETCH_THRESHOLD_PX: 120,
    LIBRARY_LOAD_LIMIT_DEFAULT: 200,
    loadMoreLibraryTracks: async () => { loaded += 1; },
    setStatus: () => {}
  });

  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(loaded, 1);
});

test("handleWindowLibraryScroll ignores non-library tabs", async () => {
  const state = { activeTab: "usb", libraryLoading: false, libraryHasMore: true };
  const el = {
    libraryTableWrap: {
      getBoundingClientRect: () => ({ bottom: 500 })
    }
  };
  let loaded = 0;

  handleWindowLibraryScroll(state, el, { innerHeight: 400 }, {
    LIBRARY_SCROLL_FETCH_THRESHOLD_PX: 120,
    LIBRARY_LOAD_LIMIT_DEFAULT: 200,
    loadMoreLibraryTracks: async () => { loaded += 1; },
    setStatus: () => {}
  });

  assert.equal(loaded, 0);
});

test("analyzeSingleTrack reports when local id cannot be resolved", async () => {
  const state = { tracks: [] };
  let status = "";

  await analyzeSingleTrack(state, { id: "usb-1" }, null, {
    resolveLocalTrackId: () => null,
    resolveLocalTrackIdAsync: async () => null,
    setStatus: (text) => { status = text; },
    trackHasCoreAnalysis: () => false,
    analyzeTrackIds: async () => {}
  });

  assert.equal(status, "Track is not in local library yet. Scan library first, then analyze.");
});

test("analyzeTrackIds forwards bpmAnalyzer from analyze_track_piece updates", async () => {
  const state = {
    tracks: [{ id: "1", bpm: null, key: null, durationMs: null, waveformPreview: [] }],
    analysisBpmRange: "full",
    analyzingTrackIds: new Set()
  };
  const realtimePayloads = [];

  await analyzeTrackIds(state, ["1"], "Analyze", {}, {
    shouldUseBatchAnalysis: () => false,
    parseAnalysisBpmRange: () => ({ min: null, max: null }),
    command: async (name, args = {}) => {
      if (name === "get_system_parallelism") return { workers: 4 };
      if (name === "analyze_track_piece") {
        if (args.piece === "bpm_key") {
          return { bpm: 140, bpmAnalyzer: "stratum", key: "Am" };
        }
        if (args.piece === "duration") return { durationMs: 60000 };
        if (args.piece === "waveform") return { waveformPeaksPath: "/tmp/test.dat", waveformPreview: [10, 20] };
        if (args.piece === "artwork") return { artworkPath: "/tmp/test.jpg" };
        return {};
      }
      if (name === "get_tracks_by_ids_with_previews") {
        return { items: [{ id: "1", bpm: 140, bpmAnalyzer: "stratum", key: "Am", durationMs: 60000, waveformPreview: [10, 20] }] };
      }
      return {};
    },
    setStatus: () => {},
    resolveMissingAnalysisPieces: () => ["duration", "artwork", "waveform", "bpm_key"],
    setTrackAnalyzingState: () => {},
    applyRealtimeAnalyzedTrackUpdate: async (payload) => {
      realtimePayloads.push(payload);
    },
    nextPaint: async () => {},
    mergeHydratedTrackIntoState: () => false,
    hydrateTrackPreviewFromBackend: async () => {},
    patchLibraryRowByTrackId: () => {},
    patchPlaylistRowByTrackId: () => {},
    updateLibraryDurationSummary: () => {},
    renderSourceChips: () => {},
    refreshCurrentPlaylistTracks: async () => {},
    countWarningsForStatus: () => 0
  });

  const bpmPiece = realtimePayloads.find((payload) => Number(payload?.bpm) === 140);
  assert.ok(bpmPiece);
  assert.equal(bpmPiece.bpmAnalyzer, "stratum");
});

test("analyzeTrackIds marks track failed when bpm_key returns empty result", async () => {
  const state = {
    tracks: [{ id: "1", bpm: null, key: null, durationMs: null, waveformPreview: [] }],
    analysisBpmRange: "full",
    analysisEngine: "essentia",
    analyzingTrackIds: new Set()
  };
  const statuses = [];
  let hydrateCalls = 0;

  const result = await analyzeTrackIds(state, ["1"], "Analyze missing", {}, {
    shouldUseBatchAnalysis: () => false,
    parseAnalysisBpmRange: () => ({ min: 70, max: 180 }),
    command: async (name, args = {}) => {
      if (name === "get_system_parallelism") return { workers: 4 };
      if (name === "analyze_track_piece") {
        if (args.piece === "bpm_key") return { bpm: null, bpmAnalyzer: null, key: null };
        if (args.piece === "duration") return { durationMs: 60000 };
        if (args.piece === "waveform") return { waveformPeaksPath: "/tmp/test.dat", waveformPreview: [10, 20] };
        if (args.piece === "artwork") return { artworkPath: "/tmp/test.jpg" };
      }
      if (name === "get_tracks_by_ids_with_previews") {
        hydrateCalls += 1;
        return { items: [] };
      }
      return {};
    },
    setStatus: (text) => { statuses.push(String(text || "")); },
    resolveMissingAnalysisPieces: () => ["duration", "artwork", "waveform", "bpm_key"],
    setTrackAnalyzingState: () => {},
    applyRealtimeAnalyzedTrackUpdate: async () => {},
    nextPaint: async () => {},
    mergeHydratedTrackIntoState: () => false,
    hydrateTrackPreviewFromBackend: async () => {},
    patchLibraryRowByTrackId: () => {},
    patchPlaylistRowByTrackId: () => {},
    updateLibraryDurationSummary: () => {},
    renderSourceChips: () => {},
    refreshCurrentPlaylistTracks: async () => {},
    countWarningsForStatus: (warnings) => Array.isArray(warnings) ? warnings.length : 0
  });

  assert.equal(result.analyzed, 0);
  assert.equal(result.failed, 1);
  assert.equal(hydrateCalls, 1, "final hydration still runs with empty id list");
  assert.ok(result.warnings.some((w) => String(w).includes("no BPM/key result")));
  assert.ok(statuses.some((s) => s.includes("Analyze missing done: analyzed 0, failed 1")));
});

test("applyRealtimeAnalyzedTrackUpdate skips no-change warning for empty bpm/key payload", async () => {
  const state = {
    tracks: [{ id: "1", artist: "A", title: "T" }],
    playlists: []
  };
  let warnCalls = 0;

  await applyRealtimeAnalyzedTrackUpdate(state, {
    trackId: "1",
    bpm: null,
    key: null,
    bpmAnalyzer: null
  }, {
    patchTrackAnalysisFields: () => false,
    debugFrontendLog: () => {},
    log: () => {},
    warn: () => { warnCalls += 1; },
    patchLibraryRowByTrackId: () => {},
    scheduleRealtimeTrackRender: () => {},
    hydrateTrackPreviewFromBackend: async () => {}
  });

  assert.equal(warnCalls, 0);
});
