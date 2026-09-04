// PWV5 colour-detail waveform rendering for the track-detail modal.
//
// PWV5 entries are 2 bytes each, big-endian u16: R(3) | G(3) | B(3) | H(5) | _(2).
// Entry count is duration-derived (~150/sec), so a 4-minute track is ~36k
// entries — far more than the canvas is wide. `drawDetailWaveform` renders only
// the visible [startMs, endMs] slice so zoom shows real detail. Heights are RMS-
// bucketed and normalised against the visible slice's own p05..p95 (with a
// gamma curve) — the same treatment the row preview gets — so the shape keeps
// its dynamics instead of maxing out on a loud master.

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
/// Returns false when the wrapper has no measurable width yet (caller should
/// retry on rAF / resize); true otherwise.
export function drawDetailWaveform(wrapperEl, bytes, { startMs, endMs, durationMs, norm }) {
  if (!wrapperEl) return false;
  const canvas = wrapperEl.querySelector(".waveform-canvas-el");
  if (!canvas) return false;

  const rect = wrapperEl.getBoundingClientRect();
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
  ctx.fillStyle = "rgba(128,140,160,0.14)";
  ctx.fillRect(0, H - 1, W, 1); // faint baseline

  const n = bytes.length >>> 1;
  const dur = durationMs > 0 ? durationMs : 1;
  if (n === 0) return true;

  let e0 = Math.floor((Math.max(0, startMs) / dur) * n);
  let e1 = Math.ceil((Math.min(dur, endMs) / dur) * n);
  e0 = Math.max(0, Math.min(n - 1, e0));
  e1 = Math.max(e0 + 1, Math.min(n, e1));
  const span = e1 - e0;

  const at = (i) => (bytes[i * 2] << 8) | bytes[i * 2 + 1];
  const heightOf = (v) => (v >> 2) & 0x1f;

  const cols = Math.min(W, span);
  const levels = new Float32Array(cols);
  const colR = new Float32Array(cols);
  const colG = new Float32Array(cols);
  const colB = new Float32Array(cols);
  for (let c = 0; c < cols; c += 1) {
    const s = e0 + Math.floor((c / cols) * span);
    const e = Math.max(s + 1, e0 + Math.floor(((c + 1) / cols) * span));
    let sumSq = 0;
    let cnt = 0;
    let peak = 0;
    let sr = 0;
    let sg = 0;
    let sb = 0;
    let wsum = 0;
    for (let i = s; i < e && i < n; i += 1) {
      const v = at(i);
      const h = heightOf(v);
      sumSq += h * h;
      cnt += 1;
      if (h > peak) peak = h;
      // Energy-weighted mean of the real PWV5 colours in this slice.
      const w = h * h + 1;
      sr += ((v >> 13) & 7) * w;
      sg += ((v >> 10) & 7) * w;
      sb += ((v >> 7) & 7) * w;
      wsum += w;
    }
    const rms = cnt ? Math.sqrt(sumSq / cnt) : 0;
    levels[c] = 0.7 * rms + 0.3 * peak;
    colR[c] = sr / wsum;
    colG[c] = sg / wsum;
    colB[c] = sb / wsum;
  }

  const lo = norm?.lo ?? 0;
  const range = Math.max(1, (norm?.hi ?? 31) - lo);
  const barW = W / cols;

  for (let c = 0; c < cols; c += 1) {
    const norm = Math.pow(Math.max(0, Math.min(1, (levels[c] - lo) / range)), 0.8);
    if (norm <= 0.001) continue;
    const barH = Math.max(2, Math.round(norm * (H - 3)));
    const x = Math.round(c * barW);
    const w = Math.max(1, Math.round((c + 1) * barW) - x);
    ctx.fillStyle = `rgb(${chan(colR[c])},${chan(colG[c])},${chan(colB[c])})`;
    ctx.fillRect(x, H - barH, w, barH);
  }
  return true;
}
