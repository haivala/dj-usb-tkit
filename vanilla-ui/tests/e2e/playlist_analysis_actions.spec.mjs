import { test, expect } from "./coverage-fixture.mjs";

// The app-playlist track list is server-paginated/searched/sorted via
// `get_playlist_tracks` (shared TrackListController), and drag-reorder + the
// sort-commit are single-move / sort-mode `reorder_playlist_tracks` calls.
// These helpers let every mock below implement that contract in one line.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    window.__playlistTracksPage = (all, request) => {
      let rows = (all || []).slice();
      const q = String(request.query || "").trim().toLowerCase();
      if (q) rows = rows.filter((t) => `${t.title} ${t.artist} ${t.album || ""}`.toLowerCase().includes(q));
      if (request.sortBy) {
        const m = request.sortDir === "desc" ? -1 : 1;
        const k = request.sortBy;
        rows = rows.slice().sort((a, b) => {
          if (k === "bpm" || k === "durationMs") return m * ((Number(a[k]) || 0) - (Number(b[k]) || 0));
          const av = k === "artist" ? `${a.artist} ${a.title}` : String(a[k] ?? "");
          const bv = k === "artist" ? `${b.artist} ${b.title}` : String(b[k] ?? "");
          return av < bv ? -m : av > bv ? m : 0;
        });
      }
      const off = Number(request.cursor || 0);
      const lim = Number(request.limit || 0) || rows.length;
      const slice = rows.slice(off, off + lim);
      const nextOff = off + slice.length;
      return {
        playlistId: request.playlistId,
        items: slice,
        total: rows.length,
        nextCursor: nextOff < rows.length ? String(nextOff) : null,
        hasMore: nextOff < rows.length,
        totalDurationMs: rows.reduce((s, t) => s + (t.durationMs > 0 ? t.durationMs : 0), 0),
        durationKnownCount: rows.filter((t) => t.durationMs > 0).length,
        unanalyzedCount: rows.filter((t) => !t.analysisReady).length,
      };
    };
    window.__applyReorder = (cur, request) => {
      const list = (cur || []).slice();
      const byId = new Map(list.map((t) => [t.id, t]));
      let ids;
      if (Array.isArray(request.orderedTrackIds) && request.orderedTrackIds.length) {
        ids = request.orderedTrackIds;
      } else if (request.sortBy) {
        const m = request.sortDir === "desc" ? -1 : 1;
        const k = request.sortBy;
        ids = list.sort((a, b) => {
          const av = k === "artist" ? `${a.artist} ${a.title}` : String(a[k] ?? "");
          const bv = k === "artist" ? `${b.artist} ${b.title}` : String(b[k] ?? "");
          return av < bv ? -m : av > bv ? m : 0;
        }).map((t) => t.id);
      } else if (request.moveTrackId) {
        ids = list.map((t) => t.id).filter((id) => id !== request.moveTrackId);
        const at = request.beforeTrackId ? ids.indexOf(request.beforeTrackId) : -1;
        ids.splice(at < 0 ? ids.length : at, 0, request.moveTrackId);
      } else {
        ids = list.map((t) => t.id);
      }
      return ids.map((id) => byId.get(id)).filter(Boolean);
    };
  });
});

test("playlist analyze-missing skips already-analyzed tracks and targets the rest", async ({ page }) => {
  await page.addInitScript(() => {
    // registerBackendJobEvents() only calls listen("job:event", ...) when
    // window.isTauri is truthy (see components/playback/actions.mjs). Without
    // this, the app never subscribes to job:event and the per-row
    // "analyzing" state (now driven entirely by job:event, see
    // job_manager.mjs handleJobEvent) would never update.
    window.isTauri = true;
    // @tauri-apps/api's real invoke()/isTauri() (bundled into main.js) route
    // through window.__TAURI_INTERNALS__.invoke once window.isTauri is set,
    // bypassing window.__TAURI__.core.invoke entirely. Forward it to our mock
    // so every invoke call -- not just event listener registration -- reaches
    // the mock regardless of which path the bundled API code takes.
    window.__TAURI_INTERNALS__ = {
      invoke: (cmd, args) => window.__TAURI__.core.invoke(cmd, args)
    };
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));

    const playlists = [{
      id: "pl-1",
      name: "Testi",
      source: "local",
      lastExportedAt: null,
      lastExportedUsbRoot: null,
      lastExportedTrackCount: null,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString()
    }];

    const playlistTracks = {
      "pl-1": [
        {
          id: "playlist-entry-1",
          localTrackId: "local-missing-1",
          title: "Local Missing",
          artist: "Artist A",
          album: "Album A",
          filePath: "/music/local-missing.mp3",
          waveformPeaksPath: "",
          waveformPreview: [],
          durationMs: null,
          bpm: null,
          key: null,
          analysisReady: false
        },
        {
          id: "usb-track-1",
          localTrackId: "usb-track-1",
          title: "USB Ready",
          artist: "Artist USB",
          album: "USB Album",
          filePath: "/USB/Contents/Artist USB/USB Ready.mp3",
          usbAnalysisPath: "/USB/PIONEER/USBANLZ/P001/TEST/ANLZ0000.DAT",
          waveformPeaksPath: "/USB/PIONEER/USBANLZ/P001/TEST/ANLZ0000.DAT",
          waveformPreview: [10, 20, 30],
          durationMs: 180000,
          bpm: 128,
          key: null,
          analysisReady: true
        }
      ]
    };

    const libraryTracks = [
      {
        id: "local-missing-1",
        title: "Local Missing",
        artist: "Artist A",
        album: "Album A",
        filePath: "/music/local-missing.mp3",
        waveformPeaksPath: "",
        waveformPreview: [],
        durationMs: null,
        bpm: null,
        key: null,
        analysisReady: false
      }
    ];

    const hydratedTrack = () => ({
      ...libraryTracks[0]
    });

    const analyzedRequests = [];
    const listeners = new Map();
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

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const request = payload?.request || payload;
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") {
            return { ok: true, data: { found: false, path: null } };
          }
          if (command === "list_playlists") {
            return { ok: true, data: { items: playlists } };
          }
          if (command === "get_playlist_tracks") {
            return { ok: true, data: window.__playlistTracksPage(playlistTracks[request.playlistId], request) };
          }
          if (command === "search_tracks" || command === "list_tracks") {
            return { ok: true, data: { total: libraryTracks.length, items: libraryTracks } };
          }
          if (command === "browse_source_files") {
            return { ok: true, data: { total: libraryTracks.length, items: libraryTracks } };
          }
          if (command === "analyze_new_tracks") {
            // "Analyze Missing" now sends no ids + a playlistId; the backend
            // picks the playlist's unanalyzed local tracks. Mirror that here.
            let ids = Array.isArray(request.trackIds) ? request.trackIds.map(String) : [];
            if (!ids.length && request.playlistId) {
              ids = (playlistTracks[request.playlistId] || [])
                .filter((t) => !t.analysisReady)
                .map((t) => String(t.localTrackId || t.id));
            }
            analyzedRequests.push({ trackIds: ids, playlistId: request.playlistId || null });
            const jobId = "job-analysis-mock";
            emitJobEvent({
              event: "job.started",
              jobId,
              jobType: "analysis",
              stage: "analyze_new_tracks",
              current: 0,
              total: Math.max(1, ids.length),
              percent: 0,
              message: "Analyzing selected tracks",
              timestamp: nowIso()
            });
            for (const trackId of ids) {
              if (trackId !== "local-missing-1") continue;
              emitJobEvent({
                event: "job.progress",
                jobId,
                jobType: "analysis",
                stage: "analyze_new_tracks",
                trackId,
                current: 0,
                total: 1,
                percent: 0,
                message: "Analyzing 1/1: Local Missing",
                trackReady: false,
                timestamp: nowIso()
              });
            }
            await new Promise((resolve) => { setTimeout(resolve, 200); });
            for (const trackId of ids) {
              if (trackId !== "local-missing-1") continue;
              playlistTracks["pl-1"][0].durationMs = 180000;
              playlistTracks["pl-1"][0].waveformPeaksPath = "/tmp/local-missing.DAT";
              playlistTracks["pl-1"][0].waveformPreview = [5, 10, 20];
              playlistTracks["pl-1"][0].bpm = 128;
              playlistTracks["pl-1"][0].analysisReady = true;
              libraryTracks[0].durationMs = 180000;
              libraryTracks[0].waveformPeaksPath = "/tmp/local-missing.DAT";
              libraryTracks[0].waveformPreview = [5, 10, 20];
              libraryTracks[0].bpm = 128;
              libraryTracks[0].analysisReady = true;
              emitJobEvent({
                event: "job.progress",
                jobId,
                jobType: "analysis",
                stage: "analyze_new_tracks",
                trackId,
                trackTitle: "Local Missing",
                filePath: "/music/local-missing.mp3",
                current: 1,
                total: 1,
                percent: 100,
                message: "Analyzing 1/1: Local Missing",
                trackReady: true,
                failed: false,
                analysisReady: true,
                bpm: 128,
                bpmAnalyzer: "mock-analyzer",
                durationMs: 180000,
                waveformPeaksPath: "/tmp/local-missing.DAT",
                waveformPreview: [5, 10, 20],
                timestamp: nowIso()
              });
            }
            emitJobEvent({
              event: "job.completed",
              jobId,
              jobType: "analysis",
              stage: "analyze_new_tracks",
              current: ids.length,
              total: Math.max(1, ids.length),
              percent: 100,
              message: `Analysis finished: ${ids.length} analyzed, 0 failed`,
              timestamp: nowIso()
            });
            return {
              ok: true,
              data: {
                jobId,
                analyzed: ids.length,
                failed: 0,
                warnings: [],
                items: ids.includes("local-missing-1") ? [hydratedTrack()] : []
              }
            };
          }
          if (command === "get_tracks_by_ids_with_previews") {
            const ids = request.trackIds || [];
            const items = ids.includes("local-missing-1") ? [hydratedTrack()] : [];
            return { ok: true, data: { items } };
          }
          if (command === "set_frontend_setting" || command === "get_frontend_settings") {
            return command === "get_frontend_settings"
              ? { ok: true, data: { settings: {} } }
              : { ok: true, data: { key: request.key, value: request.value } };
          }
          if (command === "resolve_playback_source") {
            return { ok: true, data: { resolvedPath: null, matchedBy: "none", trackId: null } };
          }
          if (command === "validate_usb_root") {
            return {
              ok: true,
              data: {
                valid: true,
                hasWriteAccess: true,
                normalizedRoot: "/USB",
                hasVendorRoot: true,
                hasContents: true,
                hasPdb: true,
                hasEdb: true,
                warnings: []
              }
            };
          }
          if (command === "run_usb_diagnostics") {
            return {
              ok: true,
              data: {
                overallStatus: "PASS",
                durationMs: 1,
                pdbIntegrity: { title: "PDB Integrity", status: "PASS", checks: [], counts: null },
                edbAccess: { title: "Database Access", status: "PASS", checks: [], counts: null },
                contentsIntegrity: { title: "Contents Integrity", status: "PASS", checks: [], counts: null },
                analysisIntegrity: { title: "Analysis Files", status: "PASS", checks: [], counts: null },
                playlistResolution: { title: "Playlist Resolution", status: "PASS", checks: [], counts: null },
                playlistDetails: [],
                warnings: []
              }
            };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      },
      event: { listen }
    };

    window.__playlistAnalysisTest = { analyzedRequests };
  });

  await page.goto("/");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistPanelTitle")).toContainText("Testi (2 tracks, Total time: 3:00)");
  // Footer total comes straight from the backend (get_playlist_tracks), not a
  // client-side sum -- 1 of the 2 tracks has no known length.
  await expect(page.locator("#playlistTotalDuration")).toHaveText("Total time: 3:00 (1 without length)");
  await expect(page.locator("#analyzePlaylistMissingBtn")).toHaveText("Analyze Missing Tracks (1)");
  await expect(page.locator("#analyzePlaylistMissingBtn")).toBeVisible();
  await expect(page.locator("#exportPlaylistBtn")).toBeHidden();

  await page.locator("#analyzePlaylistMissingBtn").click();
  // A transient loading-state class applied right after the click and cleared
  // once analysis completes (see the inverse check below) -- the default 5s
  // expect timeout is tight enough to flake under heavy parallel worker load
  // (e.g. the full suite at high --workers), even though the underlying
  // behavior is correct and fast under normal conditions.
  await expect(page.locator('#playlistTracksBody .track-grid-row[data-track-id="local-missing-1"]')).toHaveClass(
    /is-analyzing/,
    { timeout: 15_000 },
  );
  await expect(page.locator('#playlistTracksBody .track-grid-row[data-track-id="playlist-entry-1"]')).toHaveCount(0);

  await page.waitForFunction(() => {
    const reqs = window.__playlistAnalysisTest?.analyzedRequests || [];
    return reqs.length === 1;
  });

  const analyzedRequests = await page.evaluate(() => window.__playlistAnalysisTest.analyzedRequests);
  // The frontend delegates scoping to the backend: it sends the playlist id, not
  // a client-collected id list. The mock resolves it to the unanalyzed track.
  expect(analyzedRequests.map((item) => item.playlistId)).toEqual(["pl-1"]);
  expect(analyzedRequests.map((item) => item.trackIds)).toEqual([["local-missing-1"]]);
  await expect(page.locator("#statusText")).toContainText("Analyze Missing Tracks done: analyzed 1, failed 0");
  await expect(page.locator('#playlistTracksBody .track-grid-row[data-track-id="local-missing-1"]')).not.toHaveClass(/is-analyzing/);

  await page.locator('.nav-item[data-view="library"]').click();
  await expect(page.locator('#libraryTableBody .track-grid-row[data-track-id="local-missing-1"] [data-action="analyze-track"]')).toHaveText("Reanalyze");
  await expect(page.locator('#libraryTableBody .track-grid-row[data-track-id="local-missing-1"] .td-bpm')).toContainText("128");
  await expect(page.locator('#libraryTableBody .track-grid-row[data-track-id="local-missing-1"] .waveform.waveform-canvas')).toHaveCount(1);
});

test("playlist actions hide Analyze Missing when unnecessary and keep Export visible", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const playlists = [{
      id: "pl-1",
      name: "Ready Playlist",
      source: "local",
      lastExportedAt: null,
      lastExportedUsbRoot: null,
      lastExportedTrackCount: null,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString()
    }];

    const playlistTracks = {
      "pl-1": [
        {
          id: "local-ready-1",
          title: "Local Ready",
          artist: "Artist A",
          album: "Album A",
          filePath: "/music/local-ready.mp3",
          waveformPeaksPath: "/tmp/local-ready.dat",
          waveformPreview: [10, 20, 30],
          durationMs: 180000,
          bpm: 128,
          key: "8A",
          analysisReady: true
        }
      ]
    };

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const request = payload?.request || payload;
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") {
            return { ok: true, data: { found: false, path: null } };
          }
          if (command === "list_playlists") {
            return { ok: true, data: { items: playlists } };
          }
          if (command === "get_playlist_tracks") {
            return { ok: true, data: window.__playlistTracksPage(playlistTracks[request.playlistId], request) };
          }
          if (command === "search_tracks" || command === "list_tracks") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "browse_source_files") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "set_frontend_setting" || command === "get_frontend_settings") {
            return command === "get_frontend_settings"
              ? { ok: true, data: { settings: {} } }
              : { ok: true, data: { key: request.key, value: request.value } };
          }
          if (command === "resolve_playback_source") {
            return { ok: true, data: { resolvedPath: null, matchedBy: "none", trackId: null } };
          }
          if (command === "validate_usb_root") {
            return {
              ok: true,
              data: {
                valid: false,
                hasWriteAccess: false,
                normalizedRoot: "",
                hasVendorRoot: false,
                hasContents: false,
                hasPdb: false,
                hasEdb: false,
                warnings: []
              }
            };
          }
          if (command === "run_usb_diagnostics") {
            return {
              ok: true,
              data: {
                overallStatus: "PASS",
                durationMs: 1,
                pdbIntegrity: { title: "PDB Integrity", status: "PASS", checks: [], counts: null },
                edbAccess: { title: "Database Access", status: "PASS", checks: [], counts: null },
                contentsIntegrity: { title: "Contents Integrity", status: "PASS", checks: [], counts: null },
                analysisIntegrity: { title: "Analysis Files", status: "PASS", checks: [], counts: null },
                playlistResolution: { title: "Playlist Resolution", status: "PASS", checks: [], counts: null },
                playlistDetails: [],
                warnings: []
              }
            };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  });

  await page.goto("/");
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistPanelTitle")).toContainText("Ready Playlist (1 track, Total time: 3:00)");
  await expect(page.locator("#analyzePlaylistMissingBtn")).toBeHidden();
  await expect(page.locator("#exportPlaylistBtn")).toBeVisible();
  await expect(page.locator("#exportPlaylistBtn")).toHaveText("Select USB first");
});

test("playlist analyze-missing offers unanalyzed tracks that live on a USB drive", async ({ page }) => {
  // Regression: a track imported into the media library from an MP3 folder on a
  // USB stick gets is_usb_path=true from the backend. It is still a real,
  // analyzable library track -- the playlist must offer "Analyze Missing Tracks"
  // for it, not hide the button and let the backend export gate reject it later.
  await page.addInitScript(() => {
    window.isTauri = true;
    window.__TAURI_INTERNALS__ = {
      invoke: (cmd, args) => window.__TAURI__.core.invoke(cmd, args)
    };
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));

    const playlists = [{
      id: "pl-1",
      name: "USB Import Playlist",
      source: "local",
      lastExportedAt: null,
      lastExportedUsbRoot: null,
      lastExportedTrackCount: null,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString()
    }];

    const playlistTracks = {
      "pl-1": [
        {
          id: "local-ready-entry",
          localTrackId: "local-ready-1",
          title: "Local Ready",
          artist: "Artist A",
          album: "Album A",
          filePath: "/music/local-ready.mp3",
          waveformPeaksPath: "/tmp/local-ready.dat",
          waveformPreview: [10, 20, 30],
          durationMs: 180000,
          bpm: 128,
          key: "8A",
          analysisReady: true
        },
        {
          id: "usb-import-entry",
          localTrackId: "usb-import-1",
          title: "USB Import",
          artist: "Artist B",
          album: "Album B",
          filePath: "/USB/MP3/usb-import.mp3",
          isUsbPath: true,
          waveformPeaksPath: "",
          waveformPreview: [],
          durationMs: null,
          bpm: null,
          key: null,
          analysisReady: false
        }
      ]
    };

    const analyzedRequests = [];
    const listeners = new Map();
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

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const request = payload?.request || payload;
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") {
            return { ok: true, data: { found: false, path: null } };
          }
          if (command === "list_playlists") {
            return { ok: true, data: { items: playlists } };
          }
          if (command === "get_playlist_tracks") {
            return { ok: true, data: window.__playlistTracksPage(playlistTracks[request.playlistId], request) };
          }
          if (command === "search_tracks" || command === "list_tracks") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "browse_source_files") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "analyze_new_tracks") {
            // "Analyze Missing" sends the playlist id, not an id list; the
            // backend resolves it to that playlist's unanalyzed local tracks.
            let ids = Array.isArray(request.trackIds) ? request.trackIds.map(String) : [];
            if (!ids.length && request.playlistId) {
              ids = (playlistTracks[request.playlistId] || [])
                .filter((t) => !t.analysisReady)
                .map((t) => String(t.localTrackId || t.id));
            }
            analyzedRequests.push({ trackIds: ids, playlistId: request.playlistId || null });
            return {
              ok: true,
              data: { jobId: "job-mock", analyzed: ids.length, failed: 0, warnings: [], items: [] }
            };
          }
          if (command === "set_frontend_setting" || command === "get_frontend_settings") {
            return command === "get_frontend_settings"
              ? { ok: true, data: { settings: {} } }
              : { ok: true, data: { key: request.key, value: request.value } };
          }
          if (command === "resolve_playback_source") {
            return { ok: true, data: { resolvedPath: null, matchedBy: "none", trackId: null } };
          }
          if (command === "validate_usb_root") {
            return {
              ok: true,
              data: {
                valid: false,
                hasWriteAccess: false,
                normalizedRoot: "",
                hasVendorRoot: false,
                hasContents: false,
                hasPdb: false,
                hasEdb: false,
                warnings: []
              }
            };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      },
      event: { listen }
    };

    window.__playlistAnalysisTest = { analyzedRequests };
  });

  await page.goto("/");
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();

  await expect(page.locator("#analyzePlaylistMissingBtn")).toHaveText("Analyze Missing Tracks (1)");
  await expect(page.locator("#analyzePlaylistMissingBtn")).toBeVisible();
  await expect(page.locator("#exportPlaylistBtn")).toBeHidden();

  await page.locator("#analyzePlaylistMissingBtn").click();
  await page.waitForFunction(() => (window.__playlistAnalysisTest?.analyzedRequests || []).length === 1);
  const analyzedRequests = await page.evaluate(() => window.__playlistAnalysisTest.analyzedRequests);
  expect(analyzedRequests.map((item) => item.playlistId)).toEqual(["pl-1"]);
  expect(analyzedRequests.map((item) => item.trackIds)).toEqual([["usb-import-1"]]);
});

test("selecting a playlist with tracks right after an empty one still paints its waveforms", async ({ page }) => {
  // Regression: the empty-state chrome sets `#playlistTableWrap` to
  // display:none. renderTrackTable() paints each row's waveform canvas via
  // getBoundingClientRect(), which reads 0x0 while an ancestor is hidden, so
  // the canvas locks to 1x1 and is never repainted once the wrap is shown
  // again -- every waveform came up blank when the previously-selected
  // playlist was empty. The wrap must be revealed before the rows render.
  await page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const playlists = [
      { id: "pl-empty", name: "Empty Playlist", source: "local", lastExportedAt: null, lastExportedUsbRoot: null, lastExportedTrackCount: null, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() },
      { id: "pl-full", name: "Full Playlist", source: "local", lastExportedAt: null, lastExportedUsbRoot: null, lastExportedTrackCount: null, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() }
    ];

    const playlistTracks = {
      "pl-empty": [],
      "pl-full": [
        { id: "t1", title: "Song A", artist: "Artist", album: "Album", filePath: "/music/a.mp3", waveformPeaksPath: "/tmp/a.dat", waveformPreview: [8, 20, 42, 65, 30, 55], durationMs: 180000, bpm: 120, key: "8A", analysisReady: true },
        { id: "t2", title: "Song B", artist: "Artist", album: "Album", filePath: "/music/b.mp3", waveformPeaksPath: "/tmp/b.dat", waveformPreview: [7, 19, 43, 61, 34, 57], durationMs: 180000, bpm: 122, key: "9A", analysisReady: true }
      ]
    };

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const request = payload?.request || payload;
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") return { ok: true, data: { found: false, path: null } };
          if (command === "list_playlists") return { ok: true, data: { items: playlists } };
          if (command === "get_playlist_tracks") {
            return { ok: true, data: window.__playlistTracksPage(playlistTracks[request.playlistId] || [], request) };
          }
          if (command === "search_tracks" || command === "list_tracks") return { ok: true, data: { total: 0, items: [] } };
          if (command === "browse_source_files") return { ok: true, data: { total: 0, items: [] } };
          if (command === "set_frontend_setting" || command === "get_frontend_settings") {
            return command === "get_frontend_settings"
              ? { ok: true, data: { settings: {} } }
              : { ok: true, data: { key: request.key, value: request.value } };
          }
          if (command === "resolve_playback_source") {
            return { ok: true, data: { resolvedPath: null, matchedBy: "none", trackId: null } };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  });

  await page.goto("/");

  // Select the empty playlist first -- this hides #playlistTableWrap.
  await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Empty Playlist" }).click();
  await expect(page.locator("#playlistEmptyState")).toBeVisible();
  await expect(page.locator("#playlistTableWrap")).toBeHidden();

  // Now select the non-empty one.
  await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Full Playlist" }).click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);

  // Every waveform canvas must be sized to its laid-out box, not the 1x1 it
  // gets when measured inside a display:none ancestor.
  await expect.poll(async () => page.evaluate(() => {
    const canvases = Array.from(document.querySelectorAll("#playlistTracksBody .waveform.waveform-canvas .waveform-canvas-el"));
    return canvases.length > 0 && canvases.every((c) => c.width > 1 && c.height > 1);
  })).toBe(true);
});

function installReorderTauriMock(page, { usbSameNamePlaylistName, exportPruneStale } = {}) {
  return page.addInitScript(({ usbSameNamePlaylistName, exportPruneStale }) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    if (exportPruneStale !== undefined) {
      window.localStorage.setItem("djusbtkit.exportPruneStale", exportPruneStale ? "1" : "0");
    }

    const playlists = [{
      id: "pl-1",
      name: "Reorder Playlist",
      source: "local",
      lastExportedAt: null,
      lastExportedUsbRoot: null,
      lastExportedTrackCount: null,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString()
    }];

    const playlistTracks = {
      "pl-1": [
        { id: "t1", title: "Song A", artist: "Nina", album: "", filePath: "/music/a.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A", analysisReady: true },
        { id: "t2", title: "Song B", artist: "Alex", album: "", filePath: "/music/b.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A", analysisReady: true },
        { id: "t3", title: "Song C", artist: "Max", album: "", filePath: "/music/c.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A", analysisReady: true }
      ]
    };

    window.__reorderPlaylistTrackCalls = [];

    // Mirrors the backend's export_prune_stale setting: seeded from the param,
    // updated when the sync-mode toggle persists it via set_frontend_setting,
    // read by refresh_playlist_export_status / fetch_usb_playlists.
    let currentPruneStale = exportPruneStale !== false;
    const exportStatusFor = () => playlists.map((playlist) => {
      const sameNameExistsOnUsb = !!usbSameNamePlaylistName
        && String(playlist.name || "").trim().toLowerCase()
          === String(usbSameNamePlaylistName).trim().toLowerCase();
      return {
        playlistId: playlist.id,
        playlistName: playlist.name,
        sameNameExistsOnUsb,
        locksReorder: !currentPruneStale && sameNameExistsOnUsb
      };
    });

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const request = payload?.request || payload;
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") {
            return { ok: true, data: { found: false, path: null } };
          }
          if (command === "list_playlists") {
            return { ok: true, data: { items: playlists } };
          }
          if (command === "get_playlist_tracks") {
            return { ok: true, data: window.__playlistTracksPage(playlistTracks[request.playlistId] || [], request) };
          }
          if (command === "reorder_playlist_tracks") {
            window.__reorderPlaylistTrackCalls.push(request);
            playlistTracks[request.playlistId] = window.__applyReorder(playlistTracks[request.playlistId] || [], request);
            return { ok: true, data: { playlistId: request.playlistId, reordered: playlistTracks[request.playlistId].length } };
          }
          if (command === "search_tracks" || command === "list_tracks") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "browse_source_files") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "set_frontend_setting" || command === "get_frontend_settings") {
            if (command === "set_frontend_setting" && request.key === "ui_export_prune_stale_v1") {
              currentPruneStale = request.value === "1";
            }
            return command === "get_frontend_settings"
              ? { ok: true, data: { settings: {} } }
              : { ok: true, data: { key: request.key, value: request.value } };
          }
          if (command === "refresh_playlist_export_status") {
            return { ok: true, data: { playlistUsbExportStatus: exportStatusFor() } };
          }
          if (command === "resolve_playback_source") {
            return { ok: true, data: { resolvedPath: null, matchedBy: "none", trackId: null } };
          }
          if (command === "pick_usb_folder") {
            return usbSameNamePlaylistName ? "/USB" : null;
          }
          if (command === "validate_usb_root") {
            const valid = !!usbSameNamePlaylistName;
            return {
              ok: true,
              data: {
                valid,
                hasWriteAccess: valid,
                normalizedRoot: valid ? "/USB" : "",
                hasVendorRoot: valid,
                hasContents: valid,
                hasPdb: valid,
                hasEdb: valid,
                warnings: []
              }
            };
          }
          if (command === "fetch_usb_playlists") {
            const items = usbSameNamePlaylistName
              ? [{
                  id: "usb-pl-1",
                  name: usbSameNamePlaylistName,
                  source: "mock-tauri",
                  tracks: [{ title: "USB Track" }],
                  trackCount: 1
                }]
              : [];
            return {
              ok: true,
              data: {
                items,
                stats: { indexedTracks: 0, playlistReferencedTracks: 0, playlistEntries: items.length },
                warnings: [],
                playlistUsbExportStatus: exportStatusFor()
              }
            };
          }
          if (command === "run_usb_diagnostics") {
            return {
              ok: true,
              data: {
                overallStatus: "PASS",
                durationMs: 1,
                pdbIntegrity: { title: "PDB Integrity", status: "PASS", checks: [], counts: null },
                edbAccess: { title: "Database Access", status: "PASS", checks: [], counts: null },
                contentsIntegrity: { title: "Contents Integrity", status: "PASS", checks: [], counts: null },
                analysisIntegrity: { title: "Analysis Files", status: "PASS", checks: [], counts: null },
                playlistResolution: { title: "Playlist Resolution", status: "PASS", checks: [], counts: null },
                playlistDetails: [],
                warnings: [],
                playlistUsbExportStatus: []
              }
            };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  }, { usbSameNamePlaylistName, exportPruneStale });
}

async function dragPlaylistTrackRow(page, fromTrackId, toTrackId, dropNearTop) {
  const source = page.locator(`#playlistTracksBody .track-grid-row[data-track-id="${fromTrackId}"] [data-playlist-track-drag-handle]`);
  const target = page.locator(`#playlistTracksBody .track-grid-row[data-track-id="${toTrackId}"]`);
  const sourceBox = await source.boundingBox();
  const targetBox = await target.boundingBox();
  await page.mouse.move(sourceBox.x + sourceBox.width / 2, sourceBox.y + sourceBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(sourceBox.x + sourceBox.width / 2, sourceBox.y + sourceBox.height / 2 + 10, { steps: 5 });
  const targetY = dropNearTop ? targetBox.y + targetBox.height * 0.25 : targetBox.y + targetBox.height * 0.75;
  await page.mouse.move(targetBox.x + targetBox.width / 2, targetY, { steps: 10 });
  await page.mouse.up();
}

test("playlist track drag-and-drop reorder persists the new order", async ({ page }) => {
  await installReorderTauriMock(page);
  await page.goto("/");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  const rows = page.locator("#playlistTracksBody .track-grid-row");
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toHaveAttribute("data-track-id", "t1");
  await expect(rows.nth(1)).toHaveAttribute("data-track-id", "t2");
  await expect(rows.nth(2)).toHaveAttribute("data-track-id", "t3");

  // Drag "Song A" (t1, row 0) to below "Song C" (t3, row 2).
  await dragPlaylistTrackRow(page, "t1", "t3", false);

  await expect(rows.nth(0)).toHaveAttribute("data-track-id", "t2");
  await expect(rows.nth(1)).toHaveAttribute("data-track-id", "t3");
  await expect(rows.nth(2)).toHaveAttribute("data-track-id", "t1");

  const calls = await page.evaluate(() => window.__reorderPlaylistTrackCalls);
  expect(calls.length).toBeGreaterThan(0);
  expect(calls[calls.length - 1]).toEqual({ playlistId: "pl-1", moveTrackId: "t1", beforeTrackId: null });
});

test("playlist track drag that does not change position skips persisting order", async ({ page }) => {
  await installReorderTauriMock(page);
  await page.goto("/");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  const rows = page.locator("#playlistTracksBody .track-grid-row");
  await expect(rows).toHaveCount(3);

  const handle = page.locator('#playlistTracksBody .track-grid-row[data-track-id="t1"] [data-playlist-track-drag-handle]');
  const box = await handle.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 2, { steps: 3 });
  await page.mouse.up();

  await expect(rows.nth(0)).toHaveAttribute("data-track-id", "t1");
  const calls = await page.evaluate(() => window.__reorderPlaylistTrackCalls);
  expect(calls).toEqual([]);
});

test("playlist track drag handle stays enabled while a column sort is active", async ({ page }) => {
  await installReorderTauriMock(page);
  await page.goto("/");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
  await expect(page.locator("#playlistTracksBody [data-playlist-track-drag-handle]")).toHaveCount(3);

  await page.locator('#panel-playlist .track-grid-cell.sortable[data-sort-key="artist"]').click();
  await expect(page.locator("#playlistTracksBody [data-playlist-track-drag-handle]")).toHaveCount(3);
});

test("dragging a playlist track while sorted commits the sorted order first, then applies the manual move", async ({ page }) => {
  await installReorderTauriMock(page);
  await page.goto("/");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  const rows = page.locator("#playlistTracksBody .track-grid-row");
  await expect(rows).toHaveCount(3);

  // Sort by artist -> view becomes [t2 (Alex), t3 (Max), t1 (Nina)], distinct
  // from the persisted insertion order [t1, t2, t3].
  await page.locator('#panel-playlist .track-grid-cell.sortable[data-sort-key="artist"]').click();
  await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();
  await expect(rows.nth(0)).toHaveAttribute("data-track-id", "t2");
  await expect(rows.nth(1)).toHaveAttribute("data-track-id", "t3");
  await expect(rows.nth(2)).toHaveAttribute("data-track-id", "t1");

  // Drag "Alex" (t2, row 0) down past "Nina" (t1, row 2) while sorted.
  await dragPlaylistTrackRow(page, "t2", "t1", false);

  // The sorted view is persisted FIRST (sort-commit), then the single move --
  // so `beforeTrackId` lines up with what the user actually saw.
  const calls = await page.evaluate(() => window.__reorderPlaylistTrackCalls);
  expect(calls).toEqual([
    { playlistId: "pl-1", sortBy: "artist", sortDir: "asc" },
    { playlistId: "pl-1", moveTrackId: "t2", beforeTrackId: null },
  ]);

  // Sort indicator cleared the moment the drag started; nothing jumps around --
  // final order is the sorted order with just t2 dragged to the end.
  await expect(page.locator("#panel-playlist .sort-hint")).toBeHidden();
  await expect(page.locator("#panel-playlist .sortable.sort-asc, #panel-playlist .sortable.sort-desc")).toHaveCount(0);
  await expect(rows.nth(0)).toHaveAttribute("data-track-id", "t3");
  await expect(rows.nth(1)).toHaveAttribute("data-track-id", "t1");
  await expect(rows.nth(2)).toHaveAttribute("data-track-id", "t2");
});

test("playlist track drag handle is hidden while a search filter is active", async ({ page }) => {
  await installReorderTauriMock(page);
  await page.goto("/");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
  await expect(page.locator("#playlistTracksBody [data-playlist-track-drag-handle]")).toHaveCount(3);
  await expect(page.locator("#playlistTotalDuration")).toHaveText("Total time: 9:00");

  await page.locator("#playlistSearchInput").fill("Song A");
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(1);
  await expect(page.locator("#playlistTracksBody [data-playlist-track-drag-handle]")).toHaveCount(0);
  // Search is a backend query param now (like the library and USB views), so
  // the footer total tracks the filtered set -- one 3:00 track here.
  await expect(page.locator("#playlistTotalDuration")).toHaveText("Total time: 3:00");
});

async function connectUsbAndFetchPlaylists(page) {
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await page.locator("#refreshUsbBtn").click();
  await expect(page.locator("#usbPlaylists li[data-usb-playlist-li]")).toHaveCount(1);
}

test("playlist track drag handle is disabled with a tooltip when additive export won't reorder it on the USB", async ({ page }) => {
  await installReorderTauriMock(page, { usbSameNamePlaylistName: "Reorder Playlist", exportPruneStale: false });
  await page.goto("/");

  await connectUsbAndFetchPlaylists(page);

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  const rows = page.locator("#playlistTracksBody .track-grid-row");
  await expect(rows).toHaveCount(3);
  await expect(page.locator("#playlistTracksBody [data-playlist-track-drag-handle]")).toHaveCount(0);

  const disabledHandles = page.locator("#playlistTracksBody .drag-handle.disabled");
  await expect(disabledHandles).toHaveCount(3);
  await expect(disabledHandles.first()).toHaveAttribute(
    "data-tooltip",
    "Won't reorder on USB — \"Reorder Playlist\" already exists there, and additive export keeps its existing track order unchanged. New tracks are still added in your chosen order."
  );

  // Same backend-computed collision also drives the Export button into
  // "append" mode -- one flag, two UI spots.
  await expect(page.locator("#exportPlaylistBtn")).toHaveText("Append to (Reorder Playlist) on USB: (USB)");
  await expect(page.locator("#exportPlaylistBtn")).toHaveAttribute(
    "data-tooltip",
    'Append current playlist tracks to existing USB playlist "Reorder Playlist"'
  );

  // Dragging the disabled handle is a no-op -- it isn't draggable, so no
  // dragstart/reorder ever fires.
  const box = await disabledHandles.first().boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 40, { steps: 5 });
  await page.mouse.up();

  await expect(rows.nth(0)).toHaveAttribute("data-track-id", "t1");
  const calls = await page.evaluate(() => window.__reorderPlaylistTrackCalls);
  expect(calls).toEqual([]);
});

test("playlist header sort is disabled with a tooltip when additive export won't reorder it on the USB", async ({ page }) => {
  await installReorderTauriMock(page, { usbSameNamePlaylistName: "Reorder Playlist", exportPruneStale: false });
  await page.goto("/");

  await connectUsbAndFetchPlaylists(page);

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  const rows = page.locator("#playlistTracksBody .track-grid-row");
  await expect(rows).toHaveCount(3);

  const artistHeader = page.locator('#panel-playlist .track-grid-cell.sortable[data-sort-key="artist"]');
  await expect(artistHeader).toHaveAttribute(
    "data-tooltip",
    "Won't reorder on USB — \"Reorder Playlist\" already exists there, and additive export keeps its existing track order unchanged. New tracks are still added in your chosen order."
  );

  await artistHeader.click();

  // Clicking a locked header is a no-op: no hint, no sort classes, no reorder of rows.
  await expect(page.locator("#panel-playlist .sort-hint")).toBeHidden();
  await expect(artistHeader).not.toHaveClass(/sort-asc|sort-desc/);
  await expect(rows.nth(0)).toHaveAttribute("data-track-id", "t1");
  await expect(rows.nth(1)).toHaveAttribute("data-track-id", "t2");
  await expect(rows.nth(2)).toHaveAttribute("data-track-id", "t3");
});

test("changing the export sync mode releases and re-engages the open playlist's reorder lock without a USB rescan", async ({ page }) => {
  await installReorderTauriMock(page, { usbSameNamePlaylistName: "Reorder Playlist", exportPruneStale: false });
  await page.goto("/");

  await connectUsbAndFetchPlaylists(page);
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();

  const grid = page.locator('[data-track-grid][data-body-id="playlistTracksBody"]');
  const dragHandles = page.locator("#playlistTracksBody [data-playlist-track-drag-handle]");
  const artistHeader = page.locator('#panel-playlist .track-grid-cell.sortable[data-sort-key="artist"]');
  const lockedTooltip =
    "Won't reorder on USB — \"Reorder Playlist\" already exists there, and additive export keeps its existing track order unchanged. New tracks are still added in your chosen order.";

  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);

  // additive + same name on USB -> locked
  await expect(grid).toHaveAttribute("data-sort-locked", "true");
  await expect(dragHandles).toHaveCount(0);
  await expect(artistHeader).toHaveAttribute("data-tooltip", lockedTooltip);
  await expect(page.locator("#exportPlaylistBtn")).toHaveText("Append to (Reorder Playlist) on USB: (USB)");

  // switch to mirror -> lock releases immediately (no fetch_usb_playlists rerun)
  await page.locator("#settingsBtn").click();
  await page.locator("#exportSyncModeMirror").check({ force: true });
  await page.locator("#settingsCloseBtn").click();

  await expect(grid).toHaveAttribute("data-sort-locked", "false");
  await expect(dragHandles).toHaveCount(3);
  await expect(artistHeader).not.toHaveAttribute("data-tooltip", /.+/);
  await expect(page.locator("#exportPlaylistBtn")).toHaveText("Export to USB: USB");

  // a view-only column sort is now allowed
  await artistHeader.click();
  await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();

  // switch back to additive -> lock re-engages, active sort committed
  await page.locator("#settingsBtn").click();
  await page.locator("#exportSyncModeAdditive").check({ force: true });
  await page.locator("#settingsCloseBtn").click();

  await expect(grid).toHaveAttribute("data-sort-locked", "true");
  await expect(dragHandles).toHaveCount(0);
  await expect(page.locator("#panel-playlist .sort-hint")).toBeHidden();
  await expect(page.locator("#exportPlaylistBtn")).toHaveText("Append to (Reorder Playlist) on USB: (USB)");

  const calls = await page.evaluate(() => window.__reorderPlaylistTrackCalls);
  expect(calls.length).toBe(1);
  expect(calls[0]).toEqual({ playlistId: "pl-1", sortBy: "artist", sortDir: "asc" });
});

test("playlist track drag handle stays enabled in additive mode when no same-name USB playlist exists", async ({ page }) => {
  await installReorderTauriMock(page, { usbSameNamePlaylistName: "Some Other Playlist", exportPruneStale: false });
  await page.goto("/");

  await connectUsbAndFetchPlaylists(page);

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
  await expect(page.locator("#playlistTracksBody [data-playlist-track-drag-handle]")).toHaveCount(3);
  await expect(page.locator("#playlistTracksBody .drag-handle.disabled")).toHaveCount(0);
  await expect(page.locator("#exportPlaylistBtn")).toHaveText("Export to USB: USB");
});

test("switching away from a sorted playlist commits the sort as its real order, and the next playlist doesn't inherit it", async ({ page }) => {
  // Column sort is a free, reversible view op while browsing -- it only
  // becomes the playlist's real (and thus exported) order once you leave
  // the view. The playlist panel's header/table DOM is shared across every
  // playlist, so the *next* playlist viewed must never show a leftover
  // hint/arrow from a sort that belonged to a different one.
  await page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.exportPruneStale", "0");

    const playlists = [
      { id: "pl-free", name: "Free Playlist", source: "local", lastExportedAt: null, lastExportedUsbRoot: null, lastExportedTrackCount: null, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() },
      { id: "pl-locked", name: "Locked Playlist", source: "local", lastExportedAt: null, lastExportedUsbRoot: null, lastExportedTrackCount: null, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() }
    ];

    const playlistTracks = {
      "pl-free": [
        { id: "t1", title: "Song A", artist: "Artist", album: "Zulu Album", filePath: "/music/a.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A", analysisReady: true },
        { id: "t2", title: "Song B", artist: "Artist", album: "Alpha Album", filePath: "/music/b.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A", analysisReady: true }
      ],
      "pl-locked": [
        { id: "t3", title: "Song C", artist: "Artist", album: "Zulu Album", filePath: "/music/c.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A", analysisReady: true },
        { id: "t4", title: "Song D", artist: "Artist", album: "Alpha Album", filePath: "/music/d.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A", analysisReady: true }
      ]
    };

    window.__reorderPlaylistTrackCalls = [];

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const request = payload?.request || payload;
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") return { ok: true, data: { found: false, path: null } };
          if (command === "list_playlists") return { ok: true, data: { items: playlists } };
          if (command === "get_playlist_tracks") {
            return { ok: true, data: window.__playlistTracksPage(playlistTracks[request.playlistId], request) };
          }
          if (command === "reorder_playlist_tracks") {
            window.__reorderPlaylistTrackCalls.push(request);
            playlistTracks[request.playlistId] = window.__applyReorder(playlistTracks[request.playlistId], request);
            return { ok: true, data: { playlistId: request.playlistId, reordered: playlistTracks[request.playlistId].length } };
          }
          if (command === "search_tracks" || command === "list_tracks") return { ok: true, data: { total: 0, items: [] } };
          if (command === "browse_source_files") return { ok: true, data: { total: 0, items: [] } };
          if (command === "set_frontend_setting" || command === "get_frontend_settings") {
            return command === "get_frontend_settings"
              ? { ok: true, data: { settings: {} } }
              : { ok: true, data: { key: request.key, value: request.value } };
          }
          if (command === "resolve_playback_source") {
            return { ok: true, data: { resolvedPath: null, matchedBy: "none", trackId: null } };
          }
          if (command === "pick_usb_folder") return "/USB";
          if (command === "validate_usb_root") {
            return {
              ok: true,
              data: { valid: true, hasWriteAccess: true, normalizedRoot: "/USB", hasVendorRoot: true, hasContents: true, hasPdb: true, hasEdb: true, warnings: [] }
            };
          }
          if (command === "fetch_usb_playlists") {
            const items = [{ id: "usb-pl-1", name: "Locked Playlist", source: "mock-tauri", tracks: [{ title: "USB Track" }], trackCount: 1 }];
            const usbNames = new Set(items.map((item) => String(item.name || "").trim().toLowerCase()));
            return {
              ok: true,
              data: {
                items,
                stats: { indexedTracks: 0, playlistReferencedTracks: 0, playlistEntries: items.length },
                warnings: [],
                playlistUsbExportStatus: playlists.map((playlist) => {
                  const sameNameExistsOnUsb = usbNames.has(String(playlist.name || "").trim().toLowerCase());
                  return {
                    playlistId: playlist.id,
                    playlistName: playlist.name,
                    sameNameExistsOnUsb,
                    locksReorder: sameNameExistsOnUsb
                  };
                })
              }
            };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  });

  await page.goto("/");
  await connectUsbAndFetchPlaylists(page);

  // Sort the unlocked playlist by Album.
  await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Free Playlist" }).click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);
  await page.locator('#panel-playlist .sortable[data-sort-key="album"]').click();
  await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();
  await expect(page.locator("#playlistTracksBody .track-grid-row .track-title")).toHaveText(["Song B", "Song A"]);

  // Sorting alone must not have committed anything yet.
  expect(await page.evaluate(() => window.__reorderPlaylistTrackCalls.length)).toBe(0);

  // Switch to the locked playlist -- leaving Free Playlist commits its sort.
  await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Locked Playlist" }).click();
  const lockedRows = page.locator("#playlistTracksBody .track-grid-row");
  await expect(lockedRows).toHaveCount(2);

  const reorderCalls = await page.evaluate(() => window.__reorderPlaylistTrackCalls);
  expect(reorderCalls).toEqual([{ playlistId: "pl-free", sortBy: "album", sortDir: "asc" }]);

  // The locked playlist must not inherit the sort -- it was never sorted itself.
  await expect(page.locator("#panel-playlist .sort-hint")).toBeHidden();
  await expect(page.locator('#panel-playlist .sortable[data-sort-key="album"]')).not.toHaveClass(/sort-asc|sort-desc/);
  await expect(lockedRows.nth(0)).toHaveAttribute("data-track-id", "t3");
  await expect(lockedRows.nth(1)).toHaveAttribute("data-track-id", "t4");
});

function installTwoPlaylistTauriMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const playlists = [
      { id: "pl-a", name: "Playlist A", source: "local", lastExportedAt: null, lastExportedUsbRoot: null, lastExportedTrackCount: null, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() },
      { id: "pl-b", name: "Playlist B", source: "local", lastExportedAt: null, lastExportedUsbRoot: null, lastExportedTrackCount: null, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() }
    ];

    const playlistTracks = {
      "pl-a": [
        { id: "t1", title: "Song A", artist: "Artist", album: "Zulu Album", filePath: "/music/a.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A", analysisReady: true },
        { id: "t2", title: "Song B", artist: "Artist", album: "Alpha Album", filePath: "/music/b.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A", analysisReady: true }
      ],
      "pl-b": [
        { id: "t3", title: "Song C", artist: "Artist", album: "Zulu Album", filePath: "/music/c.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A", analysisReady: true },
        { id: "t4", title: "Song D", artist: "Artist", album: "Alpha Album", filePath: "/music/d.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A", analysisReady: true }
      ]
    };

    window.__reorderPlaylistTrackCalls = [];

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const request = payload?.request || payload;
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") return { ok: true, data: { found: false, path: null } };
          if (command === "list_playlists") return { ok: true, data: { items: playlists } };
          if (command === "get_playlist_tracks") {
            return { ok: true, data: window.__playlistTracksPage(playlistTracks[request.playlistId], request) };
          }
          if (command === "reorder_playlist_tracks") {
            window.__reorderPlaylistTrackCalls.push(request);
            playlistTracks[request.playlistId] = window.__applyReorder(playlistTracks[request.playlistId], request);
            return { ok: true, data: { playlistId: request.playlistId, reordered: playlistTracks[request.playlistId].length } };
          }
          if (command === "search_tracks" || command === "list_tracks") return { ok: true, data: { total: 0, items: [] } };
          if (command === "browse_source_files") return { ok: true, data: { total: 0, items: [] } };
          if (command === "set_frontend_setting" || command === "get_frontend_settings") {
            return command === "get_frontend_settings"
              ? { ok: true, data: { settings: {} } }
              : { ok: true, data: { key: request.key, value: request.value } };
          }
          if (command === "resolve_playback_source") {
            return { ok: true, data: { resolvedPath: null, matchedBy: "none", trackId: null } };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  });
}

test("switching between two unlocked playlists commits the outgoing sort and doesn't carry it into the next", async ({ page }) => {
  await installTwoPlaylistTauriMock(page);
  await page.goto("/");

  await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Playlist A" }).click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);
  await page.locator('#panel-playlist .sortable[data-sort-key="album"]').click();
  await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();
  await expect(page.locator("#playlistTracksBody .track-grid-row .track-title")).toHaveText(["Song B", "Song A"]);

  await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Playlist B" }).click();
  const rowsB = page.locator("#playlistTracksBody .track-grid-row");
  await expect(rowsB).toHaveCount(2);

  const reorderCalls = await page.evaluate(() => window.__reorderPlaylistTrackCalls);
  expect(reorderCalls).toEqual([{ playlistId: "pl-a", sortBy: "album", sortDir: "asc" }]);

  await expect(page.locator("#panel-playlist .sort-hint")).toBeHidden();
  await expect(page.locator('#panel-playlist .sortable[data-sort-key="album"]')).not.toHaveClass(/sort-asc|sort-desc/);
  await expect(rowsB.nth(0)).toHaveAttribute("data-track-id", "t3");
  await expect(rowsB.nth(1)).toHaveAttribute("data-track-id", "t4");
});

test("switching to a non-playlist view (Library) also commits an active playlist sort", async ({ page }) => {
  await installTwoPlaylistTauriMock(page);
  await page.goto("/");

  await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Playlist A" }).click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);
  await page.locator('#panel-playlist .sortable[data-sort-key="album"]').click();
  await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();

  await page.locator('.nav-item[data-view="library"]').click();
  await expect.poll(() => page.evaluate(() => window.__reorderPlaylistTrackCalls.length)).toBeGreaterThan(0);
  const reorderCalls = await page.evaluate(() => window.__reorderPlaylistTrackCalls);
  expect(reorderCalls).toEqual([{ playlistId: "pl-a", sortBy: "album", sortDir: "asc" }]);
});

test("a same-playlist refresh (search) does not clear or commit an active sort", async ({ page }) => {
  await installTwoPlaylistTauriMock(page);
  await page.goto("/");

  await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Playlist A" }).click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);
  await page.locator('#panel-playlist .sortable[data-sort-key="album"]').click();
  await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();
  await expect(page.locator("#playlistTracksBody .track-grid-row .track-title")).toHaveText(["Song B", "Song A"]);

  // Typing in the playlist search box refreshes the same playlist's view --
  // the sort must survive that (still just a view op), and search alone
  // must never trigger a commit (it would only submit the filtered subset).
  await page.locator("#playlistSearchInput").fill("Song");
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);
  await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();
  await expect(page.locator('#panel-playlist .sortable[data-sort-key="album"]')).toHaveClass(/sort-asc/);
  await expect(page.locator("#playlistTracksBody .track-grid-row .track-title")).toHaveText(["Song B", "Song A"]);
  expect(await page.evaluate(() => window.__reorderPlaylistTrackCalls.length)).toBe(0);
});
