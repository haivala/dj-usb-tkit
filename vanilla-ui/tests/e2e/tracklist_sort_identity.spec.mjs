// Regression coverage for: sorting a tracklist by column must not change
// which track Play/scrub-play/Remove act on. Row click handlers must
// resolve track identity by the row's data-id, never by its data-index
// into an array that may no longer match rendered (sorted) order.
import { test, expect } from "./coverage-fixture.mjs";

function baseTauriMock() {
  return {
    clear_frontend_log: async () => "",
    append_frontend_log: async () => null,
    show_window: async () => null,
    get_backend_log_buffer: async () => [],
    detect_external_master_db: async () => ({ ok: true, data: { found: false, path: null } }),
    set_frontend_setting: async (request) => ({ ok: true, data: { key: request.key, value: request.value } }),
    get_frontend_settings: async () => ({ ok: true, data: { settings: {} } }),
    resolve_playback_source: async () => ({ ok: true, data: { resolvedPath: null, matchedBy: "none", trackId: null } }),
    validate_usb_root: async () => ({
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
    }),
    fetch_usb_playlists: async () => ({ ok: true, data: { items: [], warnings: [] } }),
    fetch_usb_histories: async () => ({ ok: true, data: { items: [], warnings: [] } })
  };
}

test("Library: scrub-play after column sort plays the clicked row's track, not the pre-sort one", async ({ page }) => {
  await page.addInitScript(({ base }) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));

    // Insertion order deliberately differs from album-sorted order, so a
    // stale index lookup after sorting resolves to the wrong track.
    const tracks = [
      { id: "t1", title: "Song One", artist: "Artist", album: "Zulu Album", filePath: "/music/one.mp3", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A" },
      { id: "t2", title: "Song Two", artist: "Artist", album: "Alpha Album", filePath: "/music/two.mp3", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A" },
      { id: "t3", title: "Song Three", artist: "Artist", album: "Mike Album", filePath: "/music/three.mp3", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A" }
    ];

    window.__playResolvedCalls = [];

    const handlers = {
      ...base,
      list_playlists: async () => ({ ok: true, data: { items: [] } }),
      list_tracks: async () => ({ ok: true, data: { total: tracks.length, items: tracks } }),
      search_tracks: async () => ({ ok: true, data: { total: tracks.length, items: tracks } }),
      // Library column sort is a backend query param now (shared TrackListController).
      browse_source_files: async (request = {}) => {
        let rows = tracks.slice();
        const q = String(request.query || "").trim().toLowerCase();
        if (q) rows = rows.filter((t) => `${t.title} ${t.artist} ${t.album || ""}`.toLowerCase().includes(q));
        if (request.sortBy) {
          const m = request.sortDir === "desc" ? -1 : 1;
          const key = request.sortBy;
          rows = rows.slice().sort((a, b) => {
            if (key === "bpm" || key === "durationMs") return m * ((Number(a[key]) || 0) - (Number(b[key]) || 0));
            const av = key === "artist" ? `${a.artist} ${a.title}` : String(a[key] ?? "");
            const bv = key === "artist" ? `${b.artist} ${b.title}` : String(b[key] ?? "");
            return av < bv ? -m : av > bv ? m : 0;
          });
        }
        return { ok: true, data: { total: rows.length, items: rows, nextCursor: null, hasMore: false } };
      },
      play_resolved_track: async (request) => {
        window.__playResolvedCalls.push(request);
        return { ok: true, data: { path: request.filePath, trackId: request.trackId, durationMs: 0, positionMs: 0 } };
      }
    };

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const request = payload?.request || payload;
          const handler = handlers[command];
          if (handler) return handler(request);
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  }, { base: {} });

  await page.goto("/");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);

  // Sort ascending by Album: Alpha Album (Song Two), Mike Album (Song Three), Zulu Album (Song One).
  await page.locator('#panel-library .sortable[data-sort-key="album"]').click();
  await expect(page.locator("#libraryTableBody .track-grid-row .track-title")).toHaveText([
    "Song Two",
    "Song Three",
    "Song One"
  ]);

  // Click the waveform of the row now showing "Song Two" -- it sits at
  // array index 0 in the sorted view but was originally at index 1.
  await page.locator('#libraryTableBody .track-grid-row', { hasText: "Song Two" })
    .locator(".waveform")
    .click();

  await expect.poll(() => page.evaluate(() => window.__playResolvedCalls.length)).toBeGreaterThan(0);
  const call = await page.evaluate(() => window.__playResolvedCalls[0]);
  expect(call.trackId).toBe("t2");
  expect(call.filePath).toBe("/music/two.mp3");
});

test("Playlist: Play and Remove after column sort act on the clicked row's track", async ({ page }) => {
  await page.addInitScript(({ base }) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const playlists = [{
      id: "pl-1",
      name: "Test Playlist",
      source: "local",
      lastExportedAt: null,
      lastExportedUsbRoot: null,
      lastExportedTrackCount: null,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString()
    }];

    const playlistTracks = {
      "pl-1": [
        { id: "t1", title: "Song One", artist: "Artist", album: "Zulu Album", filePath: "/music/one.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A" },
        { id: "t2", title: "Song Two", artist: "Artist", album: "Alpha Album", filePath: "/music/two.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A" },
        { id: "t3", title: "Song Three", artist: "Artist", album: "Mike Album", filePath: "/music/three.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A" }
      ]
    };

    window.__playResolvedCalls = [];
    window.__removeTrackCalls = [];

    const handlers = {
      ...base,
      list_playlists: async () => ({ ok: true, data: { items: playlists } }),
      get_playlist_tracks: async (request) => {
        const items = playlistTracks[request.playlistId] || [];
        const totalDurationMs = items.reduce((sum, t) => sum + (t.durationMs > 0 ? t.durationMs : 0), 0);
        const durationKnownCount = items.filter((t) => t.durationMs > 0).length;
        return { ok: true, data: { playlistId: request.playlistId, items, totalDurationMs, durationKnownCount } };
      },
      list_tracks: async () => ({ ok: true, data: { total: 0, items: [] } }),
      search_tracks: async () => ({ ok: true, data: { total: 0, items: [] } }),
      browse_source_files: async () => ({ ok: true, data: { total: 0, items: [] } }),
      play_resolved_track: async (request) => {
        window.__playResolvedCalls.push(request);
        return { ok: true, data: { path: request.filePath, trackId: request.trackId, durationMs: 0, positionMs: 0 } };
      },
      remove_tracks_from_playlist: async (request) => {
        window.__removeTrackCalls.push(request);
        playlistTracks[request.playlistId] = (playlistTracks[request.playlistId] || [])
          .filter((t) => !request.trackIds.includes(t.id));
        return { ok: true, data: { removed: request.trackIds.length } };
      }
    };

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const request = payload?.request || payload;
          const handler = handlers[command];
          if (handler) return handler(request);
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  }, { base: {} });

  await page.goto("/");
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);

  // Sort ascending by Album: Alpha Album (Song Two), Mike Album (Song Three), Zulu Album (Song One).
  await page.locator('#panel-playlist .sortable[data-sort-key="album"]').click();
  await expect(page.locator("#playlistTracksBody .track-grid-row .track-title")).toHaveText([
    "Song Two",
    "Song Three",
    "Song One"
  ]);

  // Play the row now showing "Song Two" (array index 0 post-sort, index 1 pre-sort).
  await page.locator('#playlistTracksBody .track-grid-row', { hasText: "Song Two" })
    .locator('[data-action="play-library"]')
    .click();
  await expect.poll(() => page.evaluate(() => window.__playResolvedCalls.length)).toBeGreaterThan(0);
  const playCall = await page.evaluate(() => window.__playResolvedCalls[0]);
  expect(playCall.trackId).toBe("t2");
  expect(playCall.filePath).toBe("/music/two.mp3");

  // Remove the row now showing "Song Three" (array index 1 post-sort, index 2 pre-sort).
  await page.locator('#playlistTracksBody .track-grid-row', { hasText: "Song Three" })
    .locator('[data-action="remove-playlist-track"]')
    .click();
  await expect.poll(() => page.evaluate(() => window.__removeTrackCalls.length)).toBeGreaterThan(0);
  const removeCall = await page.evaluate(() => window.__removeTrackCalls[0]);
  expect(removeCall.trackIds).toEqual(["t3"]);
});

test("Playlist: header click shows and clears the sorted-by hint like other tracklists", async ({ page }) => {
  await page.addInitScript(({ base }) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const playlists = [{
      id: "pl-1",
      name: "Test Playlist",
      source: "local",
      lastExportedAt: null,
      lastExportedUsbRoot: null,
      lastExportedTrackCount: null,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString()
    }];

    const playlistTracks = {
      "pl-1": [
        { id: "t1", title: "Song One", artist: "Artist", album: "Zulu Album", filePath: "/music/one.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A" },
        { id: "t2", title: "Song Two", artist: "Artist", album: "Alpha Album", filePath: "/music/two.mp3", waveformPeaksPath: "", waveformPreview: [], durationMs: 180000, bpm: 120, key: "8A" }
      ]
    };

    const handlers = {
      ...base,
      list_playlists: async () => ({ ok: true, data: { items: playlists } }),
      get_playlist_tracks: async (request) => {
        const items = playlistTracks[request.playlistId] || [];
        const totalDurationMs = items.reduce((sum, t) => sum + (t.durationMs > 0 ? t.durationMs : 0), 0);
        const durationKnownCount = items.filter((t) => t.durationMs > 0).length;
        return { ok: true, data: { playlistId: request.playlistId, items, totalDurationMs, durationKnownCount } };
      },
      list_tracks: async () => ({ ok: true, data: { total: 0, items: [] } }),
      search_tracks: async () => ({ ok: true, data: { total: 0, items: [] } }),
      browse_source_files: async () => ({ ok: true, data: { total: 0, items: [] } })
    };

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const request = payload?.request || payload;
          const handler = handlers[command];
          if (handler) return handler(request);
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  }, { base: {} });

  await page.goto("/");
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);

  const albumHeader = page.locator('#panel-playlist .sortable[data-sort-key="album"]');
  const sortHint = page.locator("#panel-playlist .sort-hint");
  await expect(sortHint).toBeHidden();

  // Ascending click shows the hint, like Library/USB/History tracklists.
  await albumHeader.click();
  await expect(sortHint).toBeVisible();
  await expect(albumHeader).toHaveClass(/sort-asc/);

  // Descending click keeps it visible.
  await albumHeader.click();
  await expect(sortHint).toBeVisible();
  await expect(albumHeader).toHaveClass(/sort-desc/);

  // Third click cycles back to unsorted -- hint hides again.
  await albumHeader.click();
  await expect(sortHint).toBeHidden();
  await expect(albumHeader).not.toHaveClass(/sort-asc|sort-desc/);
});

test("Library: sorting by BPM column actually orders rows by numeric BPM value", async ({ page }) => {
  await page.addInitScript(({ base }) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));

    // BPM values are deliberately non-monotonic with insertion order and
    // include decimals/duplicates, so a broken comparator (e.g. lexicographic
    // string compare, or a comparator applied to the wrong array) reorders
    // rows without the BPM column actually ending up sorted.
    const tracks = [
      { id: "t1", title: "Track A", artist: "Artist", album: "Album", filePath: "/music/a.mp3", waveformPreview: [], durationMs: 180000, bpm: 128, key: "8A" },
      { id: "t2", title: "Track B", artist: "Artist", album: "Album", filePath: "/music/b.mp3", waveformPreview: [], durationMs: 180000, bpm: 90, key: "8A" },
      { id: "t3", title: "Track C", artist: "Artist", album: "Album", filePath: "/music/c.mp3", waveformPreview: [], durationMs: 180000, bpm: 174, key: "8A" },
      { id: "t4", title: "Track D", artist: "Artist", album: "Album", filePath: "/music/d.mp3", waveformPreview: [], durationMs: 180000, bpm: 9, key: "8A" },
      { id: "t5", title: "Track E", artist: "Artist", album: "Album", filePath: "/music/e.mp3", waveformPreview: [], durationMs: 180000, bpm: 128.5, key: "8A" }
    ];

    const handlers = {
      ...base,
      list_playlists: async () => ({ ok: true, data: { items: [] } }),
      list_tracks: async () => ({ ok: true, data: { total: tracks.length, items: tracks } }),
      search_tracks: async () => ({ ok: true, data: { total: tracks.length, items: tracks } }),
      browse_source_files: async (request = {}) => {
        let rows = tracks.slice();
        if (request.sortBy) {
          const m = request.sortDir === "desc" ? -1 : 1;
          const key = request.sortBy;
          rows = rows.slice().sort((a, b) => {
            if (key === "bpm" || key === "durationMs") return m * ((Number(a[key]) || 0) - (Number(b[key]) || 0));
            const av = key === "artist" ? `${a.artist} ${a.title}` : String(a[key] ?? "");
            const bv = key === "artist" ? `${b.artist} ${b.title}` : String(b[key] ?? "");
            return av < bv ? -m : av > bv ? m : 0;
          });
        }
        return { ok: true, data: { total: rows.length, items: rows, nextCursor: null, hasMore: false } };
      }
    };

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const request = payload?.request || payload;
          const handler = handlers[command];
          if (handler) return handler(request);
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  }, { base: {} });

  await page.goto("/");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(5);

  const bpmCellTexts = () => page.locator("#libraryTableBody .track-grid-row .td-bpm").allTextContents();

  // Ascending: 9, 90, 128, 128.5, 174.
  await page.locator('#panel-library .sortable[data-sort-key="bpm"]').click();
  await expect(page.locator("#libraryTableBody .track-grid-row .track-title")).toHaveText([
    "Track D",
    "Track B",
    "Track A",
    "Track E",
    "Track C"
  ]);
  expect((await bpmCellTexts()).map(Number)).toEqual([9, 90, 128, 128.5, 174]);

  // Descending: 174, 128.5, 128, 90, 9.
  await page.locator('#panel-library .sortable[data-sort-key="bpm"]').click();
  await expect(page.locator("#libraryTableBody .track-grid-row .track-title")).toHaveText([
    "Track C",
    "Track E",
    "Track A",
    "Track B",
    "Track D"
  ]);
  expect((await bpmCellTexts()).map(Number)).toEqual([174, 128.5, 128, 90, 9]);
});
