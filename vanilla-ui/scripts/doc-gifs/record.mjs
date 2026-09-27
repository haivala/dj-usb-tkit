// Records the cue-editor GIFs in docs/assets/ from the real frontend build.
//
//   npm run docs:gifs            (from vanilla-ui/; needs cargo and ffmpeg)
//
// 1. Synthesises a 129 BPM demo track with ffmpeg and puts it in a temporary
//    source folder with the backend's fixture tracks.
// 2. Scans and analyzes that folder with the real backend
//    (`dump_doc_gif_fixture` bin) and dumps what the frontend would receive.
// 3. Serves dist/ with Tauri's invoke stubbed from that dump
//    (page_init.js), drives the cue editor in headless Chromium, captures
//    lossless frames over CDP, and encodes each scene as a GIF.
import { chromium } from "@playwright/test";
import { spawn, execFileSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const uiDir = resolve(here, "../..");
const repoDir = resolve(uiDir, "..");
const fixturesDir = join(repoDir, "backend/tests/fixtures");
const outDir = join(repoDir, "docs/assets");
const BASE_URL = "http://127.0.0.1:4173/";
const VIEWPORT = { width: 1280, height: 800 };
const GIF_WIDTH = 960;
const EDITOR_TITLE = "Demo Groove";

const run = (cmd, args, opts = {}) => execFileSync(cmd, args, { stdio: "inherit", ...opts });

// --- 1. Demo library ---

// 150 s at 129 BPM: kick + off-beat hats, bass from bar 17, a kickless
// breakdown in bars 25–32, and a snare on 2 and 4 from bar 33, so the
// waveform and the overview strip have some shape.
function synthesizeDemoTrack(path) {
  const beat = "(60/129)";
  const t = "(t-0.05)";
  const inBeat = `mod(${t},${beat})`;
  const offBeat = `mod(${t}-${beat}/2,${beat})`;
  const section = `floor(${t}/(${beat}*32))`;
  const kick = `if(gte(${t},0),sin(2*PI*(45*${inBeat}+(110/28)*(1-exp(-28*${inBeat}))))*exp(-7*${inBeat}),0)`;
  const hat = `(random(0)*2-1)*exp(-70*${offBeat})*0.22`;
  const bass = `0.25*sin(2*PI*55*t)*(1-exp(-20*${offBeat}))*exp(-6*${offBeat})`;
  const snare = `(random(1)*2-1)*exp(-25*mod(${t}-${beat},2*${beat}))*0.3`;
  const mix = `0.8*(if(eq(${section},3),0,${kick})+${hat}+if(gte(${section},2),${bass},0)+if(gte(${section},4),${snare},0))`;
  run("ffmpeg", [
    "-v", "error", "-y",
    "-f", "lavfi", "-i", `aevalsrc=exprs='${mix}|${mix}':s=44100:d=150`,
    "-i", join(fixturesDir, "artwork/folder_cover.jpg"),
    "-map", "0:a", "-map", "1:v", "-c:v", "mjpeg", "-disposition:v", "attached_pic",
    "-metadata", `title=${EDITOR_TITLE}`, "-metadata", "artist=DJ USB Tkit", "-metadata", "album=Docs",
    "-b:a", "192k", path,
  ]);
}

function buildFixture(work) {
  const src = join(work, "music");
  mkdirSync(src, { recursive: true });
  synthesizeDemoTrack(join(src, "demo_groove.mp3"));
  // track_no_art.mp3 is silent, which analysis rejects; the rest analyze.
  cpSync(join(fixturesDir, "audio/embedded"), join(src, "embedded"), { recursive: true });
  cpSync(join(fixturesDir, "audio/folder"), join(src, "folder"), { recursive: true });
  cpSync(join(fixturesDir, "audio/parent"), join(src, "parent"), { recursive: true });

  const json = join(work, "fixture.json");
  run("cargo", [
    "run", "-q", "--release", "-p", "backend", "--features", "dev-tools", "--bin", "dump_doc_gif_fixture",
    "--", join(work, "data"), src, EDITOR_TITLE, json,
  ], { cwd: repoDir });

  const fixture = JSON.parse(readFileSync(json, "utf8"));
  // The page can't read local files: inline artwork, and show short paths.
  const mime = { ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".png": "image/png" };
  for (const track of fixture.tracks) {
    if (track.artworkPath) {
      const type = mime[extname(track.artworkPath).toLowerCase()] || "image/jpeg";
      track.artworkDataUrl = `data:${type};base64,${readFileSync(track.artworkPath).toString("base64")}`;
    }
    track.artworkPath = null;
    track.filePath = `/music/${relative(src, track.filePath)}`;
    track.waveformPeaksPath = track.waveformPeaksPath && `/analysis/${basename(track.waveformPeaksPath)}`;
  }
  fixture.tracks.sort((a, b) => (a.title === EDITOR_TITLE ? -1 : b.title === EDITOR_TITLE ? 1 : 0));
  fixture.detail.track = fixture.tracks.find((t) => t.id === fixture.detail.track.id);
  return fixture;
}

// --- 2. Browser session + capture ---

async function openSession(browser, fixture, opts) {
  const ctx = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 1, colorScheme: "dark" });
  await ctx.addInitScript({
    content: `window.__DOC_GIF_FIXTURE__=${JSON.stringify(fixture)};window.__DOC_GIF_OPTS__=${JSON.stringify(opts)};`,
  });
  await ctx.addInitScript({ path: join(here, "page_init.js") });
  const page = await ctx.newPage();
  await page.goto(BASE_URL);
  const row = page.locator("#libraryTableBody .track-grid-row", { hasText: EDITOR_TITLE });
  await row.locator('.waveform-cell [data-action="edit-track-detail"]').click();
  await page.locator("#trackDetailOverlay").waitFor({ state: "visible" });
  await page.waitForTimeout(400);
  const wf = await page.locator("#trackDetailWaveform").boundingBox();
  return { ctx, page, wf };
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

// Captures PNG frames over CDP while `scene` runs; returns the frame list and
// the union of the editor dialog's boxes before and after (it grows with the
// cue list), for cropping.
async function capture(page, framesDir, scene) {
  rmSync(framesDir, { recursive: true, force: true });
  mkdirSync(framesDir, { recursive: true });
  const dialog = page.locator("#trackDetailOverlay > *").first();
  const boxes = [await dialog.boundingBox()];
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
  boxes.push(await dialog.boundingBox());
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
  ];
}

// --- main ---

const work = mkdtempSync(join(tmpdir(), "doc-gifs-"));
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
  for (const scene of scenes(fixture)) {
    if (only.length && !only.includes(scene.name)) continue;
    const { ctx, page, wf } = await openSession(browser, fixture, scene.opts);
    const framesDir = join(work, `frames-${scene.name}`);
    let captured = null;
    await scene.run(page, wf, async (body) => {
      captured = await capture(page, framesDir, body);
    });
    await ctx.close();
    encodeGif(captured, framesDir, join(outDir, `${scene.name}.gif`));
  }
} finally {
  await browser?.close();
  server?.kill();
  rmSync(work, { recursive: true, force: true });
}
