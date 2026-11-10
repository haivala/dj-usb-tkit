import { test, expect, uiTextCheck, useTestLanguage } from "./coverage-fixture.mjs";

// Exercises the "Name this drive" prompt and renaming end-to-end in a real browser DOM,
// where element ids, overlay visibility, and recent-USB-pill wiring are all
// part of the assertion surface.
function installTauriMock(
  page,
  { deviceName = null, suggestedName = null, getNameMode = "normal", takenName = null, nameTakenBy = null } = {}
) {
  return page.addInitScript(({ deviceName, suggestedName, getNameMode, takenName, nameTakenBy }) => {
    const backend = window.__e2eBackend;
    // The names saved: the backend took them.
    window.__setNameCalls = [];
    // A name saved before a reload (`sessionStorage`) is the drive's from the start.
    let storedName = sessionStorage.getItem("deviceName") || deviceName;
    const usbRoot = "/Volumes/USB-TEST";
    let forgotten = false;
    const devices = () => forgotten ? [] : [{
      id: "dev-1", rootPath: usbRoot, label: storedName, mounted: false,
      firstSeenAt: "2024-01-01T00:00:00Z", lastSeenAt: "2024-01-01T00:00:00Z"
    }];

    backend.install((command, r) => {
      switch (command) {
        case "list_usb_devices": return backend.ok({ items: devices() });
        case "prune_usb_device":
          forgotten = true;
          return backend.ok({ pruned: true, devices: devices() });
        case "select_usb":
          return backend.ok({
            normalizedRoot: String(r.path || ""),
            devices: devices(),
            // The name couldn't be read ("throw"): the backend logs why and sends none.
            deviceName: getNameMode === "throw" ? null : {
              name: storedName,
              suggestedName: storedName ? null : suggestedName,
              nameTakenBy: storedName ? null : nameTakenBy
            }
          });
        case "set_usb_device_name": {
          const name = r.name || "";
          if (!name.trim()) return backend.fail("mock empty name", "VALIDATION");
          if (name === takenName) {
            return backend.fail(`Drive name '${name}' is already used by another drive ('${takenName}')`, "VALIDATION");
          }
          const updated = name !== storedName;
          storedName = name;
          window.__setNameCalls.push({ usbRoot: r.usbRoot, name });
          return backend.ok({ name, updated, statusText: updated ? `mock renamed to ${name}` : "mock name unchanged" });
        }
        default: return undefined;
      }
    });
  }, { deviceName, suggestedName, getNameMode, takenName, nameTakenBy });
}

async function openRecentUsbNamingPrompt(page, opts = {}) {
  await installTauriMock(page, opts);
  await page.goto("/");
  await page.locator('.nav-item[data-view="usb"]').click();
  const pill = page.locator('#usbRecentList button[data-usb-recent-path="/Volumes/USB-TEST"]');
  await expect(pill).toBeVisible();
  await pill.click();
  return pill;
}

test("naming a drive from its recent pill", async ({ page }) => {
  // An unnamed drive's pill opens the naming prompt, saves the name, and shows
  // it in the status-line badge.
  await openRecentUsbNamingPrompt(page);

  await expect(page.locator("#driveNameOverlay")).toBeVisible();
  await expect(page.locator("#usbNameBadge")).toBeVisible();
  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Unnamed");
  await page.locator("#driveNameInput").fill("Club Stick");
  await page.locator("#driveNameOkBtn").click();

  await expect(page.locator("#driveNameOverlay")).toBeHidden();
  const calls = await page.evaluate(() => window.__setNameCalls);
  expect(calls).toEqual([{ usbRoot: "/Volumes/USB-TEST", name: "Club Stick" }]);

  await expect(page.locator("#usbNameBadge")).toBeVisible();
  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Club Stick");

  // Selecting a drive closes the recent list; its header reopens it.
  await expect(page.locator("#usbRecentRow")).toHaveClass(/collapsed/);
  await expect(page.locator('#usbRecentList button[data-usb-recent-path="/Volumes/USB-TEST"]')).toBeHidden();
  await page.locator("#usbRecentHeader").click();
  await expect(page.locator("#usbRecentRow")).not.toHaveClass(/collapsed/);
  await expect(page.locator('#usbRecentList button[data-usb-recent-path="/Volumes/USB-TEST"]')).toBeVisible();

  // An already-named drive's pill doesn't open the prompt, and the badge shows its name.
  await page.evaluate(() => sessionStorage.setItem("deviceName", "Club Stick"));
  await page.reload();
  {
    await page.locator('.nav-item[data-view="usb"]').click();
    await page.locator('#usbRecentList button[data-usb-recent-path="/Volumes/USB-TEST"]').click();

    // Give the (skipped) async naming check a turn to run before asserting
    // it stayed closed, rather than just checking the initial synchronous state.
    await expect(page.locator("#usbRootPathText")).toHaveText("Club Stick");
    await expect(page.locator("#usbRootPathText")).toHaveAttribute("data-tooltip", "/Volumes/USB-TEST");
    await expect(page.locator("#usbRootPathText")).toHaveClass(/usb-path-valid/);
    await expect(page.locator("#driveNameOverlay")).toBeHidden();
    await expect(page.locator("#usbNameBadge")).toBeVisible();
    await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Club Stick");
  }
});

test("forgetting a recent USB drops it from the list the prune answers with", async ({ page }) => {
  await installTauriMock(page);
  await page.goto("/");
  await page.locator('.nav-item[data-view="usb"]').click();
  const pill = page.locator('#usbRecentList button[data-usb-recent-path="/Volumes/USB-TEST"]');
  await expect(pill).toBeVisible();
  const lists = await page.evaluate(() => window.__calls.filter((c) => c.command === "list_usb_devices").length);

  await page.locator("#usbRecentList .usb-cfg-recent-prune-btn").click();
  await expect(pill).toHaveCount(0);
  expect(await page.evaluate(() => window.__calls.filter((c) => c.command === "prune_usb_device").map((c) => c.request.id))).toEqual(["dev-1"]);
  // The list came with the prune's answer: no second read.
  expect(await page.evaluate(() => window.__calls.filter((c) => c.command === "list_usb_devices").length)).toBe(lists);
});

test("selecting the same USB again picks up a session recorded since the last read", async ({ page }) => {
  await installTauriMock(page, { deviceName: "Club Stick" });
  await page.addInitScript(() => {
    window.__sessions = [{ id: "h-1", name: "HISTORY 001", tracks: [] }];
    const backend = window.__e2eBackend;
    backend.override(async (command, request, payload, next) => {
      if (command === "fetch_usb_histories") {
        return window.__e2eBackend.ok(window.__e2eBackend.usbHistories({ items: window.__sessions }));
      }
    });
  });
  await page.goto("/");
  const pill = page.locator('#usbRecentList button[data-usb-recent-path="/Volumes/USB-TEST"]');
  const history = page.locator("#historyList");

  await page.locator('.nav-item[data-view="usb"]').click();
  await pill.click();
  await expect(page.locator("#usbRootPathText")).toHaveClass(/usb-path-valid/);
  await page.locator('.nav-item[data-view="usb-history"]').click();
  await expect(history).toContainText("HISTORY 001");
  await expect(history).not.toContainText("HISTORY 002");

  await page.evaluate(() => window.__sessions.push({ id: "h-2", name: "HISTORY 002", tracks: [] }));
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#selectUsbFolderBtn").click();
  await page.locator('.nav-item[data-view="usb-history"]').click();
  await expect(history).toContainText("HISTORY 002");
});

test("recent USB pill naming prompt pre-fills the OS-suggested drive label", async ({ page }) => {
  await openRecentUsbNamingPrompt(page, { suggestedName: "CLUBSTICK" });

  await expect(page.locator("#driveNameOverlay")).toBeVisible();
  await expect(page.locator("#driveNameInput")).toHaveValue("CLUBSTICK");
});

test("recent USB naming prompt can be dismissed without saving", async ({ page }) => {
  const pill = await openRecentUsbNamingPrompt(page);
  await expect(page.locator("#driveNameOverlay")).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.locator("#driveNameOverlay")).toBeHidden();

  // Selecting the drive closed the recent list: reopen it to pick it again.
  const reopenRecent = () => page.locator("#usbRecentHeader").click();
  await reopenRecent();
  await pill.click();
  await expect(page.locator("#driveNameOverlay")).toBeVisible();
  await page.locator("#driveNameOverlay").dispatchEvent("click");
  await expect(page.locator("#driveNameOverlay")).toBeHidden();

  await reopenRecent();
  await pill.click();
  await expect(page.locator("#driveNameOverlay")).toBeVisible();
  await page.locator("#driveNameSkipBtn").click();
  await expect(page.locator("#driveNameOverlay")).toBeHidden();

  const calls = await page.evaluate(() => window.__setNameCalls);
  expect(calls).toEqual([]);
});

test("recent USB naming check failures do not open a blocking prompt", async ({ page }) => {
  await openRecentUsbNamingPrompt(page, { getNameMode: "throw" });

  await expect(page.locator("#usbRootPathText")).toContainText("/Volumes/USB-TEST");
  await expect(page.locator("#driveNameOverlay")).toBeHidden();
});

async function openNamedDrive(page, opts = {}) {
  await installTauriMock(page, { deviceName: "Club Stick", ...opts });
  await page.goto("/");
  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Not connected");
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator('#usbRecentList button[data-usb-recent-path="/Volumes/USB-TEST"]').click();
  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Club Stick");
}

test("clicking the header drive name renames the connected drive", async ({ page }) => {
  await openNamedDrive(page);
  await expect(page.locator("#usbNameBadge")).toHaveAttribute("data-tooltip", "Rename this drive");

  await page.locator("#usbNameBadge").focus();
  await page.locator("#usbNameBadgeLabel").click();
  const overlay = page.locator("#driveNameOverlay");
  await expect(overlay).toBeVisible();
  await expect(page.locator("#driveNameTitle")).toHaveText("Rename This Drive", { useInnerText: true });
  await expect(page.locator("#driveNameSkipBtn")).toHaveText("Cancel", { useInnerText: true });
  await expect(page.locator("#driveNameInput")).toHaveValue("Club Stick");
  for (const dismissal of ["Escape", "backdrop", "Cancel"]) {
    if (dismissal === "Escape") await page.keyboard.press("Escape");
    else if (dismissal === "backdrop") await overlay.click({ position: { x: 5, y: 5 } });
    else await page.locator("#driveNameSkipBtn").click();
    await expect(overlay).toBeHidden();
    await expect(page.locator("#usbNameBadge")).toBeFocused();
    expect(await page.evaluate(() => window.__setNameCalls)).toEqual([]);
    await page.locator("#usbNameBadgeLabel").click();
    await expect(page.locator("#driveNameInput")).toBeFocused();
  }

  await page.locator("#driveNameInput").fill("Booth Stick");
  await page.locator("#driveNameInput").press("Enter");

  await expect(overlay).toBeHidden();
  expect(await page.evaluate(() => window.__setNameCalls)).toEqual([
    { usbRoot: "/Volumes/USB-TEST", name: "Booth Stick" }
  ]);
  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Booth Stick");
  await expect(page.locator("#usbRootPathText")).toHaveText("Booth Stick");
  await expect(page.locator("#statusText")).toHaveText("mock renamed to Booth Stick");

  await page.locator("#usbNameBadgeLabel").click();
  await page.locator("#driveNameOkBtn").click();
  await expect(page.locator("#statusText")).toHaveText("mock name unchanged");
});

test("renaming to a name another drive has shows the error and cancel keeps the old name", async ({ page }) => {
  await openNamedDrive(page, { takenName: "Booth Stick" });

  await page.locator("#usbNameBadge").focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("#driveNameOverlay")).toBeVisible();
  await page.locator("#driveNameInput").fill("Booth Stick");
  await page.locator("#driveNameOkBtn").click();

  await expect(page.locator("#driveNameError")).toBeVisible();
  await expect(page.locator("#driveNameError")).toContainText("already used by another drive");
  await expect(page.locator("#driveNameOverlay")).toBeVisible();

  await page.locator("#driveNameInput").fill("   ");
  await page.locator("#driveNameOkBtn").click();
  await expect(page.locator("#driveNameError")).toHaveText("mock empty name");

  await page.locator("#driveNameSkipBtn").click();
  await expect(page.locator("#driveNameOverlay")).toBeHidden();
  expect(await page.evaluate(() => window.__setNameCalls)).toEqual([]);
  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Club Stick");
});

test("a drive left unnamed can be named later from the header", async ({ page }) => {
  await openRecentUsbNamingPrompt(page);
  await page.locator("#driveNameSkipBtn").click();
  await expect(page.locator("#driveNameOverlay")).toBeHidden();
  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Unnamed");
  await expect(page.locator("#usbNameBadge")).toHaveAttribute("data-tooltip", "Name this drive");

  await page.locator("#usbNameBadgeLabel").click();
  await expect(page.locator("#driveNameTitle")).toHaveText("Name This Drive", { useInnerText: true });
  await page.locator("#driveNameInput").fill("Club Stick");
  await page.locator("#driveNameOkBtn").click();

  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Club Stick");
});

test("a drive carrying another drive's name is asked for its own and says which name it carries", async ({ page }) => {
  await openRecentUsbNamingPrompt(page, { nameTakenBy: "Club Stick" });

  await expect(page.locator("#driveNameOverlay")).toBeVisible();
  await expect(page.locator("#driveNameTitle")).toHaveText("Name This Drive", { useInnerText: true });
  await expect(page.locator("#driveNameConflict")).toBeVisible();
  await expect(page.locator("#driveNameConflict")).toContainText("This drive carries the name “Club Stick”, but");
  await page.locator("#driveNameSkipBtn").click();
  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Unnamed");

  // Naming it later from the header still explains why.
  await page.locator("#usbNameBadgeLabel").click();
  await expect(page.locator("#driveNameConflict")).toBeVisible();
  await page.locator("#driveNameInput").fill("Booth Stick");
  await page.locator("#driveNameOkBtn").click();
  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Booth Stick");
});

test("in the test language, the drive naming dialog shows only catalog text and data", async ({ page }) => {
  await installTauriMock(page);
  await useTestLanguage(page);
  await page.goto("/");
  await page.locator('.nav-item[data-view="usb"]').click();
  const pill = page.locator('#usbRecentList button[data-usb-recent-path="/Volumes/USB-TEST"]');
  await expect(pill).toBeVisible();
  const check = uiTextCheck(page);
  await check.collect();
  await pill.click();
  await expect(page.locator("#driveNameOverlay")).toBeVisible();
  await check.collect();
  await check.expectNoEnglish();
});
