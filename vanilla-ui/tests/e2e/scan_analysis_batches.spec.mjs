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

// `pageSize`: the library pages that many rows at a time. `unmaterialized`:
// the rows are files the library hasn't given an id yet (their id is their
// path) until `resolve_track_identity` materializes them (`materializeFails`:
// it can't).
function installScanAnalysisMock(page, opts = {}) {
  return page.addInitScript((opts) => {
    const backend = window.__e2eBackend;
    const trackCount = Math.max(1, Number(opts.trackCount || 40));
    const pieceDelayMs = Number(opts.pieceDelayMs ?? 60);
    window.__e2eSourceFolders = ["/music"];
    window.localStorage.setItem("djusbtkit.analysisBpmRange", String(opts.analysisBpmRange || "70-180"));

    let analyzeNewTracksCalls = 0;
    const claimedTrackIds = new Set();
    const pieceEventsByPiece = { duration: 0, artwork: 0, waveform: 0, bpm_key: 0 };
    let bpmRangeSeen = null;
    let analysisPaused = false;
    let analysisCancelled = false;
    const logFileLines = [];
    let openLogFileCalls = 0;
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

    const tracks = Array.from({ length: trackCount }, (_, index) => {
      const n = String(index + 1).padStart(trackCount > 99 ? 4 : 2, "0");
      const filePath = `/music/Batch Artist - Track ${n}.wav`;
      const id = opts.unmaterialized ? filePath : `track-${n}`;
      return {
        id,
        localTrackId: opts.unmaterialized ? null : id,
        title: `Track ${n}`,
        artist: opts.variedArtists ? (index % 2 === 0 ? "B Artist" : "A Artist") : "Batch Artist",
        album: "Batch Album",
        bpm: opts.seedBpmKey ? 99 : null,
        key: opts.seedBpmKey ? "8B" : null,
        filePath,
        fileSizeBytes: 1000 + index,
        durationMs: opts.seedDuration ? 180000 : null,
        artworkPath: opts.seedArtwork ? `/tmp/${id}.jpg` : null,
        waveformPeaksPath: opts.seedExistingWaveform ? `/tmp/${id}.DAT` : null,
        waveformPreview: [],
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      };
    });
    const findRow = (id) => tracks.find((t) => t.localTrackId === id || t.id === id);

    // Mirrors track_has_core_analysis_for_source_status
    // (backend/src/service/source_roots.rs): a track counts toward the library
    // duration total only once it has all three of bpm/waveform/duration.
    const isCountable = (track) => Number.isFinite(track.bpm) && track.bpm > 0
      && !!track.waveformPeaksPath
      && Number.isFinite(track.durationMs) && track.durationMs > 0;
    const countedMs = (list) => list.filter(isCountable).reduce((sum, t) => sum + t.durationMs, 0);
    // A row as the backend sends it: its library id, and analysisReady from its fields.
    const asRow = ({ localTrackId, ...row }) => ({ ...row, id: localTrackId || row.id, analysisReady: isCountable(row) });

    // Simulates the real backend's analyze_new_tracks_with_progress worker:
    // for each requested track, a trackStarted event as a worker picks it up,
    // one trackReady:false event per piece (artwork, waveform, duration,
    // bpm_key -- backend/src/service/analysis.rs's TrackPartialUpdate), then one
    // trackReady:true event carrying all fields (build_done_progress_success).
    // The command itself has returned by then, as the real one does;
    // `job.completed` carries the result.
    const runAnalyzeNewTracks = async (jobId, ids) => {
      const total = Math.max(1, ids.length);
      const job = { jobId, jobType: "analysis", stage: "analyze_new_tracks", total };
      // Every new batch starts fresh, as the backend's pause/cancel flags do.
      analysisPaused = false;
      analysisCancelled = false;
      // Live library-duration-total baseline: every *other* track that's
      // already countable, so a re-analyzed track isn't double-counted.
      const batch = new Set(ids.map(findRow));
      const others = tracks.filter((t) => !batch.has(t));
      let libraryTotalDurationMs = countedMs(others);
      let libraryDurationKnownCount = others.filter(isCountable).length;
      window.__emitJobEvent({ ...job, event: "job.started", current: 0, percent: 0, message: "Analyzing selected tracks" });

      let analyzed = 0;
      let failed = 0;
      const finish = (current) => window.__emitJobEvent({
        ...job, event: "job.completed", current, percent: Math.round((current / total) * 100),
        message: `Analysis finished: analyzed ${analyzed}, failed ${failed}`
      });

      for (let i = 0; i < ids.length; i += 1) {
        // A pause blocks picking up the next track; a cancel ends the batch.
        while (analysisPaused && !analysisCancelled) await sleep(15);
        if (analysisCancelled) return finish(i);

        const id = ids[i];
        const track = findRow(id);
        if (!track) {
          failed += 1;
          continue;
        }
        const progress = {
          ...job, event: "job.progress", trackId: id, trackTitle: track.title, filePath: track.filePath,
          message: `Analyzing ${i + 1}/${ids.length}: ${track.title}`
        };
        const emitPartial = (extra) => window.__emitJobEvent({
          ...progress, current: i, percent: Math.round((i / total) * 100), trackReady: false, ...extra
        });

        // A worker picked the track up (build_started_progress): a reanalysis
        // when the track already has a waveform.
        emitPartial({ trackStarted: true, reanalysis: !!track.waveformPeaksPath });
        if (opts.holdAfterTrackStart) {
          // Stands in for the decode + BPM/key detection, where the real
          // backend emits nothing: held until the test releases it.
          await new Promise((resolve) => { window.__heldTrack = { id, release: resolve }; });
          window.__heldTrack = null;
        }

        await sleep(pieceDelayMs);
        track.artworkPath = `/tmp/${id}.jpg`;
        pieceEventsByPiece.artwork += 1;
        emitPartial({ artworkPath: track.artworkPath });

        await sleep(pieceDelayMs);
        track.waveformPeaksPath = `/tmp/${id}.DAT`;
        track.waveformPreview = [8, 20, 42, 65, 30, 55];
        pieceEventsByPiece.waveform += 1;
        emitPartial({ waveformPeaksPath: track.waveformPeaksPath, waveformPreview: track.waveformPreview });

        await sleep(pieceDelayMs);
        track.durationMs = 180000;
        pieceEventsByPiece.duration += 1;
        emitPartial({ durationMs: track.durationMs });

        if (opts.pauseBeforeBpmKey) {
          // Held until the test has asserted the track is still not ready
          // (window.__releaseBeforeBpmKey), so that can't lose a race.
          await new Promise((resolve) => { window.__releaseBeforeBpmKey = resolve; });
        }

        await sleep(pieceDelayMs);
        const number = Number(String(track.title).replace(/\D+/g, ""));
        track.bpm = 120 + (number % 4);
        track.key = `${(number % 12) + 1}A`;
        track.updatedAt = "2026-03-04T00:00:00Z";
        pieceEventsByPiece.bpm_key += 1;
        emitPartial({ bpm: track.bpm, bpmSourceText: "Analyzed with: mock-analyzer", key: track.key });

        analyzed += 1;
        if (isCountable(track)) {
          libraryTotalDurationMs += track.durationMs;
          libraryDurationKnownCount += 1;
        }
        window.__emitJobEvent({
          ...progress,
          current: i + 1,
          percent: Math.round(((i + 1) / total) * 100),
          trackReady: true,
          failed: false,
          analysisReady: isCountable(track),
          bpm: track.bpm,
          bpmAnalyzer: "mock-analyzer",
          bpmSourceText: "Analyzed with: mock-analyzer",
          key: track.key,
          durationMs: track.durationMs,
          artworkPath: track.artworkPath,
          waveformPeaksPath: track.waveformPeaksPath,
          waveformPreview: track.waveformPreview,
          libraryDurationText: backend.totalTimeText(
            libraryTotalDurationMs,
            Math.max(0, tracks.length - libraryDurationKnownCount)
          ),
          // The track's row as saved, whole, as the backend sends it.
          track: asRow(track),
        });
      }
      finish(ids.length);
    };

    const scanWarnings = opts.scanWarnings || [];
    backend.install((command, r, payload) => {
      switch (command) {
        case "append_frontend_log":
          logFileLines.push(`[${payload?.level}] ${payload?.message}`);
          return null;
        case "open_frontend_log":
          openLogFileCalls += 1;
          return null;
        case "scan_library":
          // The backend logs each warning as it makes it (`logging::log`).
          for (const warning of scanWarnings) window.__emitBackendLog(warning);
          return backend.ok({ jobId: "job-scan-mock", indexed: tracks.length, updated: 0, removed: 0, warnings: scanWarnings });
        case "browse_source_files":
          return backend.ok(backend.listPage(tracks, r, { pageSize: opts.pageSize, countable: isCountable, map: asRow }));
        case "resolve_track_identity": {
          if (opts.materializeFails) return backend.fail("materialize failed", "INTERNAL_ERROR");
          const row = tracks.find((t) => (r.filePath && t.filePath === r.filePath) || (r.trackId && findRow(String(r.trackId)) === t));
          if (!row) return backend.ok({ trackId: null, resolvedBy: "none", materialized: false });
          if (row.localTrackId) return backend.ok({ trackId: row.localTrackId, resolvedBy: "self", materialized: false });
          row.localTrackId = `ml-${row.title.replace(/\D+/g, "")}`;
          return backend.ok({ trackId: row.localTrackId, resolvedBy: "materialized", materialized: true });
        }
        case "analyze_new_tracks": {
          analyzeNewTracksCalls += 1;
          let ids = (r.trackIds || []).map(String);
          // Backend-scoped request: the caller sends no ids, the backend picks
          // the tracks that still need core analysis (here: the whole library).
          if (!ids.length && (r.scopeToLibraryFilter || r.playlistId)) {
            ids = tracks.filter((t) => !isCountable(t)).map((t) => String(t.localTrackId || t.id));
          }
          // The backend parses the `bpmRange` string (service::analysis).
          const [min, max] = String(r.bpmRange || "").split("-").map((n) => Number(n.trim()));
          bpmRangeSeen = { min, max };
          // A track already in the job is skipped, as the backend's coordinator does.
          const alreadyAnalyzing = ids.filter((id) => claimedTrackIds.has(id));
          ids = ids.filter((id) => !claimedTrackIds.has(id));
          ids.forEach((id) => claimedTrackIds.add(id));
          const jobId = `job-analysis-mock-${analyzeNewTracksCalls}`;
          if (ids.length) runAnalyzeNewTracks(jobId, ids).finally(() => ids.forEach((id) => claimedTrackIds.delete(id)));
          return backend.ok({
            jobId,
            queued: ids.length,
            skipped: alreadyAnalyzing.length,
            message: ids.length
              ? `Analyzing ${ids.length} tracks`
              : alreadyAnalyzing.length ? "Already analyzing 1 track" : "No tracks need analysis",
          });
        }
        case "set_analysis_paused":
          analysisPaused = !!r.paused;
          return backend.ok({ paused: analysisPaused });
        case "cancel_analysis":
          analysisCancelled = true;
          return backend.ok(null);
        default: return undefined;
      }
    }, { tauriRuntime: true });

    const callCount = (command) => window.__calls.filter((c) => c.command === command).length;
    window.__scanTestStats = {
      get analyzeNewTracksCalls() { return analyzeNewTracksCalls; },
      get browseCalls() { return callCount("browse_source_files"); },
      get materializeCalls() { return callCount("resolve_track_identity"); },
      get pieceEventsByPiece() { return pieceEventsByPiece; },
      get bpmRangeSeen() { return bpmRangeSeen; },
      get analysisPaused() { return analysisPaused; },
      get analysisCancelled() { return analysisCancelled; },
      get logFileLines() { return logFileLines; },
      get openLogFileCalls() { return openLogFileCalls; }
    };
  }, opts);
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

test("an analysis run during an export keeps its pause/cancel but leaves the export's progress bar and timer alone; the progress bar passes to an analysis still running when the export ends", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 1, pieceDelayMs: 50, pauseBeforeBpmKey: true });
  await page.goto("/");
  {
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
  }

  await page.reload();
  {
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
  }
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

test("while a track is analyzed everything can still start an analysis, except that track's own button; tracks waiting in the job have their own button disabled, and new work after a cancel brings the controls back", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 2, pieceDelayMs: 0, holdAfterTrackStart: true });
  await page.goto("/");
  {
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
  }

  await page.reload();
  {
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
  }
});

test("a request for a track already in the job leaves that row analyzing; the final message is the job's, with its warning count as the Event Log link", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 1, pieceDelayMs: 0, holdAfterTrackStart: true });
  await page.goto("/");
  {
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
  }

  await page.reload();
  {
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
        message: "Analysis finished: analyzed 3, failed 1",
        warningText: "(1 warning)",
      });
    });
    await expect(page.locator("#statusText")).toContainText("Analysis finished: analyzed 3, failed 1");
    await expect(page.locator("#statusText a, #statusText button")).toContainText("(1 warning)");
    await page.locator("#eventLogBtn").click();
    await expect(page.locator("#eventLogList")).toContainText("/music/bad.mp3: decode failed");
  }
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
  await installScanAnalysisMock(page, { trackCount: 2, pieceDelayMs: 120 });
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
  // A track's failure is an entry JS makes from the job event's `failedText`.
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
      // As the backend words it.
      failedText: "Track analysis failed: Bad Track (bad-1) - decode failed",
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

test("pause clicked mid-track keeps the elapsed timer counting until that track actually finishes; pause clicked between tracks unfreezes the timer if the next track was already picked up", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 3, pieceDelayMs: 300 });
  await page.goto("/");
  {
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
  }

  await page.reload();
  {
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
  }
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
  await installScanAnalysisMock(page, { trackCount: 260, pageSize: 200, pieceDelayMs: 120, unmaterialized: true });
  await page.goto("/");

  await scrollLibraryToRow(page, 230);
  const targetRow = libraryRowAt(page, 230);
  await targetRow.locator('[data-action="analyze-track"]').click();

  await expect(page.locator("#statusText")).not.toContainText("Track is not in the local library yet");
  await expect.poll(async () => {
    return page.evaluate(() => Number(window.__scanTestStats?.materializeCalls || 0));
  }).toBeGreaterThan(0);
  await expect.poll(async () => {
    return page.evaluate(() => Number(window.__scanTestStats?.analyzeNewTracksCalls || 0));
  }).toBeGreaterThan(0);
});

test("analyze reports the not-in-local-library message when materializing the track fails", async ({ page }) => {
  await installScanAnalysisMock(page, {
    trackCount: 260, pageSize: 200, pieceDelayMs: 120, unmaterialized: true, materializeFails: true
  });
  await page.goto("/");

  await scrollLibraryToRow(page, 230);
  const targetRow = libraryRowAt(page, 230);
  await targetRow.locator('[data-action="analyze-track"]').click();

  await expect(page.locator("#statusText")).toContainText(
    "Track is not in the local library yet. Run Analyze Library first."
  );
  const analyzeCalls = await page.evaluate(() => Number(window.__scanTestStats?.analyzeNewTracksCalls || 0));
  expect(analyzeCalls).toBe(0);
});

test("analyze on auto-loaded track updates row in place without full reload", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 260, pageSize: 200, pieceDelayMs: 120 });
  await page.goto("/");

  await scrollLibraryToRow(page, 230);

  const beforeStats = await page.evaluate(() => ({
    browse: Number(window.__scanTestStats?.browseCalls || 0)
  }));

  const targetRow = libraryRowAt(page, 230);
  const before = await targetRow.evaluate((row) => ({
    trackId: String(row.getAttribute("data-track-id") || ""),
    hasBpmPill: !!row.querySelector(".bpm-pill")
  }));
  expect(before.trackId.startsWith("track-")).toBeTruthy();
  expect(before.hasBpmPill).toBeFalsy();

  await targetRow.locator('[data-action="analyze-track"]').click();

  await expect(page.locator("#statusText")).toContainText("Analysis finished: analyzed 1, failed 0");

  const afterTrackId = await libraryRowAt(page, 230)
    .evaluate((row) => String(row.getAttribute("data-track-id") || ""));
  expect(afterTrackId).toBe(before.trackId);

  const afterStats = await page.evaluate(() => ({
    browse: Number(window.__scanTestStats?.browseCalls || 0),
    analyzeCalls: Number(window.__scanTestStats?.analyzeNewTracksCalls || 0)
  }));
  expect(afterStats.analyzeCalls).toBeGreaterThan(0);
  expect(afterStats.browse).toBeGreaterThanOrEqual(beforeStats.browse);
});
