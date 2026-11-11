// The track table's columns: one layout (which optional columns show, and in
// what order) for all four track lists. The backend owns the column catalog
// and the saved layout (`get_track_columns`, `set_track_columns`, and
// `trackColumns` in `get_frontend_settings` at startup) and cleans every
// layout it gets; this module draws it.
//
// A layout is applied to the templates the rows are cloned from (tplTrackRow,
// tplTrackRowFallback) and to each grid's header row: the optional cells are
// put in the layout's order between the Track and action cells, and a hidden
// one gets `hidden`. It stays in the row, so filling a row never misses a
// cell. The rows on screen are then built again; the windowed lists hold only
// the rows around the viewport.

import { t } from "../../ui_texts.mjs";
import { cloneTemplate } from "../../ui_utils.mjs";
import { createDialogLifecycle } from "./dialog_lifecycle.mjs";
import { bindListDragReorder } from "./list_drag_reorder.mjs";
import { clearTrackSort, trackListControllerFor } from "../shell/actions.mjs";
import { isUsbOrderEditing } from "../usb/order_edit.mjs";

function trackGrids(document) {
  return [...document.querySelectorAll("[data-track-grid]")];
}

// The grid's columns: the widths (`data-width`) of its shown header cells, in order.
function setGridColumns(grid) {
  const cells = grid.querySelectorAll(".track-grid-head > .track-grid-row > :not([hidden])");
  grid.style.setProperty("--track-grid-cols", [...cells].map((cell) => cell.dataset.width).join(" "));
}

// Puts the shared header row (tplTrackGridHead) into every track grid, before
// the context collects its elements (the library's select-all box has an id).
export function buildTrackGridHeads(document) {
  for (const grid of trackGrids(document)) {
    const head = cloneTemplate(document, "tplTrackGridHead");
    if (!grid.hasAttribute("data-drag-column")) head.querySelector(".td-drag").remove();
    const box = head.querySelector("[data-select-all]");
    box.dataset.selectAll = grid.dataset.selectAllView;
    if (grid.dataset.selectAllId) box.id = grid.dataset.selectAllId;
    grid.prepend(head);
    setGridColumns(grid);
  }
}

// Puts the optional cells of `row` in `order`, before the action cell, and
// hides the ones in `hidden`.
function arrangeRow(row, order, hidden) {
  const action = row.querySelector(":scope > .td-action");
  for (const id of order) {
    const cell = row.querySelector(`:scope > .td-${id}`);
    row.insertBefore(cell, action);
    cell.hidden = hidden.has(id);
  }
}

// Shows `layout` ({ order, hidden }: the optional columns, as the backend
// cleaned them) on every grid.
export function applyTrackColumns(ctx, layout) {
  if (!Array.isArray(layout?.order)) return;
  const { document, state, tableSortState } = ctx;
  const previous = state.trackColumns;
  state.trackColumns = layout;
  if (JSON.stringify(previous) === JSON.stringify(layout)) return;
  const arrange = (row) => arrangeRow(row, layout.order, new Set(layout.hidden));
  for (const id of ["tplTrackRow", "tplTrackRowFallback"]) arrange(document.getElementById(id).content.firstElementChild);
  for (const grid of trackGrids(document)) {
    const bodyId = grid.dataset.bodyId;
    const ctl = trackListControllerFor(ctx, bodyId);
    arrange(grid.querySelector(".track-grid-head > .track-grid-row"));
    setGridColumns(grid);
    // At startup nothing is drawn yet, and an empty list has no cells to move.
    if (previous && ctl?.total) ctl.rerender();
    // A list sorted by a column now hidden goes back to its own order. (Track,
    // whose header also sorts by title, can't be hidden.)
    const sort = tableSortState[bodyId];
    if (sort && grid.querySelector(`.sortable[data-sort-key="${sort.key}"]`)?.hidden) {
      clearTrackSort(tableSortState, bodyId, grid);
      ctl?.applyHeaderSort()?.catch?.((err) => console.error("Track list re-sort failed:", err));
    }
  }
}

// Off while a USB playlist's order is edited: a header sort there is the
// playlist's pending order, which hiding its column mustn't drop. The dialog
// closes if it's open.
export function renderTrackColumnsButtons(ctx) {
  const blocked = isUsbOrderEditing(ctx.state);
  for (const button of ctx.document.querySelectorAll("[data-track-columns-button]")) {
    button.disabled = blocked;
    button.dataset.tooltip = t(blocked ? "ui-track-columns-blocked-order-edit" : "ui-track-columns-button-tooltip");
  }
  if (blocked) ctx.trackColumnsDialog?.close();
}

// The editor: one row per optional column (drag handle, checkbox, label).
// Every change is saved at once; only the newest save's answer is shown, so
// a late answer to an older one can't undo a newer change.
export function createTrackColumnsDialogController(ctx) {
  const { el, document } = ctx;
  const dialog = createDialogLifecycle(el.trackColumnsOverlay);
  let labels = new Map();
  let saveSeq = 0;

  function render(layout) {
    const hidden = new Set(layout.hidden || []);
    el.trackColumnsList.replaceChildren(...layout.order.map((id) => {
      const row = cloneTemplate(document, "tplTrackColumnItem");
      row.dataset.trackColumn = id;
      const box = row.querySelector("input");
      box.checked = !hidden.has(id);
      const labelKey = labels.get(id);
      row.querySelector(".add-to-name").textContent = labelKey ? t(labelKey) : id;
      return row;
    }));
  }

  // The layout the list shows now.
  function listedLayout() {
    const rows = [...el.trackColumnsList.querySelectorAll(".track-columns-item")];
    return {
      order: rows.map((row) => row.dataset.trackColumn),
      hidden: rows.filter((row) => !row.querySelector("input").checked).map((row) => row.dataset.trackColumn),
    };
  }

  async function save(layout) {
    const seq = ++saveSeq;
    let data;
    try {
      data = await ctx.command("set_track_columns", { layout });
    } catch (err) {
      if (seq !== saveSeq) return;
      ctx.emitStatus(err?.message || String(err));
      if (dialog.isOpen() && ctx.state.trackColumns) render(ctx.state.trackColumns);
      return;
    }
    if (seq !== saveSeq) return;
    applyTrackColumns(ctx, data.layout);
    if (dialog.isOpen() && JSON.stringify(listedLayout()) !== JSON.stringify(data.layout)) render(data.layout);
  }

  el.trackColumnsList.addEventListener("change", (event) => {
    if (event.target.closest?.("input[type='checkbox']")) save(listedLayout());
  });
  bindListDragReorder(el.trackColumnsList, {
    handleSelector: "[data-track-column-drag-handle]",
    itemSelector: ".track-columns-item",
    // The row is already where it was dropped.
    onMove: () => save(listedLayout()),
  });
  // An empty layout is the default one: the backend puts every column in its default place.
  el.trackColumnsResetBtn.addEventListener("click", () => save({ order: [], hidden: [] }));
  el.trackColumnsDoneBtn.addEventListener("click", () => dialog.close());

  return {
    close: () => dialog.close(),
    isOpen: () => dialog.isOpen(),
    async open() {
      if (isUsbOrderEditing(ctx.state)) return;
      let data;
      try {
        data = await ctx.command("get_track_columns");
      } catch (err) {
        ctx.emitStatus(err?.message || String(err));
        return;
      }
      labels = new Map(data.columns.map((c) => [c.id, c.labelKey]));
      applyTrackColumns(ctx, data.layout);
      render(data.layout);
      await dialog.open(el.trackColumnsList.querySelector("input"));
    },
  };
}

export function bindTrackColumnsEvents(ctx) {
  ctx.document.addEventListener("click", (event) => {
    const button = event.target?.closest?.("[data-track-columns-button]");
    if (!button || button.disabled) return;
    ctx.trackColumnsDialog.open().catch((err) => console.error("Column editor failed:", err));
  });
  renderTrackColumnsButtons(ctx);
}
