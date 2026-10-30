import { test, expect } from "./coverage-fixture.mjs";

function installEmptyStateMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const playlists = [];

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "get_library_sources") {
            const folders = (window.__e2eSourceFolders || [])
              .map((path) => ({ path, label: path, parent: null, enabled: true, missing: false, fullyAnalyzed: false }));
            return { ok: true, data: { folders, rekordboxDbEnabled: false, mixxxDbEnabled: false, everConfigured: folders.length > 0 } };
          }
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_rekordbox_db") return { ok: true, data: { found: false, path: null } };
          if (command === "list_playlists") return { ok: true, data: { items: playlists } };
          if (command === "browse_source_files") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "create_playlist") {
            const name = String(payload?.request?.name || "Untitled");
            const id = `pl-${Date.now()}`;
            playlists.push({
              id, name, source: "local",
              lastExportedAt: null, lastExportedUsbRoot: null, lastExportedTrackCount: null,
              tracks: []
            });
            return { ok: true, data: { playlistId: id, name } };
          }
          if (command === "get_playlist_tracks") {
            const playlistId = String(payload?.request?.playlistId || "");
            return { ok: true, data: { playlistId, items: [] } };
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

test("Empty states render correctly", async ({ page }) => {
  await installEmptyStateMock(page);
  await page.goto("/");

  await expect(page.locator("#libraryEmptyState")).toContainText("Add a music folder to get started");
  await expect(page.getByRole("button", { name: "Add Folder" })).toBeVisible();

  await page.locator('.nav-item[data-view="usb"]').click();
  await expect(page.locator("#usbEmptyState")).toContainText("Select USB Folder");

  await page.locator("#addPlaylistBtn").click();
  await page.locator("#navPlaylistList .nav-new-input").fill("Empty Playlist");
  await page.locator("#navPlaylistList .nav-new-input").press("Enter");
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistEmptyState")).toContainText("Browse Library or USB to add tracks");
  await expect(page.locator("#playlistExportStatus")).toBeHidden();
  await page.locator("#playlistEmptyState").getByRole("button", { name: "USB", exact: true }).click();
  await expect(page.locator('.nav-item[data-view="usb"]')).toHaveClass(/active/);
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await page.locator("#playlistEmptyState").getByRole("button", { name: "Media Library" }).click();
  await expect(page.locator('.nav-item[data-view="library"]')).toHaveClass(/active/);
});

test("Configured source folders with zero indexed tracks prompt scan and keep controls visible", async ({ page }) => {
  await installEmptyStateMock(page);
  await page.addInitScript(() => {
    // The library\'s source folders, as `get_library_sources` answers.
    window.__e2eSourceFolders = ["/music"];
  });
  await page.goto("/");

  await expect(page.locator("#libraryEmptyState")).toBeEmpty();
  await expect(page.locator("#libraryContent")).not.toHaveClass(/hidden/);
  await expect(page.locator("#sourceChipsContainer")).toContainText("/music");
  await expect(page.locator("#libraryTableBody")).toContainText("No tracks available.");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveClass(/track-grid-row-empty/);
});

test("A playlist with tracks shows loading, never the empty screen", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    const playlist = (id, name) => ({
      id, name, source: "local",
      lastExportedAt: null, lastExportedUsbRoot: null, lastExportedTrackCount: null,
      tracks: []
    });
    const tracks = [
      { id: "t1", title: "Loaded One", artist: "Artist", album: "Album", filePath: "/music/one.mp3" },
      { id: "t2", title: "Loaded Two", artist: "Artist", album: "Album", filePath: "/music/two.mp3" }
    ];
    // The full playlist's tracks arrive only when the test releases them.
    let release = null;
    window.__releaseFullPlaylist = () => release?.();

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "get_library_sources") {
            const folders = (window.__e2eSourceFolders || [])
              .map((path) => ({ path, label: path, parent: null, enabled: true, missing: false, fullyAnalyzed: false }));
            return { ok: true, data: { folders, rekordboxDbEnabled: false, mixxxDbEnabled: false, everConfigured: folders.length > 0 } };
          }
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_rekordbox_db") return { ok: true, data: { found: false, path: null } };
          if (command === "list_playlists") {
            return { ok: true, data: { items: [playlist("pl-empty", "Nothing Yet"), playlist("pl-full", "Has Tracks")] } };
          }
          if (command === "browse_source_files") return { ok: true, data: { total: 0, items: [] } };
          if (command === "get_playlist_tracks") {
            const playlistId = String(payload?.request?.playlistId || "");
            if (playlistId !== "pl-full") {
              return { ok: true, data: { playlistId, items: [], total: 0, listTrackCount: 0 } };
            }
            const query = String(payload?.request?.query || "");
            if (query) {
              // A search that matches nothing in a playlist that has tracks.
              return { ok: true, data: { playlistId, items: [], total: 0, listTrackCount: tracks.length } };
            }
            await new Promise((resolve) => { release = resolve; });
            return { ok: true, data: { playlistId, items: tracks, total: tracks.length, listTrackCount: tracks.length } };
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
  await page.goto("/");

  const items = page.locator("#navPlaylistList .nav-playlist-item");
  await items.filter({ hasText: "Nothing Yet" }).click();
  await expect(page.locator("#playlistEmptyState")).toContainText("Browse Library or USB to add tracks");

  // From an empty playlist to one with tracks: the empty screen goes at once
  // and the table shows it's loading.
  await items.filter({ hasText: "Has Tracks" }).click();
  await expect(page.locator("#playlistEmptyState")).toBeEmpty();
  await expect(page.locator("#playlistTableWrap")).toBeVisible();
  await expect(page.locator("#playlistTableWrap .track-grid-loading")).toBeVisible();

  await page.evaluate(() => window.__releaseFullPlaylist());
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);
  await expect(page.locator("#playlistTableWrap .track-grid-loading")).toBeHidden();

  // No search match isn't an empty playlist.
  await page.locator("#playlistSearchInput").fill("zzz");
  await expect(page.locator("#playlistTracksBody")).toContainText("No tracks available.");
  await expect(page.locator("#playlistEmptyState")).toBeEmpty();
  await expect(page.locator("#playlistSearchInput")).toBeVisible();
});
