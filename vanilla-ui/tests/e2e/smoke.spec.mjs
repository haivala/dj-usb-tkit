import { test, expect, offerTestLanguage, uiTextCheck, useTestLanguage } from "./coverage-fixture.mjs";

function installBasicTauriMock(page, opts = {}) {
  return page.addInitScript(({ opts }) => {
    const backend = window.__e2eBackend;
    // Playlist track loads still in flight (see `playlistTracksDelayMs`).
    window.__pendingTrackLoads = 0;
    // The update check only runs under the Tauri runtime.
    if (opts?.updateCheck) backend.useTauriRuntime();
    backend.install(async (command, request, payload) => {
      if (command === "rename_playlist" && opts?.failRename) return backend.fail("Rename failed: disk full (mock)");
      if (command === "get_playlist_tracks" && opts?.playlistTracksDelayMs) {
        window.__pendingTrackLoads += 1;
        await new Promise((resolve) => setTimeout(resolve, opts.playlistTracksDelayMs));
        window.__pendingTrackLoads -= 1;
        return backend.answer(command, payload);
      }
      if (command === "check_for_update" && opts?.updateCheck) {
        // `nextUpdateChecks` (set by a test) answers the later re-checks.
        return backend.ok(window.__nextUpdateChecks?.shift() || opts.updateCheck);
      }
      if (command === "install_update") {
        const job = { jobId: "job-update-1", jobType: "update", stage: "install_update", current: 0, total: 1 };
        window.__emitJobEvent({ ...job, event: "job.started", percent: 0, message: "Checking for update..." });
        window.__emitJobEvent({ ...job, event: "job.progress", percent: 40, message: "Downloading update 0.3.0..." });
        if (opts?.installFails) {
          const message = "Update failed: signature mismatch (mock)";
          window.__emitJobEvent({ ...job, event: "job.failed", percent: 100, message });
          return backend.fail(message, "INTERNAL_ERROR");
        }
        // A real install restarts the app; the call never comes back.
        return new Promise(() => {});
      }
    });
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
  // Sidebar icon clicks suppress mousedown focus; establish the focus to restore.
  await playlistItem.locator("[data-delete-playlist]").focus();
  await playlistItem.locator("[data-delete-playlist]").click();
  const confirmOverlay = page.locator("#confirmOverlay");
  await expect(confirmOverlay).toBeVisible();
  for (const dismissal of ["Escape", "backdrop", "Cancel"]) {
    await expect(page.locator("#confirmOkBtn")).toBeFocused();
    if (dismissal === "Escape") await page.keyboard.press("Escape");
    else if (dismissal === "backdrop") await confirmOverlay.click({ position: { x: 5, y: 5 } });
    else await page.locator("#confirmCancelBtn").click();
    await expect(confirmOverlay).toBeHidden();
    await expect(playlistItem.locator("[data-delete-playlist]")).toBeFocused();
    await expect(playlistItem).toBeVisible();
    expect(await page.evaluate(() => window.__calls.filter((c) => c.command === "delete_playlist"))).toHaveLength(0);
    await playlistItem.locator("[data-delete-playlist]").click();
  }
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
    window.__e2eSourceFolders = ["/music", "/media/library"];
    window.__e2eBackend.install();
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
  // The backend's texts for the update line and the banner.
  statusText: "Version 0.3.0 is available.",
  bannerText: { critical: "Critical update available: 0.3.0", feature: "New features available: 0.3.0" }[severity] || "",
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

  await page.evaluate(() => window.__emitJobEvent({
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
    action: "none", checkFailed,
    statusText: checkFailed ? "Couldn't check for updates." : "Up to date.", bannerText: "",
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

// The mock keeps the saved language in sessionStorage, so it outlives the reload, and refuses
// the change while `e2eJobRunning` is set, as the backend does while a job runs.
test("changing the language reloads the window in it, and a refused change keeps the page", async ({ page }) => {
  await offerTestLanguage(page);
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    backend.install((command, request) => {
      if (command === "get_ui_texts") {
        const testLanguage = window.sessionStorage.getItem("e2eLanguage") === "en-XA";
        return backend.ok(testLanguage ? window.__e2eUiTextsTestLanguage : window.__e2eUiTexts);
      }
      if (command === "set_language") {
        if (window.sessionStorage.getItem("e2eJobRunning")) {
          return backend.fail("Could not change the language: a job is running. Try again when it has finished.", "VALIDATION_ERROR");
        }
        window.sessionStorage.setItem("e2eLanguage", request.language);
        return backend.ok({ language: request.language });
      }
    });
  });
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await page.locator("#settingsBtn").click();
  const select = page.locator("#languageSelect");
  await expect(select).toHaveValue("en");
  // Each language named in itself, as get_ui_texts lists them.
  await expect(select.locator("option")).toHaveText(["English", /^\[.+\]$/]);

  // Refused while a job runs: the page stays, in English.
  await page.evaluate(() => window.sessionStorage.setItem("e2eJobRunning", "1"));
  await select.selectOption("en-XA");
  await expect(page.locator("#statusText")).toHaveText(
    "Could not change the language: a job is running. Try again when it has finished."
  );
  await expect(select).toHaveValue("en");
  await expect(page.locator("html")).toHaveAttribute("lang", "en");

  // Saved: the window reloads in the test language.
  await page.evaluate(() => window.sessionStorage.removeItem("e2eJobRunning"));
  await select.selectOption("en-XA");
  await expect(page.locator("html")).toHaveAttribute("lang", "en-XA");
  await page.locator("#settingsBtn").click();
  await expect(page.locator("#languageSelect")).toHaveValue("en-XA");
  await expect(page.locator('label[for="languageSelect"] span')).toHaveText(/^\[.+\]$/);
});

test("in the test language, the first-run Library, Help and the update banner and card show only catalog text and data", async ({ page }) => {
  await installBasicTauriMock(page, { updateCheck: selfUpdatable("feature") });
  await useTestLanguage(page);
  await page.goto("/");
  const check = uiTextCheck(page);
  // No library folders yet: the Library's first-run empty state, and the update banner.
  await expect(page.locator("#libraryEmptyState")).not.toBeEmpty();
  await expect(page.locator("#updateBanner")).toBeVisible();
  await check.collect();

  await page.locator("#helpBtn").click();
  await expect(page.locator("#helpOverlay")).toBeVisible();
  await check.collect();
  await page.locator("#helpCloseBtn").click();
  await expect(page.locator("#helpOverlay")).toBeHidden();

  await page.locator("#settingsBtn").click();
  await expect(page.locator("#settingsUpdateSection")).toBeVisible();
  await check.collect();

  await check.expectNoEnglish();
});
