// Records the docs media in docs/assets/ -- the cue-editor and
// playlist-import GIFs and the README / cue-editor screenshots -- from the
// real frontend build.
//
//   npm run docs:media           (from vanilla-ui/; needs cargo and ffmpeg)
//   npm run docs:media -- cue-editor cue-editor-drag-cues   (only those)
//
// 1. Synthesises a made-up 10-track library (two albums, both in the checked
//    source folder; the other, unchecked one only shows the "Filtered" badge)
//    with ffmpeg.
// 2. Scans and analyzes it with the real backend (`dump_doc_gif_fixture`
//    bin) and dumps what the frontend would receive.
// 3. Serves dist/ with Tauri's invoke stubbed from that dump (page_init.js,
//    which also stands in for the playlists and a connected USB), drives the
//    app in headless Chromium, and writes each GIF scene (lossless frames
//    over CDP, encoded with ffmpeg) and each screenshot.
import { chromium } from "@playwright/test";
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const uiDir = resolve(here, "../..");
const repoDir = resolve(uiDir, "..");
const outDir = join(repoDir, "docs/assets");
const BASE_URL = "http://127.0.0.1:4173/";
const VIEWPORT = { width: 1280, height: 837 };
const GIF_WIDTH = 960;
const EDITOR_TITLE = "Demo Groove";
// What the source folders look like in the app (long, so the chips truncate).
const SHOWN_ROOT = "/home/dj/Music/Projects/Chiphead.Music";
const FOLDERS = ["Syyskuu", "Heinäkuu"];

const run = (cmd, args, opts = {}) => execFileSync(cmd, args, { stdio: "inherit", ...opts });

// --- 1. Demo library ---

const ALBUMS = [
  {
    folder: "Syyskuu/Night Shift EP",
    album: "Night Shift EP",
    artist: "Kaamos Unit",
    cover:
      "gradients=s=400x400:c0=0x2e1065:c1=0xdb2777:c2=0xf59e0b:n=3:x0=0:y0=0:x1=400:y1=400:seed=7," +
      "drawgrid=w=40:h=40:t=1:c=white@0.12",
    tracks: [
      // The editor track of the GIFs: 150 s, kickless breakdown in bars 25–32.
      { title: EDITOR_TITLE, bpm: 129, rootHz: 55, seconds: 150, breakdown: 3, minor: true },
      { title: "Low Orbit", bpm: 124, rootHz: 49, seconds: 170, breakdown: 2, minor: false },
      { title: "Frost Line", bpm: 126, rootHz: 43.65, seconds: 160, breakdown: 4, minor: true },
      { title: "Northbound", bpm: 122, rootHz: 41.2, seconds: 185, breakdown: 3, minor: false },
      { title: "Afterglow", bpm: 128, rootHz: 61.74, seconds: 175, breakdown: 5, minor: true },
    ],
  },
  {
    folder: "Syyskuu/Lakeside Tapes",
    album: "Lakeside Tapes",
    artist: "Midsummer Static",
    cover:
      "mandelbrot=s=400x400:start_x=-0.7453:start_y=0.1127:start_scale=0.012:outer=normalized_iteration_count," +
      "hue=h=190:s=1.1,gblur=sigma=0.8",
    tracks: [
      { title: "Sauna Talk", bpm: 132, rootHz: 36.71, seconds: 165, breakdown: 2, minor: true },
      { title: "Pier Lights", bpm: 136, rootHz: 58.27, seconds: 155, breakdown: 4, minor: false },
      { title: "White Nights", bpm: 140, rootHz: 46.25, seconds: 180, breakdown: 3, minor: true },
      { title: "Driftwood", bpm: 134, rootHz: 65.41, seconds: 150, breakdown: 5, minor: false },
      { title: "Last Ferry", bpm: 138, rootHz: 51.91, seconds: 190, breakdown: 2, minor: true },
    ],
  },
];

// Kick + off-beat hats, bass from the third 8-bar section, a kickless
// breakdown in section `breakdown`, and a snare on 2 and 4 from the fifth, so
// the waveform and the overview strip have some shape. A sustained triad on
// the bass root gives the key analysis something to find.
function synthesizeTrack(path, { title, artist, album, cover, bpm, rootHz, minor, seconds, breakdown }) {
  const beat = `(60/${bpm})`;
  const t = "(t-0.05)";
  const inBeat = `mod(${t},${beat})`;
  const offBeat = `mod(${t}-${beat}/2,${beat})`;
  const section = `floor(${t}/(${beat}*32))`;
  const kick = `if(gte(${t},0),sin(2*PI*(45*${inBeat}+(110/28)*(1-exp(-28*${inBeat}))))*exp(-7*${inBeat}),0)`;
  const hat = `(random(0)*2-1)*exp(-70*${offBeat})*0.22`;
  const bass = `0.25*sin(2*PI*${rootHz}*t)*(1-exp(-20*${offBeat}))*exp(-6*${offBeat})`;
  const snare = `(random(1)*2-1)*exp(-25*mod(${t}-${beat},2*${beat}))*0.3`;
  const third = minor ? 1.18921 : 1.25992;
  const pad = [1, third, 1.49831].map((r) => `sin(2*PI*${(rootHz * 4 * r).toFixed(3)}*t)`).join("+");
  const mix = `0.8*(if(eq(${section},${breakdown}),0,${kick})+${hat}+if(gte(${section},2),${bass},0)+if(gte(${section},4),${snare},0)+0.07*(${pad}))`;
  run("ffmpeg", [
    "-v", "error", "-y",
    "-f", "lavfi", "-i", `aevalsrc=exprs='${mix}|${mix}':s=44100:d=${seconds}`,
    "-f", "lavfi", "-t", "0.04", "-i", cover,
    "-map", "0:a", "-map", "1:v", "-c:v", "mjpeg", "-disposition:v", "attached_pic",
    "-metadata", `title=${title}`, "-metadata", `artist=${artist}`, "-metadata", `album=${album}`,
    "-b:a", "192k", path,
  ]);
}

function buildFixture(work) {
  const src = join(work, "Chiphead.Music");
  for (const { folder, tracks, ...albumTags } of ALBUMS) {
    mkdirSync(join(src, folder), { recursive: true });
    tracks.forEach((track, i) => {
      const file = `${String(i + 1).padStart(2, "0")} ${track.title}.mp3`;
      synthesizeTrack(join(src, folder, file), { ...albumTags, ...track });
    });
  }

  const json = join(work, "fixture.json");
  run("cargo", [
    "run", "-q", "--release", "-p", "backend", "--features", "dev-tools", "--bin", "dump_doc_gif_fixture",
    "--", join(work, "data"), src, EDITOR_TITLE, json,
  ], { cwd: repoDir });

  const fixture = JSON.parse(readFileSync(json, "utf8"));
  // The page can't read local files: inline artwork, and show made-up paths.
  const mime = { ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".png": "image/png" };
  for (const track of fixture.tracks) {
    if (track.artworkPath) {
      const type = mime[extname(track.artworkPath).toLowerCase()] || "image/jpeg";
      track.artworkDataUrl = `data:${type};base64,${readFileSync(track.artworkPath).toString("base64")}`;
    }
    track.artworkPath = null;
    track.filePath = `${SHOWN_ROOT}/${relative(src, track.filePath)}`;
    track.waveformPeaksPath = track.waveformPeaksPath && `/analysis/${basename(track.waveformPeaksPath)}`;
  }
  // Album order, then track number (the file name), as a tidy library shows.
  fixture.tracks.sort((a, b) => {
    const folder = (t) => ALBUMS.findIndex((al) => t.filePath.includes(`/${al.folder}/`));
    return folder(a) - folder(b) || a.filePath.localeCompare(b.filePath);
  });
  fixture.detail.track = fixture.tracks.find((t) => t.id === fixture.detail.track.id);
  fixture.sourceRoots = FOLDERS.map((f) => `${SHOWN_ROOT}/${f}`);
  // Heinäkuu unchecked, so the Library shows the "Filtered" badge.
  fixture.sourceRootEnabled = { [fixture.sourceRoots[0]]: true, [fixture.sourceRoots[1]]: false };
  const byTitle = (...titles) => fixture.tracks.filter((t) => titles.includes(t.title));
  // Sidebar order top to bottom; `current` is the active playlist.
  fixture.playlists = [
    { id: "pl-event1", name: "Event 1", tracks: byTitle("Afterglow", "White Nights", "Last Ferry", "Pier Lights") },
    { id: "pl-bass", name: "Bass", current: true, tracks: byTitle("Low Orbit", "Northbound", "Sauna Talk", "Driftwood", EDITOR_TITLE) },
    { id: "pl-house", name: "House", tracks: byTitle("Frost Line", "Afterglow", "Pier Lights") },
    { id: "pl-1", name: "Playlist 1", tracks: [] },
  ];
  return fixture;
}

// --- 2. Browser session + capture ---

async function openApp(browser, fixture, opts) {
  const ctx = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 1, colorScheme: "dark" });
  await ctx.addInitScript({
    content: `window.__DOC_GIF_FIXTURE__=${JSON.stringify(fixture)};window.__DOC_GIF_OPTS__=${JSON.stringify(opts)};`,
  });
  await ctx.addInitScript({ path: join(here, "page_init.js") });
  const page = await ctx.newPage();
  await page.goto(BASE_URL);
  await page.locator("#libraryTableBody .track-grid-row", { hasText: EDITOR_TITLE }).waitFor();
  await page.waitForTimeout(300);
  return { ctx, page };
}

async function openEditor(page) {
  const row = page.locator("#libraryTableBody .track-grid-row", { hasText: EDITOR_TITLE });
  await row.locator('.waveform-cell [data-action="edit-track-detail"]').click();
  await page.locator("#trackDetailOverlay").waitFor({ state: "visible" });
  await page.waitForTimeout(400);
  return page.locator("#trackDetailWaveform").boundingBox();
}

async function openSession(browser, fixture, opts) {
  const { ctx, page } = await openApp(browser, fixture, opts);
  const wf = await openEditor(page);
  return { ctx, page, wf };
}

// The app connects a USB only from the picker or a recent-USB button, and has
// no stored "current playlist": click both, as a user would, then go back to
// the Library.
async function connectUsbAndPickPlaylist(page, fixture) {
  await page.locator("[data-usb-recent-path]").first().dispatchEvent("click");
  await page.locator("#usbNameBadgeLabel", { hasText: "Chiphead" }).waitFor();
  await page.locator("#statusText", { hasText: "Diagnostics complete" }).waitFor();
  const current = fixture.playlists.find((p) => p.current);
  await page.locator(`.nav-playlist-item[data-playlist-id="${current.id}"]`).click();
  await page.locator("#playlistTracksBody .track-grid-row").first().waitFor();
  await page.locator('.nav-item[data-view="library"]').click();
  await page.locator("#libraryTableBody .track-grid-row").first().waitFor();
  await page.mouse.move(VIEWPORT.width - 2, VIEWPORT.height - 2);
  await page.waitForTimeout(600);
}

// Scroll-zooms the editor to 0:00–0:15, then drags the view back to 0:00
// (zooming around the left edge drifts a fraction of a second in).
async function zoomToIntro(page, wf, { ticks = 14, tickDelay = 20 } = {}) {
  await page.mouse.move(wf.x + 4, wf.y + wf.height / 2, { steps: tickDelay > 30 ? 25 : 1 });
  for (let i = 0; i < ticks; i += 1) {
    await page.mouse.wheel(0, -100);
    await page.waitForTimeout(tickDelay);
  }
}
async function panToStart(page, wf) {
  const y = wf.y + wf.height * 0.8;
  await page.mouse.move(wf.x + wf.width / 2, y);
  await page.mouse.down();
  await page.mouse.move(wf.x + wf.width / 2 + 120, y, { steps: 8 });
  await page.mouse.up();
}

async function moveTo(page, x, y, steps = 18) {
  await page.mouse.move(x, y, { steps });
}
async function clickOn(page, locator, pause = 250) {
  const b = await locator.boundingBox();
  await moveTo(page, b.x + b.width / 2, b.y + b.height / 2);
  await page.waitForTimeout(120);
  await page.mouse.down();
  await page.waitForTimeout(70);
  await page.mouse.up();
  await page.waitForTimeout(pause);
}
async function typeInto(page, locator, text) {
  await clickOn(page, locator, 150);
  await page.keyboard.press("Control+A");
  await page.keyboard.type(text, { delay: 140 });
  await page.waitForTimeout(200);
  await page.keyboard.press("Enter");
  await locator.blur();
}
async function dragMarker(page, marker, dx, { shift = false, steps = 60, stepDelay = 28 } = {}) {
  const b = await marker.boundingBox();
  const x0 = b.x + b.width / 2;
  const y = b.y + b.height / 2;
  await moveTo(page, x0, y, 20);
  await page.waitForTimeout(250);
  if (shift) {
    const list = await page.locator("#trackDetailCueList").boundingBox();
    await page.keyboard.down("Shift");
    await page.evaluate(([x, top]) => window.__docGifKey("⇧ Shift", x, top), [list.x + list.width / 2, list.y - 30]);
    await page.waitForTimeout(350);
  }
  await page.mouse.down();
  for (let i = 1; i <= steps; i += 1) {
    await page.mouse.move(x0 + (dx * i) / steps, y);
    await page.waitForTimeout(stepDelay);
  }
  await page.waitForTimeout(250);
  await page.mouse.up();
  if (shift) {
    await page.waitForTimeout(250);
    await page.keyboard.up("Shift");
    await page.evaluate(() => window.__docGifKey(null));
  }
}

const FULL_WINDOW = { x: 0, y: 0, width: VIEWPORT.width, height: VIEWPORT.height };

// Captures PNG frames over CDP while `scene` runs; returns the frame list and,
// for cropping, the union of the editor dialog's boxes before and after (it
// grows with the cue list) -- or the whole window with `fullWindow`.
async function capture(page, framesDir, scene, { fullWindow = false } = {}) {
  rmSync(framesDir, { recursive: true, force: true });
  mkdirSync(framesDir, { recursive: true });
  const dialog = page.locator("#trackDetailOverlay > *").first();
  const cropBox = async () => (fullWindow ? FULL_WINDOW : dialog.boundingBox());
  const boxes = [await cropBox()];
  const cdp = await page.context().newCDPSession(page);
  const frames = [];
  cdp.on("Page.screencastFrame", async ({ data, metadata, sessionId }) => {
    const file = join(framesDir, `${String(frames.length).padStart(5, "0")}.png`);
    frames.push({ file, t: metadata.timestamp });
    writeFileSync(file, Buffer.from(data, "base64"));
    await cdp.send("Page.screencastFrameAck", { sessionId }).catch(() => {});
  });
  await cdp.send("Page.startScreencast", { format: "png", everyNthFrame: 1 });
  await page.waitForTimeout(700);
  await scene();
  await page.waitForTimeout(300);
  await cdp.send("Page.stopScreencast");
  boxes.push(await cropBox());
  const x = Math.min(...boxes.map((b) => b.x));
  const y = Math.min(...boxes.map((b) => b.y));
  const crop = {
    x: Math.floor(x),
    y: Math.floor(y),
    w: Math.ceil(Math.max(...boxes.map((b) => b.x + b.width)) - x),
    h: Math.ceil(Math.max(...boxes.map((b) => b.y + b.height)) - y),
  };
  return { frames, crop };
}

function encodeGif({ frames, crop }, framesDir, out) {
  // Frames only arrive when the page changes: hold each until the next one.
  let list = "";
  frames.forEach((f, i) => {
    const next = i + 1 < frames.length ? frames[i + 1].t : f.t + 0.5;
    list += `file '${f.file}'\nduration ${Math.max(0.01, next - f.t).toFixed(3)}\n`;
  });
  list += `file '${frames.at(-1).file}'\n`;
  const listFile = join(framesDir, "list.txt");
  writeFileSync(listFile, list);
  const palette = join(framesDir, "palette.png");
  const vf = `fps=12,crop=${crop.w}:${crop.h}:${crop.x}:${crop.y},scale=${GIF_WIDTH}:-1:flags=lanczos`;
  const input = ["-v", "error", "-y", "-f", "concat", "-safe", "0", "-i", listFile];
  run("ffmpeg", [...input, "-vf", `${vf},palettegen=max_colors=128:stats_mode=diff`, palette]);
  run("ffmpeg", [
    ...input, "-i", palette,
    "-lavfi", `${vf}[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle`,
    out,
  ]);
  console.log(`wrote ${relative(repoDir, out)}`);
}

// --- 3. Scenes ---

function scenes(fixture) {
  const bpm = fixture.detail.track.bpm;
  const firstBeat = fixture.detail.firstBeatMs ?? 0;
  const beatMs = 60000 / bpm;
  const onBeat = (n) => Math.round(firstBeat + n * beatMs);
  // Pixels per beat at the 0:00–0:15 zoom.
  const beatPx = (wf) => (beatMs / 15000) * wf.width;

  return [
    {
      // BPM and first beat off, then fixed by typing: the grid follows.
      name: "cue-editor-beatgrid",
      opts: { cues: [], bpm: bpm - 2, firstBeatMs: firstBeat + 215 },
      async run(page, wf, record) {
        await page.mouse.move(700, 230);
        await record(async () => {
          await zoomToIntro(page, wf, { tickDelay: 45 });
          await page.waitForTimeout(1300);
          await typeInto(page, page.locator("#trackDetailBpm"), String(bpm));
          await page.waitForTimeout(1500);
          await typeInto(page, page.locator("#trackDetailFirstBeatMs"), String(firstBeat));
          await page.waitForTimeout(1500);
          await clickOn(page, page.locator("#trackDetailFirstBeatPlus"), 1100);
          await clickOn(page, page.locator("#trackDetailFirstBeatMinus"), 900);
          await moveTo(page, wf.x + wf.width * 0.6, wf.y + wf.height * 0.55, 20);
          await page.waitForTimeout(1500);
        });
      },
    },
    {
      // Cue A dragged with Quantize on (hops beat to beat), cue B with Shift (free).
      name: "cue-editor-drag-cues",
      opts: {
        cues: [
          { positionMs: onBeat(8), colorId: 5, name: "Intro" },
          { positionMs: onBeat(20), colorId: 1, name: "Build" },
        ],
      },
      async run(page, wf, record) {
        await zoomToIntro(page, wf);
        await panToStart(page, wf);
        await page.mouse.move(700, 230);
        await page.waitForTimeout(500);
        const markers = page.locator("#trackDetailCueMarkers .cue-marker:not(.is-playback-start)");
        await record(async () => {
          await dragMarker(page, markers.nth(0), beatPx(wf) * 5.6, { steps: 77 });
          await page.waitForTimeout(900);
          await dragMarker(page, markers.nth(1), -beatPx(wf) * 3.6, { shift: true, steps: 75 });
          await page.waitForTimeout(900);
          await moveTo(page, wf.x + wf.width * 0.85, wf.y + wf.height * 0.3, 20);
          await page.waitForTimeout(1400);
        });
      },
    },
    {
      // "First beat" adds the start marker; dragging it makes it a "Start marker".
      name: "cue-editor-playback-start",
      opts: {
        cues: [
          { positionMs: onBeat(12), colorId: 5, name: "Drop in" },
          { positionMs: onBeat(24), colorId: 1, name: "Build" },
        ],
      },
      async run(page, wf, record) {
        await zoomToIntro(page, wf);
        await panToStart(page, wf);
        await page.mouse.move(700, 230);
        await page.waitForTimeout(500);
        await record(async () => {
          await page.waitForTimeout(500);
          await clickOn(page, page.locator("#trackDetailStartFirstBeat"), 1600);
          const marker = page.locator("#trackDetailCueMarkers .cue-marker.is-playback-start");
          await dragMarker(page, marker, beatPx(wf) * 4);
          await page.waitForTimeout(700);
          await moveTo(page, wf.x + wf.width * 0.8, wf.y + wf.height * 0.3, 20);
          await page.waitForTimeout(1500);
        });
      },
    },
    {
      // docs/EXTERNAL_LIBRARIES.md: Import under New, pick a rekordbox
      // playlist from the grouped list, and it opens as a new playlist.
      name: "import-playlist",
      editor: false,
      opts: {
        externalLibraries: {
          rekordbox: [
            { id: "3924571", name: "Friday Set", kind: "playlist", titles: ["Low Orbit", "Sauna Talk", "Pier Lights", "White Nights", "Afterglow"] },
            { id: "1180562", name: "Sets / Warmup", kind: "playlist", titles: ["Northbound", "Driftwood", "Frost Line"] },
            { id: "8841207", name: "HISTORY 2026-09-19", kind: "history", titles: ["Frost Line", "Last Ferry"] },
          ],
          mixxx: [
            { id: "4", name: "Late Night", kind: "playlist", titles: ["Afterglow", "Last Ferry"] },
            { id: "2", name: "Peak Time", kind: "crate", titles: ["White Nights", "Last Ferry", "Pier Lights"] },
            { id: "7", name: "2026-09-26", kind: "history", titles: ["Sauna Talk", "Driftwood"] },
          ],
        },
      },
      async run(page, _wf, record) {
        await page.mouse.move(820, 420);
        await page.waitForTimeout(300);
        await record(async () => {
          await page.waitForTimeout(600);
          await clickOn(page, page.locator("#importPlaylistBtn"), 900);
          // The native drop-down list isn't in a screencast: step through the
          // choices on the closed select instead, as the keyboard does.
          const select = page.locator("#playlistImportSelect");
          await moveTo(page, ...(await center(select)));
          await select.focus();
          await page.waitForTimeout(700);
          for (let i = 0; i < 4; i += 1) {
            await page.keyboard.press("ArrowDown");
            await page.waitForTimeout(650);
          }
          for (let i = 0; i < 4; i += 1) {
            await page.keyboard.press("ArrowUp");
            await page.waitForTimeout(450);
          }
          await page.waitForTimeout(500);
          await clickOn(page, page.locator("#playlistImportOkBtn"), 300);
          await page.locator("#playlistTracksBody .track-grid-row").first().waitFor();
          await moveTo(page, 820, 520, 20);
          await page.waitForTimeout(2200);
        }, { fullWindow: true });
      },
    },
  ];
}

async function center(locator) {
  const b = await locator.boundingBox();
  return [b.x + b.width / 2, b.y + b.height / 2];
}

// --- 4. Screenshots ---

function shots(fixture) {
  const bpm = fixture.detail.track.bpm;
  const firstBeat = fixture.detail.firstBeatMs ?? 0;
  const onBeat = (n) => Math.round(firstBeat + n * (60000 / bpm));

  return [
    {
      // README hero: the Library with a USB connected and a playlist active.
      name: "DJ-USB-Tkit",
      opts: {},
      async run() {},
    },
    {
      // docs/CUE_EDITOR.md: a start marker off the first beat, one hot cue,
      // paused mid-intro, the hot cue selected.
      name: "cue-editor",
      opts: {
        cues: [
          { positionMs: onBeat(3), playbackStart: true },
          { positionMs: onBeat(16), colorId: 5, name: "Drop" },
        ],
      },
      async run(page) {
        const wf = await openEditor(page);
        await zoomToIntro(page, wf, { ticks: 10 });
        await panToStart(page, wf);
        const playPause = page.locator("#trackDetailPlayPause");
        await playPause.click();
        await page.waitForTimeout(1500);
        await playPause.click();
        await page.locator("#statusText", { hasText: "Paused" }).waitFor();
        await page.locator("#trackDetailCueList .cue-row:not(.is-playback-start) .cue-row-pos").click();
        await page.mouse.move(VIEWPORT.width - 2, VIEWPORT.height - 2);
        await page.waitForTimeout(500);
      },
    },
  ];
}

// --- main ---

const work = mkdtempSync(join(tmpdir(), "doc-media-"));
let server = null;
let browser = null;
try {
  const fixture = buildFixture(work);
  run("node", ["scripts/build.mjs"], { cwd: uiDir });
  server = spawn("node", ["tests/e2e/static-server.mjs"], { cwd: uiDir, stdio: "ignore" });
  for (let i = 0; i < 50; i += 1) {
    if (await fetch(BASE_URL).then(() => true, () => false)) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  browser = await chromium.launch();
  const only = process.argv.slice(2);
  const wanted = (name) => !only.length || only.includes(name);
  for (const scene of scenes(fixture)) {
    if (!wanted(scene.name)) continue;
    const { ctx, page, wf } = scene.editor === false
      ? await openApp(browser, fixture, scene.opts)
      : await openSession(browser, fixture, scene.opts);
    const framesDir = join(work, `frames-${scene.name}`);
    let captured = null;
    await scene.run(page, wf, async (body, captureOpts) => {
      captured = await capture(page, framesDir, body, captureOpts);
    });
    await ctx.close();
    encodeGif(captured, framesDir, join(outDir, `${scene.name}.gif`));
  }
  for (const shot of shots(fixture)) {
    if (!wanted(shot.name)) continue;
    const { ctx, page } = await openApp(browser, fixture, shot.opts);
    await connectUsbAndPickPlaylist(page, fixture);
    await shot.run(page);
    const out = join(outDir, `${shot.name}.png`);
    await page.screenshot({ path: out });
    await ctx.close();
    console.log(`wrote ${relative(repoDir, out)}`);
  }
} finally {
  await browser?.close();
  server?.kill();
  rmSync(work, { recursive: true, force: true });
}
