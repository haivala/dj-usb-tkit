import { test, expect, openUsb } from "./coverage-fixture.mjs";

// The `reorder_playlist_tracks` requests the app sent.
const reorderRequests = (page) =>
  page.evaluate(() => window.__calls.filter((c) => c.command === "reorder_playlist_tracks").map((c) => c.request));

// A playlist's tracks: analyzed, with a length, at 120 BPM.
const songs = (...rows) => rows.map(([id, title, album = ""]) => ({
  id, title, artist: "Artist", album, filePath: `/music/${id}.mp3`, waveformPeaksPath: "", waveformPreview: [],
  durationMs: 180000, bpm: 120, key: "8A", analysisReady: true
}));

test("playlist analyze-missing skips already-analyzed tracks and targets the rest", async ({ page }) => {
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    window.__e2eSourceFolders = ["/music"];
    const missing = {
      title: "Local Missing", artist: "Artist A", album: "Album A", filePath: "/music/local-missing.mp3",
      waveformPeaksPath: "", waveformPreview: [], durationMs: null, bpm: null, key: null, analysisReady: false
    };
    const entry = { id: "playlist-entry-1", localTrackId: "local-missing-1", ...missing, fileSizeBytes: 1.5 * 1024 ** 3 };
    const libraryTrack = { id: "local-missing-1", ...missing };
    backend.seed({
      tracks: [libraryTrack],
      playlists: [{
        id: "pl-1",
        name: "Testi",
        tracks: [entry, {
          id: "usb-track-1",
          localTrackId: "usb-track-1",
          title: "USB Ready",
          artist: "Artist USB",
          album: "USB Album",
          filePath: "/USB/Contents/Artist USB/USB Ready.mp3",
          usbAnalysisPath: "/USB/PIONEER/USBANLZ/P001/TEST/ANLZ0000.DAT",
          waveformPeaksPath: "/USB/PIONEER/USBANLZ/P001/TEST/ANLZ0000.DAT",
          waveformPreview: [10, 20, 30],
          fileSizeBytes: 2 * 1024 ** 3,
          durationMs: 180000,
          bpm: 128,
          key: null,
          analysisReady: true
        }]
      }]
    });

    const analyzedRequests = [];
    // The analysis holds until the test releases it, so its transient
    // "analyzing" row state is guaranteed observable.
    let releaseAnalysis = null;
    const analyzed = { durationMs: 180000, waveformPeaksPath: "/tmp/local-missing.DAT", waveformPreview: [5, 10, 20], bpm: 128, analysisReady: true };
    const job = { jobId: "job-analysis-mock", jobType: "analysis", stage: "analyze_new_tracks" };
    backend.install((command, request) => {
      if (command !== "analyze_new_tracks") return undefined;
      // "Analyze Missing" sends no ids + its playlist's id; the backend picks the
      // playlist's unanalyzed local tracks.
      let ids = (request.trackIds || []).map(String);
      if (!ids.length && request.playlistIds?.length) {
        ids = request.playlistIds.flatMap((id) => backend.store.playlistTracks.get(id))
          .filter((t) => !t.analysisReady).map((t) => String(t.localTrackId || t.id));
      }
      analyzedRequests.push({ trackIds: ids, playlistIds: request.playlistIds });
      const ours = ids.filter((id) => id === "local-missing-1");
      const total = Math.max(1, ids.length);
      (async () => {
        window.__emitJobEvent({ ...job, event: "job.started", current: 0, total, percent: 0, message: "Analyzing selected tracks" });
        for (const trackId of ours) {
          window.__emitJobEvent({
            ...job, event: "job.progress", trackId, current: 0, total: 1, percent: 0,
            message: "Analyzing 1/1: Local Missing", trackReady: false
          });
        }
        await new Promise((resolve) => { releaseAnalysis = resolve; });
        for (const trackId of ours) {
          Object.assign(entry, analyzed);
          Object.assign(libraryTrack, analyzed);
          window.__emitJobEvent({
            ...job, ...analyzed, event: "job.progress", trackId, trackTitle: "Local Missing", filePath: missing.filePath,
            current: 1, total: 1, percent: 100, message: "Analyzing 1/1: Local Missing",
            trackReady: true, failed: false, bpmAnalyzer: "mock-analyzer",
            // The track's row as saved, whole, as the backend sends it.
            track: { ...libraryTrack }
          });
        }
        window.__emitJobEvent({
          ...job, event: "job.completed", current: ids.length, total, percent: 100,
          message: `Analysis finished: analyzed ${ids.length}, failed 0`
        });
      })();
      return backend.ok({ jobId: job.jobId, queued: ids.length, skipped: 0, message: `Analyzing ${ids.length} tracks` });
    }, { tauriRuntime: true });

    window.__playlistAnalysisTest = { analyzedRequests, releaseAnalysis: () => releaseAnalysis?.() };
  });

  await page.goto("/");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistPanelTitle")).toContainText("Testi (2 tracks)");
  // Footer total comes straight from the backend (get_playlist_tracks), not a
  // client-side sum -- 1 of the 2 tracks has no known length.
  await expect(page.locator("#playlistTotalDuration")).toHaveText("Total time: 3:00 (1 without length)");
  // A search narrows the footer total, but the heading keeps the whole
  // playlist: that's what an export copies.
  await page.locator("#playlistSearchInput").fill("USB Ready");
  await expect(page.locator("#playlistTotalDuration")).toHaveText("Total time: 3:00");
  await expect(page.locator("#playlistPanelTitle")).toContainText("Testi (2 tracks)");
  await page.locator("#playlistSearchInput").fill("");
  await expect(page.locator("#playlistTotalDuration")).toHaveText("Total time: 3:00 (1 without length)");
  await expect(page.locator("#analyzePlaylistMissingBtn")).toHaveText("Analyze Missing Tracks (1)");
  await expect(page.locator("#analyzePlaylistMissingBtn")).toBeVisible();
  await expect(page.locator("#exportPlaylistBtn")).toBeHidden();

  await page.locator("#analyzePlaylistMissingBtn").click();
  // The row shows the analyzing state while the (held) analysis runs...
  await expect(page.locator('#playlistTracksBody .track-grid-row[data-track-id="local-missing-1"]')).toHaveClass(
    /is-analyzing/
  );
  await expect(page.locator('#playlistTracksBody .track-grid-row[data-track-id="playlist-entry-1"]')).toHaveCount(0);
  // Only that track's own button is disabled, here as in the library.
  const ownButton = page.locator('#playlistTracksBody .track-grid-row[data-track-id="local-missing-1"] [data-action="analyze-track"]');
  const otherButton = page.locator('#playlistTracksBody .track-grid-row:not([data-track-id="local-missing-1"]) [data-action="analyze-track"]').first();
  await expect(ownButton).toBeDisabled();
  await expect(otherButton).toBeEnabled();
  await expect(page.locator("#analyzePlaylistMissingBtn")).toBeEnabled();
  // ...and clears it once the analysis completes (checked below).
  await page.evaluate(() => window.__playlistAnalysisTest.releaseAnalysis());

  await page.waitForFunction(() => {
    const reqs = window.__playlistAnalysisTest?.analyzedRequests || [];
    return reqs.length === 1;
  });

  const analyzedRequests = await page.evaluate(() => window.__playlistAnalysisTest.analyzedRequests);
  // The frontend delegates scoping to the backend: it sends the playlist id, not
  // a client-collected id list. The mock resolves it to the unanalyzed track.
  expect(analyzedRequests.map((item) => item.playlistIds)).toEqual([["pl-1"]]);
  expect(analyzedRequests.map((item) => item.trackIds)).toEqual([["local-missing-1"]]);
  await expect(page.locator("#statusText")).toContainText("Analysis finished: analyzed 1, failed 0");
  await expect(page.locator('#playlistTracksBody .track-grid-row[data-track-id="local-missing-1"]')).not.toHaveClass(/is-analyzing/);
  await expect(ownButton).toBeEnabled();

  await page.locator('.nav-item[data-view="library"]').click();
  await expect(page.locator('#libraryTableBody .track-grid-row[data-track-id="local-missing-1"] [data-action="analyze-track"]')).toHaveText("Reanalyze");
  await expect(page.locator('#libraryTableBody .track-grid-row[data-track-id="local-missing-1"] .td-bpm')).toContainText("128");
  await expect(page.locator('#libraryTableBody .track-grid-row[data-track-id="local-missing-1"] .waveform.waveform-canvas')).toHaveCount(1);
});

test("playlist actions hide Analyze Missing when unnecessary and keep Export visible", async ({ page }) => {
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    backend.seed({
      playlists: [{
        id: "pl-1",
        name: "Ready Playlist",
        tracks: [{
          id: "local-ready-1", title: "Local Ready", artist: "Artist A", album: "Album A", filePath: "/music/local-ready.mp3",
          waveformPeaksPath: "/tmp/local-ready.dat", waveformPreview: [10, 20, 30], durationMs: 180000, bpm: 128, key: "8A",
          analysisReady: true
        }]
      }]
    });
    backend.install((command, request, payload) => {
      if (command === "list_playlists") {
        const answer = backend.answer(command, payload);
        answer.data.playlistUsbExportStatus = [
          { playlistId: "pl-1", usbSelected: false, exportButtonText: "Select USB first", exportButtonTitle: "Select a valid USB folder first" }
        ];
        return answer;
      }
    });
  });

  await page.goto("/");
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistPanelTitle")).toContainText("Ready Playlist (1 track)");
  await expect(page.locator("#analyzePlaylistMissingBtn")).toBeHidden();
  await expect(page.locator("#exportPlaylistBtn")).toBeVisible();
  await expect(page.locator("#exportPlaylistBtn")).toHaveText("Select USB first");
});

test("playlist analyze-missing offers unanalyzed tracks that live on a USB drive", async ({ page }) => {
  // Regression: a track imported into the media library from an MP3 folder on a
  // USB stick gets is_usb_path=true from the backend. It is still a real,
  // analyzable library track -- the playlist must offer "Analyze Missing Tracks"
  // for it, not hide the button and let the backend export gate reject it later.
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    window.__e2eSourceFolders = ["/music"];
    backend.seed({
      playlists: [{
        id: "pl-1",
        name: "USB Import Playlist",
        tracks: [
          {
            id: "local-ready-entry", localTrackId: "local-ready-1", title: "Local Ready", artist: "Artist A", album: "Album A",
            filePath: "/music/local-ready.mp3", waveformPeaksPath: "/tmp/local-ready.dat", waveformPreview: [10, 20, 30],
            durationMs: 180000, bpm: 128, key: "8A", analysisReady: true
          },
          {
            id: "usb-import-entry", localTrackId: "usb-import-1", title: "USB Import", artist: "Artist B", album: "Album B",
            filePath: "/USB/MP3/usb-import.mp3", isUsbPath: true, waveformPeaksPath: "", waveformPreview: [],
            durationMs: null, bpm: null, key: null, analysisReady: false
          }
        ]
      }]
    });
    const analyzedRequests = [];
    backend.install((command, request) => {
      if (command !== "analyze_new_tracks") return undefined;
      // "Analyze Missing" sends the playlist id, not an id list; the backend
      // resolves it to that playlist's unanalyzed local tracks.
      let ids = (request.trackIds || []).map(String);
      if (!ids.length && request.playlistIds?.length) {
        ids = request.playlistIds.flatMap((id) => backend.store.playlistTracks.get(id))
          .filter((t) => !t.analysisReady).map((t) => String(t.localTrackId || t.id));
      }
      analyzedRequests.push({ trackIds: ids, playlistIds: request.playlistIds });
      return backend.ok({ jobId: "job-mock", queued: ids.length, skipped: 0, message: `Analyzing ${ids.length} tracks` });
    }, { tauriRuntime: true });
    window.__playlistAnalysisTest = { analyzedRequests };
  });

  await page.goto("/");
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();

  await expect(page.locator("#analyzePlaylistMissingBtn")).toHaveText("Analyze Missing Tracks (1)");
  await expect(page.locator("#analyzePlaylistMissingBtn")).toBeVisible();
  await expect(page.locator("#exportPlaylistBtn")).toBeHidden();

  await page.locator("#analyzePlaylistMissingBtn").click();
  await page.waitForFunction(() => (window.__playlistAnalysisTest?.analyzedRequests || []).length === 1);
  const analyzedRequests = await page.evaluate(() => window.__playlistAnalysisTest.analyzedRequests);
  expect(analyzedRequests.map((item) => item.playlistIds)).toEqual([["pl-1"]]);
  expect(analyzedRequests.map((item) => item.trackIds)).toEqual([["usb-import-1"]]);
});

test("selecting a playlist with tracks right after an empty one still paints its waveforms", async ({ page }) => {
  // Regression: the empty-state chrome sets `#playlistTableWrap` to
  // display:none. renderTrackTable() paints each row's waveform canvas via
  // getBoundingClientRect(), which reads 0x0 while an ancestor is hidden, so
  // the canvas locks to 1x1 and is never repainted once the wrap is shown
  // again -- every waveform came up blank when the previously-selected
  // playlist was empty. The wrap must be revealed before the rows render.
  await page.addInitScript(() => {
    window.__e2eBackend.seed({
      playlists: [
        { id: "pl-empty", name: "Empty Playlist" },
        {
          id: "pl-full",
          name: "Full Playlist",
          tracks: [
            { id: "t1", title: "Song A", artist: "Artist", album: "Album", filePath: "/music/a.mp3", waveformPeaksPath: "/tmp/a.dat", waveformPreview: [8, 20, 42, 65, 30, 55], durationMs: 180000, bpm: 120, key: "8A", analysisReady: true },
            { id: "t2", title: "Song B", artist: "Artist", album: "Album", filePath: "/music/b.mp3", waveformPeaksPath: "/tmp/b.dat", waveformPreview: [7, 19, 43, 61, 34, 57], durationMs: 180000, bpm: 122, key: "9A", analysisReady: true }
          ]
        }
      ]
    });
    window.__e2eBackend.install();
  });

  await page.goto("/");

  // Select the empty playlist first -- this hides #playlistTableWrap.
  await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Empty Playlist" }).click();
  await expect(page.locator("#playlistEmptyState")).toBeVisible();
  await expect(page.locator("#playlistTableWrap")).toBeHidden();

  // Now select the non-empty one.
  await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Full Playlist" }).click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);

  // Every waveform canvas must be sized to its laid-out box, not the 1x1 it
  // gets when measured inside a display:none ancestor.
  await expect.poll(async () => page.evaluate(() => {
    const canvases = Array.from(document.querySelectorAll("#playlistTracksBody .waveform.waveform-canvas .waveform-canvas-el"));
    return canvases.length > 0 && canvases.every((c) => c.width > 1 && c.height > 1);
  })).toBe(true);
});

// The app's playlists (by default one, "Reorder Playlist") and, when
// `usbSameNamePlaylistName` is set, a USB holding a playlist of that name: in
// additive export mode (`exportPruneStale: false`), the same-name playlist's
// order is locked.
function installReorderTauriMock(page, {
  usbSameNamePlaylistName,
  exportPruneStale,
  playlists = [{
    id: "pl-1",
    name: "Reorder Playlist",
    tracks: [["t1", "Song A", "Nina"], ["t2", "Song B", "Alex"], ["t3", "Song C", "Max"]].map(([id, title, artist]) => ({
      ...songs([id, title])[0], artist, filePath: `/music/${title.at(-1).toLowerCase()}.mp3`
    }))
  }]
} = {}) {
  return page.addInitScript(({ usbSameNamePlaylistName, exportPruneStale, playlists }) => {
    const backend = window.__e2eBackend;
    // A test can name another USB playlist for the loads after a reload.
    usbSameNamePlaylistName = sessionStorage.getItem("usbPlaylistName") || usbSameNamePlaylistName;
    if (exportPruneStale !== undefined) {
      window.localStorage.setItem("djusbtkit.exportPruneStale", exportPruneStale ? "1" : "0");
    }
    backend.seed({ playlists });

    // Mirrors the backend's export_prune_stale setting: seeded from the param,
    // updated by set_export_sync_mode, read by list_playlists / fetch_usb_playlists.
    let currentPruneStale = exportPruneStale !== false;
    // The backend's selected USB: set by a valid select_usb.
    let usbSelected = false;
    const sameName = (name) => !!usbSameNamePlaylistName
      && String(name || "").trim().toLowerCase() === String(usbSameNamePlaylistName).trim().toLowerCase();
    const exportStatusFor = () => backend.store.playlists.map((playlist) => {
      const sameNameExistsOnUsb = sameName(playlist.name);
      const locksReorder = !currentPruneStale && sameNameExistsOnUsb;
      return {
        playlistId: playlist.id,
        playlistName: playlist.name,
        usbSelected: true, sameNameExistsOnUsb,
        locksReorder,
        markLabel: sameNameExistsOnUsb ? "On this USB" : null,
        // Backend-built label (service::export::compute_playlist_usb_export_status);
        // usb root here is "/USB" so the trailing dir is "USB".
        exportButtonText: locksReorder
          ? `Append to (${playlist.name}) on USB: (USB)`
          : "Export to USB: USB",
        exportButtonTitle: locksReorder
          ? `Append current playlist tracks to existing USB playlist "${playlist.name}"`
          : "Export current playlist to selected USB",
        reorderLockTooltip: locksReorder
          ? `Won't reorder on USB — "${playlist.name}" already exists there, and additive export keeps its existing track order unchanged. New tracks are still added in your chosen order.`
          : null,
        // The line under the title, as the backend words it.
        exportStatusText: [
          sameNameExistsOnUsb ? "On this USB." : "Not on this USB.",
          playlist.lastExportedAt
            ? `Last exported ${playlist.lastExportedAt} to ${playlist.lastExportedUsbRoot} (${playlist.lastExportedTrackCount} tracks).`
            : null
        ].filter(Boolean).join(" ")
      };
    });

    backend.install(async (command, request, payload) => {
      if (command === "get_playlist_tracks" && request.usbRoot) {
        // Read for a USB, each track carries what export puts on it
        // (`__exportFiles`, by track id), as the backend sends it.
        const answer = backend.answer(command, payload);
        answer.data.items = answer.data.items.map((track) => {
          const exportFile = window.__exportFiles?.[track.id];
          return exportFile ? { ...track, exportFile } : track;
        });
        return answer;
      }
      if (command === "get_playlist_export_space") {
        // `__exportSpaceGate` (a promise) holds the answer back, to show
        // the track list doesn't wait for it.
        await window.__exportSpaceGate;
        return backend.ok({ exportSpace: window.__exportSpace || null });
      }
      if (command === "set_export_sync_mode") {
        // As the backend: the open playlist's sort is saved first when the new
        // mode locks its order.
        const lockedAfter = (pruneStale) => {
          const before = currentPruneStale;
          currentPruneStale = pruneStale;
          const status = exportStatusFor().find((entry) => entry.playlistId === request.playlistId);
          currentPruneStale = before;
          return usbSelected && !!status?.locksReorder;
        };
        const locked = lockedAfter(request.pruneStale);
        if (locked && request.sort && !lockedAfter(currentPruneStale)) {
          const tracks = backend.store.playlistTracks;
          tracks.set(request.playlistId, backend.applyReorder(tracks.get(request.playlistId) || [], request.sort));
        }
        currentPruneStale = request.pruneStale;
        const open = backend.store.playlists.find((playlist) => playlist.id === request.playlistId);
        return backend.ok({
          pruneStale: currentPruneStale,
          locked,
          statusText: locked
            ? `Export sync mode: additive — "${open.name}" already exists on USB, so its track order is locked here (current order kept).`
            : currentPruneStale ? "Export sync mode: mirror (exact match)" : "Export sync mode: additive"
        });
      }
      if (command === "list_playlists") {
        const noUsb = (status) => ({
          ...status,
          usbSelected: false,
          sameNameExistsOnUsb: false,
          locksReorder: false,
          markLabel: backend.store.playlists.find((playlist) => playlist.id === status.playlistId)?.lastExportedAt
            ? "Exported to USB"
            : null,
          exportButtonText: "Select USB first",
          exportButtonTitle: "Select a valid USB folder first"
        });
        const answer = backend.answer(command, payload);
        const statuses = exportStatusFor();
        answer.data.playlistUsbExportStatus = usbSelected ? statuses : statuses.map(noUsb);
        return answer;
      }
      if (command === "pick_usb_folder") return usbSameNamePlaylistName ? "/USB" : null;
      if (command === "select_usb") {
        usbSelected = !!usbSameNamePlaylistName;
        return backend.ok({ valid: usbSelected, normalizedRoot: usbSelected ? "/USB" : "", warnings: [] });
      }
      if (command === "fetch_usb_playlists") {
        const items = usbSameNamePlaylistName
          ? [{ id: "usb-pl-1", name: usbSameNamePlaylistName, source: "mock-tauri", tracks: [{ title: "USB Track" }], trackCount: 1 }]
          : [];
        return backend.ok({
          items,
          stats: { indexedTracks: 0, playlistReferencedTracks: 0, playlistEntries: items.length },
          playlistUsbExportStatus: exportStatusFor()
        });
      }
    });
  }, { usbSameNamePlaylistName, exportPruneStale, playlists });
}

async function dragPlaylistTrackRow(page, fromTrackId, toTrackId, dropNearTop) {
  const source = page.locator(`#playlistTracksBody .track-grid-row[data-track-id="${fromTrackId}"] [data-playlist-track-drag-handle]`);
  const target = page.locator(`#playlistTracksBody .track-grid-row[data-track-id="${toTrackId}"]`);
  const sourceBox = await source.boundingBox();
  const targetBox = await target.boundingBox();
  await page.mouse.move(sourceBox.x + sourceBox.width / 2, sourceBox.y + sourceBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(sourceBox.x + sourceBox.width / 2, sourceBox.y + sourceBox.height / 2 + 10, { steps: 5 });
  const targetY = dropNearTop ? targetBox.y + targetBox.height * 0.25 : targetBox.y + targetBox.height * 0.75;
  await page.mouse.move(targetBox.x + targetBox.width / 2, targetY, { steps: 10 });
  await page.mouse.up();
}

test("playlist track reorder", async ({ page }) => {
  await installReorderTauriMock(page);
  await page.goto("/");
  // Playlist track drag-and-drop reorder persists the new order.
  {
    await page.locator("#navPlaylistList .nav-playlist-item").first().click();
    const rows = page.locator("#playlistTracksBody .track-grid-row");
    await expect(rows).toHaveCount(3);
    await expect(rows.nth(0)).toHaveAttribute("data-track-id", "t1");
    await expect(rows.nth(1)).toHaveAttribute("data-track-id", "t2");
    await expect(rows.nth(2)).toHaveAttribute("data-track-id", "t3");

    // Drag "Song A" (t1, row 0) to below "Song C" (t3, row 2).
    await dragPlaylistTrackRow(page, "t1", "t3", false);

    await expect(rows.nth(0)).toHaveAttribute("data-track-id", "t2");
    await expect(rows.nth(1)).toHaveAttribute("data-track-id", "t3");
    await expect(rows.nth(2)).toHaveAttribute("data-track-id", "t1");

    const calls = await reorderRequests(page);
    expect(calls.length).toBeGreaterThan(0);
    expect(calls[calls.length - 1]).toEqual({ playlistId: "pl-1", moveTrackId: "t1", beforeTrackId: null });
  }

  await page.reload();
  // Playlist track drag that does not change position skips persisting order.
  {
    await page.locator("#navPlaylistList .nav-playlist-item").first().click();
    const rows = page.locator("#playlistTracksBody .track-grid-row");
    await expect(rows).toHaveCount(3);

    const handle = page.locator('#playlistTracksBody .track-grid-row[data-track-id="t1"] [data-playlist-track-drag-handle]');
    const box = await handle.boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 2, { steps: 3 });
    await page.mouse.up();

    await expect(rows.nth(0)).toHaveAttribute("data-track-id", "t1");
    const calls = await reorderRequests(page);
    expect(calls).toEqual([]);
  }

  await page.reload();
  // Playlist track drag handle stays enabled while a column sort is active.
  {
    await page.locator("#navPlaylistList .nav-playlist-item").first().click();
    await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
    await expect(page.locator("#playlistTracksBody [data-playlist-track-drag-handle]")).toHaveCount(3);

    await page.locator('#panel-playlist .track-grid-cell.sortable[data-sort-key="artist"]').click();
    await expect(page.locator("#playlistTracksBody [data-playlist-track-drag-handle]")).toHaveCount(3);
  }

  await page.reload();
  // Dragging a playlist track while sorted commits the sorted order first, then applies the manual
  // move.
  {
    await page.locator("#navPlaylistList .nav-playlist-item").first().click();
    const rows = page.locator("#playlistTracksBody .track-grid-row");
    await expect(rows).toHaveCount(3);

    // Sort by artist -> view becomes [t2 (Alex), t3 (Max), t1 (Nina)], distinct
    // from the persisted insertion order [t1, t2, t3].
    await page.locator('#panel-playlist .track-grid-cell.sortable[data-sort-key="artist"]').click();
    await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();
    await expect(rows.nth(0)).toHaveAttribute("data-track-id", "t2");
    await expect(rows.nth(1)).toHaveAttribute("data-track-id", "t3");
    await expect(rows.nth(2)).toHaveAttribute("data-track-id", "t1");

    // Drag "Alex" (t2, row 0) down past "Nina" (t1, row 2) while sorted.
    await dragPlaylistTrackRow(page, "t2", "t1", false);

    // The sorted view is persisted FIRST (sort-commit), then the single move --
    // so `beforeTrackId` lines up with what the user actually saw.
    const calls = await reorderRequests(page);
    expect(calls).toEqual([
      { playlistId: "pl-1", sortBy: "artist", sortDir: "asc" },
      { playlistId: "pl-1", moveTrackId: "t2", beforeTrackId: null },
    ]);

    // Sort indicator cleared the moment the drag started; nothing jumps around --
    // final order is the sorted order with just t2 dragged to the end.
    await expect(page.locator("#panel-playlist .sort-hint")).toBeHidden();
    await expect(page.locator("#panel-playlist .sortable.sort-asc, #panel-playlist .sortable.sort-desc")).toHaveCount(0);
    await expect(rows.nth(0)).toHaveAttribute("data-track-id", "t3");
    await expect(rows.nth(1)).toHaveAttribute("data-track-id", "t1");
    await expect(rows.nth(2)).toHaveAttribute("data-track-id", "t2");
  }

  await page.reload();
  // Playlist track drag handle is hidden while a search filter is active.
  {
    await page.locator("#navPlaylistList .nav-playlist-item").first().click();
    await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
    await expect(page.locator("#playlistTracksBody [data-playlist-track-drag-handle]")).toHaveCount(3);
    await expect(page.locator("#playlistTotalDuration")).toHaveText("Total time: 9:00");

    await page.locator("#playlistSearchInput").fill("Song A");
    await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(1);
    await expect(page.locator("#playlistTracksBody [data-playlist-track-drag-handle]")).toHaveCount(0);
    // Search is a backend query param now (like the library and USB views), so
    // the footer total tracks the filtered set -- one 3:00 track here.
    await expect(page.locator("#playlistTotalDuration")).toHaveText("Total time: 3:00");
  }
});

async function connectUsbAndFetchPlaylists(page) {
  await openUsb(page, "usb-playlists");
  await expect(page.locator("#usbPlaylists [data-usb-playlist]")).toHaveCount(1);
}

test("sidebar checkmarks show which playlists are on the connected USB", async ({ page }) => {
  await installReorderTauriMock(page, { usbSameNamePlaylistName: "Reorder Playlist" });
  await page.addInitScript(() => window.__e2eBackend.store.playlists.push({
    id: "pl-2", name: "Elsewhere Set",
    lastExportedAt: "2026-04-07T10:00:00Z", lastExportedUsbRoot: "/OTHER_USB", lastExportedTrackCount: 2
  }));
  await page.goto("/");

  const statusOf = (id) => page.locator(`#navPlaylistList .nav-playlist-item[data-playlist-id="${id}"] .nav-playlist-status`);

  // No USB yet: the mark means "exported at some point".
  await expect(statusOf("pl-1")).not.toHaveClass(/exported/);
  await expect(statusOf("pl-2")).toHaveClass(/exported/);
  await expect(statusOf("pl-2")).toHaveAttribute("aria-label", "Exported to USB");

  // With this USB scanned, the mark means "on this USB".
  await connectUsbAndFetchPlaylists(page);
  await expect(statusOf("pl-1")).toHaveClass(/exported/);
  await expect(statusOf("pl-1")).toHaveText("✓");
  await expect(statusOf("pl-1")).toHaveAttribute("aria-label", "On this USB");
  await expect(statusOf("pl-2")).not.toHaveClass(/exported/);
  await expect(statusOf("pl-2")).toHaveText("");

  // The line under the open playlist's title says the same.
  await page.locator('#navPlaylistList .nav-playlist-item[data-playlist-id="pl-1"]').click();
  await expect(page.locator("#playlistExportStatus")).toHaveText("On this USB.");
  await page.locator('#navPlaylistList .nav-playlist-item[data-playlist-id="pl-2"]').click();
  await expect(page.locator("#playlistExportStatus")).toHaveText(/^Not on this USB\. Last exported .+ to \/OTHER_USB \(2 tracks\)\.$/);
});

// Playlist track drag handle is disabled with a tooltip when additive export won't reorder it on
// the USB; playlist header sort is disabled with a tooltip when additive export won't reorder it on
// the USB; changing the export sync mode releases and re-engages the open playlist's reorder lock
// without a USB rescan.
test("additive export reorder lock", async ({ page }) => {
  await installReorderTauriMock(page, { usbSameNamePlaylistName: "Reorder Playlist", exportPruneStale: false });
  await page.goto("/");
  {
    await connectUsbAndFetchPlaylists(page);

    await page.locator("#navPlaylistList .nav-playlist-item").first().click();
    const rows = page.locator("#playlistTracksBody .track-grid-row");
    await expect(rows).toHaveCount(3);
    await expect(page.locator("#playlistTracksBody [data-playlist-track-drag-handle]")).toHaveCount(0);

    const disabledHandles = page.locator("#playlistTracksBody .drag-handle.disabled");
    await expect(disabledHandles).toHaveCount(3);
    await expect(disabledHandles.first()).toHaveAttribute(
      "data-tooltip",
      "Won't reorder on USB — \"Reorder Playlist\" already exists there, and additive export keeps its existing track order unchanged. New tracks are still added in your chosen order."
    );

    // Same backend-computed collision also drives the Export button into
    // "append" mode -- one flag, two UI spots.
    await expect(page.locator("#exportPlaylistBtn")).toHaveText("Append to (Reorder Playlist) on USB: (USB)");
    await expect(page.locator("#exportPlaylistBtn")).toHaveAttribute(
      "data-tooltip",
      'Append current playlist tracks to existing USB playlist "Reorder Playlist"'
    );

    // Dragging the disabled handle is a no-op -- it isn't draggable, so no
    // dragstart/reorder ever fires.
    const box = await disabledHandles.first().boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 40, { steps: 5 });
    await page.mouse.up();

    await expect(rows.nth(0)).toHaveAttribute("data-track-id", "t1");
    const calls = await reorderRequests(page);
    expect(calls).toEqual([]);
  }

  await page.reload();
  {
    await connectUsbAndFetchPlaylists(page);

    await page.locator("#navPlaylistList .nav-playlist-item").first().click();
    const rows = page.locator("#playlistTracksBody .track-grid-row");
    await expect(rows).toHaveCount(3);

    const artistHeader = page.locator('#panel-playlist .track-grid-cell.sortable[data-sort-key="artist"]');
    await expect(artistHeader).toHaveAttribute(
      "data-tooltip",
      "Won't reorder on USB — \"Reorder Playlist\" already exists there, and additive export keeps its existing track order unchanged. New tracks are still added in your chosen order."
    );

    await artistHeader.click();

    // Clicking a locked header is a no-op: no hint, no sort classes, no reorder of rows.
    await expect(page.locator("#panel-playlist .sort-hint")).toBeHidden();
    await expect(artistHeader).not.toHaveClass(/sort-asc|sort-desc/);
    await expect(rows.nth(0)).toHaveAttribute("data-track-id", "t1");
    await expect(rows.nth(1)).toHaveAttribute("data-track-id", "t2");
    await expect(rows.nth(2)).toHaveAttribute("data-track-id", "t3");
  }

  await page.reload();
  {
    await connectUsbAndFetchPlaylists(page);
    await page.locator("#navPlaylistList .nav-playlist-item").first().click();

    const grid = page.locator('[data-track-grid][data-body-id="playlistTracksBody"]');
    const dragHandles = page.locator("#playlistTracksBody [data-playlist-track-drag-handle]");
    const artistHeader = page.locator('#panel-playlist .track-grid-cell.sortable[data-sort-key="artist"]');
    const lockedTooltip =
      "Won't reorder on USB — \"Reorder Playlist\" already exists there, and additive export keeps its existing track order unchanged. New tracks are still added in your chosen order.";

    await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);

    // additive + same name on USB -> locked
    await expect(grid).toHaveAttribute("data-sort-locked", "true");
    await expect(dragHandles).toHaveCount(0);
    await expect(artistHeader).toHaveAttribute("data-tooltip", lockedTooltip);
    await expect(page.locator("#exportPlaylistBtn")).toHaveText("Append to (Reorder Playlist) on USB: (USB)");

    // switch to mirror -> lock releases immediately (no fetch_usb_playlists rerun)
    await page.locator("#settingsBtn").click();
    await page.locator("#exportSyncModeMirror").check({ force: true });
    await page.locator("#settingsCloseBtn").click();

    await expect(grid).toHaveAttribute("data-sort-locked", "false");
    await expect(dragHandles).toHaveCount(3);
    await expect(artistHeader).not.toHaveAttribute("data-tooltip", /.+/);
    await expect(page.locator("#exportPlaylistBtn")).toHaveText("Export to USB: USB");

    // a view-only column sort is now allowed
    await artistHeader.click();
    await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();

    // switch back to additive -> lock re-engages, active sort committed
    await page.locator("#settingsBtn").click();
    await page.locator("#exportSyncModeAdditive").check({ force: true });
    await page.locator("#settingsCloseBtn").click();

    await expect(grid).toHaveAttribute("data-sort-locked", "true");
    await expect(dragHandles).toHaveCount(0);
    await expect(page.locator("#panel-playlist .sort-hint")).toBeHidden();
    await expect(page.locator("#exportPlaylistBtn")).toHaveText("Append to (Reorder Playlist) on USB: (USB)");

    // The mode change carries the sort along: the backend saves it before the lock.
    const requests = await page.evaluate(() =>
      window.__calls.filter((c) => c.command === "set_export_sync_mode").map((c) => c.request));
    expect(requests).toEqual([
      { pruneStale: true, playlistId: "pl-1", sort: null },
      { pruneStale: false, playlistId: "pl-1", sort: { sortBy: "artist", sortDir: "asc" } }
    ]);
    expect(await reorderRequests(page)).toEqual([]);
    await expect(page.locator("#statusText")).toContainText('"Reorder Playlist" already exists on USB, so its track order is locked here');
  }

  // With no same-name playlist on the USB, additive mode leaves the drag handle enabled.
  await page.evaluate(() => sessionStorage.setItem("usbPlaylistName", "Some Other Playlist"));
  await page.reload();
  {
    await connectUsbAndFetchPlaylists(page);

    await page.locator("#navPlaylistList .nav-playlist-item").first().click();
    await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
    await expect(page.locator("#playlistTracksBody [data-playlist-track-drag-handle]")).toHaveCount(3);
    await expect(page.locator("#playlistTracksBody .drag-handle.disabled")).toHaveCount(0);
    await expect(page.locator("#exportPlaylistBtn")).toHaveText("Export to USB: USB");
  }
});

// With a USB connected, the FMT column shows what export puts on it, as the backend says; export is
// disabled from the click until the export finishes, so a double click starts one export; the
// backend's export gate rejection is shown in the status bar; a search with no matches doesn't
// change what Export to USB does; export sends an active column sort along, and a failed export
// keeps it on screen.
test("playlist export", async ({ page }) => {
  // Many steps in one test: past 20 s with every spec running in parallel, close to the
  // 30 s default (each step just runs slower; none hangs). Three times the time.
  test.slow();
  await installReorderTauriMock(page, { usbSameNamePlaylistName: "Reorder Playlist" });
  await page.goto("/");
  {
    await page.evaluate(() => {
      window.__exportFiles = {
        t1: { kind: "transcode", badge: "AIFF", tooltip: "AIFF transcode · 40 MB, exported to this USB — source: FLAC · 44.1 kHz · 16-bit" },
        t2: { kind: "convert", badge: "MP3", tooltip: "Converted to MP3 320 on export — source: FLAC · 44.1 kHz · 16-bit" },
        t3: { kind: "refused", badge: "AAC", tooltip: "M4A · 48 kHz · 256 kbps — AAC can't go on this USB" },
      };
    });
    await connectUsbAndFetchPlaylists(page);
    await page.locator("#navPlaylistList .nav-playlist-item").first().click();
    const badge = (id) => page.locator(`#playlistTracksBody .track-grid-row[data-track-id="${id}"] .format-badge`);
    await expect(badge("t1")).toHaveText("AIFF");
    await expect(badge("t1")).toHaveAttribute("data-tooltip", /^AIFF transcode · 40 MB, exported to this USB — source: FLAC/);
    await expect(badge("t1")).not.toHaveClass(/export-convert|warn/);
    await expect(badge("t2")).toHaveText("MP3");
    await expect(badge("t2")).toHaveClass(/export-convert/);
    await expect(badge("t2")).toHaveAttribute("data-tooltip", /^Converted to MP3 320 on export — source: FLAC/);
    await expect(badge("t3")).toHaveText("AAC ⚠");
    await expect(badge("t3")).toHaveClass(/warn/);
    await expect(badge("t3")).toHaveAttribute("data-tooltip", /AAC can't go on this USB/);
  }

  await page.reload();
  {
    await connectUsbAndFetchPlaylists(page);
    await page.locator("#navPlaylistList .nav-playlist-item").first().click();
    await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
    // Hold export_to_usb open until the test releases it, counting the calls.
    await page.evaluate(() => {
      window.__exportCalls = 0;
      const backend = window.__e2eBackend;
      backend.override(async (command, request, payload, next) => {
        if (command !== "export_to_usb") return next();
        window.__exportCalls += 1;
        await new Promise((resolve) => { window.__finishExport = resolve; });
        return { ok: false, error: { code: "INTERNAL", message: "stopped by test" } };
      });
    });

    const exportBtn = page.locator("#exportPlaylistBtn");
    await exportBtn.dblclick();
    await expect(exportBtn).toBeDisabled();
    expect(await page.evaluate(() => window.__exportCalls)).toBe(1);

    await page.evaluate(() => window.__finishExport());
    await expect(page.locator("#statusText")).toContainText("stopped by test");
    await expect(exportBtn).toBeEnabled();
    expect(await page.evaluate(() => window.__exportCalls)).toBe(1);
  }

  await page.reload();
  {
    await connectUsbAndFetchPlaylists(page);
    await openReorderPlaylist(page);
    await answerExports(page, {
      ok: false,
      error: { code: "VALIDATION", message: "the log's wording", details: { statusText: "mock export blocked text" } }
    });

    await page.locator("#exportPlaylistBtn").click();
    await expect(page.locator("#statusText")).toHaveText("mock export blocked text");
  }

  await page.reload();
  {
    await connectUsbAndFetchPlaylists(page);
    await openReorderPlaylist(page);
    await answerExports(page, { ok: true, data: { exportedTracks: 3, statusText: "Export complete" } });

    await page.locator("#playlistSearchInput").fill("no such track");
    await expect(page.locator("#playlistTotalDuration")).toHaveText("Total time: 0:00");
    await page.locator("#exportPlaylistBtn").click();
    await expect(page.locator("#statusText")).toContainText("Export complete");
  }

  await page.reload();
  {
    await connectUsbAndFetchPlaylists(page);
    await openReorderPlaylist(page);
    const artistHeader = page.locator('#panel-playlist .track-grid-cell.sortable[data-sort-key="artist"]');
    await answerExports(page, { ok: true, data: { exportedTracks: 3, statusText: "Export complete" } });

    // The backend saves the sort as the order and exports, in one command.
    await artistHeader.click();
    await expect(artistHeader).toHaveClass(/sort-asc/);
    await page.locator("#exportPlaylistBtn").click();
    await expect(page.locator("#statusText")).toContainText("Export complete");
    const flow = await page.evaluate(() => window.__exportFlow);
    expect(flow.map((call) => call.command)).toEqual(["export_to_usb"]);
    expect(flow[0].sort).toEqual({ sortBy: "artist", sortDir: "asc" });
    // The page shows the order it exported, with no sort left over.
    await expect(artistHeader).not.toHaveClass(/sort-asc/);

    // A failed export says so once; the sort stays, and the playlists are read
    // again (the export may have saved the sort or changed the USB).
    const readsBefore = await page.evaluate(() => {
      window.__exportFlow = [];
      window.__exportAnswer = { ok: false, error: { code: "IO", message: "disk full" } };
      return window.__playlistReads || 0;
    });
    await artistHeader.click();
    await page.locator("#exportPlaylistBtn").click();
    await expect(page.locator("#statusText")).toHaveText("Export failed: disk full. See Event Log for details.");
    await expect(artistHeader).toHaveClass(/sort-asc/);
    expect(await page.evaluate(() => window.__exportFlow.map((call) => call.command))).toEqual(["export_to_usb"]);
    await expect.poll(() => page.evaluate(() => window.__playlistReads)).toBe(readsBefore + 1);
  }

  // A failed export (the mock answers "Unhandled") is reported in the status
  // bar and left to the backend's one Event Log entry: the real backend logs
  // every failed command once, the frontend adds nothing.
  await page.reload();
  {
    await connectUsbAndFetchPlaylists(page);
    await page.locator("#navPlaylistList .nav-playlist-item").first().click();
    await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);

    await page.locator("#exportPlaylistBtn").click();
    await expect(page.locator("#statusText")).toContainText("Export failed: Unhandled: export_to_usb. See Event Log for details.");
    await page.locator("#eventLogBtn").click();
    await expect(page.locator("#eventLogList")).not.toContainText("export_to_usb");
  }

  // The playlist heading shows what an export would copy against the USB's free space.
  await page.reload();
  await page.evaluate(() => {
    window.__exportSpace = { bytesToCopy: 1.2 * 1024 ** 3, tracksToCopy: 2, availableBytes: 27 * 1024 ** 3, fits: true, summaryText: "2 of 3 tracks to copy · 1.2 GB · 27.0 GB free on USB" };
  });
  {
    // No USB yet: no estimate.
    await page.locator("#navPlaylistList .nav-playlist-item").first().click();
    await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
    await expect(page.locator("#playlistExportSpace")).toBeHidden();

    // The estimate is asked for after the rows are shown: a slow one leaves the
    // list usable and fills the line in when it arrives.
    // Held back from the start: connecting the USB asks for it too.
    await page.evaluate(() => {
      window.__exportSpaceGate = new Promise((resolve) => { window.__releaseExportSpace = resolve; });
    });
    await connectUsbAndFetchPlaylists(page);
    await page.locator("#navPlaylistList .nav-playlist-item").first().click();
    const space = page.locator("#playlistExportSpace");
    await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
    await expect(space).toBeHidden();
    await page.evaluate(() => window.__releaseExportSpace());
    await expect(space).toHaveText("2 of 3 tracks to copy · 1.2 GB · 27.0 GB free on USB");
    await expect(space).not.toHaveClass(/is-warning/);

    // The backend decides it won't fit; the next load (here a search) shows it.
    await page.evaluate(() => {
      window.__exportSpace = { bytesToCopy: 31 * 1024 ** 3, tracksToCopy: 3, availableBytes: 27 * 1024 ** 3, fits: false, summaryText: "Doesn't fit: 3 of 3 tracks to copy · 31.0 GB · 27.0 GB free on USB" };
    });
    await page.locator("#playlistSearchInput").fill("Song");
    await expect(space).toHaveText("Doesn't fit: 3 of 3 tracks to copy · 31.0 GB · 27.0 GB free on USB");
    await expect(space).toHaveClass(/is-warning/);

    await page.evaluate(() => {
      window.__exportSpace = { bytesToCopy: 0, tracksToCopy: 0, availableBytes: 27 * 1024 ** 3, fits: true, summaryText: "All tracks on USB · 27.0 GB free on USB" };
    });
    await page.locator("#playlistSearchInput").fill("");
    await expect(space).toHaveText("All tracks on USB · 27.0 GB free on USB");
    await expect(space).not.toHaveClass(/is-warning/);

    // What the USB audio formats mean for the export, counted by the
    // backend: tracks it converts, and lossy ones it refuses.
    const formatWarning = page.locator("#playlistFormatWarning");
    await expect(formatWarning).toBeHidden();
    await page.evaluate(() => {
      window.__exportSpace = {
        bytesToCopy: 0, tracksToCopy: 0, availableBytes: 27 * 1024 ** 3, fits: true,
        formatNotice: "2 tracks will be converted to MP3 320 on export", formatNoticeBlocks: false,
      };
    });
    await page.locator("#playlistSearchInput").fill("Song");
    await expect(formatWarning).toHaveText("2 tracks will be converted to MP3 320 on export");
    await expect(formatWarning).not.toHaveClass(/is-warning/);
    await page.evaluate(() => {
      window.__exportSpace = {
        bytesToCopy: 0, tracksToCopy: 0, availableBytes: 27 * 1024 ** 3, fits: true,
        formatNotice: "1 track will be converted on export · 3 AAC tracks can't go on this USB; export is blocked",
        formatNoticeBlocks: true,
      };
    });
    await page.locator("#playlistSearchInput").fill("");
    await expect(formatWarning).toHaveText(
      "1 track will be converted on export · 3 AAC tracks can't go on this USB; export is blocked"
    );
    await expect(formatWarning).toHaveClass(/is-warning/);
  }
});

// Answers export_to_usb with `window.__exportAnswer` (first `answer`) and
// records it and sort commits (reorder_playlist_tracks) in
// `window.__exportFlow`; `window.__playlistReads` counts list_playlists.
async function answerExports(page, answer) {
  await page.evaluate((answer) => {
    window.__exportFlow = [];
    window.__exportAnswer = answer;
    const backend = window.__e2eBackend;
    backend.override(async (command, request, payload, next) => {
      if (command === "export_to_usb") {
        window.__exportFlow.push({ command, options: payload?.request?.options, sort: payload?.request?.sort });
        return window.__exportAnswer;
      }
      if (command === "reorder_playlist_tracks") window.__exportFlow.push({ command });
      if (command === "list_playlists") window.__playlistReads = (window.__playlistReads || 0) + 1;
    });
  }, answer);
}

async function openReorderPlaylist(page) {
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);
}

test("a successful export clears the stale diagnostics report and sends the backup setting", async ({ page }) => {
  await installReorderTauriMock(page, { usbSameNamePlaylistName: "Reorder Playlist" });
  await page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.exportBackup", "0");
    window.__historyReads = 0;
    const backend = window.__e2eBackend;
    backend.override(async (command, request, payload, next) => {
      if (command === "fetch_usb_histories") {
        window.__historyReads += 1;
        return backend.ok({
          items: [{ id: "h-1", name: "HISTORY 001", tracks: [] }],
          summaryText: "1 session, 3 tracks", counts: { importedPlaylists: 1, importedTracks: 3 },
          warnings: []
        });
      }
    });
  });
  await page.goto("/");
  await connectUsbAndFetchPlaylists(page);
  await expect(page.locator("#diagOverallStatus")).toHaveText("PASS");
  await page.locator('.nav-item[data-view="usb-history"]').click();
  await expect(page.locator("#historyList")).toContainText("HISTORY 001");
  await openReorderPlaylist(page);
  await answerExports(page, { ok: true, data: { playlistName: "Reorder Playlist", exportedTracks: 3, statusText: "Export complete: Reorder Playlist - 3 tracks, 0 skipped" } });

  await page.locator("#exportPlaylistBtn").click();
  await expect(page.locator("#statusText")).toContainText("Export complete: Reorder Playlist - 3 tracks, 0 skipped");
  await expect(page.locator("#diagOverallStatus")).toHaveText("");
  const flow = await page.evaluate(() => window.__exportFlow);
  expect(flow.map((call) => call.command)).toEqual(["export_to_usb"]);
  expect(flow[0].options.backupBeforeExport).toBe(false);
  // The loaded USB playlists and their totals are out of date now.
  await expect(page.locator("#usbPlaylists [data-usb-playlist]")).toHaveCount(0);
  await expect(page.locator("#usbCountsText")).toHaveText("");

  // Opening the page reads them again.
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(page.locator("#usbPlaylists [data-usb-playlist]")).toHaveCount(1);

  // An export doesn't touch the history: it stays loaded, with its totals.
  await page.locator('.nav-item[data-view="usb-history"]').click();
  await expect(page.locator("#historyList")).toContainText("HISTORY 001");
  await expect(page.locator("#historyCountsText")).toHaveText("1 session, 3 tracks");
  expect(await page.evaluate(() => window.__historyReads)).toBe(1);
});

// Switching away from a sorted playlist commits the sort as its real order, and the next playlist
// doesn't inherit it.
test("leaving a sorted locked playlist", async ({ page }) => {
  // Column sort is a free, reversible view op while browsing -- it only
  // becomes the playlist's real (and thus exported) order once you leave
  // the view. The playlist panel's header/table DOM is shared across every
  // playlist, so the *next* playlist viewed must never show a leftover
  // hint/arrow from a sort that belonged to a different one.
  await installReorderTauriMock(page, {
    usbSameNamePlaylistName: "Locked Playlist",
    exportPruneStale: false,
    playlists: [
      { id: "pl-free", name: "Free Playlist", tracks: songs(["t1", "Song A", "Zulu Album"], ["t2", "Song B", "Alpha Album"]) },
      { id: "pl-locked", name: "Locked Playlist", tracks: songs(["t3", "Song C", "Zulu Album"], ["t4", "Song D", "Alpha Album"]) }
    ]
  });
  await page.goto("/");
  await connectUsbAndFetchPlaylists(page);

  // Sort the unlocked playlist by Album.
  await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Free Playlist" }).click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);
  await page.locator('#panel-playlist .sortable[data-sort-key="album"]').click();
  await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();
  await expect(page.locator("#playlistTracksBody .track-grid-row .track-title")).toHaveText(["Song B", "Song A"]);

  // Sorting alone must not have committed anything yet.
  expect(await reorderRequests(page).then((calls) => calls.length)).toBe(0);

  // Switch to the locked playlist -- leaving Free Playlist commits its sort.
  await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Locked Playlist" }).click();
  const lockedRows = page.locator("#playlistTracksBody .track-grid-row");
  await expect(lockedRows).toHaveCount(2);

  const reorderCalls = await reorderRequests(page);
  expect(reorderCalls).toEqual([{ playlistId: "pl-free", sortBy: "album", sortDir: "asc" }]);

  // The locked playlist must not inherit the sort -- it was never sorted itself.
  await expect(page.locator("#panel-playlist .sort-hint")).toBeHidden();
  await expect(page.locator('#panel-playlist .sortable[data-sort-key="album"]')).not.toHaveClass(/sort-asc|sort-desc/);
  await expect(lockedRows.nth(0)).toHaveAttribute("data-track-id", "t3");
  await expect(lockedRows.nth(1)).toHaveAttribute("data-track-id", "t4");

  // A sort whose save fails still lets the view change, and says why.
  await page.evaluate(() => {
    const backend = window.__e2eBackend;
    backend.override(async (command) => {
      if (command === "reorder_playlist_tracks") return backend.fail("Save track order failed: boom");
    });
  });
  await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Free Playlist" }).click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);
  await page.locator('#panel-playlist .sortable[data-sort-key="album"]').click();
  await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();
  await page.locator('.nav-item[data-view="library"]').click();
  await expect(page.locator("#panel-library")).toHaveClass(/active/);
  await expect(page.locator("#statusText")).toContainText("Save track order failed: boom");
});

function installTwoPlaylistTauriMock(page) {
  return installReorderTauriMock(page, {
    playlists: [
      { id: "pl-a", name: "Playlist A", tracks: songs(["t1", "Song A", "Zulu Album"], ["t2", "Song B", "Alpha Album"]) },
      { id: "pl-b", name: "Playlist B", tracks: songs(["t3", "Song C", "Zulu Album"], ["t4", "Song D", "Alpha Album"]) }
    ]
  });
}

test("leaving a sorted playlist", async ({ page }) => {
  await installTwoPlaylistTauriMock(page);
  await page.goto("/");
  // Switching between two unlocked playlists commits the outgoing sort and doesn't carry it into
  // the next.
  {
    await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Playlist A" }).click();
    await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);
    await page.locator('#panel-playlist .sortable[data-sort-key="album"]').click();
    await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();
    await expect(page.locator("#playlistTracksBody .track-grid-row .track-title")).toHaveText(["Song B", "Song A"]);

    await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Playlist B" }).click();
    const rowsB = page.locator("#playlistTracksBody .track-grid-row");
    await expect(rowsB).toHaveCount(2);

    const reorderCalls = await reorderRequests(page);
    expect(reorderCalls).toEqual([{ playlistId: "pl-a", sortBy: "album", sortDir: "asc" }]);

    await expect(page.locator("#panel-playlist .sort-hint")).toBeHidden();
    await expect(page.locator('#panel-playlist .sortable[data-sort-key="album"]')).not.toHaveClass(/sort-asc|sort-desc/);
    await expect(rowsB.nth(0)).toHaveAttribute("data-track-id", "t3");
    await expect(rowsB.nth(1)).toHaveAttribute("data-track-id", "t4");
  }

  await page.reload();
  // Switching to a non-playlist view (Library) also commits an active playlist sort.
  {
    await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Playlist A" }).click();
    await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);
    await page.locator('#panel-playlist .sortable[data-sort-key="album"]').click();
    await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();

    await page.locator('.nav-item[data-view="library"]').click();
    await expect.poll(() => reorderRequests(page).then((calls) => calls.length)).toBeGreaterThan(0);
    const reorderCalls = await reorderRequests(page);
    expect(reorderCalls).toEqual([{ playlistId: "pl-a", sortBy: "album", sortDir: "asc" }]);
  }

  await page.reload();
  // A same-playlist refresh (search) does not clear or commit an active sort.
  {
    await page.locator("#navPlaylistList .nav-playlist-item", { hasText: "Playlist A" }).click();
    await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);
    await page.locator('#panel-playlist .sortable[data-sort-key="album"]').click();
    await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();
    await expect(page.locator("#playlistTracksBody .track-grid-row .track-title")).toHaveText(["Song B", "Song A"]);

    // Typing in the playlist search box refreshes the same playlist's view --
    // the sort must survive that (still just a view op), and search alone
    // must never trigger a commit (it would only submit the filtered subset).
    await page.locator("#playlistSearchInput").fill("Song");
    await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);
    await expect(page.locator("#panel-playlist .sort-hint")).toBeVisible();
    await expect(page.locator('#panel-playlist .sortable[data-sort-key="album"]')).toHaveClass(/sort-asc/);
    await expect(page.locator("#playlistTracksBody .track-grid-row .track-title")).toHaveText(["Song B", "Song A"]);
    expect(await reorderRequests(page).then((calls) => calls.length)).toBe(0);
  }
});

