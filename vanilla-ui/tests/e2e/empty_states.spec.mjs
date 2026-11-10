import { test, expect } from "./coverage-fixture.mjs";

function installEmptyStateMock(page) {
  return page.addInitScript(() => {
    const backend = window.__e2eBackend;
    backend.install((command, request) => {
      if (command === "create_playlist") {
        const answer = backend.answer(command, { request });
        // Its tracks can go after an export, with a removed source folder.
        Object.assign(backend.store.playlists[0], {
          lastExportedAt: "2026-08-06T21:03:26Z", lastExportedUsbRoot: "/usb", lastExportedTrackCount: 52
        });
        return answer;
      }
    });
  });
}

test("Empty states render correctly", async ({ page }) => {
  await installEmptyStateMock(page);
  await page.goto("/");

  await expect(page.locator("#libraryEmptyState")).toContainText("Add a music folder to get started");
  await expect(page.locator("body")).toHaveClass(/library-onboarding/);
  await expect(page.getByRole("button", { name: "Add Folder" })).toBeVisible();

  await page.locator('.nav-item[data-view="usb"]').click();
  await expect(page.locator("#usbEmptyState")).toContainText("Select USB Folder");

  await page.locator("#addPlaylistBtn").click();
  await page.locator("#navPlaylistList .nav-new-input").fill("Empty Playlist");
  await page.locator("#navPlaylistList .nav-new-input").press("Enter");
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistEmptyState")).toContainText("Browse Library or USB to add tracks");
  await expect(page.locator("#playlistExportStatus")).toBeHidden();
  await page.locator("#playlistEmptyState").getByRole("button", { name: "USB", exact: true }).click();
  await expect(page.locator('.nav-item[data-view="usb"]')).toHaveClass(/active/);
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await page.locator("#playlistEmptyState").getByRole("button", { name: "Media Library" }).click();
  await expect(page.locator('.nav-item[data-view="library"]')).toHaveClass(/active/);

  // With a USB selected, the second link goes to its playlists.
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await expect(page.locator('.nav-sub-item[data-view="usb-playlists"]')).toHaveClass(/revealed/);
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistEmptyState .empty-state-action")).toHaveText(["Media Library", "USB Playlists"]);
  await page.locator("#playlistEmptyState").getByRole("button", { name: "USB Playlists" }).click();
  await expect(page.locator("#panel-usb-playlists")).toHaveClass(/active/);
});

test("Configured source folders with zero indexed tracks prompt scan and keep controls visible", async ({ page }) => {
  await installEmptyStateMock(page);
  await page.addInitScript(() => {
    // The library\'s source folders, as `get_library_sources` answers.
    window.__e2eSourceFolders = ["/music"];
  });
  await page.goto("/");

  await expect(page.locator("#libraryEmptyState")).toBeEmpty();
  await expect(page.locator("#libraryContent")).not.toHaveClass(/hidden/);
  await expect(page.locator("#sourceChipsContainer")).toContainText("/music");
  await expect(page.locator("#libraryTableBody")).toContainText("No tracks available.");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveClass(/track-grid-row-empty/);
});

test("A playlist with tracks shows loading, never the empty screen", async ({ page }) => {
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    const tracks = [
      { id: "t1", title: "Loaded One", artist: "Artist", album: "Album", filePath: "/music/one.mp3" },
      { id: "t2", title: "Loaded Two", artist: "Artist", album: "Album", filePath: "/music/two.mp3" }
    ];
    backend.seed({ playlists: [{ id: "pl-empty", name: "Nothing Yet" }, { id: "pl-full", name: "Has Tracks", tracks }] });
    // The full playlist's tracks arrive only when the test releases them.
    let release = null;
    window.__releaseFullPlaylist = () => release?.();
    backend.install(async (command, request) => {
      if (command === "get_playlist_tracks" && request.playlistId === "pl-full" && !request.query) {
        await new Promise((resolve) => { release = resolve; });
      }
    });
  });
  await page.goto("/");

  const items = page.locator("#navPlaylistList .nav-playlist-item");
  await items.filter({ hasText: "Nothing Yet" }).click();
  await expect(page.locator("#playlistEmptyState")).toContainText("Browse Library or USB to add tracks");

  // From an empty playlist to one with tracks: the empty screen goes at once
  // and the table shows it's loading.
  await items.filter({ hasText: "Has Tracks" }).click();
  await expect(page.locator("#playlistEmptyState")).toBeEmpty();
  await expect(page.locator("#playlistTableWrap")).toBeVisible();
  await expect(page.locator("#playlistTableWrap .track-grid-loading")).toBeVisible();

  await page.evaluate(() => window.__releaseFullPlaylist());
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);
  await expect(page.locator("#playlistTableWrap .track-grid-loading")).toBeHidden();

  // No search match isn't an empty playlist.
  await page.locator("#playlistSearchInput").fill("zzz");
  await expect(page.locator("#playlistTracksBody")).toContainText("No tracks available.");
  await expect(page.locator("#playlistEmptyState")).toBeEmpty();
  await expect(page.locator("#playlistSearchInput")).toBeVisible();
});
