import { test, expect } from "./coverage-fixture.mjs";

function installTauriMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.__tracklistOptionRequests = [];
    window.__tracklistBuilds = [];
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
          if (command === "browse_source_files") {
            return {
              ok: true,
              data: { items: [], total: 0, nextCursor: null, hasMore: false, totalDurationMs: 0, durationUnknownCount: 0, listTrackCount: 0, listSizeBytes: 0, warnings: [] }
            };
          }
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
          if (command === "get_usb_history_tracklist_options") {
            window.__tracklistOptionRequests.push(payload.request);
            return {
              ok: true,
              data: { startTracks: ["1. DJ One - Opener", "2. DJ Two - Peak Time", "3. DJ Three - Closer", "4. DJ Four - cut by the backend…"] }
            };
          }
          if (command === "build_usb_history_tracklist") {
            window.__tracklistBuilds.push(payload.request);
            return {
              ok: true,
              data: {
                suggestedFileName: "from the backend.txt",
                contents: "the backend's tracklist",
                exportedText: "Exported, says the backend",
                cancelledText: "Cancelled, says the backend"
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
                  },
                  {
                    id: "hist-0",
                    name: "HISTORY 000",
                    createdAt: null,
                    tracks: historyTracks.slice(0, 1)
                  }
                ],
                counts: { importedPlaylists: 2, importedTracks: 5 },
                summaryText: "2 sessions, 5 tracks",
                statusText: "USB histories loaded: 2",
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
                listSummaryText: "4 tracks",
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

  // The list shows the sessions in the order the backend sends: newest first.
  const rows = page.locator("#historyList li");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0).locator(".playlist-label")).toHaveText("HISTORY 001");
  await expect(rows.nth(1).locator(".playlist-label")).toHaveText("HISTORY 000");
  // The date is its own line below the name, and a session without one has no line.
  const firstDate = rows.nth(0).locator(".history-date");
  await expect(firstDate).toHaveText("2026-08-01");
  await expect(rows.nth(1).locator(".history-date")).toBeHidden();
  const nameBox = await rows.nth(0).locator(".playlist-label").boundingBox();
  const dateBox = await firstDate.boundingBox();
  expect(dateBox.y).toBeGreaterThanOrEqual(nameBox.y + nameBox.height);
  const fontSize = (locator) => locator.evaluate((node) => parseFloat(getComputedStyle(node).fontSize));
  expect(await fontSize(firstDate)).toBeLessThan(await fontSize(rows.nth(0).locator(".playlist-label")));
  // Same layout as USB Playlists: totals on the header line, the open
  // session's name + track count above its table.
  await expect(page.locator(".panel-header #historyCountsText")).toHaveText("2 sessions, 5 tracks");
  await expect(page.locator("#historyHeading .track-list-heading-name")).toHaveText("HISTORY 001");
  await expect(page.locator("#historyHeading .track-list-heading-count")).toHaveText("4 tracks");
  // Opening the first session by itself doesn't replace the read's status.
  await expect(page.locator("#statusText")).toContainText("USB histories loaded: 2");

  // Under the session's tracks, like an app playlist's Export to USB.
  await expect(page.locator("#panel-usb-history .right > .panel-actions #exportHistoryTracklistBtn")).toBeEnabled();
  await expect(page.locator("#exportHistoryTracklistBtn")).toHaveCSS("background-image", /linear-gradient/);
  await expect(page.locator("#exportHistoryTracklistBtn")).toHaveText("Export Tracklist to Text File");
  await page.locator("#exportHistoryTracklistBtn").click();

  await expect(page.locator("#tracklistExportOverlay")).toBeVisible();

  const options = page.locator("#tracklistExportStartTrack option");
  await expect(options).toHaveCount(4);
  await expect(options.nth(0)).toHaveText("1. DJ One - Opener");
  await expect(options.nth(1)).toHaveText("2. DJ Two - Peak Time");
  await expect(options.nth(2)).toHaveText("3. DJ Three - Closer");
  await expect(options.nth(3)).toHaveText("4. DJ Four - cut by the backend…");
  expect(await page.evaluate(() => window.__tracklistOptionRequests)).toEqual([{ usbRoot: "/Volumes/USB-TEST", id: "hist-1" }]);

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
  for (const dismissal of ["Escape", "backdrop", "Escape", "backdrop"]) {
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
  const status = page.locator("#statusText");

  // A cancelled dialog saves nothing. It opens with times on, placed before the track.
  await page.locator("#exportHistoryTracklistBtn").click();
  await expect(page.locator("#tracklistExportTimesToggle")).toBeChecked();
  await expect(page.locator("#tracklistExportPlacement")).toHaveValue("before");
  await page.locator("#tracklistExportCancelBtn").click();
  expect(await page.evaluate(() => window.__saves)).toEqual([]);

  await page.locator("#exportHistoryTracklistBtn").click();
  await page.locator("#tracklistExportStartTrack").selectOption({ index: 1 });
  await page.locator("#tracklistExportOkBtn").click();
  await expect(status).toContainText("Exported, says the backend");
  const [save] = await page.evaluate(() => window.__saves);
  expect(save.suggestedFileName).toBe("from the backend.txt");
  expect(save.contents).toBe("the backend's tracklist");
  expect(await page.evaluate(() => window.__tracklistBuilds)).toEqual([
    { usbRoot: "/Volumes/USB-TEST", id: "hist-1", startIndex: 1, timeMode: "before" }
  ]);

  await page.evaluate(() => { window.__saveAccepted = false; });
  await page.locator("#exportHistoryTracklistBtn").click();
  await page.locator("#tracklistExportTimesToggle").uncheck();
  await page.locator("#tracklistExportOkBtn").click();
  await expect(status).toContainText("Cancelled, says the backend");
  expect((await page.evaluate(() => window.__tracklistBuilds))[1]).toEqual(
    { usbRoot: "/Volumes/USB-TEST", id: "hist-1", startIndex: 0, timeMode: "off" }
  );

  // Another row exports its own session.
  await page.locator("#historyList li").nth(1).locator("button").click();
  await expect(page.locator("#historyHeading .track-list-heading-name")).toHaveText("HISTORY 000");
  await page.locator("#exportHistoryTracklistBtn").click();
  await page.locator("#tracklistExportOkBtn").click();
  await expect.poll(() => page.evaluate(() => window.__tracklistBuilds.at(-1)?.id)).toBe("hist-0");
});

test("a session the backend says can't export disables Export Tracklist, and a long name stays on its row", async ({ page }) => {
  await installTauriMock(page);
  const longName = `HISTORY ${"very long session name ".repeat(20)}`;
  await page.addInitScript((name) => {
    const invoke = window.__TAURI__.core.invoke;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "fetch_usb_histories") {
        return {
          ok: true,
          data: {
            items: [{ id: "hist-1", name, createdAt: "2026-08-01", canExportTracklist: false, tracks: [] }],
            counts: { importedPlaylists: 1, importedTracks: 0 },
            warnings: []
          }
        };
      }
      return invoke(command, payload);
    };
  }, longName);
  await page.goto("/");
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-history"]').click();

  await expect(page.locator("#historyHeading .track-list-heading-name")).toHaveText(longName.trim());
  const button = page.locator("#exportHistoryTracklistBtn");
  await expect(button).toBeDisabled();
  await expect(button).toBeInViewport({ ratio: 1 });
  expect(await page.locator("#historyHeading").evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
});
