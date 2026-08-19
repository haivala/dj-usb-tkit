import test from "node:test";
import assert from "node:assert/strict";
import { createDragAutoScroller } from "../dnd_autoscroll.mjs";

class FakeContainer extends EventTarget {
  constructor(rect) {
    super();
    this.scrollTop = 0;
    this._rect = rect;
  }

  getBoundingClientRect() {
    return this._rect;
  }
}

function frameQueue() {
  const pending = [];
  return {
    requestFrame: (cb) => {
      pending.push(cb);
      return pending.length;
    },
    cancelFrame: (id) => {
      pending[id - 1] = null;
    },
    flushOne() {
      const cb = pending.shift();
      if (cb) cb();
    }
  };
}

test("update() near the top edge scrolls the container upward", () => {
  const container = new FakeContainer({ top: 0, bottom: 400 });
  const { requestFrame, cancelFrame, flushOne } = frameQueue();
  const scroller = createDragAutoScroller(container, { edgePx: 48, maxSpeed: 20, requestFrame, cancelFrame });

  container.scrollTop = 100;
  scroller.update(10); // 10px from top, inside the 48px edge zone
  flushOne();

  assert.ok(container.scrollTop < 100, "scrollTop should decrease near the top edge");
});

test("update() near the bottom edge scrolls the container downward", () => {
  const container = new FakeContainer({ top: 0, bottom: 400 });
  const { requestFrame, cancelFrame, flushOne } = frameQueue();
  const scroller = createDragAutoScroller(container, { edgePx: 48, maxSpeed: 20, requestFrame, cancelFrame });

  scroller.update(395); // 5px from bottom
  flushOne();

  assert.ok(container.scrollTop > 0, "scrollTop should increase near the bottom edge");
});

test("update() away from any edge does not schedule scrolling", () => {
  const container = new FakeContainer({ top: 0, bottom: 400 });
  const { requestFrame, cancelFrame } = frameQueue();
  const scroller = createDragAutoScroller(container, { edgePx: 48, maxSpeed: 20, requestFrame, cancelFrame });

  scroller.update(200); // dead center, far from both edges
  assert.equal(container.scrollTop, 0);
});

test("stop() cancels a pending scroll frame", () => {
  const container = new FakeContainer({ top: 0, bottom: 400 });
  const { requestFrame, cancelFrame, flushOne } = frameQueue();
  const scroller = createDragAutoScroller(container, { edgePx: 48, maxSpeed: 20, requestFrame, cancelFrame });

  scroller.update(10);
  scroller.stop();
  flushOne();

  assert.equal(container.scrollTop, 0, "no scroll should apply once stopped");
});

test("wheel input scrolls the container while attached and stops once detached", () => {
  const container = new FakeContainer({ top: 0, bottom: 400 });
  const scroller = createDragAutoScroller(container);

  scroller.attachWheel();
  const wheelEvent = new Event("wheel", { cancelable: true });
  wheelEvent.deltaY = 40;
  container.dispatchEvent(wheelEvent);
  assert.equal(container.scrollTop, 40);
  assert.equal(wheelEvent.defaultPrevented, true);

  scroller.detachWheel();
  const secondWheelEvent = new Event("wheel", { cancelable: true });
  secondWheelEvent.deltaY = 40;
  container.dispatchEvent(secondWheelEvent);
  assert.equal(container.scrollTop, 40, "scrollTop should not change once wheel listener is detached");
});
