import { test, expect, openUsb, uiTextCheck, useTestLanguage } from "./coverage-fixture.mjs";

// `usb`: a connected stick with one playlist of two tracks.
function installSelectionMock(page, mockOpts = {}) {
  return page.addInitScript((opts) => {
    const backend = window.__e2eBackend;
    window.__e2eSourceFolders = ["/music"];
    if (opts.usb) window.localStorage.setItem("djusbtkit.usbRoot", "/Volumes/USB-TEST");
    const usbTracks = ["Stick A", "Stick B"].map((title, i) => ({
      id: String(101 + i),
      title,
      artist: "Stick Artist",
      album: "Stick Album",
      filePath: `/Volumes/USB-TEST/Contents/stick-${i}.mp3`,
      usbMediaPath: `/Contents/stick-${i}.mp3`,
      // The first is matched to a library track and analyzed; the second neither.
      localTrackId: i === 0 ? "t1" : null,
      bpm: i === 0 ? 128 : null,
    }));

    const tracks = [
      { id: "t1", title: "Sel One", artist: "Artist", album: "Album", filePath: "/music/one.mp3" },
      { id: "t2", title: "Sel Two", artist: "Artist", album: "Album", filePath: "/music/two.mp3" },
      // No bpm field: a not-yet-analyzed library track.
      { id: "t3", title: "Sel Three", artist: "Artist", album: "Album", filePath: "/music/three.mp3" }
    ];
    backend.seed({ tracks });

    backend.install(async (command, request, payload) => {
      if (command === "add_track_candidates_to_playlist") {
        for (const track of request.tracks || []) {
          // Real serde rejects a non-numeric bpm on AddTrackCandidate -- a UI display
          // sentinel like bpm: "" forwarded by mistake fails this test too.
          const bpm = track?.bpm;
          if (bpm !== null && bpm !== undefined && typeof bpm !== "number") {
            return backend.fail(`invalid type: string "${bpm}", expected f64`, "INVALID_ARGS");
          }
          // `id` is a serde alias of `trackId`: sending both is "duplicate field trackId".
          if (track && typeof track === "object" && "id" in track) {
            return backend.fail("duplicate field trackId", "INVALID_ARGS");
          }
        }
        // `unresolvedAdd`: the backend matched none of them to a library track.
        if (opts.unresolvedAdd) {
          const unresolved = request.tracks.map((t) => ({ ...t, localTrackId: null, trackId: null }));
          return backend.answer(command, { request: { ...request, tracks: unresolved } });
        }
      }
      // Convert…: the first selected track gets a transcode, the rest are
      // skipped (the backend decides which need one).
      if (command === "get_transcode_formats") {
        return backend.ok({
          formats: [
            { format: "mp3", label: "MP3 320 kbps", isDefault: true },
            { format: "aiff", label: "AIFF (lossless)", isDefault: false },
            { format: "wav", label: "WAV (lossless)", isDefault: false },
          ],
        });
      }
      if (command === "get_transcode_folder") {
        return { ok: true, data: window.__transcodeFolder || { path: "/data/transcodes", isDefault: true } };
      }
      if (command === "pick_transcode_folder") return "/converted";
      if (command === "set_transcode_folder") {
        const path = payload?.request?.path;
        window.__transcodeFolder = path ? { path, isDefault: false } : null;
        return { ok: true, data: window.__transcodeFolder || { path: "/data/transcodes", isDefault: true } };
      }
      if (command === "transcode_tracks") {
        const { trackIds = [], format } = payload?.request || {};
        const converted = { ...tracks.find((t) => t.id === trackIds[0]) };
        converted.transcodes = [{
          format, label: format.toUpperCase(), filePath: `/data/transcodes/x.${format}`,
          sizeBytes: 30 * 1024 * 1024, outdated: false,
          badge: format.toUpperCase(), tooltip: `${format.toUpperCase()} transcode · 30 MB`,
        }];
        Object.assign(tracks.find((t) => t.id === trackIds[0]), converted);
        return backend.ok({
          jobId: "j1", transcoded: 1, skipped: trackIds.length - 1, failed: 0, cancelled: false,
          messages: trackIds.slice(1).map(() => "two.mp3: not a lossless file"),
          tracks: [converted],
          summary: `Converting done: 1 track converted, ${trackIds.length - 1} skipped`,
        });
      }
      if (command === "remove_track_transcodes") {
        const ids = payload?.request?.trackIds || [];
        const cleared = tracks.filter((t) => ids.includes(t.id)).map((t) => Object.assign(t, { transcodes: [] }));
        return { ok: true, data: { removed: 1, tracks: cleared, summary: "Removed 1 transcode" } };
      }
      if (command === "analyze_new_tracks") {
        const ids = payload?.request?.trackIds || [];
        return { ok: true, data: { jobId: "job-mock", queued: ids.length, skipped: 0, message: `Analyzing ${ids.length} tracks` } };
      }
      // What the selected tracks share: genres differ, the release date is shared.
      if (command === "get_tracks_metadata_summary" || command === "get_usb_tracks_metadata_summary") {
        const count = (payload?.request?.trackIds || payload?.request?.usbMediaPathsRaw || []).length;
        return backend.ok({
          count,
          genre: { mixed: true },
          rating: { value: 0, mixed: false },
          releaseDate: { value: "2001-04-02", mixed: false },
          genreOptions: ["House", "Techno"],
        });
      }
      // The saves answer with the edited rows, whole, as the backend does.
      if (command === "save_track_metadata_edits") {
        const r = payload?.request || {};
        const year = Number(r.releaseDate?.slice(0, 4)) || null;
        const saved = tracks.filter((t) => (r.trackIds || []).includes(t.id));
        if (r.releaseDate !== undefined) for (const t of saved) t.year = year;
        return backend.ok({
          items: saved.map((t) => ({ trackId: t.id, genre: r.genre, rating: r.rating ?? 0, year: t.year ?? undefined })),
          tracks: saved.map((t) => ({ ...t })),
          statusText: `Saved track details for ${saved.length} ${saved.length === 1 ? "track" : "tracks"}`,
        });
      }
      if (command === "save_usb_track_metadata_edits") {
        const r = payload?.request || {};
        const year = Number(r.releaseDate?.slice(0, 4)) || null;
        const paths = (r.tracks || []).map((t) => t.usbMediaPathRaw);
        const saved = usbTracks.filter((t) => paths.includes(t.usbMediaPath));
        if (r.releaseDate !== undefined) for (const t of saved) t.year = year;
        return backend.ok({
          items: (r.tracks || []).map((t) => ({ usbMediaPathRaw: t.usbMediaPathRaw, year: year ?? undefined })),
          tracks: saved.map((t) => ({ ...t })),
          localTracks: [],
          statusText: `Saved track details for ${r.tracks.length} ${r.tracks.length === 1 ? "track" : "tracks"} to USB`,
        });
      }
      if (opts.usb) {
        if (command === "fetch_usb_playlists") {
          return backend.ok({
            items: [{ id: "usb-1", name: "Stick Set", source: "mock", trackCount: 2, tracks: [{}, {}] }],
            stats: { indexedTracks: 2, playlistReferencedTracks: 2, playlistEntries: 2 },
            playlistUsbExportStatus: [],
            warnings: [],
          });
        }
        if (command === "fetch_usb_playlist_tracks") {
          return { ok: true, data: { items: usbTracks, total: usbTracks.length, totalDurationMs: 0, durationUnknownCount: usbTracks.length, warnings: [] } };
        }
      }
      // Three minutes per ticked track; "t3" has no length.
      if (command === "get_selection_duration") {
        const ids = payload?.request?.trackIds || [];
        const known = ids.filter((id) => id !== "t3").length;
        return backend.ok({
          totalDurationMs: known * 180_000, durationUnknownCount: ids.length - known, trackCount: ids.length,
          durationUnknownText: ids.length - known
            ? `${ids.length - known} of the selected tracks ${ids.length - known === 1 ? "has" : "have"} no length yet`
            : "",
          durationText: known ? `· ${known * 3}:00${ids.length - known ? "+" : ""}` : "",
        });
      }
    });
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

// Selection + bulk add to playlist; Selection state stays correct when filtering visible rows; the
// selection bar is always there; its actions wait for a selection, and Reanalyze sends the picked
// tracks; Convert… makes transcodes of the selection, shown under the format; Remove deletes them;
// Settings shows the transcode folder; Change picks one and Use default goes back; the playlist
// view selects tracks and removes them from the playlist only; Edit… shows what the selection
// shares and saves only the changed fields for every selected track; + Add to names the active
// playlist; Add to… shows what each playlist holds and adds to several, new ones too; Add to… from
// an app playlist copies its tracks to another playlist by id.
test("library selection and bulk add", async ({ page }) => {
  await installSelectionMock(page);
  await page.goto("/");
  {
    await page.locator("#addPlaylistBtn").click();
    await page.locator("#navPlaylistList .nav-new-input").fill("Bulk Add");
    await page.locator("#navPlaylistList .nav-new-input").press("Enter");

    await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);
    await page.evaluate(() => {
      // Select every row, including the unanalyzed (no-bpm) "Sel Three" track.
      const checkboxes = document.querySelectorAll('#libraryTableBody .track-grid-row input[type="checkbox"]');
      checkboxes.forEach((checkbox) => {
        checkbox.checked = true;
        checkbox.dispatchEvent(new Event("change", { bubbles: true }));
      });
    });
    await expect(page.locator(`${libraryBar} .selection-count`)).toContainText("3 selected");
    // The header keeps its fill-missing job; the selection has its own actions.
    await expect(page.locator("#analyzeLibraryBtn")).toHaveText("Analyze Library");

    await page.evaluate(() => {
      document.querySelector('[data-selection-bar="library"] [data-selection-action="add"]')?.click();
    });
    await expect(page.locator("#statusText")).toContainText("Added 3");

    await page.locator("#navPlaylistList .nav-playlist-item").first().click();
    await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
    await expect(page.locator("#playlistTracksBody")).toContainText("Sel One");
    await expect(page.locator("#playlistTracksBody")).toContainText("Sel Three");
  }

  await page.reload();
  {
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
  }

  await page.reload();
  {
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
    await expect(page.locator("#analyzeLibraryBtn")).toHaveText("Analyze Library");

    await bar.locator('[data-selection-action="clear"]').click();
    await expect(bar.locator(".selection-count")).toHaveText("0 selected");
    await expect(page.locator("#libraryTableBody .td-select input:checked")).toHaveCount(0);
  }

  await page.reload();
  {
    await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);
    const bar = page.locator(libraryBar);
    await expect(bar.locator('[data-selection-action="convert"]')).toBeDisabled();

    await tickRows(page, "libraryTableBody", 2);
    const convert = bar.locator('[data-selection-action="convert"]');
    for (const dismissal of ["Escape", "backdrop", "Cancel"]) {
      await convert.click();
      await expect(page.locator("#convertApplyBtn")).toBeFocused();
      if (dismissal === "Escape") await page.keyboard.press("Escape");
      else if (dismissal === "backdrop") await page.locator("#convertOverlay").click({ position: { x: 5, y: 5 } });
      else await page.locator("#convertCancelBtn").click();
      await expect(page.locator("#convertOverlay")).toBeHidden();
      await expect(convert).toBeFocused();
    }
    expect((await page.evaluate(() => window.__calls)).filter((c) => c.command === "transcode_tracks")).toHaveLength(0);
    await bar.locator('[data-selection-action="convert"]').click();
    await expect(page.locator("#convertOverlay")).toBeVisible();
    await expect(page.locator("#convertTitle")).toHaveText("Convert 2 tracks");
    await expect(page.locator("#convertFolderHint")).toHaveText("Saved in /data/transcodes (change it in Settings).");
    // The backend's choices, its default picked.
    await expect(page.locator("#convertFormatGroup label")).toHaveText(["MP3 320 kbps", "AIFF (lossless)", "WAV (lossless)"]);
    await expect(page.locator("#convertFormatGroup input[value='mp3']")).toBeChecked();
    await page.locator("#convertFormatGroup input[value='aiff']").check();
    await page.locator("#convertApplyBtn").click();
    await expect(page.locator("#convertOverlay")).toBeHidden();
    await expect.poll(async () => (await page.evaluate(() => window.__calls))
      .find((c) => c.command === "transcode_tracks")?.request)
      .toEqual({ trackIds: ["t1", "t2"], format: "aiff" });
    await expect(page.locator("#statusText")).toContainText("Converting done: 1 track converted, 1 skipped");

    // The transcode is a smaller line under the format badge, not a new row.
    const first = page.locator("#libraryTableBody .track-grid-row").first();
    await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);
    const line = first.locator(".td-format .format-transcode");
    await expect(line).toHaveText("AIFF");
    await expect(line).toHaveAttribute("data-tooltip", "AIFF transcode · 30 MB");
    await expect(line).not.toHaveClass(/outdated/);

    // The selection stays; Cancel changes nothing.
    await expect(bar.locator(".selection-count")).toHaveText("2 selected");
    await bar.locator('[data-selection-action="convert"]').click();
    await page.locator("#convertCancelBtn").click();
    await expect(page.locator("#convertOverlay")).toBeHidden();
    expect((await page.evaluate(() => window.__calls)).filter((c) => c.command === "transcode_tracks")).toHaveLength(1);

    await bar.locator('[data-selection-action="convert"]').click();
    await page.locator("#convertRemoveBtn").click();
    await expect.poll(async () => (await page.evaluate(() => window.__calls))
      .find((c) => c.command === "remove_track_transcodes")?.request)
      .toEqual({ trackIds: ["t1", "t2"] });
    await expect(first.locator(".format-transcode")).toHaveCount(0);
    await expect(page.locator("#statusText")).toContainText("Removed 1 transcode");
  }

  await page.reload();
  {
    await page.locator("#settingsBtn").click();
    await expect(page.locator("#transcodeFolderText")).toHaveText("/data/transcodes");
    await expect(page.locator("#transcodeFolderDefaultTag")).toBeVisible();
    await expect(page.locator("#transcodeFolderResetBtn")).toBeHidden();

    await page.locator("#transcodeFolderChangeBtn").click();
    await expect(page.locator("#transcodeFolderText")).toHaveText("/converted");
    await expect(page.locator("#transcodeFolderDefaultTag")).toBeHidden();
    await expect.poll(async () => (await page.evaluate(() => window.__calls))
      .find((c) => c.command === "set_transcode_folder")?.request)
      .toEqual({ path: "/converted" });

    await page.locator("#transcodeFolderResetBtn").click();
    await expect(page.locator("#transcodeFolderText")).toHaveText("/data/transcodes");
    await expect(page.locator("#transcodeFolderDefaultTag")).toBeVisible();
    await expect(page.locator("#transcodeFolderResetBtn")).toBeHidden();
  }

  await page.reload();
  {
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
    await expect(bar.locator(".selection-time")).toHaveAttribute("data-tooltip", "1 of the selected tracks has no length yet");
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
  }

  await page.reload();
  {
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

    const edit = page.locator(`${libraryBar} [data-selection-action="edit"]`);
    for (const dismissal of ["Escape", "backdrop", "Cancel"]) {
      await page.locator("#groupEditGenre").fill("Discard this edit");
      if (dismissal === "Escape") await page.keyboard.press("Escape");
      else if (dismissal === "backdrop") await dialog.click({ position: { x: 5, y: 5 } });
      else await page.locator("#groupEditCancelBtn").click();
      await expect(dialog).toBeHidden();
      await expect(edit).toBeFocused();
      expect((await page.evaluate(() => window.__calls)).filter((c) => c.command === "save_track_metadata_edits")).toHaveLength(0);
      await edit.click();
      await expect(page.locator("#groupEditGenre")).toBeFocused();
      await expect(page.locator("#groupEditGenre")).toHaveValue("");
    }

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
  }

  await page.reload();
  {
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

    // In sidebar order, newest first.
    await expect(page.locator("#statusText")).toContainText("Added 3 tracks (skipped 1) to Late, Warmup");
    const request = await lastCall(page, "add_library_selection_to_playlists");
    expect(request.playlistIds).toEqual(["pl-3", "pl-1"]);
    expect(request.trackIds).toEqual(["t1", "t2"]);

    // Both now hold the whole selection: shown ticked, and can't be ticked off.
    await addTo.click();
    await expect(row("Warmup").locator(".add-to-note")).toHaveText("Already in");
    await expect(row("Warmup").locator("input")).toBeDisabled();
    await expect(row("Late").locator("input")).toBeChecked();
    await expect(page.locator("#addToApplyBtn")).toBeDisabled();
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(addTo).toBeFocused();
    for (const dismissal of ["backdrop", "Cancel"]) {
      await addTo.click();
      await expect(dialog).toBeVisible();
      if (dismissal === "backdrop") await dialog.click({ position: { x: 5, y: 5 } });
      else await page.locator("#addToCancelBtn").click();
      await expect(dialog).toBeHidden();
      await expect(addTo).toBeFocused();
    }
    expect((await page.evaluate(() => window.__calls)).filter((c) => c.command === "add_library_selection_to_playlists")).toHaveLength(1);
  }

  await page.reload();
  {
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
  }
});

// The library is server-paginated (browse_source_files, one page of the list controller at a time).
// "select all" used to only select state.filteredTracks -- the tracks already loaded into the
// UI -- so on any library with more matches than one page, it silently missed the rest. This
// mock implements real offset pagination (unlike installSelectionMock's single-page stub) so
// that regression can't sneak back in unnoticed.
function installPaginatedLibraryMock(page, { trackCount }) {
  return page.addInitScript(({ trackCount }) => {
    window.__e2eSourceFolders = ["/music"];
    window.__e2eBackend.seed({
      tracks: Array.from({ length: trackCount }, (_, i) => ({
        id: `t${i + 1}`,
        title: `Track ${String(i + 1).padStart(4, "0")}`,
        artist: "Artist",
        album: "Album",
        filePath: `/music/track-${i + 1}.mp3`
      }))
    });
    window.__e2eBackend.install();
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

test("USB playlist row selection", async ({ page }) => {
  await installSelectionMock(page, { usb: true });
  await page.goto("/");
  // USB playlist rows are selected and added to the active playlist in list order.
  {
    await createPlaylist(page, "From Stick");

    await openUsb(page, "usb-playlists");
    await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(2);

    const bar = page.locator('[data-selection-bar="usbPlaylist"]');
    await expect(bar.locator('[data-selection-action="reanalyze"]')).toHaveCount(0);
    // Ticked bottom row first: the add still goes in list order.
    await page.locator("#usbPlaylistTracks .track-grid-row").nth(1).locator(".td-select input").check();
    await page.locator("#usbPlaylistTracks .track-grid-row").nth(0).locator(".td-select input").check();
    await expect(bar.locator(".selection-count")).toHaveText("2 selected");
    await bar.locator('[data-selection-action="add"]').click();

    // The mock rejects what real serde would (both `id` and `trackId`, a non-numeric bpm),
    // so the backend's status line means the request went through.
    await expect(page.locator("#statusText")).toContainText("Added 2 tracks (skipped 0) to From Stick");
    const calls = await page.evaluate(() => window.__calls);
    const addAt = calls.findIndex((c) => c.command === "add_track_candidates_to_playlist");
    const request = calls[addAt].request;
    expect(request).toMatchObject({ playlistId: "pl-1", usbRoot: "/Volumes/USB-TEST", usbRootValid: true });
    expect(request.tracks.map((t) => t.title)).toEqual(["Stick A", "Stick B"]);
    expect(request.tracks.map((t) => t.trackId)).toEqual(["101", "102"]);
    expect(request.tracks.map((t) => t.localTrackId)).toEqual(["t1", null]);
    expect(request.tracks.map((t) => t.bpm)).toEqual([128, null]);
    expect(request.tracks.some((t) => "id" in t)).toBe(false);
    // A resolved add re-reads the open list (the playlists come with its answer).
    const after = calls.slice(addAt + 1).map((c) => c.command);
    expect(after).toContain("get_playlist_tracks");
  }

  await page.reload();
  // Edit… on USB playlist rows writes them to the stick and shows the new year in the rows.
  {
    await openUsb(page, "usb-playlists");
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
  }

  await page.reload();
  // Add to… from a USB playlist sends the rows' library matches and the rows themselves.
  {
    await createPlaylist(page, "From Stick");
    await openUsb(page, "usb-playlists");
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
  }
});

test("USB rows can't be added without an active playlist, and rows the backend can't resolve aren't added", async ({ page }) => {
  await installSelectionMock(page, { usb: true, unresolvedAdd: true });
  await page.goto("/");

  await openUsb(page, "usb-playlists");
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(2);
  const bar = page.locator('[data-selection-bar="usbPlaylist"]');
  const firstRow = page.locator("#usbPlaylistTracks .track-grid-row").nth(0);

  // No playlist yet: the bar's "+ Add to" is gone (Add to… picks one) and the row's + waits.
  await firstRow.locator(".td-select input").check();
  await expect(bar.locator('[data-selection-action="add"]')).toBeHidden();
  await expect(firstRow.locator('[data-action="add-usb"]')).toBeDisabled();

  await createPlaylist(page, "Nowhere");
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(2);
  await firstRow.locator('[data-action="add-usb"]').click();

  await expect(page.locator("#statusText")).toContainText("No imported track IDs found to add");
  const calls = await page.evaluate(() => window.__calls);
  const addAt = calls.findIndex((c) => c.command === "add_track_candidates_to_playlist");
  expect(addAt).toBeGreaterThanOrEqual(0);
  // Nothing was added, so the open list isn't re-read.
  const after = calls.slice(addAt + 1).map((c) => c.command);
  expect(after).not.toContain("get_playlist_tracks");
});

const lastCall = async (page, command) => (await page.evaluate(() => window.__calls))
  .filter((c) => c.command === command).pop()?.request;

test("in the test language, the Library and playlist pages and their dialogs show only catalog text and data", async ({ page }) => {
  await installSelectionMock(page);
  await useTestLanguage(page);
  await page.goto("/");
  const check = uiTextCheck(page);
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);
  await check.collect();

  await createPlaylist(page, "Warmup");
  // The new playlist's page, still empty.
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#badgeLabel")).toHaveText("Warmup");
  await check.collect();
  await page.locator('.nav-item[data-view="library"]').click();
  await tickRows(page, "libraryTableBody", 2);
  await check.collect();

  await page.locator(`${libraryBar} [data-selection-action="convert"]`).click();
  await expect(page.locator("#convertOverlay")).toBeVisible();
  await check.collect();
  await page.keyboard.press("Escape");
  await expect(page.locator("#convertOverlay")).toBeHidden();

  await page.locator(`${libraryBar} [data-selection-action="add-to"]`).click();
  await expect(page.locator("#addToOverlay")).toBeVisible();
  await check.collect();
  await page.keyboard.press("Escape");
  await expect(page.locator("#addToOverlay")).toBeHidden();

  await page.locator(`${libraryBar} [data-selection-action="edit"]`).click();
  await expect(page.locator("#groupEditOverlay")).toBeVisible();
  await check.collect();
  await page.keyboard.press("Escape");
  await expect(page.locator("#groupEditOverlay")).toBeHidden();

  await page.locator(`${libraryBar} [data-selection-action="add"]`).click();
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);
  await check.collect();

  const playlistItem = page.locator("#navPlaylistList .nav-playlist-item").first();
  await playlistItem.locator("[data-delete-playlist]").click();
  await expect(page.locator("#confirmOverlay")).toBeVisible();
  await check.collect();
  await page.keyboard.press("Escape");
  await expect(page.locator("#confirmOverlay")).toBeHidden();

  await check.expectNoEnglish();
});
