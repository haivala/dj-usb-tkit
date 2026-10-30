import { test, expect } from "./coverage-fixture.mjs";

// `usb`: a connected stick with one playlist of two tracks.
function installSelectionMock(page, mockOpts = {}) {
  return page.addInitScript((opts) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    // The library\'s source folders, as `get_library_sources` answers.
    window.__e2eSourceFolders = ["/music"];
    if (opts.usb) window.localStorage.setItem("djusbtkit.usbRoot", "/Volumes/USB-TEST");
    window.__calls = [];
    const usbTracks = ["Stick A", "Stick B"].map((title, i) => ({
      id: String(101 + i),
      title,
      artist: "Stick Artist",
      album: "Stick Album",
      filePath: `/Volumes/USB-TEST/Contents/stick-${i}.mp3`,
      usbMediaPath: `/Contents/stick-${i}.mp3`,
      // The first is matched to a library track; the second never was.
      localTrackId: i === 0 ? "t1" : null,
    }));

    const tracks = [
      { id: "t1", title: "Sel One", artist: "Artist", album: "Album", filePath: "/music/one.mp3" },
      { id: "t2", title: "Sel Two", artist: "Artist", album: "Album", filePath: "/music/two.mp3" },
      // No bpm field -- mirrors a real not-yet-analyzed library track. Regression coverage for
      // the "invalid type: string \"\", expected f64" crash: add_track_candidates_to_playlist
      // below rejects a non-numeric bpm the way the real Rust backend's serde does, so this only
      // stays green if the frontend never forwards a UI display sentinel like "" as bpm.
      { id: "t3", title: "Sel Three", artist: "Artist", album: "Album", filePath: "/music/three.mp3" }
    ];
    const playlists = [];
    const playlistTracks = new Map();
    const addIds = (playlistId, ids) => {
      const current = playlistTracks.get(playlistId) || [];
      const fresh = ids.filter((id, i) => !current.includes(id) && ids.indexOf(id) === i);
      playlistTracks.set(playlistId, [...current, ...fresh]);
      return { playlistId, added: fresh.length, skipped: ids.length - fresh.length };
    };
    const addToPlaylists = (playlistIds, ids, requested) => {
      const items = playlistIds.map((playlistId) => addIds(playlistId, ids));
      const sum = (key) => items.reduce((total, item) => total + item[key], 0);
      return {
        ok: true,
        data: {
          requested, resolved: ids.length, unresolved: requested - ids.length,
          added: sum("added"), skipped: sum("skipped"), items, resolutions: [],
        },
      };
    };

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "get_library_sources") {
            const folders = (window.__e2eSourceFolders || [])
              .map((path) => ({ path, label: path, parent: null, enabled: true, missing: false, fullyAnalyzed: false }));
            return { ok: true, data: { folders, rekordboxDbEnabled: false, mixxxDbEnabled: false, everConfigured: folders.length > 0 } };
          }
          window.__calls.push({ command, request: payload?.request ?? null });
          if (command === "get_playlist_membership") {
            const ids = payload?.request?.trackIds || [];
            const items = playlists.map((p) => {
              const held = playlistTracks.get(p.id) || [];
              const contained = ids.filter((id) => id && held.includes(id)).length;
              const state = contained === 0 ? "none" : contained >= ids.length ? "all" : "some";
              return { playlistId: p.id, name: p.name, contained, state };
            });
            return { ok: true, data: { selected: ids.length, items } };
          }
          if (command === "add_library_selection_to_playlists") {
            const r = payload?.request || {};
            return addToPlaylists(r.playlistIds, r.trackIds.map(String), r.trackIds.length);
          }
          if (command === "add_track_candidates_to_playlists") {
            const r = payload?.request || {};
            const ids = r.tracks.map((t) => String(t.localTrackId || t.trackId || "")).filter(Boolean);
            return addToPlaylists(r.playlistIds, ids, r.tracks.length);
          }
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_rekordbox_db") return { ok: true, data: { found: false, path: null } };
          if (command === "list_playlists") return { ok: true, data: { items: playlists } };
          if (command === "browse_source_files") {
            // Library search is a backend query param now (shared TrackListController).
            const q = String(payload?.request?.query || "").trim().toLowerCase();
            const rows = q
              ? tracks.filter((t) => `${t.title} ${t.artist} ${t.album || ""}`.toLowerCase().includes(q))
              : tracks;
            return { ok: true, data: { total: rows.length, items: rows, nextCursor: null, hasMore: false } };
          }
          if (command === "create_playlist") {
            const name = String(payload?.request?.name || "Untitled");
            const id = `pl-${playlists.length + 1}`;
            const row = {
              id, name, source: "local",
              lastExportedAt: null, lastExportedUsbRoot: null, lastExportedTrackCount: null,
              tracks: []
            };
            playlists.push(row);
            playlistTracks.set(id, []);
            return { ok: true, data: { playlistId: id, name } };
          }
          if (command === "list_matching_track_ids") {
            const q = String(payload?.request?.query || "").trim().toLowerCase();
            const rows = q
              ? tracks.filter((t) => `${t.title} ${t.artist} ${t.album || ""}`.toLowerCase().includes(q))
              : tracks;
            const ids = rows.map((t) => t.id);
            return { ok: true, data: { trackIds: ids, total: ids.length } };
          }
          if (command === "add_library_selection_to_playlist") {
            const playlistId = String(payload?.request?.playlistId || "");
            const wanted = Array.isArray(payload?.request?.trackIds) ? payload.request.trackIds.map(String) : [];
            const current = playlistTracks.get(playlistId) || [];
            let added = 0;
            let skipped = 0;
            for (const id of wanted) {
              if (current.includes(id)) { skipped += 1; continue; }
              current.push(id);
              added += 1;
            }
            playlistTracks.set(playlistId, current);
            return { ok: true, data: { playlistId, added, skipped } };
          }
          if (command === "add_track_candidates_to_playlist") {
            const playlistId = String(payload?.request?.playlistId || "");
            const candidates = Array.isArray(payload?.request?.tracks) ? payload.request.tracks : [];
            // Real serde rejects a non-numeric bpm on AddTrackCandidate with
            // "invalid type: string \"\", expected f64" -- mimic that here so a regression
            // (e.g. forwarding a UI display sentinel like bpm: "") fails this test too.
            for (const track of candidates) {
              const bpm = track?.bpm;
              if (bpm !== null && bpm !== undefined && typeof bpm !== "number") {
                return {
                  ok: false,
                  error: { code: "INVALID_ARGS", message: `invalid type: string "${bpm}", expected f64` }
                };
              }
              // AddTrackCandidate.track_id declares `id` as a serde alias of `trackId` --
              // sending both keys at once makes the real backend reject the request with
              // "duplicate field trackId". Mimic that here so a regression fails this test too.
              if (track && typeof track === "object" && "id" in track) {
                return {
                  ok: false,
                  error: { code: "INVALID_ARGS", message: "duplicate field trackId" }
                };
              }
            }
            const ids = candidates
              .map((track) => String(track?.localTrackId || track?.trackId || track?.id || "").trim())
              .filter(Boolean);
            const current = playlistTracks.get(playlistId) || [];
            let added = 0;
            for (const id of ids) {
              if (!current.includes(id)) {
                current.push(id);
                added += 1;
              }
            }
            playlistTracks.set(playlistId, current);
            return {
              ok: true,
              data: {
                playlistId,
                requested: candidates.length,
                resolved: ids.length,
                unresolved: candidates.length - ids.length,
                added,
                skipped: ids.length - added,
                resolutions: candidates.map((track, index) => ({
                  previousId: track?.trackId || track?.id || null,
                  trackId: ids[index] || null,
                  resolvedBy: ids[index] ? "self" : "none",
                  materialized: false
                }))
              }
            };
          }
          if (command === "get_playlist_tracks") {
            const playlistId = String(payload?.request?.playlistId || "");
            const ids = playlistTracks.get(playlistId) || [];
            const items = tracks.filter((t) => ids.includes(t.id));
            return { ok: true, data: { playlistId, items, total: items.length } };
          }
          if (command === "remove_tracks_from_playlist") {
            const playlistId = String(payload?.request?.playlistId || "");
            const gone = new Set((payload?.request?.trackIds || []).map(String));
            const current = playlistTracks.get(playlistId) || [];
            const kept = current.filter((id) => !gone.has(id));
            playlistTracks.set(playlistId, kept);
            return { ok: true, data: { playlistId, removed: current.length - kept.length } };
          }
          if (command === "analyze_new_tracks") {
            const ids = payload?.request?.trackIds || [];
            return { ok: true, data: { analyzed: ids.length, failed: 0, incomplete: 0, warnings: [], items: [] } };
          }
          // What the selected tracks share: genres differ, the release date is shared.
          if (command === "get_tracks_metadata_summary" || command === "get_usb_tracks_metadata_summary") {
            const count = (payload?.request?.trackIds || payload?.request?.usbMediaPathsRaw || []).length;
            return {
              ok: true,
              data: {
                count,
                genre: { mixed: true },
                rating: { value: 0, mixed: false },
                releaseDate: { value: "2001-04-02", mixed: false },
                genreOptions: ["House", "Techno"],
              },
            };
          }
          // The saves answer with the edited rows, whole, as the backend does.
          if (command === "save_track_metadata_edits") {
            const r = payload?.request || {};
            const year = Number(r.releaseDate?.slice(0, 4)) || null;
            const saved = tracks.filter((t) => (r.trackIds || []).includes(t.id));
            if (r.releaseDate !== undefined) for (const t of saved) t.year = year;
            return {
              ok: true,
              data: {
                items: saved.map((t) => ({ trackId: t.id, genre: r.genre, rating: r.rating ?? 0, year: t.year ?? undefined })),
                tracks: saved.map((t) => ({ ...t })),
              },
            };
          }
          if (command === "save_usb_track_metadata_edits") {
            const r = payload?.request || {};
            const year = Number(r.releaseDate?.slice(0, 4)) || null;
            const paths = (r.tracks || []).map((t) => t.usbMediaPathRaw);
            const saved = usbTracks.filter((t) => paths.includes(t.usbMediaPath));
            if (r.releaseDate !== undefined) for (const t of saved) t.year = year;
            return {
              ok: true,
              data: {
                items: (r.tracks || []).map((t) => ({ usbMediaPathRaw: t.usbMediaPathRaw, year: year ?? undefined })),
                tracks: saved.map((t) => ({ ...t })),
                localTracks: [],
              },
            };
          }
          if (opts.usb) {
            if (command === "pick_usb_folder") return "/Volumes/USB-TEST";
            if (command === "list_usb_devices") return { ok: true, data: { items: [] } };
            if (command === "validate_usb_root") {
              return {
                ok: true,
                data: {
                  valid: true, hasWriteAccess: true, normalizedRoot: "/Volumes/USB-TEST",
                  hasVendorRoot: true, hasContents: true, hasPdb: true, hasEdb: true, warnings: [],
                },
              };
            }
            if (command === "fetch_usb_playlists") {
              return {
                ok: true,
                data: {
                  items: [{ id: "usb-1", name: "Stick Set", source: "mock", trackCount: 2, tracks: [{}, {}] }],
                  stats: { indexedTracks: 2, playlistReferencedTracks: 2, playlistEntries: 2 },
                  warnings: [],
                },
              };
            }
            if (command === "fetch_usb_playlist_tracks") {
              return { ok: true, data: { items: usbTracks, total: usbTracks.length, totalDurationMs: 0, durationUnknownCount: usbTracks.length, warnings: [] } };
            }
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          // Three minutes per ticked track; "t3" has no length.
          if (command === "get_selection_duration") {
            const ids = payload?.request?.trackIds || [];
            const known = ids.filter((id) => id !== "t3").length;
            return {
              ok: true,
              data: { totalDurationMs: known * 180_000, durationUnknownCount: ids.length - known, trackCount: ids.length },
            };
          }
          if (command === "get_backend_log_buffer") return [];
          if (opts.usb) return { ok: true, data: {} };
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  }, mockOpts);
}

const libraryBar = '[data-selection-bar="library"]';

async function tickRows(page, bodyId, count = Infinity) {
  await page.evaluate(({ bodyId, count }) => {
    const boxes = [...document.querySelectorAll(`#${bodyId} .td-select input[type="checkbox"]`)].slice(0, count);
    for (const box of boxes) {
      box.checked = true;
      box.dispatchEvent(new Event("change", { bubbles: true }));
    }
  }, { bodyId, count });
}

// A new playlist opens, and becomes the add target; back to the library.
async function createPlaylist(page, name) {
  await page.locator("#addPlaylistBtn").click();
  await page.locator("#navPlaylistList .nav-new-input").fill(name);
  await page.locator("#navPlaylistList .nav-new-input").press("Enter");
  await expect(page.locator("#badgeLabel")).toHaveText(name);
  await page.locator('.nav-item[data-view="library"]').click();
}

test("Selection + bulk add to playlist", async ({ page }) => {
  await installSelectionMock(page);
  await page.goto("/");

  await page.locator("#addPlaylistBtn").click();
  await page.locator("#navPlaylistList .nav-new-input").fill("Bulk Add");
  await page.locator("#navPlaylistList .nav-new-input").press("Enter");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);
  await page.evaluate(() => {
    // Select every row, including the unanalyzed (no-bpm) "Sel Three" track -- regression
    // coverage for the bpm-crash fix (see the mock's add_track_candidates_to_playlist handler).
    const checkboxes = document.querySelectorAll('#libraryTableBody .track-grid-row input[type="checkbox"]');
    checkboxes.forEach((checkbox) => {
      checkbox.checked = true;
      checkbox.dispatchEvent(new Event("change", { bubbles: true }));
    });
  });
  await expect(page.locator(`${libraryBar} .selection-count`)).toContainText("3 selected");
  // The header keeps its fill-missing job; the selection has its own actions.
  await expect(page.locator("#scanLibraryBtn")).toHaveText("Scan Library");

  await page.evaluate(() => {
    document.querySelector('[data-selection-bar="library"] [data-selection-action="add"]')?.click();
  });
  await expect(page.locator("#statusText")).toContainText("Added 3");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
  await expect(page.locator("#playlistTracksBody")).toContainText("Sel One");
  await expect(page.locator("#playlistTracksBody")).toContainText("Sel Three");
});

test("Selection state stays correct when filtering visible rows", async ({ page }) => {
  await installSelectionMock(page);
  await page.goto("/");

  await page.locator("#addPlaylistBtn").click();
  await page.locator("#navPlaylistList .nav-new-input").fill("Filter Add");
  await page.locator("#navPlaylistList .nav-new-input").press("Enter");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);
  await page.evaluate(() => {
    const checkboxes = Array.from(document.querySelectorAll('#libraryTableBody input[type="checkbox"]'));
    for (const checkbox of checkboxes) {
      checkbox.checked = true;
      checkbox.dispatchEvent(new Event("change", { bubbles: true }));
    }
  });
  await expect(page.locator(`${libraryBar} .selection-count`)).toContainText("3 selected");

  await page.locator('.nav-item[data-view="library"]').click();
  await page.locator("#librarySearch").fill("Sel Two");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);
  await expect(page.locator(`${libraryBar} .selection-count`)).toContainText("1 selected");

  await page.evaluate(() => {
    document.querySelector('[data-selection-bar="library"] [data-selection-action="add"]')?.click();
  });
  await expect(page.locator("#statusText")).toContainText("Added 1");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(1);
  await expect(page.locator("#playlistTracksBody")).toContainText("Sel Two");
});

// The library is server-paginated (browse_source_files, one page of the list controller at a time).
// "select all" used to only select state.filteredTracks -- the tracks already loaded into the
// UI -- so on any library with more matches than one page, it silently missed the rest. This
// mock implements real offset pagination (unlike installSelectionMock's single-page stub) so
// that regression can't sneak back in unnoticed.
function installPaginatedLibraryMock(page, { trackCount }) {
  return page.addInitScript(({ trackCount }) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    // The library\'s source folders, as `get_library_sources` answers.
    window.__e2eSourceFolders = ["/music"];

    const allTracks = Array.from({ length: trackCount }, (_, i) => ({
      id: `t${i + 1}`,
      title: `Track ${String(i + 1).padStart(4, "0")}`,
      artist: "Artist",
      album: "Album",
      filePath: `/music/track-${i + 1}.mp3`
    }));
    const playlists = [];
    const playlistTracks = new Map();

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "get_library_sources") {
            const folders = (window.__e2eSourceFolders || [])
              .map((path) => ({ path, label: path, parent: null, enabled: true, missing: false, fullyAnalyzed: false }));
            return { ok: true, data: { folders, rekordboxDbEnabled: false, mixxxDbEnabled: false, everConfigured: folders.length > 0 } };
          }
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_rekordbox_db") return { ok: true, data: { found: false, path: null } };
          if (command === "list_playlists") return { ok: true, data: { items: playlists } };
          if (command === "browse_source_files") {
            const limit = Number(payload?.request?.limit) || allTracks.length;
            const offset = Number(payload?.request?.offset) || 0;
            const page = allTracks.slice(offset, offset + limit);
            const nextOffset = offset + page.length;
            const hasMore = nextOffset < allTracks.length;
            return {
              ok: true,
              data: { total: allTracks.length, items: page, nextCursor: hasMore ? String(nextOffset) : null, hasMore }
            };
          }
          if (command === "create_playlist") {
            const name = String(payload?.request?.name || "Untitled");
            const id = `pl-${Date.now()}`;
            const row = {
              id, name, source: "local",
              lastExportedAt: null, lastExportedUsbRoot: null, lastExportedTrackCount: null,
              tracks: []
            };
            playlists.push(row);
            playlistTracks.set(id, []);
            return { ok: true, data: { playlistId: id, name } };
          }
          if (command === "list_matching_track_ids") {
            const ids = allTracks.map((t) => t.id);
            return { ok: true, data: { trackIds: ids, total: ids.length } };
          }
          if (command === "add_library_selection_to_playlist") {
            const playlistId = String(payload?.request?.playlistId || "");
            const wanted = payload?.request?.allMatching
              ? allTracks.map((t) => t.id)
              : (Array.isArray(payload?.request?.trackIds) ? payload.request.trackIds.map(String) : []);
            const current = playlistTracks.get(playlistId) || [];
            let added = 0;
            let skipped = 0;
            for (const id of wanted) {
              if (current.includes(id)) { skipped += 1; continue; }
              current.push(id);
              added += 1;
            }
            playlistTracks.set(playlistId, current);
            return { ok: true, data: { playlistId, added, skipped } };
          }
          if (command === "add_track_candidates_to_playlist") {
            const playlistId = String(payload?.request?.playlistId || "");
            const candidates = Array.isArray(payload?.request?.tracks) ? payload.request.tracks : [];
            const ids = candidates
              .map((track) => String(track?.localTrackId || track?.trackId || track?.id || "").trim())
              .filter(Boolean);
            const current = playlistTracks.get(playlistId) || [];
            let added = 0;
            for (const id of ids) {
              if (!current.includes(id)) {
                current.push(id);
                added += 1;
              }
            }
            playlistTracks.set(playlistId, current);
            return {
              ok: true,
              data: {
                playlistId,
                requested: candidates.length,
                resolved: ids.length,
                unresolved: candidates.length - ids.length,
                added,
                skipped: ids.length - added,
                resolutions: candidates.map((track, index) => ({
                  previousId: track?.trackId || track?.id || null,
                  trackId: ids[index] || null,
                  resolvedBy: ids[index] ? "self" : "none",
                  materialized: false
                }))
              }
            };
          }
          if (command === "get_playlist_tracks") {
            const playlistId = String(payload?.request?.playlistId || "");
            const ids = playlistTracks.get(playlistId) || [];
            return { ok: true, data: { playlistId, items: allTracks.filter((t) => ids.includes(t.id)) } };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          if (command === "get_backend_log_buffer") return [];
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        }
      }
    };
  }, { trackCount });
}

test("select all loads and adds every matching track across multiple pages, not just the first page", async ({ page }) => {
  const trackCount = 220; // over one page (150) -- forces a second page
  await installPaginatedLibraryMock(page, { trackCount });
  await page.goto("/");

  await page.locator("#addPlaylistBtn").click();
  await page.locator("#navPlaylistList .nav-new-input").fill("All Tracks");
  await page.locator("#navPlaylistList .nav-new-input").press("Enter");

  await expect(page.locator("#libraryTableBody .track-grid-row").first()).toHaveCount(1);
  const initialCount = await page.locator("#libraryTableBody .track-grid-row").count();
  expect(initialCount).toBeLessThan(trackCount);

  await page.evaluate(() => {
    const checkbox = document.getElementById("selectAllTracks");
    checkbox.checked = true;
    checkbox.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await expect(page.locator(`${libraryBar} .selection-count`)).toContainText(`${trackCount} selected`, { timeout: 15000 });

  await page.evaluate(() => {
    document.querySelector('[data-selection-bar="library"] [data-selection-action="add"]')?.click();
  });
  await expect(page.locator("#statusText")).toContainText(`Added ${trackCount}`, { timeout: 15000 });

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  // The table only renders the rows around the viewport; the title carries
  // the whole playlist's count.
  await expect(page.locator("#playlistPanelTitle")).toContainText(`(${trackCount} tracks)`, { timeout: 15000 });
});

test("the selection bar is always there; its actions wait for a selection, and Reanalyze sends the picked tracks", async ({ page }) => {
  await installSelectionMock(page);
  await page.goto("/");
  await createPlaylist(page, "Bar");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);

  const bar = page.locator(libraryBar);
  await expect(bar).toBeVisible();
  await expect(bar).toHaveClass(/is-empty/);
  await expect(bar.locator(".selection-count")).toHaveText("0 selected");
  await expect(bar.locator('[data-selection-action="reanalyze"]')).toBeDisabled();
  await expect(bar.locator('[data-selection-action="clear"]')).toBeDisabled();

  await expect(bar.locator(".selection-time")).toBeHidden();
  await tickRows(page, "libraryTableBody", 2);
  await expect(bar.locator(".selection-count")).toHaveText("2 selected");
  // The backend sums the ticked tracks; the bar shows what it says.
  await expect(bar.locator(".selection-time")).toHaveText("· 6:00");
  await expect.poll(async () => (await page.evaluate(() => window.__calls))
    .filter((c) => c.command === "get_selection_duration").pop()?.request)
    .toMatchObject({ view: "library", trackIds: ["t1", "t2"] });
  await expect(page.locator("#selectAllTracks")).toHaveJSProperty("indeterminate", true);
  await bar.locator('[data-selection-action="reanalyze"]').click();
  await expect.poll(async () => (await page.evaluate(() => window.__calls))
    .find((c) => c.command === "analyze_new_tracks")?.request)
    .toMatchObject({ trackIds: ["t1", "t2"], reanalyze: true });
  // The header button never turned into a selection action.
  await expect(page.locator("#scanLibraryBtn")).toHaveText("Scan Library");

  await bar.locator('[data-selection-action="clear"]').click();
  await expect(bar.locator(".selection-count")).toHaveText("0 selected");
  await expect(page.locator("#libraryTableBody .td-select input:checked")).toHaveCount(0);
});

test("the playlist view selects tracks and removes them from the playlist only", async ({ page }) => {
  await installSelectionMock(page);
  await page.goto("/");
  await createPlaylist(page, "Trim");
  await tickRows(page, "libraryTableBody");
  await page.locator(`${libraryBar} [data-selection-action="add"]`).click();
  await expect(page.locator("#statusText")).toContainText("Added 3");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
  const bar = page.locator('[data-selection-bar="playlist"]');
  await expect(bar.locator('[data-selection-action="add"]')).toHaveCount(0);

  // Select all takes the whole playlist (`limit: 0`), then one is unticked.
  await page.locator('[data-select-all="playlist"]').check();
  await expect(bar.locator(".selection-count")).toHaveText("3 selected");
  const selectAll = (await page.evaluate(() => window.__calls))
    .filter((c) => c.command === "get_playlist_tracks").pop();
  expect(selectAll.request.limit).toBe(0);
  // "t3" has no length: the time is a lower bound.
  await expect(bar.locator(".selection-time")).toHaveText("· 6:00+");
  const playlistTime = (await page.evaluate(() => window.__calls))
    .filter((c) => c.command === "get_selection_duration").pop();
  expect(playlistTime.request).toMatchObject({ view: "playlist", id: expect.any(String) });
  await page.locator("#playlistTracksBody .track-grid-row").nth(1).locator(".td-select input").uncheck();
  await expect(bar.locator(".selection-count")).toHaveText("2 selected");
  await expect(bar.locator(".selection-time")).toHaveText("· 3:00+");

  await bar.locator('[data-selection-action="remove"]').click();
  await expect(page.locator("#statusText")).toContainText("Removed 2 tracks from Trim");
  const removed = (await page.evaluate(() => window.__calls)).find((c) => c.command === "remove_tracks_from_playlist");
  expect(removed.request.trackIds).toEqual(["t1", "t3"]);
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(1);
  await expect(page.locator("#playlistTracksBody")).toContainText("Sel Two");
  await expect(bar.locator(".selection-count")).toHaveText("0 selected");
});

test("USB playlist rows are selected and added to the active playlist in list order", async ({ page }) => {
  await installSelectionMock(page, { usb: true });
  await page.goto("/");
  await createPlaylist(page, "From Stick");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await page.locator("#refreshUsbBtn").click();
  await page.locator('[data-usb-playlist-index="0"]').click();
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(2);

  const bar = page.locator('[data-selection-bar="usbPlaylist"]');
  await expect(bar.locator('[data-selection-action="reanalyze"]')).toHaveCount(0);
  // Ticked bottom row first: the add still goes in list order.
  await page.locator("#usbPlaylistTracks .track-grid-row").nth(1).locator(".td-select input").check();
  await page.locator("#usbPlaylistTracks .track-grid-row").nth(0).locator(".td-select input").check();
  await expect(bar.locator(".selection-count")).toHaveText("2 selected");
  await bar.locator('[data-selection-action="add"]').click();

  await expect.poll(async () => (await page.evaluate(() => window.__calls))
    .find((c) => c.command === "add_track_candidates_to_playlist")?.request?.tracks?.map((t) => t.title))
    .toEqual(["Stick A", "Stick B"]);
});

test("Edit… shows what the selection shares and saves only the changed fields for every selected track", async ({ page }) => {
  await installSelectionMock(page);
  await page.goto("/");
  await createPlaylist(page, "Edit");
  await tickRows(page, "libraryTableBody", 2);
  await page.locator(`${libraryBar} [data-selection-action="edit"]`).click();

  const dialog = page.locator("#groupEditOverlay");
  await expect(dialog).toBeVisible();
  await expect(page.locator("#groupEditTitle")).toHaveText("Edit 2 tracks");
  // Mixed genres: empty with a "Mixed" placeholder; the shared release date
  // is shown.
  await expect(page.locator("#groupEditGenre")).toHaveValue("");
  await expect(page.locator("#groupEditGenre")).toHaveAttribute("placeholder", "Mixed");
  await expect(page.locator("#groupEditReleaseDate")).toHaveValue("2001-04-02");
  await expect(page.locator("#groupEditGenreOptions option")).toHaveCount(2);

  await page.locator("#groupEditGenre").fill("Techno");
  await page.locator("#groupEditRating [data-rating='4']").click();
  // A date that isn't YYYY, YYYY-MM or YYYY-MM-DD keeps the dialog open.
  await page.locator("#groupEditReleaseDate").fill("2001/04");
  await page.locator("#groupEditApplyBtn").click();
  await expect(page.locator("#groupEditReleaseDate")).toHaveAttribute("aria-invalid", "true");
  await page.locator("#groupEditReleaseDate").fill("2001-05");
  await page.locator("#groupEditApplyBtn").click();
  await expect(dialog).toBeHidden();

  const save = (await page.evaluate(() => window.__calls)).find((c) => c.command === "save_track_metadata_edits");
  expect(save.request).toEqual({ trackIds: ["t1", "t2"], genre: "Techno", rating: 4, releaseDate: "2001-05" });
  await expect(page.locator("#statusText")).toContainText("Saved track details for 2 tracks");
});

test("Edit… on USB playlist rows writes them to the stick and shows the new year in the rows", async ({ page }) => {
  await installSelectionMock(page, { usb: true });
  await page.goto("/");
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await page.locator("#refreshUsbBtn").click();
  await page.locator('[data-usb-playlist-index="0"]').click();
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(2);

  await page.locator('[data-select-all="usbPlaylist"]').check();
  const bar = page.locator('[data-selection-bar="usbPlaylist"]');
  await expect(bar.locator(".selection-count")).toHaveText("2 selected");
  await bar.locator('[data-selection-action="edit"]').click();
  await expect(page.locator("#groupEditTitle")).toHaveText("Edit 2 tracks");
  await page.locator("#groupEditReleaseDate").fill("2010-09-14");
  await page.locator("#groupEditReleaseDate").press("Enter");
  await expect(page.locator("#groupEditOverlay")).toBeHidden();

  const calls = await page.evaluate(() => window.__calls);
  const summary = calls.find((c) => c.command === "get_usb_tracks_metadata_summary");
  expect(summary.request.usbMediaPathsRaw).toEqual(["/Contents/stick-0.mp3", "/Contents/stick-1.mp3"]);
  const save = calls.find((c) => c.command === "save_usb_track_metadata_edits");
  expect(save.request.releaseDate).toBe("2010-09-14");
  expect(save.request.genre).toBeUndefined();
  expect(save.request.tracks.map((t) => t.usbMediaPathRaw)).toEqual(["/Contents/stick-0.mp3", "/Contents/stick-1.mp3"]);
  await expect(page.locator("#usbPlaylistTracks .td-year")).toHaveText(["2010", "2010"]);
});


const lastCall = async (page, command) => (await page.evaluate(() => window.__calls))
  .filter((c) => c.command === command).pop()?.request;

test("+ Add to names the active playlist; Add to… shows what each playlist holds and adds to several, new ones too", async ({ page }) => {
  await installSelectionMock(page);
  await page.goto("/");
  const bar = page.locator(libraryBar);
  const add = bar.locator('[data-selection-action="add"]');
  const addTo = bar.locator('[data-selection-action="add-to"]');

  // No active playlist: only Add to… is offered.
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);
  await tickRows(page, "libraryTableBody", 1);
  await expect(add).toBeHidden();
  await expect(addTo).toBeEnabled();
  await bar.locator('[data-selection-action="clear"]').click();

  await createPlaylist(page, "Warmup");
  await tickRows(page, "libraryTableBody", 1);
  await expect(add).toBeVisible();
  await expect(add).toHaveText("+ Add to Warmup");
  await expect(add).toHaveAttribute("data-tooltip", "Add the selected tracks to Warmup");
  await add.click();
  await expect(page.locator("#statusText")).toContainText("Added 1 track (skipped 0) to Warmup");
  await bar.locator('[data-selection-action="clear"]').click();

  await createPlaylist(page, "Peak");
  await expect(add).toHaveText("+ Add to Peak");
  await tickRows(page, "libraryTableBody", 2);
  await addTo.click();

  const dialog = page.locator("#addToOverlay");
  await expect(dialog).toBeVisible();
  await expect(page.locator("#addToTitle")).toHaveText("Add 2 tracks to playlists");
  expect((await lastCall(page, "get_playlist_membership")).trackIds).toEqual(["t1", "t2"]);
  const row = (name) => page.locator("#addToList li", { hasText: name });
  // Warmup holds one of the two: partly ticked until ticked.
  await expect(row("Warmup").locator(".add-to-note")).toHaveText("1 of 2 in");
  await expect(row("Warmup").locator("input")).toHaveJSProperty("indeterminate", true);
  await expect(row("Peak").locator(".add-to-note")).toHaveText("");
  await expect(page.locator("#addToApplyBtn")).toBeDisabled();

  // A playlist made here is ticked; it isn't made active.
  await page.locator("#addToNewName").fill("Late");
  await page.locator("#addToNewName").press("Enter");
  await expect(row("Late").locator("input")).toBeChecked();
  await expect(page.locator("#badgeLabel")).toHaveText("Peak");
  await row("Warmup").locator("input").check();
  await page.locator("#addToApplyBtn").click();
  await expect(dialog).toBeHidden();

  await expect(page.locator("#statusText")).toContainText("Added 3 tracks (skipped 1) to Warmup, Late");
  const request = await lastCall(page, "add_library_selection_to_playlists");
  expect(request.playlistIds).toEqual(["pl-1", "pl-3"]);
  expect(request.trackIds).toEqual(["t1", "t2"]);

  // Both now hold the whole selection: shown ticked, and can't be ticked off.
  await addTo.click();
  await expect(row("Warmup").locator(".add-to-note")).toHaveText("Already in");
  await expect(row("Warmup").locator("input")).toBeDisabled();
  await expect(row("Late").locator("input")).toBeChecked();
  await expect(page.locator("#addToApplyBtn")).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
});

test("Add to… from a USB playlist sends the rows' library matches and the rows themselves", async ({ page }) => {
  await installSelectionMock(page, { usb: true });
  await page.goto("/");
  await createPlaylist(page, "From Stick");
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await page.locator("#refreshUsbBtn").click();
  await page.locator('[data-usb-playlist-index="0"]').click();
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(2);

  await page.locator('[data-select-all="usbPlaylist"]').check();
  await page.locator('[data-selection-bar="usbPlaylist"] [data-selection-action="add-to"]').click();
  // One entry per row: the matched library track, or none.
  expect((await lastCall(page, "get_playlist_membership")).trackIds).toEqual(["t1", null]);
  await page.locator("#addToList li", { hasText: "From Stick" }).locator("input").check();
  await page.locator("#addToApplyBtn").click();

  await expect(page.locator("#statusText")).toContainText("to From Stick");
  const request = await lastCall(page, "add_track_candidates_to_playlists");
  expect(request.playlistIds).toEqual(["pl-1"]);
  expect(request.usbRoot).toBe("/Volumes/USB-TEST");
  expect(request.tracks.map((t) => t.title)).toEqual(["Stick A", "Stick B"]);
});

test("Add to… from an app playlist copies its tracks to another playlist by id", async ({ page }) => {
  await installSelectionMock(page);
  await page.goto("/");
  await createPlaylist(page, "Other");
  await createPlaylist(page, "Trim");
  await tickRows(page, "libraryTableBody");
  await page.locator(`${libraryBar} [data-selection-action="add"]`).click();
  await expect(page.locator("#statusText")).toContainText("Added 3");

  await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Trim" }).click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
  await page.locator('[data-select-all="playlist"]').check();
  await page.locator('[data-selection-bar="playlist"] [data-selection-action="add-to"]').click();
  await expect(page.locator("#addToList li", { hasText: "Trim" }).locator(".add-to-note")).toHaveText("Already in");
  await page.locator("#addToList li", { hasText: "Other" }).locator("input").check();
  await page.locator("#addToApplyBtn").click();

  await expect(page.locator("#statusText")).toContainText("Added 3 tracks (skipped 0) to Other");
  const request = await lastCall(page, "add_track_candidates_to_playlists");
  expect(request.playlistIds).toEqual(["pl-1"]);
  expect(request.tracks).toEqual([{ trackId: "t1" }, { trackId: "t2" }, { trackId: "t3" }]);
});
