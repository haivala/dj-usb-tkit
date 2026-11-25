import { createDragAutoScroller } from "../../dnd_autoscroll.mjs";

// Drag-to-move for a tree drawn as a flat list (the sidebar's folders and
// playlists). Each item carries `data-tree-kind` ("folder" or "playlist"),
// `data-tree-id` and `data-depth`. While dragging, the item under the pointer
// shows where the dragged one would land: before it, after it, or (a folder,
// on its middle half) inside it. On drop `onMove(item, target, placement)`
// gets `{ kind, id }` for both and "before" / "after" / "inside"; the caller
// saves it and redraws. A folder can't land in itself or what it holds: those
// rows show no drop place.
export function bindTreeDragMove(list, { handleSelector, itemSelector, onMove }) {
  let dragItem = null;
  let excluded = new Set();
  let drop = null;
  const autoScroller = createDragAutoScroller(list);
  const ref = (li) => ({ kind: li.dataset.treeKind, id: li.dataset.treeId });
  const MARKS = ["drop-before", "drop-after", "drop-inside"];

  const clearMark = () => {
    if (drop) drop.target.classList.remove(...MARKS);
    drop = null;
  };

  list.addEventListener("dragstart", (event) => {
    const handle = event.target.closest(handleSelector);
    const item = handle?.closest(itemSelector);
    if (!handle || !item) {
      event.preventDefault();
      return;
    }
    dragItem = item;
    // The dragged row and, for a folder, the rows it holds (deeper rows right after it).
    excluded = new Set([item]);
    const depth = Number(item.dataset.depth || 0);
    for (let next = item.nextElementSibling; next; next = next.nextElementSibling) {
      if (!next.matches(itemSelector) || Number(next.dataset.depth || 0) <= depth) break;
      excluded.add(next);
    }
    item.classList.add("dragging");
    autoScroller.attachWheel();
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", item.dataset.treeId || "");
    }
  });

  list.addEventListener("dragover", (event) => {
    if (!dragItem) return;
    event.preventDefault();
    autoScroller.update(event.clientY);
    const target = event.target.closest(itemSelector);
    if (!target || excluded.has(target)) {
      clearMark();
      return;
    }
    const rect = target.getBoundingClientRect();
    const at = (event.clientY - rect.top) / rect.height;
    const placement = target.dataset.treeKind === "folder"
      ? (at < 0.25 ? "before" : at > 0.75 ? "after" : "inside")
      : (at < 0.5 ? "before" : "after");
    if (drop?.target === target && drop.placement === placement) return;
    clearMark();
    drop = { target, placement };
    target.classList.add(`drop-${placement}`);
  });

  list.addEventListener("drop", (event) => {
    if (!dragItem) return;
    event.preventDefault();
    if (drop) onMove(ref(dragItem), ref(drop.target), drop.placement);
    clearMark();
  });

  list.addEventListener("dragend", () => {
    autoScroller.stop();
    autoScroller.detachWheel();
    clearMark();
    dragItem?.classList.remove("dragging");
    dragItem = null;
    excluded = new Set();
  });
}
