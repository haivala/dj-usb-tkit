(function (root) {
  function normalize(value) {
    return String(value || "").trim().toLowerCase();
  }

  function normalizePath(value) {
    return String(value || "").replace(/\\/g, "/").trim().toLowerCase();
  }

  function fileName(value) {
    const p = normalizePath(value);
    const idx = p.lastIndexOf("/");
    return idx >= 0 ? p.slice(idx + 1) : p;
  }

  function stem(value) {
    const f = fileName(value);
    const idx = f.lastIndexOf(".");
    return idx > 0 ? f.slice(0, idx) : f;
  }

  function scoreLocalTrackCandidate(candidate, sourceTrack) {
    if (!candidate || !candidate.filePath) return -1;

    const srcTitle = normalize(sourceTrack && sourceTrack.title);
    const srcArtist = normalize(sourceTrack && sourceTrack.artist);
    const srcAlbum = normalize(sourceTrack && sourceTrack.album);
    const srcPath = normalizePath(sourceTrack && sourceTrack.filePath);
    const srcFile = fileName(sourceTrack && sourceTrack.filePath);
    const srcStem = stem(sourceTrack && sourceTrack.filePath);

    const candTitle = normalize(candidate.title);
    const candArtist = normalize(candidate.artist);
    const candAlbum = normalize(candidate.album);
    const candPath = normalizePath(candidate.filePath);
    const candFile = fileName(candidate.filePath);
    const candStem = stem(candidate.filePath);

    let score = 0;
    if (srcTitle && candTitle === srcTitle) score += 12;
    if (srcArtist && candArtist === srcArtist) score += 12;
    if (srcAlbum && candAlbum === srcAlbum) score += 8;
    if (srcPath && candPath === srcPath) score += 24;
    if (srcFile && candFile === srcFile) score += 16;
    if (srcStem && candStem === srcStem) score += 8;

    const srcBpm = Number(sourceTrack && sourceTrack.bpm);
    const candBpm = Number(candidate && candidate.bpm);
    if (Number.isFinite(srcBpm) && Number.isFinite(candBpm) && Math.abs(srcBpm - candBpm) <= 0.15) {
      score += 4;
    }

    return score;
  }

  function selectBestLocalMatch(sourceTrack, candidates, minScore) {
    const threshold = Number.isFinite(Number(minScore)) ? Number(minScore) : 16;
    const list = Array.isArray(candidates) ? candidates : [];
    let best = null;
    let bestScore = -1;
    for (const candidate of list) {
      const score = scoreLocalTrackCandidate(candidate, sourceTrack);
      if (score > bestScore) {
        bestScore = score;
        best = candidate;
      }
    }
    if (bestScore < threshold) return null;
    return best;
  }

  const api = {
    scoreLocalTrackCandidate,
    selectBestLocalMatch,
  };

  if (typeof module !== "undefined" && module.exports) {
    module.exports = api;
  }
  if (root) {
    root.playbackMatch = api;
  }
})(typeof globalThis !== "undefined" ? globalThis : this);
