import { test, expect } from "./coverage-fixture.mjs";

function installTauriMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    const historyTracks = [
      { id: "1", title: "Opener", artist: "DJ One", durationMs: 180000 },
      { id: "2", title: "Peak Time", artist: "DJ Two", durationMs: 240000 },
      { id: "3", title: "Closer", artist: "DJ Three", durationMs: 300000 },
      { id: "4", title: "x".repeat(200), artist: "DJ Four", durationMs: 200000 }
    ];
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "pick_usb_folder") return "/Volumes/USB-TEST";
          if (command === "get_library_sources") {
            return { ok: true, data: { folders: [], rekordboxDbEnabled: false, mixxxDbEnabled: false, everConfigured: false } };
          }
          if (command === "inspect_usb_track") {
            const trackId = String(payload?.request?.trackId || "");
            const track = historyTracks.find((t) => t.id === trackId);
            return { ok: true, data: { track: track ? { ...track, artworkChecked: true } : {}, warnings: [] } };
          }
          if (command === "validate_usb_root") {
            return {
              ok: true,
              data: {
                valid: true,
                hasWriteAccess: true,
                normalizedRoot: "/Volumes/USB-TEST",
                hasVendorRoot: true,
                hasContents: true,
                hasPdb: true,
                hasEdb: true,
                warnings: []
              }
            };
          }
          if (command === "run_usb_diagnostics") {
            return {
              ok: true,
              data: {
                overallStatus: "PASS",
                overview: [],
                playlistUsbExportStatus: [],
                sectionGroups: [],
                playlistDetails: [],
                warnings: [],
                durationMs: 5
              }
            };
          }
          if (command === "fetch_usb_histories") {
            return {
              ok: true,
              data: {
                items: [
                  {
                    id: "hist-1",
                    name: "HISTORY 001",
                    createdAt: "2026-08-01",
                    tracks: historyTracks
                  }
                ],
                counts: { importedPlaylists: 1, importedTracks: 3 },
                warnings: []
              }
            };
          }
          if (command === "fetch_usb_history_tracks") {
            return {
              ok: true,
              data: {
                items: historyTracks.map((t) => ({ ...t, bpm: 120, key: "8A", waveformPreview: [10, 20, 10] })),
                total: historyTracks.length,
                hasMore: false,
                nextCursor: null,
                totalDurationMs: historyTracks.reduce((s, t) => s + (t.durationMs || 0), 0),
                durationUnknownCount: 0,
                warnings: []
              }
            };
          }
          return window.__e2eBackend.answer(command);
        },
        convertFileSrc: (path) => path
      },
      event: {
        listen: async () => () => {}
      }
    };
  });
}

test("Export Tracklist dialog's start-track select is populated from the selected history session", async ({ page }) => {
  await installTauriMock(page);
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-history"]').click();

  await expect(page.locator("#historyList")).toContainText("HISTORY 001");
  await page.locator('#historyList [data-history-index]').click();
  // Same layout as USB Playlists: totals on the header line, the open
  // session's name + track count above its table.
  await expect(page.locator(".panel-header #historyCountsText")).toHaveText("1 session, 3 tracks");
  await expect(page.locator("#historyHeading .track-list-heading-name")).toHaveText("HISTORY 001");
  await expect(page.locator("#historyHeading .track-list-heading-count")).toHaveText("4 tracks");

  await expect(page.locator("#exportHistoryTracklistBtn")).toBeEnabled();
  await page.locator("#exportHistoryTracklistBtn").click();

  await expect(page.locator("#tracklistExportOverlay")).toBeVisible();

  const options = page.locator("#tracklistExportStartTrack option");
  await expect(options).toHaveCount(4);
  await expect(options.nth(0)).toHaveText("1. DJ One - Opener");
  await expect(options.nth(1)).toHaveText("2. DJ Two - Peak Time");
  await expect(options.nth(2)).toHaveText("3. DJ Three - Closer");
  const longLabel = await options.nth(3).textContent();
  expect(longLabel).toHaveLength(64);
  expect(longLabel.endsWith("…")).toBe(true);

  // Times-on by default, so the placement row starts visible; toggling times
  // off/on should hide/show it.
  await expect(page.locator("#tracklistExportPlacementRow")).toBeVisible();
  await page.locator("#tracklistExportTimesToggle").uncheck();
  await expect(page.locator("#tracklistExportPlacementRow")).toBeHidden();
  await page.locator("#tracklistExportTimesToggle").check();
  await expect(page.locator("#tracklistExportPlacementRow")).toBeVisible();

  // Closing and reopening must rebuild the option list, not append to it.
  await page.locator("#tracklistExportCancelBtn").click();
  await expect(page.locator("#tracklistExportOverlay")).toBeHidden();
  await expect(page.locator("#exportHistoryTracklistBtn")).toBeFocused();
  // Escape and backdrop dismissal also restore focus and allow reopening.
  for (const dismissal of ["Escape", "backdrop"]) {
    await page.locator("#exportHistoryTracklistBtn").click();
    await expect(page.locator("#tracklistExportOkBtn")).toBeFocused();
    if (dismissal === "Escape") await page.keyboard.press("Escape");
    else await page.locator("#tracklistExportOverlay").click({ position: { x: 5, y: 5 } });
    await expect(page.locator("#tracklistExportOverlay")).toBeHidden();
    await expect(page.locator("#exportHistoryTracklistBtn")).toBeFocused();
  }
  await page.locator("#exportHistoryTracklistBtn").click();
  await expect(page.locator("#tracklistExportOverlay")).toBeVisible();
  await expect(options).toHaveCount(4);

  await page.screenshot({ path: "test-results/tracklist-export-dialog.png" });
});

test("Export Tracklist saves the session from the chosen start track, and reports a dismissed save", async ({ page }) => {
  await installTauriMock(page);
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__saves = [];
    window.__saveAccepted = true;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command !== "save_text_file") return invoke(command, payload);
      window.__saves.push(payload);
      return window.__saveAccepted;
    };
  });
  await page.goto("/");
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-history"]').click();
  await page.locator("#historyList [data-history-index]").click();
  const status = page.locator("#statusText");

  // A cancelled dialog saves nothing.
  await page.locator("#exportHistoryTracklistBtn").click();
  await page.locator("#tracklistExportCancelBtn").click();
  expect(await page.evaluate(() => window.__saves)).toEqual([]);

  await page.locator("#exportHistoryTracklistBtn").click();
  await page.locator("#tracklistExportStartTrack").selectOption({ index: 1 });
  await page.locator("#tracklistExportOkBtn").click();
  await expect(status).toContainText("Tracklist exported: HISTORY 001");
  const [save] = await page.evaluate(() => window.__saves);
  expect(save.suggestedFileName).toBe("HISTORY 001.txt");
  const lines = save.contents.split("\n");
  expect(lines).toHaveLength(3);
  expect(lines.slice(0, 2)).toEqual(["0:00 DJ Two - Peak Time", "4:00 DJ Three - Closer"]);

  await page.evaluate(() => { window.__saveAccepted = false; });
  await page.locator("#exportHistoryTracklistBtn").click();
  await page.locator("#tracklistExportOkBtn").click();
  await expect(status).toContainText("Tracklist export cancelled");
});
