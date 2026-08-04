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

test("open() populates the start-track select from the given tracks, defaulting to the first", () => {
  const { el, controller } = makeController();
  controller.open({ tracks: TRACKS });

  const options = Array.from(el.tracklistExportStartTrack.options);
  assert.equal(options.length, 3);
  assert.equal(options[0].value, "0");
  assert.equal(options[0].textContent, "1. Artist A - Title A");
  assert.equal(options[2].textContent, "3. Artist C - Title C");
  assert.equal(el.tracklistExportStartTrack.value, "0");
});

test("open() rebuilds the start-track select on every open (no stale options from a prior session)", () => {
  const { el, controller } = makeController();
  controller.open({ tracks: TRACKS });
  controller.close(null);
  controller.open({ tracks: [{ artist: "Solo", title: "Only" }] });

  const options = Array.from(el.tracklistExportStartTrack.options);
  assert.equal(options.length, 1);
  assert.equal(options[0].textContent, "1. Solo - Only");
});

test("open() truncates pathologically long option labels so the native dropdown can't blow out", () => {
  const { el, controller } = makeController();
  const longTitle = "x".repeat(200);
  controller.open({ tracks: [{ artist: "Artist", title: longTitle }] });

  const option = el.tracklistExportStartTrack.options[0];
  assert.equal(option.textContent.length, 64);
  assert.ok(option.textContent.endsWith("…"));
});

test("syncPlacementVisibility hides the placement row when times toggle is off", () => {
  const { el, controller } = makeController();

  el.tracklistExportTimesToggle.checked = false;
  controller.syncPlacementVisibility();
  assert.equal(el.tracklistExportPlacementRow.classList.contains("hidden"), true);

  el.tracklistExportTimesToggle.checked = true;
  controller.syncPlacementVisibility();
  assert.equal(el.tracklistExportPlacementRow.classList.contains("hidden"), false);
});

test("open() defaults show the placement row when times are enabled", () => {
  const { el, controller } = makeController();
  controller.open({ tracks: TRACKS, defaultTimesEnabled: true, defaultPlacement: "before" });
  assert.equal(el.tracklistExportPlacementRow.classList.contains("hidden"), false);
});
