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
// the rows around the viewport. Each grid's `--track-grid-cols` is built from
// TRACK_COLUMN_WIDTHS.

import { t } from "../../ui_texts.mjs";
import { cloneTemplate } from "../../ui_utils.mjs";
import { createDialogLifecycle } from "./dialog_lifecycle.mjs";
import { bindListDragReorder } from "./list_drag_reorder.mjs";
import { clearTrackSort, trackListControllerFor } from "../shell/actions.mjs";
import { isUsbOrderEditing } from "../usb/order_edit.mjs";

// Each column's width in the grid, by column id: the one place they're set.
// The defaults in styles.css (`--track-grid-cols`) match the default layout.
export const TRACK_COLUMN_WIDTHS = {
  drag: "24px",
  select: "44px",
  cover: "52px",
  waveform: "178px",
  track: "minmax(100px, 1.6fr)",
  album: "minmax(100px, 1.2fr)",
  year: "64px",
  bpm: "78px",
  key: "58px",
  length: "8ch",
  format: "72px",
  action: "minmax(5ch, 14ch)",
};

const LEADING_COLUMNS = ["select", "cover", "waveform", "track"];

function trackGrids(document) {
  return [...document.querySelectorAll("[data-track-grid]")];
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
  }
}

// Puts the optional cells of `row` in `order`, before the action cell, and
// hides the ones in `hidden`.
function arrangeRow(row, order, hidden) {
  const action = row.querySelector(":scope > .td-action");
  if (!action) return;
  for (const id of order) {
    const cell = row.querySelector(`:scope > .td-${id}`);
    if (!cell) continue;
    row.insertBefore(cell, action);
    cell.hidden = hidden.has(id);
  }
}

const ROW_TEMPLATES = ["tplTrackRow", "tplTrackRowFallback"];

// Shows `layout` ({ order, hidden }: the optional columns, as the backend
// cleaned them) on every grid.
export function applyTrackColumns(ctx, layout) {
  if (!layout || !Array.isArray(layout.order)) return;
  const { document } = ctx;
  const previous = ctx.state.trackColumns;
  ctx.state.trackColumns = layout;
  if (previous && JSON.stringify(previous) === JSON.stringify(layout)) return;
  const hidden = new Set(layout.hidden || []);
  const shown = layout.order.filter((id) => !hidden.has(id));
  for (const id of ROW_TEMPLATES) {
    const row = document.getElementById(id)?.content.firstElementChild;
    if (row) arrangeRow(row, layout.order, hidden);
  }
  for (const grid of trackGrids(document)) {
    arrangeRow(grid.querySelector(".track-grid-head > .track-grid-row"), layout.order, hidden);
    const lead = grid.hasAttribute("data-drag-column") ? ["drag", ...LEADING_COLUMNS] : LEADING_COLUMNS;
    const widths = [...lead, ...shown, "action"].map((id) => TRACK_COLUMN_WIDTHS[id] || "auto");
    grid.style.setProperty("--track-grid-cols", widths.join(" "));
  }
  // At startup (no layout yet) nothing is drawn: the first rows come from the arranged templates.
  if (previous) {
    for (const ctl of [ctx.libraryTracksCtl, ctx.playlistTracksCtl, ctx.usbPlaylistTracksCtl, ctx.usbHistoryTracksCtl]) {
      // An empty list has no cells to move.
      if (ctl?.total) ctl.rerender();
    }
  }
  clearSortsOnHiddenColumns(ctx, hidden);
}

// A list sorted by a column that is now hidden goes back to its own order:
// the sort is cleared and the list fetched again. Every list, since the
// layout is every list's. The Track column (artist/title) can't be hidden.
function clearSortsOnHiddenColumns(ctx, hidden) {
  for (const grid of trackGrids(ctx.document)) {
    const bodyId = grid.dataset.bodyId;
    const sort = ctx.tableSortState[bodyId];
    if (!sort) continue;
    const owner = [...grid.querySelectorAll('.sortable[role="columnheader"]')].find((h) =>
      h.dataset.sortKey === sort.key || (h.dataset.sortAlt || "").split(",").includes(sort.key));
    if (!owner || ![...hidden].some((id) => owner.classList.contains(`td-${id}`))) continue;
    clearTrackSort(ctx.tableSortState, bodyId, grid);
    trackListControllerFor(ctx, bodyId)?.applyHeaderSort()
      ?.catch?.((err) => console.error("Track list re-sort failed:", err));
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
