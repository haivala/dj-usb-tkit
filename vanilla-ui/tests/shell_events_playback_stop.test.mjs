import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { bindShellEvents } from "../components/shell/events.mjs";

function setup(playbackRowKey) {
  const dom = new JSDOM(`
    <!doctype html>
    <body>
      <nav id="navSidebar"></nav>
      <div id="unrelated">unrelated</div>
      <div class="track-grid-row" data-playback-row="row-active">
        <button data-action="play-library" data-row-key="row-active">active-row-play</button>
        <span class="track-title">Active Row Title</span>
      </div>
      <div class="track-grid-row" data-playback-row="row-other">
        <button data-action="play-library" data-row-key="row-other">other-row-play</button>
      </div>
    </body>
  `);
  const document = dom.window.document;
  const state = { playbackActive: true, playbackRowKey };
  let stopCalls = 0;
  const stopPlaybackIfActive = () => {
    stopCalls += 1;
    return Promise.resolve();
  };

  bindShellEvents({
    state,
    el: { navSidebar: document.getElementById("navSidebar") },
    document,
    window: dom.window,
    sidebarExpandBtn: null,
    confirmDialog: { isOpen: () => false, close: () => {} },
    constants: {},
    persistSetting: () => {},
    setStatus: () => {},
    switchView: async () => {},
    handleSortHeaderClick: () => {},
    stopPlaybackIfActive
  });

  return { document, state, getStopCalls: () => stopCalls };
}

function click(document, element) {
  element.dispatchEvent(new document.defaultView.MouseEvent("click", { bubbles: true, cancelable: true }));
}

test("global click-outside stop does not fire for a transport/play click on a different row", () => {
  const { document, getStopCalls } = setup("row-active");
  const otherRowPlayBtn = document.querySelector('.track-grid-row[data-playback-row="row-other"] [data-action="play-library"]');
  click(document, otherRowPlayBtn);
  assert.equal(getStopCalls(), 0);
});

test("global click-outside stop still fires for a click unrelated to any transport control", () => {
  const { document, getStopCalls } = setup("row-active");
  click(document, document.getElementById("unrelated"));
  assert.equal(getStopCalls(), 1);
});

test("global click-outside stop does not fire for a non-transport click inside the active row", () => {
  const { document, getStopCalls } = setup("row-active");
  const title = document.querySelector('.track-grid-row[data-playback-row="row-active"] .track-title');
  click(document, title);
  assert.equal(getStopCalls(), 0);
});
