import { test, expect } from "@playwright/test";

function installSelectionMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));

    const tracks = [
      { id: "t1", title: "Sel One", artist: "Artist", album: "Album", filePath: "/music/one.mp3" },
      { id: "t2", title: "Sel Two", artist: "Artist", album: "Album", filePath: "/music/two.mp3" }
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
          if (command === "add_tracks_to_playlist") {
            const playlistId = String(payload?.request?.playlistId || "");
            const ids = Array.isArray(payload?.request?.trackIds) ? payload.request.trackIds.map(String) : [];
            const current = playlistTracks.get(playlistId) || [];
            let added = 0;
            for (const id of ids) {
              if (!current.includes(id)) {
                current.push(id);
                added += 1;
              }
            }
            playlistTracks.set(playlistId, current);
            return { ok: true, data: { added, skipped: ids.length - added } };
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

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(2);
  await page.evaluate(() => {
    const checkbox = document.querySelector('#libraryTableBody .track-grid-row input[type="checkbox"]');
    if (!checkbox) return;
    checkbox.checked = true;
    checkbox.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await expect(page.locator("#selectionCount")).toContainText("1 selected");

  await page.evaluate(() => {
    document.getElementById("addSelectedBtn")?.click();
  });
  await expect(page.locator("#statusText")).toContainText("Added 1");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(1);
  await expect(page.locator("#playlistTracksBody")).toContainText("Sel One");
});

test("Selection state stays correct when filtering visible rows", async ({ page }) => {
  await installSelectionMock(page);
  await page.goto("/");

  await page.locator("#addPlaylistBtn").click();
  await page.locator("#navPlaylistList .nav-new-input").fill("Filter Add");
  await page.locator("#navPlaylistList .nav-new-input").press("Enter");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(2);
  await page.evaluate(() => {
    const checkboxes = Array.from(document.querySelectorAll('#libraryTableBody input[type="checkbox"]'));
    for (const checkbox of checkboxes) {
      checkbox.checked = true;
      checkbox.dispatchEvent(new Event("change", { bubbles: true }));
    }
  });
  await expect(page.locator("#selectionCount")).toContainText("2 selected");

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
