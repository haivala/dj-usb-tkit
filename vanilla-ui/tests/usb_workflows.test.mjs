import test from "node:test";
import assert from "node:assert/strict";

import {
  handleUsbPlayerMenuListClick,
  renderUsbBackgroundColor,
  renderUsbPlayerMenuEditor,
  syncUsbPlayerMenuEditorControls,
} from "../components/usb/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

test("player menu single-select clears opposite list and enables proper actions", () => {
  const ctx = makeTestCtx();
  const { state, el } = ctx;
  Object.assign(state, {
    usbRoot: "/USB",
    usbRootValid: true,
    usbPlayerMenuAvailable: [
      { kind: 133, name: "BPM", origin: "both" },
      { kind: 134, name: "RATING", origin: "both" },
    ],
    usbPlayerMenuCurrent: [
      { kind: 132, name: "PLAYLIST", origin: "both", removable: false },
      { kind: 139, name: "KEY", origin: "both", removable: true },
    ],
    usbPlayerMenuAvailableSelectedKind: null,
    usbPlayerMenuCurrentSelectedKind: null,
  });

  renderUsbPlayerMenuEditor(ctx);
  handleUsbPlayerMenuListClick(ctx, "available", {
    target: el.usbPlayerMenuAvailable.querySelector(".player-menu-item[data-menu-kind='133']")
  });
  syncUsbPlayerMenuEditorControls(ctx);
  assert.equal(el.usbPlayerMenuAddBtn.disabled, false);
  assert.equal(el.usbPlayerMenuRemoveBtn.disabled, true);
  assert.equal(state.usbPlayerMenuCurrentSelectedKind, null);

  handleUsbPlayerMenuListClick(ctx, "current", {
    target: el.usbPlayerMenuCurrent.querySelector(".player-menu-item[data-menu-kind='139']")
  });
  syncUsbPlayerMenuEditorControls(ctx);
  assert.equal(el.usbPlayerMenuAddBtn.disabled, true);
  assert.equal(el.usbPlayerMenuRemoveBtn.disabled, false);
  assert.equal(el.usbPlayerMenuUpBtn.disabled, false);
  assert.equal(el.usbPlayerMenuDownBtn.disabled, true);
  assert.equal(state.usbPlayerMenuAvailableSelectedKind, null);
  assert.equal(
    el.usbPlayerMenuCurrent.querySelector(".player-menu-item[data-menu-kind='139']")?.classList.contains("is-selected"),
    true,
  );

  // Selecting a backend-flagged non-removable item disables the Remove button.
  handleUsbPlayerMenuListClick(ctx, "current", {
    target: el.usbPlayerMenuCurrent.querySelector(".player-menu-item[data-menu-kind='132']")
  });
  syncUsbPlayerMenuEditorControls(ctx);
  assert.equal(el.usbPlayerMenuRemoveBtn.disabled, true);
});

test("background colour chips select the shared colour and name both when rekordbox split them", () => {
  const ctx = makeTestCtx();
  const { state, el } = ctx;
  Object.assign(state, { usbRoot: "/USB", usbRootValid: true, usbWritable: true });
  const selected = () =>
    [...el.usbBackgroundColorChips.querySelectorAll(".bg-color-chip.is-selected")].map((c) => c.dataset.color);

  state.usbBackgroundColor = { pdbColor: 6, edbColor: 6, writable: true };
  renderUsbBackgroundColor(ctx);
  assert.equal(el.usbBackgroundColorChips.querySelectorAll(".bg-color-chip").length, 9);
  assert.deepEqual(selected(), ["6"]);
  assert.equal(el.usbBackgroundColorChips.querySelector(".bg-color-chip").disabled, false);
  assert.equal(el.usbBackgroundColorNote.classList.contains("hidden"), true);

  state.usbBackgroundColor = { pdbColor: 6, edbColor: 0, writable: true };
  renderUsbBackgroundColor(ctx);
  assert.deepEqual(selected(), []);
  assert.match(el.usbBackgroundColorNote.textContent, /Aqua for older players \(PDB\), Default for rekordbox/);

  state.usbBackgroundColor = { pdbColor: null, edbColor: 0, writable: false, reason: "no t19 page" };
  renderUsbBackgroundColor(ctx);
  assert.equal(el.usbBackgroundColorChips.querySelector(".bg-color-chip").disabled, true);
  assert.equal(el.usbBackgroundColorNote.textContent, "no t19 page");
});
