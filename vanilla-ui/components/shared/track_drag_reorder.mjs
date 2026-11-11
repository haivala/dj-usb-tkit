import { t } from "../../ui_texts.mjs";
import { createDragAutoScroller } from "../../dnd_autoscroll.mjs";

// Drag-to-reorder for a windowed track grid (the app playlist and the USB
// playlist). A row moves among the rendered rows while dragged; on drop the
// backend is told about the one move (`onMove`), then the list is re-read
// (`onSettled`).
//
// - canDrag(): whether a drag may start now (the row handle is only rendered
//   draggable when it may, this is the last check).
// - sortActive() / commitSort(): WYSIWYG -- a column sort the user is looking
//   at becomes the list's real order the moment a drag starts, so the drop
//   reorders what is on screen. commitSort may return a Promise; the move waits
//   for it.
// - onMove({ moveTrackId, beforeTrackId }) -> Promise: the single move.
//   `beforeTrackId` is null for "to the end".
// - onSettled(): re-read the list (after a move, or after a sort commit with no
//   positional change).
export function bindTrackDragReorder(body, ctl, {
  scrollContainer,
  canDrag = () => true,
  sortActive = () => false,
  commitSort = () => null,
  onMove,
  onSettled,
  emitStatus,
}) {
  let dragSourceRow = null;
  let dragOriginalOrder = null;
  let dragSortCommit = null; // Promise while a sorted-list drag commits its order, else null
  const autoScroller = createDragAutoScroller(scrollContainer);
  const reportError = (err) => {
    console.error(err);
    emitStatus(err?.message || String(err));
  };
  const settle = () => Promise.resolve(onSettled()).catch((e) => console.error(e));
  const rowIds = () => Array.from(body.querySelectorAll(".track-grid-row[data-track-id]"))
    .map((r) => r.dataset.trackId);

  body.addEventListener("dragstart", (event) => {
    const handle = event.target.closest("[data-playlist-track-drag-handle]");
    const row = handle?.closest(".track-grid-row[data-track-id]");
    if (!handle || !row || !canDrag()) {
      event.preventDefault();
      return;
    }
    dragSourceRow = row;
    dragOriginalOrder = rowIds();
    dragSortCommit = sortActive()
      ? Promise.resolve(commitSort()).catch(reportError)
      : null;
    row.classList.add("dragging");
    // Only the rows around the viewport are rendered; hold them still while
    // this row is moved among them (autoscroll only adds rows).
    ctl.freezeWindow(true);
    autoScroller.attachWheel();
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", row.dataset.trackId);
    }
  });

  body.addEventListener("dragover", (event) => {
    if (!dragSourceRow) return;
    event.preventDefault();
    autoScroller.update(event.clientY);
    const targetRow = event.target.closest(".track-grid-row[data-track-id]");
    if (!targetRow || targetRow === dragSourceRow) return;
    const rect = targetRow.getBoundingClientRect();
    if (event.clientY < rect.top + rect.height / 2) {
      targetRow.before(dragSourceRow);
    } else {
      targetRow.after(dragSourceRow);
    }
  });

  body.addEventListener("drop", (event) => {
    if (dragSourceRow) event.preventDefault();
  });

  body.addEventListener("dragend", () => {
    autoScroller.stop();
    autoScroller.detachWheel();
    if (!dragSourceRow) return;
    ctl.freezeWindow(false);
    dragSourceRow.classList.remove("dragging");
    const movedRow = dragSourceRow;
    const newOrder = rowIds();
    const originalOrder = dragOriginalOrder;
    const pendingCommit = dragSortCommit;
    dragSourceRow = null;
    dragOriginalOrder = null;
    dragSortCommit = null;
    if (!originalOrder || newOrder.join("\u0000") === originalOrder.join("\u0000")) {
      // No positional change. Any active sort was still committed on dragstart;
      // reconcile the loaded page with the newly committed order.
      if (pendingCommit) pendingCommit.finally(settle);
      return;
    }
    // Single move: the backend repositions this one track relative to its new
    // neighbour, over the whole list -- the DOM only holds loaded rows. The
    // next row can be a skeleton (its page not loaded yet): resolve it through
    // its slot in the list.
    const nextRow = movedRow.nextElementSibling;
    const beforeTrackId = nextRow?.classList?.contains("track-grid-row")
      ? (nextRow.dataset.trackId || ctl.view[Number(nextRow.dataset.trackIndex)]?.id || null)
      : null;
    if (nextRow && !beforeTrackId) {
      emitStatus(t("ui-playlist-track-order-still-loading"));
      settle();
      return;
    }
    const moveTrackId = movedRow.dataset.trackId;
    (async () => {
      try {
        if (pendingCommit) await pendingCommit; // the sorted order must land before the move
        await onMove({ moveTrackId, beforeTrackId });
      } catch (err) {
        reportError(err);
      } finally {
        settle();
      }
    })();
  });
}
