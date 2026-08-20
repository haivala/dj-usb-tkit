import { test, expect } from "./coverage-fixture.mjs";

function installSelectionMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));

    const tracks = [
      { id: "t1", title: "Sel One", artist: "Artist", album: "Album", filePath: "/music/one.mp3" },
      { id: "t2", title: "Sel Two", artist: "Artist", album: "Album", filePath: "/music/two.mp3" },
      // No bpm field -- mirrors a real not-yet-analyzed library track. Regression coverage for
      // the "invalid type: string \"\", expected f64" crash: add_track_candidates_to_playlist
      // below rejects a non-numeric bpm the way the real Rust backend's serde does, so this only
      // stays green if the frontend never forwards a UI display sentinel like "" as bpm.
      { id: "t3", title: "Sel Three", artist: "Artist", album: "Album", filePath: "/music/three.mp3" }
    ];
    const playlists = [];
    const playlistTracks = new Map();

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") return { ok: true, data: { found: false, path: null } };
          if (command === "list_playlists") return { ok: true, data: { items: playlists } };
          if (command === "list_tracks" || command === "search_tracks" || command === "browse_source_files") {
            return { ok: true, data: { total: tracks.length, items: tracks } };
          }
          if (command === "create_playlist") {
            const name = String(payload?.request?.name || "Untitled");
            const id = `pl-${Date.now()}`;
            const row = {
              id, name, source: "local",
              lastExportedAt: null, lastExportedUsbRoot: null, lastExportedTrackCount: null,
              tracks: []
            };
            playlists.push(row);
            playlistTracks.set(id, []);
            return { ok: true, data: { playlistId: id, name } };
          }
          if (command === "add_track_candidates_to_playlist") {
            const playlistId = String(payload?.request?.playlistId || "");
            const candidates = Array.isArray(payload?.request?.tracks) ? payload.request.tracks : [];
            // Real serde rejects a non-numeric bpm on AddTrackCandidate with
            // "invalid type: string \"\", expected f64" -- mimic that here so a regression
            // (e.g. forwarding a UI display sentinel like bpm: "") fails this test too.
            for (const track of candidates) {
              const bpm = track?.bpm;
              if (bpm !== null && bpm !== undefined && typeof bpm !== "number") {
                return {
                  ok: false,
                  error: { code: "INVALID_ARGS", message: `invalid type: string "${bpm}", expected f64` }
                };
              }
            }
            const ids = candidates
              .map((track) => String(track?.localTrackId || track?.trackId || track?.id || "").trim())
              .filter(Boolean);
            const current = playlistTracks.get(playlistId) || [];
            let added = 0;
            for (const id of ids) {
              if (!current.includes(id)) {
                current.push(id);
                added += 1;
              }
            }
            playlistTracks.set(playlistId, current);
            return {
              ok: true,
              data: {
                playlistId,
                requested: candidates.length,
                resolved: ids.length,
                unresolved: candidates.length - ids.length,
                added,
                skipped: ids.length - added,
                resolutions: candidates.map((track, index) => ({
                  previousId: track?.trackId || track?.id || null,
                  trackId: ids[index] || null,
                  resolvedBy: ids[index] ? "self" : "none",
                  materialized: false
                }))
              }
            };
          }
          if (command === "get_playlist_tracks") {
            const playlistId = String(payload?.request?.playlistId || "");
            const ids = playlistTracks.get(playlistId) || [];
            return { ok: true, data: { playlistId, items: tracks.filter((t) => ids.includes(t.id)) } };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          if (command === "get_backend_log_buffer") return [];
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  });
}

test("Selection + bulk add to playlist", async ({ page }) => {
  await installSelectionMock(page);
  await page.goto("/");

  await page.locator("#addPlaylistBtn").click();
  await page.locator("#navPlaylistList .nav-new-input").fill("Bulk Add");
  await page.locator("#navPlaylistList .nav-new-input").press("Enter");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);
  await page.evaluate(() => {
    // Select every row, including the unanalyzed (no-bpm) "Sel Three" track -- regression
    // coverage for the bpm-crash fix (see the mock's add_track_candidates_to_playlist handler).
    const checkboxes = document.querySelectorAll('#libraryTableBody .track-grid-row input[type="checkbox"]');
    checkboxes.forEach((checkbox) => {
      checkbox.checked = true;
      checkbox.dispatchEvent(new Event("change", { bubbles: true }));
    });
  });
  await expect(page.locator("#selectionCount")).toContainText("3 selected");

  await page.evaluate(() => {
    document.getElementById("addSelectedBtn")?.click();
  });
  await expect(page.locator("#statusText")).toContainText("Added 3");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
  await expect(page.locator("#playlistTracksBody")).toContainText("Sel One");
  await expect(page.locator("#playlistTracksBody")).toContainText("Sel Three");
});

test("Selection state stays correct when filtering visible rows", async ({ page }) => {
  await installSelectionMock(page);
  await page.goto("/");

  await page.locator("#addPlaylistBtn").click();
  await page.locator("#navPlaylistList .nav-new-input").fill("Filter Add");
  await page.locator("#navPlaylistList .nav-new-input").press("Enter");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);
  await page.evaluate(() => {
    const checkboxes = Array.from(document.querySelectorAll('#libraryTableBody input[type="checkbox"]'));
    for (const checkbox of checkboxes) {
      checkbox.checked = true;
      checkbox.dispatchEvent(new Event("change", { bubbles: true }));
    }
  });
  await expect(page.locator("#selectionCount")).toContainText("3 selected");

  await page.locator('.nav-item[data-view="library"]').click();
  await page.locator("#librarySearch").fill("Sel Two");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);
  await expect(page.locator("#selectionCount")).toContainText("1 selected");

  await page.evaluate(() => {
    document.getElementById("addSelectedBtn")?.click();
  });
  await expect(page.locator("#statusText")).toContainText("Added 1");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(1);
  await expect(page.locator("#playlistTracksBody")).toContainText("Sel Two");
});

// The library is server-paginated (browse_source_files, LIBRARY_LOAD_LIMIT_DEFAULT tracks/page).
// "select all" used to only select state.filteredTracks -- the tracks already loaded into the
// UI -- so on any library with more matches than one page, it silently missed the rest. This
// mock implements real cursor pagination (unlike installSelectionMock's single-page stub) so
// that regression can't sneak back in unnoticed.
function installPaginatedLibraryMock(page, { trackCount }) {
  return page.addInitScript(({ trackCount }) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));

    const allTracks = Array.from({ length: trackCount }, (_, i) => ({
      id: `t${i + 1}`,
      title: `Track ${String(i + 1).padStart(4, "0")}`,
      artist: "Artist",
      album: "Album",
      filePath: `/music/track-${i + 1}.mp3`
    }));
    const playlists = [];
    const playlistTracks = new Map();

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") return { ok: true, data: { found: false, path: null } };
          if (command === "list_playlists") return { ok: true, data: { items: playlists } };
          if (command === "browse_source_files") {
            const limit = Number(payload?.request?.limit) || allTracks.length;
            const cursor = Number(payload?.request?.cursor) || 0;
            const page = allTracks.slice(cursor, cursor + limit);
            const nextOffset = cursor + page.length;
            const hasMore = nextOffset < allTracks.length;
            return {
              ok: true,
              data: { total: allTracks.length, items: page, nextCursor: hasMore ? String(nextOffset) : null, hasMore }
            };
          }
          if (command === "create_playlist") {
            const name = String(payload?.request?.name || "Untitled");
            const id = `pl-${Date.now()}`;
            const row = {
              id, name, source: "local",
              lastExportedAt: null, lastExportedUsbRoot: null, lastExportedTrackCount: null,
              tracks: []
            };
            playlists.push(row);
            playlistTracks.set(id, []);
            return { ok: true, data: { playlistId: id, name } };
          }
          if (command === "add_track_candidates_to_playlist") {
            const playlistId = String(payload?.request?.playlistId || "");
            const candidates = Array.isArray(payload?.request?.tracks) ? payload.request.tracks : [];
            const ids = candidates
              .map((track) => String(track?.localTrackId || track?.trackId || track?.id || "").trim())
              .filter(Boolean);
            const current = playlistTracks.get(playlistId) || [];
            let added = 0;
            for (const id of ids) {
              if (!current.includes(id)) {
                current.push(id);
                added += 1;
              }
            }
            playlistTracks.set(playlistId, current);
            return {
              ok: true,
              data: {
                playlistId,
                requested: candidates.length,
                resolved: ids.length,
                unresolved: candidates.length - ids.length,
                added,
                skipped: ids.length - added,
                resolutions: candidates.map((track, index) => ({
                  previousId: track?.trackId || track?.id || null,
                  trackId: ids[index] || null,
                  resolvedBy: ids[index] ? "self" : "none",
                  materialized: false
                }))
              }
            };
          }
          if (command === "get_playlist_tracks") {
            const playlistId = String(payload?.request?.playlistId || "");
            const ids = playlistTracks.get(playlistId) || [];
            return { ok: true, data: { playlistId, items: allTracks.filter((t) => ids.includes(t.id)) } };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          if (command === "get_backend_log_buffer") return [];
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  }, { trackCount });
}

test("select all loads and adds every matching track across multiple pages, not just the first page", async ({ page }) => {
  const trackCount = 220; // over LIBRARY_LOAD_LIMIT_DEFAULT (200) -- forces a second page
  await installPaginatedLibraryMock(page, { trackCount });
  await page.goto("/");

  await page.locator("#addPlaylistBtn").click();
  await page.locator("#navPlaylistList .nav-new-input").fill("All Tracks");
  await page.locator("#navPlaylistList .nav-new-input").press("Enter");

  await expect(page.locator("#libraryTableBody .track-grid-row").first()).toHaveCount(1);
  const initialCount = await page.locator("#libraryTableBody .track-grid-row").count();
  expect(initialCount).toBeLessThan(trackCount);

  await page.evaluate(() => {
    const checkbox = document.getElementById("selectAllTracks");
    checkbox.checked = true;
    checkbox.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await expect(page.locator("#selectionCount")).toContainText(`${trackCount} selected`, { timeout: 15000 });

  await page.evaluate(() => {
    document.getElementById("addSelectedBtn")?.click();
  });
  await expect(page.locator("#statusText")).toContainText(`Added ${trackCount}`, { timeout: 15000 });

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(trackCount, { timeout: 15000 });
});
