import { test, expect } from "./coverage-fixture.mjs";

// Exercises the "Name this drive" prompt end-to-end in a real browser DOM
// (unlike the unit tests in tests/usb_init_flow_behavior.test.mjs, which
// mock `command`/`el` directly and so can't catch real DOM-wiring issues,
// element-id typos, or overlay stacking/visibility bugs). In particular
// this covers the recent-USB-pill entry point specifically, since that's
// the one reported as not opening the prompt in practice -- it's rendered
// at startup from `list_usb_devices`, not from a fresh folder pick.
function installTauriMock(page, { deviceName = null, suggestedName = null } = {}) {
  return page.addInitScript(({ deviceName, suggestedName }) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.__setNameCalls = [];
    let storedName = deviceName;
    const usbRoot = "/Volumes/USB-TEST";

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
            return { ok: true, data: { items: [] } };
          }
          if (command === "search_tracks") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "list_tracks") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          if (command === "fetch_usb_playlists") {
            return {
              ok: true,
              data: {
                items: [],
                stats: { indexedTracks: 0, playlistReferencedTracks: 0, playlistEntries: 0 },
                warnings: []
              }
            };
          }
          if (command === "list_usb_devices") {
            return {
              ok: true,
              data: {
                items: [
                  {
                    id: "dev-1",
                    rootPath: usbRoot,
                    label: storedName,
                    mounted: false,
                    firstSeenAt: "2024-01-01T00:00:00Z",
                    lastSeenAt: "2024-01-01T00:00:00Z"
                  }
                ]
              }
            };
          }
          if (command === "prune_usb_device") {
            return { ok: true, data: { pruned: true } };
          }
          if (command === "validate_usb_root") {
            const path = String(payload?.request?.path || "");
            return {
              ok: true,
              data: {
                valid: true,
                hasWriteAccess: true,
                normalizedRoot: path,
                hasVendorRoot: true,
                hasContents: true,
                hasPdb: true,
                hasEdb: true,
                warnings: []
              }
            };
          }
          if (command === "get_usb_device_name") {
            return {
              ok: true,
              data: { name: storedName, suggestedName: storedName ? null : suggestedName }
            };
          }
          if (command === "set_usb_device_name") {
            const name = payload?.request?.name || "";
            storedName = name;
            window.__setNameCalls.push({ usbRoot: payload?.request?.usbRoot, name });
            return { ok: true, data: { saved: true } };
          }
          if (command === "run_usb_diagnostics") {
            return {
              ok: true,
              data: {
                overallStatus: "PASS",
                pdbIntegrity: { title: "PDB Integrity", status: "PASS", checks: [] },
                edbAccess: { title: "Database Access", status: "PASS", checks: [] },
                contentsIntegrity: { title: "Contents Integrity", status: "PASS", checks: [] },
                analysisIntegrity: { title: "Analysis Files", status: "PASS", checks: [] },
                playlistResolution: { title: "Playlist Resolution", status: "PASS", checks: [] },
                playlistDetails: [],
                warnings: [],
                durationMs: 1
              }
            };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled command: ${command}` } };
        }
      }
    };
  }, { deviceName, suggestedName });
}

test("clicking a recent USB pill for an unnamed drive opens the naming prompt, saves the name, and shows it in the status-line badge", async ({ page }) => {
  await installTauriMock(page, {});
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();

  const pill = page.locator('#usbRecentList button[data-usb-recent-path="/Volumes/USB-TEST"]');
  await expect(pill).toBeVisible();
  await pill.click();

  await expect(page.locator("#driveNameOverlay")).toBeVisible();
  await expect(page.locator("#usbNameBadge")).toBeVisible();
  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Not connected");
  await page.locator("#driveNameInput").fill("Club Stick");
  await page.locator("#driveNameOkBtn").click();

  await expect(page.locator("#driveNameOverlay")).toBeHidden();
  const calls = await page.evaluate(() => window.__setNameCalls);
  expect(calls).toEqual([{ usbRoot: "/Volumes/USB-TEST", name: "Club Stick" }]);

  await expect(page.locator("#usbNameBadge")).toBeVisible();
  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Club Stick");
});

test("clicking a recent USB pill for an already-named drive does not reopen the naming prompt, and shows the name in the status-line badge", async ({ page }) => {
  await installTauriMock(page, { deviceName: "Club Stick" });
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator('#usbRecentList button[data-usb-recent-path="/Volumes/USB-TEST"]').click();

  // Give the (skipped) async naming check a turn to run before asserting
  // it stayed closed, rather than just checking the initial synchronous state.
  await expect(page.locator("#usbRootPathText")).toContainText("/Volumes/USB-TEST");
  await expect(page.locator("#usbRootPathText")).toHaveClass(/usb-path-valid/);
  await expect(page.locator("#driveNameOverlay")).toBeHidden();
  await expect(page.locator("#usbNameBadge")).toBeVisible();
  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Club Stick");
});

test("recent USB pill naming prompt pre-fills the OS-suggested drive label", async ({ page }) => {
  await installTauriMock(page, { suggestedName: "CLUBSTICK" });
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator('#usbRecentList button[data-usb-recent-path="/Volumes/USB-TEST"]').click();

  await expect(page.locator("#driveNameOverlay")).toBeVisible();
  await expect(page.locator("#driveNameInput")).toHaveValue("CLUBSTICK");
});
