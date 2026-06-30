import { test, expect } from "@playwright/test";

test("playlist analyze-missing only targets local non-USB tracks", async ({ page }) => {
  await page.addInitScript(() => {
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
          key: null
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
          key: null
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
        key: null
      }
    ];

    const hydratedTrack = () => ({
      ...libraryTracks[0]
    });

    const analyzedRequests = [];

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
            return { ok: true, data: { playlistId: request.playlistId, items: playlistTracks[request.playlistId] || [] } };
          }
          if (command === "search_tracks" || command === "list_tracks") {
            return { ok: true, data: { total: libraryTracks.length, items: libraryTracks } };
          }
          if (command === "browse_source_files") {
            return { ok: true, data: { total: libraryTracks.length, items: libraryTracks } };
          }
          if (command === "get_system_parallelism") {
            return { ok: true, data: { workers: 4 } };
          }
          if (command === "analyze_track_piece") {
            analyzedRequests.push({ trackId: request.trackId, piece: request.piece });
            if (request.piece === "duration") {
              playlistTracks["pl-1"][0].durationMs = 180000;
              libraryTracks[0].durationMs = 180000;
            }
            if (request.piece === "waveform") {
              playlistTracks["pl-1"][0].waveformPeaksPath = "/tmp/local-missing.DAT";
              playlistTracks["pl-1"][0].waveformPreview = [5, 10, 20];
              libraryTracks[0].waveformPeaksPath = "/tmp/local-missing.DAT";
              libraryTracks[0].waveformPreview = [5, 10, 20];
            }
            if (request.piece === "bpm_key") {
              playlistTracks["pl-1"][0].bpm = 128;
              libraryTracks[0].bpm = 128;
            }
            return {
              ok: true,
              data: await new Promise((resolve) => {
                setTimeout(() => resolve({
                  bpm: request.piece === "bpm_key" ? 128 : null,
                  key: null,
                  durationMs: request.piece === "duration" ? 180000 : null,
                  artworkPath: null,
                  waveformPeaksPath: request.piece === "waveform" ? "/tmp/local-missing.DAT" : null,
                  waveformPreview: request.piece === "waveform" ? [5, 10, 20] : null
                }), 40);
              })
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
      }
    };

    window.__playlistAnalysisTest = { analyzedRequests };
  });

  await page.goto("/");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistPanelTitle")).toContainText("Testi");
  await expect(page.locator("#analyzePlaylistMissingBtn")).toHaveText("Analyze Missing Tracks (1)");
  await expect(page.locator("#analyzePlaylistMissingBtn")).toBeVisible();
  await expect(page.locator("#exportPlaylistBtn")).toBeHidden();

  await page.locator("#analyzePlaylistMissingBtn").click();
  await expect(page.locator('#playlistTracksBody .track-grid-row[data-track-id="local-missing-1"]')).toHaveClass(/is-analyzing/);
  await expect(page.locator('#playlistTracksBody .track-grid-row[data-track-id="playlist-entry-1"]')).toHaveCount(0);

  await page.waitForFunction(() => {
    const reqs = window.__playlistAnalysisTest?.analyzedRequests || [];
    return reqs.length === 4;
  });

  const analyzedRequests = await page.evaluate(() => window.__playlistAnalysisTest.analyzedRequests);
  expect(analyzedRequests.map((item) => item.trackId)).toEqual([
    "local-missing-1",
    "local-missing-1",
    "local-missing-1",
    "local-missing-1"
  ]);
  expect(analyzedRequests.map((item) => item.piece).sort()).toEqual(["artwork", "bpm_key", "duration", "waveform"]);
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
          key: "8A"
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
            return { ok: true, data: { playlistId: request.playlistId, items: playlistTracks[request.playlistId] || [] } };
          }
          if (command === "search_tracks" || command === "list_tracks") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "browse_source_files") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "get_system_parallelism") {
            return { ok: true, data: { workers: 4 } };
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
  await expect(page.locator("#playlistPanelTitle")).toContainText("Ready Playlist");
  await expect(page.locator("#analyzePlaylistMissingBtn")).toBeHidden();
  await expect(page.locator("#exportPlaylistBtn")).toBeVisible();
});
