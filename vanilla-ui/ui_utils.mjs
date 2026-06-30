export function escapeHtml(text) {
  return String(text ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

export function getPlaylistTabDomId(playlistId) {
  const raw = String(playlistId || "").trim();
  const normalized = raw.replace(/[^a-zA-Z0-9_-]/g, "-") || "playlist";
  return `tab-playlist-${normalized}`;
}
