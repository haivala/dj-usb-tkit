import test from "node:test";
import assert from "node:assert/strict";
import { attachCoverFallbackHandlers } from "../components/library/actions.mjs";
import { buildTrackTableRow, finishTrackRows } from "../track_table.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

test("attachCoverFallbackHandlers shows the placeholder for a cover that fails to load", () => {
  const ctx = makeTestCtx();
  const cell = ctx.el.libraryTableBody;
  cell.innerHTML = '<img class="cover-thumb" src="original" />';
  const image = cell.querySelector("img");

  attachCoverFallbackHandlers(cell);
  assert.equal(image.dataset.fallbackBound, "1");

  image.dispatchEvent(new ctx.window.Event("error"));
  const placeholder = cell.querySelector(".cover-thumb");
  assert.equal(placeholder.tagName, "DIV");
  assert.equal(placeholder.getAttribute("aria-hidden"), "true");
});

test("finishTrackRows wires cover fallback handlers on the new rows", async () => {
  const ctx = makeTestCtx();
  const tbody = ctx.el.usbPlaylistTracks;
  const rows = [
    { id: "a", title: "A", artworkUrl: "data:image/png;base64,AAA" },
    { id: "b", title: "B" },
  ].map((track, index) => buildTrackTableRow(ctx, track, { origin: "usb", index }));
  tbody.append(...rows);
  await finishTrackRows(ctx, rows);

  const img = tbody.querySelector("img.cover-thumb");
  assert.equal(img.dataset.fallbackBound, "1");
});

test("buildTrackTableRow carries the list position, stripes by it, and builds a skeleton for an unloaded slot", () => {
  const ctx = makeTestCtx();
  const loaded = buildTrackTableRow(ctx, { id: "c" }, { origin: "usb", index: 7 });
  assert.equal(loaded.dataset.trackIndex, "7");
  assert.equal(loaded.classList.contains("is-alt"), true);

  const skeleton = buildTrackTableRow(ctx, undefined, { origin: "usb", index: 8 });
  assert.equal(skeleton.classList.contains("track-grid-row-skeleton"), true);
  assert.equal(skeleton.dataset.trackIndex, "8");
  assert.equal(skeleton.dataset.trackId, undefined);
  assert.equal(skeleton.classList.contains("is-alt"), false);
});
