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

export const MAX_CUES = 8;
export const MIN_SPAN_MS = 1000;
export const DEFAULT_SPAN_MS = 120_000;
const BPM_NUDGE_STEP = 0.01;
// Bar numbers are shown every N bars, N the smallest of these that keeps the
// labels at least BAR_LABEL_MIN_PX apart.
const BAR_LABEL_STEPS = [1, 2, 4, 8, 16, 32, 64];
const BAR_LABEL_MIN_PX = 36;

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
const DEFAULT_COLOR_ID = 5;
const START_BEAT_TOOLTIP =
  "Adds a ▶ memory cue (no hot cue) on the first beat, so the CDJ loads there instead of on cue A";
const START_MARKER_TOOLTIP =
  "The CDJ loads on the ▶ start marker you placed; drag it on the waveform to move it";

// Mirrors backend `KEY_OPTIONS` (service/cues.rs). 12 majors then 12 minors.
export const KEY_OPTIONS = [
  "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
  "Cm", "C#m", "Dm", "D#m", "Em", "Fm", "F#m", "Gm", "G#m", "Am", "A#m", "Bm",
];

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

  const working = {
    track: null,
    durationMs: 0,
    bpm: null,
    key: null,
    firstBeatMs: null,
    cues: [],
    view: { startMs: 0, endMs: 0 },
    bytes: null, // decoded PWV5 Uint8Array
    waveNorm: null, // whole-track {lo, hi} amplitude reference (fixed across zoom)
    followSuspendUntil: 0,
    draggingTempId: null, // cue whose marker is being dragged on the waveform
  };
  let playPauseShowsPlaying = null;
  // The save payload as opened; anything else is an unsaved edit.
  let openedPayloadJson = "";

  function hotCues() {
    return working.cues.filter((c) => !c.playbackStart);
  }

  function startCue() {
    return working.cues.find((c) => c.playbackStart) || null;
  }

  function orderedCues() {
    return working.cues.slice().sort(byPosition);
  }

  /// The playback-start cue never lies after a hot cue, and never exists
  /// without one.
  function enforceStartOrder() {
    const start = startCue();
    if (!start) return;
    const hot = hotCues();
    if (!hot.length) {
      working.cues = hot;
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
    const barStep = barLabelStep(interval);
    // Snap `from` to the nearest grid line at or before it.
    const firstBeatIdx = Math.max(0, Math.floor((from - working.firstBeatMs) / interval));
    let safety = 0;
    for (let idx = firstBeatIdx; ; idx += 1) {
      const t = working.firstBeatMs + idx * interval;
      if (t > to || safety > 8000) break;
      safety += 1;
      const downbeat = idx % 4 === 0;
      const line = host.ownerDocument.createElement("i");
      line.className = "beatgrid-line" + (downbeat ? " is-downbeat" : "");
      line.style.left = `${msToPct(t)}%`;
      host.appendChild(line);
      const bar = idx / 4; // 0-based
      if (downbeat && bar % barStep === 0) {
        const label = host.ownerDocument.createElement("b");
        label.className = "beatgrid-bar";
        label.style.left = `${msToPct(t)}%`;
        label.textContent = String(bar + 1);
        host.appendChild(label);
      }
    }
  }

  /// Label every Nth bar so the numbers never crowd (1, 5, 9… when zoomed out).
  function barLabelStep(interval) {
    const width = el.trackDetailWaveform?.clientWidth || 0;
    const pxPerBar = width ? ((4 * interval) / viewSpanMs()) * width : 0;
    if (!pxPerBar) return 4;
    return BAR_LABEL_STEPS.find((n) => n * pxPerBar >= BAR_LABEL_MIN_PX)
      ?? BAR_LABEL_STEPS[BAR_LABEL_STEPS.length - 1];
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
    for (const cue of orderedCues()) {
      const tick = cuesHost.ownerDocument.createElement("i");
      tick.className = "overview-cue" + (cue.playbackStart ? " is-playback-start" : "");
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
    for (const cue of orderedCues()) {
      const pct = msToPct(cue.positionMs);
      const marker = host.ownerDocument.createElement("i");
      marker.className =
        "cue-marker" +
        (cue.playbackStart ? " is-playback-start" : "") +
        (pct < -2 || pct > 102 ? " off-view" : "") +
        (cue.tempId === working.draggingTempId ? " is-dragging" : "");
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
    const first = orderedCues()[0];
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
    btn.innerHTML = playing
      ? '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="7" y="6" width="3.5" height="12" rx="1"></rect><rect x="13.5" y="6" width="3.5" height="12" rx="1"></rect></svg>'
      : '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 6v12l10-6z"></path></svg>';
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
    for (const cue of orderedCues()) {
      labels.set(cue.tempId, cue.playbackStart ? "▶" : String.fromCharCode(65 + hotIndex++));
    }
    return labels;
  }

  function cueRow(cue, letter) {
    const doc = el.trackDetailCueList.ownerDocument;
    const row = doc.createElement("div");
    row.className = "cue-row" + (cue.playbackStart ? " is-playback-start" : "");
    row.dataset.tempId = cue.tempId;

    const play = doc.createElement("button");
    play.type = "button";
    play.className = "cue-row-play";
    play.dataset.action = "cue-play";
    play.setAttribute("aria-label", "Play from this cue");
    play.dataset.tooltip = "Play from here";
    play.innerHTML =
      '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 6v12l10-6z"></path></svg>';
    row.appendChild(play);

    const pos = doc.createElement("span");
    pos.className = "cue-row-pos";
    pos.textContent = formatMs(cue.positionMs);
    row.appendChild(pos);

    if (cue.playbackStart) {
      const memory = doc.createElement("span");
      memory.className = "cue-row-memory";
      memory.textContent = letter;
      memory.dataset.tooltip = "Memory cue only (no hot cue)";
      row.appendChild(memory);
    } else {
      const swatch = doc.createElement("button");
      swatch.type = "button";
      swatch.className = "cue-row-color";
      swatch.dataset.action = "cue-color";
      swatch.style.background = colorCssForId(cue.colorId);
      swatch.textContent = letter;
      swatch.setAttribute("aria-label", `Cue ${letter} colour`);
      row.appendChild(swatch);
    }

    if (cue.playbackStart) {
      // Not nameable, and removed only by the "Start the playback…" toggle.
      const label = doc.createElement("span");
      label.className = "cue-row-label";
      label.textContent = "Playback start";
      row.appendChild(label);
      const badge = doc.createElement("span");
      badge.className = "cue-row-badge";
      badge.textContent = "memory cue";
      row.appendChild(badge);
      return row;
    }

    const name = doc.createElement("input");
    name.type = "text";
    name.className = "cue-row-name";
    name.dataset.action = "cue-name";
    name.placeholder = "Name";
    name.value = cue.name || "";
    row.appendChild(name);

    const del = doc.createElement("button");
    del.type = "button";
    del.className = "cue-row-delete";
    del.dataset.action = "cue-delete";
    del.textContent = "×";
    del.setAttribute("aria-label", "Delete cue");
    row.appendChild(del);

    return row;
  }

  function renderCueList() {
    const host = el.trackDetailCueList;
    if (!host) return;
    host.textContent = "";
    const hotCount = hotCues().length;
    if (!hotCount) {
      const empty = host.ownerDocument.createElement("p");
      empty.className = "muted cue-list-empty";
      empty.textContent = "No cues yet. Double-click the waveform to add one, or play and hit “+ Cue”.";
      host.appendChild(empty);
    } else {
      const labels = cueLabels();
      for (const cue of orderedCues()) host.appendChild(cueRow(cue, labels.get(cue.tempId)));
    }
    if (el.trackDetailAddCue) el.trackDetailAddCue.disabled = hotCount >= MAX_CUES;
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

  // The modal opens zoomed to the first ~2 min, so make it unmistakable that
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

  // A stored key can predate this stepper (e.g. essentia's flat spellings)
  // and won't match any canonical `KEY_OPTIONS` entry -- rather than silently
  // dropping it, keep it visible via a synthetic option so the select always
  // reflects the real current value.
  function syncKeySelect() {
    const select = el.trackDetailKey;
    if (!select) return;
    const synthetic = select.querySelector("option[data-synthetic]");
    if (working.key != null && !KEY_OPTIONS.includes(working.key)) {
      const option = synthetic || select.ownerDocument.createElement("option");
      option.value = working.key;
      option.textContent = working.key;
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
      el.trackDetailBpm.value = working.bpm == null ? "" : String(working.bpm);
    }
    syncKeySelect();
    renderView();
    renderCueList();
    renderStartChoice();
    renderHint();
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

    notePlaybackStarted() {
      if (!playheadRafHandle) playheadRafHandle = raf(playheadTick);
    },

    setFirstBeatMs(ms) {
      const clamped = Math.max(0, Math.round(Number(ms) || 0));
      working.firstBeatMs = working.durationMs
        ? Math.min(clamped, working.durationMs - 1)
        : clamped;
      // An untouched playback-start cue follows the first beat.
      const start = startCue();
      if (start?.followsFirstBeat) {
        start.positionMs = working.firstBeatMs;
        enforceStartOrder();
      }
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
      working.bpm = Math.round(Math.min(999, parsed) * 100) / 100;
      render();
    },

    nudgeBpm(direction) {
      const base = working.bpm == null ? 0 : working.bpm;
      api.setBpm(base + direction * BPM_NUDGE_STEP);
    },

    setKey(key) {
      const trimmed = typeof key === "string" ? key.trim() : "";
      working.key = trimmed ? trimmed.slice(0, 16) : null;
      render();
    },

    /// Step to the next/previous entry in `KEY_OPTIONS`. A current value
    /// outside that list (a legacy/non-canonical stored key) jumps onto the
    /// nearest end instead of stepping relative to a position it doesn't have.
    nudgeKey(direction) {
      const count = KEY_OPTIONS.length;
      const index = working.key == null ? -1 : KEY_OPTIONS.indexOf(working.key);
      const nextIndex = index === -1
        ? (direction > 0 ? 0 : count - 1)
        : (index + direction + count) % count;
      api.setKey(KEY_OPTIONS[nextIndex]);
    },

    /// Add a cue. With an explicit `positionMs` it lands there (double-click on
    /// the waveform); with no argument it lands at the current playhead ("+ Cue"),
    /// or where playback was paused.
    /// Name and colour default to "Cue N" / the Nth palette colour (N = 1-based
    /// add order) — assigned once at creation, never renumbered later, and
    /// always user-editable afterward.
    /// The first cue on a track also applies the remembered "Start the
    /// playback on first beat" setting.
    addCue(positionMs) {
      const ordinal = hotCues().length;
      if (ordinal >= MAX_CUES) return null;
      const dur = working.durationMs || 0;
      const pos = Number.isFinite(positionMs)
        ? Math.max(0, Math.min(dur, Math.round(positionMs)))
        : Math.round(currentPositionMs());
      const cue = {
        tempId: `c${(tempIdSeq += 1)}`,
        positionMs: pos,
        colorId: HOTCUE_PALETTE[ordinal % HOTCUE_PALETTE.length].id,
        name: `Cue ${ordinal + 1}`,
      };
      working.cues.push(cue);
      if (ordinal === 0 && getStartOnFirstBeatPref()) addStartCue();
      enforceStartOrder();
      render();
      return cue;
    },

    /// The "Beat grid" slider (0-100): how strongly the grid shows over the
    /// waveform. `remember` saves it (on release, not every drag step).
    setBeatgridLevel(value, { remember = false } = {}) {
      const n = Number(value);
      const level = Number.isFinite(n) ? Math.max(0, Math.min(100, Math.round(n))) : 35;
      setBeatgridLevelPref(level, { remember });
      renderBeatgrid();
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
      if (on) addStartCue();
      else working.cues = hotCues();
      render();
    },

    addCueAtRatio(ratio) {
      return api.addCue(Math.max(0, Math.min(1, ratio)) * (working.durationMs || 0));
    },

    /// Rename-only: mutates data without re-rendering the cue list DOM, so the
    /// `<input>` the user is typing into is never destroyed/recreated (that was
    /// causing focus loss after every keystroke). Nothing else on screen depends
    /// on a cue's name while it's being edited.
    renameCue(tempId, name) {
      const cue = working.cues.find((c) => c.tempId === tempId);
      if (cue) cue.name = name;
    },

    updateCue(tempId, patch) {
      const cue = working.cues.find((c) => c.tempId === tempId);
      if (!cue) return;
      Object.assign(cue, patch);
      enforceStartOrder();
      render();
    },

    /// Drag a cue marker: move the cue to a view-relative ratio (0..1, clamped
    /// to the visible window). With `snap`, it lands on the nearest beat-grid
    /// line. Only the markers + cue list re-render — the waveform canvas is
    /// untouched, so this is cheap enough to call on every pointermove.
    moveCueToViewRatio(tempId, ratio, { snap = false } = {}) {
      const cue = working.cues.find((c) => c.tempId === tempId);
      if (!cue) return;
      const dur = working.durationMs || 0;
      let ms = working.view.startMs + Math.max(0, Math.min(1, ratio)) * viewSpanMs();
      const interval = beatIntervalMs();
      if (snap && interval && working.firstBeatMs != null) {
        const idx = Math.max(0, Math.round((ms - working.firstBeatMs) / interval));
        ms = working.firstBeatMs + idx * interval;
      }
      cue.positionMs = Math.max(0, Math.min(dur, Math.round(ms)));
      // Dragging the start cue pins it (no more following the first beat);
      // it stops at the first hot cue, and a hot cue dragged before it pushes it.
      if (cue.playbackStart) cue.followsFirstBeat = false;
      enforceStartOrder();
      renderMarkers();
      renderCueList();
      renderStartChoice();
    },

    setDraggingCue(tempId) {
      working.draggingTempId = tempId || null;
      renderMarkers();
    },

    deleteCue(tempId) {
      working.cues = working.cues.filter((c) => c.tempId !== tempId);
      enforceStartOrder();
      render();
    },

    close(result) {
      if (!open) return;
      open = false;
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

    open({ track, firstBeatMs, cues, durationMs, bpm, key }) {
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
      working.firstBeatMs = firstBeatMs == null ? null : Math.round(firstBeatMs);
      working.followSuspendUntil = 0;
      working.draggingTempId = null;
      playPauseShowsPlaying = null;
      const loaded = (cues || []).map((c) => ({
        tempId: `c${(tempIdSeq += 1)}`,
        positionMs: Math.round(c.positionMs || 0),
        colorId: c.playbackStart ? null : c.colorId ?? DEFAULT_COLOR_ID,
        name: c.name || "",
        playbackStart: !!c.playbackStart,
      }));
      const start = loaded.find((c) => c.playbackStart);
      if (start) start.followsFirstBeat = start.positionMs === working.firstBeatMs;
      working.cues = [
        ...(start ? [start] : []),
        ...loaded.filter((c) => !c.playbackStart).slice(0, MAX_CUES),
      ];
      enforceStartOrder();
      applyView(0, Math.min(DEFAULT_SPAN_MS, working.durationMs || DEFAULT_SPAN_MS));

      const t = working.track;
      el.trackDetailTitle.textContent =
        `${t.album ? t.album + " · " : ""}${t.artist ? t.artist + " – " : ""}${t.title || "Track"}`;
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
      openedPayloadJson = JSON.stringify(api.toSavePayload());

      return new Promise((resolve) => {
        resolveFn = resolve;
      });
    },

    /// True once BPM, key, first beat or the cues differ from what was opened
    /// (view state like zoom or the beat-grid slider doesn't count).
    hasUnsavedChanges() {
      return open && JSON.stringify(api.toSavePayload()) !== openedPayloadJson;
    },

    toSavePayload() {
      return {
        firstBeatMs: working.firstBeatMs == null ? null : working.firstBeatMs,
        bpm: working.bpm == null ? null : working.bpm,
        key: working.key == null ? null : working.key,
        cues: orderedCues().map((c) => ({
          positionMs: Math.round(c.positionMs),
          colorId: c.playbackStart ? null : c.colorId ?? DEFAULT_COLOR_ID,
          name: !c.playbackStart && c.name?.trim() ? c.name.trim() : null,
          playbackStart: !!c.playbackStart,
        })),
      };
    },
  };

  return api;
}

/// Open the modal for a track from a USB view (playlists or history): fetch the
/// detail straight off the on-device ANLZ bundle and, on Save, write the edit
/// onto that USB *and* into the local master. The USB must be connected — a
/// not-connected row is blocked here, never silently downgraded to local-only.
async function openUsbTrackDetail(track, deps) {
  const {
    command,
    trackDetailDialog,
    emitStatus,
    state,
    applyRealtimeAnalyzedTrackUpdate,
    patchTrackAnalysisFields,
  } = deps;

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
  });
  if (!payload) return;

  try {
    const saved = await command("save_usb_track_analysis_edits", {
      usbRoot: state.usbRoot,
      usbAnalysisPathRaw,
      usbMediaPathRaw: track.usbMediaPath,
      bpm: payload.bpm,
      key: payload.key,
      durationMs: track.durationMs,
      firstBeatMs: payload.firstBeatMs,
      cues: payload.cues,
      localTrackId: track.localTrackId || null,
    });
    const n = saved.cues.length;
    emitStatus(`Saved ${n} cue${n === 1 ? "" : "s"} to USB`);
    patchTrackAnalysisFields?.(track, { bpm: saved.bpm, bpmAnalyzer: saved.bpmAnalyzer, key: saved.key });
    if (track.localTrackId) {
      applyRealtimeAnalyzedTrackUpdate?.({
        trackId: track.localTrackId,
        bpm: saved.bpm,
        bpmAnalyzer: saved.bpmAnalyzer,
        key: saved.key,
      });
    }
  } catch (err) {
    emitStatus(`Could not save cues: ${err.message}`);
  }
}

/// Open the modal for a track: resolve to a local id, fetch detail, and on Save
/// persist the edits.
export async function openTrackDetail(track, deps) {
  const {
    command,
    resolveLocalTrackIdAsync,
    trackDetailDialog,
    emitStatus,
    applyRealtimeAnalyzedTrackUpdate,
  } = deps;

  if (track?.origin === "usb") {
    return openUsbTrackDetail(track, deps);
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
    applyRealtimeAnalyzedTrackUpdate?.({
      trackId: localId,
      bpm: saved.bpm,
      bpmAnalyzer: saved.bpmAnalyzer,
      key: saved.key,
    });
  } catch (err) {
    emitStatus(`Could not save cues: ${err.message}`);
  }
}
