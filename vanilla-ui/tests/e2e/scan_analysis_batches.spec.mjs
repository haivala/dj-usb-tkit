import { test, expect } from "@playwright/test";

// NOTE: analysis is now always dispatched as a single `analyze_new_tracks`
// batch call per analyzeTrackIds() invocation (see components/library/actions.mjs).
// The old per-piece `analyze_track_piece` client-side dispatch loop no longer
// exists. The real backend still reports progressive per-track/per-piece
// updates during that single batch call via `job:event` events
// (stage: "analyze_new_tracks", trackReady: false for each of the 4 pieces,
// then a final trackReady: true event per track once done) — see
// backend/src/service/analysis.rs (analyze_local_track_with_updates,
// build_partial_progress, build_done_progress_success) and
// backend/src/tauri_commands.rs (analyze_new_tracks, emit_job_event_with_track).
// These mocks simulate that same event sequence so the frontend's real
// job_manager.mjs handleJobEvent()/applyRealtimeAnalyzedTrackUpdate() codepath
// is exercised exactly as it is in production, instead of re-implementing a
// separate (now-nonexistent) per-piece invoke mechanism.

function installScanAnalysisMock(page, opts = {}) {
  const trackCount = Number(opts?.trackCount || 40);
  const pieceDelayMs = Number(opts?.pieceDelayMs || 60);
  const workers = Number(opts?.workers || 6);
  const seedExistingWaveform = !!opts?.seedExistingWaveform;
  const seedDuration = !!opts?.seedDuration;
  const seedArtwork = !!opts?.seedArtwork;
  const variedArtists = !!opts?.variedArtists;
  const analysisBpmRange = String(opts?.analysisBpmRange || "70-180");
  return page.addInitScript(({
    trackCount,
    pieceDelayMs,
    workers,
    seedExistingWaveform,
    seedDuration,
    seedArtwork,
    variedArtists,
    analysisBpmRange
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
        track.durationMs = 180000;
        pieceEventsByPiece.duration += 1;
        emitPartial({ durationMs: track.durationMs });

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
        track.bpm = 120 + (Number(id.replace(/\D+/g, "")) % 4);
        track.key = `${(Number(id.replace(/\D+/g, "")) % 12) + 1}A`;
        track.updatedAt = "2026-03-04T00:00:00Z";
        pieceEventsByPiece.bpm_key += 1;
        emitPartial({ bpm: track.bpm, bpmAnalyzer: "mock-analyzer", key: track.key });

        analyzed += 1;
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
          waveformPreview: track.waveformPreview
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

      return { jobId, analyzed, failed, warnings: [] };
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
            return { ok: true, data: { total: filtered.length, items: filtered } };
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
      }
    };
  }, { trackCount, pieceDelayMs, workers, seedExistingWaveform, seedDuration, seedArtwork, variedArtists, analysisBpmRange });
}

function installPagedMaterializeAnalyzeMock(page, opts = {}) {
  const trackCount = Number(opts?.trackCount || 260);
  const pageSize = Number(opts?.pageSize || 200);
  const pieceDelayMs = Number(opts?.pieceDelayMs || 80);
  const materializedIds = !!opts?.materializedIds;
  return page.addInitScript(({ trackCount, pageSize, pieceDelayMs, materializedIds }) => {
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

    const listPage = (cursorValue, query = "") => {
      const q = String(query || "").toLowerCase().trim();
      const filtered = q
        ? tracks.filter((t) => `${t.title} ${t.artist} ${t.album}`.toLowerCase().includes(q))
        : tracks.slice();
      const offset = Number(String(cursorValue || "0")) || 0;
      const items = filtered.slice(offset, offset + pageSize).map(toTrackDto);
      const nextOffset = offset + items.length;
      const hasMore = nextOffset < filtered.length;
      return {
        total: filtered.length,
        items,
        next_cursor: hasMore ? String(nextOffset) : null,
        has_more: hasMore
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
          waveformPreview: row.waveformPreview
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

      return { jobId, analyzed, failed, warnings: [] };
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
          if (command === "materialize_source_track") {
            materializeCalls += 1;
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
  }, { trackCount, pageSize, pieceDelayMs, materializedIds });
}

test("scan applies per-piece row updates before track-ready status", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 1, pieceDelayMs: 250 });
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

test("scan progressively changes action buttons to Reanalyze", async ({ page }) => {
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
  ).toHaveCount(40);
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
  ).toHaveCount(40);

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

test("scan recomputes waveform piece even when one already exists", async ({ page }) => {
  // The old frontend-driven per-piece loop used to compute "missing pieces"
  // client-side and skip already-present ones before calling
  // analyze_track_piece. That resolveMissingAnalysisPieces logic was deleted
  // along with the per-piece dispatch loop. The batch analyze_new_tracks
  // command (backend/src/service/analysis.rs::analyze_local_track_with_updates)
  // has no "skip if already analyzed" branching - it always recomputes
  // duration, artwork, waveform, and bpm/key for every requested track. This
  // test now documents that real, current behavior instead of the old
  // (no-longer-true) skip premise.
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
