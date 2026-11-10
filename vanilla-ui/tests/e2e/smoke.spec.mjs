import { test, expect, offerTestLanguage, uiTextCheck, useTestLanguage } from "./coverage-fixture.mjs";

function installBasicTauriMock(page, opts = {}) {
  return page.addInitScript(({ opts }) => {
    const backend = window.__e2eBackend;
    // Playlist track loads still in flight (see `slowTracks`).
    window.__pendingTrackLoads = 0;
    // Set by a test for the loads after a reload: playlist tracks answer
    // slowly, and a rename fails.
    const slowTracks = !!sessionStorage.getItem("slowTracks");
    const failRename = !!sessionStorage.getItem("failRename");
    // The update check's answer: the one a test's `showUpdate` set, else `opts`.
    const updateCheck = JSON.parse(sessionStorage.getItem("updateCheck") || "null") || opts?.updateCheck;
    const installFails = !!sessionStorage.getItem("installFails");
    // The update check only runs under the Tauri runtime.
    if (updateCheck) backend.useTauriRuntime();
    backend.install(async (command, request, payload) => {
      if (command === "rename_playlist" && failRename) return backend.fail("Rename failed: disk full (mock)");
      if (command === "get_playlist_tracks" && slowTracks) {
        window.__pendingTrackLoads += 1;
        await new Promise((resolve) => setTimeout(resolve, 150));
        window.__pendingTrackLoads -= 1;
        return backend.answer(command, payload);
      }
      if (command === "check_for_update" && updateCheck) {
        // `nextUpdateChecks` (set by a test) answers the later re-checks.
        return backend.ok(window.__nextUpdateChecks?.shift() || updateCheck);
      }
      if (command === "install_update") {
        const job = { jobId: "job-update-1", jobType: "update", stage: "install_update", current: 0, total: 1 };
        window.__emitJobEvent({ ...job, event: "job.started", percent: 0, message: "Checking for update..." });
        window.__emitJobEvent({ ...job, event: "job.progress", percent: 40, message: "Downloading update 0.3.0..." });
        if (installFails) {
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

test("shell and sidebar", async ({ page }) => {
  await installBasicTauriMock(page);
  await page.goto("/");
  // Loads shell with library active and sidebar nav.
  {
    await expect(page.getByRole("heading", { name: "DJ USB Tkit" })).toBeVisible();
    await expect(page.locator("#panel-library")).toHaveClass(/active/);
    await expect(page.locator("#panel-usb")).not.toHaveClass(/active/);
    await expect(page.locator('.nav-item[data-view="library"]')).toHaveAttribute("aria-current", "true");
    await expect(page.locator('.nav-item[data-view="usb"]')).not.toHaveAttribute("aria-current", "true");
    // No DJ library found: no "Libraries" row.
    await expect(page.locator("#libraryChipsRow")).toHaveClass(/hidden/);
  }

  await page.reload();
  // Can create and delete playlist via sidebar.
  {
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
  }

  await page.reload();
  // Usb panel shows when clicking USB nav item.
  {
    await page.locator('.nav-item[data-view="usb"]').click();
    await expect(page.locator("#panel-usb")).toHaveClass(/active/);
    await expect(page.locator("#panel-library")).not.toHaveClass(/active/);
  }

  await page.reload();
  // Sidebar playlist rename via the hover pencil.
  {
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
  }

  await page.reload();
  // Sidebar playlists reorder by dragging and new ones go on top.
  {
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
  }

  await page.reload();
  // Sidebar resizes by dragging its edge, saves the width, and double-click resets it.
  {
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
  }

  // The sidebar width is restored on startup.
  await page.evaluate(() => localStorage.setItem("djusbtkit.sidebarWidth", "300"));
  await page.reload();
  await expect.poll(async () => (await page.locator("#navSidebar").boundingBox()).width).toBe(300);

  // Rename from the sidebar updates the sidebar and the badge.
  await page.evaluate(() => sessionStorage.setItem("slowTracks", "1"));
  await page.reload();
  {
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
  }

  // A failed rename keeps the original name and says why.
  await page.evaluate(() => sessionStorage.setItem("failRename", "1"));
  await page.reload();
  {
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
  }
});

// Draws the backend's source folders as source chips, with its Analyze label; the filter mark shows
// while the backend says tracks are hidden.
test("source chips", async ({ page }) => {
  await page.addInitScript(() => {
    window.__e2eSourceFolders = ["/music", "/media/library"];
    const backend = window.__e2eBackend;
    backend.install((command, request) => {
      if (command === "get_library_sources") {
        const sources = backend.librarySources();
        sources.folders[0] = { ...sources.folders[0], fullyAnalyzed: true };
        if (sessionStorage.getItem("folderMissing")) {
          sources.folders[0] = { ...sources.folders[0], enabled: false };
          sources.folders[1] = { ...sources.folders[1], missing: true };
          sources.filtering = true;
        }
        return backend.ok(sources);
      }
      if (command === "pick_source_folders") return ["/media/moved"];
      if (command === "relocate_source_root") {
        return backend.ok({
          oldRoot: request.oldRoot, newRoot: request.newRoot, matched: 2, updated: 2, unchanged: 0, missingAtNewRoot: 0, conflicts: 0,
          sources: backend.librarySources(["/music", "/media/moved"]),
          statusText: "Source relocated: 2 track paths updated",
        });
      }
    });
  });
  await page.goto("/");

  await expect(page.locator("#sourceChipsContainer .source-chip")).toHaveCount(2);
  await expect(page.locator("#sourceChipsContainer")).toContainText("/music");
  await expect(page.locator("#sourceChipsContainer")).toContainText("/media/library");
  await expect(page.locator("#analyzeLibraryBtn")).toHaveText("Analyze Libraries");
  await expect(page.locator("#sourceFilterIndicator")).not.toHaveClass(/active/);
  await expect(page.locator("body")).not.toHaveClass(/library-onboarding/);

  await page.evaluate(() => sessionStorage.setItem("folderMissing", "1"));
  await page.reload();
  const chips = page.locator("#sourceChipsContainer .source-chip");
  await expect(chips).toHaveCount(2);
  await expect(page.locator("#sourceFilterIndicator")).toHaveClass(/active/);
  // The backend's folder facts: analyzed, switched off, missing.
  await expect(chips.nth(0)).toHaveClass(/source-chip-analyzed/);
  await expect(chips.nth(1)).not.toHaveClass(/source-chip-analyzed/);
  await expect(chips.nth(0).locator(".source-chip-toggle")).not.toBeChecked();
  const missing = chips.nth(1);
  await expect(missing).toHaveClass(/source-chip-missing/);
  await expect(missing.locator(".source-chip-toggle")).toBeDisabled();
  await expect(missing.locator(".source-chip-toggle")).not.toBeChecked();
  await expect(missing.locator(".source-chip-toggle")).toHaveAttribute("aria-label", "Source folder missing");
  await expect(missing.locator(".source-chip-path")).toHaveAttribute("data-tooltip", /Click to relocate/);

  // Clicking it relocates it: the backend's folders and status line, the library read again.
  await page.evaluate(() => { window.__calls.length = 0; });
  await missing.locator(".source-chip-path").click();
  await expect(page.locator("#statusText")).toContainText("Source relocated: 2 track paths updated");
  await expect(chips.nth(1).locator(".source-chip-path")).toHaveText("/media/moved");
  const calls = await page.evaluate(() => window.__calls.map((c) => ({ command: c.command, request: c.request })));
  expect(calls.find((c) => c.command === "relocate_source_root").request).toEqual({ oldRoot: "/media/library", newRoot: "/media/moved" });
  expect(calls.some((c) => c.command === "browse_source_files")).toBe(true);
});

test("tooltips show after a hover delay and on focus, Escape and leaving hide them, and they stay inside the window", async ({ page }) => {
  await page.addInitScript(() => window.__e2eBackend.install());
  await page.goto("/");
  await page.evaluate(() => {
    document.body.insertAdjacentHTML("beforeend",
      '<button id="tipTarget" data-tooltip="Full detail text" style="position:fixed;left:300px;top:300px">Hover me</button>' +
      '<span id="tipEdge" data-tooltip="A tooltip far wider than the little space to the left of its target" style="position:fixed;left:2px;top:5px">x</span>');
  });
  const tip = page.locator("#app-tooltip");
  const target = page.locator("#tipTarget");

  await target.hover();
  await expect(tip).toHaveClass(/app-tooltip--visible/);
  await expect(tip).toHaveText("Full detail text");
  await expect(target).toHaveAttribute("aria-describedby", "app-tooltip");
  await page.mouse.move(600, 600);
  await expect(tip).not.toHaveClass(/app-tooltip--visible/);
  await expect(target).not.toHaveAttribute("aria-describedby");

  await target.focus();
  await expect(tip).toHaveClass(/app-tooltip--visible/);
  await page.keyboard.press("Escape");
  await expect(tip).not.toHaveClass(/app-tooltip--visible/);

  await page.locator("#tipEdge").hover();
  await expect(tip).toHaveClass(/app-tooltip--visible/);
  const box = await tip.boundingBox();
  expect(box.x).toBeGreaterThanOrEqual(8);
  expect(box.y).toBeGreaterThan(25); // below its target, not off the top
});

async function createSidebarPlaylists(page, names) {
  for (const name of names) {
    await page.locator("#addPlaylistBtn").click();
    await page.locator("#navPlaylistList .nav-new-input").fill(name);
    await page.locator("#navPlaylistList .nav-new-input").press("Enter");
    await expect(page.locator("#navPlaylistList .nav-playlist-name").first()).toHaveText(name);
  }
}

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

// Each part starts the app with the update check answering `check` (the
// mock reads it from sessionStorage), with a banner never dismissed.
async function showUpdate(page, check, { installFails = false } = {}) {
  await page.evaluate(({ check, installFails }) => {
    window.localStorage.clear();
    window.sessionStorage.setItem("updateCheck", JSON.stringify(check));
    window.sessionStorage.setItem("installFails", installFails ? "1" : "");
  }, { check, installFails });
  await page.reload();
}

test("update check", async ({ page }) => {
  await installBasicTauriMock(page);
  await page.goto("/");

  const debUrl = "https://example.test/dl/DJ_USB_Tkit_0.3.0_amd64.deb";
  // A critical release: the critical banner.
  await showUpdate(page, updateCheck("critical"));
  {
    const banner = page.locator("#updateBanner");
    await expect(banner).toBeVisible();
    await expect(banner).toHaveClass(/is-critical/);
    await expect(banner).toHaveAttribute("role", "alert");
    await expect(page.locator("#updateBannerText")).toHaveText("Critical update available: 0.3.0 — view release");
  }

  // A normal release: the settings line and dot, no banner.
  await showUpdate(page, updateCheck("normal"));
  {
    // The check has landed (the settings update line and the dot show it)…
    await expect(page.locator("#settingsUpdateStatus")).toHaveText("Version 0.3.0 is available.");
    await expect(page.locator("#settingsUpdateDot")).toBeVisible();
    // …and a routine release stays out of the way.
    await expect(page.locator("#updateBanner")).toBeHidden();
  }

  // A package-manager install: a direct download link, not the in-app updater.
  await showUpdate(page, updateCheck("feature", { installKind: "deb", downloadUrl: debUrl, action: "download" }));
  {
    const bannerActions = page.locator("#updateBannerActions");
    await expect(page.locator("#updateBanner")).toBeVisible();
    await expect(bannerActions.locator(".update-download-link")).toBeVisible();
    await expect(bannerActions.locator(".update-install-btn")).toBeHidden();
    await expect(page.locator("#settingsUpdateActions .update-install-btn")).toBeHidden();

    await bannerActions.locator(".update-download-link").click();
    await expect
      .poll(() => page.evaluate(() => window.__calls.find((c) => c.command === "plugin:opener|open_url")?.payload?.url))
      .toBe(debUrl);
  }

  // Update & restart: the update runs as a job on the footer progress bar.
  await showUpdate(page, selfUpdatable("critical"));
  {
    const install = page.locator("#updateBannerActions .update-install-btn");
    await expect(install).toBeVisible();
    await install.click();

    await expect(page.locator("#updateBanner")).toBeHidden();
    await expect(page.locator("#progressFooter")).toHaveClass(/active/);
    await expect(page.locator("#progressText")).toContainText("Downloading update 0.3.0...");
    await expect(page.locator("#progressFill")).toHaveAttribute("style", /width: 40%/);
    // No second install while the first one runs.
    await expect(page.locator("#settingsUpdateActions .update-install-btn")).toBeDisabled();
  }

  // A failed update: the reason in the footer, and it can be retried.
  await showUpdate(page, selfUpdatable("feature"), { installFails: true });
  {
    await page.locator("#updateBannerActions .update-install-btn").click();

    await expect(page.locator("#progressFooter")).toHaveClass(/active/);
    await expect(page.locator("#progressText")).toContainText("Update failed: signature mismatch (mock)");
    await expect(page.locator("#settingsUpdateActions .update-install-btn")).toBeEnabled();
    // The banner comes back with the retry button.
    await expect(page.locator("#updateBanner")).toBeVisible();
    await expect(page.locator("#updateBannerActions .update-install-btn")).toBeEnabled();
    // The direct download stays on offer as the fallback (settings drawer is closed here).
    await expect(page.locator("#settingsUpdateActions .update-download-link")).not.toHaveClass(/hidden/);
  }

  // Update & restart waits for a running job instead of cutting it short.
  await showUpdate(page, selfUpdatable("feature"));
  {
    await expect(page.locator("#updateBannerActions .update-install-btn")).toBeVisible();

    await page.evaluate(() => window.__emitJobEvent({
      event: "job.started", jobId: "job-export-1", jobType: "export", stage: "export_to_usb",
      current: 0, total: 1, percent: 0, message: "Exporting..."
    }));
    await expect(page.locator("#progressText")).toContainText("Exporting...");

    await page.locator("#updateBannerActions .update-install-btn").click();
    await expect(page.locator("#statusText")).toContainText("Finish the running job before updating.");
    expect(await page.evaluate(() => window.__calls.some((c) => c.command === "install_update"))).toBe(false);
  }

  // The settings update line offers one action; its button re-checks on demand.
  await showUpdate(page, selfUpdatable("normal"));
  {
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
  }

  // A feature release: the new-features banner; dismissing it sticks for that version.
  await showUpdate(page, updateCheck("feature"));
  {
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
  }
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

// In the test language, the first-run Library, Help and the update banner and card show only
// catalog text and data.
test("test language: first run, Help, update", async ({ page }) => {
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

// A library row shows the backend's source icon, format badge, waveform and analyze state; its +
// waits for an active playlist.
test("library row", async ({ page }) => {
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    window.__e2eSourceFolders = ["/music"];
    const track = (id, extra) => ({ id, title: `Track ${id}`, artist: "Made-up Artist", filePath: `/music/${id}.wav`, ...extra });
    backend.seed({ tracks: [
      track("usb", { fileSource: { kind: "usb", missing: false, tooltip: "On USB: /stick/a.mp3" }, analysisReady: true }),
      track("usb-off", { fileSource: { kind: "usb", missing: true, tooltip: "USB not connected: /stick/a.mp3" } }),
      track("local", {
        fileSource: { kind: "local", missing: false, tooltip: "In local library: /music/b.mp3" },
        // PWV4 colour data only: still drawn on a canvas.
        waveformColorData: [1, 2, 3, 4, 5, 6], waveformPeaksPath: "/x/ANLZ0000.EXT",
      }),
      track("autofix", { formatExt: "wav", formatCompat: { severity: "autofix", badge: "WAV", tooltip: "Will be automatically converted to standard PCM on export." } }),
      track("warn", { formatExt: "wav", formatCompat: { severity: "warn", badge: "WAV", tooltip: "44.1 kHz · 16-bit — cannot be safely converted" } }),
      track("ok", { formatExt: "wav", formatCompat: { severity: "ok", badge: "WAV" } }),
    ] });
    backend.install();
  });
  await page.goto("/");
  const row = (id) => page.locator(`#libraryTableBody .track-grid-row[data-track-id="${id}"]`);
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(6);

  // Source icon: the backend's tooltip; red when the USB isn't there; USB and
  // local look different; none without the backend's field.
  const icon = (id) => row(id).locator(".td-select .track-source");
  await expect(icon("usb")).toHaveAttribute("data-tooltip", "On USB: /stick/a.mp3");
  await expect(icon("usb")).not.toHaveClass(/is-missing/);
  await expect(icon("usb-off")).toHaveClass(/is-missing/);
  await expect(icon("usb-off")).toHaveAttribute("data-tooltip", "USB not connected: /stick/a.mp3");
  expect(await icon("usb").innerHTML()).not.toBe(await icon("local").innerHTML());
  await expect(icon("ok")).toHaveCount(0);

  // Format badge, as the backend's formatCompat describes it.
  const badge = (id) => row(id).locator(".format-badge");
  await expect(badge("autofix")).toHaveText("WAV ⟳");
  await expect(badge("autofix")).toHaveClass("format-badge autofix");
  await expect(badge("autofix")).toHaveAttribute("data-tooltip", /Will be automatically converted to standard PCM on export/);
  await expect(badge("warn")).toHaveText("WAV ⚠");
  await expect(badge("warn")).toHaveClass("format-badge warn");
  await expect(badge("warn")).toHaveAttribute("data-tooltip", "44.1 kHz · 16-bit — cannot be safely converted");
  await expect(badge("ok")).toHaveText("WAV");
  await expect(badge("ok")).toHaveClass("format-badge");
  await expect(badge("ok")).not.toHaveAttribute("data-tooltip");

  await expect(row("local").locator(".waveform.waveform-canvas .waveform-canvas-el")).toHaveCount(1);

  // Analyze or Reanalyze, from the backend's analysisReady.
  const analyze = (id) => row(id).locator('[data-action="analyze-track"]');
  await expect(analyze("usb-off")).toHaveText("Analyze");
  await expect(analyze("usb-off")).toHaveAttribute("data-tooltip", "Add the missing waveform, BPM, key and first beat; a BPM or key you edited or imported is kept");
  await expect(analyze("usb")).toHaveText("Reanalyze");
  await expect(analyze("usb")).toHaveAttribute("data-tooltip", "Reset waveform, BPM, key and first beat to a fresh analysis, replacing imported or edited values (cues are kept)");

  // No playlist yet: + is off and says why; a new (active) playlist turns it on.
  const add = row("ok").locator('[data-action="add-library"]');
  await expect(add).toBeDisabled();
  await expect(add).toHaveAttribute("data-tooltip", "Create and activate a playlist first, then add tracks to it.");
  await page.locator("#addPlaylistBtn").click();
  await page.locator(".nav-new-input").fill("<Mix & Match>");
  await page.locator(".nav-new-input").press("Enter");
  await expect(add).toBeEnabled();
  // Enter, then the input's blur: one playlist.
  expect(await page.evaluate(() => window.__calls.filter((c) => c.command === "create_playlist").length)).toBe(1);
  await expect(add).toHaveAttribute("data-tooltip", "Add to <Mix & Match>");

  // The sidebar shows the name as text, marks it the active playlist, and
  // with no export status, no export mark.
  const item = page.locator("#navPlaylistList .nav-playlist-item").first();
  await expect(item.locator(".nav-playlist-name")).toHaveText("<Mix & Match>");
  await expect(item).toHaveClass(/playlist-active-mode/);
  await expect(item.locator(".nav-playlist-status")).not.toHaveClass(/exported/);
  await expect(item.locator(".nav-playlist-status")).not.toHaveAttribute("aria-label");

  // Clicking another playlist while a new one's name is typed saves the new
  // one, then opens the clicked one; with no name typed it only opens it.
  const items = page.locator("#navPlaylistList .nav-playlist-item");
  const creates = () => page.evaluate(() => window.__calls.filter((c) => c.command === "create_playlist").length);
  await page.locator("#addPlaylistBtn").click();
  await page.locator(".nav-new-input").fill("Second");
  await items.filter({ hasText: "<Mix & Match>" }).click();
  await expect(items.filter({ hasText: "Second" })).toHaveCount(1);
  expect(await creates()).toBe(2);
  await expect(page.locator("#panel-playlist")).toHaveClass(/active/);
  await expect(page.locator("#playlistPanelTitle")).toHaveText(/^<Mix & Match>/);
  await page.waitForTimeout(300);
  await expect(page.locator("#playlistPanelTitle")).toHaveText(/^<Mix & Match>/);

  await page.locator("#addPlaylistBtn").click();
  await items.filter({ hasText: "Second" }).click();
  await expect(page.locator("#playlistPanelTitle")).toHaveText(/^Second/);
  await expect(page.locator(".nav-new-input")).toHaveCount(0);
  expect(await creates()).toBe(2);
});

// Startup: defaults backup on and Engine library off, opens the top playlist, runs the data
// migrations after the first load and logs their failure; a changed setting goes to the settings
// database, and a start with only that database restores every stored setting; a failed first load
// says so.
test("startup and settings", async ({ page }) => {
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    backend.seed({ playlists: [{ id: "pl-top", name: "Top Set" }, { id: "pl-2", name: "Second" }] });
    backend.install((command) => {
      if (command === "get_frontend_settings" && sessionStorage.getItem("fromDb")) {
        return backend.ok({
          values: {
            ui_sidebar_collapsed_v1: "1", ui_export_backup_v1: "0", ui_export_prune_stale_v1: "0",
            ui_export_engine_library_v1: "1", ui_analysis_bpm_range_v1: "58-115",
          },
          nodeAvailable: true, essentiaInstalled: true,
        });
      }
      if (command === "get_frontend_settings" && sessionStorage.getItem("startFails")) {
        return backend.ok({ values: { ui_analysis_bpm_range_v1: "fast" }, nodeAvailable: true, essentiaInstalled: true });
      }
      if (command === "list_playlists" && sessionStorage.getItem("startFails")) return backend.fail("database locked");
      if (command === "run_data_migrations") return backend.fail("Data upgrade failed: disk full");
    });
  });
  await page.goto("/");

  await expect(page.locator("#exportBackupCheckbox")).toBeChecked();
  await expect(page.locator("#exportEngineLibraryCheckbox")).not.toBeChecked();
  await expect(page.locator("#exportSyncModeMirror")).toBeChecked();
  await expect(page.locator("#navSidebar")).not.toHaveClass(/collapsed/);
  await expect(page.locator('#navPlaylistList .nav-playlist-item[data-playlist-id="pl-top"]')).toHaveClass(/playlist-active-mode/);
  await expect(page.locator("#settingsVersionText")).toHaveText(/^Version /);

  await expect.poll(() => page.evaluate(() => window.__calls.some((c) => c.command === "run_data_migrations"))).toBe(true);
  const order = await page.evaluate(() => window.__calls.map((c) => c.command));
  expect(order.indexOf("run_data_migrations")).toBeGreaterThan(order.indexOf("browse_source_files"));
  await page.locator("#eventLogBtn").click();
  await expect(page.locator("#eventLogList")).toContainText("Data upgrade failed: disk full");

  await page.locator("#sidebarCollapseBtn").click();
  await expect(page.locator("#navSidebar")).toHaveClass(/collapsed/);
  expect(await page.evaluate(() => window.__calls
    .filter((c) => c.command === "set_frontend_setting" && c.request?.key === "ui_sidebar_collapsed_v1")
    .map((c) => c.request.value))).toEqual(["1"]);

  await page.evaluate(() => { localStorage.clear(); sessionStorage.setItem("fromDb", "1"); });
  await page.reload();
  await expect(page.locator("#navSidebar")).toHaveClass(/collapsed/);
  await expect(page.locator("#sidebarExpandBtn")).toHaveClass(/visible/);
  await expect(page.locator("#exportBackupCheckbox")).not.toBeChecked();
  await expect(page.locator("#exportEngineLibraryCheckbox")).toBeChecked();
  await expect(page.locator("#exportSyncModeAdditive")).toBeChecked();
  await expect(page.locator("#analysisBpmRangeSelect")).toHaveValue("58-115");
  expect(await page.evaluate(() => localStorage.getItem("djusbtkit.exportBackup"))).toBe("0");

  // A first load that fails says so, and the migrations wait for a load that
  // works; a stored BPM range the app doesn't offer falls back to the default.
  await page.evaluate(() => { localStorage.clear(); sessionStorage.removeItem("fromDb"); sessionStorage.setItem("startFails", "1"); });
  await page.reload();
  await expect(page.locator("#statusText")).toHaveText("Initialization failed: database locked");
  await expect(page.locator("#progressText")).toHaveText("Initializing failed");
  await expect(page.locator("#progressFooter")).toHaveClass(/error/);
  expect(await page.evaluate(() => window.__calls.some((c) => c.command === "run_data_migrations"))).toBe(false);
  await expect(page.locator("#analysisBpmRangeSelect")).toHaveValue("70-180");
});

// Analysis engine: choosing Essentia shows its install row; download, cancel, a missing Node.js and
// remove each show what they do and choosing is saved.
test("analysis engine", async ({ page }) => {
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    const flag = (key) => sessionStorage.getItem(key);
    backend.install(async (command) => {
      if (command === "get_frontend_settings") {
        return backend.ok({ values: {}, nodeAvailable: !flag("noNode"), essentiaInstalled: !!flag("installed") });
      }
      if (command === "download_essentia") {
        await new Promise((resolve) => { window.__finishDownload = resolve; });
        return backend.fail("download cancelled");
      }
      if (command === "cancel_essentia_download") {
        window.__finishDownload?.();
        return backend.ok(null);
      }
      if (command === "remove_essentia") return backend.ok(null);
    });
  });
  await page.goto("/");
  const engine = page.locator("#analysisEngineSelect");
  const row = page.locator("#essentiaInstallRow");
  const status = page.locator("#essentiaNodeStatus");
  const download = page.locator("#essentiaDownloadBtn");
  const cancel = page.locator("#essentiaCancelBtn");
  const remove = page.locator("#essentiaRemoveBtn");
  const savedEngines = () => page.evaluate(() => window.__calls
    .filter((c) => c.command === "set_frontend_setting" && c.request?.key === "ui_analysis_engine_v1")
    .map((c) => c.request.value));

  await page.locator("#settingsBtn").click();
  expect(await engine.locator("option").evaluateAll((options) => options.map((o) => o.value))).toEqual(["stratum", "essentia"]);
  await expect(row).toBeHidden();
  await engine.selectOption("essentia");
  await expect(row).toBeVisible();
  await expect(status).toHaveText("Essentia files not installed");
  expect(await savedEngines()).toEqual(["essentia"]);

  await download.click();
  await expect(status).toHaveText("Downloading...");
  await expect(download).toBeHidden();
  await expect(cancel).toBeVisible();
  await expect(remove).toBeHidden();
  await cancel.click();
  await expect(status).toContainText("download cancelled");
  await expect(status).toHaveClass(/essentia-warn/);
  expect(await page.evaluate(() => window.__calls.some((c) => c.command === "cancel_essentia_download"))).toBe(true);

  // Without Node.js the row says where to get it.
  await page.evaluate(() => sessionStorage.setItem("noNode", "1"));
  await page.reload();
  await page.locator("#settingsBtn").click();
  await expect(status.locator(".essentia-node-link")).toBeVisible();
  await expect(download).toBeVisible();

  // Installed: ready, and Remove goes back to stratum.
  await page.evaluate(() => { sessionStorage.removeItem("noNode"); sessionStorage.setItem("installed", "1"); });
  await page.reload();
  await page.locator("#settingsBtn").click();
  await expect(status).toHaveText("✓ Essentia ready");
  await expect(status).toHaveClass(/essentia-ready/);
  await expect(download).toBeHidden();
  await remove.click();
  await expect(engine).toHaveValue("stratum");
  await expect(row).toBeHidden();
  expect(await page.evaluate(() => window.__calls.some((c) => c.command === "remove_essentia"))).toBe(true);
  expect(await savedEngines()).toEqual(["stratum"]);
});
