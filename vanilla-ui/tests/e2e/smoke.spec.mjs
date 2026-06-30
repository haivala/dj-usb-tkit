import { test, expect } from "@playwright/test";

function installBasicTauriMock(page, opts = {}) {
  return page.addInitScript(({ opts }) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const playlists = [];
    const failRename = !!opts?.failRename;

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
            return { ok: true, data: { items: playlists } };
          }
          if (command === "search_tracks" || command === "list_tracks") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "create_playlist") {
            const name = payload?.request?.name || "Untitled";
            const playlistId = `pl-${Date.now()}`;
            playlists.push({
              id: playlistId,
              name,
              source: "local",
              lastExportedAt: null,
              lastExportedUsbRoot: null,
              lastExportedTrackCount: null,
              tracks: [],
              createdAt: new Date().toISOString(),
              updatedAt: new Date().toISOString()
            });
            return { ok: true, data: { playlistId, name } };
          }
          if (command === "delete_playlist") {
            const playlistId = payload?.request?.playlistId || "";
            const idx = playlists.findIndex((p) => p.id === playlistId);
            if (idx >= 0) playlists.splice(idx, 1);
            return { ok: true, data: { playlistId, deleted: idx >= 0 } };
          }
          if (command === "rename_playlist") {
            const playlistId = payload?.request?.playlistId || "";
            const name = String(payload?.request?.name || "").trim();
            if (failRename) {
              return { ok: false, error: { code: "INTERNAL", message: "rename failed (mock)" } };
            }
            const row = playlists.find((p) => p.id === playlistId);
            if (!row || !name) {
              return { ok: false, error: { code: "VALIDATION", message: "rename failed" } };
            }
            row.name = name;
            return { ok: true, data: { playlistId, name } };
          }
          if (command === "get_playlist_tracks") {
            return { ok: true, data: { playlistId: payload?.request?.playlistId || "", items: [] } };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  }, { opts });
}

test("loads shell with library active and sidebar nav", async ({ page }) => {
  await installBasicTauriMock(page);
  await page.goto("/");

  await expect(page.getByRole("heading", { name: "DJ USB Tkit" })).toBeVisible();
  await expect(page.locator("#panel-library")).toHaveClass(/active/);
  await expect(page.locator("#panel-usb")).not.toHaveClass(/active/);
  await expect(page.locator('.nav-item[data-view="library"]')).toHaveAttribute("aria-current", "true");
  await expect(page.locator('.nav-item[data-view="usb"]')).not.toHaveAttribute("aria-current", "true");
});

test("can create and delete playlist via sidebar", async ({ page }) => {
  await installBasicTauriMock(page);
  await page.goto("/");

  await page.locator("#addPlaylistBtn").click();

  const nameInput = page.locator("#navPlaylistList .nav-new-input");
  await expect(nameInput).toBeVisible();
  await nameInput.fill("Smoke Playlist");
  await nameInput.press("Enter");

  const playlistItem = page.locator("#navPlaylistList .nav-playlist-item").first();
  await expect(playlistItem).toBeVisible();
  await expect(playlistItem).toContainText("Smoke Playlist");
  await expect(page.locator("#badgeLabel")).toHaveText("Smoke Playlist");

  // Delete the playlist
  await playlistItem.locator("[data-delete-playlist]").click();
  const confirmOverlay = page.locator("#confirmOverlay");
  await expect(confirmOverlay).toBeVisible();
  await page.locator("#confirmOkBtn").click();

  await expect(page.locator("#navPlaylistList .nav-playlist-item")).toHaveCount(0);
  await expect(page.locator("#statusText")).toContainText("Playlist deleted");
  await expect(page.locator("#statusText")).toContainText("Playlist deleted");
});

test("usb panel shows when clicking USB nav item", async ({ page }) => {
  await installBasicTauriMock(page);
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await expect(page.locator("#panel-usb")).toHaveClass(/active/);
  await expect(page.locator("#panel-library")).not.toHaveClass(/active/);
});

test("restores source roots from localStorage as source chips", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem(
      "djusbtkit.sourceRoots",
      JSON.stringify(["/music", "/media/library"])
    );
  });
  await page.goto("/");

  await expect(page.locator("#sourceChipsContainer .source-chip")).toHaveCount(2);
  await expect(page.locator("#sourceChipsContainer")).toContainText("/music");
  await expect(page.locator("#sourceChipsContainer")).toContainText("/media/library");
});

test("sidebar playlist rename updates sidebar and badge", async ({ page }) => {
  await installBasicTauriMock(page);
  await page.goto("/");

  await page.locator("#addPlaylistBtn").click();
  await page.locator("#navPlaylistList .nav-new-input").fill("Rename Me");
  await page.locator("#navPlaylistList .nav-new-input").press("Enter");

  const item = page.locator("#navPlaylistList .nav-playlist-item").first();
  await expect(item).toBeVisible();
  await expect(item).toContainText("Rename Me");
  await page.waitForFunction(() => {
    const input = document.querySelector("#navPlaylistList .nav-rename-input");
    if (input) return true;
    const itemEl = document.querySelector("#navPlaylistList .nav-playlist-item");
    if (!itemEl) return false;
    itemEl.dispatchEvent(new MouseEvent("dblclick", {
      bubbles: true,
      cancelable: true,
      composed: true,
      detail: 2
    }));
    return !!document.querySelector("#navPlaylistList .nav-rename-input");
  });
  const renameInput = page.locator("#navPlaylistList .nav-rename-input");
  await expect(renameInput).toBeVisible();
  await renameInput.fill("Renamed Playlist");
  await renameInput.press("Enter");

  await expect(item).toContainText("Renamed Playlist");
  await expect(page.locator("#badgeLabel")).toHaveText("Renamed Playlist");
});

test("sidebar playlist rename failure keeps original name and sets status", async ({ page }) => {
  await installBasicTauriMock(page, { failRename: true });
  await page.goto("/");

  await page.locator("#addPlaylistBtn").click();
  await page.locator("#navPlaylistList .nav-new-input").fill("Rename Fail");
  await page.locator("#navPlaylistList .nav-new-input").press("Enter");

  const item = page.locator("#navPlaylistList .nav-playlist-item").first();
  await expect(item).toBeVisible();
  await expect(item).toContainText("Rename Fail");
  await page.waitForFunction(() => {
    const input = document.querySelector("#navPlaylistList .nav-rename-input");
    if (input) return true;
    const itemEl = document.querySelector("#navPlaylistList .nav-playlist-item");
    if (!itemEl) return false;
    itemEl.dispatchEvent(new MouseEvent("dblclick", {
      bubbles: true,
      cancelable: true,
      composed: true,
      detail: 2
    }));
    return !!document.querySelector("#navPlaylistList .nav-rename-input");
  });
  const renameInput = page.locator("#navPlaylistList .nav-rename-input");
  await expect(renameInput).toBeVisible();
  await renameInput.fill("Should Not Save");
  await renameInput.press("Enter");

  await expect(item).toContainText("Rename Fail");
  await expect(page.locator("#statusText")).toContainText("Rename failed");
});
