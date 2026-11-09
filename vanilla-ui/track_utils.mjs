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
