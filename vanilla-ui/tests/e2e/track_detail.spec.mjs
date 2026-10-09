import { test, expect } from "./coverage-fixture.mjs";

function installTrackDetailMock(page, opts = {}) {
  return page.addInitScript((opts) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    if (opts.startOnFirstBeat) window.localStorage.setItem("djusbtkit.cueStartOnFirstBeat", "1");
    // Quantize defaults on; tests that place cues at exact spots turn it off.
    if (opts.quantize === false) window.localStorage.setItem("djusbtkit.cueQuantize", "0");
    if (opts.followGrid) window.localStorage.setItem("djusbtkit.cueFollowGrid", "1");
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
        ...(opts.camelot ? { keyDisplay: "8A", keyColor: 7 } : {}),
        durationMs: 180000,
        analysisReady: true,
        waveformPreview: Array.from({ length: 80 }, (_, i) => (i % 7) * 12),
      },
    ];
    if (opts.secondTrack) {
      tracks.push({ ...tracks[0], id: "t2", title: "Other Track", filePath: "/music/two.mp3" });
    }
    // What the backend's key_notation renders for the keys these tests use.
    const CAMELOT = { "Am": "8A", "F#m": "11A", "C": "8B" };
    const keyLabel = (k) => (opts.camelot && CAMELOT[k]) || k;
    const withKeyLabel = (t) => ({ ...t, keyDisplay: keyLabel(t.key) });

    // Base64 of a small PWV5 payload (2 bytes/entry).
    const pwv5 = new Uint8Array(4000);
    for (let i = 0; i < pwv5.length; i += 2) {
      const h = 8 + (i % 20);
      const v = (2 << 13) | (3 << 10) | (5 << 7) | (h << 2);
      pwv5[i] = (v >> 8) & 0xff;
      pwv5[i + 1] = v & 0xff;
    }
    const detailWaveformB64 = btoa(String.fromCharCode.apply(null, pwv5));

    const MAJORS = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          window.__calls.push({ command, request: payload?.request ?? null });
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "set_frontend_setting") {
            const r = payload?.request || {};
            if (r.key === "ui_key_notation_v1") opts.camelot = r.value === "camelot";
            return { ok: true, data: null };
          }
          if (command === "detect_external_rekordbox_db") return { ok: true, data: { found: false, path: null } };
          if (command === "list_playlists") return { ok: true, data: { items: [] } };
          if (command === "get_backend_log_buffer") return [];
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          if (command === "list_tracks" || command === "search_tracks") {
            return { ok: true, data: { total: tracks.length, items: tracks.map(withKeyLabel) } };
          }
          if (command === "browse_source_files") {
            return { ok: true, data: { total: tracks.length, items: tracks.map(withKeyLabel), nextCursor: null, hasMore: false } };
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
                keyOptions: [MAJORS, MAJORS.map((k) => `${k}m`)].map((keys, i) => ({ label: i ? "Minor" : "Major", keys: keys.map((k) => ({ value: k, label: keyLabel(k) })) })),
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
          if (command === "set_playback_metronome") {
            const r = payload?.request || {};
            return { ok: true, data: { enabled: !!r.enabled && r.bpm > 0 } };
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
                ...(opts.camelot && payload?.request?.key === "F#m" ? { keyDisplay: "11A", keyColor: 10 } : {}),
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

  // All eight A–H slots are always there, so adding a cue doesn't change the height.
  const cells = page.locator("#trackDetailCueList > *");
  await expect(cells).toHaveCount(8);
  const listHeight = (await page.locator("#trackDetailCueList").boundingBox()).height;
  await page.locator("#trackDetailAddCue").click();
  await expect(page.locator("#trackDetailCueList .cue-row")).toHaveCount(1);
  await expect(cells).toHaveCount(8);
  await expect(page.locator("#trackDetailCueList .cue-slot-empty").first().locator(".cue-row-color")).toHaveText("B");
  expect((await page.locator("#trackDetailCueList").boundingBox()).height).toBe(listHeight);

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
  await expect(page.locator("#trackDetailStartCue .cue-row.is-playback-start")).toHaveCount(0);

  // Choosing First beat adds the memory-only start cue there (120 ms), shown
  // in the "Playback starts at" row rather than among the A–H slots.
  await firstBeat.click();
  await expect(firstBeat).toHaveAttribute("aria-checked", "true");
  await expect(firstCue).toHaveAttribute("aria-checked", "false");
  await expect(firstBeat).toHaveText("First beat");
  const rows = page.locator("#trackDetailCueList .cue-row");
  await expect(rows).toHaveCount(1);
  const start = page.locator("#trackDetailStartCue .cue-row.is-playback-start");
  await expect(start).toHaveCount(1);
  await expect(start.locator(".cue-row-pos")).toHaveText("0:00.12");
  await expect(start.locator(".cue-row-delete")).toHaveCount(0);
  await expect(start.locator(".cue-row-color")).toHaveCount(0);
  await expect(start.locator(".cue-row-name")).toHaveCount(0);
  await expect(start.locator(".cue-row-label")).toHaveText("Playback start");
  await expect(page.locator("#trackDetailCueMarkers .cue-marker.is-playback-start")).toHaveCount(1);
  // …and the greyed-out part shrinks to the first beat.
  await expect.poll(() => preStartEdge(page)).toBeLessThan(10);
  // Hot cues keep their A.. lettering, the same in the list as on the waveform.
  await expect(page.locator("#trackDetailCueMarkers .cue-marker:not(.is-playback-start)")).toHaveText("A");
  await expect(rows.first().locator(".cue-row-color")).toHaveText("A");
  await expect(start.locator(".cue-row-memory")).toHaveText("▶");

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
  // The opening 60-bar view only draws bar lines; one zoom step spreads the
  // beats far enough apart to draw them too.
  await page.locator("#trackDetailZoomIn").click();
  const lineWidth = (selector) =>
    page
      .locator(`#trackDetailBeatgrid .beatgrid-line${selector}`)
      .first()
      .evaluate((n) => n.getBoundingClientRect().width);
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
  const defaultBeatWidth = await lineWidth(":not(.is-downbeat)");
  const defaultBarWidth = await lineWidth(".is-downbeat");

  // The thumb follows the pointer: a click lands on the matching value
  // (no inherited text-input padding skewing the track).
  const box = await slider.boundingBox();
  await slider.click({ position: { x: box.width * 0.75, y: box.height / 2 } });
  expect(Math.abs(Number(await slider.inputValue()) - 75)).toBeLessThanOrEqual(6);
  await slider.click({ position: { x: box.width * 0.25, y: box.height / 2 } });
  expect(Math.abs(Number(await slider.inputValue()) - 25)).toBeLessThanOrEqual(6);

  await slider.fill("100");
  await expect.poll(gridOpacity).toBe(1);
  // The slider drives thickness too.
  expect(await lineWidth(":not(.is-downbeat)")).toBeGreaterThan(defaultBeatWidth);
  expect(await lineWidth(".is-downbeat")).toBeGreaterThan(defaultBarWidth);
  expect(await page.evaluate(() => localStorage.getItem("djusbtkit.cueBeatgridLevel"))).toBe("100");

  await slider.fill("0");
  const faint = await gridOpacity();
  expect(faint).toBeGreaterThan(0);
  expect(faint).toBeLessThan(defaultOpacity);
  expect(await lineWidth(":not(.is-downbeat)")).toBeLessThan(defaultBeatWidth);
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
  await expect(rows).toHaveCount(1);
  await expect(page.locator("#trackDetailStartCue .cue-row.is-playback-start")).toHaveCount(1);
  await expect(firstBeat).toHaveAttribute("aria-checked", "true");
  await expect(firstBeat).toBeEnabled();

  // A second cue doesn't add another start cue, nor count it toward the 8.
  await page.locator("#trackDetailAddCue").click();
  await expect(rows).toHaveCount(2);
  await expect(page.locator("#trackDetailStartCue .cue-row.is-playback-start")).toHaveCount(1);

  for (let i = 0; i < 2; i += 1) {
    await page.locator("#trackDetailCueList .cue-row:not(.is-playback-start) .cue-row-delete").first().click();
  }
  await expect(page.locator("#trackDetailCueList .cue-row")).toHaveCount(0);
  await expect(page.locator("#trackDetailStartCue .cue-row")).toHaveCount(0);
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
  await expect(page.locator("#trackDetailStartCue .cue-row.is-playback-start")).toHaveCount(0);

  await firstBeat.click();
  await expect(page.locator("#trackDetailStartCue .cue-row.is-playback-start")).toHaveCount(1);
  await firstCue.click();
  await expect(page.locator("#trackDetailStartCue .cue-row.is-playback-start")).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem("djusbtkit.cueStartOnFirstBeat"))).toBe("0");
});

test("the playback-start cue follows an untouched first beat and is never after a hot cue", async ({ page }) => {
  await openCueEditor(page, { seedCues: [30000], seedStart: 120, quantize: false });
  const firstBeat = page.locator("#trackDetailStartFirstBeat");
  await expect(firstBeat).toHaveAttribute("aria-checked", "true");

  await expect(firstBeat).toHaveText("First beat");
  const startPos = page.locator("#trackDetailStartCue .cue-row.is-playback-start .cue-row-pos");
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

  // The opening view is 60 bars: ~5 px a beat is too dense for beat lines,
  // so only the bar lines are drawn.
  const beatLines = page.locator("#trackDetailBeatgrid .beatgrid-line:not(.is-downbeat)");
  const barLines = page.locator("#trackDetailBeatgrid .beatgrid-line.is-downbeat");
  await expect(beatLines).toHaveCount(0);
  expect(await barLines.count()).toBeGreaterThanOrEqual(60);

  // Labelled every Nth bar, never closer than 36 px.
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
  // Zoomed in, every beat has its line again.
  expect(await beatLines.count()).toBeGreaterThan(0);

  // Packed tighter than 8 px a bar (whole 3:00 track at 512 BPM = 384 bars,
  // ~3 px each), bar lines thin to every Nth bar too.
  await page.locator("#trackDetailZoomFit").click();
  await page.locator("#trackDetailBpmDouble").click();
  await page.locator("#trackDetailBpmDouble").click();
  await expect(page.locator("#trackDetailBpm")).toHaveValue("512.00");
  const barXs = await barLines.evaluateAll((nodes) =>
    nodes.map((n) => n.getBoundingClientRect().left)
  );
  expect(barXs.length).toBeGreaterThan(20);
  expect(barXs.length).toBeLessThan(384 / 2);
  for (let i = 1; i < barXs.length; i += 1) {
    expect(barXs[i] - barXs[i - 1]).toBeGreaterThanOrEqual(7.5);
  }
});

test("the overview strip shows the visible window and moves the view", async ({ page }) => {
  await openCueEditor(page, { seedCues: [30000, 170000] });
  const overview = page.locator("#trackDetailOverview");
  const windowBox = page.locator("#trackDetailOverviewWindow");
  await expect(page.locator("#trackDetailOverviewCues .overview-cue")).toHaveCount(2);

  // The modal opens on 60 bars at 128 BPM (112.5 s) of a 3:00 track: the box covers 5/8.
  const ov = await overview.boundingBox();
  let box = await windowBox.boundingBox();
  expect(Math.abs(box.x - ov.x)).toBeLessThan(2);
  expect(Math.abs(box.width - (ov.width * 5) / 8)).toBeLessThan(3);

  // Zoom in, then click near the end: the view (same zoom) centres there.
  for (let i = 0; i < 3; i += 1) await page.locator("#trackDetailZoomIn").click();
  const zoomedWidth = (ov.width * 112.5) / 8 / 180; // 112.5 s / 2^3 of 180 s
  await expect.poll(async () => (await windowBox.boundingBox()).width).toBeCloseTo(zoomedWidth, 0);
  // 94% of 3:00 is 2:49, so the 14 s view (2:42–2:56) takes in the 2:50 cue.
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

const BEAT_MS = 60000 / 128; // mock track: firstBeatMs 120, 128 BPM
const offBeat = (ms) => {
  const beats = (ms - 120) / BEAT_MS;
  return Math.abs(beats - Math.round(beats)) * BEAT_MS;
};
const savedCues = async (page) => {
  await page.locator("#trackDetailSaveBtn").click();
  const call = await page.evaluate(() =>
    window.__calls.find((c) => c.command === "save_track_analysis_edits")
  );
  return call.request;
};

test("with Q on, a double-clicked cue lands on the nearest beat; Shift+double-click places it freely", async ({ page }) => {
  await openCueEditor(page);
  const wf = page.locator("#trackDetailWaveform");
  const wfBox = await wf.boundingBox();
  // The opening view is 60 bars at 128 BPM = 112.5 s.
  await wf.dblclick({ position: { x: wfBox.width * (30 / 112.5), y: 100 } });
  await wf.dblclick({ position: { x: wfBox.width * 0.62, y: 100 }, modifiers: ["Shift"] });
  const { cues } = await savedCues(page);
  expect(cues).toHaveLength(2);
  expect(Math.abs(cues[0].positionMs - 30000)).toBeLessThan(BEAT_MS);
  expect(offBeat(cues[0].positionMs)).toBeLessThanOrEqual(1);
  expect(Math.abs(cues[1].positionMs - 0.62 * 112500)).toBeLessThan(400);
  expect(offBeat(cues[1].positionMs)).toBeGreaterThan(1);
});

test("beat grid lines sit on whole device pixels, evenly spaced, at a fractional-pixel tempo", async ({ page }) => {
  await openCueEditor(page);
  // 119 BPM at this zoom puts the beats a fractional number of pixels apart.
  await page.locator("#trackDetailBpm").fill("119");
  await page.locator("#trackDetailBpm").press("Enter");
  await page.locator("#trackDetailZoomIn").click();
  const { lefts, dpr } = await page.evaluate(() => ({
    dpr: window.devicePixelRatio || 1,
    lefts: [...document.querySelectorAll("#trackDetailBeatgrid .beatgrid-line")]
      .map((n) => parseFloat(n.style.left))
  }));
  expect(lefts.length).toBeGreaterThan(20);
  for (const left of lefts) {
    expect(Math.abs(left * dpr - Math.round(left * dpr))).toBeLessThan(1e-6);
  }
  // Snapping moves a line at most half a device pixel, so neighbours stay
  // within one device pixel of the true beat spacing.
  const gaps = lefts.slice(1).map((l, i) => l - lefts[i]);
  expect(Math.max(...gaps) - Math.min(...gaps)).toBeLessThanOrEqual(1 / dpr + 1e-6);
});

test("the themed ▲/▼ arrows step the input itself: 0.01 BPM and 1 ms, not a whole beat", async ({ page }) => {
  await openCueEditor(page);
  const bpm = page.locator("#trackDetailBpm");
  const firstBeat = page.locator("#trackDetailFirstBeatMs");
  const arrow = (input, dir) =>
    input.locator(`xpath=..`).locator(`.number-stepper-btns button[data-step="${dir}"]`);
  await expect(bpm).toHaveValue("128.00");
  await expect(firstBeat).toHaveValue("120");

  await arrow(bpm, 1).click();
  await expect(bpm).toHaveValue("128.01");
  await arrow(bpm, -1).click();
  await arrow(bpm, -1).click();
  await expect(bpm).toHaveValue("127.99");

  await arrow(firstBeat, 1).click();
  await expect(firstBeat).toHaveValue("121");
  await arrow(firstBeat, -1).click();
  await arrow(firstBeat, -1).click();
  await expect(firstBeat).toHaveValue("119");
  // The arrows go through the same change handling as typing: they're saved.
  const saved = await savedCues(page);
  expect(saved.bpm).toBe(127.99);
  expect(saved.firstBeatMs).toBe(119);
});

test("÷2 and ×2 fix a half/double-tempo BPM in one click", async ({ page }) => {
  await openCueEditor(page);
  const bpm = page.locator("#trackDetailBpm");
  await expect(bpm).toHaveValue("128.00");
  await page.locator("#trackDetailBpmHalf").click();
  await expect(bpm).toHaveValue("64.00");
  await page.locator("#trackDetailBpmDouble").click();
  await page.locator("#trackDetailBpmDouble").click();
  await expect(bpm).toHaveValue("256.00");
  // The grid follows: twice as many beat lines as at 128.
  expect((await savedCues(page)).bpm).toBe(256);
});

test("keyboard: Space plays/pauses (never presses the focused Save), C adds a cue, 1–8 jump, ←/→ move the selected cue", async ({ page }) => {
  await openCueEditor(page, { seedCues: [30000, 90000] });
  const calls = (command) =>
    page.evaluate((command) => window.__calls.filter((c) => c.command === command), command);
  await expect(page.locator("#trackDetailSaveBtn")).toBeFocused();

  await page.keyboard.press("Space");
  await expect.poll(async () => (await calls("play_resolved_track")).length).toBe(1);
  await page.keyboard.press("Space");
  await expect.poll(async () => (await calls("pause_playback_native")).length).toBe(1);
  expect(await calls("save_track_analysis_edits")).toHaveLength(0);
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();

  // C: a cue at the paused spot (on the nearest beat, Q is on), selected.
  await page.keyboard.press("c");
  const rows = page.locator("#trackDetailCueList .cue-row");
  await expect(rows).toHaveCount(3);
  await expect(page.locator("#trackDetailCueList .cue-row.is-selected")).toHaveCount(1);
  await expect(page.locator("#trackDetailCueMarkers .cue-marker.is-selected")).toHaveCount(1);

  // The new cue sits near the start, so it is A; 3 jumps to the 90 s cue
  // (now C, at 90 s of 180 s) and selects it.
  await page.keyboard.press("3");
  await expect.poll(async () => (await calls("play_resolved_track")).length).toBe(2);
  expect((await calls("play_resolved_track"))[1].request.startRatio).toBeCloseTo(0.5, 3);
  const selectedPos = page.locator("#trackDetailCueList .cue-row.is-selected .cue-row-pos");
  await expect(selectedPos).toHaveText("1:30.00");

  // →: onto the next beat line; ← twice: two beats back; Shift+→: 10 ms.
  const next = 120 + Math.ceil((90000 - 120) / BEAT_MS) * BEAT_MS; // 90120
  await page.keyboard.press("ArrowRight");
  await expect(selectedPos).toHaveText("1:30.12");
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("ArrowLeft");
  const twoBack = Math.round(next - 2 * BEAT_MS); // 89182.5 → 89183
  await expect(selectedPos).toHaveText(`1:29.${String(Math.floor((twoBack % 1000) / 10)).padStart(2, "0")}`);
  await page.keyboard.press("Shift+ArrowRight");
  const { cues } = await savedCues(page);
  const moved = cues.find((c) => c.positionMs > 80000);
  expect(moved.positionMs).toBe(twoBack + 10);
});

test("shortcuts stay out of text fields: typing c/1/Space in a cue name only types", async ({ page }) => {
  await openCueEditor(page, { seedCues: [30000] });
  const name = page.locator("#trackDetailCueList .cue-row .cue-row-name").first();
  await name.click();
  await name.press("End");
  await page.keyboard.type(" c1 ");
  await expect(name).toHaveValue(/ c1 $/);
  await expect(page.locator("#trackDetailCueList .cue-row")).toHaveCount(1);
  expect(
    await page.evaluate(() => window.__calls.some((c) => c.command === "play_resolved_track"))
  ).toBe(false);
});

test("undo/redo: buttons and Ctrl+Z / Ctrl+Shift+Z step through edits; a name typed is one step", async ({ page }) => {
  await openCueEditor(page, { seedCues: [30000] });
  const undo = page.locator("#trackDetailUndo");
  const redo = page.locator("#trackDetailRedo");
  const rows = page.locator("#trackDetailCueList .cue-row");
  const bpm = page.locator("#trackDetailBpm");
  await expect(undo).toBeDisabled();
  await expect(redo).toBeDisabled();

  await page.locator("#trackDetailWaveform").dblclick({ position: { x: 700, y: 100 } });
  await expect(rows).toHaveCount(2);
  await page.locator("#trackDetailBpmDouble").click();
  await expect(bpm).toHaveValue("256.00");
  const name = page.locator("#trackDetailCueList .cue-row .cue-row-name").first();
  const originalName = await name.inputValue();
  await name.fill("");
  await name.pressSequentially("Intro");
  await page.locator("#trackDetailTitle").click();

  await page.keyboard.press("Control+z"); // the whole name at once
  await expect(page.locator("#trackDetailCueList .cue-row .cue-row-name").first()).toHaveValue(originalName);
  await page.keyboard.press("Control+z");
  await expect(bpm).toHaveValue("128.00");
  await undo.click();
  await expect(rows).toHaveCount(1);
  await expect(undo).toBeDisabled();
  await expect(redo).toBeEnabled();

  await page.keyboard.press("Control+Shift+z");
  await expect(rows).toHaveCount(2);
  await redo.click();
  await expect(bpm).toHaveValue("256.00");

  // A new edit clears the redo steps.
  await page.locator("#trackDetailBpmHalf").click();
  await expect(redo).toBeDisabled();

  // Undoing everything leaves nothing unsaved: an outside click closes.
  for (let i = 0; i < 3; i += 1) await page.keyboard.press("Control+z");
  await expect(undo).toBeDisabled();
  await page.locator("#trackDetailOverlay").click({ position: { x: 5, y: 5 } });
  await expect(page.locator("#trackDetailOverlay")).toBeHidden();
});

test("the metronome has the native engine click on the grid, following grid edits and the mix, off on close", async ({ page }) => {
  await openCueEditor(page);
  const sent = () =>
    page.evaluate(() =>
      window.__calls
        .filter((c) => c.command === "set_playback_metronome")
        .map((c) => c.request)
    );
  const last = async () => (await sent()).at(-1);
  const metronome = page.locator("#trackDetailMetronome");
  await expect(metronome).toHaveAttribute("aria-pressed", "false");
  // The Mix slider only shows while the metronome is on.
  const mix = page.locator("#trackDetailMetronomeMix");
  await expect(mix).toBeHidden();
  // Never on by surprise: opening reports it off (or says nothing at all).
  expect((await sent()).every((r) => r.enabled === false)).toBe(true);

  await metronome.click();
  await expect(metronome).toHaveAttribute("aria-pressed", "true");
  await expect(mix).toBeVisible();
  await expect.poll(last).toEqual({ enabled: true, firstBeatMs: 120, bpm: 128, mix: 0.5 });

  // Grid edits (and their undo) reach the engine while it's on.
  await page.locator("#trackDetailBpmDouble").click();
  await expect.poll(last).toEqual({ enabled: true, firstBeatMs: 120, bpm: 256, mix: 0.5 });
  await page.locator("#trackDetailFirstBeatPlus").click();
  await expect.poll(async () => (await last()).firstBeatMs).toBeGreaterThan(120);
  await page.keyboard.press("Control+z");
  await expect.poll(last).toEqual({ enabled: true, firstBeatMs: 120, bpm: 256, mix: 0.5 });

  // No repeat sends when nothing changed (e.g. zooming).
  const count = (await sent()).length;
  await page.locator("#trackDetailZoomIn").click();
  expect((await sent()).length).toBe(count);

  // The Mix slider (both at full level in the middle) reaches the engine live
  // and is remembered.
  await expect(mix).toHaveValue("50");
  await mix.fill("80");
  await expect.poll(async () => (await last()).mix).toBe(0.8);
  expect((await last()).enabled).toBe(true);
  expect(await page.evaluate(() => localStorage.getItem("djusbtkit.cueMetronomeMix"))).toBe("80");

  await metronome.click();
  await expect.poll(async () => (await last()).enabled).toBe(false);
  await expect(mix).toBeHidden();

  // Closing the editor turns it off in the engine.
  await metronome.click();
  await expect.poll(async () => (await last()).enabled).toBe(true);
  await page.locator("#trackDetailCancelBtn").click();
  await expect.poll(async () => (await last()).enabled).toBe(false);

  // Reopened, the metronome is off (so Mix is hidden) and the mix is where it was left.
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(mix).toBeHidden();
  await metronome.click();
  await expect(mix).toHaveValue("80");
});

test("track-detail modal edits BPM, saves it, and the library row/tooltip update", async ({ page }) => {
  await installTrackDetailMock(page);
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();

  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  await expect(page.locator("#trackDetailBpm")).toHaveValue("128.00");

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

test("Camelot notation shows the backend's labels but saves the classic key", async ({ page }) => {
  await installTrackDetailMock(page, { camelot: true });
  await page.goto("/");

  const keyPill = page.locator('#libraryTableBody .track-grid-row .td-key .key-pill');
  await expect(keyPill).toHaveText("8A");
  await expect(keyPill).toHaveClass(/key-pill--h7/);

  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  await expect(page.locator("#trackDetailKey")).toHaveValue("Am");
  await expect(page.locator("#trackDetailKey option:checked")).toHaveText("8A");

  await page.locator("#trackDetailKey").selectOption({ label: "11A" });
  await page.locator("#trackDetailSaveBtn").click();
  await expect(page.locator("#trackDetailOverlay")).toBeHidden();

  const saveCall = await page.evaluate(() =>
    window.__calls.find((c) => c.command === "save_track_analysis_edits")
  );
  expect(saveCall.request.key).toBe("F#m");
  await expect(keyPill).toHaveText("11A");
  await expect(keyPill).toHaveClass(/key-pill--h10/);
});

test("switching key notation in settings persists it and re-fetches the backend's labels", async ({ page }) => {
  await installTrackDetailMock(page);
  await page.goto("/");

  const keyPill = page.locator('#libraryTableBody .track-grid-row .td-key .key-pill');
  await expect(keyPill).toHaveText("Am");

  await page.locator("#settingsBtn").click();
  await expect(page.locator("#keyNotationSelect")).toHaveValue("classic");
  await page.locator("#keyNotationSelect").selectOption("camelot");
  await page.locator("#settingsCloseBtn").click();

  await expect(keyPill).toHaveText("8A");
  const saved = await page.evaluate(() =>
    window.__calls.filter((c) => c.command === "set_frontend_setting" && c.request?.key === "ui_key_notation_v1")
  );
  expect(saved.map((c) => c.request.value)).toEqual(["camelot"]);
  expect(await page.evaluate(() => localStorage.getItem("djusbtkit.keyNotation"))).toBe("camelot");

  await page.locator("#settingsBtn").click();
  await page.locator("#keyNotationSelect").selectOption("classic");
  await page.locator("#settingsCloseBtn").click();
  await expect(keyPill).toHaveText("Am");
});

test("key stepper steps through the backend's key options and wraps at the ends", async ({ page }) => {
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

  await expect(page.locator("#trackDetailBpm")).toHaveValue("128.00");
  await page.locator("#trackDetailBpmPlus").click();
  await expect(page.locator("#trackDetailBpm")).toHaveValue("128.01");
  await page.locator("#trackDetailBpmMinus").click();
  await page.locator("#trackDetailBpmMinus").click();
  await expect(page.locator("#trackDetailBpm")).toHaveValue("127.99");
});

// Grid as opened: first beat 120 ms at 128 BPM (468.75 ms a beat). One cue on
// beat 64 (30120 ms), one off the grid (60000 ms = beat 127.744), and the
// start cue on the first beat.
const FOLLOW_GRID_SEED = { seedCues: [30120, 60000], seedStart: 120, quantize: false };

async function setBpmInput(page, value) {
  await page.locator("#trackDetailBpm").fill(value);
  await page.locator("#trackDetailBpm").dispatchEvent("change");
}

async function savedCuePositions(page) {
  await page.locator("#trackDetailSaveBtn").click();
  await expect(page.locator("#trackDetailOverlay")).toBeHidden();
  const saveCall = await page.evaluate(() =>
    window.__calls.find((c) => c.command === "save_track_analysis_edits")
  );
  return {
    bpm: saveCall.request.bpm,
    firstBeatMs: saveCall.request.firstBeatMs,
    positions: saveCall.request.cues.map((c) => c.positionMs),
  };
}

test("cues follow grid is off by default: BPM and first-beat edits leave cues where they are", async ({ page }) => {
  await openCueEditor(page, FOLLOW_GRID_SEED);
  await expect(page.locator("#trackDetailFollowGrid")).toHaveAttribute("aria-pressed", "false");

  await setBpmInput(page, "120");
  await page.locator("#trackDetailFirstBeatPlus").click();

  // The untouched start cue still follows the first beat, as before.
  expect(await savedCuePositions(page)).toEqual({ bpm: 120, firstBeatMs: 620, positions: [620, 30120, 60000] });
});

test("cues follow grid: BPM and first-beat edits keep every cue on its beat, undoably, without drift", async ({ page }) => {
  await openCueEditor(page, FOLLOW_GRID_SEED);
  const toggle = page.locator("#trackDetailFollowGrid");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  const persisted = await page.evaluate(() => ({
    local: window.localStorage.getItem("djusbtkit.cueFollowGrid"),
    db: window.__calls
      .filter((c) => c.command === "set_frontend_setting")
      .map((c) => c.request)
      .filter((r) => r?.key === "ui_cue_follow_grid_v1")
      .map((r) => r.value),
  }));
  expect(persisted).toEqual({ local: "1", db: ["1"] });

  const startPos = page.locator("#trackDetailStartCue .cue-row.is-playback-start .cue-row-pos");
  const hotPos = page.locator("#trackDetailCueList .cue-row:not(.is-playback-start) .cue-row-pos");

  // 128 -> 120 BPM (500 ms a beat): beat 64 is now 32120 ms, beat 127.744 63992 ms.
  await setBpmInput(page, "120");
  await expect(hotPos).toHaveText(["0:32.12", "1:03.99"]);
  await expect(startPos).toHaveText("0:00.12");

  // One undo puts the grid and the cues back; redo moves them again.
  await page.locator("#trackDetailUndo").click();
  await expect(page.locator("#trackDetailBpm")).toHaveValue("128.00");
  await expect(hotPos).toHaveText(["0:30.12", "1:00.00"]);
  await page.locator("#trackDetailRedo").click();
  await expect(hotPos).toHaveText(["0:32.12", "1:03.99"]);

  // A first-beat nudge moves every cue by one beat (500 ms), the start cue once.
  await page.locator("#trackDetailFirstBeatPlus").click();
  await expect(page.locator("#trackDetailFirstBeatMs")).toHaveValue("620");
  await expect(hotPos).toHaveText(["0:32.62", "1:04.49"]);
  await expect(startPos).toHaveText("0:00.62");

  // Many 0.01 BPM steps up and back down land on the exact same ms.
  for (let i = 0; i < 40; i += 1) await page.locator("#trackDetailBpmPlus").click();
  await expect(page.locator("#trackDetailBpm")).toHaveValue("120.40");
  for (let i = 0; i < 40; i += 1) await page.locator("#trackDetailBpmMinus").click();
  await expect(page.locator("#trackDetailBpm")).toHaveValue("120.00");

  expect(await savedCuePositions(page)).toEqual({ bpm: 120, firstBeatMs: 620, positions: [620, 32620, 64492] });
});

test("cues follow grid is remembered and leaves a hand-moved cue's new spot alone", async ({ page }) => {
  await openCueEditor(page, { ...FOLLOW_GRID_SEED, followGrid: true });
  await expect(page.locator("#trackDetailFollowGrid")).toHaveAttribute("aria-pressed", "true");

  await setBpmInput(page, "120");
  // Move the first hot cue by hand: 1 selects it, Shift+→ moves it 10 ms.
  // (Shortcuts stay out of the BPM field, so leave it first.)
  await page.locator("#trackDetailBpm").blur();
  await page.keyboard.press("1");
  await page.keyboard.press("Shift+ArrowRight");
  const hotPos = page.locator("#trackDetailCueList .cue-row:not(.is-playback-start) .cue-row-pos");
  await expect(hotPos).toHaveText(["0:32.13", "1:03.99"]);

  // Back to 128 BPM: the moved cue keeps its new beat (64.02), the other its old one.
  await setBpmInput(page, "128");
  expect(await savedCuePositions(page)).toEqual({ bpm: 128, firstBeatMs: 120, positions: [120, 30129, 60000] });
});

test("double-click the waveform adds a cue at that position without starting playback", async ({ page }) => {
  await installTrackDetailMock(page);
  await page.goto("/");
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();

  // Default view is 60 bars (0–112.5 s) of the 180 s track; x≈300/1227 ≈ 24 % ⇒ ~28 s.
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

  await page.locator("#trackDetailCueList .cue-row .cue-row-play").click();
  await expect.poll(async () => (await playCalls()).length).toBe(1);
  expect((await playCalls()).at(-1).request.startRatio).toBeCloseTo(90000 / 180000, 2);

  await page.locator("#trackDetailCueMarkers .cue-marker").first().click();
  await expect.poll(async () => (await playCalls()).length).toBe(2);
  expect((await playCalls()).at(-1).request.startRatio).toBeCloseTo(90000 / 180000, 2);
});

test("dragging a cue marker moves it without playing; Quantize (Q, on by default) snaps it, Shift places it freely", async ({ page }) => {
  await installTrackDetailMock(page, { seedCues: [30000] });
  await page.goto("/");
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  const quantize = page.locator("#trackDetailQuantize");
  await expect(quantize).toHaveAttribute("aria-pressed", "true");

  const interval = 60000 / 128; // firstBeatMs 120, 128 BPM
  const offBeatMs = (ms) => {
    const beats = (ms - 120) / interval;
    return Math.abs(beats - Math.round(beats)) * interval;
  };
  const wfBox = await page.locator("#trackDetailWaveform").boundingBox();
  const marker = page.locator("#trackDetailCueMarkers .cue-marker").first();
  const dragTo = async (ratio, { shift = false } = {}) => {
    const box = await marker.boundingBox();
    const y = box.y + box.height / 2;
    if (shift) await page.keyboard.down("Shift");
    await page.mouse.move(box.x + 2, y);
    await page.mouse.down();
    await page.mouse.move(wfBox.x + wfBox.width * (ratio - 0.03), y, { steps: 5 });
    await page.mouse.move(wfBox.x + wfBox.width * ratio, y, { steps: 5 });
    await page.mouse.up();
    if (shift) await page.keyboard.up("Shift");
  };
  const listedMs = async () => {
    const text = await page.locator("#trackDetailCueList .cue-row .cue-row-pos").first().textContent();
    const [m, rest] = text.split(":");
    return Number(m) * 60000 + Number(rest) * 1000;
  };

  // Default view is 60 bars (0–112.5 s): drag the 30 s marker to ≈60 s: on a beat.
  await dragTo(60 / 112.5);
  let ms = await listedMs();
  expect(ms).toBeGreaterThan(59000);
  expect(ms).toBeLessThan(61000);
  expect(offBeatMs(ms)).toBeLessThanOrEqual(10); // list shows centiseconds
  expect(
    await page.evaluate(() => window.__calls.some((c) => c.command === "play_resolved_track"))
  ).toBe(false);

  // Shift: placed where the pointer is (≈37.1 s), not pulled to a beat.
  await dragTo(0.33, { shift: true });
  ms = await listedMs();
  expect(Math.abs(ms - 0.33 * 112500)).toBeLessThan(400);

  // Q off (remembered): a plain drag is free, Shift snaps.
  await quantize.click();
  await expect(quantize).toHaveAttribute("aria-pressed", "false");
  expect(await page.evaluate(() => localStorage.getItem("djusbtkit.cueQuantize"))).toBe("0");
  await dragTo(0.3, { shift: true });

  await page.locator("#trackDetailSaveBtn").click();
  const saveCall = await page.evaluate(() =>
    window.__calls.find((c) => c.command === "save_track_analysis_edits")
  );
  expect(saveCall.request.cues).toHaveLength(1);
  const snapped = saveCall.request.cues[0].positionMs;
  expect(snapped).toBeGreaterThan(33000);
  expect(snapped).toBeLessThan(34500);
  expect(offBeatMs(snapped)).toBeLessThanOrEqual(1);
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
  await installTrackDetailMock(page, { quantize: false });
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

test("opening the editor on the track already playing from its row shows it playing, and hands it back on close", async ({ page }) => {
  await installTrackDetailMock(page);
  await page.goto("/");
  const calls = (command) =>
    page.evaluate((command) => window.__calls.filter((c) => c.command === command), command);
  const row = page.locator("#libraryTableBody .track-grid-row").first();

  await row.locator('[data-action="play-library"]').click();
  await expect(row.locator(".waveform")).toHaveClass(/is-playing/);
  await page.waitForTimeout(400);

  await row.locator('[data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  const btn = page.locator("#trackDetailPlayPause");
  // Still the same playback -- shown here, not restarted or stopped.
  await expect(btn).toHaveAttribute("aria-label", "Pause");
  await expect(page.locator("#trackDetailWaveform")).toHaveClass(/is-playing/);
  await expect.poll(() => modalPlayheadMs(page)).toBeGreaterThan(300);
  expect(await calls("play_resolved_track")).toHaveLength(1);
  expect(await calls("stop_playback_native")).toHaveLength(0);

  // Pause works on the adopted playback.
  await btn.click();
  await expect(btn).toHaveAttribute("aria-label", "Play");
  expect(await calls("pause_playback_native")).toHaveLength(1);
  await btn.click();
  await expect(btn).toHaveAttribute("aria-label", "Pause");

  // Closing leaves it playing, back on the row.
  await page.locator("#trackDetailCloseBtn").click();
  await expect(page.locator("#trackDetailOverlay")).toBeHidden();
  await expect(row.locator(".waveform")).toHaveClass(/is-playing/);
  expect(await calls("stop_playback_native")).toHaveLength(0);
});

test("opening the editor while another track plays stops that track", async ({ page }) => {
  await installTrackDetailMock(page, { secondTrack: true });
  await page.goto("/");
  const calls = (command) =>
    page.evaluate((command) => window.__calls.filter((c) => c.command === command), command);
  const rows = page.locator("#libraryTableBody .track-grid-row");
  await expect(rows).toHaveCount(2);
  const other = rows.filter({ hasText: "Other Track" });

  await other.locator('[data-action="play-library"]').click();
  await expect(other.locator(".waveform")).toHaveClass(/is-playing/);

  await rows.filter({ hasText: "Cue Track" }).locator('[data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  await expect.poll(async () => (await calls("stop_playback_native")).length).toBe(1);
  await expect(page.locator("#trackDetailPlayPause")).toHaveAttribute("aria-label", "Play");
  await expect(other.locator(".waveform")).not.toHaveClass(/is-playing/);
});

test("the modal opens zoomed to 60 bars; Fit shows the whole track; zoom windows cue markers", async ({ page }) => {
  // durationMs 180000; one cue inside the default 2-min view, one past it.
  await installTrackDetailMock(page, { seedCues: [30000, 170000] });
  await page.goto("/");
  await page.locator('#libraryTableBody .waveform-cell [data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  await expect(page.locator("#trackDetailCueList .cue-row")).toHaveCount(2);

  const visibleMarkers = page.locator("#trackDetailCueMarkers .cue-marker:not(.off-view)");
  // Default view is 60 bars at 128 BPM (0–112.5 s), so only the 30 s cue is on screen.
  await expect(visibleMarkers).toHaveCount(1);

  // The zoom-range readout makes the initial zoomed-in view unmistakable: it
  // names the visible window, centred under the waveform; the 3-min total
  // sits at the footer's right end.
  const zoomRange = page.locator("#trackDetailZoomRange");
  const totalTime = page.locator("#trackDetailTotalTime");
  await expect(zoomRange).toBeVisible();
  await expect(zoomRange).toHaveClass(/is-zoomed/);
  await expect(zoomRange).toHaveText("0:00–1:53");
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

    const MAJORS = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          const request = payload?.request ?? payload;
          window.__calls.push({ command, request: request ?? null });
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_rekordbox_db") return { ok: true, data: { found: false, path: null } };
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
            return { ok: true, data: { track, firstBeatMs: 90, cues: [], detailWaveform: detailWaveformB64, keyOptions: [MAJORS, MAJORS.map((k) => `${k}m`)].map((keys, i) => ({ label: i ? "Minor" : "Major", keys: keys.map((k) => ({ value: k, label: k })) })) } };
          }
          if (command === "save_track_analysis_edits") {
            return {
              ok: true,
              data: {
                trackId: "local-1",
                firstBeatMs: request?.firstBeatMs ?? null,
                cues: request?.cues ?? [],
                bpm: request?.bpm ?? null,
                bpmAnalyzer: request?.bpm != null ? "user" : null,
                key: request?.key ?? null,
                keySource: request?.key != null ? "user" : null,
                anlzRegenerated: true,
              },
            };
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
  await page.locator("#trackDetailBpm").fill("131.5");
  await page.locator("#trackDetailBpm").dispatchEvent("change");
  await page.locator("#trackDetailKey").selectOption("Em");

  await page.locator("#trackDetailSaveBtn").click();
  await expect(page.locator("#trackDetailOverlay")).toBeHidden();

  const saveCall = await page.evaluate(() => window.__calls.find((c) => c.command === "save_track_analysis_edits"));
  expect(saveCall).toBeTruthy();
  expect(saveCall.request.trackId).toBe("local-1");
  expect(saveCall.request.cues).toHaveLength(1);

  // The open playlist's row redraws without a reload.
  await expect(row.locator(".td-bpm .bpm-pill")).toHaveText("131.50");
  await expect(row.locator(".td-key .key-pill")).toHaveText("Em");
});

// --- Cue editor opened from a USB view (playlists / history) ---------------
//
// A USB row edits the on-device bundle directly: open goes through
// get_usb_track_detail, save through save_usb_track_analysis_edits with the
// row's raw on-device paths, and a not-connected USB blocks the flow.

// `rowLocalTrackId`: the USB row's localTrackId hint (null = the row doesn't
// know its library track; the save response still names it).
// `libraryTracks`: rows the library table loads, to watch cross-view updates.
// `tauriEvents`: subscribe the app to `job:event` (window.__emitJobEvent fires
// one), and hold the USB save open until window.__releaseUsbSave() while it
// emits the backend's usb_write job progress.
function installUsbTrackDetailMock(page, mockOpts = {}) {
  return page.addInitScript((opts) => {
    const listeners = new Map();
    window.__emitJobEvent = (payload) => {
      for (const cb of (listeners.get("job:event") || []).slice()) cb({ event: "job:event", payload });
    };
    let releaseUsbSave = () => {};
    const usbSaveGate = new Promise((resolve) => { releaseUsbSave = resolve; });
    window.__releaseUsbSave = () => releaseUsbSave();
    if (opts.tauriEvents) {
      // registerBackendJobEvents() only listens when window.isTauri is set, and
      // the bundled invoke then routes through __TAURI_INTERNALS__.
      window.isTauri = true;
      window.__TAURI_INTERNALS__ = { invoke: (cmd, args) => window.__TAURI__.core.invoke(cmd, args) };
    }
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
      localTrackId: opts.rowLocalTrackId === undefined ? "local-1" : opts.rowLocalTrackId,
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

    const MAJORS = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          window.__calls.push({ command, request: payload?.request ?? null });
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "allow_asset_paths") return null;
          if (command === "get_backend_log_buffer") return [];
          if (command === "detect_external_rekordbox_db") return { ok: true, data: { found: false, path: null } };
          if (command === "check_source_roots") return { ok: true, data: { roots: [] } };
          if (command === "list_playlists") return { ok: true, data: { items: [] } };
          if (command === "list_usb_devices") return { ok: true, data: { items: [] } };
          if (command === "list_tracks" || command === "search_tracks") return { ok: true, data: { total: 0, items: [] } };
          if (command === "browse_source_files") {
            const items = opts.libraryTracks || [];
            return { ok: true, data: { total: items.length, items, nextCursor: null, hasMore: false } };
          }
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
            return { ok: true, data: { firstBeatMs: 100, cues: [{ id: "c1", positionMs: 5000, colorId: 5, name: "Old" }], detailWaveform: detailWaveformB64, keyOptions: [MAJORS, MAJORS.map((k) => `${k}m`)].map((keys, i) => ({ label: i ? "Minor" : "Major", keys: keys.map((k) => ({ value: k, label: k })) })) } };
          }
          if (command === "save_usb_track_analysis_edits") {
            if (window.__usbSaveFails) {
              return { ok: false, error: { code: "NOT_FOUND", message: "USB disconnected mid-edit" } };
            }
            if (opts.tauriEvents) {
              const job = { jobId: "usb-save-1", jobType: "usb_write", stage: "save_usb_track_analysis_edits" };
              window.__emitJobEvent({ ...job, event: "job.started", percent: 0, message: "USB: Saving cue edits" });
              window.__emitJobEvent({ ...job, event: "job.progress", percent: 40, message: "USB: Writing export.pdb" });
              await usbSaveGate;
              window.__emitJobEvent({ ...job, event: "job.completed", percent: 100, message: "USB: Cue edits saved" });
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
                // The backend resolves the library track itself (not only the hint).
                localTrackId: "local-1",
              },
            };
          }
          if (command === "stop_playback_native" || command === "get_playback_status_native") return { ok: true, data: {} };
          return { ok: true, data: {} };
        },
      },
      event: {
        listen: async (eventName, callback) => {
          const key = String(eventName || "");
          listeners.set(key, [...(listeners.get(key) || []), callback]);
          return () => listeners.set(key, (listeners.get(key) || []).filter((fn) => fn !== callback));
        },
      },
    };
  }, mockOpts);
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

test("cue editor edits BPM from a USB playlist row, saves it through the USB commands, and every view updates", async ({ page }) => {
  // The row doesn't know its library track: the save response names it, and
  // that library row must update too.
  await installUsbTrackDetailMock(page, {
    rowLocalTrackId: null,
    libraryTracks: [{
      id: "local-1",
      title: "USB Cue Track",
      artist: "USB Artist",
      bpm: 126,
      key: "Bm",
      durationMs: 200000,
      filePath: "/music/track.mp3",
      analysisReady: true,
      waveformPreview: Array.from({ length: 80 }, (_, i) => (i % 7) * 12),
    }],
  });
  await page.goto("/");
  const libraryBpm = page.locator('#libraryTableBody .track-grid-row[data-track-id="local-1"] .td-bpm .bpm-pill');
  await expect(libraryBpm).toHaveText("126.00");

  await openUsbView(page, "usb-playlists");
  await page.locator("#refreshUsbBtn").click();
  await page.locator('[data-usb-playlist-index="0"]').click();

  const row = page.locator("#usbPlaylistTracks .track-grid-row");
  await row.locator('[data-action="edit-track-detail"]').click();
  await expect(page.locator("#trackDetailOverlay")).toBeVisible();
  await expect(page.locator("#trackDetailBpm")).toHaveValue("126.00");

  await page.locator("#trackDetailBpm").fill("128.5");
  await page.locator("#trackDetailBpm").dispatchEvent("change");
  await page.locator("#trackDetailSaveBtn").click();
  await expect(page.locator("#trackDetailOverlay")).toBeHidden();

  const saveCall = await page.evaluate(() => window.__calls.find((c) => c.command === "save_usb_track_analysis_edits"));
  expect(saveCall.request.bpm).toBe(128.5);
  expect(saveCall.request.localTrackId).toBeNull();
  // Cues weren't edited: none are sent, so the stick's cues stay as they are.
  expect(saveCall.request.cues).toBeNull();

  // The USB row redraws with the new BPM, and so does the library track.
  await expect(row.locator(".td-bpm .bpm-pill")).toHaveText("128.50");
  await expect(libraryBpm).toHaveText("128.50");
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
  await expect(row.locator(".td-key .key-pill")).toHaveText("Dm");
});

test("a USB save runs as a background job: the progress bar shows each write and the window stays usable", async ({ page }) => {
  await installUsbTrackDetailMock(page, { tauriEvents: true });
  await page.goto("/");

  await openUsbView(page, "usb-playlists");
  await page.locator("#refreshUsbBtn").click();
  await page.locator('[data-usb-playlist-index="0"]').click();
  const row = page.locator("#usbPlaylistTracks .track-grid-row");
  await row.locator('[data-action="edit-track-detail"]').click();
  await page.locator("#trackDetailBpm").fill("128.5");
  await page.locator("#trackDetailBpm").dispatchEvent("change");
  await page.locator("#trackDetailSaveBtn").click();
  await expect(page.locator("#trackDetailOverlay")).toBeHidden();

  // While the stick is being written, the footer shows the backend's step.
  await expect(page.locator("#progressFooter")).toHaveClass(/active/);
  await expect(page.locator("#progressText")).toContainText("USB: Writing export.pdb");
  // ... and the UI still responds (switch to USB history and back).
  await page.locator('.nav-item[data-view="usb-history"]').click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();

  await page.evaluate(() => window.__releaseUsbSave());
  await expect(row.locator(".td-bpm .bpm-pill")).toHaveText("128.50");
  await expect(page.locator("#progressFooter")).not.toHaveClass(/active/, { timeout: 3000 });
});

test("a USB save waits for a running USB job instead of writing alongside it", async ({ page }) => {
  await installUsbTrackDetailMock(page, { tauriEvents: true });
  await page.goto("/");

  await openUsbView(page, "usb-playlists");
  await page.locator("#refreshUsbBtn").click();
  await page.locator('[data-usb-playlist-index="0"]').click();
  const row = page.locator("#usbPlaylistTracks .track-grid-row");
  await row.locator('[data-action="edit-track-detail"]').click();
  await page.locator("#trackDetailAddCue").click();

  // An export starts while the editor is open.
  const exportJob = { jobId: "export-1", jobType: "export", stage: "export_to_usb" };
  await page.evaluate((job) => window.__emitJobEvent({ ...job, event: "job.started", percent: 0, message: "USB: Exporting playlist" }), exportJob);
  await page.locator("#trackDetailSaveBtn").click();
  await expect(page.locator("#statusText")).toContainText("Waiting for the running USB job");
  const saveCalls = () => page.evaluate(() => window.__calls.filter((c) => c.command === "save_usb_track_analysis_edits").length);
  expect(await saveCalls()).toBe(0);

  // Once the export finishes, the edits are saved -- nothing was dropped.
  await page.evaluate((job) => window.__emitJobEvent({ ...job, event: "job.completed", percent: 100, message: "USB: Export complete" }), exportJob);
  await expect.poll(saveCalls).toBe(1);
  await page.evaluate(() => window.__releaseUsbSave());
  await expect(page.locator("#statusText")).toContainText("to USB");
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
          if (command === "detect_external_rekordbox_db") return { ok: true, data: { found: false, path: null } };
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
