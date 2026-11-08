// Track formatting utilities.

import { t } from "./ui_texts.mjs";

// `m:ss` / `h:mm:ss` for a millisecond duration (tracks carry `durationMs`,
// Option<u64>).
export function formatDurationMs(value) {
  const ms = Math.max(0, Math.round(Number(value) || 0));
  const totalSeconds = Math.floor(ms / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

// `track.bpm` is always carried through state as a number (or null) -- both
// `normalizeTrack` and the realtime analysis patch store the raw numeric value.
// This is the single place that turns it into display text: always two
// decimals ("128.00", "87.50"), so a half BPM reads apart from a whole one and
// the right-aligned column lines up on the decimal point.
// What each analyze action does, for its tooltip. A first analysis only fills
// in what's missing; analyzing an analyzed track resets it (backend
// `kept_analysis_values`).
// A row's own Analyze button: disabled while its track waits or is analyzed.
export function applyAnalyzeButtonState(state, button, trackId, analysisReady) {
  const id = String(trackId);
  const waiting = state.analysisWaitingTrackIds.has(id);
  button.disabled = waiting || state.analyzingTrackIds.has(id);
  if (waiting) button.dataset.tooltip = state.analysisWaitingTooltip;
  else button.dataset.tooltip = t(analysisReady ? "ui-track-row-reanalyze-tooltip" : "ui-track-row-analyze-tooltip");
}

export function formatBpm(value) {
  const n = Number(value);
  if (!Number.isFinite(n) || n <= 0) return "";
  return n.toFixed(2);
}

// The single "Total time: … (N without length)" renderer for every track-list
// footer (app playlist, library, USB playlist/history). Both numbers are the
// backend's (`totalDurationMs`, `durationUnknownCount`), passed straight
// through -- no client-side summing or counting.
export function renderTrackListDurationSummary(target, { totalDurationMs, durationUnknownCount } = {}) {
  if (!target) return;
  const unknown = Number(durationUnknownCount) || 0;
  const suffix = unknown > 0 ? ` (${unknown} without length)` : "";
  target.textContent = `Total time: ${formatDurationMs(totalDurationMs)}${suffix}`;
}

// `count` followed by the singular or plural noun ("1 track" / "3 tracks").
// Its last user is the playlist export status line (`formatLastExport`),
// which piece 12 of USB_TEXT_CATALOG_PLAN.md moves; then this goes.
export function counted(count, one, many = `${one}s`) {
  return `${count} ${count === 1 ? one : many}`;
}

export function formatTimestampLocal(value) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

