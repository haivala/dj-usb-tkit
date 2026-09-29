export function openExternalUrl(window, url) {
  if (window.__TAURI__?.opener?.openUrl) {
    window.__TAURI__.opener.openUrl(url);
  } else if (window.__TAURI_INTERNALS__?.invoke) {
    window.__TAURI_INTERNALS__.invoke("plugin:opener|open_url", { url });
  } else {
    window.open(url, "_blank");
  }
}

export function cssEscape(value) {
  const text = String(value || "");
  return typeof globalThis.CSS?.escape === "function"
    ? globalThis.CSS.escape(text)
    : text.replace(/["\\]/g, "\\$&");
}

// A fresh copy of the (single) element inside `<template id="…">` in index.html,
// owned by `doc` (a plain clone would belong to the template's inert document).
export function cloneTemplate(doc, id) {
  return doc.importNode(doc.getElementById(id).content.firstElementChild, true);
}
