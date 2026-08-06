(function (root) {
  // Backend guarantees (see resolve_playback_source's USB-root exclusion and
  // track_id fast path) that libraryResolved:true only ever means a genuine
  // local track was found -- no path-prefix/sourceRoots check needed here
  // anymore.
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
