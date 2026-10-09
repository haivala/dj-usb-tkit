// Track-detail modal: cue points + beat-grid ("first beat") editing.
//
// This app targets CDJ playback directly, so a cue is just a position + name +
// colour. The list is capped at 8; on save each becomes a memory point + a
// hot-cue pad. The waveform is the full-detail PWV5 colour waveform with
// scroll-to-zoom and drag-to-pan (see waveform_detail.mjs).
//
// "Start the playback on first beat" adds one more cue on top of those: the
// playback-start cue (`playbackStart: true`), a memory point only (no pad, no
// colour), never later than any hot cue, so the CDJ loads there instead of on
// cue A. With no cues at all the CDJ already starts at the first audio, so
// the toggle is then disabled and shown checked, purely informational.

import { drawDetailWaveform, base64ToBytes, computeWaveNorm } from "./waveform_detail.mjs";
import { isUsbRootChangeBlocked } from "../usb/actions.mjs";
import { cloneTemplate } from "../../ui_utils.mjs";
import { coverElement } from "../../track_table.mjs";
import { buildCoverSrcCandidates, attachCoverFallbackHandlers } from "../library/actions.mjs";
import { formatBpm } from "../../track_utils.mjs";
import {
  STORAGE_KEY_CUE_START_ON_FIRST_BEAT,
  FRONTEND_DB_KEY_CUE_START_ON_FIRST_BEAT,
  STORAGE_KEY_CUE_BEATGRID_LEVEL,
  FRONTEND_DB_KEY_CUE_BEATGRID_LEVEL,
  STORAGE_KEY_CUE_QUANTIZE,
  FRONTEND_DB_KEY_CUE_QUANTIZE,
  STORAGE_KEY_CUE_METRONOME_MIX,
  FRONTEND_DB_KEY_CUE_METRONOME_MIX,
  STORAGE_KEY_CUE_FOLLOW_GRID,
  FRONTEND_DB_KEY_CUE_FOLLOW_GRID,
} from "../../settings_keys.mjs";

export const MAX_CUES = 8;
export const MIN_SPAN_MS = 1000;
export const DEFAULT_SPAN_MS = 60_000;
export const DEFAULT_VIEW_BARS = 60;

/// The view the editor opens on: the first 60 bars (4 beats each) at the
/// track's BPM, so beat lines are spaced the same on every track whatever
/// its tempo. The editor only opens for analysed tracks, which have a BPM.
export function defaultViewSpanMs(bpm) {
  return (DEFAULT_VIEW_BARS * 4 * 60_000) / Number(bpm);
}
const BPM_NUDGE_STEP = 0.01;
// Bar numbers (and bar lines) are shown every N bars, N the smallest of these
// that keeps them at least BAR_LABEL_MIN_PX (GRID_LINE_MIN_PX) apart.
const BAR_STEPS = [1, 2, 4, 8, 16, 32, 64];
const BAR_LABEL_MIN_PX = 36;
// Grid lines closer than this would hatch over the waveform. Ordinary beat
// lines are left out until zooming in spreads the beats this far apart; bar
// lines thin to every 2nd, 4th… bar (the bar-number steps) the same way.
const GRID_LINE_MIN_PX = 8;
const UNDO_LIMIT = 100;
// ←/→ with Shift, or without a beat grid: a fine nudge.
const FINE_NUDGE_MS = 10;

// Mirrors backend `HOTCUE_PALETTE` (service/cues.rs). id -> css colour.
export const HOTCUE_PALETTE = [
  { id: 1, css: "#DE44CF" },
  { id: 2, css: "#E12424" },
  { id: 3, css: "#E97A1E" },
  { id: 4, css: "#E3C71B" },
  { id: 5, css: "#4EB648" },
  { id: 6, css: "#1FADC4" },
  { id: 7, css: "#2A5BD8" },
  { id: 8, css: "#8A3FD1" },
];
const START_BEAT_TOOLTIP =
  "Adds a ▶ memory cue (no hot cue) on the first beat, so the CDJ loads there instead of on cue A";
const START_MARKER_TOOLTIP =
  "The CDJ loads on the ▶ start marker you placed; drag it on the waveform to move it";

export function colorCssForId(colorId) {
  return HOTCUE_PALETTE.find((c) => c.id === colorId)?.css || "#8892a0";
}

function formatMs(ms) {
  const total = Math.max(0, Math.round(ms));
  const m = Math.floor(total / 60000);
  const s = Math.floor((total % 60000) / 1000);
  const cs = Math.floor((total % 1000) / 10);
  return `${m}:${String(s).padStart(2, "0")}.${String(cs).padStart(2, "0")}`;
}

// m:ss, for the zoom-range readout (no centiseconds — it's a coarse orientation cue).
function formatClock(ms) {
  const total = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

let tempIdSeq = 0;

const raf = (cb) => (globalThis.requestAnimationFrame || ((f) => setTimeout(f, 16)))(cb);
const caf = (h) => (globalThis.cancelAnimationFrame || globalThis.clearTimeout)(h);
const nowMs = () => globalThis.performance?.now?.() ?? Date.now();

/// Position order; the playback-start cue wins a tie with a hot cue.
function byPosition(a, b) {
  return a.positionMs - b.positionMs || Number(!!b.playbackStart) - Number(!!a.playbackStart);
}

export function createTrackDetailController(el, prefs = {}) {
  const {
    getStartOnFirstBeatPref = () => false,
    setStartOnFirstBeatPref = () => {},
    getBeatgridLevelPref = () => 35,
    setBeatgridLevelPref = () => {},
    getQuantizePref = () => true,
    setQuantizePref = () => {},
    getFollowGridPref = () => false,
    setFollowGridPref = () => {},
    // The native engine mixes the clicks into the track (`set_playback_metronome`).
    setPlaybackMetronome = () => Promise.resolve(),
    getMetronomeMixPref = () => 50,
    setMetronomeMixPref = () => {},
  } = prefs;
  let resolveFn = null;
  let open = false;
  let resizeObserver = null;
  let playheadRafHandle = 0;
  let renderViewRafHandle = 0;
  let waveformRetryHandle = 0;
  let waveformRetries = 0;
  // What the overview canvas was last drawn for; it only changes with the
  // track or the width, not on every pan/zoom.
  let overviewDrawnKey = "";
  // What the key <select> was last built from.
  let keySelectBuiltFor = "";

  const working = {
    track: null,
    durationMs: 0,
    bpm: null,
    key: null,
    // The key picker's `{ label, keys }` groups, from the backend's track detail.
    keyGroups: [],
    firstBeatMs: null,
    cues: [],
    view: { startMs: 0, endMs: 0 },
    bytes: null, // decoded PWV5 Uint8Array
    waveNorm: null, // whole-track {lo, hi} amplitude reference (fixed across zoom)
    followSuspendUntil: 0,
    draggingTempId: null, // cue whose marker is being dragged on the waveform
    selectedTempId: null, // the cue ←/→ moves: last added, clicked, dragged or jumped to
  };
  let playPauseShowsPlaying = null;
  const openListeners = [];
  // The save payload as opened; anything else is an unsaved edit.
  let openedPayloadJson = "";
  // Its cue list alone: a save that didn't edit cues leaves a USB's cues as
  // they are (see `cuesEdited`).
  let openedCuesJson = "";
  // Undo/redo: snapshots of the editable state (see `mutate`).
  let undoStack = [];
  let redoStack = [];
  let lastMutateKey = null;
  let mutating = false;
  let dragSeq = 0;
  let nudgeSeq = 0;
  // Metronome: off on every open (a sound should never start by surprise).
  const metronome = { on: false, sent: "" };

  // --- Undo/redo ---------------------------------------------------------

  function snapshotState() {
    return JSON.stringify({
      bpm: working.bpm,
      key: working.key,
      firstBeatMs: working.firstBeatMs,
      cues: working.cues,
    });
  }

  function restoreState(json) {
    const s = JSON.parse(json);
    working.bpm = s.bpm;
    working.key = s.key;
    working.firstBeatMs = s.firstBeatMs;
    working.cues = s.cues;
    working.draggingTempId = null;
    if (!working.cues.some((c) => c.tempId === working.selectedTempId)) {
      working.selectedTempId = null;
    }
  }

  /// Run an edit and record the state before it for undo. Edits sharing a
  /// `key` in a row (typing one cue's name, one drag) are one undo step; an
  /// edit that changes nothing records nothing. Callers render themselves.
  function mutate(key, fn) {
    if (mutating) return fn();
    mutating = true;
    const before = snapshotState();
    let result;
    try {
      result = fn();
    } finally {
      mutating = false;
    }
    if (snapshotState() !== before) {
      if (!(key && key === lastMutateKey)) {
        undoStack.push(before);
        if (undoStack.length > UNDO_LIMIT) undoStack.shift();
      }
      redoStack = [];
      lastMutateKey = key || null;
      renderUndo();
    }
    return result;
  }

  function renderUndo() {
    if (el.trackDetailUndo) el.trackDetailUndo.disabled = !undoStack.length;
    if (el.trackDetailRedo) el.trackDetailRedo.disabled = !redoStack.length;
  }

  // --- Quantize ------------------------------------------------------------

  function hasGrid() {
    return beatIntervalMs() > 0 && working.firstBeatMs != null;
  }

  /// Whether this placement snaps: Quantize on, unless Shift (`free`) is held;
  /// Quantize off, only with Shift.
  function snaps(free) {
    return hasGrid() && getQuantizePref() !== !!free;
  }

  function snapToBeat(ms) {
    const interval = beatIntervalMs();
    const idx = Math.max(0, Math.round((ms - working.firstBeatMs) / interval));
    return working.firstBeatMs + idx * interval;
  }

  // --- Cues follow grid ---------------------------------------------------

  /// The grid a BPM / first-beat edit starts from, for `moveCuesWithGrid`.
  function gridBefore() {
    return {
      firstBeatMs: working.firstBeatMs,
      intervalMs: beatIntervalMs(),
      key: `${working.firstBeatMs}|${working.bpm}`,
    };
  }

  /// With "cues follow grid" on, move every cue from the `before` grid to the
  /// current one so it keeps its beat (fractional beats too: an off-grid cue
  /// keeps its offset). Off, cues keep their time in the audio, as in
  /// rekordbox. Each cue remembers the exact beat it was put on (`gridPin`:
  /// the beat, the position it gave and the grid it gave it on), so many small
  /// edits in a row don't drift from rounding. A pin only counts while both
  /// still match: any other move or grid edit drops it.
  function moveCuesWithGrid(before) {
    if (!getFollowGridPref()) return;
    if (before.firstBeatMs == null || !(before.intervalMs > 0) || !hasGrid()) return;
    const interval = beatIntervalMs();
    const maxMs = working.durationMs ? working.durationMs - 1 : Infinity;
    const key = `${working.firstBeatMs}|${working.bpm}`;
    for (const cue of working.cues) {
      const pin = cue.gridPin;
      const beat = pin && pin.positionMs === cue.positionMs && pin.grid === before.key
        ? pin.beat
        : (cue.positionMs - before.firstBeatMs) / before.intervalMs;
      cue.positionMs = Math.max(0, Math.min(maxMs, Math.round(working.firstBeatMs + beat * interval)));
      cue.gridPin = { beat, positionMs: cue.positionMs, grid: key };
    }
    // Rounding or the track's ends can put two hot cues on the same ms; the
    // later one steps 1 ms on.
    let previous = -1;
    for (const cue of hotCues().sort(byPosition)) {
      if (cue.positionMs <= previous) {
        cue.positionMs = previous + 1;
        cue.gridPin.positionMs = cue.positionMs;
      }
      previous = cue.positionMs;
    }
    enforceStartOrder();
  }

  function renderTools() {
    el.trackDetailQuantize?.setAttribute("aria-pressed", String(getQuantizePref()));
    el.trackDetailFollowGrid?.setAttribute("aria-pressed", String(getFollowGridPref()));
    el.trackDetailMetronome?.setAttribute("aria-pressed", String(metronome.on));
    const mix = el.trackDetailMetronomeMix;
    if (mix) {
      mix.value = String(getMetronomeMixPref());
      // The Mix slider only matters while the metronome is on.
      const wrap = mix.closest("label");
      if (wrap) wrap.hidden = !metronome.on;
    }
  }

  // --- Metronome -----------------------------------------------------------

  /// Tell the engine the metronome state and grid whenever either changes
  /// (toggle, BPM/first-beat edits, undo, open/close). The clicks are mixed
  /// into the track's own samples, so they sit exactly on the heard beat.
  function syncMetronome() {
    const request = {
      enabled: open && metronome.on && hasGrid(),
      firstBeatMs: working.firstBeatMs ?? 0,
      bpm: Number(working.bpm) || 0,
      mix: getMetronomeMixPref() / 100,
    };
    const key = JSON.stringify(request);
    if (key === metronome.sent) return;
    metronome.sent = key;
    Promise.resolve(setPlaybackMetronome(request)).catch(() => {});
  }

  /// Cues with a hot-cue pad: neither the playback start nor a memory cue.
  function hotCues() {
    return working.cues.filter((c) => !c.playbackStart && !c.memory);
  }

  /// Every cue except the playback-start cue (which `withoutStartCue` drops).
  function withoutStartCue() {
    return working.cues.filter((c) => !c.playbackStart);
  }

  function startCue() {
    return working.cues.find((c) => c.playbackStart) || null;
  }

  function orderedCues() {
    return working.cues.slice().sort(byPosition);
  }

  /// The cues the editor shows: memory cues (e.g. from rekordbox) are kept
  /// and saved as they are, but not shown or edited here yet.
  function shownCues() {
    return orderedCues().filter((c) => !c.memory);
  }

  /// The playback-start cue never lies after a hot cue, and never exists
  /// without one.
  function enforceStartOrder() {
    const start = startCue();
    if (!start) return;
    const hot = hotCues();
    if (!hot.length) {
      working.cues = withoutStartCue();
      return;
    }
    const earliest = Math.min(...hot.map((c) => c.positionMs));
    if (start.positionMs > earliest) start.positionMs = earliest;
  }

  function addStartCue() {
    if (startCue()) return;
    working.cues.push({
      tempId: `c${(tempIdSeq += 1)}`,
      positionMs: working.firstBeatMs == null ? 0 : working.firstBeatMs,
      colorId: null,
      name: "",
      playbackStart: true,
      followsFirstBeat: true,
    });
    enforceStartOrder();
  }

  function beatIntervalMs() {
    const bpm = Number(working.bpm) || 0;
    return bpm > 0 ? 60000 / bpm : 0;
  }

  function viewSpanMs() {
    return Math.max(1, working.view.endMs - working.view.startMs);
  }

  // Unclamped: callers cull out-of-view items themselves.
  function msToPct(ms) {
    return ((ms - working.view.startMs) / viewSpanMs()) * 100;
  }

  function applyView(startMs, span) {
    const dur = working.durationMs || 1;
    const clampedSpan = Math.max(MIN_SPAN_MS, Math.min(span, dur));
    const clampedStart = Math.max(0, Math.min(startMs, dur - clampedSpan));
    working.view = { startMs: clampedStart, endMs: clampedStart + clampedSpan };
  }

  function scheduleRenderView() {
    if (renderViewRafHandle) return;
    renderViewRafHandle = raf(() => {
      renderViewRafHandle = 0;
      renderView();
    });
  }

  /// The live playhead fraction (0..1, whole-track) the shared playback module
  /// writes on the modal waveform, or 0 when nothing has played.
  function playheadFullRatio() {
    const wf = el.trackDetailWaveform;
    const win = wf?.ownerDocument?.defaultView;
    if (!wf || !win?.getComputedStyle) return 0;
    const styles = win.getComputedStyle(wf);
    const pct = parseFloat(styles.getPropertyValue("--playhead-position"));
    if (Number.isFinite(pct) && pct > 0) return Math.min(1, pct / 100);
    const x = parseFloat(styles.getPropertyValue("--playhead-x"));
    const w = wf.clientWidth || 0;
    if (Number.isFinite(x) && w > 0) return Math.min(1, Math.max(0, x / w));
    return 0;
  }

  function renderWaveform() {
    const wf = el.trackDetailWaveform;
    if (!wf || !open) return;
    if (!working.bytes) {
      working.bytes = base64ToBytes(working.track?.detailWaveform);
      working.waveNorm = computeWaveNorm(working.bytes);
    }
    const ok = drawDetailWaveform(wf, working.bytes, {
      startMs: working.view.startMs,
      endMs: working.view.endMs,
      durationMs: working.durationMs,
      norm: working.waveNorm,
    });
    renderOverviewCanvas();
    if (ok) {
      waveformRetries = 0;
    } else if (!waveformRetryHandle && waveformRetries < 20) {
      waveformRetries += 1;
      waveformRetryHandle = raf(() => {
        waveformRetryHandle = 0;
        renderWaveform();
      });
    }
  }

  function renderBeatgrid() {
    const host = el.trackDetailBeatgrid;
    if (!host) return;
    const level = getBeatgridLevelPref();
    el.trackDetailWaveform?.style.setProperty("--grid-level", String(level / 100));
    if (el.trackDetailGridLevel) el.trackDetailGridLevel.value = String(level);
    host.textContent = "";
    const interval = beatIntervalMs();
    if (!interval || !working.durationMs || working.firstBeatMs == null) return;
    const from = Math.max(working.firstBeatMs, working.view.startMs - interval);
    const to = Math.min(working.durationMs, working.view.endMs + interval);
    const width = el.trackDetailWaveform?.clientWidth || 0;
    const labelStep = barStep(interval, BAR_LABEL_MIN_PX);
    const lineStep = width ? barStep(interval, GRID_LINE_MIN_PX) : 1;
    const showBeats = !width || (interval / viewSpanMs()) * width >= GRID_LINE_MIN_PX;
    // Lines sit on whole device pixels: at a fractional position the browser
    // smears some across two pixels and not others, so an even grid looks
    // uneven (and its spacing jitters). The % fallback is for an unlaid-out host.
    const dpr = (typeof window !== "undefined" && window.devicePixelRatio) || 1;
    // Widths in whole device pixels too (at least one per CSS px on HiDPI),
    // thicker as the "Beat grid" slider goes up; centred on the line's pixel.
    const dpx = Math.max(1, Math.round(dpr)) / dpr;
    const setWidth = (name, devicePx) => {
      host.style.setProperty(`--${name}-w`, `${devicePx * dpx}px`);
      host.style.setProperty(`--${name}-ml`, `${-Math.floor(devicePx / 2) * dpx}px`);
    };
    setWidth("beat", 1 + Math.round((level / 100) * 2));
    setWidth("bar", 2 + Math.round((level / 100) * 2));
    host.style.setProperty("--dpx", `${dpx}px`);
    const lineLeft = (t) => width
      ? `${Math.round(((t - working.view.startMs) / viewSpanMs()) * width * dpr) / dpr}px`
      : `${msToPct(t)}%`;
    // Snap `from` to the nearest grid line at or before it.
    const firstBeatIdx = Math.max(0, Math.floor((from - working.firstBeatMs) / interval));
    let safety = 0;
    for (let idx = firstBeatIdx; ; idx += 1) {
      const t = working.firstBeatMs + idx * interval;
      if (t > to || safety > 8000) break;
      safety += 1;
      const downbeat = idx % 4 === 0;
      const bar = idx / 4; // 0-based
      if (downbeat ? bar % lineStep !== 0 : !showBeats) continue;
      const line = cloneTemplate(host.ownerDocument, "tplBeatgridLine");
      if (downbeat) line.classList.add("is-downbeat");
      const left = lineLeft(t);
      line.style.left = left;
      host.appendChild(line);
      // Steps are powers of two and labels need more room, so every
      // labelled bar also has its line.
      if (downbeat && bar % labelStep === 0) {
        const label = cloneTemplate(host.ownerDocument, "tplBeatgridBar");
        label.style.left = left;
        label.textContent = String(bar + 1);
        host.appendChild(label);
      }
    }
  }

  /// Every Nth bar, N the smallest step keeping those bars at least `minPx`
  /// apart -- for bar numbers (1, 5, 9… when zoomed out) and bar lines.
  function barStep(interval, minPx) {
    const width = el.trackDetailWaveform?.clientWidth || 0;
    const pxPerBar = width ? ((4 * interval) / viewSpanMs()) * width : 0;
    if (!pxPerBar) return 4;
    return BAR_STEPS.find((n) => n * pxPerBar >= minPx) ?? BAR_STEPS[BAR_STEPS.length - 1];
  }

  /// The whole-track strip under the waveform: drawn once per track/width.
  function renderOverviewCanvas() {
    const host = el.trackDetailOverview;
    if (!host || !working.bytes || !working.durationMs) return;
    const key = `${working.track?.id ?? ""}:${host.clientWidth}:${working.bytes.length}`;
    if (key === overviewDrawnKey) return;
    const ok = drawDetailWaveform(host, working.bytes, {
      startMs: 0,
      endMs: working.durationMs,
      durationMs: working.durationMs,
      norm: working.waveNorm,
    });
    if (ok) overviewDrawnKey = key;
  }

  /// The visible-window box and cue ticks on the overview strip.
  function renderOverview() {
    const dur = working.durationMs || 0;
    const box = el.trackDetailOverviewWindow;
    if (box && dur) {
      const left = (working.view.startMs / dur) * 100;
      box.style.left = `${left}%`;
      box.style.width = `${Math.min(100 - left, (viewSpanMs() / dur) * 100)}%`;
    }
    const cuesHost = el.trackDetailOverviewCues;
    if (!cuesHost) return;
    cuesHost.textContent = "";
    if (!dur) return;
    for (const cue of shownCues()) {
      const tick = cloneTemplate(cuesHost.ownerDocument, "tplOverviewCue");
      if (cue.playbackStart) tick.classList.add("is-playback-start");
      tick.style.left = `${(cue.positionMs / dur) * 100}%`;
      if (!cue.playbackStart) tick.style.setProperty("--cue-color", colorCssForId(cue.colorId));
      cuesHost.appendChild(tick);
    }
  }

  /// The usage hints show until the track has a cue; then they fold into "?".
  function renderHint() {
    const hasCues = hotCues().length > 0;
    if (el.trackDetailHint) el.trackDetailHint.hidden = hasCues;
    if (el.trackDetailHintBtn) el.trackDetailHintBtn.hidden = !hasCues;
  }

  function renderMarkers() {
    const host = el.trackDetailCueMarkers;
    if (!host) return;
    host.textContent = "";
    const labels = cueLabels();
    for (const cue of shownCues()) {
      const pct = msToPct(cue.positionMs);
      const marker = cloneTemplate(host.ownerDocument, "tplCueMarker");
      marker.classList.toggle("is-playback-start", !!cue.playbackStart);
      marker.classList.toggle("off-view", pct < -2 || pct > 102);
      marker.classList.toggle("is-dragging", cue.tempId === working.draggingTempId);
      marker.classList.toggle("is-selected", cue.tempId === working.selectedTempId);
      marker.style.left = `${pct}%`;
      if (!cue.playbackStart) marker.style.setProperty("--cue-color", colorCssForId(cue.colorId));
      marker.dataset.tempId = cue.tempId;
      marker.textContent = labels.get(cue.tempId);
      marker.dataset.tooltip = cue.playbackStart
        ? `Playback start · ${formatMs(cue.positionMs)}`
        : `${marker.textContent} · ${formatMs(cue.positionMs)}`;
      host.appendChild(marker);
    }
    renderPreStart();
    renderOverview();
  }

  /// Grey out the waveform before where the CDJ starts playback: the
  /// playback-start cue, else the first hot cue (the start cue is never later).
  /// With no cues the CDJ starts at the first audio, so nothing is greyed.
  function renderPreStart() {
    const shade = el.trackDetailPreStart;
    if (!shade) return;
    const first = shownCues()[0];
    const pct = first ? Math.max(0, Math.min(100, msToPct(first.positionMs))) : 0;
    shade.hidden = pct <= 0;
    shade.style.width = `${pct}%`;
  }

  // Both flags are the shared playback module's projection of backend state
  // onto this waveform (`setWaveformPlayhead`); the modal keeps no copy.
  function isPlaying() {
    return !!el.trackDetailWaveform?.classList.contains("is-playing");
  }

  function isPaused() {
    return !!el.trackDetailWaveform?.classList.contains("is-paused");
  }

  /// Where "now" is: the live playhead, or the backend's paused position.
  function currentPositionMs() {
    return isPlaying() || isPaused() ? playheadFullRatio() * working.durationMs : 0;
  }

  function renderPlayPause() {
    const btn = el.trackDetailPlayPause;
    const playing = isPlaying();
    if (!btn || playing === playPauseShowsPlaying) return;
    playPauseShowsPlaying = playing;
    const label = playing ? "Pause" : "Play";
    btn.classList.toggle("is-playing", playing);
    btn.setAttribute("aria-label", label);
    btn.dataset.tooltip = label;
    btn.replaceChildren(cloneTemplate(btn.ownerDocument, playing ? "tplIconPause" : "tplIconPlay"));
  }

  function positionModalPlayhead() {
    const ph = el.trackDetailPlayhead;
    if (!ph) return;
    const posMs = currentPositionMs();
    if (posMs <= 0) {
      ph.hidden = true;
      return;
    }
    const pct = msToPct(posMs);
    ph.hidden = pct < 0 || pct > 100;
    ph.style.left = `${pct}%`;
  }

  function playheadTick() {
    const wf = el.trackDetailWaveform;
    renderPlayPause();
    if (!open || !wf || !wf.classList.contains("is-playing")) {
      playheadRafHandle = 0;
      // Paused: the playhead stays at the backend's position; stopped: hidden.
      positionModalPlayhead();
      return;
    }
    const posMs = playheadFullRatio() * working.durationMs;
    // Follow: keep the playhead in view when zoomed in, unless the user just
    // panned/zoomed manually.
    if (
      posMs > 0 &&
      viewSpanMs() < working.durationMs &&
      nowMs() > working.followSuspendUntil
    ) {
      const span = viewSpanMs();
      if (posMs > working.view.startMs + span * 0.85 || posMs < working.view.startMs) {
        applyView(posMs - span * 0.3, span);
        renderView();
      }
    }
    positionModalPlayhead();
    playheadRafHandle = raf(playheadTick);
  }

  /// Hot cues are lettered A–H by position, the same on the waveform markers
  /// and in the cue list; the playback-start cue is "▶".
  function cueLabels() {
    const labels = new Map();
    let hotIndex = 0;
    for (const cue of shownCues()) {
      labels.set(cue.tempId, cue.playbackStart ? "▶" : String.fromCharCode(65 + hotIndex++));
    }
    return labels;
  }

  function cueRow(cue, letter) {
    const doc = el.trackDetailCueList.ownerDocument;
    const row = cloneTemplate(doc, cue.playbackStart ? "tplCueRowStart" : "tplCueRow");
    row.classList.toggle("is-selected", cue.tempId === working.selectedTempId);
    row.dataset.tempId = cue.tempId;
    row.querySelector(".cue-row-pos").textContent = formatMs(cue.positionMs);

    if (cue.playbackStart) {
      // Not nameable, and removed only by the "Start the playback…" toggle.
      row.querySelector(".cue-row-memory").textContent = letter;
      return row;
    }

    const swatch = row.querySelector(".cue-row-color");
    swatch.style.background = colorCssForId(cue.colorId);
    swatch.textContent = letter;
    swatch.setAttribute("aria-label", `Cue ${letter} colour`);
    row.querySelector(".cue-row-name").value = cue.name || "";
    return row;
  }

  /// A free A–H slot. Letters follow position, so it only marks room for one
  /// more cue; it isn't a pad that can be filled directly.
  function emptySlot(letter) {
    const slot = cloneTemplate(el.trackDetailCueList.ownerDocument, "tplCueSlotEmpty");
    slot.querySelector(".cue-row-color").textContent = letter;
    return slot;
  }

  /// Always all eight A–H slots, so the modal keeps its height as cues come
  /// and go. The playback-start cue sits in the "Playback starts at" row.
  function renderCueList() {
    const host = el.trackDetailCueList;
    if (!host) return;
    host.textContent = "";
    const hot = hotCues().sort(byPosition);
    const labels = cueLabels();
    for (const cue of hot) host.appendChild(cueRow(cue, labels.get(cue.tempId)));
    for (let i = hot.length; i < MAX_CUES; i++) {
      host.appendChild(emptySlot(String.fromCharCode(65 + i)));
    }
    const startHost = el.trackDetailStartCue;
    if (startHost) {
      startHost.textContent = "";
      const start = startCue();
      if (start) startHost.appendChild(cueRow(start, labels.get(start.tempId)));
      startHost.hidden = !start;
    }
    if (el.trackDetailAddCue) el.trackDetailAddCue.disabled = hot.length >= MAX_CUES;
  }

  /// "Playback starts at [First cue | First beat]". The second choice reads
  /// "Start marker" once the start cue sits off the first beat (dragged, or
  /// pulled back by a hot cue). With no cues neither applies: the CDJ starts
  /// at the first audio, which the note says.
  function renderStartChoice() {
    const cueBtn = el.trackDetailStartFirstCue;
    const beatBtn = el.trackDetailStartFirstBeat;
    if (!cueBtn || !beatBtn) return;
    const hasCues = hotCues().length > 0;
    const start = startCue();
    const firstBeat = working.firstBeatMs == null ? 0 : working.firstBeatMs;
    const moved = !!start && start.positionMs !== firstBeat;
    cueBtn.disabled = beatBtn.disabled = !hasCues;
    cueBtn.setAttribute("aria-checked", String(hasCues && !start));
    beatBtn.setAttribute("aria-checked", String(hasCues && !!start));
    beatBtn.textContent = moved ? "Start marker" : "First beat";
    beatBtn.dataset.tooltip = moved ? START_MARKER_TOOLTIP : START_BEAT_TOOLTIP;
    if (el.trackDetailStartNote) el.trackDetailStartNote.hidden = hasCues;
  }

  // The modal opens zoomed to the first 60 bars, so make it unmistakable that
  // the waveform is a window, not the whole track: show the visible span vs the
  // track length ("0:00–2:00 of 5:34"), accented while zoomed, and point at Fit.
  function renderZoomHint() {
    const out = el.trackDetailZoomRange;
    if (!out) return;
    const dur = working.durationMs || 0;
    const zoomed = dur > 0 && viewSpanMs() < dur - 1;
    out.classList.toggle("is-zoomed", zoomed);
    const total = el.trackDetailTotalTime;
    if (total) {
      total.hidden = !dur;
      total.textContent = dur ? formatClock(dur) : "";
    }
    if (!dur) {
      out.hidden = true;
      return;
    }
    out.hidden = false;
    out.textContent = zoomed
      ? `${formatClock(working.view.startMs)}–${formatClock(working.view.endMs)}`
      : "Whole track";
  }

  function renderView() {
    if (!open) return;
    renderWaveform();
    renderBeatgrid();
    renderMarkers();
    positionModalPlayhead();
    renderZoomHint();
    renderPlayPause();
  }

  /// The backend's key option values in stepper order (majors, then minors).
  function keyOptions() {
    return working.keyGroups.flatMap((g) => g.keys.map((k) => k.value));
  }

  /// Build the key <select> from the backend's groups (once per change).
  function buildKeySelect(select) {
    const built = JSON.stringify(working.keyGroups);
    if (built === keySelectBuiltFor) return;
    keySelectBuiltFor = built;
    select.textContent = "";
    const { Option } = select.ownerDocument.defaultView;
    for (const group of working.keyGroups) {
      const optgroup = cloneTemplate(select.ownerDocument, "tplOptgroup");
      optgroup.label = group.label;
      // `value` is the classic key a save sends; `label` is it in the
      // user's key notation (both from the backend).
      for (const key of group.keys) optgroup.append(new Option(key.label, key.value));
      select.appendChild(optgroup);
    }
  }

  // A stored key can predate this stepper (e.g. essentia's flat spellings)
  // and won't match any of the backend's key options -- rather than silently
  // dropping it, keep it visible via a synthetic option so the select always
  // reflects the real current value.
  function syncKeySelect() {
    const select = el.trackDetailKey;
    if (!select) return;
    buildKeySelect(select);
    const synthetic = select.querySelector("option[data-synthetic]");
    if (working.key != null && !keyOptions().includes(working.key)) {
      const option = synthetic || new select.ownerDocument.defaultView.Option();
      option.value = working.key;
      // The backend's label for the stored key when it's still the row's key.
      option.textContent = working.key === working.track?.key && working.track?.keyDisplay
        ? working.track.keyDisplay
        : working.key;
      option.dataset.synthetic = "1";
      if (!synthetic) select.prepend(option);
    } else if (synthetic) {
      synthetic.remove();
    }
    if (working.key != null) select.value = working.key;
  }

  function render() {
    if (!open) return;
    if (el.trackDetailFirstBeatMs) {
      el.trackDetailFirstBeatMs.value =
        working.firstBeatMs == null ? "" : String(working.firstBeatMs);
    }
    if (el.trackDetailBpm) {
      // Two decimals, as in the track list (the ▲/▼ step is 0.01).
      el.trackDetailBpm.value = working.bpm == null ? "" : formatBpm(working.bpm);
    }
    syncKeySelect();
    renderView();
    renderCueList();
    renderStartChoice();
    renderHint();
    renderTools();
    renderUndo();
    syncMetronome();
  }

  const api = {
    isOpen: () => open,
    getWorking: () => working,
    getView: () => ({ ...working.view }),
    isPlaying,
    isPaused,
    render,
    beatIntervalMs,

    viewRatioToTrackRatio(ratio) {
      const trackMs = working.view.startMs + Math.max(0, Math.min(1, ratio)) * viewSpanMs();
      return working.durationMs > 0 ? trackMs / working.durationMs : 0;
    },

    setView(startMs, span, { suspendFollow = true } = {}) {
      applyView(startMs, span);
      if (suspendFollow) working.followSuspendUntil = nowMs() + 1000;
      scheduleRenderView();
    },

    zoomAt(ratio, factor) {
      const span = viewSpanMs();
      const anchor = working.view.startMs + Math.max(0, Math.min(1, ratio)) * span;
      const newSpan = span * factor;
      api.setView(anchor - Math.max(0, Math.min(1, ratio)) * newSpan, newSpan);
    },

    panByMs(deltaMs) {
      api.setView(working.view.startMs + deltaMs, viewSpanMs());
    },

    fitView() {
      api.setView(0, working.durationMs || DEFAULT_SPAN_MS);
    },

    /// Called after every open, once the modal is shown.
    onOpened(listener) {
      openListeners.push(listener);
    },

    notePlaybackStarted() {
      if (!playheadRafHandle) playheadRafHandle = raf(playheadTick);
    },

    setFirstBeatMs(ms) {
      mutate(null, () => {
        const before = gridBefore();
        const clamped = Math.max(0, Math.round(Number(ms) || 0));
        working.firstBeatMs = working.durationMs
          ? Math.min(clamped, working.durationMs - 1)
          : clamped;
        // Before the start-cue rule below: an untouched start cue sits on
        // the first beat either way, and must not move twice.
        moveCuesWithGrid(before);
        // An untouched playback-start cue follows the first beat.
        const start = startCue();
        if (start?.followsFirstBeat) {
          start.positionMs = working.firstBeatMs;
          enforceStartOrder();
        }
      });
      render();
    },

    /// Move the view (same zoom) so it is centred on a whole-track ratio.
    centerViewAtRatio(ratio) {
      const span = viewSpanMs();
      const at = Math.max(0, Math.min(1, ratio)) * (working.durationMs || 0);
      api.setView(at - span / 2, span);
    },

    nudgeFirstBeat(direction) {
      const interval = beatIntervalMs();
      if (!interval) return;
      const base = working.firstBeatMs == null ? 0 : working.firstBeatMs;
      api.setFirstBeatMs(base + direction * interval);
    },

    setBpm(bpm) {
      const parsed = Number.parseFloat(bpm);
      if (!Number.isFinite(parsed) || parsed <= 0) return;
      mutate(null, () => {
        const before = gridBefore();
        working.bpm = Math.max(0.01, Math.round(Math.min(999, parsed) * 100) / 100);
        moveCuesWithGrid(before);
      });
      render();
    },

    /// ×2 / ÷2: fix a half- or double-tempo analysis in one step.
    scaleBpm(factor) {
      if (working.bpm == null) return;
      api.setBpm(working.bpm * factor);
    },

    nudgeBpm(direction) {
      const base = working.bpm == null ? 0 : working.bpm;
      api.setBpm(base + direction * BPM_NUDGE_STEP);
    },

    setKey(key) {
      const trimmed = typeof key === "string" ? key.trim() : "";
      mutate(null, () => {
        working.key = trimmed ? trimmed.slice(0, 16) : null;
      });
      render();
    },

    /// Step to the next/previous key option. A current value outside the
    /// list (a legacy/non-canonical stored key) jumps onto the nearest end
    /// instead of stepping relative to a position it doesn't have.
    nudgeKey(direction) {
      const options = keyOptions();
      const count = options.length;
      if (!count) return;
      const index = working.key == null ? -1 : options.indexOf(working.key);
      const nextIndex = index === -1
        ? (direction > 0 ? 0 : count - 1)
        : (index + direction + count) % count;
      api.setKey(options[nextIndex]);
    },

    /// Add a cue. With an explicit `positionMs` it lands there (double-click on
    /// the waveform); with no argument it lands at the current playhead ("+ Cue"),
    /// or where playback was paused.
    /// Name and colour default to "Cue N" / the Nth palette colour (N = 1-based
    /// add order) — assigned once at creation, never renumbered later, and
    /// always user-editable afterward.
    /// The first cue on a track also applies the remembered "Start the
    /// playback on first beat" setting.
    /// With Quantize on it lands on the nearest beat; `free` (Shift) inverts that.
    addCue(positionMs, { free = false } = {}) {
      const ordinal = hotCues().length;
      if (ordinal >= MAX_CUES) return null;
      const dur = working.durationMs || 0;
      let ms = Number.isFinite(positionMs) ? positionMs : currentPositionMs();
      if (snaps(free)) ms = snapToBeat(ms);
      const cue = {
        tempId: `c${(tempIdSeq += 1)}`,
        positionMs: Math.max(0, Math.min(dur, Math.round(ms))),
        colorId: HOTCUE_PALETTE[ordinal % HOTCUE_PALETTE.length].id,
        name: `Cue ${ordinal + 1}`,
      };
      mutate(null, () => {
        working.cues.push(cue);
        if (ordinal === 0 && getStartOnFirstBeatPref()) addStartCue();
        enforceStartOrder();
      });
      working.selectedTempId = cue.tempId;
      render();
      return cue;
    },

    /// The "Q" toggle (remembered): snap new and dragged cues to the grid.
    toggleQuantize() {
      setQuantizePref(!getQuantizePref());
      renderTools();
    },

    /// The "cues follow grid" toggle (remembered): BPM and first-beat edits
    /// move every cue with the grid instead of leaving it in place.
    toggleFollowGrid() {
      setFollowGridPref(!getFollowGridPref());
      renderTools();
    },

    /// The metronome toggle: the engine clicks on the grid while playing.
    toggleMetronome() {
      metronome.on = !metronome.on;
      renderTools();
      syncMetronome();
    },

    /// Only toggles classes: re-rendering the list would destroy a name
    /// input being focused, or the swatch a colour popover is anchored to.
    selectCue(tempId) {
      if (!working.cues.some((c) => c.tempId === tempId)) return;
      working.selectedTempId = tempId;
      for (const host of [el.trackDetailCueMarkers, el.trackDetailCueList, el.trackDetailStartCue]) {
        for (const node of host?.querySelectorAll("[data-temp-id]") || []) {
          node.classList.toggle("is-selected", node.dataset.tempId === tempId);
        }
      }
    },

    /// Hot cue by letter order (0 = A); null when there is no such cue.
    hotCueAt(index) {
      return shownCues().filter((c) => !c.playbackStart)[index] || null;
    },

    /// ←/→: move the selected cue one beat (onto the next grid line with
    /// Quantize on), or FINE_NUDGE_MS with `fine` (Shift) or without a grid.
    /// Each press is an undo step; a held key's auto-repeat joins its press.
    nudgeSelectedCue(direction, { fine = false, repeat = false } = {}) {
      const cue = working.cues.find((c) => c.tempId === working.selectedTempId);
      if (!cue) return false;
      if (!repeat) nudgeSeq += 1;
      const interval = beatIntervalMs();
      let ms;
      if (fine || !hasGrid()) {
        ms = cue.positionMs + direction * FINE_NUDGE_MS;
      } else if (getQuantizePref()) {
        // Positions are whole ms, so a cue "on" a beat can sit a fraction of a
        // ms off it: within 1.5 ms counts as on the beat.
        const idx = (cue.positionMs - working.firstBeatMs) / interval;
        const nearest = Math.round(idx);
        const onBeat = Math.abs(idx - nearest) * interval < 1.5;
        const next = onBeat
          ? nearest + direction
          : direction > 0 ? Math.floor(idx) + 1 : Math.ceil(idx) - 1;
        ms = working.firstBeatMs + Math.max(0, next) * interval;
      } else {
        ms = cue.positionMs + direction * interval;
      }
      const dur = working.durationMs || 0;
      mutate(`nudge:${nudgeSeq}`, () => {
        cue.positionMs = Math.max(0, Math.min(dur, Math.round(ms)));
        if (cue.playbackStart) cue.followsFirstBeat = false;
        enforceStartOrder();
      });
      render();
      return true;
    },

    canUndo: () => undoStack.length > 0,

    undo() {
      if (!undoStack.length) return false;
      redoStack.push(snapshotState());
      restoreState(undoStack.pop());
      lastMutateKey = null;
      render();
      return true;
    },

    redo() {
      if (!redoStack.length) return false;
      undoStack.push(snapshotState());
      restoreState(redoStack.pop());
      lastMutateKey = null;
      render();
      return true;
    },

    /// The "Beat grid" slider (0-100): how strongly the grid shows over the
    /// waveform. `remember` saves it (on release, not every drag step).
    setBeatgridLevel(value, { remember = false } = {}) {
      const n = Number(value);
      const level = Number.isFinite(n) ? Math.max(0, Math.min(100, Math.round(n))) : 35;
      setBeatgridLevelPref(level, { remember });
      renderBeatgrid();
    },

    /// The metronome "Mix" slider (0-100): music only, both at full level at
    /// 50, clicks only. Applies live while playing; `remember` saves it (on
    /// release, not every drag step).
    setMetronomeMix(value, { remember = false } = {}) {
      const n = Number(value);
      const mix = Number.isFinite(n) ? Math.max(0, Math.min(100, Math.round(n))) : 50;
      setMetronomeMixPref(mix, { remember });
      renderTools();
      syncMetronome();
    },

    /// The "Playback starts at" choice: First beat adds the playback-start
    /// cue, First cue removes it (only while the track has cues). `remember`
    /// makes it the setting applied to the next track's first cue.
    setStartOnFirstBeat(on, { remember = false } = {}) {
      if (!hotCues().length) {
        render();
        return;
      }
      if (remember) setStartOnFirstBeatPref(!!on);
      mutate(null, () => {
        if (on) addStartCue();
        else working.cues = withoutStartCue();
      });
      render();
    },

    addCueAtRatio(ratio, opts) {
      return api.addCue(Math.max(0, Math.min(1, ratio)) * (working.durationMs || 0), opts);
    },

    /// Rename-only: mutates data without re-rendering the cue list DOM, so the
    /// `<input>` the user is typing into is never destroyed/recreated (that was
    /// causing focus loss after every keystroke). Nothing else on screen depends
    /// on a cue's name while it's being edited.
    renameCue(tempId, name) {
      const cue = working.cues.find((c) => c.tempId === tempId);
      if (cue) mutate(`name:${tempId}`, () => { cue.name = name; });
    },

    updateCue(tempId, patch) {
      const cue = working.cues.find((c) => c.tempId === tempId);
      if (!cue) return;
      mutate(null, () => {
        Object.assign(cue, patch);
        enforceStartOrder();
      });
      render();
    },

    /// Drag a cue marker: move the cue to a view-relative ratio (0..1, clamped
    /// to the visible window). With `snap`, it lands on the nearest beat-grid
    /// line. Only the markers + cue list re-render — the waveform canvas is
    /// untouched, so this is cheap enough to call on every pointermove.
    /// With Quantize on it snaps to the nearest beat; `free` (Shift) inverts that.
    /// One drag is one undo step.
    moveCueToViewRatio(tempId, ratio, { free = false } = {}) {
      const cue = working.cues.find((c) => c.tempId === tempId);
      if (!cue) return;
      const dur = working.durationMs || 0;
      let ms = working.view.startMs + Math.max(0, Math.min(1, ratio)) * viewSpanMs();
      if (snaps(free)) ms = snapToBeat(ms);
      mutate(`drag:${dragSeq}`, () => {
        cue.positionMs = Math.max(0, Math.min(dur, Math.round(ms)));
        // Dragging the start cue pins it (no more following the first beat);
        // it stops at the first hot cue, and a hot cue dragged before it pushes it.
        if (cue.playbackStart) cue.followsFirstBeat = false;
        enforceStartOrder();
      });
      renderMarkers();
      renderCueList();
      renderStartChoice();
    },

    setDraggingCue(tempId) {
      working.draggingTempId = tempId || null;
      if (tempId) {
        dragSeq += 1;
        api.selectCue(tempId);
      }
      renderMarkers();
    },

    deleteCue(tempId) {
      mutate(null, () => {
        working.cues = working.cues.filter((c) => c.tempId !== tempId);
        enforceStartOrder();
      });
      if (!working.cues.some((c) => c.tempId === working.selectedTempId)) {
        working.selectedTempId = null;
      }
      render();
    },

    close(result) {
      if (!open) return;
      open = false;
      metronome.on = false;
      syncMetronome();
      el.trackDetailOverlay.hidden = true;
      if (el.trackDetailColorPopover) el.trackDetailColorPopover.hidden = true;
      if (el.trackDetailPlayhead) el.trackDetailPlayhead.hidden = true;
      if (resizeObserver) {
        resizeObserver.disconnect();
        resizeObserver = null;
      }
      for (const h of [playheadRafHandle, renderViewRafHandle, waveformRetryHandle]) {
        if (h) caf(h);
      }
      playheadRafHandle = renderViewRafHandle = waveformRetryHandle = 0;
      const resolver = resolveFn;
      resolveFn = null;
      if (resolver) resolver(result || null);
    },

    open({ track, firstBeatMs, cues, durationMs, bpm, key, keyOptions: keyGroups, coverCandidates = [] }) {
      if (open) api.close(null);
      open = true;
      working.track = track || {};
      working.bytes = null;
      working.waveNorm = null;
      waveformRetries = 0;
      overviewDrawnKey = "";
      working.durationMs = Number(durationMs) || Number(track?.durationMs) || 0;
      working.bpm = bpm != null ? bpm : track?.bpm ?? null;
      working.key = key != null ? key : track?.key ?? null;
      working.keyGroups = Array.isArray(keyGroups) ? keyGroups : [];
      working.firstBeatMs = firstBeatMs ?? null;
      working.followSuspendUntil = 0;
      working.draggingTempId = null;
      working.selectedTempId = null;
      undoStack = [];
      redoStack = [];
      lastMutateKey = null;
      metronome.on = false;
      playPauseShowsPlaying = null;
      // The backend sends the cues already valid (capped, coloured, the
      // start cue first and never after a hot cue); only the editor's own
      // fields are added here.
      working.cues = (cues || []).map((c) => ({
        tempId: `c${(tempIdSeq += 1)}`,
        positionMs: c.positionMs,
        colorId: c.colorId ?? null,
        name: c.name || "",
        playbackStart: !!c.playbackStart,
        memory: !!c.memory,
      }));
      const start = startCue();
      if (start) start.followsFirstBeat = start.positionMs === working.firstBeatMs;
      const openSpan = defaultViewSpanMs(working.bpm);
      applyView(0, Math.min(openSpan, working.durationMs || openSpan));

      const t = working.track;
      el.trackDetailTitle.textContent = t.title || "Track";
      if (el.trackDetailSubtitle) {
        el.trackDetailSubtitle.textContent = [t.artist, t.album].filter(Boolean).join(" · ");
      }
      if (el.trackDetailCover) {
        el.trackDetailCover.replaceChildren(coverElement(el.trackDetailCover.ownerDocument, coverCandidates));
        attachCoverFallbackHandlers(el.trackDetailCover);
      }
      el.trackDetailOverlay.hidden = false;
      render();
      // Re-measure once layout has settled (canvas is otherwise sized from a
      // pre-layout rect on the first synchronous paint).
      raf(() => renderWaveform());

      const wf = el.trackDetailWaveform;
      if (wf && typeof ResizeObserver === "function") {
        resizeObserver = new ResizeObserver(() => renderWaveform());
        resizeObserver.observe(wf);
      }

      el.trackDetailSaveBtn?.focus();
      const opened = api.toSavePayload();
      openedPayloadJson = JSON.stringify(opened);
      openedCuesJson = JSON.stringify(opened.cues);
      for (const listener of openListeners) listener();

      return new Promise((resolve) => {
        resolveFn = resolve;
      });
    },

    /// True once BPM, key, first beat or the cues differ from what was opened
    /// (view state like zoom or the beat-grid slider doesn't count).
    hasUnsavedChanges() {
      return open && JSON.stringify(api.toSavePayload()) !== openedPayloadJson;
    },

    /// True once the cue list differs from what was opened. A USB save sends
    /// no cues otherwise, so a BPM/key/first-beat edit can't rewrite cues the
    /// editor can't represent (e.g. rekordbox memory cues).
    cuesEdited() {
      return JSON.stringify(api.toSavePayload().cues) !== openedCuesJson;
    },

    toSavePayload() {
      return {
        firstBeatMs: working.firstBeatMs == null ? null : working.firstBeatMs,
        bpm: working.bpm == null ? null : working.bpm,
        key: working.key == null ? null : working.key,
        // The backend trims names and validates the list on save.
        cues: orderedCues().map((c) => ({
          positionMs: c.positionMs,
          colorId: c.colorId,
          name: c.name || null,
          playbackStart: !!c.playbackStart,
          memory: !!c.memory,
        })),
      };
    },
  };

  return api;
}

/// The cue editor's remembered preferences, kept in app state and persisted
/// through the frontend settings.
export function createAppTrackDetailController(ctx) {
  return createTrackDetailController(ctx.el, {
    getStartOnFirstBeatPref: () => !!ctx.state.cueStartOnFirstBeat,
    setStartOnFirstBeatPref: (on) => {
      ctx.state.cueStartOnFirstBeat = !!on;
      ctx.persistSetting(STORAGE_KEY_CUE_START_ON_FIRST_BEAT, FRONTEND_DB_KEY_CUE_START_ON_FIRST_BEAT, on ? "1" : "0");
    },
    setPlaybackMetronome: (request) => ctx.command("set_playback_metronome", request),
    getQuantizePref: () => ctx.state.cueQuantize !== false,
    setQuantizePref: (on) => {
      ctx.state.cueQuantize = !!on;
      ctx.persistSetting(STORAGE_KEY_CUE_QUANTIZE, FRONTEND_DB_KEY_CUE_QUANTIZE, on ? "1" : "0");
    },
    getFollowGridPref: () => !!ctx.state.cueFollowGrid,
    setFollowGridPref: (on) => {
      ctx.state.cueFollowGrid = !!on;
      ctx.persistSetting(STORAGE_KEY_CUE_FOLLOW_GRID, FRONTEND_DB_KEY_CUE_FOLLOW_GRID, on ? "1" : "0");
    },
    getBeatgridLevelPref: () => ctx.state.cueBeatgridLevel,
    setBeatgridLevelPref: (level, { remember = false } = {}) => {
      ctx.state.cueBeatgridLevel = level;
      if (remember) {
        ctx.persistSetting(STORAGE_KEY_CUE_BEATGRID_LEVEL, FRONTEND_DB_KEY_CUE_BEATGRID_LEVEL, String(level));
      }
    },
    getMetronomeMixPref: () => ctx.state.cueMetronomeMix,
    setMetronomeMixPref: (mix, { remember = false } = {}) => {
      ctx.state.cueMetronomeMix = mix;
      if (remember) {
        ctx.persistSetting(STORAGE_KEY_CUE_METRONOME_MIX, FRONTEND_DB_KEY_CUE_METRONOME_MIX, String(mix));
      }
    },
  });
}

/// Open the modal for a track from a USB view (playlists or history): fetch the
/// detail straight off the on-device ANLZ bundle and, on Save, write the edit
/// onto that USB *and* into the local master. The USB must be connected — a
/// not-connected row is blocked here, never silently downgraded to local-only.
async function openUsbTrackDetail(ctx, track) {
  const {
    command,
    trackDetailDialog,
    emitStatus,
    state,
    applyRealtimeAnalyzedTrackUpdate,
    patchTrackAnalysisFields,
  } = ctx;

  if (!state?.usbRootValid || !state?.usbRoot) {
    emitStatus("Connect the USB this track is on before editing its cues.");
    return;
  }
  const usbAnalysisPathRaw = track.usbAnalysisPathRaw;
  if (!usbAnalysisPathRaw) {
    emitStatus("Analyze this track first to edit its cues.");
    return;
  }

  let detail;
  try {
    detail = await command("get_usb_track_detail", {
      usbRoot: state.usbRoot,
      usbAnalysisPathRaw,
    });
  } catch (err) {
    emitStatus(`Could not open cue editor: ${err.message}`);
    return;
  }
  if (!detail.detailWaveform) {
    emitStatus("Analyze this track first to edit its cues.");
    return;
  }

  const payload = await trackDetailDialog.open({
    track: { ...track, detailWaveform: detail.detailWaveform },
    firstBeatMs: detail.firstBeatMs,
    cues: detail.cues,
    durationMs: track.durationMs,
    bpm: track.bpm,
    key: track.key,
    keyOptions: detail.keyOptions,
    coverCandidates: buildCoverSrcCandidates(ctx, track),
  });
  if (!payload) return;

  // Another USB job (an export, diagnostics, another save) writes the same
  // databases on the stick: wait for it instead of dropping the edits.
  if (isUsbRootChangeBlocked(state)) {
    emitStatus("Waiting for the running USB job to finish before saving cues...");
    await ctx.waitForUsbJobIdle();
  }

  try {
    const saved = await command("save_usb_track_analysis_edits", {
      usbRoot: state.usbRoot,
      usbAnalysisPathRaw,
      usbMediaPathRaw: track.usbMediaPath,
      bpm: payload.bpm,
      key: payload.key,
      durationMs: track.durationMs,
      firstBeatMs: payload.firstBeatMs,
      cues: payload.cuesEdited ? payload.cues : null,
      localTrackId: track.localTrackId || null,
    });
    const n = saved.cues.length;
    emitStatus(`Saved ${n} cue${n === 1 ? "" : "s"} to USB`);
    const fields = {
      bpm: saved.bpm,
      bpmAnalyzer: saved.bpmAnalyzer,
      key: saved.key,
      keyDisplay: saved.keyDisplay,
      keyColor: saved.keyColor,
    };
    // Every loaded USB row of this file -- the same track can sit in several
    // USB playlists and the history -- not just the row that was clicked.
    const mediaPath = String(track.usbMediaPath || "").trim();
    for (const ctl of [ctx.usbPlaylistTracksCtl, ctx.usbHistoryTracksCtl]) {
      let changed = false;
      for (const item of ctl?.items || []) {
        const sameFile = item === track
          || (mediaPath && String(item?.usbMediaPath || "").trim() === mediaPath);
        if (sameFile) changed = patchTrackAnalysisFields(item, fields) || changed;
      }
      if (changed) await ctl.rerender();
    }
    patchTrackAnalysisFields(track, fields);
    // The backend also wrote the library track it resolved (not only the
    // row's hint), so refresh that one and the app playlists containing it.
    const localTrackId = saved.localTrackId || track.localTrackId;
    if (localTrackId) {
      applyRealtimeAnalyzedTrackUpdate({ trackId: localTrackId, ...fields });
    }
  } catch (err) {
    emitStatus(`Could not save cues: ${err.message}`);
  }
}

/// Open the modal for a track: resolve to a local id, fetch detail, and on Save
/// persist the edits.
export async function openTrackDetail(ctx, track) {
  const {
    command,
    resolveLocalTrackIdAsync,
    trackDetailDialog,
    emitStatus,
    applyRealtimeAnalyzedTrackUpdate,
  } = ctx;

  if (track?.origin === "usb") {
    return openUsbTrackDetail(ctx, track);
  }

  let localId = null;
  try {
    localId = await resolveLocalTrackIdAsync(track);
  } catch {
    localId = null;
  }
  if (!localId) {
    emitStatus("Analyze this track first to edit its cues.");
    return;
  }

  let detail;
  try {
    detail = await command("get_track_detail", { trackId: localId });
  } catch (err) {
    emitStatus(`Could not open cue editor: ${err.message}`);
    return;
  }
  if (!detail.detailWaveform) {
    emitStatus("Analyze this track first to edit its cues.");
    return;
  }

  const payload = await trackDetailDialog.open({
    track: { ...detail.track, detailWaveform: detail.detailWaveform },
    firstBeatMs: detail.firstBeatMs,
    cues: detail.cues,
    durationMs: detail.track?.durationMs,
    bpm: detail.track?.bpm,
    key: detail.track?.key,
    keyOptions: detail.keyOptions,
    coverCandidates: buildCoverSrcCandidates(ctx, track),
  });
  if (!payload) return;

  try {
    const saved = await command("save_track_analysis_edits", {
      trackId: localId,
      firstBeatMs: payload.firstBeatMs,
      bpm: payload.bpm,
      key: payload.key,
      cues: payload.cues,
    });
    emitStatus(
      `Saved ${saved.cues.length} cue${saved.cues.length === 1 ? "" : "s"}` +
        (saved.anlzRegenerated ? "" : " (analysis cache not updated yet)")
    );
    applyRealtimeAnalyzedTrackUpdate({
      trackId: localId,
      bpm: saved.bpm,
      bpmAnalyzer: saved.bpmAnalyzer,
      key: saved.key,
      keyDisplay: saved.keyDisplay,
      keyColor: saved.keyColor,
    });
  } catch (err) {
    emitStatus(`Could not save cues: ${err.message}`);
  }
}
