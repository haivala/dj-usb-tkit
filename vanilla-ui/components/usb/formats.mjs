// Player formats: per USB stick, which audio formats the players it goes
// into read, in the user's order of preference. Stored on the stick
// (drive.json) by the backend, which also judges the rows against it; this
// module only shows the list and sends the user's changes.

import { cloneTemplate } from "../../ui_utils.mjs";
import { bindListDragReorder, moveArrayItem } from "../shared/list_drag_reorder.mjs";

const HINT_WRITABLE = "Untick a format the players can't read · drag to set the order of preference";
const HINT_READONLY = "USB is read-only";
const LAST_TICKED_TOOLTIP = "At least one format stays ticked";

export async function loadUsbFormats(ctx) {
  const { state } = ctx;
  const usbRoot = state.usbRoot;
  state.usbFormats = null;
  renderUsbFormats(ctx);
  if (!usbRoot || !state.usbRootValid) return;
  const data = await ctx.command("get_usb_formats", { usbRoot });
  // A switched stick while this was loading: its own load lands instead.
  if (state.usbRoot !== usbRoot) return;
  state.usbFormats = data;
  renderUsbFormats(ctx);
}

export function renderUsbFormats(ctx) {
  const { state, el, document } = ctx;
  if (!el?.usbFormatsRow || !el?.usbFormatsList) return;
  const data = state.usbFormats;
  el.usbFormatsRow.classList.toggle("hidden", !data);
  el.usbFormatsList.replaceChildren();
  if (!data) return;
  const writable = !!data.writable;
  el.usbFormatsList.classList.toggle("is-readonly", !writable);
  if (el.usbFormatsHint) el.usbFormatsHint.textContent = writable ? HINT_WRITABLE : HINT_READONLY;
  const tickedCount = data.formats.filter((item) => item.allowed).length;
  for (const item of data.formats) {
    const chip = cloneTemplate(document, "tplUsbFormatChip");
    chip.dataset.usbFormat = item.format;
    const handle = chip.querySelector("[data-usb-format-drag-handle]");
    handle.draggable = writable;
    const box = chip.querySelector(".usb-cfg-format-check");
    box.checked = !!item.allowed;
    box.dataset.usbFormat = item.format;
    const lastTicked = item.allowed && tickedCount === 1;
    box.disabled = !writable || lastTicked;
    if (lastTicked) chip.querySelector(".usb-cfg-format-label").dataset.tooltip = LAST_TICKED_TOOLTIP;
    chip.querySelector(".usb-cfg-format-name").textContent = item.label;
    el.usbFormatsList.appendChild(chip);
  }
}

// Sends the whole ordered list; the stick's rows and the open playlist's
// heading are judged against it, so they reload.
async function saveUsbFormats(ctx, formats) {
  const { state } = ctx;
  const usbRoot = state.usbRoot;
  const previous = state.usbFormats;
  state.usbFormats = { ...previous, formats };
  renderUsbFormats(ctx);
  try {
    const data = await ctx.command("set_usb_formats", {
      usbRoot,
      formats: formats.map(({ format, allowed }) => ({ format, allowed })),
    });
    if (state.usbRoot !== usbRoot) return;
    state.usbFormats = data;
  } catch (err) {
    if (state.usbRoot === usbRoot) state.usbFormats = previous;
    throw err;
  } finally {
    renderUsbFormats(ctx);
  }
  if (ctx.usbPlaylistTracksCtl?.scopeId) ctx.usbPlaylistTracksCtl.reload();
  if (ctx.usbHistoryTracksCtl?.scopeId) ctx.usbHistoryTracksCtl.reload();
  ctx.refreshPlaylistExportSpace?.();
}

export function bindUsbFormatEvents(ctx) {
  const { state, el } = ctx;
  if (!el?.usbFormatsList) return;
  const fail = (err) => {
    console.error(err);
    ctx.emitStatus(`Save Player formats failed: ${err?.message || err}`);
  };

  el.usbFormatsHeader?.addEventListener("click", () => {
    el.usbFormatsRow?.classList.toggle("collapsed");
  });

  el.usbFormatsList.addEventListener("change", (event) => {
    const box = event.target.closest(".usb-cfg-format-check");
    if (!box || !state.usbFormats) return;
    const formats = state.usbFormats.formats.map((item) =>
      item.format === box.dataset.usbFormat ? { ...item, allowed: box.checked } : item
    );
    saveUsbFormats(ctx, formats).catch(fail);
  });

  bindListDragReorder(el.usbFormatsList, {
    handleSelector: "[data-usb-format-drag-handle]",
    itemSelector: ".usb-cfg-format-chip",
    horizontal: true,
    onMove: (fromIndex, toIndex) => {
      if (!state.usbFormats?.writable) {
        renderUsbFormats(ctx);
        return;
      }
      saveUsbFormats(ctx, moveArrayItem(state.usbFormats.formats, fromIndex, toIndex)).catch(fail);
    },
  });
}
