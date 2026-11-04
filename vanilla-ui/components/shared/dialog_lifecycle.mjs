// Common lifecycle for result-returning overlays. Content and validation stay
// with the caller; listeners exist only while the overlay is open.
export function createDialogLifecycle(overlay, { cancelValue = null, onClose = () => {} } = {}) {
  const document = overlay.ownerDocument;
  let resolveFn = null;
  let previousFocus = null;

  function close(result = cancelValue) {
    if (!resolveFn) return;
    const resolve = resolveFn;
    resolveFn = null;
    overlay.hidden = true;
    document.removeEventListener("keydown", onKeydown);
    overlay.removeEventListener("click", onBackdrop);
    onClose();
    if (previousFocus?.isConnected) previousFocus.focus();
    previousFocus = null;
    resolve(result);
  }

  function onKeydown(event) {
    if (event.key !== "Escape") return;
    event.preventDefault();
    close();
  }

  function onBackdrop(event) {
    if (event.target === overlay) close();
  }

  return {
    isOpen: () => !!resolveFn,
    close,
    open(focusTarget) {
      close();
      previousFocus = document.activeElement;
      const result = new Promise((resolve) => { resolveFn = resolve; });
      overlay.hidden = false;
      document.addEventListener("keydown", onKeydown);
      overlay.addEventListener("click", onBackdrop);
      focusTarget?.focus();
      return result;
    },
  };
}
