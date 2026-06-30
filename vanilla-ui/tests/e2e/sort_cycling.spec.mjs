import { test, expect } from "@playwright/test";

function installSortMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));

    const tracks = [
      { id: "t1", title: "Zeta", artist: "Bob", album: "A", filePath: "/music/zeta.mp3" },
      { id: "t2", title: "Alpha", artist: "Bob", album: "A", filePath: "/music/alpha.mp3" },
      { id: "t3", title: "Beta", artist: "Alice", album: "A", filePath: "/music/beta.mp3" },
      { id: "t4", title: "Gamma", artist: "Cara", album: "A", filePath: "/music/gamma.mp3" }
    ];

    window.__TAURI__ = {
      core: {
        invoke: async (command) => {
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") return { ok: true, data: { found: false, path: null } };
          if (command === "list_playlists") return { ok: true, data: { items: [] } };
          if (command === "list_tracks" || command === "search_tracks" || command === "browse_source_files") {
            return { ok: true, data: { total: tracks.length, items: tracks } };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          if (command === "get_backend_log_buffer") return [];
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  });
}

async function rowTitles(page) {
  return page.locator("#libraryTableBody .track-grid-row .track-title").allTextContents();
}

test("Sort cycling on Track column", async ({ page }) => {
  await installSortMock(page);
  await page.goto("/");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(4);

  const header = page.locator('#panel-library .sortable[data-sort-key="artist"]');

  await header.click();
  await expect.poll(() => rowTitles(page)).toEqual(["Beta", "Alpha", "Zeta", "Gamma"]);

  await header.click();
  await expect.poll(() => rowTitles(page)).toEqual(["Gamma", "Zeta", "Alpha", "Beta"]);

  await header.click();
  await expect.poll(() => rowTitles(page)).toEqual(["Alpha", "Beta", "Gamma", "Zeta"]);

  await header.click();
  await expect.poll(() => rowTitles(page)).toEqual(["Zeta", "Gamma", "Beta", "Alpha"]);

  await header.click();
  await expect.poll(() => rowTitles(page)).toEqual(["Zeta", "Alpha", "Beta", "Gamma"]);
});
