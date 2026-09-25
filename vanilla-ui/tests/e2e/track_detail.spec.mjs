import { test, expect } from "./coverage-fixture.mjs";

function installTrackDetailMock(page, opts = {}) {
  return page.addInitScript((opts) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    if (opts.startOnFirstBeat) window.localStorage.setItem("djusbtkit.cueStartOnFirstBeat", "1");
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
          if (command === "set_frontend_setting") return { ok: true, data: null };
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
                cues: [
                  ...(opts.seedStart != null ? [{ positionMs: opts.seedStart, playbackStart: true }] : []),
                  ...(opts.seedCues || []).map((positionMs) => ({ positionMs, colorId: 5, name: "" })),
                ],
                detailWaveform: detailWaveformB64,
              },
            };
          }
          // Tiny stand-in for the backend player clock, so pause/resume
          // report a real (advancing, then frozen) position like player.rs does.
          const clock = (window.__playerClock ||= { offsetMs: 0, startedAt: null, loaded: false });
          const positionMs = () =>
            Math.min(180000, clock.offsetMs + (clock.startedAt == null ? 0 : Date.now() - clock.startedAt));
          const status = () => ({
            path: "/music/one.mp3",
            playing: clock.loaded && clock.startedAt != null,
            paused: clock.loaded && clock.startedAt == null,
            positionMs: positionMs(),
            durationMs: 180000,
          });
          if (command === "play_resolved_track") {
            clock.offsetMs = Math.round((payload?.request?.startRatio || 0) * 180000);
            clock.startedAt = Date.now();
            clock.loaded = true;
            return { ok: true, data: { started: true, positionMs: clock.offsetMs, durationMs: 180000 } };
          }
          if (command === "pause_playback_native") {
            if (clock.loaded && clock.startedAt != null) {
              clock.offsetMs = positionMs();
              clock.startedAt = null;
            }
            return { ok: true, data: status() };
          }
          if (command === "resume_playback_native") {
            if (clock.loaded && clock.startedAt == null) clock.startedAt = Date.now();
            return { ok: true, data: status() };
          }
          if (command === "stop_playback_native") {
            clock.loaded = false;
            clock.startedAt = null;
            return { ok: true, data: { stopped: true, previousPath: null } };
          }
          if (command === "get_playback_status_native") {
            return { ok: true, data: status() };
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
  expect(Object.keys(saveCall.request.cues[0]).sort()).toEqual([
    "colorId",
    "name",
    "playbackStart",
    "positionMs",
  ]);
  expect(saveCall.request.cues[0].playbackStart).toBe(false);
});

async function openCueEditor(page, opts) {
  await installTrackDetailMock(page, opts);
  await page.goto("/");
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
}

const startPersistCalls = (page) =>
  page.evaluate(() =>
    window.__calls
      .filter((c) => c.command === "set_frontend_setting")
      .map((c) => c.request ?? c)
      .filter((r) => JSON.stringify(r).includes("ui_cue_start_on_first_beat_v1"))
  );

/// Right edge of the greyed pre-start shade, in px from the waveform's left edge.
const preStartEdge = (page) =>
  page.evaluate(() => {
    const wf = document.querySelector("#trackDetailWaveform").getBoundingClientRect();
    const shade = document.querySelector("#trackDetailPreStart");
    return shade.hidden ? null : shade.getBoundingClientRect().right - wf.left;
  });

test("playback-start choice: informational with no cues, then applies the remembered (default First cue) setting", async ({ page }) => {
  await openCueEditor(page);
  const firstCue = page.locator("#trackDetailStartFirstCue");
  const firstBeat = page.locator("#trackDetailStartFirstBeat");
  const note = page.locator("#trackDetailStartNote");
  // No cues: neither choice applies (the CDJ starts at the first audio), the note says so.
  await expect(firstCue).toBeDisabled();
  await expect(firstBeat).toBeDisabled();
  await expect(firstCue).toHaveAttribute("aria-checked", "false");
  await expect(firstBeat).toHaveAttribute("aria-checked", "false");
  await expect(note).toBeVisible();
  // No cues: the CDJ starts at the first audio, nothing is greyed out.
  await expect(page.locator("#trackDetailPreStart")).toBeHidden();

  await page.locator("#trackDetailWaveform").dblclick({ position: { x: 300, y: 100 } });
  await expect(firstBeat).toBeEnabled();
  await expect(note).toBeHidden();
  await expect(firstCue).toHaveAttribute("aria-checked", "true");
  // Starting from the first cue point: greyed up to cue A (snapped to the grid).
  await expect.poll(() => preStartEdge(page)).toBeGreaterThan(290);
  expect(await preStartEdge(page)).toBeLessThan(310);
  await expect(firstBeat).toHaveAttribute("aria-checked", "false");
  await expect(page.locator("#trackDetailCueList .cue-row")).toHaveCount(1);
  await expect(page.locator("#trackDetailCueList .cue-row.is-playback-start")).toHaveCount(0);

  // Choosing First beat adds the memory-only start cue there (120 ms), listed first.
  await firstBeat.click();
  await expect(firstBeat).toHaveAttribute("aria-checked", "true");
  await expect(firstCue).toHaveAttribute("aria-checked", "false");
  await expect(firstBeat).toHaveText("First beat");
  const rows = page.locator("#trackDetailCueList .cue-row");
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toHaveClass(/is-playback-start/);
  await expect(rows.first().locator(".cue-row-pos")).toHaveText("0:00.12");
  await expect(rows.first().locator(".cue-row-delete")).toHaveCount(0);
  await expect(rows.first().locator(".cue-row-color")).toHaveCount(0);
  await expect(rows.first().locator(".cue-row-name")).toHaveCount(0);
  await expect(rows.first().locator(".cue-row-label")).toHaveText("Playback start");
  await expect(page.locator("#trackDetailCueMarkers .cue-marker.is-playback-start")).toHaveCount(1);
  // …and the greyed-out part shrinks to the first beat.
  await expect.poll(() => preStartEdge(page)).toBeLessThan(10);
  // Hot cues keep their A.. lettering, the same in the list as on the waveform.
  await expect(page.locator("#trackDetailCueMarkers .cue-marker:not(.is-playback-start)")).toHaveText("A");
  await expect(rows.nth(1).locator(".cue-row-color")).toHaveText("A");
  await expect(rows.first().locator(".cue-row-memory")).toHaveText("▶");

  // …and remembers the choice.
  expect(await page.evaluate(() => localStorage.getItem("djusbtkit.cueStartOnFirstBeat"))).toBe("1");
  await expect.poll(async () => (await startPersistCalls(page)).length).toBe(1);

  await page.locator("#trackDetailSaveBtn").click();
  const saveCall = await page.evaluate(() =>
    window.__calls.find((c) => c.command === "save_track_analysis_edits")
  );
  expect(saveCall.request.cues).toHaveLength(2);
  expect(saveCall.request.cues[0]).toEqual({
    positionMs: 120,
    colorId: null,
    name: null,
    playbackStart: true,
  });
  expect(saveCall.request.cues[1].playbackStart).toBe(false);
});

test("an outside click or Escape closes the cue editor only when nothing is unsaved", async ({ page }) => {
  await openCueEditor(page, { seedCues: [30000] });
  const overlay = page.locator("#trackDetailOverlay");
  const outside = { position: { x: 5, y: 5 } };

  // View-only changes (zoom, beat-grid slider) aren't edits.
  await page.locator("#trackDetailZoomFit").click();
  await page.locator("#trackDetailGridLevel").fill("80");
  await overlay.click(outside);
  await expect(overlay).toBeHidden();

  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(overlay).toBeVisible();
  await page.locator("#trackDetailAddCue").click();
  await overlay.click(outside);
  await expect(overlay).toBeVisible();
  await expect(page.locator("#trackDetailSaveBtn")).toHaveClass(/is-attention/);
  await page.keyboard.press("Escape");
  await expect(overlay).toBeVisible();

  // Undoing the edit makes it clean again: Escape closes…
  await page
    .locator("#trackDetailCueList .cue-row")
    .filter({ hasNot: page.locator(".cue-row-pos", { hasText: "0:30.00" }) })
    .locator(".cue-row-delete")
    .click();
  await page.keyboard.press("Escape");
  await expect(overlay).toBeHidden();
  // …and so does an outside click.
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(overlay).toBeVisible();
  await overlay.click(outside);
  await expect(overlay).toBeHidden();
  const saves = await page.evaluate(() =>
    window.__calls.filter((c) => c.command === "save_track_analysis_edits")
  );
  expect(saves).toHaveLength(0);
});

test("cue list rows carry the same letter as their waveform marker, in position order", async ({ page }) => {
  await openCueEditor(page, { seedCues: [60000, 30000] });
  const rows = page.locator("#trackDetailCueList .cue-row");
  await expect(rows.locator(".cue-row-pos")).toHaveText(["0:30.00", "1:00.00"]);
  await expect(rows.locator(".cue-row-color")).toHaveText(["A", "B"]);
  await expect(page.locator("#trackDetailCueMarkers .cue-marker")).toHaveText(["A", "B"]);
});

test("the Beat grid slider sets how strongly the grid shows, and is remembered", async ({ page }) => {
  await openCueEditor(page);
  const slider = page.locator("#trackDetailGridLevel");
  const gridOpacity = () =>
    page
      .locator("#trackDetailBeatgrid .beatgrid-line:not(.is-downbeat)")
      .first()
      .evaluate((n) => Number(getComputedStyle(n).opacity));
  const downbeatOpacity = () =>
    page
      .locator("#trackDetailBeatgrid .beatgrid-line.is-downbeat")
      .first()
      .evaluate((n) => Number(getComputedStyle(n).opacity));
  await expect(slider).toHaveValue("35");
  const defaultOpacity = await gridOpacity();

  // The thumb follows the pointer: a click lands on the matching value
  // (no inherited text-input padding skewing the track).
  const box = await slider.boundingBox();
  await slider.click({ position: { x: box.width * 0.75, y: box.height / 2 } });
  expect(Math.abs(Number(await slider.inputValue()) - 75)).toBeLessThanOrEqual(6);
  await slider.click({ position: { x: box.width * 0.25, y: box.height / 2 } });
  expect(Math.abs(Number(await slider.inputValue()) - 25)).toBeLessThanOrEqual(6);

  await slider.fill("100");
  await expect.poll(gridOpacity).toBe(1);
  expect(await page.evaluate(() => localStorage.getItem("djusbtkit.cueBeatgridLevel"))).toBe("100");

  await slider.fill("0");
  const faint = await gridOpacity();
  expect(faint).toBeGreaterThan(0);
  expect(faint).toBeLessThan(defaultOpacity);
  // Bar starts stay visible even with the slider at 0.
  expect(await downbeatOpacity()).toBeGreaterThanOrEqual(0.6);
  expect(await page.evaluate(() => localStorage.getItem("djusbtkit.cueBeatgridLevel"))).toBe("0");

  // The grid overflows the waveform into a strip above and below it.
  const canvasBox = await page.locator("#trackDetailWaveform .waveform-canvas-el").boundingBox();
  const lineBox = await page.locator("#trackDetailBeatgrid .beatgrid-line").first().boundingBox();
  expect(lineBox.y).toBeLessThan(canvasBox.y);
  expect(lineBox.y + lineBox.height).toBeGreaterThan(canvasBox.y + canvasBox.height);
});

test("remembered start-on-first-beat: the first cue adds the start cue; deleting the last cue removes it", async ({ page }) => {
  await openCueEditor(page, { startOnFirstBeat: true });
  const firstBeat = page.locator("#trackDetailStartFirstBeat");

  await page.locator("#trackDetailWaveform").dblclick({ position: { x: 300, y: 100 } });
  const rows = page.locator("#trackDetailCueList .cue-row");
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toHaveClass(/is-playback-start/);
  await expect(firstBeat).toHaveAttribute("aria-checked", "true");
  await expect(firstBeat).toBeEnabled();

  // A second cue doesn't add another start cue, nor count it toward the 8.
  await page.locator("#trackDetailAddCue").click();
  await expect(rows).toHaveCount(3);
  await expect(page.locator("#trackDetailCueList .cue-row.is-playback-start")).toHaveCount(1);

  for (let i = 0; i < 2; i += 1) {
    await page.locator("#trackDetailCueList .cue-row:not(.is-playback-start) .cue-row-delete").first().click();
  }
  await expect(page.locator("#trackDetailCueList .cue-row")).toHaveCount(0);
  await expect(page.locator("#trackDetailCueMarkers .cue-marker")).toHaveCount(0);
  await expect(firstBeat).toBeDisabled();
  await expect(page.locator("#trackDetailStartNote")).toBeVisible();
  // Losing the cues is not a user choice: the remembered setting stays on.
  expect(await page.evaluate(() => localStorage.getItem("djusbtkit.cueStartOnFirstBeat"))).toBe("1");
});

test("an existing track's start cue drives the choice, not the remembered setting; choosing First cue remembers it", async ({ page }) => {
  await openCueEditor(page, { startOnFirstBeat: true, seedCues: [30000] });
  const firstCue = page.locator("#trackDetailStartFirstCue");
  const firstBeat = page.locator("#trackDetailStartFirstBeat");
  await expect(firstCue).toHaveAttribute("aria-checked", "true");
  await expect(page.locator("#trackDetailCueList .cue-row.is-playback-start")).toHaveCount(0);

  await firstBeat.click();
  await expect(page.locator("#trackDetailCueList .cue-row.is-playback-start")).toHaveCount(1);
  await firstCue.click();
  await expect(page.locator("#trackDetailCueList .cue-row.is-playback-start")).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem("djusbtkit.cueStartOnFirstBeat"))).toBe("0");
});

test("the playback-start cue follows an untouched first beat and is never after a hot cue", async ({ page }) => {
  await openCueEditor(page, { seedCues: [30000], seedStart: 120 });
  const firstBeat = page.locator("#trackDetailStartFirstBeat");
  await expect(firstBeat).toHaveAttribute("aria-checked", "true");

  await expect(firstBeat).toHaveText("First beat");
  const startPos = page.locator("#trackDetailCueList .cue-row.is-playback-start .cue-row-pos");
  const hotPos = page.locator("#trackDetailCueList .cue-row:not(.is-playback-start) .cue-row-pos");
  await expect(startPos).toHaveText("0:00.12");

  // Untouched: follows the first beat (+1 beat at 128 BPM = 468.75 ms).
  await page.locator("#trackDetailFirstBeatPlus").click();
  await expect(page.locator("#trackDetailFirstBeatMs")).toHaveValue("589");
  await expect(startPos).toHaveText("0:00.58");
  await expect(firstBeat).toHaveText("First beat");

  const wfBox = await page.locator("#trackDetailWaveform").boundingBox();
  const drag = async (marker, ratio) => {
    const box = await marker.boundingBox();
    const y = box.y + box.height / 2;
    await page.mouse.move(box.x + 2, y);
    const targetX = wfBox.x + wfBox.width * ratio;
    await page.mouse.down();
    // Monotonic toward the target: an overshoot would legitimately push the start cue.
    await page.mouse.move((box.x + 2 + targetX) / 2, y, { steps: 5 });
    await page.mouse.move(targetX, y, { steps: 5 });
    await page.mouse.up();
  };

  // Dragging it past the 30 s hot cue stops it on the hot cue.
  await drag(page.locator("#trackDetailCueMarkers .cue-marker.is-playback-start"), 0.5);
  await expect(startPos).toHaveText("0:30.00");
  // Off the first beat, the choice names the placed marker instead.
  await expect(firstBeat).toHaveText("Start marker");
  await expect(firstBeat).toHaveAttribute("aria-checked", "true");
  // The grey-out follows the dragged start cue.
  const startBox = await page.locator("#trackDetailCueMarkers .cue-marker.is-playback-start").boundingBox();
  expect(Math.abs((await preStartEdge(page)) - (startBox.x + 1 - wfBox.x))).toBeLessThan(3);

  // Dragging the hot cue before it pushes it back too.
  await drag(page.locator("#trackDetailCueMarkers .cue-marker:not(.is-playback-start)"), 0.1);
  const hotText = await hotPos.textContent();
  expect(hotText).toMatch(/^0:1[12]\./);
  await expect(startPos).toHaveText(hotText);

  // Once dragged it no longer follows the first beat.
  await page.locator("#trackDetailFirstBeatMinus").click();
  await expect(startPos).toHaveText(hotText);

  await page.locator("#trackDetailSaveBtn").click();
  const saveCall = await page.evaluate(() =>
    window.__calls.find((c) => c.command === "save_track_analysis_edits")
  );
  const [start, hot] = saveCall.request.cues;
  expect(start.playbackStart).toBe(true);
  expect(hot.playbackStart).toBe(false);
  expect(start.positionMs).toBe(hot.positionMs);
});

test("bar numbers label the grid without crowding, down to every bar when zoomed in", async ({ page }) => {
  await openCueEditor(page);
  const labels = page.locator("#trackDetailBeatgrid .beatgrid-bar");
  const barState = () =>
    labels.evaluateAll((nodes) => ({
      numbers: nodes.map((n) => Number(n.textContent)),
      xs: nodes.map((n) => n.getBoundingClientRect().left),
    }));

  // 2 min at 128 BPM is 64 bars: labelled every Nth bar, never closer than 36 px.
  let { numbers, xs } = await barState();
  expect(numbers[0]).toBe(1);
  const step = numbers[1] - numbers[0];
  expect(step).toBeGreaterThan(1);
  for (let i = 1; i < numbers.length; i += 1) {
    expect(numbers[i] - numbers[i - 1]).toBe(step);
    expect(xs[i] - xs[i - 1]).toBeGreaterThanOrEqual(35);
  }

  for (let i = 0; i < 4; i += 1) await page.locator("#trackDetailZoomIn").click();
  await expect.poll(async () => {
    ({ numbers } = await barState());
    return numbers.length > 1 ? numbers[1] - numbers[0] : 0;
  }).toBe(1);
});

test("the overview strip shows the visible window and moves the view", async ({ page }) => {
  await openCueEditor(page, { seedCues: [30000, 170000] });
  const overview = page.locator("#trackDetailOverview");
  const windowBox = page.locator("#trackDetailOverviewWindow");
  await expect(page.locator("#trackDetailOverviewCues .overview-cue")).toHaveCount(2);

  // The modal opens on 0–2:00 of a 3:00 track: the box covers the first 2/3.
  const ov = await overview.boundingBox();
  let box = await windowBox.boundingBox();
  expect(Math.abs(box.x - ov.x)).toBeLessThan(2);
  expect(Math.abs(box.width - (ov.width * 2) / 3)).toBeLessThan(3);

  // Zoom in, then click near the end: the view (same zoom) centres there.
  for (let i = 0; i < 3; i += 1) await page.locator("#trackDetailZoomIn").click();
  const zoomedWidth = (ov.width * 15) / 180; // 120 s / 2^3 of 180 s
  await expect.poll(async () => (await windowBox.boundingBox()).width).toBeCloseTo(zoomedWidth, 0);
  // 94% of 3:00 is 2:49, so the 15 s view (2:42–2:57) takes in the 2:50 cue.
  await overview.click({ position: { x: ov.width * 0.94, y: ov.height / 2 } });
  await expect.poll(async () => {
    const b = await windowBox.boundingBox();
    return b.x + b.width / 2 - ov.x;
  }).toBeGreaterThan(ov.width * 0.92);
  box = await windowBox.boundingBox();
  expect(Math.abs(box.width - zoomedWidth)).toBeLessThan(2);
  await expect(page.locator("#trackDetailZoomRange")).toHaveText(/^2:\d\d–2:\d\d$/);
  // The view followed: the 170 s cue is now on the main waveform.
  await expect(page.locator("#trackDetailCueMarkers .cue-marker:not(.off-view)")).toHaveText("B");

  // Dragging moves it back.
  await page.mouse.move(ov.x + ov.width * 0.94, ov.y + ov.height / 2);
  await page.mouse.down();
  await page.mouse.move(ov.x + ov.width * 0.2, ov.y + ov.height / 2, { steps: 6 });
  await page.mouse.up();
  await expect.poll(async () => {
    const b = await windowBox.boundingBox();
    return b.x + b.width / 2 - ov.x;
  }).toBeLessThan(ov.width * 0.22);

  await page.locator("#trackDetailZoomFit").click();
  await expect.poll(async () => (await windowBox.boundingBox()).width).toBeGreaterThan(ov.width - 2);
});

test("dragging on the waveform, a cue marker or the overview never selects text", async ({ page }) => {
  await openCueEditor(page, { seedCues: [30000] });
  const selected = () => page.evaluate(() => String(window.getSelection()));
  // Drag from inside `from` across the title, the time footer and the cue list.
  const dragAcrossText = async (x, y) => {
    const title = await page.locator("#trackDetailTitle").boundingBox();
    const list = await page.locator("#trackDetailCueList").boundingBox();
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(list.x + 40, list.y + list.height / 2, { steps: 8 });
    await page.mouse.move(title.x + 10, title.y + title.height / 2, { steps: 8 });
    await page.mouse.move(list.x + list.width - 40, list.y + list.height - 4, { steps: 8 });
    await page.mouse.up();
  };

  const wf = await page.locator("#trackDetailWaveform").boundingBox();
  await dragAcrossText(wf.x + wf.width * 0.6, wf.y + wf.height / 2);
  expect(await selected()).toBe("");

  const marker = await page.locator("#trackDetailCueMarkers .cue-marker").first().boundingBox();
  await dragAcrossText(marker.x + 2, marker.y + marker.height / 2);
  expect(await selected()).toBe("");

  const ov = await page.locator("#trackDetailOverview").boundingBox();
  await dragAcrossText(ov.x + ov.width * 0.3, ov.y + ov.height / 2);
  expect(await selected()).toBe("");

  // Engine-independent guard (the Tauri build renders with WebKitGTK): while
  // held, the page is marked unselectable and `selectstart` is cancelled; a
  // release anywhere, even outside the dialog, lifts it.
  const guard = () =>
    page.evaluate(() => {
      const ev = new Event("selectstart", { bubbles: true, cancelable: true });
      document.getElementById("trackDetailTitle").dispatchEvent(ev);
      return {
        marked: document.documentElement.classList.contains("is-ui-dragging"),
        blocked: ev.defaultPrevented,
      };
    });
  await page.mouse.move(wf.x + wf.width * 0.5, wf.y + wf.height / 2);
  await page.mouse.down();
  await page.mouse.move(wf.x + wf.width * 0.5, 2, { steps: 4 });
  expect(await guard()).toEqual({ marked: true, blocked: true });
  await page.mouse.up();
  expect(await guard()).toEqual({ marked: false, blocked: false });
});

test("usage hints show until the track has a cue, then fold into a ? tooltip", async ({ page }) => {
  await openCueEditor(page);
  const hint = page.locator("#trackDetailHint");
  const hintBtn = page.locator("#trackDetailHintBtn");
  await expect(hint).toBeVisible();
  await expect(hintBtn).toBeHidden();

  await page.locator("#trackDetailWaveform").dblclick({ position: { x: 300, y: 100 } });
  await expect(hint).toBeHidden();
  await expect(hintBtn).toBeVisible();
  await expect(hintBtn).toHaveAttribute("data-tooltip", /double-click to add a cue/);

  await page.locator("#trackDetailCueList .cue-row-delete").click();
  await expect(hint).toBeVisible();
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

test("dragging a cue marker moves the cue without starting playback; Shift snaps to the beat grid", async ({ page }) => {
  await installTrackDetailMock(page, { seedCues: [30000] });
  await page.goto("/");
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();

  const wfBox = await page.locator("#trackDetailWaveform").boundingBox();
  const marker = page.locator("#trackDetailCueMarkers .cue-marker").first();
  const markerBox = await marker.boundingBox();
  // Default view is 0–120 s; drag the 30 s marker to the view midpoint (≈60 s).
  const y = markerBox.y + markerBox.height / 2;
  await page.mouse.move(markerBox.x + 2, y);
  await page.mouse.down();
  await page.mouse.move(wfBox.x + wfBox.width * 0.4, y, { steps: 5 });
  await page.mouse.move(wfBox.x + wfBox.width * 0.5, y, { steps: 5 });
  await page.mouse.up();

  const pos = page.locator("#trackDetailCueList .cue-row-pos").first();
  await expect(pos).toHaveText(/^(0:59|1:00)\./);
  expect(
    await page.evaluate(() => window.__calls.some((c) => c.command === "play_resolved_track"))
  ).toBe(false);

  // Shift-drag lands exactly on a beat: firstBeatMs 120, 128 BPM ⇒ 468.75 ms/beat.
  const box2 = await marker.boundingBox();
  await page.keyboard.down("Shift");
  await page.mouse.move(box2.x + 2, y);
  await page.mouse.down();
  await page.mouse.move(wfBox.x + wfBox.width * 0.3, y, { steps: 5 });
  await page.mouse.move(wfBox.x + wfBox.width * 0.33, y, { steps: 5 });
  await page.mouse.up();
  await page.keyboard.up("Shift");

  await page.locator("#trackDetailSaveBtn").click();
  const saveCall = await page.evaluate(() =>
    window.__calls.find((c) => c.command === "save_track_analysis_edits")
  );
  expect(saveCall.request.cues).toHaveLength(1);
  const snapped = saveCall.request.cues[0].positionMs;
  expect(snapped).toBeGreaterThan(35000);
  expect(snapped).toBeLessThan(45000);
  const beats = (snapped - 120) / (60000 / 128);
  expect(Math.abs(beats - Math.round(beats)) * (60000 / 128)).toBeLessThanOrEqual(1);
});

test("a marker tooltip never jumps to the corner when the markers re-render under the pointer", async ({ page }) => {
  await installTrackDetailMock(page, { seedCues: [30000] });
  await page.goto("/");
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();

  // Hover a marker, then re-render the markers (a BPM edit) inside the
  // tooltip's show delay, so the hovered element is replaced mid-delay. The
  // hover is a synthetic `mouseover` so Chromium can't re-dispatch one onto
  // the replacement marker -- the desktop app's WebKitGTK webview doesn't,
  // and there the stale timer used to anchor on the detached marker (0,0).
  const tip = page.locator("#app-tooltip");
  await page.evaluate(() => {
    const marker = document.querySelector("#trackDetailCueMarkers .cue-marker");
    marker.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    const bpm = document.getElementById("trackDetailBpm");
    bpm.value = "129";
    bpm.dispatchEvent(new Event("change"));
  });
  await page.waitForTimeout(400);
  await expect(tip).not.toHaveClass(/app-tooltip--visible/);

  // A tooltip whose marker is re-rendered while it is showing is dropped
  // (or re-anchored on the replacement marker) -- never left at the corner.
  const marker = page.locator("#trackDetailCueMarkers .cue-marker").first();
  const nearMarker = async () => {
    const box = await marker.boundingBox();
    const tipBox = await tip.boundingBox();
    expect(Math.abs(tipBox.x + tipBox.width / 2 - (box.x + box.width / 2))).toBeLessThan(40);
    expect(tipBox.y).toBeGreaterThan(box.y - 60);
  };
  await marker.hover();
  await expect(tip).toHaveClass(/app-tooltip--visible/);
  await nearMarker();
  await page.evaluate(() => {
    const bpm = document.getElementById("trackDetailBpm");
    bpm.value = "130";
    bpm.dispatchEvent(new Event("change"));
  });
  await page.waitForTimeout(400);
  if (/app-tooltip--visible/.test((await tip.getAttribute("class")) || "")) await nearMarker();
});

// Whole-track playhead position (ms) the shared playback module projects onto
// the modal waveform.
const modalPlayheadMs = (page) =>
  page.evaluate(() => {
    const wf = document.getElementById("trackDetailWaveform");
    return (parseFloat(wf.style.getPropertyValue("--playhead-position")) || 0) / 100 * 180000;
  });

test("play/pause pauses in the backend, freezes the playhead, and + Cue lands on the paused spot", async ({ page }) => {
  await installTrackDetailMock(page);
  await page.goto("/");
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();

  const btn = page.locator("#trackDetailPlayPause");
  const calls = (command) =>
    page.evaluate((command) => window.__calls.filter((c) => c.command === command), command);

  // Nothing loaded yet: Play starts at the left edge of the visible window (0 s).
  await expect(btn).toHaveAttribute("aria-label", "Play");
  await btn.click();
  await expect.poll(async () => (await calls("play_resolved_track")).length).toBe(1);
  expect((await calls("play_resolved_track"))[0].request.startRatio).toBe(0);
  await expect(btn).toHaveAttribute("aria-label", "Pause");
  await expect.poll(() => modalPlayheadMs(page)).toBeGreaterThan(200);

  // Pause goes to the backend -- not a stop + replay.
  await btn.click();
  await expect(btn).toHaveAttribute("aria-label", "Play");
  expect(await calls("pause_playback_native")).toHaveLength(1);
  expect(await calls("stop_playback_native")).toHaveLength(0);
  expect(await calls("play_resolved_track")).toHaveLength(1);
  await expect(page.locator("#trackDetailWaveform")).toHaveClass(/is-paused/);
  await expect(page.locator("#trackDetailPlayhead")).toBeVisible();

  // The playhead holds at the backend's paused position.
  const pausedMs = await modalPlayheadMs(page);
  expect(pausedMs).toBeGreaterThan(200);
  await page.waitForTimeout(300); // prove it does NOT move while paused
  expect(await modalPlayheadMs(page)).toBeCloseTo(pausedMs, 0);

  // "+ Cue" while paused lands at the paused spot, not at 0.
  await page.locator("#trackDetailAddCue").click();
  await page.locator("#trackDetailSaveBtn").click();
  const saveCall = (await calls("save_track_analysis_edits"))[0];
  expect(Math.abs(saveCall.request.cues[0].positionMs - pausedMs)).toBeLessThan(50);
  // Leaving the editor still releases the paused track.
  await expect.poll(async () => (await calls("stop_playback_native")).length).toBe(1);
});

test("play/pause resumes in the backend from where it was paused", async ({ page }) => {
  await installTrackDetailMock(page);
  await page.goto("/");
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  const btn = page.locator("#trackDetailPlayPause");
  const calls = (command) =>
    page.evaluate((command) => window.__calls.filter((c) => c.command === command), command);

  await btn.click();
  await expect(btn).toHaveAttribute("aria-label", "Pause");
  await expect.poll(() => modalPlayheadMs(page)).toBeGreaterThan(200);
  await btn.click();
  await expect(btn).toHaveAttribute("aria-label", "Play");
  const pausedMs = await modalPlayheadMs(page);

  await btn.click();
  await expect(btn).toHaveAttribute("aria-label", "Pause");
  expect(await calls("resume_playback_native")).toHaveLength(1);
  expect(await calls("play_resolved_track")).toHaveLength(1);
  await expect(page.locator("#trackDetailWaveform")).not.toHaveClass(/is-paused/);
  // The playhead carries on from the paused spot.
  await expect.poll(() => modalPlayheadMs(page)).toBeGreaterThan(pausedMs);
  expect(await modalPlayheadMs(page)).toBeLessThan(pausedMs + 2000);
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
  // names the visible window, centred under the waveform; the 3-min total
  // sits at the footer's right end.
  const zoomRange = page.locator("#trackDetailZoomRange");
  const totalTime = page.locator("#trackDetailTotalTime");
  await expect(zoomRange).toBeVisible();
  await expect(zoomRange).toHaveClass(/is-zoomed/);
  await expect(zoomRange).toHaveText("0:00–2:00");
  await expect(totalTime).toHaveText("3:00");
  const wfBox = await page.locator("#trackDetailWaveform").boundingBox();
  const rangeBox = await zoomRange.boundingBox();
  const totalBox = await totalTime.boundingBox();
  expect(rangeBox.y).toBeGreaterThanOrEqual(wfBox.y + wfBox.height);
  expect(Math.abs(rangeBox.x + rangeBox.width / 2 - (wfBox.x + wfBox.width / 2))).toBeLessThan(4);
  expect(totalBox.y).toBeGreaterThanOrEqual(wfBox.y + wfBox.height);
  expect(wfBox.x + wfBox.width - (totalBox.x + totalBox.width)).toBeLessThan(16);

  await page.locator("#trackDetailZoomFit").click();
  await expect(visibleMarkers).toHaveCount(2);
  // Fully zoomed out, the readout switches to a plain "whole track" state.
  await expect(zoomRange).not.toHaveClass(/is-zoomed/);
  await expect(zoomRange).toHaveText("Whole track");
  await expect(totalTime).toHaveText("3:00");

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
