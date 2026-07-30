(function (root) {
  function isTransportButtonPlaying(playbackState, buttonMeta) {
    if (!playbackState) return false;
    const rowKey = String(buttonMeta && buttonMeta.rowKey || "");
    const trackId = String(buttonMeta && buttonMeta.trackId || "");

    if (playbackState.playbackPendingKind === "stop") return false;
    if (playbackState.playbackPendingKind === "play") {
      const pendingRowKey = String(playbackState.playbackPendingRowKey || "");
      const pendingTrackId = String(playbackState.playbackPendingTrackId || "");
      if (pendingRowKey && rowKey && pendingRowKey === rowKey) return true;
      if (pendingTrackId && trackId && pendingTrackId === trackId) return true;
      return false;
    }

    if (!playbackState.playbackActive) return false;
    const playbackRowKey = String(playbackState.playbackRowKey || "");
    const playbackTrackId = String(playbackState.playbackTrackId || "");
    if (playbackRowKey && rowKey && playbackRowKey === rowKey) return true;
    if (playbackTrackId && trackId && playbackTrackId === trackId) return true;
    return false;
  }

  function shouldToggleStop(playbackState, rowKey, isTrackCurrentlyPlaying) {
    if (!playbackState) return false;
    const targetRowKey = String(rowKey || "");

    if (playbackState.playbackPendingKind === "stop") return false;
    if (playbackState.playbackPendingKind === "play") {
      const pendingRowKey = String(playbackState.playbackPendingRowKey || "");
      if (pendingRowKey && targetRowKey && pendingRowKey === targetRowKey) return true;
      return !!isTrackCurrentlyPlaying;
    }

    if (!playbackState.playbackActive) return false;
    const playbackRowKey = String(playbackState.playbackRowKey || "");
    if (playbackRowKey && targetRowKey && playbackRowKey === targetRowKey) return true;
    return !!isTrackCurrentlyPlaying;
  }

  const api = {
    isTransportButtonPlaying,
    shouldToggleStop,
  };

  if (typeof module !== "undefined" && module.exports) {
    module.exports = api;
  }
  if (root) {
    root.playbackUiState = api;
  }
})(typeof globalThis !== "undefined" ? globalThis : this);
