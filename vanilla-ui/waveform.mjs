// Waveform rendering: color derivation, peak parsing, canvas drawing.

export const WAVEFORM_COLORS = {
  base: "hsla(270, 82%, 55%, 0.55)",
  mid: "hsla(300, 82%, 62%, 0.92)",
  peak: "rgba(255,255,255,0.98)"
};

const waveformPeakCache = new WeakMap();
const waveformColorCache = new WeakMap();

export function invalidateWaveformCache(element) {
  if (!element) return;
  waveformPeakCache.delete(element);
  waveformColorCache.delete(element);
}

export function setWaveformColorData(element, data) {
  if (!element) return;
  if (Array.isArray(data) && data.length > 0) {
    waveformColorCache.set(element, data);
  } else {
    waveformColorCache.delete(element);
  }
}

function drawPwv4(ctx, data, width, height) {
  ctx.clearRect(0, 0, width, height);
  const n = Math.floor(data.length / 6);
  if (n === 0) return;
  for (let x = 0; x < width; x++) {
    const s = Math.floor((x * n) / width);
    const e = Math.max(s + 1, Math.floor(((x + 1) * n) / width));
    let maxD0 = 0;
    let r = 0;
    let g = 0;
    let b = 0;
    for (let i = s; i < e && i < n; i++) {
      const off = i * 6;
      const d0 = data[off];
      if (d0 > maxD0) {
        maxD0 = d0;
        r = data[off + 3];
        g = data[off + 4];
        b = data[off + 5];
      }
    }
    if (!maxD0) continue;
    const barH = Math.max(1, Math.round((maxD0 / 127) * (height - 2)));
    ctx.fillStyle = `rgb(${(r * 2) & 0xff},${(g * 2) & 0xff},${(b * 2) & 0xff})`;
    ctx.fillRect(x, height - barH, 1, barH);
  }
}

export function deriveWaveformColors(hue, isDark) {
  if (isDark) {
    return {
      base: `hsla(${hue}, 82%, 55%, 0.55)`,
      mid: `hsla(${(hue + 30) % 360}, 82%, 62%, 0.92)`,
      peak: "rgba(255,255,255,0.98)"
    };
  }
  return {
    base: `hsla(${hue}, 82%, 48%, 0.45)`,
    mid: `hsla(${(hue + 30) % 360}, 82%, 50%, 0.80)`,
    peak: "rgba(26,21,40,0.90)"
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

  const peaks = raw
    .split(",")
    .map((v) => Math.max(0, Math.min(100, Number(v) || 0)))
    .filter((v) => Number.isFinite(v));
  waveformPeakCache.set(element, peaks);
  return peaks;
}

export function drawWaveformCanvas(element) {
  if (!element) return;
  const canvas = element.querySelector(".waveform-canvas-el");
  if (!canvas) return;

  const rect = element.getBoundingClientRect();
  const dpr = Math.max(1, window.devicePixelRatio || 1);
  const width = Math.max(1, Math.round(rect.width * dpr));
  const height = Math.max(1, Math.round(rect.height * dpr));
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }

  const ctx = canvas.getContext("2d");
  if (!ctx) return;

  const colorData = waveformColorCache.get(element);
  if (colorData && colorData.length >= 6) {
    drawPwv4(ctx, colorData, width, height);
    return;
  }

  const peaks = getWaveformPeaksFromElement(element);
  ctx.clearRect(0, 0, width, height);
  if (!peaks.length) return;

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
    for (let i = start; i < end && i < peaks.length; i += 1) {
      const v = normalize(peaks[i]);
      sumSquares += v * v;
      count += 1;
    }
    const rms = count > 0 ? Math.sqrt(sumSquares / count) : 0;
    const barHeight = Math.max(1, Math.round(rms * (height - 2)));
    const y = height - barHeight;

    const gradient = ctx.createLinearGradient(0, y, 0, height);
    gradient.addColorStop(0, WAVEFORM_COLORS.mid);
    gradient.addColorStop(1, WAVEFORM_COLORS.base);
    ctx.fillStyle = gradient;
    ctx.fillRect(x, y, 1, barHeight);
    ctx.fillStyle = WAVEFORM_COLORS.peak;
    ctx.fillRect(x, y, 1, 1);
  }
}

export function renderWaveformsIn(root = document) {
  root.querySelectorAll(".waveform.waveform-canvas").forEach((waveform) => {
    drawWaveformCanvas(waveform);
  });
}
