// PWV5 colour-detail waveform rendering for the track-detail editor.
//
// PWV5 entries are 2 bytes each, big-endian u16: R(3) | G(3) | B(3) | H(5) | _(2).
// Entry count is duration-derived (~150/sec), so a 4-minute track is ~36k
// entries — far more than the canvas is wide. `drawDetailWaveform` renders only
// the visible [startMs, endMs] slice so zoom shows real detail; a slice reaching
// past the track's ends (the centred playhead at its start or end) leaves that
// part blank. Heights are RMS-bucketed and normalised against the visible
// slice's own p05..p95 (with a gamma curve) — the same treatment the row
// preview gets — so the shape keeps its dynamics instead of maxing out on a
// loud master.

export function base64ToBytes(b64) {
  if (typeof b64 !== "string" || !b64) return new Uint8Array(0);
  let bin;
  try {
    bin = atob(b64);
  } catch {
    return new Uint8Array(0);
  }
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i += 1) out[i] = bin.charCodeAt(i);
  return out;
}

// The PWV5 colour itself: 3-bit R/G/B channels (0..7) -> 0..255, with a small
// lift so near-black entries still read. This is the real waveform colour (bass
// = red, mids = green, treble = blue), aggregated per pixel column below.
const LIFT = 18;
const SCALE = (255 - LIFT) / 7;
function chan(x) {
  return Math.round(LIFT + x * SCALE);
}

/// Whole-track amplitude reference: p05..p95 of every entry height. Computed
/// ONCE per payload so the vertical scale is fixed — zooming in is purely
/// horizontal, a quiet section stays visually quiet.
export function computeWaveNorm(bytes) {
  const n = bytes.length >>> 1;
  if (n === 0) return { lo: 0, hi: 1 };
  const stride = Math.max(1, Math.floor(n / 8192));
  const sample = [];
  for (let i = 0; i < n; i += stride) sample.push((((bytes[i * 2] << 8) | bytes[i * 2 + 1]) >> 2) & 0x1f);
  sample.sort((a, b) => a - b);
  const p = (q) => sample[Math.min(sample.length - 1, Math.floor(sample.length * q))] || 0;
  return { lo: p(0.05), hi: Math.max(p(0.95), p(0.05) + 1) };
}

/// Draw the [startMs, endMs] slice of a PWV5 payload onto the wrapper's canvas.
/// `norm` is the whole-track {lo, hi} from `computeWaveNorm` — the vertical
/// scale, kept fixed across zoom levels.
/// Returns false when the canvas has no measurable width yet (caller should
/// retry on rAF / resize); true otherwise.
export function drawDetailWaveform(wrapperEl, bytes, { startMs, endMs, durationMs, norm }) {
  if (!wrapperEl) return false;
  const canvas = wrapperEl.querySelector(".waveform-canvas-el");
  if (!canvas) return false;

  const rect = canvas.getBoundingClientRect();
  if (!rect.width || rect.width < 2) return false;
  const dpr = Math.max(1, window.devicePixelRatio || 1);
  const W = Math.max(1, Math.round(rect.width * dpr));
  const H = Math.max(1, Math.round(rect.height * dpr));
  if (canvas.width !== W || canvas.height !== H) {
    canvas.width = W;
    canvas.height = H;
  }

  const ctx = canvas.getContext("2d");
  if (!ctx) return true; // jsdom / no 2d context

  ctx.clearRect(0, 0, W, H);
  const n = bytes.length >>> 1;
  const dur = durationMs > 0 ? durationMs : 1;
  // The canvas columns the track covers; outside them the view is past its ends.
  const viewSpan = Math.max(1, endMs - startMs);
  const x0 = Math.max(0, Math.round(((0 - startMs) / viewSpan) * W));
  const x1 = Math.min(W, Math.round(((dur - startMs) / viewSpan) * W));
  if (x1 <= x0) return true;
  ctx.fillStyle = "rgba(128,140,160,0.14)";
  ctx.fillRect(x0, H - 1, x1 - x0, 1); // faint baseline
  if (n === 0) return true;

  // Columns are anchored to the track, not the view: column c always holds
  // the same entries -- one device pixel's worth when entries are narrower
  // than a pixel, else one entry -- and sits at c * colPx minus the view's
  // offset. Scrolling (playback moves the view a fraction of a pixel each
  // frame) then slides the same bars along instead of regrouping the entries
  // under each pixel, which made every bar change height from frame to frame.
  //
  // Bars sit on whole pixels, counted from the track's start: the view's
  // offset is rounded once, so every bar (and the beat grid, which rounds the
  // same way) moves a whole pixel at a time, together. Sliding by the
  // fraction instead softens 1-pixel bars into their gaps by an amount that
  // changes with the fraction, so the waveform pulsed while it moved.
  const startPx = (startMs / viewSpan) * W;
  const base = Math.round(startPx);
  const entryPx = (dur / n / viewSpan) * W;
  const perCol = Math.max(1, 1 / entryPx);
  const colPx = perCol * entryPx;
  const colCount = Math.ceil(n / perCol);
  const cFirst = Math.max(0, Math.floor(startPx / colPx));
  const cLast = Math.min(colCount, Math.ceil((startPx + W) / colPx));

  const at = (i) => (bytes[i * 2] << 8) | bytes[i * 2 + 1];
  const heightOf = (v) => (v >> 2) & 0x1f;
  const lo = norm?.lo ?? 0;
  const range = Math.max(1, (norm?.hi ?? 31) - lo);

  for (let c = cFirst; c < cLast; c += 1) {
    const s = Math.floor(c * perCol);
    const e = Math.min(n, Math.max(s + 1, Math.floor((c + 1) * perCol)));
    let sumSq = 0;
    let peak = 0;
    let sr = 0;
    let sg = 0;
    let sb = 0;
    let wsum = 0;
    for (let i = s; i < e; i += 1) {
      const v = at(i);
      const h = heightOf(v);
      sumSq += h * h;
      if (h > peak) peak = h;
      // Energy-weighted mean of the real PWV5 colours in this column.
      const w = h * h + 1;
      sr += ((v >> 13) & 7) * w;
      sg += ((v >> 10) & 7) * w;
      sb += ((v >> 7) & 7) * w;
      wsum += w;
    }
    const level = 0.7 * Math.sqrt(sumSq / (e - s)) + 0.3 * peak;
    const scaled = Math.pow(Math.max(0, Math.min(1, (level - lo) / range)), 0.8);
    if (scaled <= 0.001) continue;
    const barH = Math.max(2, Math.round(scaled * (H - 3)));
    const x = Math.max(x0, Math.round(c * colPx) - base);
    const xEnd = Math.min(x1, Math.round((c + 1) * colPx) - base);
    if (xEnd <= x) continue;
    ctx.fillStyle = `rgb(${chan(sr / wsum)},${chan(sg / wsum)},${chan(sb / wsum)})`;
    ctx.fillRect(x, H - barH, xEnd - x, barH);
  }
  return true;
}
