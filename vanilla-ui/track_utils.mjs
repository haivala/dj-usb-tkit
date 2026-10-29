// Track formatting utilities.

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
export const ANALYZE_PLAYLIST_MISSING_TOOLTIP =
  "Add the missing waveform, BPM, key and first beat to this playlist's unanalyzed tracks; a BPM or key you edited or imported is kept";

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

// Binary units, like file managers and `df` show a USB stick's size.
export function formatByteSize(bytes) {
  const n = Math.max(0, Number(bytes) || 0);
  if (n >= 1024 ** 3) return `${(n / 1024 ** 3).toFixed(1)} GB`;
  if (n >= 1024 ** 2) return `${Math.round(n / 1024 ** 2)} MB`;
  return `${Math.round(n / 1024)} KB`;
}

// The "42 tracks · 3.4 GB" summary in every track-list heading (app playlist,
// library, USB playlist/history): the whole list, ignoring the search, as the
// backend reports it in `listTrackCount` / `listSizeBytes`. The size is left
// out when no track in the list has a known size.
export function formatTrackListSummary(trackCount, sizeBytes) {
  const count = Math.max(0, Number(trackCount) || 0);
  const parts = [`${count} ${count === 1 ? "track" : "tracks"}`];
  if (Number(sizeBytes) > 0) parts.push(formatByteSize(sizeBytes));
  return parts.join(" · ");
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

