import { test, expect } from "./coverage-fixture.mjs";

function installBasicTauriMock(page, opts = {}) {
  return page.addInitScript(({ opts }) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const playlists = [];
    const failRename = !!opts?.failRename;
    let nextPlaylistId = 1;
    // Playlist track loads still in flight (see `playlistTracksDelayMs`).
    window.__pendingTrackLoads = 0;
    window.__calls = [];
    const listeners = new Map();
    // Lets a test push backend events (job:event) at the app.
    window.__emitEvent = (name, payload) => {
      for (const cb of listeners.get(name) || []) cb({ event: name, payload });
    };

    if (opts?.updateCheck) {
      // The update check only runs under the Tauri runtime (isTauriRuntime()
      // reads window.isTauri); the bundled @tauri-apps/api then invokes via
      // __TAURI_INTERNALS__, so forward that to the mock below.
      window.isTauri = true;
      window.__TAURI_INTERNALS__ = {
        invoke: (cmd, args) => window.__TAURI__.core.invoke(cmd, args)
      };
    }

    window.__TAURI__ = {
      event: {
        listen: async (name, cb) => {
          listeners.set(name, [...(listeners.get(name) || []), cb]);
          return () => listeners.set(name, (listeners.get(name) || []).filter((fn) => fn !== cb));
        }
      },
      core: {
        invoke: async (command, payload = {}) => {
          window.__calls.push({ command, payload });
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_rekordbox_db") {
            return { ok: true, data: { found: false, path: null } };
          }
          if (command === "list_playlists") {
            return { ok: true, data: { items: playlists } };
          }
          if (command === "create_playlist") {
            const name = payload?.request?.name || "Untitled";
            const playlistId = `pl-${nextPlaylistId++}`;
            // Like the backend: new playlists go to the top of the sidebar order.
            playlists.unshift({
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
          if (command === "reorder_playlists") {
            const ids = payload?.request?.orderedPlaylistIds || [];
            const byId = new Map(playlists.map((p) => [p.id, p]));
            playlists.splice(0, playlists.length, ...ids.map((id) => byId.get(id)));
            return { ok: true, data: { reordered: ids.length } };
          }
          if (command === "set_frontend_setting") {
            return { ok: true, data: { saved: true } };
          }
          if (command === "get_playlist_tracks") {
            const data = { playlistId: payload?.request?.playlistId || "", items: [] };
            if (!opts?.playlistTracksDelayMs) return { ok: true, data };
            window.__pendingTrackLoads += 1;
            await new Promise((resolve) => setTimeout(resolve, opts.playlistTracksDelayMs));
            window.__pendingTrackLoads -= 1;
            return { ok: true, data };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          if (command === "check_for_update" && opts?.updateCheck) {
            // `nextUpdateChecks` (set by a test) answers the later re-checks.
            const next = window.__nextUpdateChecks?.shift();
            return { ok: true, data: next || opts.updateCheck };
          }
          if (command === "install_update") {
            const job = { jobId: "job-update-1", jobType: "update", stage: "install_update", current: 0, total: 1 };
            window.__emitEvent("job:event", { ...job, event: "job.started", percent: 0, message: "Checking for update..." });
            window.__emitEvent("job:event", { ...job, event: "job.progress", percent: 40, message: "Downloading update 0.3.0..." });
            if (opts?.installFails) {
              const message = "Update failed: signature mismatch (mock)";
              window.__emitEvent("job:event", { ...job, event: "job.failed", percent: 100, message });
              return { ok: false, error: { code: "INTERNAL_ERROR", message } };
            }
            // A real install restarts the app; the call never comes back.
            return new Promise(() => {});
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
  // No DJ library found: no "Libraries" row.
  await expect(page.locator("#libraryChipsRow")).toHaveClass(/hidden/);
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

test("draws the backend's source folders as source chips", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.__TAURI__ = {
      core: {
        invoke: async (command) => {
          if (command === "get_library_sources") {
            const folders = ["/music", "/media/library"]
              .map((path) => ({ path, enabled: true, missing: false, fullyAnalyzed: false }));
            return { ok: true, data: { folders, rekordboxDbEnabled: false, mixxxDbEnabled: false, everConfigured: true } };
          }
          if (command === "list_playlists") return { ok: true, data: { items: [] } };
          if (command === "browse_source_files") return { ok: true, data: { total: 0, items: [] } };
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      },
      event: { listen: async () => () => {} }
    };
  });
  await page.goto("/");

  await expect(page.locator("#sourceChipsContainer .source-chip")).toHaveCount(2);
  await expect(page.locator("#sourceChipsContainer")).toContainText("/music");
  await expect(page.locator("#sourceChipsContainer")).toContainText("/media/library");
});

test("sidebar playlist rename updates sidebar and badge", async ({ page }) => {
  await installBasicTauriMock(page, { playlistTracksDelayMs: 150 });
  await page.goto("/");

  await page.locator("#addPlaylistBtn").click();
  await page.locator("#navPlaylistList .nav-new-input").fill("Rename Me");
  await page.locator("#navPlaylistList .nav-new-input").press("Enter");

  const item = page.locator("#navPlaylistList .nav-playlist-item").first();
  await expect(item).toBeVisible();
  await expect(item).toContainText("Rename Me");
  // A real double-click: its clicks switch to the playlist, whose (slow)
  // track load re-renders the sidebar; the rename field must survive that.
  await item.locator(".nav-playlist-name").dblclick();
  await page.waitForFunction(() => window.__pendingTrackLoads === 0);
  const renameInput = page.locator("#navPlaylistList .nav-rename-input");
  await expect(renameInput).toBeVisible();
  await renameInput.fill("Renamed Playlist");
  await renameInput.press("Enter");

  await expect(item).toContainText("Renamed Playlist");
  await expect(page.locator("#badgeLabel")).toHaveText("Renamed Playlist");
});

test("sidebar playlist rename failure keeps original name and sets status", async ({ page }) => {
  await installBasicTauriMock(page, { failRename: true, playlistTracksDelayMs: 150 });
  await page.goto("/");

  await page.locator("#addPlaylistBtn").click();
  await page.locator("#navPlaylistList .nav-new-input").fill("Rename Fail");
  await page.locator("#navPlaylistList .nav-new-input").press("Enter");

  const item = page.locator("#navPlaylistList .nav-playlist-item").first();
  await expect(item).toBeVisible();
  await expect(item).toContainText("Rename Fail");
  // A real double-click: its clicks switch to the playlist, whose (slow)
  // track load re-renders the sidebar; the rename field must survive that.
  await item.locator(".nav-playlist-name").dblclick();
  await page.waitForFunction(() => window.__pendingTrackLoads === 0);
  const renameInput = page.locator("#navPlaylistList .nav-rename-input");
  await expect(renameInput).toBeVisible();
  await renameInput.fill("Should Not Save");
  await renameInput.press("Enter");

  await expect(item).toContainText("Rename Fail");
  await expect(page.locator("#statusText")).toContainText("Rename failed");
});

async function createSidebarPlaylists(page, names) {
  for (const name of names) {
    await page.locator("#addPlaylistBtn").click();
    await page.locator("#navPlaylistList .nav-new-input").fill(name);
    await page.locator("#navPlaylistList .nav-new-input").press("Enter");
    await expect(page.locator("#navPlaylistList .nav-playlist-name").first()).toHaveText(name);
  }
}

test("sidebar playlist rename via the hover pencil", async ({ page }) => {
  await installBasicTauriMock(page);
  await page.goto("/");
  await createSidebarPlaylists(page, ["Pencil Me"]);

  const item = page.locator("#navPlaylistList .nav-playlist-item").first();
  const pencil = item.locator("[data-rename-playlist]");
  await expect(pencil).toHaveCSS("opacity", "0");
  await item.hover();
  await expect(pencil).toHaveCSS("opacity", "1");
  await pencil.click();

  const renameInput = page.locator("#navPlaylistList .nav-rename-input");
  await expect(renameInput).toBeFocused();
  await renameInput.fill("Penciled");
  await renameInput.press("Enter");
  await expect(item).toContainText("Penciled");
});

test("sidebar playlists reorder by dragging and new ones go on top", async ({ page }) => {
  await installBasicTauriMock(page);
  await page.goto("/");
  await createSidebarPlaylists(page, ["Alpha", "Bravo", "Charlie"]);

  const names = page.locator("#navPlaylistList .nav-playlist-name");
  await expect(names).toHaveText(["Charlie", "Bravo", "Alpha"]);

  const items = page.locator("#navPlaylistList li[data-nav-playlist-li]");
  await items.nth(2).hover();
  await items.nth(2).locator("[data-nav-playlist-drag-handle]").dragTo(items.nth(0), {
    targetPosition: { x: 20, y: 2 }
  });
  await expect(names).toHaveText(["Alpha", "Charlie", "Bravo"]);

  const reorderCalls = await page.evaluate(() =>
    window.__calls.filter((c) => c.command === "reorder_playlists").map((c) => c.payload.request)
  );
  expect(reorderCalls).toEqual([{ orderedPlaylistIds: ["pl-1", "pl-3", "pl-2"] }]);

  await createSidebarPlaylists(page, ["Delta"]);
  await expect(names).toHaveText(["Delta", "Alpha", "Charlie", "Bravo"]);
});

test("sidebar resizes by dragging its edge, saves the width, and double-click resets it", async ({ page }) => {
  await installBasicTauriMock(page);
  await page.goto("/");

  const sidebar = page.locator("#navSidebar");
  const resizer = page.locator("#navSidebarResizer");
  const width = async () => (await sidebar.boundingBox()).width;
  expect(await width()).toBe(220);

  const box = await resizer.boundingBox();
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + box.width / 2, y);
  await page.mouse.down();
  await page.mouse.move(box.x + 100, y, { steps: 5 });
  await page.mouse.up();
  const dragged = await width();
  expect(dragged).toBeGreaterThan(300);
  expect(dragged).toBeLessThan(330);

  // Dragging far past the limit stops at the maximum.
  const box2 = await resizer.boundingBox();
  await page.mouse.move(box2.x + box2.width / 2, y);
  await page.mouse.down();
  await page.mouse.move(box2.x + 2000, y, { steps: 5 });
  await page.mouse.up();
  expect(await width()).toBe(480);

  const saved = await page.evaluate(() =>
    window.__calls
      .filter((c) => c.command === "set_frontend_setting" && c.payload.request.key === "ui_sidebar_width_v1")
      .map((c) => c.payload.request.value)
  );
  expect(saved.at(-1)).toBe("480");
  expect(await page.evaluate(() => localStorage.getItem("djusbtkit.sidebarWidth"))).toBe("480");

  await resizer.dblclick();
  // The reset animates (only live dragging skips the width transition).
  await expect.poll(width).toBe(220);
});

test("sidebar width is restored on startup", async ({ page }) => {
  await installBasicTauriMock(page);
  await page.addInitScript(() => window.localStorage.setItem("djusbtkit.sidebarWidth", "300"));
  await page.goto("/");
  await expect.poll(async () => (await page.locator("#navSidebar").boundingBox()).width).toBe(300);
});

const updateCheck = (severity, extra = {}) => ({
  updateAvailable: true,
  severity,
  currentVersion: "0.2.4",
  latestVersion: "0.3.0",
  releaseUrl: "https://example.test/v0.3.0",
  installKind: "unknown",
  downloadUrl: null,
  canSelfUpdate: false,
  action: "none",
  checkFailed: false,
  ...extra,
});

const selfUpdatable = (severity) => updateCheck(severity, {
  installKind: "nsis",
  downloadUrl: "https://example.test/dl/DJ_USB_Tkit_0.3.0_x64-setup.exe",
  canSelfUpdate: true,
  action: "install",
});

test("a feature release shows the new-features banner; dismissing it sticks for that version", async ({ page }) => {
  await installBasicTauriMock(page, { updateCheck: updateCheck("feature") });
  await page.goto("/");

  const banner = page.locator("#updateBanner");
  await expect(banner).toBeVisible();
  await expect(banner).toHaveClass(/is-feature/);
  await expect(banner).not.toHaveClass(/is-critical/);
  await expect(banner).toHaveAttribute("role", "status");
  await expect(page.locator("#updateBannerText")).toHaveText("New features available: 0.3.0 — view release");

  await page.locator("#updateBannerDismissBtn").click();
  await expect(banner).toBeHidden();
  await page.reload();
  await expect(page.locator("#panel-library")).toHaveClass(/active/);
  await expect(banner).toBeHidden();
});

test("a critical release shows the critical banner", async ({ page }) => {
  await installBasicTauriMock(page, { updateCheck: updateCheck("critical") });
  await page.goto("/");

  const banner = page.locator("#updateBanner");
  await expect(banner).toBeVisible();
  await expect(banner).toHaveClass(/is-critical/);
  await expect(banner).toHaveAttribute("role", "alert");
  await expect(page.locator("#updateBannerText")).toHaveText("Critical update available: 0.3.0 — view release");
});

test("a normal release shows no banner", async ({ page }) => {
  await installBasicTauriMock(page, { updateCheck: updateCheck("normal") });
  await page.goto("/");

  // The check has landed (the settings update line and the dot show it)…
  await expect(page.locator("#settingsUpdateStatus")).toHaveText("Version 0.3.0 is available.");
  await expect(page.locator("#settingsUpdateDot")).toBeVisible();
  // …and a routine release stays out of the way.
  await expect(page.locator("#updateBanner")).toBeHidden();
});

test("a package-manager install gets a direct download link, not the in-app updater", async ({ page }) => {
  const debUrl = "https://example.test/dl/DJ_USB_Tkit_0.3.0_amd64.deb";
  await installBasicTauriMock(page, {
    updateCheck: updateCheck("feature", { installKind: "deb", downloadUrl: debUrl, action: "download" })
  });
  await page.goto("/");

  const bannerActions = page.locator("#updateBannerActions");
  await expect(page.locator("#updateBanner")).toBeVisible();
  await expect(bannerActions.locator(".update-download-link")).toBeVisible();
  await expect(bannerActions.locator(".update-install-btn")).toBeHidden();
  await expect(page.locator("#settingsUpdateActions .update-install-btn")).toBeHidden();

  await bannerActions.locator(".update-download-link").click();
  await expect
    .poll(() => page.evaluate(() => window.__calls.find((c) => c.command === "plugin:opener|open_url")?.payload?.url))
    .toBe(debUrl);
});

test("Update & restart runs the update as a job on the footer progress bar", async ({ page }) => {
  await installBasicTauriMock(page, { updateCheck: selfUpdatable("critical") });
  await page.goto("/");

  const install = page.locator("#updateBannerActions .update-install-btn");
  await expect(install).toBeVisible();
  await install.click();

  await expect(page.locator("#updateBanner")).toBeHidden();
  await expect(page.locator("#progressFooter")).toHaveClass(/active/);
  await expect(page.locator("#progressText")).toContainText("Downloading update 0.3.0...");
  await expect(page.locator("#progressFill")).toHaveAttribute("style", /width: 40%/);
  // No second install while the first one runs.
  await expect(page.locator("#settingsUpdateActions .update-install-btn")).toBeDisabled();
});

test("a failed update shows the reason in the footer and can be retried", async ({ page }) => {
  await installBasicTauriMock(page, { updateCheck: selfUpdatable("feature"), installFails: true });
  await page.goto("/");

  await page.locator("#updateBannerActions .update-install-btn").click();

  await expect(page.locator("#progressFooter")).toHaveClass(/active/);
  await expect(page.locator("#progressText")).toContainText("Update failed: signature mismatch (mock)");
  await expect(page.locator("#settingsUpdateActions .update-install-btn")).toBeEnabled();
  // The banner comes back with the retry button.
  await expect(page.locator("#updateBanner")).toBeVisible();
  await expect(page.locator("#updateBannerActions .update-install-btn")).toBeEnabled();
  // The direct download stays on offer as the fallback (settings drawer is closed here).
  await expect(page.locator("#settingsUpdateActions .update-download-link")).not.toHaveClass(/hidden/);
});

test("Update & restart waits for a running job instead of cutting it short", async ({ page }) => {
  await installBasicTauriMock(page, { updateCheck: selfUpdatable("feature") });
  await page.goto("/");
  await expect(page.locator("#updateBannerActions .update-install-btn")).toBeVisible();

  await page.evaluate(() => window.__emitEvent("job:event", {
    event: "job.started", jobId: "job-export-1", jobType: "export", stage: "export_to_usb",
    current: 0, total: 1, percent: 0, message: "Exporting..."
  }));
  await expect(page.locator("#progressText")).toContainText("Exporting...");

  await page.locator("#updateBannerActions .update-install-btn").click();
  await expect(page.locator("#statusText")).toContainText("Finish the running job before updating.");
  expect(await page.evaluate(() => window.__calls.some((c) => c.command === "install_update"))).toBe(false);
});

test("settings update line offers one action, and the header button re-checks on demand", async ({ page }) => {
  await installBasicTauriMock(page, { updateCheck: selfUpdatable("normal") });
  await page.goto("/");
  await expect(page.locator("#settingsUpdateDot")).toBeVisible();
  await expect(page.locator("#settingsBtn")).toHaveAttribute("aria-label", "Settings (update available)");

  await page.locator("#settingsBtn").click();
  const line = page.locator("#settingsUpdateSection");
  const checkBtn = page.locator(".settings-header #settingsUpdateCheckBtn");
  // The update line is the first thing in the body.
  await expect(page.locator(".settings-body > :first-child")).toHaveId("settingsUpdateSection");
  await expect(line).toBeVisible();
  await expect(line.locator("#settingsUpdateStatus")).toHaveText("Version 0.3.0 is available.");
  await expect(line.locator("#settingsUpdateReleaseLink")).toBeVisible();
  // A self-updating install gets the in-app update only, not the download too.
  await expect(line.locator(".update-install-btn")).toBeVisible();
  await expect(line.locator(".update-download-link")).toBeHidden();

  // A re-check that can't reach GitHub says so instead of "up to date"…
  const noUpdate = (checkFailed) => ({
    updateAvailable: false, severity: "none", currentVersion: "0.3.0", latestVersion: "0.3.0",
    releaseUrl: "https://example.test", installKind: "nsis", downloadUrl: null, canSelfUpdate: false,
    action: "none", checkFailed
  });
  await page.evaluate((checks) => { window.__nextUpdateChecks = checks; }, [noUpdate(true), noUpdate(false)]);
  await checkBtn.click();
  await expect(line.locator("#settingsUpdateStatus")).toHaveText("Couldn't check for updates.");
  await expect(line.locator("#settingsUpdateReleaseLink")).toBeHidden();

  // …and a manual re-check answers "Up to date" until the drawer closes.
  await checkBtn.click();
  await expect(line.locator("#settingsUpdateStatus")).toHaveText("Up to date.");
  await expect(line.locator(".update-install-btn")).toBeHidden();
  await expect(page.locator("#settingsUpdateDot")).toBeHidden();
  await page.locator("#settingsCloseBtn").click();
  await page.locator("#settingsBtn").click();
  await expect(line).toBeHidden();
  await expect(checkBtn).toBeVisible();
});
