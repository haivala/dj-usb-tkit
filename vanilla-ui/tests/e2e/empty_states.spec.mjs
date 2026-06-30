import { test, expect } from "@playwright/test";

function installEmptyStateMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const playlists = [];

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") return { ok: true, data: { found: false, path: null } };
          if (command === "list_playlists") return { ok: true, data: { items: playlists } };
          if (command === "list_tracks" || command === "search_tracks" || command === "browse_source_files") {
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
});

test("Configured source folders with zero indexed tracks prompt scan and keep controls visible", async ({ page }) => {
  await installEmptyStateMock(page);
  await page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
  });
  await page.goto("/");

  await expect(page.locator("#libraryEmptyState")).toBeEmpty();
  await expect(page.locator("#libraryContent")).not.toHaveClass(/hidden/);
  await expect(page.locator("#sourceChipsContainer")).toContainText("/music");
  await expect(page.locator("#libraryTableBody")).toContainText("No tracks available.");
});
