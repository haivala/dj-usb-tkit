// USB audio formats: per USB stick, which audio formats export puts on it, in
// the user's order of preference. Stored on the stick
// (drive.json) by the backend, which also judges the rows against it; this
// module only shows the list and sends the user's changes.

import { cloneTemplate } from "../../ui_utils.mjs";
import { bindListDragReorder, moveArrayItem } from "../shared/list_drag_reorder.mjs";


export async function loadUsbAudioFormats(ctx) {
  const { state } = ctx;
  const usbRoot = state.usbRoot;
  state.usbAudioFormats = null;
  renderUsbAudioFormats(ctx);
  // Another USB (or none): the open playlist follows it.
  ctx.reloadPlaylistForUsb();
  if (!usbRoot || !state.usbRootValid) return;
  const data = await ctx.command("get_usb_audio_formats", { usbRoot });
  // A switched stick while this was loading: its own load lands instead.
  if (state.usbRoot !== usbRoot) return;
  state.usbAudioFormats = data;
  renderUsbAudioFormats(ctx);
}

export function renderUsbAudioFormats(ctx) {
  const { state, el, document } = ctx;
  const data = state.usbAudioFormats;
  el.usbAudioFormatsRow.classList.toggle("hidden", !data);
  el.usbAudioFormatsList.replaceChildren();
  if (!data) return;
  const writable = !!data.writable;
  el.usbAudioFormatsList.classList.toggle("is-readonly", !writable);
  el.usbAudioFormatsHint.textContent = data.hint || "";
  for (const item of data.formats) {
    const chip = cloneTemplate(document, "tplUsbAudioFormatChip");
    chip.dataset.usbAudioFormat = item.format;
    const handle = chip.querySelector("[data-usb-audio-format-drag-handle]");
    handle.draggable = writable;
    const box = chip.querySelector(".usb-cfg-audio-format-check");
    box.checked = !!item.allowed;
    box.dataset.usbAudioFormat = item.format;
    box.disabled = !!item.locked;
    if (item.tooltip) chip.querySelector(".usb-cfg-audio-format-label").dataset.tooltip = item.tooltip;
    chip.querySelector(".usb-cfg-audio-format-name").textContent = item.label;
    el.usbAudioFormatsList.appendChild(chip);
  }
}

// Sends the whole ordered list and shows what the backend saved (a refused
// change shows the list as it was); the stick's rows and the open playlist
// are judged against it, so they reload.
async function saveUsbAudioFormats(ctx, formats) {
  const { state } = ctx;
  const usbRoot = state.usbRoot;
  try {
    const data = await ctx.command("set_usb_audio_formats", {
      usbRoot,
      formats: formats.map(({ format, allowed }) => ({ format, allowed })),
    });
    if (state.usbRoot !== usbRoot) return;
    state.usbAudioFormats = data;
  } finally {
    renderUsbAudioFormats(ctx);
  }
  if (ctx.usbPlaylistTracksCtl.scopeId) ctx.usbPlaylistTracksCtl.reload();
  if (ctx.usbHistoryTracksCtl.scopeId) ctx.usbHistoryTracksCtl.reload();
  ctx.reloadPlaylistForUsb();
}

export function bindUsbAudioFormatEvents(ctx) {
  const { state, el } = ctx;
  const fail = (err) => {
    console.error(err);
    ctx.emitStatus(err?.message || String(err));
  };

  el.usbAudioFormatsHeader.addEventListener("click", () => {
    el.usbAudioFormatsRow.classList.toggle("collapsed");
  });

  el.usbAudioFormatsList.addEventListener("change", (event) => {
    const box = event.target.closest(".usb-cfg-audio-format-check");
    if (!box || !state.usbAudioFormats) return;
    const formats = state.usbAudioFormats.formats.map((item) =>
      item.format === box.dataset.usbAudioFormat ? { ...item, allowed: box.checked } : item
    );
    saveUsbAudioFormats(ctx, formats).catch(fail);
  });

  bindListDragReorder(el.usbAudioFormatsList, {
    handleSelector: "[data-usb-audio-format-drag-handle]",
    itemSelector: ".usb-cfg-audio-format-chip",
    horizontal: true,
    onMove: (fromIndex, toIndex) => {
      if (!state.usbAudioFormats?.writable) {
        renderUsbAudioFormats(ctx);
        return;
      }
      saveUsbAudioFormats(ctx, moveArrayItem(state.usbAudioFormats.formats, fromIndex, toIndex)).catch(fail);
    },
  });
}
