import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import { createTracklistExportDialogController } from "../ui_controller.mjs";

function makeController() {
  const dom = new JSDOM(`
    <!doctype html>
    <body>
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
  return { el, controller: createTracklistExportDialogController(el) };
}

const TRACKS = [
  { artist: "Artist A", title: "Title A" },
  { artist: "Artist B", title: "Title B" },
  { artist: "Artist C", title: "Title C" }
];

test("createTracklistExportDialogController opens with defaults and resolves on close", async () => {
  const { el, controller } = makeController();

  const promise = controller.open({ tracks: TRACKS, defaultTimesEnabled: true, defaultPlacement: "after" });
  assert.equal(controller.isOpen(), true);
  assert.equal(el.tracklistExportOverlay.hidden, false);
  assert.equal(el.tracklistExportTimesToggle.checked, true);
  assert.equal(el.tracklistExportPlacement.value, "after");

  controller.close({ timeMode: "after", startIndex: 0 });
  assert.deepEqual(await promise, { timeMode: "after", startIndex: 0 });
  assert.equal(controller.isOpen(), false);
  assert.equal(el.tracklistExportOverlay.hidden, true);
});

test("createTracklistExportDialogController resolves null on cancel", async () => {
  const { controller } = makeController();
  const promise = controller.open({ tracks: TRACKS });
  controller.close(null);
  assert.equal(await promise, null);
});

