import { test, expect, openUsb } from "./coverage-fixture.mjs";

// The USB-playlist track view is driven by the shared TrackListController
// (vanilla-ui/components/shared/track_list_controller.mjs) over the
// server-paginated `fetch_usb_playlist_tracks` command: each page arrives
// already hydrated (bpm/key/waveform), and search + sort are backend query
// params so they span the whole playlist, not just the loaded rows. Pages are
// fetched by offset for the part of the list on screen, and only the rows
// around the viewport are rendered. These tests exercise the real built app
// end-to-end.

const PLAYLIST_SIZE = 350; // larger than one page
const PAGE_SIZE = 150; // DEFAULT_PAGE_SIZE in track_list_controller.mjs

// Track 300 (0-indexed) sorts first by title once artists tie -- lets the sort
// test assert a previously-never-rendered track landing in the first page.
// Track 320 gets a unique searchable title for the search test. Both start
// beyond the first page, so neither is fetched until scrolled/sorted/searched.
const SORT_TARGET_INDEX = 300;
const SORT_TARGET_TITLE = "AAAA Sort First";
const SEARCH_TARGET_INDEX = 320;
const SEARCH_TARGET_TITLE = "ZZZZ Unique Search Target";

// A USB stick with one playlist of `trackCount` tracks, each page arriving
// already hydrated (bpm/key/waveform). `gateFetches`: every page waits until
// the test calls `window.__releaseFetch()` -- a slow USB stick (Windows
// Defender scanning each opened file) frozen mid-load. `unhydratedId`: that
// row arrives un-hydrated, and clicking it re-hydrates it via
// `inspect_usb_track`, held until `window.__releaseInspectUsbTrack()`.
function installTauriMock(page, { trackCount, gateFetches = false, unhydratedId = null }) {
  return page.addInitScript(({ trackCount, gateFetches, unhydratedId, targets }) => {
    const backend = window.__e2eBackend;
    const tracks = Array.from({ length: trackCount }, (_, i) => {
      let title = `Track ${String(i).padStart(4, "0")}`;
      if (i === targets.sortIndex) title = targets.sortTitle;
      if (i === targets.searchIndex) title = targets.searchTitle;
      return {
        id: String(i + 1), title, artist: "Same Artist", album: "Album",
        usbMediaPath: `/Contents/Same Artist/Album/track${i + 1}.mp3`
      };
    });
    const hydrate = (t) => (t.id === unhydratedId
      ? { ...t, needsHydration: true }
      : { ...t, bpm: 120, key: "8A", waveformPreview: [10, 20, 30, 40, 30, 20, 10], needsHydration: false });
    backend.install(async (command, r) => {
      if (command === "fetch_usb_playlists") {
        return backend.ok({
          items: [{ id: "usb-1", name: "Big Playlist", source: "mock-tauri", trackCount: tracks.length, tracks: [], totalDurationMs: 0, durationUnknownCount: 0 }],
          stats: { indexedTracks: tracks.length, playlistReferencedTracks: tracks.length, playlistEntries: tracks.length },
          summaryText: `1 playlist, ${tracks.length} tracks`
        });
      }
      if (command === "fetch_usb_playlist_tracks") {
        if (gateFetches) await new Promise((resolve) => { window.__releaseFetch = resolve; });
        return backend.ok({ ...backend.listPage(tracks, r, { map: hydrate }), listSummaryText: `${tracks.length} tracks` });
      }
      if (command === "inspect_usb_track" && unhydratedId) {
        await new Promise((resolve) => { window.__releaseInspectUsbTrack = resolve; });
        // The whole row, as the backend answers.
        const row = tracks.find((t) => t.id === r.trackId);
        return backend.ok({ source: "pdb", track: { ...row, bpm: 128, key: "5A", waveformPreview: [5, 15, 25, 15, 5], needsHydration: false } });
      }
    });
  }, {
    trackCount, gateFetches, unhydratedId,
    targets: { sortIndex: SORT_TARGET_INDEX, sortTitle: SORT_TARGET_TITLE, searchIndex: SEARCH_TARGET_INDEX, searchTitle: SEARCH_TARGET_TITLE }
  });
}

const fetchCalls = (page) =>
  page.evaluate(() => window.__calls.filter((c) => c.command === "fetch_usb_playlist_tracks").map((c) => c.request));

async function selectBigPlaylist(page) {
  await page.goto("/");
  await openUsb(page, "usb-playlists");
  await expect(page.locator('[data-usb-playlist="usb-1"]')).toBeVisible();
  await expect(page.locator("#usbPlaylistTracks .track-grid-row").first()).toBeVisible();
}

function rowByTitle(page, title) {
  return page.locator("#usbPlaylistTracks .track-grid-row").filter({ hasText: title });
}

function scrollToEnd(page) {
  return page.locator('[data-track-grid][data-body-id="usbPlaylistTracks"]').locator("xpath=..")
    .evaluate((el) => { el.scrollTop = el.scrollHeight; });
}

const fetchOffsets = (page) => fetchCalls(page).then((calls) => calls.map((c) => c.offset));

test("large USB playlist paging", async ({ page }) => {
  await installTauriMock(page, { trackCount: PLAYLIST_SIZE });
  await selectBigPlaylist(page);
  // Selecting a large playlist renders the top of the first hydrated page.
  {
    await expect(rowByTitle(page, "Track 0000").locator(".bpm-pill")).toBeVisible();
    // Only the rows around the viewport are in the DOM.
    const rendered = await page.locator("#usbPlaylistTracks .track-grid-row[data-track-id]").count();
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(PAGE_SIZE);
    await expect(rowByTitle(page, SORT_TARGET_TITLE)).toHaveCount(0);
    expect(await fetchOffsets(page)).toEqual([0]);

    // The list shows names only; the open playlist's count is in its heading,
    // and the totals sit on the header line.
    await expect(page.locator('[data-usb-playlist="usb-1"] .playlist-label')).toHaveText("Big Playlist");
    await expect(page.locator("#usbPlaylistHeading .track-list-heading-name")).toHaveText("Big Playlist");
    await expect(page.locator("#usbPlaylistHeading .track-list-heading-count")).toHaveText(`${PLAYLIST_SIZE} tracks`);
    await expect(page.locator(".panel-header #usbCountsText")).toHaveText(/^1 playlist, \d+ tracks?$/);
  }

  await selectBigPlaylist(page);
  // The scrollbar spans the whole playlist from the start.
  {
    const body = page.locator("#usbPlaylistTracks");
    const rowHeight = await body.locator(".track-grid-row").first().evaluate((row) => row.getBoundingClientRect().height);
    const bodyHeight = await body.evaluate((el) => el.getBoundingClientRect().height);
    expect(Math.abs(bodyHeight - PLAYLIST_SIZE * rowHeight)).toBeLessThan(2);
  }

  await selectBigPlaylist(page);
  // Jumping to the end loads just the end of the playlist, hydrated.
  {
    await expect(rowByTitle(page, "Track 0000")).toBeVisible();

    await scrollToEnd(page);

    const lastRow = rowByTitle(page, `Track ${String(PLAYLIST_SIZE - 1).padStart(4, "0")}`);
    await expect(lastRow).toBeVisible();
    await expect(lastRow.locator(".bpm-pill")).toBeVisible();
    // The page in between is never fetched, and the top rows are gone.
    expect(await fetchOffsets(page)).not.toContain(PAGE_SIZE);
    await expect(rowByTitle(page, "Track 0000")).toHaveCount(0);
  }

  await selectBigPlaylist(page);
  // Searching re-queries the backend and shows the filtered-in track, hydrated.
  {
    const queriesBefore = (await fetchCalls(page)).map((c) => c.query || "");
    expect(queriesBefore.every((q) => q === "")).toBe(true);

    await page.locator("#usbTrackSearch").fill(SEARCH_TARGET_TITLE);

    const resultRow = rowByTitle(page, SEARCH_TARGET_TITLE);
    await expect(resultRow).toHaveCount(1);
    await expect(resultRow.locator(".bpm-pill")).toBeVisible();
    const queriesAfter = (await fetchCalls(page)).map((c) => c.query || "");
    expect(queriesAfter).toContain(SEARCH_TARGET_TITLE);
  }

  await selectBigPlaylist(page);
  // Sorting re-queries the backend so the new first page is hydrated.
  {
    // The "Track" column sorts by artist then title (see TrackSort) -- all
    // mock tracks share an artist, so SORT_TARGET_TITLE ("AAAA...") lands first.
    await page.locator('[data-track-grid][data-body-id="usbPlaylistTracks"] .track-grid-cell.sortable[data-sort-key="artist"]').click();

    const sortedRow = rowByTitle(page, SORT_TARGET_TITLE);
    await expect(sortedRow).toHaveCount(1);
    await expect(page.locator("#usbPlaylistTracks .track-grid-row").first()).toHaveText(new RegExp(SORT_TARGET_TITLE));
    await expect(sortedRow.locator(".bpm-pill")).toBeVisible();
    const sortedCalls = (await fetchCalls(page)).map((c) => c.sortBy || "");
    expect(sortedCalls).toContain("artist");
  }
});

test("a small playlist loads in one page with no further fetch", async ({ page }) => {
  await installTauriMock(page, { trackCount: 80 });
  await selectBigPlaylist(page);

  await scrollToEnd(page);
  await expect(rowByTitle(page, "Track 0079").locator(".bpm-pill")).toBeVisible();
  expect(await fetchOffsets(page)).toEqual([0]);
});

async function releaseFetch(page) {
  await page.waitForFunction(() => typeof window.__releaseFetch === "function");
  await page.evaluate(() => {
    const release = window.__releaseFetch;
    window.__releaseFetch = null;
    release();
  });
}

test("a slow page load shows the loading indicator instead of stale rows", async ({ page }) => {
  await installTauriMock(page, { trackCount: PLAYLIST_SIZE, gateFetches: true });
  await page.goto("/");
  await openUsb(page, "usb-playlists");

  const body = page.locator("#usbPlaylistTracks");
  const indicator = page.locator("#usbPlaylistTracks + .track-grid-loading");
  // Switching playlists shows it at once: nothing from a previous list lingers.
  await expect(indicator).toBeVisible();
  await expect(indicator).toHaveText("Loading tracks...");
  await expect(body).toHaveAttribute("aria-busy", "true");
  await expect(body.locator(".track-grid-row:visible")).toHaveCount(0);

  await releaseFetch(page);
  await expect(rowByTitle(page, "Track 0000")).toBeVisible();
  await expect(indicator).toBeHidden();
  await expect(body).not.toHaveAttribute("aria-busy", "true");

  // Jumping to rows not loaded yet shows skeleton rows until their page
  // arrives; the list is busy, but the indicator is only for a new list.
  await scrollToEnd(page);
  await expect(body.locator(".track-grid-row-skeleton").first()).toBeVisible();
  await expect(body).toHaveAttribute("aria-busy", "true");
  await expect(indicator).toBeHidden();

  await releaseFetch(page);
  await expect(rowByTitle(page, `Track ${String(PLAYLIST_SIZE - 1).padStart(4, "0")}`)).toBeVisible();
  await expect(body.locator(".track-grid-row-skeleton")).toHaveCount(0);
  await expect(indicator).toBeHidden();
});

// A row that somehow still isn't fully hydrated: clicking it re-hydrates via
// `inspect_usb_track` (singular), which answers with the whole row, patched
// into just that row's cells.
function installRowClickMock(page) {
  return installTauriMock(page, { trackCount: 5, unhydratedId: "3" });
}

test("clicking an unhydrated track row patches it in place instead of re-rendering the table", async ({ page }) => {
  await installRowClickMock(page);
  await selectBigPlaylist(page);
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(5);

  const targetRow = page.locator('#usbPlaylistTracks .track-grid-row[data-track-id="3"]');
  await expect(targetRow.locator(".bpm-pill")).toHaveCount(0);

  const siblingRow = page.locator('#usbPlaylistTracks .track-grid-row[data-track-id="1"]');
  await siblingRow.evaluate((row) => row.setAttribute("data-test-marker", "keep"));

  await targetRow.click();
  await page.waitForFunction(() => typeof window.__releaseInspectUsbTrack === "function");
  await page.evaluate(() => window.__releaseInspectUsbTrack());
  await expect(targetRow.locator(".bpm-pill")).toHaveText("128.00");
  await expect(targetRow.locator(".key-pill")).toHaveText("5A");
  // Play sits over the cover, and the in-place patch keeps it there.
  await expect(targetRow.locator(".td-cover .cover-play .transport-btn")).toHaveCount(1);
  await expect(targetRow.locator(".td-waveform .transport-btn")).toHaveCount(0);

  await expect(siblingRow).toHaveAttribute("data-test-marker", "keep");
  const singleCalls = await page.evaluate(() =>
    window.__calls.filter((c) => c.command === "inspect_usb_track").map((c) => c.request.trackId));
  expect(singleCalls).toEqual(["3"]);
});
