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

// Pure setter -- the playlist total is computed by the backend
// (GetPlaylistTracksData::total_duration_ms / duration_known_count, over the
// full playlist) and passed straight through here, matching the library and
// USB footers. It intentionally does not react to the client-side track search
// filter, same as the USB playlist/history footers.
export function renderTrackListDurationSummary(target, { totalDurationMs, durationKnownCount, trackCount } = {}, formatDurationMsFn = formatDurationMs) {
  if (!target) return;
  const total = Math.max(0, Number(totalDurationMs) || 0);
  const known = Math.max(0, Number(durationKnownCount) || 0);
  const unknown = Math.max(0, (Number(trackCount) || 0) - known);
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

export function filterTracksByQuery(tracks, query) {
  const q = String(query || "").trim().toLowerCase();
  if (!q) return tracks.slice();
  return tracks.filter((track) => {
    const row = `${track.title || ""} ${track.artist || ""} ${track.album || ""}`.toLowerCase();
    return row.includes(q);
  });
}
