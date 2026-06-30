(function (root) {
  function normalizePath(value) {
    return String(value || "")
      .replaceAll("\\", "/")
      .replace(/\/+/g, "/")
      .replace(/\/$/, "")
      .toLowerCase();
  }

  function pathMatchesAnyRoot(filePath, roots) {
    const fp = normalizePath(filePath);
    if (!fp) return false;
    const list = Array.isArray(roots) ? roots : [];
    return list.some((rootPath) => {
      const root = normalizePath(rootPath);
      if (!root) return false;
      return fp === root || fp.startsWith(`${root}/`);
    });
  }

  function getPlaybackSourceLabel(input) {
    const origin = String(input && input.origin || "");
    const isExternalOrigin = origin === "usb" || origin === "history";
    const libraryResolved = !!(input && input.libraryResolved);
    const hasUsbContext = !!(input && input.hasUsbContext);
    const roots = input && input.sourceRoots || [];
    const resolvedPath = input && input.resolvedPath || "";
    const inConfiguredRoots = pathMatchesAnyRoot(resolvedPath, roots);

    if (libraryResolved && inConfiguredRoots) {
      return isExternalOrigin ? "Library (matched)" : "Library";
    }
    return isExternalOrigin && hasUsbContext ? "USB" : "Local file";
  }

  const api = {
    getPlaybackSourceLabel,
    pathMatchesAnyRoot,
  };

  if (typeof module !== "undefined" && module.exports) {
    module.exports = api;
  }
  if (root) {
    root.playbackSourceLabel = api;
  }
})(typeof globalThis !== "undefined" ? globalThis : this);
