import { createDragAutoScroller } from "../../dnd_autoscroll.mjs";

// Drag-to-reorder for a vertical list whose items carry a drag handle. Shared
// by the sidebar playlists (components/playlist/events.mjs) and the USB
// playlists (components/usb/events.mjs). The dragged item moves live under the
// pointer; on drop `onMove(fromIndex, toIndex)` gets its old and new index
// among the items, and isn't called when it lands where it started.
export function bindListDragReorder(list, { handleSelector, itemSelector, onMove }) {
  let dragItem = null;
  let fromIndex = -1;
  const items = () => Array.from(list.querySelectorAll(itemSelector));
  const autoScroller = createDragAutoScroller(list);

  list.addEventListener("dragstart", (event) => {
    const handle = event.target.closest(handleSelector);
    const item = handle?.closest(itemSelector);
    if (!handle || !item) {
      event.preventDefault();
      return;
    }
    dragItem = item;
    fromIndex = items().indexOf(item);
    item.classList.add("dragging");
    autoScroller.attachWheel();
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", String(fromIndex));
    }
  });

  list.addEventListener("dragover", (event) => {
    if (!dragItem) return;
    event.preventDefault();
    autoScroller.update(event.clientY);
    const target = event.target.closest(itemSelector);
    if (!target || target === dragItem) return;
    const rect = target.getBoundingClientRect();
    if (event.clientY < rect.top + rect.height / 2) {
      target.before(dragItem);
    } else {
      target.after(dragItem);
    }
  });

  list.addEventListener("drop", (event) => {
    if (dragItem) event.preventDefault();
  });

  list.addEventListener("dragend", () => {
    autoScroller.stop();
    autoScroller.detachWheel();
    if (!dragItem) return;
    dragItem.classList.remove("dragging");
    const toIndex = items().indexOf(dragItem);
    const from = fromIndex;
    dragItem = null;
    fromIndex = -1;
    if (toIndex < 0 || toIndex === from) return;
    onMove(from, toIndex);
  });
}

// A copy of `list` with the item at `fromIndex` moved to `toIndex`.
export function moveArrayItem(list, fromIndex, toIndex) {
  const copy = list.slice();
  const [item] = copy.splice(fromIndex, 1);
  copy.splice(toIndex, 0, item);
  return copy;
}
