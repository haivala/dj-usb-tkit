import { test, expect } from "./coverage-fixture.mjs";

// Large (well over LARGE_USB_SELECTION_THRESHOLD, see components/usb/events.mjs)
// USB playlists render/hydrate one page at a time instead of all at once --
// rendering thousands of DOM rows in one go was causing a total UI freeze.
// That pagination decoupled "which tracks are rendered" from "which tracks
// are hydrated" (bpm/key/waveform/artwork fetched), and hydration-triggering
// was only wired at the two places that render a page inside
// components/usb/events.mjs's closure (selection, scroll-load-more) --
// search and sort (the latter routed through a completely separate,
// generic, delegated click handler with no access to that closure) never
// triggered hydration for newly-visible tracks. Fixed by moving hydration
// into the render functions themselves (components/usb/actions.mjs), so
// every render path gets it automatically. These tests exercise the real
// built app end-to-end -- exactly the class of bug three previous rounds of
// mocked-ctx unit tests failed to catch.

const PLAYLIST_SIZE = 350; // over the 300-track pagination threshold
const PAGE_SIZE = 150; // must match USB_SELECTION_PAGE_SIZE in events.mjs

// Track 300 (0-indexed) starts with "AAAA" so sorting the Track column
// (title, once artists all tie) puts it first -- letting the sort test
// assert on a specific, previously-never-rendered/hydrated track landing in
// the newly-visible first page. Track 320 gets a unique searchable title
// for the search test, for the same reason. Both start beyond the initial
// page (index < 150), so neither is hydrated until scrolled/sorted/searched
// into view.
const SORT_TARGET_INDEX = 300;
const SORT_TARGET_TITLE = "AAAA Sort First";
const SEARCH_TARGET_INDEX = 320;
const SEARCH_TARGET_TITLE = "ZZZZ Unique Search Target";

function installTauriMock(page, { trackCount }) {
  return page.addInitScript(({ trackCount, sortTargetIndex, sortTargetTitle, searchTargetIndex, searchTargetTitle }) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.__inspectCalls = [];

    const tracks = Array.from({ length: trackCount }, (_, i) => {
      let title = `Track ${String(i).padStart(4, "0")}`;
      if (i === sortTargetIndex) title = sortTargetTitle;
      if (i === searchTargetIndex) title = searchTargetTitle;
      return { id: String(i + 1), title, artist: "Same Artist", album: "Album" };
    });
    const tracksById = new Map(tracks.map((t) => [t.id, t]));

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
          if (command === "list_usb_devices") return { ok: true, data: { items: [] } };
          if (command === "search_tracks") return { ok: true, data: { total: 0, items: [] } };
          if (command === "list_tracks") return { ok: true, data: { total: 0, items: [] } };
          if (command === "fetch_usb_histories") return { ok: true, data: { items: [], warnings: [] } };
          if (command === "validate_usb_root") {
            const path = String(payload?.request?.path || "");
            if (!path) {
              return {
                ok: true,
                data: {
                  valid: false, hasWriteAccess: false, normalizedRoot: null,
                  hasVendorRoot: false, hasContents: false, hasPdb: false, hasEdb: false,
                  warnings: ["USB path is empty"]
                }
              };
            }
            return {
              ok: true,
              data: {
                valid: true, hasWriteAccess: true, normalizedRoot: path,
                hasVendorRoot: true, hasContents: true, hasPdb: true, hasEdb: true,
                warnings: []
              }
            };
          }
          if (command === "fetch_usb_playlists") {
            return {
              ok: true,
              data: {
                items: [
                  {
                    id: "usb-1",
                    name: "Big Playlist",
                    source: "mock-tauri",
                    tracks,
                    trackCount: tracks.length,
                    totalDurationMs: 0,
                    durationKnownCount: 0
                  }
                ],
                stats: { indexedTracks: tracks.length, playlistReferencedTracks: tracks.length, playlistEntries: tracks.length },
                warnings: []
              }
            };
          }
          if (command === "inspect_usb_tracks") {
            const items = payload?.request?.items || [];
            window.__inspectCalls.push(items.map((item) => item.trackId));
            return {
              ok: true,
              data: {
                items: items.map((item) => {
                  const base = tracksById.get(String(item.trackId));
                  if (!base) return { trackId: item.trackId, source: null, track: null };
                  return {
                    trackId: item.trackId,
                    source: "pdb",
                    track: {
                      ...base,
                      bpm: 120,
                      key: "8A",
                      waveformPreview: [10, 20, 30, 40, 30, 20, 10]
                    }
                  };
                }),
                warnings: []
              }
            };
          }
          return { ok: false, error: { code: "INTERNAL_ERROR", message: `Unknown mock command: ${command}` } };
        }
      }
    };
  }, { trackCount, sortTargetIndex: SORT_TARGET_INDEX, sortTargetTitle: SORT_TARGET_TITLE, searchTargetIndex: SEARCH_TARGET_INDEX, searchTargetTitle: SEARCH_TARGET_TITLE });
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

test("selecting a large playlist renders and hydrates only the first page", async ({ page }) => {
  await installTauriMock(page, { trackCount: PLAYLIST_SIZE });
  await selectBigPlaylist(page);

  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(PAGE_SIZE);
  await expect(rowByTitle(page, "Track 0000").locator(".bpm-pill")).toBeVisible();

  // Track 300 (beyond the first page) shouldn't even be in the DOM yet.
  await expect(rowByTitle(page, SORT_TARGET_TITLE)).toHaveCount(0);
});

test("scrolling near the bottom loads and hydrates the next page", async ({ page }) => {
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

test("searching hydrates a newly-filtered-in track that was never rendered before", async ({ page }) => {
  await installTauriMock(page, { trackCount: PLAYLIST_SIZE });
  await selectBigPlaylist(page);

  // Confirm the search target hasn't been hydrated (it's never even been
  // rendered yet, being beyond the first page).
  const callsBeforeSearch = await page.evaluate(() => window.__inspectCalls.flat());
  expect(callsBeforeSearch).not.toContain(String(SEARCH_TARGET_INDEX + 1));

  await page.locator("#usbTrackSearch").fill(SEARCH_TARGET_TITLE);

  const resultRow = rowByTitle(page, SEARCH_TARGET_TITLE);
  await expect(resultRow).toHaveCount(1);
  await expect(resultRow.locator(".bpm-pill")).toBeVisible();
});

test("sorting hydrates whatever lands in the newly-visible first page", async ({ page }) => {
  await installTauriMock(page, { trackCount: PLAYLIST_SIZE });
  await selectBigPlaylist(page);

  const callsBeforeSort = await page.evaluate(() => window.__inspectCalls.flat());
  expect(callsBeforeSort).not.toContain(String(SORT_TARGET_INDEX + 1));

  // The "Track" column sorts by artist then falls back to title when
  // artists tie (see sortTracks in track_table.mjs) -- all mock tracks
  // share the same artist, so this sorts by title, putting SORT_TARGET_TITLE
  // ("AAAA...") first.
  await page.locator('[data-track-grid][data-body-id="usbPlaylistTracks"] .track-grid-cell.sortable[data-sort-key="artist"]').click();

  const sortedRow = rowByTitle(page, SORT_TARGET_TITLE);
  await expect(sortedRow).toHaveCount(1);
  await expect(page.locator("#usbPlaylistTracks .track-grid-row").first()).toHaveText(new RegExp(SORT_TARGET_TITLE));
  await expect(sortedRow.locator(".bpm-pill")).toBeVisible();
});

test("a normal (~80-track) playlist is not paginated and hydrates in one pass", async ({ page }) => {
  await installTauriMock(page, { trackCount: 80 });
  await selectBigPlaylist(page);

  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(80);
  const calls = await page.evaluate(() => window.__inspectCalls);
  expect(calls.length).toBe(1);
  expect(calls[0].length).toBe(80);
  await expect(rowByTitle(page, "Track 0079").locator(".bpm-pill")).toBeVisible();
});

// Clicking a track row re-hydrates that one track (belt-and-suspenders for
// a track that somehow still isn't fully hydrated once visible) and patches
// just that row's cells in place -- it must not blow away and rebuild the
// whole table, which would cost more render time and reset scroll/focus for
// no reason. If the row is no longer present by the time hydration resolves
// (e.g. it scrolled/filtered out), the code falls back to a full re-render
// instead of silently doing nothing. Track id "3" is deliberately left
// unhydrated by the initial batch hydration mock below to exercise both
// paths.
function installRowClickMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.__singleInspectCalls = [];

    const tracks = Array.from({ length: 5 }, (_, i) => ({
      id: String(i + 1),
      title: `Track ${String(i).padStart(4, "0")}`,
      artist: "Same Artist",
      album: "Album"
    }));

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
          if (command === "list_usb_devices") return { ok: true, data: { items: [] } };
          if (command === "search_tracks") return { ok: true, data: { total: 0, items: [] } };
          if (command === "list_tracks") return { ok: true, data: { total: 0, items: [] } };
          if (command === "fetch_usb_histories") return { ok: true, data: { items: [], warnings: [] } };
          if (command === "validate_usb_root") {
            const path = String(payload?.request?.path || "");
            if (!path) {
              return {
                ok: true,
                data: {
                  valid: false, hasWriteAccess: false, normalizedRoot: null,
                  hasVendorRoot: false, hasContents: false, hasPdb: false, hasEdb: false,
                  warnings: ["USB path is empty"]
                }
              };
            }
            return {
              ok: true,
              data: {
                valid: true, hasWriteAccess: true, normalizedRoot: path,
                hasVendorRoot: true, hasContents: true, hasPdb: true, hasEdb: true,
                warnings: []
              }
            };
          }
          if (command === "fetch_usb_playlists") {
            return {
              ok: true,
              data: {
                items: [
                  {
                    id: "usb-1",
                    name: "Small Playlist",
                    source: "mock-tauri",
                    tracks,
                    trackCount: tracks.length,
                    totalDurationMs: 0,
                    durationKnownCount: 0
                  }
                ],
                stats: { indexedTracks: tracks.length, playlistReferencedTracks: tracks.length, playlistEntries: tracks.length },
                warnings: []
              }
            };
          }
          if (command === "inspect_usb_tracks") {
            // Initial page-render hydration: every track hydrates except
            // id "3", which the backend "misses" -- simulating the kind of
            // gap the click-to-patch path exists to paper over.
            const items = payload?.request?.items || [];
            return {
              ok: true,
              data: {
                items: items.map((item) => {
                  if (String(item.trackId) === "3") {
                    return { trackId: item.trackId, source: null, track: null };
                  }
                  return {
                    trackId: item.trackId,
                    source: "pdb",
                    track: { bpm: 120, key: "8A", waveformPreview: [10, 20, 30, 20, 10] }
                  };
                }),
                warnings: []
              }
            };
          }
          if (command === "inspect_usb_track") {
            window.__singleInspectCalls.push(payload?.request?.trackId);
            // Wait for the test to explicitly release this call instead of a
            // fixed timer -- a fixed delay races against however long the
            // test's own click()/evaluate() round trip happens to take, and
            // is not a reliable way to guarantee the DOM mutation below
            // happens before the patch attempt runs.
            await new Promise((resolve) => { window.__releaseInspectUsbTrack = resolve; });
            return {
              ok: true,
              data: { source: "pdb", track: { bpm: 128, key: "5A", waveformPreview: [5, 15, 25, 15, 5] } }
            };
          }
          return { ok: false, error: { code: "INTERNAL_ERROR", message: `Unknown mock command: ${command}` } };
        }
      }
    };
  });
}

test("clicking an unhydrated track row patches it in place instead of re-rendering the table", async ({ page }) => {
  await installRowClickMock(page);
  await selectBigPlaylist(page);
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(5);

  const targetRow = page.locator('#usbPlaylistTracks .track-grid-row[data-track-id="3"]');
  await expect(targetRow.locator(".bpm-pill")).toHaveCount(0);

  // Tag a sibling row's live DOM node -- a full re-render rebuilds the
  // tbody's innerHTML and would wipe this marker; an in-place cell patch of
  // only the clicked row leaves every other row's node untouched.
  const siblingRow = page.locator('#usbPlaylistTracks .track-grid-row[data-track-id="1"]');
  await siblingRow.evaluate((row) => row.setAttribute("data-test-marker", "keep"));

  await targetRow.click();
  await page.waitForFunction(() => typeof window.__releaseInspectUsbTrack === "function");
  await page.evaluate(() => window.__releaseInspectUsbTrack());
  await expect(targetRow.locator(".bpm-pill")).toHaveText("128");
  await expect(targetRow.locator(".key-pill")).toHaveText("5A");

  await expect(siblingRow).toHaveAttribute("data-test-marker", "keep");
  const singleCalls = await page.evaluate(() => window.__singleInspectCalls);
  expect(singleCalls).toEqual(["3"]);
});

test("falls back to a full re-render when the clicked row is gone by the time hydration resolves", async ({ page }) => {
  await installRowClickMock(page);
  await selectBigPlaylist(page);
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(5);

  const targetRow = page.locator('#usbPlaylistTracks .track-grid-row[data-track-id="3"]');
  await targetRow.click();
  await page.waitForFunction(() => typeof window.__releaseInspectUsbTrack === "function");

  // Rip the row out of the DOM while the click handler's hydration request
  // is still in flight -- when it resolves, patchUsbTrackRow's lookup for
  // data-track-id="3" finds nothing, so the code must fall back to
  // rebuilding the whole table from state instead of leaving the row gone.
  await page.locator("#usbPlaylistTracks").evaluate((container) => {
    container.querySelector('.track-grid-row[data-track-id="3"]')?.remove();
  });
  await expect(page.locator('#usbPlaylistTracks .track-grid-row[data-track-id="3"]')).toHaveCount(0);
  await page.evaluate(() => window.__releaseInspectUsbTrack());

  const recoveredRow = page.locator('#usbPlaylistTracks .track-grid-row[data-track-id="3"]');
  await expect(recoveredRow).toHaveCount(1);
  await expect(recoveredRow.locator(".bpm-pill")).toHaveText("128");
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(5);
});
