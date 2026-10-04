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
        analysisReady: true,
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
          if (command === "resolve_track_identity") {
            return { ok: true, data: { trackId: payload?.request?.trackId ?? null } };
          }
          if (command === "get_track_detail") {
            // Two minutes of PWV5 detail entries (2 bytes each, ~150/s):
            // R(3) G(3) B(3) height(5), with a varying height.
            let bytes = "";
            for (let i = 0; i < 120 * 150; i += 1) {
              const entry = (7 << 13) | (5 << 10) | (3 << 7) | ((4 + ((i * 7) % 28)) << 2);
              bytes += String.fromCharCode(entry >> 8, entry & 0xff);
            }
            return {
              ok: true,
              data: {
                track: { ...baseTracks[0], durationMs: 120_000, bpm: 120 },
                detailWaveform: btoa(bytes),
                firstBeatMs: 100,
                cues: [],
                keyOptions: []
              }
            };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled command: ${command}` } };
        }
      },
      event: { listen }
    };
  });
}

// Whether every canvas matching `selector` has visible pixels drawn on it.
function canvasesDrawn(page, selector) {
  return page.evaluate(
    (sel) =>
      [...document.querySelectorAll(sel)].map((canvas) => {
        if (!canvas.width || !canvas.height) return false;
        const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
        return pixels.some((value, i) => i % 4 === 3 && value > 0);
      }),
    selector
  );
}

test("startup hydrates waveform previews for tracks with waveform paths", async ({ page }) => {
  await installWaveformStartupMock(page);
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(2);

  await expect.poll(async () => {
    return page.locator("#libraryTableBody .waveform.waveform-canvas").count();
  }).toBe(2);

  // The waveforms are actually drawn, not just laid out: both row canvases
  // have visible pixels.
  await expect
    .poll(() => canvasesDrawn(page, "#libraryTableBody .waveform-canvas-el"))
    .toEqual([true, true]);
});

test("the cue editor draws the track's detail waveform", async ({ page }) => {
  await installWaveformStartupMock(page);
  await page.goto("/");

  const row = page.locator("#libraryTableBody .track-grid-row").first();
  await row.hover();
  await row.locator('[data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();

  await expect
    .poll(() => canvasesDrawn(page, "#trackDetailWaveform > .waveform-canvas-el"))
    .toEqual([true]);
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
            return { ok: true, data: { found: true, path: "/music/master.db", imported: true } };
          }
          if (command === "detect_external_mixxx_db") {
            // Not imported until "Import from Mixxx" (the chip's ↻) runs.
            return {
              ok: true,
              data: { found: true, path: "/home/dj/.mixxx/mixxxdb.sqlite", imported: !!window.__mixxxImported }
            };
          }
          if (command === "scan_mixxx_db") {
            window.__mixxxImported = true;
            window.__mixxxScanPayload = payload?.request || null;
            return { ok: true, data: { indexed: 2, updated: 0, removed: 0, notFound: [], warnings: [] } };
          }
          if (command === "list_mixxx_playlists") {
            return {
              ok: true,
              data: {
                items: [
                  {
                    id: "4", name: "Test playlist", kind: "playlist", trackCount: 3,
                    existingPlaylist: { id: "pl-old", name: "My Test playlist" }
                  },
                  { id: "7", name: "Peak", kind: "crate", trackCount: 2 },
                  { id: "3", name: "2026-09-30", kind: "history", trackCount: 4 }
                ]
              }
            };
          }
          if (command === "list_rekordbox_playlists") {
            return {
              ok: true,
              data: {
                items: [
                  { id: "1234567890", name: "Winter Party", kind: "playlist", trackCount: 40 },
                  { id: "1122334455", name: "HISTORY 2025-01-01", kind: "history", trackCount: 7 }
                ]
              }
            };
          }
          window.__importedPlaylists = window.__importedPlaylists || [];
          if (command === "import_mixxx_playlist" || command === "import_rekordbox_playlist") {
            const playlistId = command === "import_mixxx_playlist" ? "pl-mixxx" : "pl-rekordbox";
            // Its tracks are Mixxx-library tracks now, as the real backend reports.
            if (command === "import_mixxx_playlist") window.__mixxxImported = true;
            window.__playlistImports = [...(window.__playlistImports || []), { command, request: payload?.request || null }];
            window.__importedPlaylists.push({
              id: playlistId, name: playlistId, trackCount: 2,
              importedFrom: command === "import_mixxx_playlist" ? "Mixxx" : "rekordbox",
              createdAt: "2026-03-01T00:00:00Z", updatedAt: "2026-03-01T00:00:00Z"
            });
            return {
              ok: true,
              data: { playlistId, name: playlistId, added: 2, indexed: 2, notFound: [], warnings: [] }
            };
          }
          if (command === "list_playlists") {
            return { ok: true, data: { items: window.__importedPlaylists } };
          }
          if (command === "get_playlist_tracks") {
            return { ok: true, data: { total: 0, items: [], hasMore: false, nextCursor: null } };
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
            if (command === "browse_source_files") window.__lastBrowsePayload = payload?.request || null;
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
          if (command === "get_source_root_analysis") {
            return {
              ok: true,
              data: {
                items: (payload?.request?.sourceRoots || []).map((sourceRoot) => ({
                  sourceRoot, total: 1, analyzed: 1, fullyAnalyzed: true
                }))
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

  // The library chips sit in their own "Libraries" row, not among the folders.
  await expect(page.locator("#libraryChipsRow")).toBeVisible();
  await expect(page.locator("#sourceChipsContainer .source-chip-toggle[data-master-db]")).toHaveCount(0);

  // The rekordbox chip's checkbox (imported library) is a pure browse-filter
  // toggle -- it must never trigger a rescan, only a re-filtered reload of
  // what's already indexed.
  const masterDbToggle = page.locator('#libraryChipsContainer .source-chip-toggle[data-master-db="true"]');
  await expect(masterDbToggle).toBeVisible();
  await expect(masterDbToggle).toHaveAttribute("aria-label", "Toggle rekordbox library");
  await expect(masterDbToggle).toBeEnabled();
  await masterDbToggle.check();
  await expect(masterDbToggle).toBeChecked();
  await expect.poll(async () => page.evaluate(() => window.__scanCalls)).toBe(1);

  // Mixxx isn't imported yet: its checkbox is disabled until the chip's ↻ runs
  // the import, which then turns it on (a separate browse flag).
  const mixxxToggle = page.locator('#libraryChipsContainer .source-chip-toggle[data-mixxx-db="true"]');
  await expect(mixxxToggle).toHaveAttribute("aria-label", "Toggle Mixxx library");
  await expect(mixxxToggle).toBeDisabled();
  await expect(mixxxToggle).not.toBeChecked();
  await expect.poll(async () => page.evaluate(() => window.__lastBrowsePayload?.includeMixxxDb)).toBe(false);
  await page.locator('#libraryChipsContainer .source-chip-import[data-import-library="mixxx"]').click();
  await expect.poll(async () => page.evaluate(() => window.__mixxxScanPayload)).toEqual({
    path: "/home/dj/.mixxx/mixxxdb.sqlite"
  });
  await expect(mixxxToggle).toBeEnabled();
  await expect(mixxxToggle).toBeChecked();
  await expect.poll(async () => page.evaluate(() => window.__lastBrowsePayload?.includeMixxxDb)).toBe(true);
  await expect.poll(async () => page.evaluate(() => window.__lastBrowsePayload?.includeMasterDb)).toBe(true);

  // Unticking only filters: no import, and the browse drops Mixxx.
  await mixxxToggle.uncheck();
  await expect.poll(async () => page.evaluate(() => window.__lastBrowsePayload?.includeMixxxDb)).toBe(false);
  await expect.poll(async () => page.evaluate(() => window.__scanCalls)).toBe(1);
});

test("Import picks a rekordbox or Mixxx playlist, imports it and opens the new playlist", async ({ page }) => {
  await installSourceChipAnalysisMock(page);
  await page.goto("/");

  const importBtn = page.locator("#importPlaylistBtn");
  await expect(importBtn).toBeVisible();
  await importBtn.click();

  const overlay = page.locator("#playlistImportOverlay");
  await expect(overlay).toBeVisible();
  await expect(page.locator("#playlistImportSelect optgroup")).toHaveCount(5);
  expect(await page.locator("#playlistImportSelect optgroup").evaluateAll((groups) => groups.map((g) => g.label)))
    .toEqual(["rekordbox playlists", "rekordbox history", "Mixxx playlists", "Mixxx crates", "Mixxx history"]);
  await expect(page.locator('#playlistImportSelect optgroup[label="Mixxx crates"] option')).toHaveText("Peak (2)");
  // The closed select shows only the name: the line under it names the source.
  await expect(page.locator("#playlistImportSource")).toHaveText("rekordbox playlist · 40 tracks");
  await expect(page.locator("#playlistImportForceLabel")).toHaveText("Force update track data from rekordbox");
  await expect(page.locator("#playlistImportForce")).not.toBeChecked();
  // A list imported before says which playlist importing it again updates.
  await page.locator("#playlistImportSelect").selectOption({ label: "Test playlist (3)" });
  await expect(page.locator("#playlistImportSource"))
    .toHaveText('Mixxx playlist · 3 tracks · updates your playlist "My Test playlist"');
  await expect(page.locator("#playlistImportForceLabel")).toHaveText("Force update track data from Mixxx");
  await page.locator("#playlistImportSelect").selectOption({ label: "Peak (2)" });
  await expect(page.locator("#playlistImportSource")).toHaveText("Mixxx crate · 2 tracks");

  // Escape cancels without importing.
  await page.keyboard.press("Escape");
  await expect(overlay).toBeHidden();
  expect(await page.evaluate(() => window.__playlistImports || null)).toBeNull();

  await importBtn.click();
  await page.locator("#playlistImportSelect").selectOption({ label: "2026-09-30 (4)" });
  await expect(page.locator("#playlistImportSource")).toHaveText("Mixxx history session · 4 tracks");
  await page.locator("#playlistImportOkBtn").click();
  await expect(overlay).toBeHidden();
  await expect.poll(async () => page.evaluate(() => window.__playlistImports)).toEqual([
    { command: "import_mixxx_playlist", request: { path: "/home/dj/.mixxx/mixxxdb.sqlite", kind: "history", id: "3", force: false } }
  ]);
  await expect(page.locator('.nav-playlist-item[data-playlist-id="pl-mixxx"]')).toHaveClass(/active/);
  await expect(page.locator("#playlistPanelTitle")).toHaveText(/ · Imported from Mixxx$/);
  await expect(page.locator('.source-chip-toggle[data-mixxx-db="true"]')).toBeChecked();

  await importBtn.click();
  await expect(page.locator("#playlistImportForce")).not.toBeChecked();
  await page.locator("#playlistImportSelect").selectOption({ label: "Winter Party (40)" });
  await page.locator("#playlistImportForce").check();
  await page.locator("#playlistImportOkBtn").click();
  await expect.poll(async () => page.evaluate(() => window.__playlistImports.at(-1))).toEqual({
    command: "import_rekordbox_playlist",
    request: { path: "/music/master.db", kind: "playlist", id: "1234567890", force: true }
  });
  await expect(page.locator('.nav-playlist-item[data-playlist-id="pl-rekordbox"]')).toHaveClass(/active/);
  await expect(page.locator('.source-chip-toggle[data-master-db="true"]')).toBeChecked();
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
          if (command === "get_source_root_analysis") {
            return {
              ok: true,
              data: {
                items: [
                  { sourceRoot: "/music-a", total: 1, analyzed: 1, fullyAnalyzed: true },
                  { sourceRoot: "/music-b", total: 1, analyzed: 1, fullyAnalyzed: true }
                ]
              }
            };
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
