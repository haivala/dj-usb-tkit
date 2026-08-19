(function (root) {
  // Backend guarantees (see play_resolved_track/resolve_playback_source) that
  // libraryResolved:true only ever means a genuine local track was found.
  function getPlaybackSourceLabel(input) {
    const origin = String(input && input.origin || "");
    const isExternalOrigin = origin === "usb" || origin === "history";
    const libraryResolved = !!(input && input.libraryResolved);
    const hasUsbContext = !!(input && input.hasUsbContext);

    if (libraryResolved) {
      return isExternalOrigin ? "Library (matched)" : "Library";
    }
    return isExternalOrigin && hasUsbContext ? "USB" : "Local file";
  }

  const api = {
    getPlaybackSourceLabel,
  };

  if (typeof module !== "undefined" && module.exports) {
    module.exports = api;
  }
  if (root) {
    root.playbackSourceLabel = api;
  }
})(typeof globalThis !== "undefined" ? globalThis : this);
