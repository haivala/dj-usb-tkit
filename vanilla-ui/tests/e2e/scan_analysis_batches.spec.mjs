import { test, expect } from "./coverage-fixture.mjs";

// NOTE: `analyze_new_tracks` adds to the backend's analysis job and returns at
// once (see components/library/actions.mjs). The job then reports progressive
// per-track/per-piece updates via `job:event` events (stage: "analyze_new_tracks"; a trackStarted event
// as a worker picks the track up, trackReady: false for each of the 4 pieces,
// then a final trackReady: true event per track) — see
// backend/src/service/analysis.rs (analyze_local_track_with_updates,
// build_partial_progress, build_done_progress_success),
// backend/src/service/analysis/coordinator.rs and
// backend/src/tauri_commands.rs (emit_analysis_event).
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
  const holdAfterTrackStart = !!opts?.holdAfterTrackStart;
  const seedBpmKey = !!opts?.seedBpmKey;
  const scanWarnings = Array.isArray(opts?.scanWarnings) ? opts.scanWarnings : [];
  return page.addInitScript(({
    trackCount,
    pieceDelayMs,
    workers,
    seedExistingWaveform,
    seedDuration,
    seedArtwork,
    variedArtists,
    analysisBpmRange,
    pauseBeforeBpmKey,
    holdAfterTrackStart,
    seedBpmKey,
    scanWarnings
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
    // The library\'s source folders, as `get_library_sources` answers.
    window.__e2eSourceFolders = ["/music"];
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.analysisBpmRange", analysisBpmRange);

    const listeners = new Map();
    let analyzeNewTracksCalls = 0;
    const claimedTrackIds = new Set();
    const pieceEventsByPiece = { duration: 0, artwork: 0, waveform: 0, bpm_key: 0 };
    let bpmRangeSeen = null;
    let analysisPaused = false;
    let analysisCancelled = false;
    const logFileLines = [];
    let openLogFileCalls = 0;
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
    if (seedBpmKey) {
      for (const track of tracks) {
        track.bpm = 99;
        track.key = "8B";
      }
    }

    // Mirrors track_has_core_analysis_for_source_status
    // (backend/src/service/source_roots.rs): a track counts toward the library
    // duration total only once it has all three of bpm/waveform/duration.
    const isCountable = (track) => Number.isFinite(track.bpm) && track.bpm > 0
      && !!track.waveformPeaksPath
      && Number.isFinite(track.durationMs) && track.durationMs > 0;
    // The footer's "Total time" line, as the backend words it.
    const totalTimeText = (totalMs, unknown) => {
      const seconds = Math.floor(totalMs / 1000);
      const time = `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
      return `Total time: ${time}${unknown ? ` (${unknown} without length)` : ""}`;
    };
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

    // Backend stamps analysisReady on every Track row (has_core_analysis_fields).
    const withAnalysisReady = (list) => list.map((t) => ({ ...t, analysisReady: isCountable(t) }));

    // Simulates the real backend's analyze_new_tracks_with_progress worker:
    // for each requested track id, emits 4 progressive partial job:event
    // updates (duration, artwork, waveform, bpm_key - trackReady:false, one
    // field group each, matching backend/src/service/analysis.rs's
    // TrackPartialUpdate/on_update sequence) followed by one final
    // trackReady:true event carrying all fields (matching
    // build_done_progress_success). The command itself has returned by then,
    // as the real one does; `job.completed` carries the result.
    const runAnalyzeNewTracks = async (jobId, ids, bpmMin, bpmMax) => {
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
            message: `Analysis finished: analyzed ${analyzed}, failed ${failed}`,
            timestamp: nowIso()
          });
          return;
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

        // A worker picked the track up (build_started_progress): sent before
        // the decode, and a reanalysis when the track already has a waveform.
        emitPartial({ trackStarted: true, reanalysis: !!track.waveformPeaksPath });
        if (holdAfterTrackStart) {
          // Stands in for the decode + BPM/key detection, where the real
          // backend emits nothing: held until the test releases it, so it can
          // assert the row's state while the track is being worked on.
          await new Promise((resolve) => { window.__heldTrack = { id, release: resolve }; });
          window.__heldTrack = null;
        }

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
        emitPartial({ bpm: track.bpm, bpmSourceText: "Analyzed with: mock-analyzer", key: track.key });

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
          analysisReady: isCountable(track),
          timestamp: nowIso(),
          ...basePayload,
          bpm: track.bpm,
          bpmAnalyzer: "mock-analyzer",
          bpmSourceText: "Analyzed with: mock-analyzer",
          key: track.key,
          durationMs: track.durationMs,
          artworkPath: track.artworkPath,
          waveformPeaksPath: track.waveformPeaksPath,
          waveformPreview: track.waveformPreview,
          libraryDurationText: totalTimeText(
            libraryTotalDurationMs,
            Math.max(0, tracks.length - libraryDurationKnownCount)
          ),
          // The track's row as saved, whole, as the backend sends it.
          track: { ...track, analysisReady: isCountable(track) },
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
        message: `Analysis finished: analyzed ${analyzed}, failed ${failed}`,
        timestamp: nowIso()
      });

    };

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "append_frontend_log") {
            logFileLines.push(`[${payload?.level}] ${payload?.message}`);
            return null;
          }
          if (command === "open_frontend_log") {
            openLogFileCalls += 1;
            return null;
          }
          if (command === "scan_library") {
            // The backend logs each warning as it makes it (`logging::log`).
            for (const warning of scanWarnings) window.__emitBackendLog?.(warning);
            // Backend-owned post-scan library facts (see ScanLibraryData).
            return {
              ok: true,
              data: {
                jobId: "job-scan-mock",
                indexed: tracks.length,
                updated: 0,
                removed: 0,
                warnings: scanWarnings
              }
            };
          }
          if (command === "browse_source_files") {
            const query = String(payload?.request?.query ?? payload?.query ?? "").toLowerCase();
            const filtered = tracks.filter((t) => {
              if (!query) return true;
              return `${t.title} ${t.artist} ${t.album}`.toLowerCase().includes(query);
            });
            const { totalMs, knownCount } = computeDurationTotals(filtered);
            return {
              ok: true,
              data: { total: filtered.length, items: withAnalysisReady(filtered), durationText: totalTimeText(totalMs, filtered.length - knownCount) }
            };
          }
          if (command === "resolve_track_identity") {
            const id = String(payload?.request?.trackId ?? payload?.trackId ?? "");
            return { ok: true, data: { trackId: tracks.some((t) => t.id === id) ? id : null } };
          }
          if (command === "analyze_new_tracks") {
            analyzeNewTracksCalls += 1;
            const req = payload?.request || {};
            let ids = Array.isArray(req.trackIds) ? req.trackIds.map((v) => String(v)) : [];
            // Backend-scoped request: the caller sends no ids, the backend picks
            // the tracks that still need core analysis (here: the whole library).
            if (!ids.length && (req.scopeToLibraryFilter || req.playlistId)) {
              ids = tracks.filter((t) => !isCountable(t)).map((t) => String(t.id));
            }
            // Backend parses the `bpmRange` string (service::analysis); mirror that.
            const [rangeMin, rangeMax] = String(req.bpmRange || "").split("-").map((n) => Number(n.trim()));
            bpmRangeSeen = {
              min: Number.isFinite(rangeMin) ? rangeMin : Number(req.bpmMin || 0),
              max: Number.isFinite(rangeMax) ? rangeMax : Number(req.bpmMax || 0)
            };
            // A track already in the job is skipped, as the backend's coordinator does.
            const alreadyAnalyzingTrackIds = ids.filter((id) => claimedTrackIds.has(id));
            ids = ids.filter((id) => !claimedTrackIds.has(id));
            ids.forEach((id) => claimedTrackIds.add(id));
            const jobId = `job-analysis-mock-${analyzeNewTracksCalls}`;
            if (ids.length) {
              runAnalyzeNewTracks(jobId, ids, bpmRangeSeen.min, bpmRangeSeen.max)
                .finally(() => ids.forEach((id) => claimedTrackIds.delete(id)));
            }
            const data = {
              jobId,
              queued: ids.length,
              skipped: alreadyAnalyzingTrackIds.length,
              message: ids.length
                ? `Analyzing ${ids.length} tracks`
                : alreadyAnalyzingTrackIds.length ? "Already analyzing 1 track" : "No tracks need analysis",
            };
            return { ok: true, data };
          }
          if (command === "set_analysis_paused") {
            analysisPaused = !!(payload?.request?.paused);
            return { ok: true, data: { paused: analysisPaused } };
          }
          if (command === "cancel_analysis") {
            analysisCancelled = true;
            return { ok: true, data: null };
          }
          return window.__e2eBackend.answer(command);
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

    // Lets a test run a job of its own (an export) alongside the analysis.
    window.__emitJobEvent = emitJobEvent;
    // What the backend's `logging::log` sends as it makes a warning.
    window.__emitBackendLog = (logPayload) => {
      for (const cb of (listeners.get("backend:log") || []).slice()) cb({ event: "backend:log", payload: logPayload });
    };
    window.__hasJobListener = () => (listeners.get("job:event") || []).length > 0;

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
      },
      get logFileLines() {
        return logFileLines;
      },
      get openLogFileCalls() {
        return openLogFileCalls;
      }
    };
  }, { trackCount, pieceDelayMs, workers, seedExistingWaveform, seedDuration, seedArtwork, variedArtists, analysisBpmRange, pauseBeforeBpmKey, holdAfterTrackStart, seedBpmKey, scanWarnings });
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
    // The library\'s source folders, as `get_library_sources` answers.
    window.__e2eSourceFolders = ["/music"];
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const listeners = new Map();
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    const nowIso = () => new Date().toISOString();
    let browseQueryCount = 0;
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
        updatedAt: row.updatedAt,
        // Backend computes analysisReady from DB fields (has_core_analysis_fields).
        analysisReady: isCountable(row)
      };
    };

    // Mirrors track_has_core_analysis_for_source_status
    // (backend/src/service/source_roots.rs): a track counts toward the library
    // duration total only once it has all three of bpm/waveform/duration.
    const isCountable = (track) => Number.isFinite(track.bpm) && track.bpm > 0
      && !!track.waveformPeaksPath
      && Number.isFinite(track.durationMs) && track.durationMs > 0;
    // The footer's "Total time" line, as the backend words it.
    const totalTimeText = (totalMs, unknown) => {
      const seconds = Math.floor(totalMs / 1000);
      const time = `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
      return `Total time: ${time}${unknown ? ` (${unknown} without length)` : ""}`;
    };
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

    const listPage = (cursorValue, query = "", sortBy = null, sortDir = null) => {
      const q = String(query || "").toLowerCase().trim();
      let rows = q
        ? tracks.filter((t) => `${t.title} ${t.artist} ${t.album}`.toLowerCase().includes(q))
        : tracks.slice();
      if (sortBy) {
        // Mirrors the backend TrackSort: "artist" ties break by title.
        const m = sortDir === "desc" ? -1 : 1;
        rows = rows.slice().sort((a, b) => {
          const av = sortBy === "artist" ? `${a.artist} ${a.title}` : String(a[sortBy] ?? "");
          const bv = sortBy === "artist" ? `${b.artist} ${b.title}` : String(b[sortBy] ?? "");
          return av < bv ? -m : av > bv ? m : 0;
        });
      }
      const offset = Number(String(cursorValue || "0")) || 0;
      const items = rows.slice(offset, offset + pageSize).map(toTrackDto);
      const nextOffset = offset + items.length;
      const hasMore = nextOffset < rows.length;
      const { totalMs, knownCount } = computeDurationTotals(rows);
      return {
        total: rows.length,
        items,
        nextCursor: hasMore ? String(nextOffset) : null,
        hasMore,
        durationText: totalTimeText(totalMs, rows.length - knownCount)
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
    const runAnalyzeNewTracks = async (jobId, ids) => {
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
        emitPartial({ bpm: row.bpm, bpmSourceText: "Analyzed with: mock-analyzer", key: row.key });

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
          analysisReady: isCountable(row),
          timestamp: nowIso(),
          ...basePayload,
          bpm: row.bpm,
          bpmAnalyzer: "mock-analyzer",
          bpmSourceText: "Analyzed with: mock-analyzer",
          key: row.key,
          durationMs: row.durationMs,
          artworkPath: row.artworkPath,
          waveformPeaksPath: row.waveformPeaksPath,
          waveformPreview: row.waveformPreview,
          libraryDurationText: totalTimeText(
            libraryTotalDurationMs,
            Math.max(0, tracks.length - libraryDurationKnownCount)
          ),
          // The track's row as saved, whole, as the backend sends it.
          track: { ...row, analysisReady: isCountable(row) },
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
        message: `Analysis finished: analyzed ${analyzed}, failed ${failed}`,
        timestamp: nowIso()
      });

    };

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "get_system_parallelism") {
            return { ok: true, data: { workers: 6 } };
          }
          if (command === "browse_source_files") {
            browseQueryCount += 1;
            const query = payload?.request?.query ?? "";
            const cursor = payload?.request?.offset ?? null;
            const sortBy = payload?.request?.sortBy ?? null;
            const sortDir = payload?.request?.sortDir ?? null;
            return { ok: true, data: listPage(cursor, query, sortBy, sortDir) };
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
          if (command === "analyze_new_tracks") {
            const req = payload?.request || {};
            const ids = Array.isArray(req.trackIds) ? req.trackIds.map((v) => String(v)) : [];
            analyzeCalls += 1;
            const jobId = `job-paged-analysis-mock-${analyzeCalls}`;
            if (ids.length) runAnalyzeNewTracks(jobId, ids);
            return { ok: true, data: { jobId, queued: ids.length, skipped: 0, message: `Analyzing ${ids.length} tracks` } };
          }
          if (command === "scan_library") {
            return {
              ok: true,
              data: { jobId: "job-scan-mock", indexed: tracks.length, updated: 0, removed: 0 }
            };
          }
          return window.__e2eBackend.answer(command);
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
      get browseQueryCount() {
        return browseQueryCount;
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

  await page.locator("#analyzeLibraryBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);

  await page.waitForFunction(() => {
    const row = document.querySelector("#libraryTableBody .track-grid-row");
    return !!row && (row.textContent || "").includes("3:00");
  });
  await expect(page.locator("#libraryTableBody .track-grid-row").first()).toHaveClass(/is-analyzing/);
  await expect.poll(async () => {
    const text = await page.locator("#statusText").textContent();
    return (text || "").includes("Analyzing")
      || (text || "").includes("Analysis finished: analyzed 1, failed 0");
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
    return (text || "").includes("Analyzing")
      || (text || "").includes("Analysis finished: analyzed 1, failed 0");
  }).toBeTruthy();

  await expect(page.locator("#statusText")).toContainText("analyzed 1, failed 0");
  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(1);
  await expect(page.locator("#libraryTableBody .key-pill")).toHaveCount(1);
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats?.bpmRangeSeen)).toEqual({ min: 70, max: 180 });
  await expect(page.locator("#libraryTotalDuration")).toHaveText("Total time: 3:00");
  await expect(page.locator("#libraryTableBody .track-grid-row").first()).not.toHaveClass(/is-analyzing/);
});

async function startMockExport(page) {
  await page.waitForFunction(() => window.__hasJobListener?.());
  await page.evaluate(() => window.__emitJobEvent({
    event: "job.started",
    jobId: "export-1",
    jobType: "export",
    stage: "export",
    percent: 10,
    message: "USB: Exporting playlist"
  }));
  await expect(page.locator("#progressText")).toHaveText("USB: Exporting playlist");
  await expect(page.locator("#progressTime")).toHaveText(/^\(\d+s\)$/);
}

function finishMockExport(page) {
  return page.evaluate(() => window.__emitJobEvent({
    event: "job.completed",
    jobId: "export-1",
    jobType: "export",
    stage: "export",
    percent: 100,
    message: "USB: Export complete"
  }));
}

test("an analysis run during an export keeps its pause/cancel but leaves the export's progress bar and timer alone", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 1, pieceDelayMs: 50, pauseBeforeBpmKey: true });
  await page.goto("/");
  await startMockExport(page);

  await page.locator("#analyzeLibraryBtn").click();
  await page.waitForFunction(() => typeof window.__releaseBeforeBpmKey === "function");
  await expect(page.locator("#progressText")).toHaveText("USB: Exporting playlist");
  // The analysis keeps its own controls while the export holds the bar, and
  // pausing it leaves the export's clock running.
  await expect(page.locator("#progressPauseBtn")).toBeVisible();
  await expect(page.locator("#progressPauseBtn")).toHaveAttribute("title", "Pause analysis");
  await expect(page.locator("#progressCancelAnalysisBtn")).toBeVisible();
  await page.locator("#progressPauseBtn").click();
  await expect(page.locator("#progressPauseBtn")).toHaveAttribute("title", "Resume analysis");

  await page.evaluate(() => window.__releaseBeforeBpmKey());
  await expect(page.locator("#statusText")).toContainText("analyzed 1, failed 0");
  await expect(page.locator("#progressFooter")).toHaveClass(/active/);
  await expect(page.locator("#progressText")).toHaveText("USB: Exporting playlist");
  await expect(page.locator("#progressTime")).toHaveText(/^\(\d+s\)$/);
  await expect(page.locator("#progressPauseBtn")).toBeHidden();
  await expect(page.locator("#selectUsbFolderBtn")).toBeDisabled();

  await finishMockExport(page);
  await expect(page.locator("#progressFooter")).not.toHaveClass(/active/);
  await expect(page.locator("#selectUsbFolderBtn")).toBeEnabled();
});

test("the progress bar passes to an analysis still running when the export ends", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 1, pieceDelayMs: 50, pauseBeforeBpmKey: true });
  await page.goto("/");
  await startMockExport(page);

  await page.locator("#analyzeLibraryBtn").click();
  await page.waitForFunction(() => typeof window.__releaseBeforeBpmKey === "function");

  await finishMockExport(page);
  await expect(page.locator("#progressText")).toHaveText("Analyzing selected tracks");
  await expect(page.locator("#progressTime")).toHaveText(/^\(\d+s\)$/);
  await expect(page.locator("#progressPauseBtn")).toBeVisible();

  await page.evaluate(() => window.__releaseBeforeBpmKey());
  await expect(page.locator("#statusText")).toContainText("analyzed 1, failed 0");
  await expect(page.locator("#progressFooter")).not.toHaveClass(/active/);
});

test("every track in a large batch pulses as analyzing while a worker is on it", async ({ page }, testInfo) => {
  testInfo.setTimeout(testInfo.timeout * 3);
  const trackCount = 25;
  await installScanAnalysisMock(page, { trackCount, pieceDelayMs: 0, holdAfterTrackStart: true });
  await page.goto("/");

  await page.locator("#analyzeLibraryBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(trackCount);

  const seen = new Set();
  for (let i = 0; i < trackCount; i += 1) {
    // The mock holds each track right after its trackStarted event -- the
    // stretch where the real backend decodes and detects BPM/key without
    // emitting anything. The row must already pulse there, not only once
    // the first finished piece arrives.
    const heldId = await page.waitForFunction(
      (prev) => {
        const id = window.__heldTrack?.id;
        return id && !prev.includes(id) ? id : null;
      },
      [...seen]
    ).then((h) => h.jsonValue());
    seen.add(heldId);
    const row = page.locator(`#libraryTableBody .track-grid-row[data-track-id="${heldId}"]`);
    await expect(row).toHaveClass(/is-analyzing/);
    await expect(page.locator("#libraryTableBody .track-grid-row.is-analyzing")).toHaveCount(1);
    await page.evaluate(() => window.__heldTrack.release());
  }

  await expect(page.locator("#statusText")).toContainText(`analyzed ${trackCount}, failed 0`);
  await expect(page.locator("#libraryTableBody .track-grid-row.is-analyzing")).toHaveCount(0);
});

test("reanalyze clears the row's BPM and key until the fresh values arrive", async ({ page }) => {
  await installScanAnalysisMock(page, {
    trackCount: 1,
    pieceDelayMs: 0,
    holdAfterTrackStart: true,
    seedExistingWaveform: true,
    seedDuration: true,
    seedBpmKey: true
  });
  await page.goto("/");

  const row = page.locator("#libraryTableBody .track-grid-row").first();
  await expect(row.locator(".td-bpm")).toHaveText("99.00");
  await expect(row.locator(".td-key")).toHaveText("8B");

  await row.locator('[data-action="analyze-track"]').click();
  await page.waitForFunction(() => !!window.__heldTrack);
  await expect(row).toHaveClass(/is-analyzing/);
  await expect(row.locator(".td-bpm")).toHaveText("-");
  await expect(row.locator(".td-key")).toHaveText("-");

  await page.evaluate(() => window.__heldTrack.release());
  await expect(row).not.toHaveClass(/is-analyzing/);
  await expect(row.locator(".td-bpm .bpm-pill")).toHaveText("121.00");
  await expect(row.locator(".td-key .key-pill")).toHaveText("2A");
});

test("while a track is analyzed everything can still start an analysis, except that track's own button", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 2, pieceDelayMs: 0, holdAfterTrackStart: true });
  await page.goto("/");

  const rows = page.locator("#libraryTableBody .track-grid-row");
  const analyzeButtons = page.locator('#libraryTableBody [data-action="analyze-track"]');
  const reanalyze = page.locator('[data-selection-bar="library"] [data-selection-action="reanalyze"]');
  await rows.nth(1).locator(".td-select input").check();

  await analyzeButtons.first().click();
  await page.waitForFunction(() => !!window.__heldTrack);

  await expect(analyzeButtons.first()).toBeDisabled();
  await expect(analyzeButtons.nth(1)).toBeEnabled();
  await expect(reanalyze).toBeEnabled();
  await expect(page.locator("#analyzeLibraryBtn")).toBeEnabled();

  // The row drawn again during the analysis still has its button disabled.
  const firstTitle = await rows.first().locator(".track-title").textContent();
  await page.locator("#librarySearch").fill(firstTitle);
  await expect(rows).toHaveCount(1);
  await expect(analyzeButtons.first()).toBeDisabled();
  await page.locator("#librarySearch").fill("");
  await expect(rows).toHaveCount(2);
  await expect(analyzeButtons.first()).toBeDisabled();
  await expect(analyzeButtons.nth(1)).toBeEnabled();

  await page.evaluate(() => window.__heldTrack.release());
  await expect(analyzeButtons.first()).toBeEnabled();
});

test("a request for a track already in the job leaves that row analyzing", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 1, pieceDelayMs: 0, holdAfterTrackStart: true });
  await page.goto("/");

  const row = page.locator("#libraryTableBody .track-grid-row").first();
  await row.locator(".td-select input").check();
  await row.locator('[data-action="analyze-track"]').click();
  await page.waitForFunction(() => !!window.__heldTrack);
  await expect(row).toHaveClass(/is-analyzing/);

  await page.locator('[data-selection-bar="library"] [data-selection-action="reanalyze"]').click();
  await expect(page.locator("#statusText")).toContainText("Already analyzing 1 track");
  await expect(row).toHaveClass(/is-analyzing/);
  await expect(row.locator('[data-action="analyze-track"]')).toBeDisabled();
  await expect(page.locator("#progressPauseBtn")).toBeVisible();

  await page.evaluate(() => window.__heldTrack.release());
  await expect(row).not.toHaveClass(/is-analyzing/);
  await expect(row.locator('[data-action="analyze-track"]')).toBeEnabled();
  expect(await page.evaluate(() => window.__scanTestStats.analyzeNewTracksCalls)).toBe(2);
});

test("tracks waiting in the job have their own button disabled, and new work after a cancel brings the controls back", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 2, pieceDelayMs: 0, holdAfterTrackStart: true });
  await page.goto("/");

  const analyzeButtons = page.locator('#libraryTableBody [data-action="analyze-track"]');
  const waitingId = await page.locator("#libraryTableBody .track-grid-row").nth(1).getAttribute("data-track-id");
  await analyzeButtons.first().click();
  await page.waitForFunction(() => !!window.__heldTrack);
  const job = { jobId: "job-grows", jobType: "analysis", stage: "analyze_new_tracks" };
  const pause = page.locator("#progressPauseBtn");
  await pause.click();

  await page.evaluate(([run, id]) => window.__emitJobEvent({
    ...run, event: "job.tracks_waiting", waitingTrackIds: [id], waitingTooltip: "Waiting for its turn", message: "", percent: 0
  }), [job, waitingId]);
  const waitingButton = page.locator(`#libraryTableBody .track-grid-row[data-track-id="${waitingId}"] [data-action="analyze-track"]`);
  await expect(waitingButton).toBeDisabled();
  await expect(waitingButton).toHaveAttribute("data-tooltip", "Waiting for its turn");
  await expect(pause).toHaveAttribute("title", "Resume analysis");
  await expect(page.locator("#analyzeLibraryBtn")).toBeEnabled();

  await page.evaluate(([run, id]) => window.__emitJobEvent({
    ...run, event: "job.tracks_not_waiting", waitingTrackIds: [id], message: "", percent: 0
  }), [job, waitingId]);
  await expect(waitingButton).toBeEnabled();
  await expect(waitingButton).not.toHaveAttribute("data-tooltip", "Waiting for its turn");

  await page.locator("#progressCancelAnalysisBtn").click();
  await expect(pause).toBeHidden();
  await page.evaluate(([run, id]) => window.__emitJobEvent({
    ...run, event: "job.tracks_waiting", waitingTrackIds: [id], waitingTooltip: "Waiting for its turn", message: "", percent: 0
  }), [job, waitingId]);
  await expect(pause).toBeVisible();
  await expect(page.locator("#progressCancelAnalysisBtn")).toBeVisible();
});

test("the final message is the job's, with its warning count as the Event Log link", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 1, pieceDelayMs: 0, holdAfterTrackStart: true });
  await page.goto("/");
  await page.waitForFunction(() => window.__hasJobListener?.());
  await page.evaluate(() => {
    const job = { jobId: "job-end", jobType: "analysis", stage: "analyze_new_tracks" };
    window.__emitJobEvent({ ...job, event: "job.started", percent: 0, message: "Analyzing selected tracks" });
    // The warning reaches the Event Log as the backend logs it, not from the event.
    window.__emitBackendLog({ level: "error", source: "analysis", code: "analysis.track-failed", message: "/music/bad.mp3: decode failed" });
    window.__emitJobEvent({
      ...job,
      event: "job.completed",
      percent: 100,
      message: "Analysis finished: analyzed 3, failed 1 | (1 warning)",
      warningCount: 1,
    });
  });
  await expect(page.locator("#statusText")).toContainText("Analysis finished: analyzed 3, failed 1");
  await expect(page.locator("#statusText a, #statusText button")).toContainText("(1 warning)");
  await page.locator("#eventLogBtn").click();
  await expect(page.locator("#eventLogList")).toContainText("/music/bad.mp3: decode failed");
});

test("a first analysis keeps the row's imported BPM and key while it runs", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 1, pieceDelayMs: 0, holdAfterTrackStart: true, seedBpmKey: true });
  await page.goto("/");

  // The scan analyzes the unanalyzed track.
  await page.locator("#analyzeLibraryBtn").click();
  const row = page.locator("#libraryTableBody .track-grid-row").first();
  await page.waitForFunction(() => !!window.__heldTrack);
  await expect(row).toHaveClass(/is-analyzing/);
  await expect(row.locator(".td-bpm")).toHaveText("99.00");
  await expect(row.locator(".td-key")).toHaveText("8B");
  await page.evaluate(() => window.__heldTrack.release());
  await expect(row).not.toHaveClass(/is-analyzing/);
});

test("scan progressively changes action buttons to Reanalyze", async ({ page }, testInfo) => {
  // Simulates a full 40-track/4-piece batch (160 progress events + row re-renders); under
  // CPU contention from the rest of the suite running in parallel this can outrun the
  // default 5s expect timeout even though the app itself isn't slow — triple it here.
  testInfo.setTimeout(testInfo.timeout * 3);
  await installScanAnalysisMock(page, { trackCount: 40, pieceDelayMs: 20 });
  // Tall enough that the table renders all 40 rows (it only renders the
  // rows around the viewport).
  await page.setViewportSize({ width: 1280, height: 2400 });
  await page.goto("/");

  await page.locator("#analyzeLibraryBtn").click();
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
  await page.locator("#analyzeLibraryBtn").click();
  await expect(page.locator("#statusText")).toContainText("analyzed 1, failed 0");
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats?.bpmRangeSeen)).toEqual({ min: 88, max: 175 });
});

test("a scan's own warnings (a file found in two folders) reach the event log", async ({ page }) => {
  const message = "Same file in two places: /music/B/Song.mp3 is identical to /music/A/Song.mp3. The library keeps one; delete the extra copy.";
  await installScanAnalysisMock(page, {
    trackCount: 1,
    pieceDelayMs: 20,
    scanWarnings: [{ level: "warn", source: "library-scan", code: "scan.duplicate-file", message }]
  });
  await page.goto("/");
  await page.locator("#analyzeLibraryBtn").click();
  await expect(page.locator("#statusText")).toContainText("analyzed 1, failed 0");
  await page.locator("#eventLogBtn").click();
  await expect(page.locator("#eventLogList")).toContainText(message);
});

test("analysis populates BPM and key cells for all tracks", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 40, pieceDelayMs: 10 });
  await page.setViewportSize({ width: 1280, height: 2400 });
  await page.goto("/");

  await page.locator("#analyzeLibraryBtn").click();
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

  await page.locator("#analyzeLibraryBtn").click();
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
  await installScanAnalysisMock(page, { trackCount: 5, pieceDelayMs: 60 });
  await page.goto("/");

  await page.locator("#analyzeLibraryBtn").click();
  await expect(page.locator("#progressPauseBtn")).toBeVisible();
  await expect(page.locator("#progressCancelAnalysisBtn")).toBeVisible();

  // Click pause in the same in-page poll that sees the first track become
  // ready, as a direct DOM click (not Playwright's locator.click()) -- under
  // a loaded parallel run, actionability retries against the repainting
  // table can add enough real time for the whole batch to finish first.
  await page.waitForFunction(() => {
    if (document.querySelectorAll("#libraryTableBody .bpm-pill").length < 1) return false;
    document.getElementById("progressPauseBtn").click();
    return true;
  });
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats?.analysisPaused)).toBe(true);

  // A track already in flight when pause was clicked is allowed to finish
  // (same as the backend's own worker loop), so wait out more than one full
  // track (4 pieces x 60ms) before establishing the "paused" baseline count.
  // "(paused)" isn't a usable settle signal here: it can briefly show in the
  // gap between one track's ready event and the next track's first piece.
  await page.waitForTimeout(400);
  const pausedCount = await page.locator("#libraryTableBody .bpm-pill").count();
  expect(pausedCount).toBeLessThan(5);
  // Give the batch plenty of time to have picked up further tracks if pause
  // wasn't actually blocking new work.
  await page.waitForTimeout(400);
  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(pausedCount);
  // Only once the batch has genuinely stopped (no track still in flight)
  // does the elapsed timer freeze on "(paused)".
  await expect(page.locator("#progressTime")).toHaveText("(paused)");

  await page.locator("#progressPauseBtn").click();
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats?.analysisPaused)).toBe(false);
  await expect(page.locator("#progressTime")).not.toHaveText("(paused)");

  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(5);
  await expect(page.locator("#statusText")).toContainText("analyzed 5, failed 0");
  await expect(page.locator("#progressPauseBtn")).toBeHidden();
  await expect(page.locator("#progressCancelAnalysisBtn")).toBeHidden();
});

test("event log entries go to the session log file, which the Open log file button opens", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 2, pieceDelayMs: 10 });
  await page.goto("/");

  await page.waitForFunction(() => window.__hasJobListener?.());
  // A track's failure is an entry JS makes from the job's event.
  await page.evaluate(() => {
    const job = { jobId: "job-log", jobType: "analysis", stage: "analyze_new_tracks" };
    window.__emitJobEvent({ ...job, event: "job.started", percent: 0, message: "Analyzing selected tracks" });
    window.__emitJobEvent({
      ...job,
      event: "job.progress",
      percent: 50,
      current: 1,
      total: 2,
      trackId: "bad-1",
      trackTitle: "Bad Track",
      failed: true,
      errorMessage: "decode failed",
      message: "Analyzing 1/2",
    });
  });

  await page.locator("#eventLogBtn").click();
  await expect(page.locator("#eventLogList")).toContainText("Track analysis failed: Bad Track (bad-1) - decode failed");
  await expect
    .poll(async () => page.evaluate(() => window.__scanTestStats.logFileLines))
    .toContainEqual(expect.stringContaining("Track analysis failed: Bad Track (bad-1) - decode failed"));

  await page.locator("#eventLogOpenFileBtn").click();
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats.openLogFileCalls)).toBe(1);
});

test("pause clicked mid-track keeps the elapsed timer counting until that track actually finishes", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 3, pieceDelayMs: 300 });
  await page.goto("/");

  await page.locator("#analyzeLibraryBtn").click();
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
  await expect(page.locator("#progressTime")).not.toHaveText("(paused)");

  // Once that track finishes (its bpm/key piece lands), nothing else starts
  // because we're paused, so now it has genuinely stopped.
  await expect(page.locator("#progressTime")).toHaveText("(paused)");
  const settledCount = await page.locator("#libraryTableBody .bpm-pill").count();
  expect(settledCount).toBeLessThan(3);

  // Stays stopped -- no further tracks pick up.
  await page.waitForTimeout(300);
  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(settledCount);
});

test("pause clicked between tracks unfreezes the timer if the next track was already picked up", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 3, pieceDelayMs: 300 });
  await page.goto("/");

  await page.locator("#analyzeLibraryBtn").click();
  await expect(page.locator("#progressPauseBtn")).toBeVisible();

  // Click pause the moment track 1 is ready. By then the mock (like a real
  // backend worker) is already past its pause check and working on track 2,
  // but track 2's first piece hasn't been reported yet -- so with no rows
  // analyzing, the UI freezes on "(paused)" right away.
  await page.waitForFunction(() => {
    if ((window.__scanTestStats?.pieceEventsByPiece?.bpm_key || 0) < 1) return false;
    document.getElementById("progressPauseBtn").click();
    return true;
  });

  // Track 2's first piece shows it was in flight after all, so the timer
  // must go back to counting instead of staying frozen.
  await page.waitForFunction(() => (window.__scanTestStats?.pieceEventsByPiece?.artwork || 0) >= 2);
  await expect(page.locator("#progressTime")).not.toHaveText("(paused)");

  // Once track 2 finishes, nothing else starts, so it's genuinely stopped.
  await expect(page.locator("#progressTime")).toHaveText("(paused)");
  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(2);
  await page.waitForTimeout(400);
  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(2);
});

test("cancel button stops the batch early and reports how many tracks completed", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 5, pieceDelayMs: 40 });
  await page.goto("/");

  await page.locator("#analyzeLibraryBtn").click();
  await expect(page.locator("#progressCancelAnalysisBtn")).toBeVisible();

  await page.waitForFunction(() => {
    return document.querySelectorAll("#libraryTableBody .bpm-pill").length >= 1;
  });
  await page.locator("#progressCancelAnalysisBtn").click();

  await expect(page.locator("#progressPauseBtn")).toBeHidden();
  await expect(page.locator("#progressCancelAnalysisBtn")).toBeHidden();
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats?.analysisCancelled)).toBe(true);

  // The batch must not hang waiting on tracks that will never come; the
  // status line should settle on "Analysis finished: analyzed N, failed 0"
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

  await page.locator("#analyzeLibraryBtn").click();
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

  await page.locator("#analyzeLibraryBtn").click();
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

  await page.locator("#analyzeLibraryBtn").click();
  await expect(page.locator("#statusText")).toContainText("analyzed 8, failed 0");
  const after = await page.locator("#libraryTableBody .track-grid-row .track-title").allTextContents();

  expect(after).toEqual(before);
});

// The library table only renders the rows around the viewport: scroll a row
// into view (its page loads when scrolling settles) and locate it by its
// position in the list.
async function scrollLibraryToRow(page, index) {
  await expect(page.locator("#libraryTableBody .track-grid-row[data-track-id]").first()).toBeVisible();
  await page.evaluate((index) => {
    const wrap = document.querySelector("#libraryTableWrap");
    const row = document.querySelector("#libraryTableBody .track-grid-row");
    wrap.scrollTop = index * row.getBoundingClientRect().height;
  }, index);
  await expect(libraryRowAt(page, index)).toBeVisible();
}

function libraryRowAt(page, index) {
  return page.locator(`#libraryTableBody .track-grid-row[data-track-id][data-track-index="${index}"]`);
}

test("analyze on auto-loaded non-materialized track resolves local id", async ({ page }) => {
  await installPagedMaterializeAnalyzeMock(page, { trackCount: 260, pageSize: 200, pieceDelayMs: 120 });
  await page.goto("/");

  await scrollLibraryToRow(page, 230);
  const targetRow = libraryRowAt(page, 230);
  await targetRow.locator('[data-action="analyze-track"]').click();

  await expect(page.locator("#statusText")).not.toContainText("Track is not in the local library yet");
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

  await scrollLibraryToRow(page, 230);
  const targetRow = libraryRowAt(page, 230);
  await targetRow.locator('[data-action="analyze-track"]').click();

  await expect(page.locator("#statusText")).toContainText(
    "Track is not in the local library yet. Run Analyze Library first."
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

  await scrollLibraryToRow(page, 230);

  const beforeStats = await page.evaluate(() => ({
    browse: Number(window.__pagedAnalyzeStats?.browseQueryCount || 0)
  }));

  const targetRow = libraryRowAt(page, 230);
  const before = await targetRow.evaluate((row) => ({
    trackId: String(row.getAttribute("data-track-id") || ""),
    hasBpmPill: !!row.querySelector(".bpm-pill")
  }));
  expect(before.trackId.startsWith("ml-")).toBeTruthy();
  expect(before.hasBpmPill).toBeFalsy();

  await targetRow.locator('[data-action="analyze-track"]').click();

  await expect(page.locator("#statusText")).toContainText("Analysis finished: analyzed 1, failed 0");

  const afterTrackId = await libraryRowAt(page, 230)
    .evaluate((row) => String(row.getAttribute("data-track-id") || ""));
  expect(afterTrackId).toBe(before.trackId);

  const afterStats = await page.evaluate(() => ({
    browse: Number(window.__pagedAnalyzeStats?.browseQueryCount || 0),
    analyzeCalls: Number(window.__pagedAnalyzeStats?.analyzeCalls || 0)
  }));
  expect(afterStats.analyzeCalls).toBeGreaterThan(0);
  expect(afterStats.browse).toBeGreaterThanOrEqual(beforeStats.browse);
});

// A single-track library with analyze_new_tracks's response fully under the
// test's control (no per-piece job:event choreography needed -- that's not
// what these two tests are about) for exercising status-line formatting of
// unusual `analyze_new_tracks` response shapes: a partial-failure count, and
// a structured WarningEntry object (as opposed to a plain string) that must
// still render its `message`, not "[object Object]".
