import test from "node:test";
import assert from "node:assert/strict";
import { attachCoverFallbackHandlers } from "../components/library/actions.mjs";
import { renderTrackTable } from "../track_table.mjs";

test("attachCoverFallbackHandlers advances fallback queue and replaces with placeholder", () => {
  const listeners = {};
  const image = {
    dataset: { fallbacks: "next-a|next-b" },
    src: "original",
    addEventListener(name, fn) { listeners[name] = fn; },
    replaceWith(node) { image.replacedWith = node; }
  };

  const document = {
    createElement: (tag) => ({
      tagName: String(tag).toUpperCase(),
      className: "",
      attrs: {},
      setAttribute(k, v) { this.attrs[k] = v; }
    })
  };

  attachCoverFallbackHandlers({ querySelectorAll: () => [image] }, { document });

  assert.equal(image.dataset.fallbackBound, "1");
  assert.equal(typeof listeners.error, "function");

  listeners.error();
  assert.equal(image.src, "next-a");
  assert.equal(image.dataset.fallbacks, "next-b");

  listeners.error();
  assert.equal(image.src, "next-b");
  assert.equal(image.dataset.fallbacks, "");

  listeners.error();
  assert.equal(image.replacedWith.className, "cover-thumb");
  assert.equal(image.replacedWith.attrs["aria-hidden"], "true");
});

test("renderTrackTable wires cover fallback handlers after row render", () => {
  let coverAttachCalls = 0;
  let waveformCalls = 0;
  let transportCalls = 0;
  const inserts = [];

  const tbody = {
    innerHTML: "",
    insertAdjacentHTML(_where, html) { inserts.push(html); }
  };

  renderTrackTable(tbody, [{ id: "a" }, { id: "b" }], { origin: "usb" }, {
    createTrackRow: (track, opts) => `<tr data-id="${track.id}" data-index="${opts.index}"></tr>`,
    attachCoverFallbackHandlers: () => { coverAttachCalls += 1; },
    renderWaveformsIn: () => { waveformCalls += 1; },
    updateTransportButtonsInDom: () => { transportCalls += 1; },
    escapeHtml: (v) => String(v ?? ""),
    setStatus: () => {}
  });

  assert.equal(inserts.length, 2);
  assert.equal(coverAttachCalls, 1);
  assert.equal(waveformCalls, 1);
  assert.equal(transportCalls, 1);
});
