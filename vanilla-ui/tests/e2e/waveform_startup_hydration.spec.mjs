import { test, expect } from "@playwright/test";

function installWaveformStartupMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const listeners = new Map();
    const listen = async (eventName, callback) => {
      const key = String(eventName || "");
      const arr = listeners.get(key) || [];
      arr.push(callback);
      listeners.set(key, arr);
      return () => {
        const current = listeners.get(key) || [];
        listeners.set(key, current.filter((fn) => fn !== callback));
      };
    };

    const baseTracks = [
      {
        id: "t-1",
        title: "Track One",
        artist: "Artist",
        album: "Album",
        filePath: "/music/Track One.mp3",
        fileSizeBytes: 1000,
        waveformPeaksPath: "/tmp/t-1.DAT",
        waveformPreview: [],
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      },
      {
        id: "t-2",
        title: "Track Two",
        artist: "Artist",
        album: "Album",
        filePath: "/music/Track Two.mp3",
        fileSizeBytes: 1001,
        waveformPeaksPath: "/tmp/t-2.DAT",
        waveformPreview: [],
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      }
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
          if (command === "list_playlists") {
            return { ok: true, data: { items: [] } };
          }
          if (command === "list_tracks" || command === "search_tracks" || command === "browse_source_files") {
            return { ok: true, data: { total: baseTracks.length, items: baseTracks } };
          }
          if (command === "get_tracks_by_ids_with_previews") {
            const ids = Array.isArray(payload?.request?.trackIds)
              ? payload.request.trackIds.map((v) => String(v))
              : [];
            const items = baseTracks
              .filter((t) => ids.includes(String(t.id)))
              .map((t) => ({
                ...t,
                waveformPreview: [8, 20, 42, 65, 30, 55]
              }));
            return { ok: true, data: { items } };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled command: ${command}` } };
        }
      },
      event: { listen }
    };
  });
}

test("startup hydrates waveform previews for tracks with waveform paths", async ({ page }) => {
  await installWaveformStartupMock(page);
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(2);

  await expect.poll(async () => {
    return page.locator("#libraryTableBody .waveform.waveform-canvas").count();
  }).toBe(2);
});

function installSourceChipAnalysisMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    window.localStorage.setItem(
      "djusbtkit.sourceRootEnabled",
      JSON.stringify({ "/music": true })
    );
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.__scanCalls = 0;
    window.__picked = false;

    const listeners = new Map();
    const listen = async (eventName, callback) => {
      const key = String(eventName || "");
      const arr = listeners.get(key) || [];
      arr.push(callback);
      listeners.set(key, arr);
      return () => {
        const current = listeners.get(key) || [];
        listeners.set(key, current.filter((fn) => fn !== callback));
      };
    };

    const tracks = [
      {
        id: "t-1",
        title: "Track One",
        artist: "Artist",
        album: "Album",
        filePath: "/music/Track One.mp3",
        fileSizeBytes: 1000,
        waveformPeaksPath: "/tmp/t-1.DAT",
        waveformPreview: [8, 20, 42, 65, 30, 55],
        bpm: 128,
        key: "8A",
        durationMs: 195000,
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      },
      {
        id: "t-2",
        title: "Track Two",
        artist: "Artist",
        album: "Album",
        filePath: "/music2/Track Two.mp3",
        fileSizeBytes: 1001,
        waveformPeaksPath: "/tmp/t-2.DAT",
        waveformPreview: [7, 19, 43, 61, 34, 57],
        bpm: 126,
        key: "9A",
        durationMs: 201000,
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      }
    ];

    window.__TAURI__ = {
      core: {
        invoke: async (command) => {
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") {
            return { ok: true, data: { found: false, path: null } };
          }
          if (command === "list_playlists") {
            return { ok: true, data: { items: [] } };
          }
          if (command === "pick_source_folders") {
            window.__picked = true;
            return ["/music2"];
          }
          if (command === "scan_library") {
            window.__scanCalls += 1;
            return { ok: true, data: { indexed: 1, updated: 0, removed: 0 } };
          }
          if (command === "list_tracks" || command === "search_tracks" || command === "browse_source_files") {
            return { ok: true, data: { total: tracks.length, items: tracks } };
          }
          if (command === "get_tracks_by_ids_with_previews") {
            return { ok: true, data: { items: tracks } };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled command: ${command}` } };
        }
      },
      event: { listen }
    };
  });
}

test("source chips show analyzed green on startup and adding a source updates browse without auto scan", async ({ page }) => {
  await installSourceChipAnalysisMock(page);
  await page.goto("/");

  await expect(page.locator(".source-chip.source-chip-analyzed")).toHaveCount(1);
  await page.locator("#addSourceBtn").click();
  await expect.poll(async () => page.evaluate(() => window.__scanCalls)).toBe(0);
  await expect(page.locator(".source-chip.source-chip-analyzed")).toHaveCount(2);
});
