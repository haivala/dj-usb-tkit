import { test, expect } from "./coverage-fixture.mjs";

function installTrackDetailMock(page, opts = {}) {
  return page.addInitScript((opts) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    window.__calls = [];

    const tracks = [
      {
        id: "t1",
        title: "Cue Track",
        artist: "Artist",
        album: "Album",
        filePath: "/music/one.mp3",
        bpm: 128,
        key: "Am",
        durationMs: 180000,
        analysisReady: true,
        waveformPreview: Array.from({ length: 80 }, (_, i) => (i % 7) * 12),
      },
    ];

    // Base64 of a small PWV5 payload (2 bytes/entry).
    const pwv5 = new Uint8Array(4000);
    for (let i = 0; i < pwv5.length; i += 2) {
      const h = 8 + (i % 20);
      const v = (2 << 13) | (3 << 10) | (5 << 7) | (h << 2);
      pwv5[i] = (v >> 8) & 0xff;
      pwv5[i + 1] = v & 0xff;
    }
    const detailWaveformB64 = btoa(String.fromCharCode.apply(null, pwv5));

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          window.__calls.push({ command, request: payload?.request ?? null });
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") return { ok: true, data: { found: false, path: null } };
          if (command === "list_playlists") return { ok: true, data: { items: [] } };
          if (command === "get_backend_log_buffer") return [];
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          if (command === "list_tracks" || command === "search_tracks") {
            return { ok: true, data: { total: tracks.length, items: tracks } };
          }
          if (command === "browse_source_files") {
            return { ok: true, data: { total: tracks.length, items: tracks, nextCursor: null, hasMore: false } };
          }
          if (command === "resolve_track_identity") {
            return { ok: true, data: { trackId: "t1", resolvedBy: "self", materialized: false } };
          }
          if (command === "get_track_detail") {
            return {
              ok: true,
              data: {
                track: tracks[0],
                firstBeatMs: 120,
                cues: (opts.seedCues || []).map((positionMs) => ({ positionMs, colorId: 5, name: "" })),
                detailWaveform: detailWaveformB64,
              },
            };
          }
          if (command === "play_resolved_track") {
            return { ok: true, data: { started: true, positionMs: 0, durationMs: 180000 } };
          }
          if (command === "stop_playback_native" || command === "get_playback_status_native") {
            return { ok: true, data: {} };
          }
          if (command === "save_track_analysis_edits") {
            return {
              ok: true,
              data: {
                trackId: "t1",
                firstBeatMs: payload?.request?.firstBeatMs ?? null,
                cues: payload?.request?.cues ?? [],
                bpm: payload?.request?.bpm ?? null,
                bpmAnalyzer: payload?.request?.bpm != null ? "user" : null,
                key: payload?.request?.key ?? null,
                keySource: payload?.request?.key != null ? "user" : null,
                anlzRegenerated: true,
              },
            };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        },
      },
      event: { listen: async () => () => {} },
    };
  }, opts);
}

test("track-detail modal adds a cue at the playhead and saves it", async ({ page }) => {
  await installTrackDetailMock(page);
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();

  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  await expect(page.locator("#trackDetailFirstBeatMs")).toHaveValue("120");

  // Clicking the waveform starts playback.
  await page.locator("#trackDetailWaveform").click({ position: { x: 40, y: 20 } });
  await expect
    .poll(() => page.evaluate(() => window.__calls.some((c) => c.command === "play_resolved_track")))
    .toBe(true);

  await page.locator("#trackDetailAddCue").click();
  await expect(page.locator("#trackDetailCueList .cue-row")).toHaveCount(1);

  await page.locator("#trackDetailSaveBtn").click();
  await expect(page.locator("#trackDetailOverlay")).toBeHidden();

  const saveCall = await page.evaluate(() =>
    window.__calls.find((c) => c.command === "save_track_analysis_edits")
  );
  expect(saveCall).toBeTruthy();
  expect(Array.isArray(saveCall.request.cues)).toBe(true);
  expect(saveCall.request.cues).toHaveLength(1);
  expect(Object.keys(saveCall.request.cues[0]).sort()).toEqual(["colorId", "name", "positionMs"]);
});

test("track-detail modal edits BPM, saves it, and the library row/tooltip update", async ({ page }) => {
  await installTrackDetailMock(page);
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();

  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  await expect(page.locator("#trackDetailBpm")).toHaveValue("128");

  await page.locator("#trackDetailBpm").fill("140.25");
  await page.locator("#trackDetailBpm").dispatchEvent("change");
  await page.locator("#trackDetailSaveBtn").click();
  await expect(page.locator("#trackDetailOverlay")).toBeHidden();

  const saveCall = await page.evaluate(() =>
    window.__calls.find((c) => c.command === "save_track_analysis_edits")
  );
  expect(saveCall.request.bpm).toBe(140.25);

  const bpmPill = page.locator('#libraryTableBody .track-grid-row .td-bpm .bpm-pill');
  await expect(bpmPill).toHaveText("140.25");
  await expect(bpmPill).toHaveAttribute("data-tooltip", "Manually set");
});

test("track-detail modal edits the musical key, saves it, and the library row updates", async ({ page }) => {
  await installTrackDetailMock(page);
  await page.goto("/");

  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  await expect(page.locator("#trackDetailKey")).toHaveValue("Am");

  await page.locator("#trackDetailKey").selectOption("F#m");
  await page.locator("#trackDetailSaveBtn").click();
  await expect(page.locator("#trackDetailOverlay")).toBeHidden();

  const saveCall = await page.evaluate(() =>
    window.__calls.find((c) => c.command === "save_track_analysis_edits")
  );
  expect(saveCall.request.key).toBe("F#m");

  const keyPill = page.locator('#libraryTableBody .track-grid-row .td-key .key-pill');
  await expect(keyPill).toHaveText("F#m");
});

test("key stepper steps through KEY_OPTIONS and wraps at the ends", async ({ page }) => {
  await installTrackDetailMock(page);
  await page.goto("/");
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();

  // Fixture key is "Am" (index 21 of 24: majors C..B, then minors Cm..Bm).
  await expect(page.locator("#trackDetailKey")).toHaveValue("Am");
  await page.locator("#trackDetailKeyPlus").click();
  await expect(page.locator("#trackDetailKey")).toHaveValue("A#m");
  await page.locator("#trackDetailKeyMinus").click();
  await page.locator("#trackDetailKeyMinus").click();
  await expect(page.locator("#trackDetailKey")).toHaveValue("G#m");

  // Wrap: stepping past the last minor (Bm) lands back on the first major (C).
  await page.locator("#trackDetailKey").selectOption("Bm");
  await page.locator("#trackDetailKeyPlus").click();
  await expect(page.locator("#trackDetailKey")).toHaveValue("C");
  // Wrap the other way: stepping back from the first major (C) lands on Bm.
  await page.locator("#trackDetailKeyMinus").click();
  await expect(page.locator("#trackDetailKey")).toHaveValue("Bm");
});

test("BPM stepper nudges by 0.01 and clamps to a positive value", async ({ page }) => {
  await installTrackDetailMock(page);
  await page.goto("/");
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();

  await expect(page.locator("#trackDetailBpm")).toHaveValue("128");
  await page.locator("#trackDetailBpmPlus").click();
  await expect(page.locator("#trackDetailBpm")).toHaveValue("128.01");
  await page.locator("#trackDetailBpmMinus").click();
  await page.locator("#trackDetailBpmMinus").click();
  await expect(page.locator("#trackDetailBpm")).toHaveValue("127.99");
});

test("double-click the waveform adds a cue at that position without starting playback", async ({ page }) => {
  await installTrackDetailMock(page);
  await page.goto("/");
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();

  // Default view is 0–120 s of the 180 s track; x≈300/1227 ≈ 24 % ⇒ ~29 s.
  await page.locator("#trackDetailWaveform").dblclick({ position: { x: 300, y: 100 } });
  await expect(page.locator("#trackDetailCueList .cue-row")).toHaveCount(1);

  await page.locator("#trackDetailSaveBtn").click();
  const saveCall = await page.evaluate(() =>
    window.__calls.find((c) => c.command === "save_track_analysis_edits")
  );
  expect(saveCall.request.cues).toHaveLength(1);
  expect(saveCall.request.cues[0].positionMs).toBeGreaterThan(15000);
  expect(saveCall.request.cues[0].positionMs).toBeLessThan(45000);
  // A cue-only edit still carries the track's current (unedited) bpm, not
  // null -- the on-device beat-grid rewrite needs a real bpm value even when
  // bpm itself wasn't touched this save.
  expect(saveCall.request.bpm).toBe(128);
  expect(
    await page.evaluate(() => window.__calls.some((c) => c.command === "play_resolved_track"))
  ).toBe(false);
});

test("cue names default to 'Cue N' and stay editable without losing focus", async ({ page }) => {
  await installTrackDetailMock(page);
  await page.goto("/");
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();

  await page.locator("#trackDetailAddCue").click();
  await page.locator("#trackDetailAddCue").click();
  const rows = page.locator("#trackDetailCueList .cue-row");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0).locator(".cue-row-name")).toHaveValue("Cue 1");
  await expect(rows.nth(1).locator(".cue-row-name")).toHaveValue("Cue 2");

  // Typing character-by-character must not lose focus (regression: the cue
  // list used to fully rebuild its DOM on every keystroke).
  const nameInput = rows.nth(0).locator(".cue-row-name");
  await nameInput.click();
  await nameInput.fill("");
  await nameInput.pressSequentially("Intro Drop", { delay: 15 });
  await expect(nameInput).toHaveValue("Intro Drop");
  expect(await nameInput.evaluate((el) => el === document.activeElement)).toBe(true);

  await page.locator("#trackDetailSaveBtn").click();
  const saveCall = await page.evaluate(() =>
    window.__calls.find((c) => c.command === "save_track_analysis_edits")
  );
  expect(saveCall.request.cues.map((c) => c.name).sort()).toEqual(["Cue 2", "Intro Drop"]);
});

test("a cue row's play button and its waveform marker both play from that cue's position", async ({ page }) => {
  await installTrackDetailMock(page, { seedCues: [90000] });
  await page.goto("/");
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  await expect(page.locator("#trackDetailCueList .cue-row")).toHaveCount(1);

  const playCalls = () =>
    page.evaluate(() => window.__calls.filter((c) => c.command === "play_resolved_track"));

  await page.locator("#trackDetailCueList .cue-row-play").click();
  await expect.poll(async () => (await playCalls()).length).toBe(1);
  expect((await playCalls()).at(-1).request.startRatio).toBeCloseTo(90000 / 180000, 2);

  await page.locator("#trackDetailCueMarkers .cue-marker").first().click();
  await expect.poll(async () => (await playCalls()).length).toBe(2);
  expect((await playCalls()).at(-1).request.startRatio).toBeCloseTo(90000 / 180000, 2);
});

test("the modal opens zoomed to ~2 min; Fit shows the whole track; zoom windows cue markers", async ({ page }) => {
  // durationMs 180000; one cue inside the default 2-min view, one past it.
  await installTrackDetailMock(page, { seedCues: [30000, 170000] });
  await page.goto("/");
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  await expect(page.locator("#trackDetailCueList .cue-row")).toHaveCount(2);

  const visibleMarkers = page.locator("#trackDetailCueMarkers .cue-marker:not(.off-view)");
  // Default view is 0–120 s, so only the 30 s cue is on screen.
  await expect(visibleMarkers).toHaveCount(1);

  // The zoom-range readout makes the initial zoomed-in view unmistakable: it
  // names the visible window vs the 3-min track and points at Fit.
  const zoomRange = page.locator("#trackDetailZoomRange");
  await expect(zoomRange).toBeVisible();
  await expect(zoomRange).toHaveClass(/is-zoomed/);
  await expect(zoomRange).toContainText("0:00–2:00 of 3:00");

  await page.locator("#trackDetailZoomFit").click();
  await expect(visibleMarkers).toHaveCount(2);
  // Fully zoomed out, the readout switches to a plain "whole track" state.
  await expect(zoomRange).not.toHaveClass(/is-zoomed/);
  await expect(zoomRange).toHaveText("Whole track");

  // Scroll-wheel zoom toward the left edge → the 170 s cue leaves the view again.
  await page.locator("#trackDetailWaveform").hover({ position: { x: 15, y: 100 } });
  await page.mouse.wheel(0, -500);
  await expect(visibleMarkers).toHaveCount(1);
});

test("cue editor opens + saves from an app-playlist track row", async ({ page }) => {
  // Regression: the app-playlist panel click handler only routed play/scrub
  // actions to handleTrackAction, so the cue button (added to every track list)
  // was inert there — clicking it did nothing.
  await page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    window.__calls = [];

    const pwv5 = new Uint8Array(4000);
    for (let i = 0; i < pwv5.length; i += 2) {
      const h = 8 + (i % 20);
      const v = (2 << 13) | (3 << 10) | (5 << 7) | (h << 2);
      pwv5[i] = (v >> 8) & 0xff;
      pwv5[i + 1] = v & 0xff;
    }
    const detailWaveformB64 = btoa(String.fromCharCode.apply(null, pwv5));

    const track = {
      id: "plt-entry-1",
      localTrackId: "local-1",
      title: "Playlist Cue Track",
      artist: "Artist",
      album: "Album",
      filePath: "/music/one.mp3",
      bpm: 128,
      durationMs: 180000,
      analysisReady: true,
      waveformPreview: Array.from({ length: 80 }, (_, i) => (i % 7) * 12),
    };

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const request = payload?.request ?? payload;
          window.__calls.push({ command, request: request ?? null });
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") return { ok: true, data: { found: false, path: null } };
          if (command === "get_backend_log_buffer") return [];
          if (command === "list_playlists") {
            return { ok: true, data: { items: [{ id: "pl-1", name: "My Set", source: "local", createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() }] } };
          }
          if (command === "get_playlist_tracks") {
            return { ok: true, data: { playlistId: "pl-1", items: [track], total: 1, nextCursor: null, hasMore: false, totalDurationMs: 180000, durationKnownCount: 1, unanalyzedCount: 0 } };
          }
          if (command === "list_tracks" || command === "search_tracks" || command === "browse_source_files") {
            return { ok: true, data: { total: 0, items: [], nextCursor: null, hasMore: false } };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          if (command === "resolve_track_identity") {
            return { ok: true, data: { trackId: "local-1", resolvedBy: "self", materialized: false } };
          }
          if (command === "get_track_detail") {
            return { ok: true, data: { track, firstBeatMs: 90, cues: [], detailWaveform: detailWaveformB64 } };
          }
          if (command === "save_track_analysis_edits") {
            return { ok: true, data: { trackId: "local-1", firstBeatMs: request?.firstBeatMs ?? null, cues: request?.cues ?? [], anlzRegenerated: true } };
          }
          if (command === "stop_playback_native" || command === "get_playback_status_native") return { ok: true, data: {} };
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
        },
      },
      event: { listen: async () => () => {} },
    };
  });
  await page.goto("/");

  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  const row = page.locator("#playlistTracksBody .track-grid-row");
  await expect(row).toHaveCount(1);

  await row.locator('[data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  await expect(page.locator("#trackDetailFirstBeatMs")).toHaveValue("90");

  await page.locator("#trackDetailAddCue").click();
  await expect(page.locator("#trackDetailCueList .cue-row")).toHaveCount(1);

  await page.locator("#trackDetailSaveBtn").click();
  await expect(page.locator("#trackDetailOverlay")).toBeHidden();

  const saveCall = await page.evaluate(() => window.__calls.find((c) => c.command === "save_track_analysis_edits"));
  expect(saveCall).toBeTruthy();
  expect(saveCall.request.trackId).toBe("local-1");
  expect(saveCall.request.cues).toHaveLength(1);
});

// --- Cue editor opened from a USB view (playlists / history) ---------------
//
// A USB row edits the on-device bundle directly: open goes through
// get_usb_track_detail, save through save_usb_track_analysis_edits with the
// row's raw on-device paths, and a not-connected USB blocks the flow.

function installUsbTrackDetailMock(page) {
  return page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    window.localStorage.setItem("djusbtkit.usbRoot", "/Volumes/USB-TEST");
    window.__calls = [];

    const pwv5 = new Uint8Array(4000);
    for (let i = 0; i < pwv5.length; i += 2) {
      const h = 8 + (i % 20);
      const v = (2 << 13) | (3 << 10) | (5 << 7) | (h << 2);
      pwv5[i] = (v >> 8) & 0xff;
      pwv5[i + 1] = v & 0xff;
    }
    const detailWaveformB64 = btoa(String.fromCharCode.apply(null, pwv5));

    const usbTrack = {
      id: "usbrow-1",
      localTrackId: "local-1",
      title: "USB Cue Track",
      artist: "USB Artist",
      album: "USB Album",
      bpm: 126,
      key: "Bm",
      durationMs: 200000,
      filePath: "/Volumes/USB-TEST/Contents/USB Artist/USB Album/track.mp3",
      usbMediaPath: "/Contents/USB Artist/USB Album/track.mp3",
      usbAnalysisPath: "/Volumes/USB-TEST/PIONEER/USBANLZ/P001/0000A1B2/ANLZ0000.DAT",
      usbAnalysisPathRaw: "/PIONEER/USBANLZ/P001/0000A1B2/ANLZ0000.DAT",
      waveformPreview: Array.from({ length: 80 }, (_, i) => (i % 7) * 12),
    };
    const tracksResponse = {
      ok: true,
      data: {
        items: [usbTrack], total: 1, nextCursor: null, hasMore: false,
        totalDurationMs: 200000, durationKnownCount: 1, warnings: [],
      },
    };

    // When set, save_usb_track_analysis_edits fails (USB yanked mid-edit).
    window.__usbSaveFails = false;

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          window.__calls.push({ command, request: payload?.request ?? null });
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "allow_asset_paths") return null;
          if (command === "get_backend_log_buffer") return [];
          if (command === "detect_external_master_db") return { ok: true, data: { found: false, path: null } };
          if (command === "check_source_roots") return { ok: true, data: { roots: [] } };
          if (command === "list_playlists") return { ok: true, data: { items: [] } };
          if (command === "list_usb_devices") return { ok: true, data: { items: [] } };
          if (command === "list_tracks" || command === "search_tracks") return { ok: true, data: { total: 0, items: [] } };
          if (command === "browse_source_files") return { ok: true, data: { total: 0, items: [], nextCursor: null, hasMore: false } };
          if (command === "pick_usb_folder") return "/Volumes/USB-TEST";
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
                items: [{ id: "usb-1", name: "Warmup", source: "mock", trackCount: 1, tracks: [{}] }],
                stats: { indexedTracks: 1, playlistReferencedTracks: 1, playlistEntries: 1 },
                warnings: [],
              },
            };
          }
          if (command === "fetch_usb_histories") {
            return {
              ok: true,
              data: {
                items: [{ id: "hist-1", name: "HISTORY 2024-01-01", source: "mock", trackCount: 1, tracks: [{}] }],
                warnings: [],
              },
            };
          }
          if (command === "fetch_usb_playlist_tracks" || command === "fetch_usb_history_tracks") return tracksResponse;
          if (command === "get_usb_track_detail") {
            return { ok: true, data: { firstBeatMs: 100, cues: [{ id: "c1", positionMs: 5000, colorId: 5, name: "Old" }], detailWaveform: detailWaveformB64 } };
          }
          if (command === "save_usb_track_analysis_edits") {
            if (window.__usbSaveFails) {
              return { ok: false, error: { code: "NOT_FOUND", message: "USB disconnected mid-edit" } };
            }
            return {
              ok: true,
              data: {
                firstBeatMs: payload?.request?.firstBeatMs ?? null,
                cues: payload?.request?.cues ?? [],
                bpm: payload?.request?.bpm ?? null,
                bpmAnalyzer: payload?.request?.bpm != null ? "user" : null,
                key: payload?.request?.key ?? null,
                keySource: payload?.request?.key != null ? "user" : null,
                anlzUpdated: true, edbUpdated: true, localUpdated: true,
              },
            };
          }
          if (command === "stop_playback_native" || command === "get_playback_status_native") return { ok: true, data: {} };
          return { ok: true, data: {} };
        },
      },
      event: { listen: async () => () => {} },
    };
  });
}

async function openUsbView(page, subView) {
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator(`.nav-item[data-view="${subView}"]`).click();
}

test("cue editor opens + saves from a USB playlist row through the USB commands", async ({ page }) => {
  await installUsbTrackDetailMock(page);
  await page.goto("/");

  await openUsbView(page, "usb-playlists");
  await page.locator("#refreshUsbBtn").click();
  await page.locator('[data-usb-playlist-index="0"]').click();

  const row = page.locator("#usbPlaylistTracks .track-grid-row");
  await expect(row).toHaveCount(1);
  await expect(row.locator('[data-action="analyze-track"]')).toHaveCount(0);
  await row.locator('[data-action="edit-track-detail"]').click();

  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  await expect(page.locator("#trackDetailCueList .cue-row")).toHaveCount(1);
  const detailCall = await page.evaluate(() => window.__calls.find((c) => c.command === "get_usb_track_detail"));
  expect(detailCall.request.usbAnalysisPathRaw).toBe("/PIONEER/USBANLZ/P001/0000A1B2/ANLZ0000.DAT");

  await page.locator("#trackDetailAddCue").click();
  await page.locator("#trackDetailSaveBtn").click();
  await expect(page.locator("#trackDetailOverlay")).toBeHidden();

  const saveCall = await page.evaluate(() => window.__calls.find((c) => c.command === "save_usb_track_analysis_edits"));
  expect(saveCall.request.usbAnalysisPathRaw).toBe("/PIONEER/USBANLZ/P001/0000A1B2/ANLZ0000.DAT");
  expect(saveCall.request.usbMediaPathRaw).toBe("/Contents/USB Artist/USB Album/track.mp3");
  expect(saveCall.request.localTrackId).toBe("local-1");
  expect(saveCall.request.cues).toHaveLength(2);
  // A cue-only USB edit still carries the current (unedited) bpm -- the
  // on-device beat-grid rewrite always needs a real value, not null.
  expect(saveCall.request.bpm).toBe(126);
  await expect(page.locator("#statusText")).toContainText("to USB");
});

test("cue editor edits BPM from a USB playlist row and saves it through the USB commands", async ({ page }) => {
  await installUsbTrackDetailMock(page);
  await page.goto("/");

  await openUsbView(page, "usb-playlists");
  await page.locator("#refreshUsbBtn").click();
  await page.locator('[data-usb-playlist-index="0"]').click();

  const row = page.locator("#usbPlaylistTracks .track-grid-row");
  await row.locator('[data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  await expect(page.locator("#trackDetailBpm")).toHaveValue("126");

  await page.locator("#trackDetailBpm").fill("128.5");
  await page.locator("#trackDetailBpm").dispatchEvent("change");
  await page.locator("#trackDetailSaveBtn").click();
  await expect(page.locator("#trackDetailOverlay")).toBeHidden();

  const saveCall = await page.evaluate(() => window.__calls.find((c) => c.command === "save_usb_track_analysis_edits"));
  expect(saveCall.request.bpm).toBe(128.5);
});

test("cue editor edits the musical key from a USB playlist row and saves it through the USB commands", async ({ page }) => {
  await installUsbTrackDetailMock(page);
  await page.goto("/");

  await openUsbView(page, "usb-playlists");
  await page.locator("#refreshUsbBtn").click();
  await page.locator('[data-usb-playlist-index="0"]').click();

  const row = page.locator("#usbPlaylistTracks .track-grid-row");
  await row.locator('[data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  await expect(page.locator("#trackDetailKey")).toHaveValue("Bm");

  await page.locator("#trackDetailKey").selectOption("Dm");
  await page.locator("#trackDetailSaveBtn").click();
  await expect(page.locator("#trackDetailOverlay")).toBeHidden();

  const saveCall = await page.evaluate(() => window.__calls.find((c) => c.command === "save_usb_track_analysis_edits"));
  expect(saveCall.request.key).toBe("Dm");
});

test("cue editor also opens from a USB history row", async ({ page }) => {
  await installUsbTrackDetailMock(page);
  await page.goto("/");

  await openUsbView(page, "usb-history");
  await page.locator("#refreshHistoryBtn").click();
  await page.locator('[data-history-index="0"]').click();

  const row = page.locator("#historyTracks .track-grid-row");
  await expect(row).toHaveCount(1);
  await expect(row.locator('[data-action="analyze-track"]')).toHaveCount(0);
  await row.locator('[data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  const detailCall = await page.evaluate(() => window.__calls.find((c) => c.command === "get_usb_track_detail"));
  expect(detailCall.request.usbAnalysisPathRaw).toBe("/PIONEER/USBANLZ/P001/0000A1B2/ANLZ0000.DAT");
});

test("a USB-side save that fails on the device is surfaced, not silently dropped", async ({ page }) => {
  await installUsbTrackDetailMock(page);
  await page.goto("/");

  await openUsbView(page, "usb-playlists");
  await page.locator("#refreshUsbBtn").click();
  await page.locator('[data-usb-playlist-index="0"]').click();
  await page.locator('#usbPlaylistTracks .track-grid-row [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();

  // USB yanked between open and save: the command errors.
  await page.evaluate(() => { window.__usbSaveFails = true; });
  await page.locator("#trackDetailAddCue").click();
  await page.locator("#trackDetailSaveBtn").click();

  await expect(page.locator("#statusText")).toContainText("USB disconnected mid-edit");
});

test("cue button is disabled for an un-analyzed track", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    const tracks = [{ id: "t1", title: "Raw", artist: "A", filePath: "/music/raw.mp3", analysisReady: false }];
    window.__TAURI__ = {
      core: {
        invoke: async (command) => {
          if (command === "detect_external_master_db") return { ok: true, data: { found: false, path: null } };
          if (command === "list_playlists") return { ok: true, data: { items: [] } };
          if (command === "get_backend_log_buffer") return [];
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") return { ok: true, data: { items: [], warnings: [] } };
          if (command === "list_tracks" || command === "search_tracks" || command === "browse_source_files") {
            return { ok: true, data: { total: tracks.length, items: tracks, nextCursor: null, hasMore: false } };
          }
          return { ok: true, data: {} };
        },
      },
      event: { listen: async () => () => {} },
    };
  });
  await page.goto("/");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);
  await expect(
    page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]')
  ).toBeDisabled();
});
