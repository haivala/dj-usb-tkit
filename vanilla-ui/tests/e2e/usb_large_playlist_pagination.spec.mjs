import { test, expect } from "./coverage-fixture.mjs";

// The USB-playlist track view is driven by the shared TrackListController
// (vanilla-ui/components/shared/track_list_controller.mjs) over the
// server-paginated `fetch_usb_playlist_tracks` command: each page arrives
// already hydrated (bpm/key/waveform), and search + sort are backend query
// params so they span the whole playlist, not just the loaded rows. These
// tests exercise the real built app end-to-end.

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

// One `fetch_usb_playlist_tracks` implementation shared by the mocks below.
// `hydrate(track)` decides what a returned (page) row looks like.
function playlistTracksResponder(tracks, hydrate) {
  return (payload) => {
    const req = payload?.request || {};
    let rows = tracks.slice();
    const q = String(req.query || "").trim().toLowerCase();
    if (q) {
      rows = rows.filter((t) => `${t.title} ${t.artist} ${t.album || ""}`.toLowerCase().includes(q));
    }
    if (req.sortBy) {
      const dir = req.sortDir === "desc" ? -1 : 1;
      rows = rows.slice().sort((a, b) => {
        // Mirrors the backend sort_usb_tracks: "artist" ties break by title.
        const av = req.sortBy === "artist" ? `${a.artist}\0${a.title}` : String(a[req.sortBy] ?? "");
        const bv = req.sortBy === "artist" ? `${b.artist}\0${b.title}` : String(b[req.sortBy] ?? "");
        return av < bv ? -dir : av > bv ? dir : 0;
      });
    }
    const total = rows.length;
    const offset = Number(req.cursor || 0);
    const limit = Number(req.limit || 0) || total;
    const page = rows.slice(offset, offset + limit).map((t) => hydrate(t));
    const nextOffset = offset + page.length;
    return {
      ok: true,
      data: {
        items: page,
        total,
        hasMore: nextOffset < total,
        nextCursor: nextOffset < total ? String(nextOffset) : null,
        totalDurationMs: 0,
        durationKnownCount: 0,
        warnings: [],
      },
    };
  };
}

function baseUsbCommands(payload, command) {
  if (command === "clear_frontend_log") return "";
  if (command === "append_frontend_log") return null;
  if (command === "show_window") return null;
  if (command === "detect_external_rekordbox_db") return { ok: true, data: { found: false, path: null } };
  if (command === "pick_usb_folder") return "/Volumes/USB-TEST";
  if (command === "list_playlists") return { ok: true, data: { items: [] } };
  if (command === "list_usb_devices") return { ok: true, data: { items: [] } };
  if (command === "search_tracks") return { ok: true, data: { total: 0, items: [] } };
  if (command === "list_tracks") return { ok: true, data: { total: 0, items: [] } };
  if (command === "fetch_usb_histories") return { ok: true, data: { items: [], warnings: [] } };
  if (command === "validate_usb_root") {
    const path = String(payload?.request?.path || "");
    return {
      ok: true,
      data: path
        ? { valid: true, hasWriteAccess: true, normalizedRoot: path, hasVendorRoot: true, hasContents: true, hasPdb: true, hasEdb: true, warnings: [] }
        : { valid: false, hasWriteAccess: false, normalizedRoot: null, hasVendorRoot: false, hasContents: false, hasPdb: false, hasEdb: false, warnings: ["USB path is empty"] },
    };
  }
  return undefined;
}

// `gateFetches`: every `fetch_usb_playlist_tracks` waits until the test calls
// `window.__releaseFetch()` -- a slow USB stick (Windows Defender scanning
// each opened file) frozen mid-load.
function installTauriMock(page, { trackCount, gateFetches = false }) {
  return page.addInitScript(({ trackCount, gateFetches, sortTargetIndex, sortTargetTitle, searchTargetIndex, searchTargetTitle }) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.__fetchCalls = [];

    const tracks = Array.from({ length: trackCount }, (_, i) => {
      let title = `Track ${String(i).padStart(4, "0")}`;
      if (i === sortTargetIndex) title = sortTargetTitle;
      if (i === searchTargetIndex) title = searchTargetTitle;
      return { id: String(i + 1), title, artist: "Same Artist", album: "Album" };
    });

    const base = window.__usbSpecHelpers.baseUsbCommands;
    const responder = window.__usbSpecHelpers.playlistTracksResponder(
      tracks,
      (t) => ({ ...t, bpm: 120, key: "8A", waveformPreview: [10, 20, 30, 40, 30, 20, 10] })
    );

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const b = base(payload, command);
          if (b !== undefined) return b;
          if (command === "fetch_usb_playlists") {
            return {
              ok: true,
              data: {
                items: [{ id: "usb-1", name: "Big Playlist", source: "mock-tauri", trackCount: tracks.length, tracks: [], totalDurationMs: 0, durationKnownCount: 0 }],
                stats: { indexedTracks: tracks.length, playlistReferencedTracks: tracks.length, playlistEntries: tracks.length },
                warnings: [],
              },
            };
          }
          if (command === "fetch_usb_playlist_tracks") {
            window.__fetchCalls.push({ ...(payload?.request || {}) });
            if (gateFetches) await new Promise((resolve) => { window.__releaseFetch = resolve; });
            return responder(payload);
          }
          return { ok: false, error: { code: "INTERNAL_ERROR", message: `Unknown mock command: ${command}` } };
        },
      },
    };
  }, { trackCount, gateFetches, sortTargetIndex: SORT_TARGET_INDEX, sortTargetTitle: SORT_TARGET_TITLE, searchTargetIndex: SEARCH_TARGET_INDEX, searchTargetTitle: SEARCH_TARGET_TITLE });
}

// The spec helpers (plain functions) can't cross into addInitScript's serialized
// closure, so re-expose them on window from a second init script.
function installSpecHelpers(page) {
  return page.addInitScript(`
    window.__usbSpecHelpers = {
      baseUsbCommands: ${baseUsbCommands.toString()},
      playlistTracksResponder: ${playlistTracksResponder.toString()},
    };
  `);
}

async function selectBigPlaylist(page) {
  await page.goto("/");
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await page.locator("#refreshUsbBtn").click();
  await expect(page.locator('[data-usb-playlist="usb-1"]')).toBeVisible();
  await page.locator('[data-usb-playlist="usb-1"]').click();
  await expect(page.locator("#usbPlaylistTracks .track-grid-row").first()).toBeVisible();
}

function rowByTitle(page, title) {
  return page.locator("#usbPlaylistTracks .track-grid-row").filter({ hasText: title });
}

test("selecting a large playlist renders the first hydrated page", async ({ page }) => {
  await installSpecHelpers(page);
  await installTauriMock(page, { trackCount: PLAYLIST_SIZE });
  await selectBigPlaylist(page);

  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(PAGE_SIZE);
  await expect(rowByTitle(page, "Track 0000").locator(".bpm-pill")).toBeVisible();
  await expect(rowByTitle(page, SORT_TARGET_TITLE)).toHaveCount(0);

  // The list shows names only; the open playlist's count is in its heading,
  // and the totals sit on the header line.
  await expect(page.locator('[data-usb-playlist="usb-1"] .playlist-label')).toHaveText("Big Playlist");
  await expect(page.locator("#usbPlaylistHeading .track-list-heading-name")).toHaveText("Big Playlist");
  await expect(page.locator("#usbPlaylistHeading .track-list-heading-count")).toHaveText(`${PLAYLIST_SIZE} tracks`);
  await expect(page.locator(".panel-header #usbCountsText")).toHaveText(/^1 playlists, \d+ tracks$/);
});

test("scrolling near the bottom loads the next hydrated page", async ({ page }) => {
  await installSpecHelpers(page);
  await installTauriMock(page, { trackCount: PLAYLIST_SIZE });
  await selectBigPlaylist(page);
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(PAGE_SIZE);

  const wrap = page.locator('[data-track-grid][data-body-id="usbPlaylistTracks"]').locator("xpath=..");
  await wrap.evaluate((el) => { el.scrollTop = el.scrollHeight; });

  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(Math.min(PLAYLIST_SIZE, PAGE_SIZE * 2));
  const row200 = rowByTitle(page, "Track 0200");
  await expect(row200).toBeVisible();
  await expect(row200.locator(".bpm-pill")).toBeVisible();
});

test("searching re-queries the backend and shows the filtered-in track, hydrated", async ({ page }) => {
  await installSpecHelpers(page);
  await installTauriMock(page, { trackCount: PLAYLIST_SIZE });
  await selectBigPlaylist(page);

  const queriesBefore = await page.evaluate(() => window.__fetchCalls.map((c) => c.query || ""));
  expect(queriesBefore.every((q) => q === "")).toBe(true);

  await page.locator("#usbTrackSearch").fill(SEARCH_TARGET_TITLE);

  const resultRow = rowByTitle(page, SEARCH_TARGET_TITLE);
  await expect(resultRow).toHaveCount(1);
  await expect(resultRow.locator(".bpm-pill")).toBeVisible();
  const queriesAfter = await page.evaluate(() => window.__fetchCalls.map((c) => c.query || ""));
  expect(queriesAfter).toContain(SEARCH_TARGET_TITLE);
});

test("sorting re-queries the backend so the new first page is hydrated", async ({ page }) => {
  await installSpecHelpers(page);
  await installTauriMock(page, { trackCount: PLAYLIST_SIZE });
  await selectBigPlaylist(page);

  // The "Track" column sorts by artist then title (see sort_usb_tracks) -- all
  // mock tracks share an artist, so SORT_TARGET_TITLE ("AAAA...") lands first.
  await page.locator('[data-track-grid][data-body-id="usbPlaylistTracks"] .track-grid-cell.sortable[data-sort-key="artist"]').click();

  const sortedRow = rowByTitle(page, SORT_TARGET_TITLE);
  await expect(sortedRow).toHaveCount(1);
  await expect(page.locator("#usbPlaylistTracks .track-grid-row").first()).toHaveText(new RegExp(SORT_TARGET_TITLE));
  await expect(sortedRow.locator(".bpm-pill")).toBeVisible();
  const sortedCalls = await page.evaluate(() => window.__fetchCalls.map((c) => c.sortBy || ""));
  expect(sortedCalls).toContain("artist");
});

test("a small playlist loads in one page with no further fetch", async ({ page }) => {
  await installSpecHelpers(page);
  await installTauriMock(page, { trackCount: 80 });
  await selectBigPlaylist(page);

  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(80);
  await expect(rowByTitle(page, "Track 0079").locator(".bpm-pill")).toBeVisible();
  const calls = await page.evaluate(() => window.__fetchCalls);
  expect(calls.length).toBe(1);
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
  await installSpecHelpers(page);
  await installTauriMock(page, { trackCount: PLAYLIST_SIZE, gateFetches: true });
  await page.goto("/");
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await page.locator("#refreshUsbBtn").click();
  await page.locator('[data-usb-playlist="usb-1"]').click();

  const body = page.locator("#usbPlaylistTracks");
  const indicator = page.locator("#usbPlaylistTracks + .track-grid-loading");
  // Switching playlists shows it at once: nothing from a previous list lingers.
  await expect(indicator).toBeVisible();
  await expect(indicator).toHaveText("Loading tracks…");
  await expect(body).toHaveAttribute("aria-busy", "true");
  await expect(body.locator(".track-grid-row:visible")).toHaveCount(0);

  await releaseFetch(page);
  await expect(body.locator(".track-grid-row")).toHaveCount(PAGE_SIZE);
  await expect(indicator).toBeHidden();
  await expect(body).not.toHaveAttribute("aria-busy", "true");

  // Scrolling for the next page keeps the loaded rows and shows the
  // indicator below them until the page arrives.
  const wrap = page.locator('[data-track-grid][data-body-id="usbPlaylistTracks"]').locator("xpath=..");
  await wrap.evaluate((el) => { el.scrollTop = el.scrollHeight; });
  await expect(indicator).toBeVisible();
  await expect(body.locator(".track-grid-row:visible")).toHaveCount(PAGE_SIZE);

  await releaseFetch(page);
  await expect(body.locator(".track-grid-row")).toHaveCount(PAGE_SIZE * 2);
  await expect(indicator).toBeHidden();
});

// A row that somehow still isn't fully hydrated: clicking it re-hydrates via
// `inspect_usb_track` (singular) and patches just that row's cells in place;
// if the row is gone by the time hydration resolves it full-rerenders.
// Track id "3" is left un-hydrated by the page fetch to exercise both paths.
function installRowClickMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.__singleInspectCalls = [];

    const tracks = Array.from({ length: 5 }, (_, i) => ({ id: String(i + 1), title: `Track ${String(i).padStart(4, "0")}`, artist: "Same Artist", album: "Album" }));
    const base = window.__usbSpecHelpers.baseUsbCommands;
    const responder = window.__usbSpecHelpers.playlistTracksResponder(
      tracks,
      (t) => (t.id === "3"
        ? { ...t, needsHydration: true }
        : { ...t, bpm: 120, key: "8A", waveformPreview: [10, 20, 30, 20, 10], needsHydration: false })
    );

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const b = base(payload, command);
          if (b !== undefined) return b;
          if (command === "fetch_usb_playlists") {
            return {
              ok: true,
              data: {
                items: [{ id: "usb-1", name: "Small Playlist", source: "mock-tauri", trackCount: tracks.length, tracks: [], totalDurationMs: 0, durationKnownCount: 0 }],
                stats: { indexedTracks: tracks.length, playlistReferencedTracks: tracks.length, playlistEntries: tracks.length },
                warnings: [],
              },
            };
          }
          if (command === "fetch_usb_playlist_tracks") return responder(payload);
          if (command === "inspect_usb_track") {
            window.__singleInspectCalls.push(payload?.request?.trackId);
            await new Promise((resolve) => { window.__releaseInspectUsbTrack = resolve; });
            return { ok: true, data: { source: "pdb", track: { bpm: 128, key: "5A", waveformPreview: [5, 15, 25, 15, 5] } } };
          }
          return { ok: false, error: { code: "INTERNAL_ERROR", message: `Unknown mock command: ${command}` } };
        },
      },
    };
  });
}

test("clicking an unhydrated track row patches it in place instead of re-rendering the table", async ({ page }) => {
  await installSpecHelpers(page);
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
  const singleCalls = await page.evaluate(() => window.__singleInspectCalls);
  expect(singleCalls).toEqual(["3"]);
});

test("falls back to a full re-render when the clicked row is gone by the time hydration resolves", async ({ page }) => {
  await installSpecHelpers(page);
  await installRowClickMock(page);
  await selectBigPlaylist(page);
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(5);

  const targetRow = page.locator('#usbPlaylistTracks .track-grid-row[data-track-id="3"]');
  await targetRow.click();
  await page.waitForFunction(() => typeof window.__releaseInspectUsbTrack === "function");

  await page.locator("#usbPlaylistTracks").evaluate((container) => {
    container.querySelector('.track-grid-row[data-track-id="3"]')?.remove();
  });
  await expect(page.locator('#usbPlaylistTracks .track-grid-row[data-track-id="3"]')).toHaveCount(0);
  await page.evaluate(() => window.__releaseInspectUsbTrack());

  const recoveredRow = page.locator('#usbPlaylistTracks .track-grid-row[data-track-id="3"]');
  await expect(recoveredRow).toHaveCount(1);
  await expect(recoveredRow.locator(".bpm-pill")).toHaveText("128.00");
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(5);
});
