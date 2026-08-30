// Track normalization and formatting utilities.

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

// The backend sends exactly one canonical duration field, `durationMs`
// (Option<u64> milliseconds), on every track-bearing response.
export function normalizeDurationMs(track) {
  const ms = Number(track?.durationMs);
  return Number.isFinite(ms) && ms > 0 ? Math.round(ms) : null;
}

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

export function getHistoryDateValue(history) {
  return history?.createdAt || history?.sourceCreatedAt || history?.sourcePlayedAt || "";
}

export function getHistoryDateDisplay(history) {
  const value = getHistoryDateValue(history);
  if (!value) return "";
  return `not earlier than ${value}`;
}

export function formatTimestampLocal(value) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

