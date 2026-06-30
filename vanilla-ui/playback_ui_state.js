(function (root) {
  function isTransportButtonPlaying(playbackState, buttonMeta) {
    if (!playbackState || !playbackState.playbackActive) return false;
    const playbackRowKey = String(playbackState.playbackRowKey || "");
    const playbackTrackId = String(playbackState.playbackTrackId || "");
    const rowKey = String(buttonMeta && buttonMeta.rowKey || "");
    const trackId = String(buttonMeta && buttonMeta.trackId || "");

    if (playbackRowKey && rowKey && playbackRowKey === rowKey) return true;
    if (playbackTrackId && trackId && playbackTrackId === trackId) return true;
    return false;
  }

  function shouldToggleStop(playbackState, rowKey, isTrackCurrentlyPlaying) {
    if (!playbackState || !playbackState.playbackActive) return false;
    const playbackRowKey = String(playbackState.playbackRowKey || "");
    const targetRowKey = String(rowKey || "");
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
