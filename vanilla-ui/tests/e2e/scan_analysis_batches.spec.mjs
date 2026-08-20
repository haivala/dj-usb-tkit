import { test, expect } from "./coverage-fixture.mjs";

// NOTE: analysis is dispatched as a single `analyze_new_tracks` batch call per
// analyzeTrackIds() invocation (see components/library/actions.mjs). The
// backend reports progressive per-track/per-piece updates during that call
// via `job:event` events (stage: "analyze_new_tracks", trackReady: false for
// each of the 4 pieces, then a final trackReady: true event per track) — see
// backend/src/service/analysis.rs (analyze_local_track_with_updates,
// build_partial_progress, build_done_progress_success) and
// backend/src/tauri_commands.rs (analyze_new_tracks, emit_job_event_with_track).
// These mocks simulate that same event sequence so the frontend's real
// job_manager.mjs handleJobEvent()/applyRealtimeAnalyzedTrackUpdate() codepath
// is exercised exactly as it is in production.

function installScanAnalysisMock(page, opts = {}) {
  const trackCount = Number(opts?.trackCount || 40);
  const pieceDelayMs = Number(opts?.pieceDelayMs || 60);
  const workers = Number(opts?.workers || 6);
  const seedExistingWaveform = !!opts?.seedExistingWaveform;
  const seedDuration = !!opts?.seedDuration;
  const seedArtwork = !!opts?.seedArtwork;
  const variedArtists = !!opts?.variedArtists;
  const analysisBpmRange = String(opts?.analysisBpmRange || "70-180");
  const pauseBeforeBpmKey = !!opts?.pauseBeforeBpmKey;
  return page.addInitScript(({
    trackCount,
    pieceDelayMs,
    workers,
    seedExistingWaveform,
    seedDuration,
    seedArtwork,
    variedArtists,
    analysisBpmRange,
    pauseBeforeBpmKey
  }) => {
    // registerBackendJobEvents() (components/playback/actions.mjs) gates the
    // "job:event" listen() call behind isTauriRuntime(), which checks
    // window.isTauri (a plain boolean flag set by the real Tauri runtime),
    // not just the presence of window.__TAURI__.event.listen. Without this,
    // the app never subscribes to job:event and our simulated per-piece
    // progress events would be emitted to zero listeners.
    window.isTauri = true;
    // @tauri-apps/api's real invoke()/isTauri() (bundled into main.js) route
    // through window.__TAURI_INTERNALS__.invoke once window.isTauri is set,
    // bypassing window.__TAURI__.core.invoke entirely. Forward it to our mock
    // so every invoke call -- not just event listener registration -- reaches
    // the mock regardless of which path the bundled API code takes.
    window.__TAURI_INTERNALS__ = {
      invoke: (cmd, args) => window.__TAURI__.core.invoke(cmd, args)
    };
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.analysisBpmRange", analysisBpmRange);

    const listeners = new Map();
    let analyzeNewTracksCalls = 0;
    const pieceEventsByPiece = { duration: 0, artwork: 0, waveform: 0, bpm_key: 0 };
    let bpmRangeSeen = null;
    let analysisPaused = false;
    let analysisCancelled = false;
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    const nowIso = () => new Date().toISOString();

    const listen = async (eventName, callback) => {
      const key = String(eventName || "");
      const arr = listeners.get(key) || [];
      arr.push(callback);
      listeners.set(key, arr);
      return () => {
        const current = listeners.get(key) || [];
        listeners.set(key, current.filter((fn) => fn !== callback));
      };
    };

    const emitJobEvent = (jobPayload) => {
      const cbs = listeners.get("job:event") || [];
      for (const cb of cbs.slice()) {
        try {
          cb({ event: "job:event", payload: jobPayload });
        } catch (_) {
          // ignore listener errors in mock
        }
      }
    };

    const tracks = Array.from({ length: Math.max(1, trackCount) }, (_, index) => {
      const n = String(index + 1).padStart(2, "0");
      return {
        id: `track-${n}`,
        title: `Track ${n}`,
        artist: variedArtists ? (index % 2 === 0 ? "B Artist" : "A Artist") : "Batch Artist",
        album: "Batch Album",
        bpm: null,
        key: null,
        filePath: `/music/Batch Artist - Track ${n}.wav`,
        fileSizeBytes: 1000 + index,
        durationMs: null,
        artworkPath: null,
        waveformPeaksPath: null,
        waveformPreview: [],
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      };
    });
    if (seedExistingWaveform) {
      for (const track of tracks) {
        track.waveformPeaksPath = `/tmp/${track.id}.DAT`;
      }
    }
    if (seedDuration) {
      for (const track of tracks) {
        track.durationMs = 180000;
      }
    }
    if (seedArtwork) {
      for (const track of tracks) {
        track.artworkPath = `/tmp/${track.id}.jpg`;
      }
    }

    // Mirrors track_has_core_analysis_for_source_status
    // (backend/src/service/mod.rs): a track counts toward the library
    // duration total only once it has all three of bpm/waveform/duration.
    const isCountable = (track) => Number.isFinite(track.bpm) && track.bpm > 0
      && !!track.waveformPeaksPath
      && Number.isFinite(track.durationMs) && track.durationMs > 0;
    const computeDurationTotals = (list) => {
      let totalMs = 0;
      let knownCount = 0;
      for (const track of list) {
        if (isCountable(track)) {
          totalMs += track.durationMs;
          knownCount += 1;
        }
      }
      return { totalMs, knownCount };
    };

    // Simulates the real backend's analyze_new_tracks_with_progress worker:
    // for each requested track id, emits 4 progressive partial job:event
    // updates (duration, artwork, waveform, bpm_key - trackReady:false, one
    // field group each, matching backend/src/service/analysis.rs's
    // TrackPartialUpdate/on_update sequence) followed by one final
    // trackReady:true event carrying all fields (matching
    // build_done_progress_success). The command's own promise resolves only
    // after every track has been processed, with the real analyzed/failed
    // counts - this matches the real analyze_new_tracks Tauri command, whose
    // handler awaits the whole spawn_blocking batch before returning.
    const runAnalyzeNewTracks = async (ids, bpmMin, bpmMax) => {
      const jobId = `job-analysis-mock-${analyzeNewTracksCalls}`;
      const total = Math.max(1, ids.length);
      // Every new batch starts fresh, matching the real backend's
      // reset-at-start-of-batch behavior for its pause/cancel flags.
      analysisPaused = false;
      analysisCancelled = false;
      // Live library-duration-total baseline (mirrors
      // analyze_new_tracks_with_progress in backend/src/service/analysis.rs):
      // seeded from every *other* track that's already countable, excluding
      // this batch's own tracks so a re-analyzed track isn't double-counted.
      const idSet = new Set(ids.map(String));
      const { totalMs: baselineTotalMs, knownCount: baselineKnownCount } =
        computeDurationTotals(tracks.filter((t) => !idSet.has(t.id)));
      let libraryTotalDurationMs = baselineTotalMs;
      let libraryDurationKnownCount = baselineKnownCount;
      emitJobEvent({
        event: "job.started",
        jobId,
        jobType: "analysis",
        stage: "analyze_new_tracks",
        current: 0,
        total,
        percent: 0,
        message: "Analyzing selected tracks",
        timestamp: nowIso()
      });

      let analyzed = 0;
      let failed = 0;

      for (let i = 0; i < ids.length; i += 1) {
        // Simulates the real worker pool: a pause blocks picking up the
        // next track (the current one, if any, already finished), and a
        // cancel breaks out permanently instead of waiting for a resume.
        while (analysisPaused && !analysisCancelled) {
          await sleep(15);
        }
        if (analysisCancelled) {
          emitJobEvent({
            event: "job.completed",
            jobId,
            jobType: "analysis",
            stage: "analyze_new_tracks",
            current: i,
            total,
            percent: Math.round((i / total) * 100),
            message: `Analysis finished: ${analyzed} analyzed, ${failed} failed`,
            timestamp: nowIso()
          });
          return {
            jobId,
            analyzed,
            failed,
            warnings: [`Analysis cancelled: ${analyzed} of ${total} tracks analyzed`],
            items: tracks.filter((track) => ids.includes(String(track.id)))
          };
        }

        const id = ids[i];
        const track = tracks.find((t) => t.id === id);
        if (!track) {
          failed += 1;
          continue;
        }

        const basePayload = {
          jobType: "analysis",
          stage: "analyze_new_tracks",
          trackId: id,
          trackTitle: track.title,
          filePath: track.filePath
        };
        const emitPartial = (extra) => {
          emitJobEvent({
            event: "job.progress",
            jobId,
            current: i,
            total,
            percent: Math.round((i / total) * 100),
            message: `Analyzing ${i + 1}/${ids.length}: ${track.title}`,
            trackReady: false,
            timestamp: nowIso(),
            ...basePayload,
            ...extra
          });
        };

        await sleep(Math.max(0, pieceDelayMs));
        track.artworkPath = `/tmp/${id}.jpg`;
        pieceEventsByPiece.artwork += 1;
        emitPartial({ artworkPath: track.artworkPath });

        await sleep(Math.max(0, pieceDelayMs));
        track.waveformPeaksPath = `/tmp/${id}.DAT`;
        track.waveformPreview = [8, 20, 42, 65, 30, 55];
        pieceEventsByPiece.waveform += 1;
        emitPartial({
          waveformPeaksPath: track.waveformPeaksPath,
          waveformPreview: track.waveformPreview
        });

        await sleep(Math.max(0, pieceDelayMs));
        track.durationMs = 180000;
        pieceEventsByPiece.duration += 1;
        emitPartial({ durationMs: track.durationMs });

        if (pauseBeforeBpmKey) {
          // Holds here instead of racing a fixed delay against whatever the
          // test does next -- the test releases this explicitly (via
          // window.__releaseBeforeBpmKey) once it's done asserting the
          // track is still not-ready, so that assertion can never lose a
          // timing race against this mock's own progress under load.
          await new Promise((resolve) => { window.__releaseBeforeBpmKey = resolve; });
        }

        await sleep(Math.max(0, pieceDelayMs));
        track.bpm = 120 + (Number(id.replace(/\D+/g, "")) % 4);
        track.key = `${(Number(id.replace(/\D+/g, "")) % 12) + 1}A`;
        track.updatedAt = "2026-03-04T00:00:00Z";
        pieceEventsByPiece.bpm_key += 1;
        emitPartial({ bpm: track.bpm, bpmAnalyzer: "mock-analyzer", key: track.key });

        analyzed += 1;
        if (isCountable(track)) {
          libraryTotalDurationMs += track.durationMs;
          libraryDurationKnownCount += 1;
        }
        emitJobEvent({
          event: "job.progress",
          jobId,
          current: i + 1,
          total,
          percent: Math.round(((i + 1) / total) * 100),
          message: `Analyzing ${i + 1}/${ids.length}: ${track.title}`,
          trackReady: true,
          failed: false,
          timestamp: nowIso(),
          ...basePayload,
          bpm: track.bpm,
          bpmAnalyzer: "mock-analyzer",
          key: track.key,
          durationMs: track.durationMs,
          artworkPath: track.artworkPath,
          waveformPeaksPath: track.waveformPeaksPath,
          waveformPreview: track.waveformPreview,
          libraryTotalDurationMs,
          libraryDurationUnknownCount: Math.max(0, tracks.length - libraryDurationKnownCount)
        });
      }

      emitJobEvent({
        event: "job.completed",
        jobId,
        jobType: "analysis",
        stage: "analyze_new_tracks",
        current: ids.length,
        total,
        percent: 100,
        message: `Analysis finished: ${analyzed} analyzed, ${failed} failed`,
        timestamp: nowIso()
      });

      return {
        jobId,
        analyzed,
        failed,
        warnings: [],
        items: tracks.filter((track) => ids.includes(String(track.id)))
      };
    };

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") {
            return { ok: true, data: { found: false, path: null } };
          }
          if (command === "list_playlists") {
            return { ok: true, data: { items: [] } };
          }
          if (command === "scan_library") {
            return {
              ok: true,
              data: { jobId: "job-scan-mock", indexed: tracks.length, updated: 0, removed: 0 }
            };
          }
          if (command === "search_tracks" || command === "browse_source_files") {
            const query = String(payload?.request?.query ?? payload?.query ?? "").toLowerCase();
            const filtered = tracks.filter((t) => {
              if (!query) return true;
              return `${t.title} ${t.artist} ${t.album}`.toLowerCase().includes(query);
            });
            const { totalMs, knownCount } = computeDurationTotals(filtered);
            return {
              ok: true,
              data: { total: filtered.length, items: filtered, totalDurationMs: totalMs, durationKnownCount: knownCount }
            };
          }
          if (command === "list_tracks") {
            return { ok: true, data: { total: tracks.length, items: tracks } };
          }
          if (command === "get_tracks_by_ids_with_previews") {
            const ids = Array.isArray(payload?.request?.trackIds)
              ? payload.request.trackIds.map((v) => String(v))
              : [];
            const items = tracks.filter((t) => ids.includes(String(t.id)));
            return { ok: true, data: { items } };
          }
          if (command === "analyze_new_tracks") {
            analyzeNewTracksCalls += 1;
            const req = payload?.request || {};
            const ids = Array.isArray(req.trackIds) ? req.trackIds.map((v) => String(v)) : [];
            bpmRangeSeen = {
              min: Number(req.bpmMin || 0),
              max: Number(req.bpmMax || 0)
            };
            const data = await runAnalyzeNewTracks(ids, req.bpmMin, req.bpmMax);
            return { ok: true, data };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          if (command === "set_analysis_paused") {
            analysisPaused = !!(payload?.request?.paused);
            return { ok: true, data: { paused: analysisPaused } };
          }
          if (command === "cancel_analysis") {
            analysisCancelled = true;
            return { ok: true, data: null };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled command: ${command}` } };
        }
      },
      event: { listen }
    };
    // The bundled @tauri-apps/api/core `invoke`/`isTauri` (imported directly
    // by api_client.mjs, not read off window.__TAURI__) is what actually
    // gets called once window.isTauri is true - see api_client.mjs's
    // invoke(): `if (isTauriRuntime()) return tauriInvoke(...)`, which reads
    // window.__TAURI_INTERNALS__.invoke, entirely bypassing
    // window.__TAURI__.core.invoke. Bridge it to the same mock handler.
    window.__TAURI_INTERNALS__ = {
      invoke: (cmd, args, options) => window.__TAURI__.core.invoke(cmd, args, options),
      transformCallback: (callback) => callback,
      convertFileSrc: (filePath) => filePath
    };

    window.__scanTestStats = {
      get analyzeNewTracksCalls() {
        return analyzeNewTracksCalls;
      },
      get pieceEventsByPiece() {
        return pieceEventsByPiece;
      },
      get bpmRangeSeen() {
        return bpmRangeSeen;
      },
      get analysisPaused() {
        return analysisPaused;
      },
      get analysisCancelled() {
        return analysisCancelled;
      }
    };
  }, { trackCount, pieceDelayMs, workers, seedExistingWaveform, seedDuration, seedArtwork, variedArtists, analysisBpmRange, pauseBeforeBpmKey });
}

function installPagedMaterializeAnalyzeMock(page, opts = {}) {
  const trackCount = Number(opts?.trackCount || 260);
  const pageSize = Number(opts?.pageSize || 200);
  const pieceDelayMs = Number(opts?.pieceDelayMs || 80);
  const materializedIds = !!opts?.materializedIds;
  const materializeFails = !!opts?.materializeFails;
  return page.addInitScript(({ trackCount, pageSize, pieceDelayMs, materializedIds, materializeFails }) => {
    // See installScanAnalysisMock for why this is required: registerBackendJobEvents()
    // only calls listen("job:event", ...) when window.isTauri is truthy.
    window.isTauri = true;
    // @tauri-apps/api's real invoke()/isTauri() (bundled into main.js) route
    // through window.__TAURI_INTERNALS__.invoke once window.isTauri is set,
    // bypassing window.__TAURI__.core.invoke entirely. Forward it to our mock
    // so every invoke call -- not just event listener registration -- reaches
    // the mock regardless of which path the bundled API code takes.
    window.__TAURI_INTERNALS__ = {
      invoke: (cmd, args) => window.__TAURI__.core.invoke(cmd, args)
    };
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const listeners = new Map();
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    const nowIso = () => new Date().toISOString();
    let listQueryCount = 0;
    let searchQueryCount = 0;
    let analyzeCalls = 0;
    let materializeCalls = 0;

    const tracks = Array.from({ length: Math.max(1, trackCount) }, (_, index) => {
      const n = String(index + 1).padStart(4, "0");
      const filePath = `/music/Auto Artist - Track ${n}.wav`;
      return {
        id: materializedIds ? `ml-${n}` : filePath,
        localTrackId: materializedIds ? `ml-${n}` : null,
        title: `Track ${n}`,
        artist: "Auto Artist",
        album: "Auto Album",
        bpm: null,
        key: null,
        filePath,
        fileSizeBytes: 10_000 + index,
        durationMs: null,
        artworkPath: null,
        waveformPeaksPath: null,
        waveformPreview: [],
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      };
    });

    const toTrackDto = (row) => {
      const id = row.localTrackId || row.id;
      return {
        id,
        title: row.title,
        artist: row.artist,
        album: row.album,
        bpm: row.bpm,
        key: row.key,
        filePath: row.filePath,
        fileSizeBytes: row.fileSizeBytes,
        durationMs: row.durationMs,
        artworkPath: row.artworkPath,
        waveformPeaksPath: row.waveformPeaksPath,
        waveformPreview: row.waveformPreview,
        createdAt: row.createdAt,
        updatedAt: row.updatedAt
      };
    };

    // Mirrors track_has_core_analysis_for_source_status
    // (backend/src/service/mod.rs): a track counts toward the library
    // duration total only once it has all three of bpm/waveform/duration.
    const isCountable = (track) => Number.isFinite(track.bpm) && track.bpm > 0
      && !!track.waveformPeaksPath
      && Number.isFinite(track.durationMs) && track.durationMs > 0;
    const computeDurationTotals = (list) => {
      let totalMs = 0;
      let knownCount = 0;
      for (const track of list) {
        if (isCountable(track)) {
          totalMs += track.durationMs;
          knownCount += 1;
        }
      }
      return { totalMs, knownCount };
    };

    const listPage = (cursorValue, query = "") => {
      const q = String(query || "").toLowerCase().trim();
      const filtered = q
        ? tracks.filter((t) => `${t.title} ${t.artist} ${t.album}`.toLowerCase().includes(q))
        : tracks.slice();
      const offset = Number(String(cursorValue || "0")) || 0;
      const items = filtered.slice(offset, offset + pageSize).map(toTrackDto);
      const nextOffset = offset + items.length;
      const hasMore = nextOffset < filtered.length;
      const { totalMs, knownCount } = computeDurationTotals(filtered);
      return {
        total: filtered.length,
        items,
        next_cursor: hasMore ? String(nextOffset) : null,
        has_more: hasMore,
        totalDurationMs: totalMs,
        durationKnownCount: knownCount
      };
    };

    const findRowByAnalysisId = (id) => tracks.find((t) => (t.localTrackId || t.id) === id || t.id === id || t.localTrackId === id);

    const emitJobEvent = (jobPayload) => {
      const cbs = listeners.get("job:event") || [];
      for (const cb of cbs.slice()) {
        try {
          cb({ event: "job:event", payload: jobPayload });
        } catch (_) {
          // ignore listener errors in mock
        }
      }
    };

    // Same progressive-per-piece simulation as installScanAnalysisMock,
    // adapted to look rows up via findRowByAnalysisId (id may be a
    // filePath before materialization, or an ml-* localTrackId after).
    const runAnalyzeNewTracks = async (ids) => {
      analyzeCalls += 1;
      const jobId = `job-paged-analysis-mock-${analyzeCalls}`;
      const total = Math.max(1, ids.length);
      emitJobEvent({
        event: "job.started",
        jobId,
        jobType: "analysis",
        stage: "analyze_new_tracks",
        current: 0,
        total,
        percent: 0,
        message: "Analyzing selected tracks",
        timestamp: nowIso()
      });

      let analyzed = 0;
      let failed = 0;

      // Live library-duration-total baseline (mirrors
      // analyze_new_tracks_with_progress in backend/src/service/analysis.rs):
      // seeded from every *other* track that's already countable, excluding
      // this batch's own tracks so a re-analyzed track isn't double-counted.
      const batchRows = new Set(ids.map((id) => findRowByAnalysisId(id)).filter(Boolean));
      const { totalMs: baselineTotalMs, knownCount: baselineKnownCount } =
        computeDurationTotals(tracks.filter((t) => !batchRows.has(t)));
      let libraryTotalDurationMs = baselineTotalMs;
      let libraryDurationKnownCount = baselineKnownCount;

      for (let i = 0; i < ids.length; i += 1) {
        const id = ids[i];
        const row = findRowByAnalysisId(id);
        if (!row) {
          failed += 1;
          continue;
        }

        const basePayload = {
          jobType: "analysis",
          stage: "analyze_new_tracks",
          trackId: id,
          trackTitle: row.title,
          filePath: row.filePath
        };
        const emitPartial = (extra) => {
          emitJobEvent({
            event: "job.progress",
            jobId,
            current: i,
            total,
            percent: Math.round((i / total) * 100),
            message: `Analyzing ${i + 1}/${ids.length}: ${row.title}`,
            trackReady: false,
            timestamp: nowIso(),
            ...basePayload,
            ...extra
          });
        };

        await sleep(Math.max(0, pieceDelayMs));
        row.durationMs = 182000;
        emitPartial({ durationMs: row.durationMs });

        await sleep(Math.max(0, pieceDelayMs));
        row.artworkPath = `/tmp/${id}.jpg`;
        emitPartial({ artworkPath: row.artworkPath });

        await sleep(Math.max(0, pieceDelayMs));
        row.waveformPeaksPath = `/tmp/${id}.DAT`;
        row.waveformPreview = [10, 25, 45, 70, 50, 30];
        emitPartial({ waveformPeaksPath: row.waveformPeaksPath, waveformPreview: row.waveformPreview });

        await sleep(Math.max(0, pieceDelayMs));
        row.bpm = 123;
        row.key = "8A";
        emitPartial({ bpm: row.bpm, bpmAnalyzer: "mock-analyzer", key: row.key });

        analyzed += 1;
        if (isCountable(row)) {
          libraryTotalDurationMs += row.durationMs;
          libraryDurationKnownCount += 1;
        }
        emitJobEvent({
          event: "job.progress",
          jobId,
          current: i + 1,
          total,
          percent: Math.round(((i + 1) / total) * 100),
          message: `Analyzing ${i + 1}/${ids.length}: ${row.title}`,
          trackReady: true,
          failed: false,
          timestamp: nowIso(),
          ...basePayload,
          bpm: row.bpm,
          bpmAnalyzer: "mock-analyzer",
          key: row.key,
          durationMs: row.durationMs,
          artworkPath: row.artworkPath,
          waveformPeaksPath: row.waveformPeaksPath,
          waveformPreview: row.waveformPreview,
          libraryTotalDurationMs,
          libraryDurationUnknownCount: Math.max(0, tracks.length - libraryDurationKnownCount)
        });
      }

      emitJobEvent({
        event: "job.completed",
        jobId,
        jobType: "analysis",
        stage: "analyze_new_tracks",
        current: ids.length,
        total,
        percent: 100,
        message: `Analysis finished: ${analyzed} analyzed, ${failed} failed`,
        timestamp: nowIso()
      });

      return {
        jobId,
        analyzed,
        failed,
        warnings: [],
        items: tracks
          .filter((track) => ids.some((id) => findRowByAnalysisId(id) === track))
          .map(toTrackDto)
      };
    };

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") {
            return { ok: true, data: { found: false, path: null } };
          }
          if (command === "list_playlists") {
            return { ok: true, data: { items: [] } };
          }
          if (command === "get_system_parallelism") {
            return { ok: true, data: { workers: 6 } };
          }
          if (command === "list_tracks") {
            listQueryCount += 1;
            const cursor = payload?.request?.cursor ?? null;
            return { ok: true, data: listPage(cursor, "") };
          }
          if (command === "search_tracks") {
            searchQueryCount += 1;
            const query = payload?.request?.query ?? payload?.query ?? "";
            const cursor = payload?.request?.cursor ?? payload?.cursor ?? null;
            return { ok: true, data: listPage(cursor, query) };
          }
          if (command === "browse_source_files") {
            searchQueryCount += 1;
            const query = payload?.request?.query ?? "";
            const cursor = payload?.request?.cursor ?? null;
            return { ok: true, data: listPage(cursor, query) };
          }
          if (command === "resolve_track_identity") {
            materializeCalls += 1;
            if (materializeFails) {
              return { ok: false, error: { code: "INTERNAL_ERROR", message: "materialize failed" } };
            }
            const filePath = String(payload?.request?.filePath || "");
            const row = tracks.find((t) => t.filePath === filePath);
            if (!row) {
              return { ok: true, data: { trackId: null, resolvedBy: "none", materialized: false } };
            }
            if (!row.localTrackId) {
              const suffix = String(row.filePath).replace(/[^0-9]+/g, "") || "1";
              row.localTrackId = `ml-${suffix}`;
            }
            return {
              ok: true,
              data: { trackId: row.localTrackId, resolvedBy: "materialized", materialized: true }
            };
          }
          if (command === "materialize_source_track") {
            materializeCalls += 1;
            if (materializeFails) {
              return { ok: false, error: { code: "INTERNAL_ERROR", message: "materialize failed" } };
            }
            const filePath = String(payload?.request?.filePath || "");
            const row = tracks.find((t) => t.filePath === filePath);
            if (!row) {
              return { ok: false, error: { code: "NOT_FOUND", message: "path not found" } };
            }
            if (!row.localTrackId) {
              const suffix = String(row.filePath).replace(/[^0-9]+/g, "") || "1";
              row.localTrackId = `ml-${suffix}`;
            }
            return { ok: true, data: { trackId: row.localTrackId } };
          }
          if (command === "analyze_new_tracks") {
            const req = payload?.request || {};
            const ids = Array.isArray(req.trackIds) ? req.trackIds.map((v) => String(v)) : [];
            const data = await runAnalyzeNewTracks(ids);
            return { ok: true, data };
          }
          if (command === "get_tracks_by_ids_with_previews") {
            const ids = Array.isArray(payload?.request?.trackIds)
              ? payload.request.trackIds.map((v) => String(v))
              : [];
            const items = tracks
              .filter((t) => ids.includes(String(t.localTrackId || t.id)) || ids.includes(String(t.id)))
              .map(toTrackDto);
            return { ok: true, data: { items } };
          }
          if (command === "scan_library") {
            return {
              ok: true,
              data: { jobId: "job-scan-mock", indexed: tracks.length, updated: 0, removed: 0 }
            };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled command: ${command}` } };
        }
      },
      event: { listen: async (eventName, callback) => {
        const key = String(eventName || "");
        const arr = listeners.get(key) || [];
        arr.push(callback);
        listeners.set(key, arr);
        return () => {
          const current = listeners.get(key) || [];
          listeners.set(key, current.filter((fn) => fn !== callback));
        };
      } }
    };
    // See installScanAnalysisMock for why this bridge is required.
    window.__TAURI_INTERNALS__ = {
      invoke: (cmd, args, options) => window.__TAURI__.core.invoke(cmd, args, options),
      transformCallback: (callback) => callback,
      convertFileSrc: (filePath) => filePath
    };

    window.__pagedAnalyzeStats = {
      get listQueryCount() {
        return listQueryCount;
      },
      get searchQueryCount() {
        return searchQueryCount;
      },
      get analyzeCalls() {
        return analyzeCalls;
      },
      get materializeCalls() {
        return materializeCalls;
      }
    };
  }, { trackCount, pageSize, pieceDelayMs, materializedIds, materializeFails });
}

test("scan applies per-piece row updates before track-ready status", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 1, pieceDelayMs: 250, pauseBeforeBpmKey: true });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);

  await page.waitForFunction(() => {
    const row = document.querySelector("#libraryTableBody .track-grid-row");
    return !!row && (row.textContent || "").includes("3:00");
  });
  await expect(page.locator("#libraryTableBody .track-grid-row").first()).toHaveClass(/is-analyzing/);
  await expect.poll(async () => {
    const text = await page.locator("#statusText").textContent();
    return (text || "").includes("Scan analysis: 0/1 track(s) ready")
      || (text || "").includes("Analyzing")
      || (text || "").includes("Scan done: 1 tracks / 1 albums | analyzed 1, failed 0");
  }).toBeTruthy();
  await expect(page.locator("#libraryTotalDuration")).toContainText("1 without length");

  // The mock is holding just before the bpm_key piece / trackReady event
  // (see pauseBeforeBpmKey) specifically so the assertion above -- which
  // must observe the track as still not-ready -- can never lose a race
  // against the mock's own progress regardless of system load. Release it
  // now that the not-ready assertions are done.
  await page.evaluate(() => window.__releaseBeforeBpmKey?.());

  await page.waitForFunction(() => {
    const waveform = document.querySelector("#libraryTableBody .track-grid-row .waveform");
    return !!waveform?.classList.contains("waveform-canvas");
  });
  await expect.poll(async () => {
    const text = await page.locator("#statusText").textContent();
    return (text || "").includes("Scan analysis: 0/1 track(s) ready")
      || (text || "").includes("Analyzing")
      || (text || "").includes("Scan done: 1 tracks / 1 albums | analyzed 1, failed 0");
  }).toBeTruthy();

  await expect(page.locator("#statusText")).toContainText("analyzed 1, failed 0");
  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(1);
  await expect(page.locator("#libraryTableBody .key-pill")).toHaveCount(1);
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats?.bpmRangeSeen)).toEqual({ min: 70, max: 180 });
  await expect(page.locator("#libraryTotalDuration")).toHaveText("Total time: 3:00");
  await expect(page.locator("#libraryTableBody .track-grid-row").first()).not.toHaveClass(/is-analyzing/);
});

test("scan progressively changes action buttons to Reanalyze", async ({ page }, testInfo) => {
  // Simulates a full 40-track/4-piece batch (160 progress events + row re-renders); under
  // CPU contention from the rest of the suite running in parallel this can outrun the
  // default 5s expect timeout even though the app itself isn't slow — triple it here.
  testInfo.setTimeout(testInfo.timeout * 3);
  await installScanAnalysisMock(page, { trackCount: 40, pieceDelayMs: 20 });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(40);

  await page.waitForFunction(() => {
    const buttons = Array.from(document.querySelectorAll('#libraryTableBody button[data-action="analyze-track"]'));
    const ready = buttons.filter((el) => String(el.textContent || "").trim() === "Reanalyze").length;
    return ready > 0 && ready < buttons.length;
  });

  const midScanReady = await page
    .locator('#libraryTableBody button[data-action="analyze-track"]', { hasText: "Reanalyze" })
    .count();
  expect(midScanReady).toBeGreaterThan(0);
  expect(midScanReady).toBeLessThan(40);

  await expect(
    page.locator('#libraryTableBody button[data-action="analyze-track"]', { hasText: "Reanalyze" })
  ).toHaveCount(40, { timeout: 15_000 });
  await expect(
    page.locator('#libraryTableBody button[data-action="analyze-track"]', { hasText: /^Analyze$/ })
  ).toHaveCount(0);
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats?.analyzeNewTracksCalls || 0)).toBeGreaterThan(0);
});

test("scan forwards selected BPM range to analyze_new_tracks", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 1, pieceDelayMs: 20, analysisBpmRange: "88-175" });
  await page.goto("/");
  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#statusText")).toContainText("analyzed 1, failed 0");
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats?.bpmRangeSeen)).toEqual({ min: 88, max: 175 });
});

test("analysis populates BPM and key cells for all tracks", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 40, pieceDelayMs: 10 });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(40);
  await expect(
    page.locator('#libraryTableBody button[data-action="analyze-track"]', { hasText: "Reanalyze" })
  ).toHaveCount(40, { timeout: 15_000 });

  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(40);
  await expect(page.locator("#libraryTableBody .key-pill")).toHaveCount(40);
});

test("library total duration advances only when each track is fully ready", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 2, pieceDelayMs: 120, workers: 3 });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(2);

  await page.waitForFunction(() => {
    const row = document.querySelector("#libraryTableBody .track-grid-row");
    const total = document.querySelector("#libraryTotalDuration")?.textContent || "";
    return !!row
      && (row.textContent || "").includes("3:00")
      && total.includes("2 without length");
  });

  await page.waitForFunction(() => {
    const total = document.querySelector("#libraryTotalDuration")?.textContent || "";
    return total.includes("Total time: 3:00 (1 without length)");
  });

  await expect(page.locator("#libraryTotalDuration")).toHaveText("Total time: 6:00");
});

test("pause button stops picking up new tracks and resume continues the batch", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 5, pieceDelayMs: 40 });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#progressPauseBtn")).toBeVisible();
  await expect(page.locator("#progressCancelAnalysisBtn")).toBeVisible();

  await page.waitForFunction(() => {
    return document.querySelectorAll("#libraryTableBody .bpm-pill").length >= 1;
  });
  await page.locator("#progressPauseBtn").click();
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats?.analysisPaused)).toBe(true);

  // A track already in flight when pause was clicked is allowed to finish
  // (same as the backend's own worker loop), so give that a moment to
  // settle before establishing the "paused" baseline count.
  await page.waitForTimeout(250);
  const pausedCount = await page.locator("#libraryTableBody .bpm-pill").count();
  expect(pausedCount).toBeLessThan(5);
  // Give the batch plenty of time to have picked up further tracks if pause
  // wasn't actually blocking new work.
  await page.waitForTimeout(300);
  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(pausedCount);
  // Only once the batch has genuinely stopped (no track still in flight)
  // does the elapsed timer freeze on "(paused)" -- this is checked after
  // settling rather than right after the click, since the click itself only
  // stops new tracks from starting, not whatever was already running.
  await expect(page.locator("#progressText")).toContainText("(paused)");

  await page.locator("#progressPauseBtn").click();
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats?.analysisPaused)).toBe(false);
  await expect(page.locator("#progressText")).not.toContainText("(paused)");

  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(5);
  await expect(page.locator("#statusText")).toContainText("analyzed 5, failed 0");
  await expect(page.locator("#progressPauseBtn")).toBeHidden();
  await expect(page.locator("#progressCancelAnalysisBtn")).toBeHidden();
});

test("pause clicked mid-track keeps the elapsed timer counting until that track actually finishes", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 3, pieceDelayMs: 300 });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#progressPauseBtn")).toBeVisible();

  // Click pause partway through track 1's own pieces (after "waveform", one
  // piece before it would report ready) so it's guaranteed to still be
  // in-flight when the click lands. Dispatched as a direct in-page DOM
  // click (not Playwright's locator.click()) so the timing isn't at the
  // mercy of actionability/stability retries while the table is actively
  // repainting -- those can add hundreds of ms of real time and blow past
  // the very window this test is trying to land in.
  await page.waitForFunction(() => (window.__scanTestStats?.pieceEventsByPiece?.waveform || 0) >= 1);
  await page.evaluate(() => document.getElementById("progressPauseBtn").click());

  // The in-flight track hasn't reported ready yet, so the batch hasn't
  // really stopped -- the timer must still be ticking, not frozen.
  await expect(page.locator("#progressText")).not.toContainText("(paused)");

  // Once that track finishes (its bpm/key piece lands), nothing else starts
  // because we're paused, so now it has genuinely stopped.
  await expect(page.locator("#progressText")).toContainText("(paused)");
  const settledCount = await page.locator("#libraryTableBody .bpm-pill").count();
  expect(settledCount).toBeLessThan(3);

  // Stays stopped -- no further tracks pick up.
  await page.waitForTimeout(300);
  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(settledCount);
});

test("cancel button stops the batch early and reports how many tracks completed", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 5, pieceDelayMs: 40 });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#progressCancelAnalysisBtn")).toBeVisible();

  await page.waitForFunction(() => {
    return document.querySelectorAll("#libraryTableBody .bpm-pill").length >= 1;
  });
  await page.locator("#progressCancelAnalysisBtn").click();

  await expect(page.locator("#progressPauseBtn")).toBeHidden();
  await expect(page.locator("#progressCancelAnalysisBtn")).toBeHidden();
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats?.analysisCancelled)).toBe(true);

  // The batch must not hang waiting on tracks that will never come; the
  // status line should settle on "Scan done: ... | analyzed N, failed 0"
  // instead of staying on "Analyzing...". Read N from the status text
  // itself (the frontend's own source of truth for how many completed)
  // rather than a DOM snapshot taken mid-flight, since one extra track can
  // legitimately finish in the small window between clicking cancel and the
  // coordinator observing it -- same race as the backend's own test for
  // this (see analyze_new_tracks_cancel_stops_early_without_hanging).
  let finalCount = null;
  await expect.poll(async () => {
    const text = (await page.locator("#statusText").textContent()) || "";
    const match = text.match(/analyzed (\d+), failed 0/);
    if (!match) return null;
    finalCount = Number(match[1]);
    return finalCount;
  }).not.toBeNull();

  expect(finalCount).toBeGreaterThan(0);
  expect(finalCount).toBeLessThan(5);
  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(finalCount);

  // No further tracks should show up after settling.
  await page.waitForTimeout(200);
  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(finalCount);
});

test("scan recomputes waveform piece even when one already exists", async ({ page }) => {
  // analyze_new_tracks (backend/src/service/analysis.rs::analyze_local_track_with_updates)
  // has no "skip if already analyzed" branching - it always recomputes
  // artwork, waveform, duration, and bpm/key for every requested track.
  await installScanAnalysisMock(page, {
    trackCount: 1,
    pieceDelayMs: 20,
    seedExistingWaveform: true,
    seedDuration: true,
    seedArtwork: true
  });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);
  await expect(page.locator("#statusText")).toContainText("analyzed 1, failed 0");

  await expect.poll(async () => {
    return page.evaluate(() => window.__scanTestStats?.pieceEventsByPiece?.waveform || 0);
  }).toBeGreaterThan(0);
  await expect.poll(async () => {
    return page.evaluate(() => window.__scanTestStats?.pieceEventsByPiece?.bpm_key || 0);
  }).toBeGreaterThan(0);
});

test("analysis patch updates BPM cell in-place without replacing row node", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 1, pieceDelayMs: 120 });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);

  const initial = await page.evaluate(() => {
    const row = document.querySelector("#libraryTableBody .track-grid-row");
    const bpmCell = row?.querySelector(".td-bpm");
    return {
      rowId: row?.dataset?.trackId || "",
      hasBpmPill: !!bpmCell?.querySelector(".bpm-pill"),
      bpmText: String(bpmCell?.textContent || "").trim()
    };
  });
  expect(initial.hasBpmPill).toBeFalsy();
  expect(initial.bpmText).toBe("-");

  await page.waitForFunction(() => {
    const row = document.querySelector("#libraryTableBody .track-grid-row");
    const bpmCell = row?.querySelector(".td-bpm");
    return !!bpmCell?.querySelector(".bpm-pill");
  });

  const after = await page.evaluate(() => {
    const row = document.querySelector("#libraryTableBody .track-grid-row");
    const bpmCell = row?.querySelector(".td-bpm");
    return {
      rowId: row?.dataset?.trackId || "",
      hasBpmPill: !!bpmCell?.querySelector(".bpm-pill")
    };
  });
  expect(after.hasBpmPill).toBeTruthy();
  expect(after.rowId).toBe(initial.rowId);
});

test("sorted library order stays stable during live analysis patching", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 8, pieceDelayMs: 60, variedArtists: true });
  await page.goto("/");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(8);

  const sortHeader = page.locator('#panel-library .sortable[data-sort-key="artist"]');
  await sortHeader.click();
  const before = await page.locator("#libraryTableBody .track-grid-row .track-title").allTextContents();

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#statusText")).toContainText("analyzed 8, failed 0");
  const after = await page.locator("#libraryTableBody .track-grid-row .track-title").allTextContents();

  expect(after).toEqual(before);
});

test("analyze on auto-loaded non-materialized track resolves local id", async ({ page }) => {
  await installPagedMaterializeAnalyzeMock(page, { trackCount: 260, pageSize: 200, pieceDelayMs: 120 });
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(200);

  await page.evaluate(() => {
    const wrap = document.querySelector("#libraryTableWrap");
    if (!wrap) return;
    wrap.scrollTop = wrap.scrollHeight;
    wrap.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(260);

  const targetRow = page.locator("#libraryTableBody .track-grid-row").nth(230);
  await targetRow.locator('[data-action="analyze-track"]').click();

  await expect(page.locator("#statusText")).not.toContainText("Track is not in local library yet");
  await expect.poll(async () => {
    return page.evaluate(() => Number(window.__pagedAnalyzeStats?.materializeCalls || 0));
  }).toBeGreaterThan(0);
  await expect.poll(async () => {
    return page.evaluate(() => Number(window.__pagedAnalyzeStats?.analyzeCalls || 0));
  }).toBeGreaterThan(0);
});

test("analyze reports the not-in-local-library message when materializing the track fails", async ({ page }) => {
  await installPagedMaterializeAnalyzeMock(page, {
    trackCount: 260, pageSize: 200, pieceDelayMs: 120, materializeFails: true
  });
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(200);

  await page.evaluate(() => {
    const wrap = document.querySelector("#libraryTableWrap");
    if (!wrap) return;
    wrap.scrollTop = wrap.scrollHeight;
    wrap.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(260);

  const targetRow = page.locator("#libraryTableBody .track-grid-row").nth(230);
  await targetRow.locator('[data-action="analyze-track"]').click();

  await expect(page.locator("#statusText")).toContainText(
    "Track is not in local library yet. Scan library first, then analyze."
  );
  const analyzeCalls = await page.evaluate(() => Number(window.__pagedAnalyzeStats?.analyzeCalls || 0));
  expect(analyzeCalls).toBe(0);
});

test("analyze on auto-loaded track updates row in place without full reload", async ({ page }) => {
  await installPagedMaterializeAnalyzeMock(page, {
    trackCount: 260,
    pageSize: 200,
    pieceDelayMs: 120,
    materializedIds: true
  });
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(200);

  await page.evaluate(() => {
    const wrap = document.querySelector("#libraryTableWrap");
    if (!wrap) return;
    wrap.scrollTop = wrap.scrollHeight;
    wrap.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(260);

  const beforeStats = await page.evaluate(() => ({
    list: Number(window.__pagedAnalyzeStats?.listQueryCount || 0),
    search: Number(window.__pagedAnalyzeStats?.searchQueryCount || 0)
  }));

  const targetRow = page.locator("#libraryTableBody .track-grid-row").nth(230);
  const before = await targetRow.evaluate((row) => ({
    trackId: String(row.getAttribute("data-track-id") || ""),
    hasBpmPill: !!row.querySelector(".bpm-pill")
  }));
  expect(before.trackId.startsWith("ml-")).toBeTruthy();
  expect(before.hasBpmPill).toBeFalsy();

  await targetRow.locator('[data-action="analyze-track"]').click();

  await expect(page.locator("#statusText")).toContainText("Analyze missing done: analyzed 1, failed 0");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(260);

  const afterTrackId = await page.locator("#libraryTableBody .track-grid-row").nth(230)
    .evaluate((row) => String(row.getAttribute("data-track-id") || ""));
  expect(afterTrackId).toBe(before.trackId);

  const afterStats = await page.evaluate(() => ({
    list: Number(window.__pagedAnalyzeStats?.listQueryCount || 0),
    search: Number(window.__pagedAnalyzeStats?.searchQueryCount || 0),
    analyzeCalls: Number(window.__pagedAnalyzeStats?.analyzeCalls || 0)
  }));
  expect(afterStats.analyzeCalls).toBeGreaterThan(0);
  expect(afterStats.list).toBe(beforeStats.list);
  expect(afterStats.search).toBeGreaterThanOrEqual(beforeStats.search);
});

// A single-track library with analyze_new_tracks's response fully under the
// test's control (no per-piece job:event choreography needed -- that's not
// what these two tests are about) for exercising status-line formatting of
// unusual `analyze_new_tracks` response shapes: a partial-failure count, and
// a structured WarningEntry object (as opposed to a plain string) that must
// still render its `message`, not "[object Object]".
function installAnalyzeResponseMock(page, { analyzeResponse }) {
  return page.addInitScript(({ analyzeResponse }) => {
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const track = {
      id: "t-1",
      title: "Track One",
      artist: "Artist",
      album: "Album",
      filePath: "/music/Track One.mp3",
      fileSizeBytes: 1000,
      bpm: null,
      key: null,
      durationMs: null,
      waveformPeaksPath: null,
      waveformPreview: [],
      createdAt: "2026-03-01T00:00:00Z",
      updatedAt: "2026-03-01T00:00:00Z"
    };

    window.__TAURI__ = {
      core: {
        invoke: async (command) => {
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") {
            return { ok: true, data: { found: false, path: null } };
          }
          if (command === "list_playlists") return { ok: true, data: { items: [] } };
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          if (command === "list_tracks" || command === "search_tracks" || command === "browse_source_files") {
            return { ok: true, data: { total: 1, items: [track] } };
          }
          if (command === "analyze_new_tracks") {
            return { ok: true, data: analyzeResponse };
          }
          if (command === "get_tracks_by_ids_with_previews") {
            return { ok: true, data: { items: [] } };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled command: ${command}` } };
        }
      },
      event: { listen: async () => () => {} }
    };
  }, { analyzeResponse });
}

test("a failed analyze_new_tracks count is reported in the status line", async ({ page }) => {
  await installAnalyzeResponseMock(page, {
    analyzeResponse: {
      jobId: "job-1",
      analyzed: 0,
      failed: 1,
      warnings: ["1 bpm_key (essentia): no BPM/key result"]
    }
  });
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);
  await page.locator('[data-action="analyze-track"]').click();

  await expect(page.locator("#statusText")).toContainText("done: analyzed 0, failed 1");
});

test("a structured warning entry's message renders in the status line instead of [object Object]", async ({ page }) => {
  await installAnalyzeResponseMock(page, {
    analyzeResponse: {
      jobId: "job-1",
      analyzed: 1,
      failed: 0,
      warnings: [{
        level: "info",
        code: "analysis.auto-select-limit",
        message: "Auto analysis limit reached: selected 1 of 5 eligible tracks (limit 1). Run analysis again or select tracks explicitly to continue.",
        source: "analysis"
      }]
    }
  });
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);
  await page.locator('[data-action="analyze-track"]').click();

  await expect(page.locator("#statusText")).toContainText(
    "Auto analysis limit reached: selected 1 of 5 eligible tracks"
  );
  await expect(page.locator("#statusText")).not.toContainText("[object Object]");
});
