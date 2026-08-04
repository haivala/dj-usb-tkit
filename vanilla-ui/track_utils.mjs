// Track normalization and formatting utilities.

export function normalizeDurationMs(track) {
  if (!track || typeof track !== "object") return null;
  const directMs = [
    track.durationMs,
    track.duration_ms,
    track.lengthMs,
    track.length_ms
  ].map((v) => Number(v)).find((v) => Number.isFinite(v) && v > 0);
  if (Number.isFinite(directMs)) return Math.round(directMs);

  const directSeconds = [
    track.durationSec,
    track.duration_sec,
    track.durationSeconds,
    track.duration_seconds,
    track.lengthSec,
    track.length_sec
  ].map((v) => Number(v)).find((v) => Number.isFinite(v) && v > 0);
  if (Number.isFinite(directSeconds)) return Math.round(directSeconds * 1000);

  const generic = Number(track.duration);
  if (!Number.isFinite(generic) || generic <= 0) return null;
  // Legacy payloads may provide seconds in `duration`; assume seconds for small values.
  if (generic <= 24 * 60 * 60) return Math.round(generic * 1000);
  return Math.round(generic);
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

export function updateTrackListDurationSummary(target, tracks) {
  const items = Array.isArray(tracks) ? tracks : [];
  const durations = items
    .map((track) => Number(track?.durationMs))
    .filter((value) => Number.isFinite(value) && value > 0);
  const totalMs = durations.reduce((sum, value) => sum + value, 0);
  const unknownCount = Math.max(0, items.length - durations.length);
  if (target) {
    const unknownSuffix = unknownCount > 0 ? ` (${unknownCount} without length)` : "";
    target.textContent = `Total time: ${formatDurationMs(totalMs)}${unknownSuffix}`;
  }
  return { totalMs, unknownCount };
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
