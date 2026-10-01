// Track formatting utilities.

// Shared "infinite scroll" check: if `wrap` is scrolled within `thresholdPx`
// of its bottom, and nothing else is already loading/blocking it, load the
// next page. Originally specific to the library table
// (handleLibraryTableWrapScroll); extracted so other large, paginated track
// lists (USB playlist/history) can reuse the exact same check instead of
// re-implementing it.
export function loadMoreIfNearBottom(wrap, thresholdPx, isBusy, hasMore, loadMore) {
  if (!wrap || isBusy() || !hasMore()) return;
  const remaining = wrap.scrollHeight - wrap.scrollTop - wrap.clientHeight;
  if (remaining > thresholdPx) return;
  return loadMore();
}

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
export const ANALYZE_TOOLTIP =
  "Add the missing waveform, BPM, key and first beat; a BPM or key you edited or imported is kept";
export const REANALYZE_TOOLTIP =
  "Reset waveform, BPM, key and first beat to a fresh analysis, replacing imported or edited values (cues are kept)";
export const ANALYZE_SELECTED_TOOLTIP =
  "Unanalyzed tracks get the missing waveform, BPM, key and first beat (edited or imported BPM/key kept); analyzed tracks are reset to a fresh analysis, replacing those values (cues are kept)";
export const ANALYZE_PLAYLIST_MISSING_TOOLTIP =
  "Add the missing waveform, BPM, key and first beat to this playlist's unanalyzed tracks; a BPM or key you edited or imported is kept";

export function formatBpm(value) {
  const n = Number(value);
  if (!Number.isFinite(n) || n <= 0) return "";
  return n.toFixed(2);
}

// The single "Total time: … (N without length)" renderer for every track-list
// footer (app playlist, library, USB playlist/history). The totals are always
// backend-computed and passed straight through -- no client-side summing, and
// no reaction to a client-side search filter. Callers give the unknown count
// either directly as `unknownCount`, or as `trackCount` - `durationKnownCount`.
export function renderTrackListDurationSummary(
  target,
  { totalDurationMs, unknownCount, durationKnownCount, trackCount } = {},
  formatDurationMsFn = formatDurationMs
) {
  if (!target) return;
  const total = Math.max(0, Number(totalDurationMs) || 0);
  const unknown = Number.isFinite(Number(unknownCount))
    ? Math.max(0, Number(unknownCount))
    : Math.max(0, (Number(trackCount) || 0) - Math.max(0, Number(durationKnownCount) || 0));
  const suffix = unknown > 0 ? ` (${unknown} without length)` : "";
  target.textContent = `Total time: ${formatDurationMsFn(total)}${suffix}`;
}

export function buildTracklistText(tracks, timeMode) {
  const items = Array.isArray(tracks) ? tracks : [];
  let cumulativeMs = 0;
  return items
    .map((track) => {
      const line = `${track?.artist || ""} - ${track?.title || ""}`;
      if (timeMode !== "before" && timeMode !== "after") return line;
      const stamp = formatDurationMs(cumulativeMs);
      const durationMs = Number(track?.durationMs);
      cumulativeMs += Number.isFinite(durationMs) && durationMs > 0 ? durationMs : 0;
      return timeMode === "before" ? `${stamp} ${line}` : `${line} - ${stamp}`;
    })
    .join("\n");
}

// The backend resolves a history session's date itself (from the export log,
// then the PDB's track date_created -- see `apply_history_dates_*` in
// service/usb.rs) and hands it back as the one `createdAt` field.
export function getHistoryDateValue(history) {
  return history?.createdAt || "";
}

export function formatTimestampLocal(value) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

