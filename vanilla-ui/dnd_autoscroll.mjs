// Shared auto-scroll behavior for native HTML5 drag-and-drop lists. Used by
// both the app-playlist track reorder (components/playlist/events.mjs) and
// the USB playlist sidebar reorder (components/usb/events.mjs) so a drag
// that's held near a scrollable list's edge -- or receives wheel input --
// keeps scrolling the list instead of getting stuck off-screen.
export function createDragAutoScroller(container, options = {}) {
  const { edgePx = 48, maxSpeed = 18, requestFrame, cancelFrame } = options;
  // Chromium (and other browsers) suspend requestAnimationFrame callbacks
  // for the duration of a native HTML5 drag-and-drop operation -- the OS-level
  // drag session takes over the tab's render loop, so rAF-scheduled scroll
  // ticks would simply never fire while a drag is in progress. A plain timer
  // keeps running during a drag, so that's the default scheduler here.
  const raf = requestFrame || ((cb) => window.setTimeout(cb, 16));
  const caf = cancelFrame || ((id) => window.clearTimeout(id));

  let frameId = null;
  let speed = 0;

  function tick() {
    frameId = null;
    if (speed === 0 || !container) return;
    container.scrollTop += speed;
    frameId = raf(tick);
  }

  function update(clientY) {
    if (!container) return;
    const rect = container.getBoundingClientRect();
    const distanceFromTop = clientY - rect.top;
    const distanceFromBottom = rect.bottom - clientY;

    if (distanceFromTop >= 0 && distanceFromTop < edgePx) {
      const proximity = (edgePx - distanceFromTop) / edgePx;
      speed = -Math.ceil(proximity * maxSpeed);
    } else if (distanceFromBottom >= 0 && distanceFromBottom < edgePx) {
      const proximity = (edgePx - distanceFromBottom) / edgePx;
      speed = Math.ceil(proximity * maxSpeed);
    } else {
      speed = 0;
    }

    if (speed !== 0 && frameId === null) {
      frameId = raf(tick);
    }
  }

  function stop() {
    speed = 0;
    if (frameId !== null) {
      caf(frameId);
      frameId = null;
    }
  }

  function onWheel(event) {
    if (!container) return;
    event.preventDefault();
    container.scrollTop += event.deltaY;
  }

  return {
    update,
    stop,
    attachWheel() {
      container?.addEventListener("wheel", onWheel, { passive: false });
    },
    detachWheel() {
      container?.removeEventListener("wheel", onWheel);
    }
  };
}
