// Track-detail modal: cue points + beat-grid ("first beat") editing.
//
// This app targets CDJ playback directly, so a cue is just a position + name +
// colour. The list is capped at 8; on save each becomes a memory point + a
// hot-cue pad. The waveform is the full-detail PWV5 colour waveform with
// scroll-to-zoom and drag-to-pan (see waveform_detail.mjs).

import { drawDetailWaveform, base64ToBytes, computeWaveNorm } from "./waveform_detail.mjs";

export const MAX_CUES = 8;
export const MIN_SPAN_MS = 1000;
export const DEFAULT_SPAN_MS = 120_000;

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

let tempIdSeq = 0;

const raf = (cb) => (globalThis.requestAnimationFrame || ((f) => setTimeout(f, 16)))(cb);
const caf = (h) => (globalThis.cancelAnimationFrame || globalThis.clearTimeout)(h);
const nowMs = () => globalThis.performance?.now?.() ?? Date.now();

export function createTrackDetailController(el) {
  let resolveFn = null;
  let open = false;
  let resizeObserver = null;
  let playheadRafHandle = 0;
  let renderViewRafHandle = 0;
  let waveformRetryHandle = 0;
  let waveformRetries = 0;

  const working = {
    track: null,
    durationMs: 0,
    bpm: null,
    firstBeatMs: null,
    cues: [],
    view: { startMs: 0, endMs: 0 },
    bytes: null, // decoded PWV5 Uint8Array
    waveNorm: null, // whole-track {lo, hi} amplitude reference (fixed across zoom)
    followSuspendUntil: 0,
  };

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
    host.textContent = "";
    const interval = beatIntervalMs();
    if (!interval || !working.durationMs || working.firstBeatMs == null) return;
    const from = Math.max(working.firstBeatMs, working.view.startMs - interval);
    const to = Math.min(working.durationMs, working.view.endMs + interval);
    // Snap `from` to the nearest grid line at or before it.
    const firstBeatIdx = Math.max(0, Math.floor((from - working.firstBeatMs) / interval));
    let safety = 0;
    for (let idx = firstBeatIdx; ; idx += 1) {
      const t = working.firstBeatMs + idx * interval;
      if (t > to || safety > 8000) break;
      safety += 1;
      const line = host.ownerDocument.createElement("i");
      line.className = "beatgrid-line" + (idx % 4 === 0 ? " is-downbeat" : "");
      line.style.left = `${msToPct(t)}%`;
      host.appendChild(line);
    }
  }

  function renderMarkers() {
    const host = el.trackDetailCueMarkers;
    if (!host) return;
    host.textContent = "";
    const ordered = working.cues.slice().sort((a, b) => a.positionMs - b.positionMs);
    ordered.forEach((cue, i) => {
      const pct = msToPct(cue.positionMs);
      const marker = host.ownerDocument.createElement("i");
      marker.className = "cue-marker" + (pct < -2 || pct > 102 ? " off-view" : "");
      marker.style.left = `${pct}%`;
      marker.style.setProperty("--cue-color", colorCssForId(cue.colorId));
      marker.dataset.tempId = cue.tempId;
      marker.textContent = String.fromCharCode(65 + i);
      marker.dataset.tooltip = `${marker.textContent} · ${formatMs(cue.positionMs)}`;
      host.appendChild(marker);
    });
  }

  function positionModalPlayhead() {
    const ph = el.trackDetailPlayhead;
    if (!ph) return;
    const posMs = playheadFullRatio() * working.durationMs;
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
    if (!open || !wf || !wf.classList.contains("is-playing")) {
      playheadRafHandle = 0;
      if (el.trackDetailPlayhead) el.trackDetailPlayhead.hidden = true;
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

  function cueRow(cue) {
    const doc = el.trackDetailCueList.ownerDocument;
    const row = doc.createElement("div");
    row.className = "cue-row";
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

    const swatch = doc.createElement("button");
    swatch.type = "button";
    swatch.className = "cue-row-color";
    swatch.dataset.action = "cue-color";
    swatch.style.background = colorCssForId(cue.colorId);
    row.appendChild(swatch);

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
    const ordered = working.cues.slice().sort((a, b) => a.positionMs - b.positionMs);
    if (!ordered.length) {
      const empty = host.ownerDocument.createElement("p");
      empty.className = "muted cue-list-empty";
      empty.textContent = "No cues yet. Double-click the waveform to add one, or play and hit “+ Cue”.";
      host.appendChild(empty);
    } else {
      for (const cue of ordered) host.appendChild(cueRow(cue));
    }
    if (el.trackDetailAddCue) el.trackDetailAddCue.disabled = working.cues.length >= MAX_CUES;
  }

  function renderView() {
    if (!open) return;
    renderWaveform();
    renderBeatgrid();
    renderMarkers();
    positionModalPlayhead();
  }

  function render() {
    if (!open) return;
    if (el.trackDetailFirstBeatMs) {
      el.trackDetailFirstBeatMs.value =
        working.firstBeatMs == null ? "" : String(working.firstBeatMs);
    }
    renderView();
    renderCueList();
  }

  const api = {
    isOpen: () => open,
    getWorking: () => working,
    getView: () => ({ ...working.view }),
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
      render();
    },

    nudgeFirstBeat(direction) {
      const interval = beatIntervalMs();
      if (!interval) return;
      const base = working.firstBeatMs == null ? 0 : working.firstBeatMs;
      api.setFirstBeatMs(base + direction * interval);
    },

    /// Add a cue. With an explicit `positionMs` it lands there (double-click on
    /// the waveform); with no argument it lands at the current playhead ("+ Cue").
    /// Name and colour default to "Cue N" / the Nth palette colour (N = 1-based
    /// add order) — assigned once at creation, never renumbered later, and
    /// always user-editable afterward.
    addCue(positionMs) {
      if (working.cues.length >= MAX_CUES) return null;
      const dur = working.durationMs || 0;
      const pos = Number.isFinite(positionMs)
        ? Math.max(0, Math.min(dur, Math.round(positionMs)))
        : Math.round(playheadFullRatio() * dur);
      const ordinal = working.cues.length;
      const cue = {
        tempId: `c${(tempIdSeq += 1)}`,
        positionMs: pos,
        colorId: HOTCUE_PALETTE[ordinal % HOTCUE_PALETTE.length].id,
        name: `Cue ${ordinal + 1}`,
      };
      working.cues.push(cue);
      render();
      return cue;
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
      render();
    },

    deleteCue(tempId) {
      working.cues = working.cues.filter((c) => c.tempId !== tempId);
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

    open({ track, firstBeatMs, cues, durationMs, bpm }) {
      if (open) api.close(null);
      open = true;
      working.track = track || {};
      working.bytes = null;
      working.waveNorm = null;
      waveformRetries = 0;
      working.durationMs = Number(durationMs) || Number(track?.durationMs) || 0;
      working.bpm = bpm != null ? bpm : track?.bpm ?? null;
      working.firstBeatMs = firstBeatMs == null ? null : Math.round(firstBeatMs);
      working.followSuspendUntil = 0;
      working.cues = (cues || []).slice(0, MAX_CUES).map((c) => ({
        tempId: `c${(tempIdSeq += 1)}`,
        positionMs: Math.round(c.positionMs || 0),
        colorId: c.colorId ?? DEFAULT_COLOR_ID,
        name: c.name || "",
      }));
      applyView(0, Math.min(DEFAULT_SPAN_MS, working.durationMs || DEFAULT_SPAN_MS));

      const t = working.track;
      el.trackDetailTitle.textContent =
        `${t.artist ? t.artist + " – " : ""}${t.title || "Track"} · Cues & beat grid`;
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

      return new Promise((resolve) => {
        resolveFn = resolve;
      });
    },

    toSavePayload() {
      return {
        firstBeatMs: working.firstBeatMs == null ? null : working.firstBeatMs,
        cues: working.cues
          .slice()
          .sort((a, b) => a.positionMs - b.positionMs)
          .map((c) => ({
            positionMs: Math.round(c.positionMs),
            colorId: c.colorId ?? DEFAULT_COLOR_ID,
            name: c.name?.trim() ? c.name.trim() : null,
          })),
      };
    },
  };

  return api;
}

/// Open the modal for a track: resolve to a local id, fetch detail, and on Save
/// persist the edits.
export async function openTrackDetail(track, deps) {
  const { command, resolveLocalTrackIdAsync, trackDetailDialog, emitStatus } = deps;
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
  });
  if (!payload) return;

  try {
    const saved = await command("save_track_analysis_edits", {
      trackId: localId,
      firstBeatMs: payload.firstBeatMs,
      cues: payload.cues,
    });
    emitStatus(
      `Saved ${saved.cues.length} cue${saved.cues.length === 1 ? "" : "s"}` +
        (saved.anlzRegenerated ? "" : " (analysis cache not updated yet)")
    );
  } catch (err) {
    emitStatus(`Could not save cues: ${err.message}`);
  }
}
