import { test, expect } from "./coverage-fixture.mjs";

function installTauriMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    const historyTracks = [
      { id: "1", title: "Opener", artist: "DJ One", durationMs: 180000 },
      { id: "2", title: "Peak Time", artist: "DJ Two", durationMs: 240000 },
      { id: "3", title: "Closer", artist: "DJ Three", durationMs: 300000 }
    ];
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") {
            return { ok: true, data: { found: false, path: null } };
          }
          if (command === "pick_usb_folder") return "/Volumes/USB-TEST";
          if (command === "list_playlists") return { ok: true, data: { items: [] } };
          if (command === "search_tracks") return { ok: true, data: { total: 0, items: [] } };
          if (command === "list_tracks") return { ok: true, data: { total: 0, items: [] } };
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
                pdbIntegrity: { title: "PDB Integrity", status: "PASS", checks: [] },
                edbAccess: { title: "Database Access", status: "PASS", checks: [] },
                contentsIntegrity: { title: "Contents Integrity", status: "PASS", checks: [] },
                analysisIntegrity: { title: "Analysis Files", status: "PASS", checks: [] },
                playlistResolution: { title: "Playlist Resolution", status: "PASS", checks: [] },
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
          return { ok: true, data: {} };
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
  await page.locator("#refreshHistoryBtn").click();

  await expect(page.locator("#historyList")).toContainText("HISTORY 001");
  await page.locator('#historyList [data-history-index]').click();

  await expect(page.locator("#exportHistoryTracklistBtn")).toBeEnabled();
  await page.locator("#exportHistoryTracklistBtn").click();

  await expect(page.locator("#tracklistExportOverlay")).toBeVisible();

  const options = page.locator("#tracklistExportStartTrack option");
  await expect(options).toHaveCount(3);
  await expect(options.nth(0)).toHaveText("1. DJ One - Opener");
  await expect(options.nth(1)).toHaveText("2. DJ Two - Peak Time");
  await expect(options.nth(2)).toHaveText("3. DJ Three - Closer");

  await page.screenshot({ path: "test-results/tracklist-export-dialog.png" });
});
