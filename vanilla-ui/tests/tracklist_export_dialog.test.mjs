import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import { createTracklistExportDialogController } from "../ui_controller.mjs";

function makeController() {
  const dom = new JSDOM(`
    <!doctype html>
    <body>
      <button id="trigger">Export tracklist</button>
      <div id="tracklistExportOverlay" hidden>
        <select id="tracklistExportStartTrack"></select>
        <div id="tracklistExportPlacementRow" class="hidden">
          <select id="tracklistExportPlacement">
            <option value="before" selected>Before</option>
            <option value="after">After</option>
          </select>
        </div>
        <input type="checkbox" id="tracklistExportTimesToggle" checked />
        <button id="tracklistExportOkBtn" type="button"></button>
      </div>
    </body>
  `, { pretendToBeVisual: true });
  const { document } = dom.window;
  const el = {
    tracklistExportOverlay: document.getElementById("tracklistExportOverlay"),
    tracklistExportStartTrack: document.getElementById("tracklistExportStartTrack"),
    tracklistExportPlacementRow: document.getElementById("tracklistExportPlacementRow"),
    tracklistExportTimesToggle: document.getElementById("tracklistExportTimesToggle"),
    tracklistExportPlacement: document.getElementById("tracklistExportPlacement"),
    tracklistExportOkBtn: document.getElementById("tracklistExportOkBtn")
  };
  const trigger = document.getElementById("trigger");
  trigger.focus();
  return { document, trigger, el, controller: createTracklistExportDialogController(el) };
}

const TRACKS = [
  { artist: "Artist A", title: "Title A" },
  { artist: "Artist B", title: "Title B" },
  { artist: "Artist C", title: "Title C" }
];

test("createTracklistExportDialogController opens with defaults and resolves on close", async () => {
  const { document, trigger, el, controller } = makeController();

  const promise = controller.open({ tracks: TRACKS, defaultTimesEnabled: true, defaultPlacement: "after" });
  assert.equal(controller.isOpen(), true);
  assert.equal(el.tracklistExportOverlay.hidden, false);
  assert.equal(document.activeElement, el.tracklistExportOkBtn);
  assert.equal(el.tracklistExportTimesToggle.checked, true);
  assert.equal(el.tracklistExportPlacement.value, "after");

  controller.close({ timeMode: "after", startIndex: 0 });
  assert.deepEqual(await promise, { timeMode: "after", startIndex: 0 });
  assert.equal(controller.isOpen(), false);
  assert.equal(el.tracklistExportOverlay.hidden, true);
  assert.equal(document.activeElement, trigger);
  controller.close(null);
  assert.equal(document.activeElement, trigger);
});

test("createTracklistExportDialogController resolves null on cancel", async () => {
  const { controller } = makeController();
  const promise = controller.open({ tracks: TRACKS });
  controller.close(null);
  assert.equal(await promise, null);
});

for (const dismissal of ["Escape", "backdrop"]) {
  test(`export dialog supports repeated ${dismissal} dismissal without keeping closed-dialog handlers`, async () => {
    const { document, trigger, el, controller } = makeController();
    const dismiss = () => {
      const event = dismissal === "Escape"
        ? new document.defaultView.KeyboardEvent("keydown", { key: "Escape", cancelable: true })
        : new document.defaultView.MouseEvent("click", { bubbles: true });
      (dismissal === "Escape" ? document : el.tracklistExportOverlay).dispatchEvent(event);
      return event;
    };
    for (let i = 0; i < 3; i++) {
      const pending = controller.open({ tracks: TRACKS });
      el.tracklistExportOkBtn.click(); // Controller leaves result collection to its caller.
      assert.equal(controller.isOpen(), true);
      dismiss();
      assert.equal(await pending, null);
      assert.equal(el.tracklistExportOverlay.hidden, true);
      assert.equal(document.activeElement, trigger);
      assert.equal(dismiss().defaultPrevented, false);
    }
  });
}

test("reopening export cancels the previous request and rebuilds its options", async () => {
  const { document, trigger, el, controller } = makeController();
  const first = controller.open({ tracks: TRACKS });
  const second = controller.open({ tracks: TRACKS.slice(0, 1) });
  assert.equal(await first, null);
  assert.equal(controller.isOpen(), true);
  assert.equal(el.tracklistExportStartTrack.options.length, 1);
  const result = { timeMode: "off", startIndex: 0 };
  controller.close(result);
  assert.deepEqual(await second, result);
  assert.equal(document.activeElement, trigger);
});

test("export dialog can close after its original trigger is removed", async () => {
  const { trigger, controller } = makeController();
  const pending = controller.open({ tracks: TRACKS });
  trigger.remove();
  controller.close(null);
  assert.equal(await pending, null);
});
