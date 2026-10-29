import { test, expect } from "./coverage-fixture.mjs";

function installWaveformStartupMock(page) {
  return page.addInitScript(() => {
    // The library\'s source folders, as `get_library_sources` answers.
    window.__e2eSourceFolders = ["/music"];
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
          if (command === "get_library_sources") {
            const folders = (window.__e2eSourceFolders || [])
              .map((path) => ({ path, enabled: true, missing: false, fullyAnalyzed: false }));
            return { ok: true, data: { folders, rekordboxDbEnabled: false, mixxxDbEnabled: false, everConfigured: folders.length > 0 } };
          }
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_rekordbox_db") {
            return { ok: true, data: { found: false, path: null } };
          }
          if (command === "list_playlists") {
            return { ok: true, data: { items: [] } };
          }
          if (command === "browse_source_files") {
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
  await expect(page.locator("#trackDetailView")).toBeVisible();

  await expect
    .poll(() => canvasesDrawn(page, "#trackDetailWaveform > .waveform-canvas-el"))
    .toEqual([true]);
});

function installSourceChipAnalysisMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    // The backend's library sources; the source commands below change them
    // as the real backend does.
    const folder = (path) => ({ path, enabled: true, missing: false, fullyAnalyzed: true });
    window.__sources = {
      folders: [folder("/music")], rekordboxDbEnabled: false, mixxxDbEnabled: false, everConfigured: true
    };
    window.__libraryToggles = [];
    window.__browseCalls = 0;
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
          const sources = window.__sources;
          if (command === "get_library_sources") return { ok: true, data: sources };
          if (command === "add_source_folders") {
            const known = new Set(sources.folders.map((f) => f.path));
            const added = payload.request.paths.filter((path) => !known.has(path));
            sources.folders.push(...added.map(folder));
            return { ok: true, data: { sources, added } };
          }
          if (command === "set_external_library_enabled") {
            const { library, enabled } = payload.request;
            window.__libraryToggles.push({ library, enabled });
            sources[library === "mixxx" ? "mixxxDbEnabled" : "rekordboxDbEnabled"] = enabled;
            return { ok: true, data: sources };
          }
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_rekordbox_db") {
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
          if (command === "browse_source_files") {
            window.__browseCalls += 1;
            return { ok: true, data: { total: tracks.length, items: tracks } };
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
  // click -- see backend/src/service/library.rs scan_library /
  // playback.rs resolve_playback_source.
  await expect.poll(async () => page.evaluate(() => window.__scanCalls)).toBe(1);
  await expect.poll(async () => page.evaluate(() => window.__lastScanPayload)).toEqual({
    sourceRoots: ["/music2"],
    incremental: true
  });
  await expect(page.locator(".source-chip.source-chip-analyzed")).toHaveCount(2);

  // The library chips sit in their own "Libraries" row, not among the folders.
  await expect(page.locator("#libraryChipsRow")).toBeVisible();
  await expect(page.locator("#sourceChipsContainer .source-chip-toggle[data-rekordbox-db]")).toHaveCount(0);

  // The rekordbox chip's checkbox (imported library) is a pure browse-filter
  // toggle -- it must never trigger a rescan, only the switch and a reload of
  // what's already indexed.
  const rekordboxDbToggle = page.locator('#libraryChipsContainer .source-chip-toggle[data-rekordbox-db="true"]');
  await expect(rekordboxDbToggle).toBeVisible();
  await expect(rekordboxDbToggle).toHaveAttribute("aria-label", "Toggle rekordbox library");
  await expect(rekordboxDbToggle).toBeEnabled();
  const browseCalls = await page.evaluate(() => window.__browseCalls);
  await rekordboxDbToggle.check();
  await expect(rekordboxDbToggle).toBeChecked();
  await expect.poll(async () => page.evaluate(() => window.__browseCalls)).toBeGreaterThan(browseCalls);
  await expect.poll(async () => page.evaluate(() => window.__scanCalls)).toBe(1);

  // Mixxx isn't imported yet: its checkbox is disabled until the chip's ↻ runs
  // the import, which then switches it on.
  const mixxxToggle = page.locator('#libraryChipsContainer .source-chip-toggle[data-mixxx-db="true"]');
  await expect(mixxxToggle).toHaveAttribute("aria-label", "Toggle Mixxx library");
  await expect(mixxxToggle).toBeDisabled();
  await expect(mixxxToggle).not.toBeChecked();
  await page.locator('#libraryChipsContainer .source-chip-import[data-import-library="mixxx"]').click();
  await expect.poll(async () => page.evaluate(() => window.__mixxxScanPayload)).toEqual({
    path: "/home/dj/.mixxx/mixxxdb.sqlite"
  });
  await expect(mixxxToggle).toBeEnabled();
  await expect(mixxxToggle).toBeChecked();
  await expect(rekordboxDbToggle).toBeChecked();

  // Unticking only switches it off: no import.
  await mixxxToggle.uncheck();
  await expect(mixxxToggle).not.toBeChecked();
  await expect.poll(async () => page.evaluate(() => window.__libraryToggles)).toEqual([
    { library: "rekordbox", enabled: true },
    { library: "mixxx", enabled: true },
    { library: "mixxx", enabled: false },
  ]);
  await expect.poll(async () => page.evaluate(() => window.__scanCalls)).toBe(1);
});

test("Import ticks rekordbox and Mixxx playlists, imports each and opens the first", async ({ page }) => {
  await installSourceChipAnalysisMock(page);
  await page.goto("/");

  const importBtn = page.locator("#importPlaylistBtn");
  await expect(importBtn).toBeVisible();
  await importBtn.click();

  const overlay = page.locator("#playlistImportOverlay");
  const list = page.locator("#playlistImportList");
  const row = (name) => list.locator("li", { hasText: name });
  const okBtn = page.locator("#playlistImportOkBtn");
  const forceLabel = page.locator("#playlistImportForceLabel");
  await expect(overlay).toBeVisible();
  // A column per library, its lists grouped by kind.
  await expect(list.locator(".playlist-import-column-title")).toHaveText(["rekordbox", "Mixxx"]);
  const column = (label) => list.locator(".playlist-import-column", { hasText: label });
  await expect(column("rekordbox").locator(".check-list-group")).toHaveText(["Playlists", "History"]);
  await expect(column("Mixxx").locator(".check-list-group")).toHaveText(["Playlists", "Crates", "History"]);
  await expect(row("Peak").locator(".add-to-note")).toHaveText("2 tracks");
  // A list imported before says which playlist importing it again updates.
  await expect(row("Test playlist").locator(".add-to-note")).toHaveText('3 tracks · updates "My Test playlist"');
  // Nothing ticked: nothing to import.
  await expect(okBtn).toBeDisabled();
  await expect(forceLabel).toHaveText("Force update track data");
  await expect(page.locator("#playlistImportForce")).not.toBeChecked();
  // The force option names the library when every ticked list comes from one.
  await row("Peak").locator("input").check();
  await expect(okBtn).toBeEnabled();
  await expect(okBtn).toHaveText("Import");
  await expect(forceLabel).toHaveText("Force update track data from Mixxx");
  await row("Winter Party").locator("input").check();
  await expect(okBtn).toHaveText("Import 2");
  await expect(forceLabel).toHaveText("Force update track data");

  // Escape cancels without importing.
  await page.keyboard.press("Escape");
  await expect(overlay).toBeHidden();
  expect(await page.evaluate(() => window.__playlistImports || null)).toBeNull();

  // One list: imported and opened.
  await importBtn.click();
  await expect(okBtn).toBeDisabled();
  await row("2026-09-30").locator("input").check();
  await okBtn.click();
  await expect(overlay).toBeHidden();
  await expect.poll(async () => page.evaluate(() => window.__playlistImports)).toEqual([
    { command: "import_mixxx_playlist", request: { path: "/home/dj/.mixxx/mixxxdb.sqlite", kind: "history", id: "3", force: false } }
  ]);
  await expect(page.locator('.nav-playlist-item[data-playlist-id="pl-mixxx"]')).toHaveClass(/active/);
  await expect(page.locator("#playlistPanelTitle")).toHaveText(/ · Imported from Mixxx$/);
  await expect(page.locator('.source-chip-toggle[data-mixxx-db="true"]')).toBeChecked();

  // Several lists from both libraries: each imported in list order, with
  // force for all; the first opens, and both libraries' chips turn on.
  await importBtn.click();
  await expect(page.locator("#playlistImportForce")).not.toBeChecked();
  await row("Peak").locator("input").check();
  await row("Winter Party").locator("input").check();
  await page.locator("#playlistImportForce").check();
  await okBtn.click();
  await expect.poll(async () => page.evaluate(() => window.__playlistImports.slice(1))).toEqual([
    { command: "import_rekordbox_playlist", request: { path: "/music/master.db", kind: "playlist", id: "1234567890", force: true } },
    { command: "import_mixxx_playlist", request: { path: "/home/dj/.mixxx/mixxxdb.sqlite", kind: "crate", id: "7", force: true } }
  ]);
  await expect(page.locator('.nav-playlist-item[data-playlist-id="pl-rekordbox"]')).toHaveClass(/active/);
  await expect(page.locator('.source-chip-toggle[data-rekordbox-db="true"]')).toBeChecked();
  await expect(page.locator('.source-chip-toggle[data-mixxx-db="true"]')).toBeChecked();
  await expect(page.locator("#statusText")).toContainText("Imported 2 of 2 playlists: 4 track(s)");
});
