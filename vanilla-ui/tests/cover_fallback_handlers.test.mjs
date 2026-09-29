import test from "node:test";
import assert from "node:assert/strict";
import { attachCoverFallbackHandlers } from "../components/library/actions.mjs";
import { renderTrackTable } from "../track_table.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

test("attachCoverFallbackHandlers advances fallback queue and replaces with placeholder", () => {
  const ctx = makeTestCtx();
  const cell = ctx.el.libraryTableBody;
  cell.innerHTML = '<img class="cover-thumb" src="original" data-fallbacks="next-a|next-b" />';
  const image = cell.querySelector("img");

  attachCoverFallbackHandlers(cell);
  assert.equal(image.dataset.fallbackBound, "1");

  const fail = () => image.dispatchEvent(new ctx.window.Event("error"));
  fail();
  assert.equal(image.getAttribute("src"), "next-a");
  assert.equal(image.dataset.fallbacks, "next-b");

  fail();
  assert.equal(image.getAttribute("src"), "next-b");
  assert.equal(image.dataset.fallbacks, "");

  fail();
  const placeholder = cell.querySelector(".cover-thumb");
  assert.equal(placeholder.tagName, "DIV");
  assert.equal(placeholder.getAttribute("aria-hidden"), "true");
});

test("renderTrackTable wires cover fallback handlers after row render", async () => {
  const ctx = makeTestCtx();
  const tbody = ctx.el.usbPlaylistTracks;
  await renderTrackTable(ctx, tbody, [
    { id: "a", title: "A", artworkDataUrl: "data:image/png;base64,AAA" },
    { id: "b", title: "B" },
  ], { origin: "usb" });

  assert.equal(tbody.querySelectorAll(".track-grid-row").length, 2);
  const img = tbody.querySelector("img.cover-thumb");
  assert.equal(img.dataset.fallbackBound, "1");
});

test("renderTrackTable in append mode adds rows without clearing existing ones, offsetting indices", async () => {
  const ctx = makeTestCtx();
  const tbody = ctx.el.usbPlaylistTracks;
  await renderTrackTable(ctx, tbody, [{ id: "a" }, { id: "b" }], { origin: "usb" });
  assert.equal(tbody.querySelectorAll(".track-grid-row").length, 2);

  await renderTrackTable(ctx, tbody, [{ id: "c" }, { id: "d" }], { origin: "usb", append: true, indexOffset: 2 });

  const rows = [...tbody.querySelectorAll(".track-grid-row")];
  assert.equal(rows.length, 4, "append should add to, not replace, the previous page's rows");
  assert.deepEqual(rows.map((row) => row.dataset.trackIndex), ["0", "1", "2", "3"],
    "appended rows continue the index sequence from indexOffset");
});
