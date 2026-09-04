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

  await page.locator("#trackDetailZoomFit").click();
  await expect(visibleMarkers).toHaveCount(2);

  // Scroll-wheel zoom toward the left edge → the 170 s cue leaves the view again.
  await page.locator("#trackDetailWaveform").hover({ position: { x: 15, y: 100 } });
  await page.mouse.wheel(0, -500);
  await expect(visibleMarkers).toHaveCount(1);
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
