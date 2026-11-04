import test from "node:test";
import assert from "node:assert/strict";
import { buildTrackTableRow } from "../track_table.mjs";
import { showTrackRow } from "../components/library/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

const usb = { kind: "usb", missing: false, tooltip: "On USB: /stick/a.mp3" };
const usbOff = { kind: "usb", missing: true, tooltip: "USB not connected: /stick/a.mp3" };
const local = { kind: "local", missing: false, tooltip: "In local library: /music/b.mp3" };

function iconOf(track) {
  const ctx = makeTestCtx();
  const row = buildTrackTableRow(ctx, { id: "a", title: "A", ...track }, { origin: "local", withCheckbox: true, index: 0 });
  return row.querySelector(".td-select .track-source");
}

test("a row shows the backend's source icon and tooltip", () => {
  const icon = iconOf({ fileSource: usb });
  assert.equal(icon.dataset.tooltip, usb.tooltip);
  assert.equal(icon.classList.contains("is-missing"), false);
});

test("a USB track that isn't available is red", () => {
  const icon = iconOf({ fileSource: usbOff });
  assert.equal(icon.classList.contains("is-missing"), true);
  assert.equal(icon.dataset.tooltip, usbOff.tooltip);
});

test("USB and local tracks have different icons", () => {
  assert.notEqual(iconOf({ fileSource: usb }).innerHTML, iconOf({ fileSource: local }).innerHTML);
});

test("a row without the backend's field has no icon", () => {
  assert.equal(iconOf({}), null);
});

test("a track update keeps the source the playlist was read with", () => {
  const ctx = makeTestCtx();
  const copy = { id: "a", title: "Old", fileSource: usbOff };
  ctx.state.playlists = [{ id: "p1", name: "P1", tracks: [copy] }];
  showTrackRow(ctx, { id: "a", title: "New" });
  assert.equal(copy.title, "New");
  assert.deepEqual(copy.fileSource, usbOff);
});
