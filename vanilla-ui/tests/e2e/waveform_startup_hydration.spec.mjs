import { test, expect } from "./coverage-fixture.mjs";

function installWaveformStartupMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

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

    const baseTracks = [
      {
        id: "t-1",
        title: "Track One",
        artist: "Artist",
        album: "Album",
        filePath: "/music/Track One.mp3",
        fileSizeBytes: 1000,
        waveformPeaksPath: "/tmp/t-1.DAT",
        waveformPreview: [],
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      },
      {
        id: "t-2",
        title: "Track Two",
        artist: "Artist",
        album: "Album",
        filePath: "/music/Track Two.mp3",
        fileSizeBytes: 1001,
        waveformPeaksPath: "/tmp/t-2.DAT",
        waveformPreview: [],
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      }
    ];

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
          if (command === "list_tracks" || command === "search_tracks" || command === "browse_source_files") {
            return { ok: true, data: { total: baseTracks.length, items: baseTracks } };
          }
          if (command === "get_tracks_by_ids_with_previews") {
            const ids = Array.isArray(payload?.request?.trackIds)
              ? payload.request.trackIds.map((v) => String(v))
              : [];
            const items = baseTracks
              .filter((t) => ids.includes(String(t.id)))
              .map((t) => ({
                ...t,
                waveformPreview: [8, 20, 42, 65, 30, 55]
              }));
            return { ok: true, data: { items } };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled command: ${command}` } };
        }
      },
      event: { listen }
    };
  });
}

test("startup hydrates waveform previews for tracks with waveform paths", async ({ page }) => {
  await installWaveformStartupMock(page);
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(2);

  await expect.poll(async () => {
    return page.locator("#libraryTableBody .waveform.waveform-canvas").count();
  }).toBe(2);
});

function installSourceChipAnalysisMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    window.localStorage.setItem(
      "djusbtkit.sourceRootEnabled",
      JSON.stringify({ "/music": true })
    );
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.__scanCalls = 0;
    window.__picked = false;

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

    const tracks = [
      {
        id: "t-1",
        title: "Track One",
        artist: "Artist",
        album: "Album",
        filePath: "/music/Track One.mp3",
        fileSizeBytes: 1000,
        waveformPeaksPath: "/tmp/t-1.DAT",
        waveformPreview: [8, 20, 42, 65, 30, 55],
        bpm: 128,
        key: "8A",
        durationMs: 195000,
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      },
      {
        id: "t-2",
        title: "Track Two",
        artist: "Artist",
        album: "Album",
        filePath: "/music2/Track Two.mp3",
        fileSizeBytes: 1001,
        waveformPeaksPath: "/tmp/t-2.DAT",
        waveformPreview: [7, 19, 43, 61, 34, 57],
        bpm: 126,
        key: "9A",
        durationMs: 201000,
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      }
    ];

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") {
            return { ok: true, data: { found: true, path: "/music/master.db" } };
          }
          if (command === "list_playlists") {
            return { ok: true, data: { items: [] } };
          }
          if (command === "pick_source_folders") {
            window.__picked = true;
            return ["/music2"];
          }
          if (command === "scan_library") {
            window.__scanCalls += 1;
            window.__lastScanPayload = payload?.request || null;
            return { ok: true, data: { indexed: 1, updated: 0, removed: 0 } };
          }
          if (command === "list_tracks" || command === "search_tracks" || command === "browse_source_files") {
            return {
              ok: true,
              data: {
                total: tracks.length,
                items: tracks,
                // Per-root readiness is backend-computed and delivered here.
                sourceRootAnalysis: [
                  { sourceRoot: "/music", total: 1, analyzed: 1, fullyAnalyzed: true },
                  { sourceRoot: "/music2", total: 1, analyzed: 1, fullyAnalyzed: true }
                ]
              }
            };
          }
          if (command === "get_tracks_by_ids_with_previews") {
            return { ok: true, data: { items: tracks } };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled command: ${command}` } };
        }
      },
      event: { listen }
    };
  });
}

test("source chips show analyzed green on startup and adding a source indexes it via scan_library", async ({ page }) => {
  await installSourceChipAnalysisMock(page);
  await page.goto("/");

  await expect(page.locator(".source-chip.source-chip-analyzed")).toHaveCount(1);
  await page.locator("#addSourceBtn").click();
  // Newly added folders must be indexed immediately (scan_library, metadata-only
  // insert) so tracks are playable right away without a manual "Scan Library"
  // click -- see backend/src/service/mod.rs scan_library / resolve_playback_source.
  await expect.poll(async () => page.evaluate(() => window.__scanCalls)).toBe(1);
  await expect.poll(async () => page.evaluate(() => window.__lastScanPayload)).toEqual({
    sourceRoots: ["/music2"],
    incremental: true
  });
  await expect(page.locator(".source-chip.source-chip-analyzed")).toHaveCount(2);

  // The master.db chip's checkbox is a pure browse-filter toggle -- it must
  // never trigger a rescan (scanning master.db isn't a thing; it's already
  // a fully-populated external database), only a re-filtered reload of
  // what's already indexed.
  const masterDbToggle = page.locator('.source-chip-toggle[data-master-db="true"]');
  await expect(masterDbToggle).toBeVisible();
  await expect(masterDbToggle).toHaveAttribute("aria-label", "Toggle desktop library");
  await expect(page.locator("#importMasterDbBtn")).toBeVisible();
  await masterDbToggle.check();
  await expect(masterDbToggle).toBeChecked();
  await expect.poll(async () => page.evaluate(() => window.__scanCalls)).toBe(1);
});

// Regression coverage for: searching the library used to make a fully
// -analyzed folder's chip lose its green state whenever the search term
// didn't match any track in that folder (backend/src/service/mod.rs's
// browse_source_files computed source_root_analysis from the search
// -filtered track list instead of each folder's full contents, so a
// zero-match folder's `total` collapsed to 0 and `fully_analyzed` -- which
// requires `total > 0` -- went false). The fix moved that computation to
// the unfiltered set; this mock simulates the *fixed* backend contract
// (sourceRootAnalysis always reflects full folder contents, independent of
// the query) as a frontend regression guard -- it can't exercise the real
// Rust computation itself (see the backend test in mod.rs for that), only
// that the frontend keeps trusting/applying that field correctly rather
// than re-deriving it from whatever's currently visible.
function installSourceChipSearchMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music-a", "/music-b"]));
    window.localStorage.setItem(
      "djusbtkit.sourceRootEnabled",
      JSON.stringify({ "/music-a": true, "/music-b": true })
    );
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const makeTrack = (id, title, filePath) => ({
      id,
      title,
      artist: "Artist",
      album: "Album",
      filePath,
      fileSizeBytes: 1000,
      waveformPeaksPath: `/tmp/${id}.DAT`,
      waveformPreview: [8, 20, 42, 65, 30, 55],
      bpm: 128,
      key: "8A",
      durationMs: 195000,
      createdAt: "2026-03-01T00:00:00Z",
      updatedAt: "2026-03-01T00:00:00Z"
    });

    // trackB's title deliberately shares no substring with the search term
    // used below -- searching for it matches zero tracks in /music-b, the
    // exact case that used to flip that folder's chip out of green.
    const trackA = makeTrack("t-a", "Findable Alpha", "/music-a/Findable Alpha.mp3");
    const trackB = makeTrack("t-b", "Unrelated Bravo", "/music-b/Unrelated Bravo.mp3");

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
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
          if (command === "list_tracks" || command === "search_tracks") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "browse_source_files") {
            const query = String(payload?.request?.query || "").trim().toLowerCase();
            const visible = [trackA, trackB].filter(
              (t) => !query || t.title.toLowerCase().includes(query)
            );
            return {
              ok: true,
              data: {
                total: visible.length,
                items: visible,
                nextCursor: null,
                hasMore: false,
                // Both folders are fully analyzed regardless of the
                // current search -- this must not track `visible`.
                sourceRootAnalysis: [
                  { sourceRoot: "/music-a", total: 1, analyzed: 1, fullyAnalyzed: true },
                  { sourceRoot: "/music-b", total: 1, analyzed: 1, fullyAnalyzed: true }
                ],
                totalDurationMs: visible.reduce((sum, t) => sum + t.durationMs, 0),
                durationKnownCount: visible.length
              }
            };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled command: ${command}` } };
        }
      },
      event: { listen: async () => () => {} }
    };
  });
}

test("searching does not clear an unrelated folder's analyzed chip", async ({ page }) => {
  await installSourceChipSearchMock(page);
  await page.goto("/");

  await expect(page.locator(".source-chip.source-chip-analyzed")).toHaveCount(2);

  // Matches only /music-a's track -- /music-b's chip must stay green even
  // though nothing in /music-b matches this search.
  await page.locator("#librarySearch").fill("Findable Alpha");
  await expect(page.locator(".source-chip.source-chip-analyzed")).toHaveCount(2);

  await page.locator("#librarySearch").fill("");
  await expect(page.locator(".source-chip.source-chip-analyzed")).toHaveCount(2);
});
