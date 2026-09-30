import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import {
  fillPlaylistSidebarItem,
  populatePlaylistPanel
} from "../components/playlist/actions.mjs";
import {
  renderUsbRecentRoots,
  updateUsbRootText
} from "../components/usb/actions.mjs";
import {
  setActiveListItem,
  renderEmptyState
} from "../components/shell/actions.mjs";
import { APP_TEMPLATES, makeTestCtx } from "./test_helpers.mjs";

function makeDom() {
  return new JSDOM(`
    <!doctype html>
    <body>
      ${APP_TEMPLATES}
      <div id="usbRecentRow" class="hidden"></div>
      <div id="usbRecentList"></div>
      <div id="playlistPanelTitle"></div>
      <div id="playlistExportStatus"></div>
      <input id="playlistSearchInput" />
      <div id="usbConnectionBar" class="hidden"></div>
      <div id="usbRootPathText" class="usb-path-invalid"></div>
      <div id="container"></div>
      <div id="list">
        <button id="a" class="active"></button>
        <button id="b"></button>
      </div>
    </body>
  `);
}

test("fillPlaylistSidebarItem shows the name as text and the export marker", () => {
  const { document } = makeDom().window;
  const item = document.getElementById("tplNavPlaylistItem").content.firstElementChild.cloneNode(true).firstElementChild;
  fillPlaylistSidebarItem(item, { id: "p1", name: `<Mix & Match>`, lastExportedAt: "2026-04-07T10:00:00Z" });

  assert.equal(item.querySelector(".nav-playlist-name").textContent, "<Mix & Match>");
  const status = item.querySelector(".nav-playlist-status");
  assert.equal(status.classList.contains("exported"), true);
  assert.equal(status.dataset.tooltip, "Exported to USB");
  assert.equal(item.querySelector(".nav-playlist-delete").dataset.deletePlaylist, "p1");
});

test("renderUsbRecentRoots toggles row visibility and renders buttons", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = {
    usbRecentRow: document.getElementById("usbRecentRow"),
    usbRecentList: document.getElementById("usbRecentList")
  };

  renderUsbRecentRoots({ el, document, state: { usbRecentRoots: ["/USB/A", "", " /USB/B "] } });

  assert.equal(el.usbRecentRow.classList.contains("hidden"), false);
  assert.equal(el.usbRecentList.querySelectorAll("button").length, 2);
  assert.equal(el.usbRecentList.querySelector("button")?.dataset.usbRecentPath, "/USB/A");
});

test("populatePlaylistPanel fills export status and search input", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = {
    playlistPanelTitle: document.getElementById("playlistPanelTitle"),
    playlistExportStatus: document.getElementById("playlistExportStatus"),
    playlistSearchInput: document.getElementById("playlistSearchInput")
  };
  const ctx = makeTestCtx({ state: { ...makeTestCtx().state, playlistTrackSearch: "acid" } });

  populatePlaylistPanel(ctx, { name: "Set A", tracks: [], trackCount: 2, totalDurationMs: 61000 });

  assert.equal(ctx.el.playlistPanelTitle.textContent, "Set A (2 tracks)");
  assert.equal(ctx.el.playlistExportStatus.textContent, "Not exported yet.");
  assert.equal(ctx.el.playlistSearchInput.value, "acid");
  assert.equal(ctx.el.exportPlaylistBtn.textContent, "Select USB first");
});

test("updateUsbRootText renders the disconnected state", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = {
    usbConnectionBar: document.getElementById("usbConnectionBar"),
    usbRootPathText: document.getElementById("usbRootPathText")
  };

  updateUsbRootText({ el }, null, false);
  assert.equal(el.usbConnectionBar.classList.contains("hidden"), false);
  assert.equal(el.usbRootPathText.textContent, "No USB selected");
  assert.equal(el.usbRootPathText.classList.contains("usb-path-valid"), false);
});

test("setActiveListItem only keeps the chosen button active", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const container = document.getElementById("list");
  const activeButton = document.getElementById("b");

  setActiveListItem(container, activeButton);

  assert.equal(document.getElementById("a").classList.contains("active"), false);
  assert.equal(activeButton.classList.contains("active"), true);
});

test("renderEmptyState clones template and wires one-shot action", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const container = document.getElementById("container");
  let clicks = 0;

  renderEmptyState(container, {
    icon: "!",
    heading: "Nothing here",
    body: "Add something",
    actionLabel: "Create",
    onAction: () => { clicks += 1; }
  });

  assert.equal(container.querySelector(".empty-state-heading")?.textContent, "Nothing here");
  const button = container.querySelector(".empty-state-action");
  assert.equal(button?.classList.contains("hidden"), false);
  button?.click();
  button?.click();
  assert.equal(clicks, 1);
});
