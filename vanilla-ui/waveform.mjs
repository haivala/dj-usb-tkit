// Waveform rendering: color derivation, peak parsing, canvas drawing.

// The theme's waveform colours. `hue` and `isDark` let a bar's colour move
// within the theme by the sound's frequency balance (waveformBarColors).
export const WAVEFORM_COLORS = {
  base: "hsla(270, 82%, 55%, 0.55)",
  mid: "hsla(300, 82%, 62%, 0.92)",
  peak: "rgba(255,255,255,0.98)",
  hue: 270,
  isDark: true
};

// The backend's row waveform: entries of four values, 0-127 -- the height,
// then the low, mid and high band -- read from the track's analysis file.
const ENTRY_VALUES = 4;

const waveformPeakCache = new WeakMap();

export function invalidateWaveformCache(element) {
  if (!element) return;
  waveformPeakCache.delete(element);
}

export function deriveWaveformColors(hue, isDark) {
  if (isDark) {
    return {
      base: `hsla(${hue}, 82%, 55%, 0.55)`,
      mid: `hsla(${(hue + 30) % 360}, 82%, 62%, 0.92)`,
      peak: "rgba(255,255,255,0.98)",
      hue,
      isDark
    };
  }
  return {
    base: `hsla(${hue}, 82%, 48%, 0.45)`,
    mid: `hsla(${(hue + 30) % 360}, 82%, 50%, 0.80)`,
    peak: "rgba(26,21,40,0.90)",
    hue,
    isDark
  };
}

// Where a sound sits between bass (0) and treble (1), from its three bands;
// null without any band (an analysis file with heights only).
export function bandTone(low, mid, high) {
  const total = low + mid + high;
  if (!total) return null;
  return (mid * 0.5 + high) / total;
}

// A bar's gradient colours: the theme's own for bass, turning up to 60
// degrees further round the hue and lighter (darker on a light theme) as
// the highs take over. Without bands, the theme's colours as they are.
export function waveformBarColors(tone) {
  if (tone === null) return { top: WAVEFORM_COLORS.mid, bottom: WAVEFORM_COLORS.base };
  const { hue, isDark } = WAVEFORM_COLORS;
  const h = (hue + Math.round(tone * 60)) % 360;
  const top = (h + 30) % 360;
  if (isDark) {
    return {
      top: `hsla(${top}, 82%, ${Math.round(62 + tone * 14)}%, 0.92)`,
      bottom: `hsla(${h}, 82%, ${Math.round(55 + tone * 12)}%, 0.55)`
    };
  }
  return {
    top: `hsla(${top}, 82%, ${Math.round(50 - tone * 10)}%, 0.80)`,
    bottom: `hsla(${h}, 82%, ${Math.round(48 - tone * 10)}%, 0.45)`
  };
}

export function getWaveformPeaksFromElement(element) {
  if (!element) return [];
  const cached = waveformPeakCache.get(element);
  if (cached) return cached;

  const raw = String(element.dataset.peaks || "");
  if (!raw) {
    waveformPeakCache.set(element, []);
    return [];
  }

  const peaks = raw.split(",").map(Number);
  waveformPeakCache.set(element, peaks);
  return peaks;
}

// Reads layout geometry only - no DOM writes. Call this for every element
// up front (in a batch) before calling paintMeasuredWaveform on any of them,
// so the reads don't interleave with the canvas-resize writes below and
// force a layout flush per element (layout thrashing) when painting many
// waveforms at once.
function measureWaveformCanvas(element) {
  const canvas = element.querySelector(".waveform-canvas-el");
  if (!canvas) return null;

  const rect = element.getBoundingClientRect();
  const dpr = Math.max(1, element.ownerDocument.defaultView.devicePixelRatio || 1);
  return {
    element,
    canvas,
    width: Math.max(1, Math.round(rect.width * dpr)),
    height: Math.max(1, Math.round(rect.height * dpr)),
  };
}

// Writes/draws only - safe to run for many elements in sequence without
// re-measuring layout in between.
function paintMeasuredWaveform({ element, canvas, width, height }) {
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }

  const ctx = canvas.getContext("2d");

  const values = getWaveformPeaksFromElement(element);
  ctx.clearRect(0, 0, width, height);
  const entries = Math.floor(values.length / ENTRY_VALUES);
  if (!entries) return;
  const peaks = Array.from({ length: entries }, (_, i) => values[i * ENTRY_VALUES]);

  const sorted = peaks.slice().sort((a, b) => a - b);
  const p05 = sorted[Math.floor((sorted.length - 1) * 0.05)] ?? 0;
  const p95 = sorted[Math.floor((sorted.length - 1) * 0.95)] ?? 100;
  const range = Math.max(1, p95 - p05);

  const normalize = (value) => {
    const n = Math.max(0, Math.min(1, (value - p05) / range));
    return Math.pow(n, 0.85);
  };

  for (let x = 0; x < width; x += 1) {
    const start = Math.floor((x * peaks.length) / width);
    const end = Math.max(start + 1, Math.floor(((x + 1) * peaks.length) / width));
    let sumSquares = 0;
    let count = 0;
    // The column's colour follows the bands of its loudest entry.
    let loudest = start;
    for (let i = start; i < end && i < peaks.length; i += 1) {
      const v = normalize(peaks[i]);
      sumSquares += v * v;
      count += 1;
      if (peaks[i] > peaks[loudest]) loudest = i;
    }
    const rms = count > 0 ? Math.sqrt(sumSquares / count) : 0;
    const barHeight = Math.max(1, Math.round(rms * (height - 2)));
    const y = height - barHeight;

    const at = loudest * ENTRY_VALUES;
    const colors = waveformBarColors(bandTone(values[at + 1], values[at + 2], values[at + 3]));
    const gradient = ctx.createLinearGradient(0, y, 0, height);
    gradient.addColorStop(0, colors.top);
    gradient.addColorStop(1, colors.bottom);
    ctx.fillStyle = gradient;
    ctx.fillRect(x, y, 1, barHeight);
    ctx.fillStyle = WAVEFORM_COLORS.peak;
    ctx.fillRect(x, y, 1, 1);
  }
}

export function drawWaveformCanvas(element) {
  if (!element) return;
  const measured = measureWaveformCanvas(element);
  if (!measured) return;
  paintMeasuredWaveform(measured);
}

const WAVEFORM_PAINT_CHUNK_SIZE = 200;

// The cue editor's waveform shares the classes but draws its own zoomed PWV5
// view; repainting it here would blank it.
export function renderWaveformsIn(root = document) {
  return renderWaveforms(root.querySelectorAll(".waveform.waveform-canvas:not(.track-detail-canvas)"));
}

// Paint the given `.waveform.waveform-canvas` elements (e.g. just the rows a
// track table has built, not the whole table).
export async function renderWaveforms(elements) {
  if (!elements.length) return;

  // Measure every element before painting any of them, so all the
  // layout-forcing getBoundingClientRect() reads happen before any of the
  // canvas-resize writes - avoids one forced layout flush per element.
  const measured = [];
  elements.forEach((element) => {
    const m = measureWaveformCanvas(element);
    if (m) measured.push(m);
  });

  for (let i = 0; i < measured.length; i += 1) {
    paintMeasuredWaveform(measured[i]);
    if ((i + 1) % WAVEFORM_PAINT_CHUNK_SIZE === 0 && i + 1 < measured.length) {
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
  }
}
