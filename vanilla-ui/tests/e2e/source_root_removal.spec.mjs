import { test, expect } from "./coverage-fixture.mjs";

function installSourceRemovalMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem(
      "djusbtkit.sourceRoots",
      JSON.stringify(["/music/a", "/music/b"])
    );
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.__scanCalls = [];

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

    const trackStore = [
      {
        id: "a-1",
        title: "A One",
        artist: "Artist A",
        album: "Album A",
        filePath: "/music/a/Artist A - A One.mp3",
        fileSizeBytes: 1000,
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      },
      {
        id: "a-2",
        title: "A Two",
        artist: "Artist A",
        album: "Album A",
        filePath: "/music/a/Artist A - A Two.mp3",
        fileSizeBytes: 1001,
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      },
      {
        id: "b-1",
        title: "B One",
        artist: "Artist B",
        album: `Album B & <b>Two</b>`,
        filePath: "/music/b/Artist B - B One.mp3",
        fileSizeBytes: 1002,
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      }
    ];

    // Ground-truth files that exist on disk under /music/a and /music/b --
    // independent of trackStore (the DB state), which remove_tracks_by_source_roots
    // empties out. scan_library "rediscovers" these and assigns fresh ids, just
    // like the real backend does on re-scan (it never reuses a deleted row's id).
    const filesOnDisk = trackStore.map((t) => ({ ...t, id: `${t.id}-rescanned` }));

    const underRoot = (filePath, root) => {
      const p = String(filePath || "").replace(/\\/g, "/");
      const r = String(root || "").replace(/\\/g, "/").replace(/\/+$/, "");
      return !!r && (p === r || p.startsWith(`${r}/`));
    };

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
            return { ok: true, data: { total: trackStore.length, items: trackStore } };
          }
          if (command === "remove_tracks_by_source_roots") {
            const roots = Array.isArray(payload?.request?.sourceRoots)
              ? payload.request.sourceRoots.map((v) => String(v || "").trim()).filter(Boolean)
              : [];
            const before = trackStore.length;
            for (let i = trackStore.length - 1; i >= 0; i -= 1) {
              const filePath = trackStore[i]?.filePath || "";
              if (roots.some((root) => underRoot(filePath, root))) {
                trackStore.splice(i, 1);
              }
            }
            return { ok: true, data: { removed: before - trackStore.length } };
          }
          if (command === "pick_source_folders") {
            return window.__pickReturn || [];
          }
          if (command === "scan_library") {
            const roots = Array.isArray(payload?.request?.sourceRoots)
              ? payload.request.sourceRoots.map((v) => String(v || "").trim()).filter(Boolean)
              : [];
            window.__scanCalls.push(roots);
            let indexed = 0;
            for (const seed of filesOnDisk) {
              const alreadyIndexed = trackStore.some((t) => t.filePath === seed.filePath);
              if (!alreadyIndexed && roots.some((root) => underRoot(seed.filePath, root))) {
                trackStore.push({ ...seed });
                indexed += 1;
              }
            }
            return { ok: true, data: { indexed, updated: 0, removed: 0, notFound: [] } };
          }
          if (command === "get_tracks_by_ids_with_previews") {
            const ids = Array.isArray(payload?.request?.trackIds)
              ? payload.request.trackIds.map((v) => String(v))
              : [];
            const items = trackStore.filter((t) => ids.includes(String(t.id)));
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

test("removing a source folder deletes corresponding tracks from the library", async ({ page }) => {
  await installSourceRemovalMock(page);
  await page.goto("/");

  await expect(page.locator("#sourceChipsContainer .source-chip")).toHaveCount(2);
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);
  expect(await page.locator("#libraryTableBody .track-grid-row").first().getAttribute("style")).toBeNull();
  await expect(
    page.locator("#libraryTableBody .track-grid-row", { hasText: "B One" }).locator(".td-album")
  ).toHaveText("Album B & <b>Two</b>");

  await page.locator("#sourceChipsContainer .source-chip-remove").first().click();
  await page.locator("#confirmOkBtn").click();

  await expect(page.locator("#sourceChipsContainer .source-chip")).toHaveCount(1);
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);
  await expect(page.locator("#libraryTableBody")).toContainText("B One");
  await expect(page.locator("#libraryTableBody")).not.toContainText("A One");
  await expect(page.locator("#libraryTableBody")).not.toContainText("A Two");
  await expect(page.locator("#statusText")).toContainText("removed 2 track(s)");
});

test("after removing source A, reloading tracks still filters by remaining roots", async ({ page }) => {
  await installSourceRemovalMock(page);
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);

  // Remove source A
  await page.locator("#sourceChipsContainer .source-chip-remove").first().click();
  await page.locator("#confirmOkBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);

  // Simulate a UI action that triggers loadTracks (e.g. search)
  const searchInput = page.locator("#librarySearch");
  if (await searchInput.isVisible()) {
    await searchInput.fill("B");
    await page.waitForTimeout(300); // debounce
    await expect(page.locator("#libraryTableBody")).toContainText("B One");
    await expect(page.locator("#libraryTableBody")).not.toContainText("A One");
  }
});

// Regression coverage for: removing a source folder then re-adding the same
// folder left its tracks visible in the browse view (via a live filesystem
// scan) but with no backing row in the tracks DB, so playback failed with
// "track not found in Library or selected USB" until the user separately
// clicked "Scan Library". The fix indexes newly (re-)added folders
// immediately as part of "Add Source" -- see
// vanilla-ui/components/library/events.mjs addSourceBtn handler and
// backend/src/service/mod.rs scan_library.
test("re-adding a removed source folder indexes it via scan_library so tracks are usable again", async ({ page }) => {
  await installSourceRemovalMock(page);
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);

  await page.locator("#sourceChipsContainer .source-chip-remove").first().click();
  await page.locator("#confirmOkBtn").click();
  await expect(page.locator("#sourceChipsContainer .source-chip")).toHaveCount(1);
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);

  await page.evaluate(() => { window.__pickReturn = ["/music/a"]; });
  await page.locator("#addSourceBtn").click();

  await expect.poll(async () => page.evaluate(() => window.__scanCalls.length)).toBe(1);
  const lastScanRoots = await page.evaluate(() => window.__scanCalls[window.__scanCalls.length - 1]);
  expect(lastScanRoots).toEqual(["/music/a"]);

  await expect(page.locator("#sourceChipsContainer .source-chip")).toHaveCount(2);
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);
  await expect(page.locator("#libraryTableBody")).toContainText("A One");
  await expect(page.locator("#libraryTableBody")).toContainText("A Two");

  // The re-indexed tracks must carry real, freshly-assigned ids from
  // scan_library -- not the raw file path (the pre-fix placeholder id) and
  // not the old, now-deleted row's id -- so playback/resolve_playback_source
  // can find them by id immediately.
  const restoredRow = page.locator("#libraryTableBody .track-grid-row", { hasText: "A One" });
  await expect(restoredRow).toHaveAttribute("data-track-id", "a-1-rescanned");
});
