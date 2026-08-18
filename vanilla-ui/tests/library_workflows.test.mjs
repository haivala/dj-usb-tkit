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

test("analyzeTrackIds calls analyze_new_tracks and hydrates bpm/key from the response", async () => {
  const state = {
    tracks: [{ id: "1", bpm: null, key: null, durationMs: null, waveformPreview: [] }],
    analysisBpmRange: "full",
    analyzingTrackIds: new Set()
  };
  const analyzeCalls = [];
  const merged = [];

  await analyzeTrackIds(state, ["1"], "Analyze", {}, {
    parseAnalysisBpmRange: () => ({ min: null, max: null }),
    command: async (name, args = {}) => {
      if (name === "analyze_new_tracks") {
        analyzeCalls.push(args);
        return { jobId: "job-1", analyzed: 1, failed: 0, warnings: [] };
      }
      if (name === "get_tracks_by_ids_with_previews") {
        return { items: [{ id: "1", bpm: 140, bpmAnalyzer: "stratum", key: "Am", durationMs: 60000, waveformPreview: [10, 20] }] };
      }
      return {};
    },
    setStatus: () => {},
    setTrackAnalyzingState: () => {},
    nextPaint: async () => {},
    mergeHydratedTrackIntoState: (item) => { merged.push(item); return true; },
    patchLibraryRowByTrackId: () => {},
    patchPlaylistRowByTrackId: () => {},
    applySearchLocalFilter: () => {},
    renderSourceChips: () => {},
    refreshCurrentPlaylistTracks: async () => {},
    countWarningsForStatus: () => 0
  });

  assert.equal(analyzeCalls.length, 1);
  assert.deepEqual(analyzeCalls[0].trackIds, ["1"]);
  const bpmMerge = merged.find((item) => Number(item?.bpm) === 140);
  assert.ok(bpmMerge);
  assert.equal(bpmMerge.bpmAnalyzer, "stratum");
});

test("analyzeTrackIds reports failed count from analyze_new_tracks response", async () => {
  const state = {
    tracks: [{ id: "1", bpm: null, key: null, durationMs: null, waveformPreview: [] }],
    analysisBpmRange: "full",
    analysisEngine: "essentia",
    analyzingTrackIds: new Set()
  };
  const statuses = [];
  let hydrateCalls = 0;

  const result = await analyzeTrackIds(state, ["1"], "Analyze missing", {}, {
    parseAnalysisBpmRange: () => ({ min: 70, max: 180 }),
    command: async (name) => {
      if (name === "analyze_new_tracks") {
        return { jobId: "job-1", analyzed: 0, failed: 1, warnings: ["1 bpm_key (essentia): no BPM/key result"] };
      }
      if (name === "get_tracks_by_ids_with_previews") {
        hydrateCalls += 1;
        return { items: [] };
      }
      return {};
    },
    setStatus: (text) => { statuses.push(String(text || "")); },
    setTrackAnalyzingState: () => {},
    nextPaint: async () => {},
    mergeHydratedTrackIntoState: () => false,
    patchLibraryRowByTrackId: () => {},
    patchPlaylistRowByTrackId: () => {},
    applySearchLocalFilter: () => {},
    renderSourceChips: () => {},
    refreshCurrentPlaylistTracks: async () => {},
    countWarningsForStatus: (warnings) => Array.isArray(warnings) ? warnings.length : 0
  });

  assert.equal(result.analyzed, 0);
  assert.equal(result.failed, 1);
  assert.equal(hydrateCalls, 1, "final hydration still runs after a batch call");
  assert.ok(result.warnings.some((w) => String(w).includes("no BPM/key result")));
});

test("analyzeTrackIds surfaces the auto-select-limit notice from a structured WarningEntry, not [object Object]", async () => {
  const state = {
    tracks: [{ id: "1", bpm: null, key: null, durationMs: null, waveformPreview: [] }],
    analysisBpmRange: "full",
    analyzingTrackIds: new Set()
  };
  const statuses = [];

  const result = await analyzeTrackIds(state, ["1"], "Analyze", {}, {
    parseAnalysisBpmRange: () => ({ min: null, max: null }),
    command: async (name) => {
      if (name === "analyze_new_tracks") {
        return {
          jobId: "job-1",
          analyzed: 1,
          failed: 0,
          warnings: [{
            level: "info",
            code: "analysis.auto-select-limit",
            message: "Auto analysis limit reached: selected 1 of 5 eligible tracks (limit 1). Run analysis again or select tracks explicitly to continue.",
            source: "analysis"
          }]
        };
      }
      if (name === "get_tracks_by_ids_with_previews") return { items: [] };
      return {};
    },
    setStatus: (text) => { statuses.push(String(text || "")); },
    setTrackAnalyzingState: () => {},
    nextPaint: async () => {},
    mergeHydratedTrackIntoState: () => false,
    patchLibraryRowByTrackId: () => {},
    patchPlaylistRowByTrackId: () => {},
    applySearchLocalFilter: () => {},
    renderSourceChips: () => {},
    refreshCurrentPlaylistTracks: async () => {},
    countWarningsForStatus: () => 0
  });

  assert.equal(result.warnings.length, 1);
  const finalStatus = statuses[statuses.length - 1];
  assert.doesNotMatch(finalStatus, /\[object Object\]/);
  assert.match(finalStatus, /Auto analysis limit reached: selected 1 of 5 eligible tracks/);
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
    hydrateTrackPreviewFromBackend: async () => {}
  });

  assert.equal(warnCalls, 0);
});
