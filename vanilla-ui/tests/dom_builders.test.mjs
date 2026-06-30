import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import {
  renderPlaylistSidebarItemContent,
  updatePlaylistPanelTitle,
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
import { escapeHtml } from "../ui_utils.mjs";

function makeDom() {
  return new JSDOM(`
    <!doctype html>
    <body>
      <template id="emptyStateTemplate">
        <div class="empty-state">
          <div class="empty-state-icon"></div>
          <div class="empty-state-heading"></div>
          <div class="empty-state-body"></div>
          <button class="empty-state-action hidden" type="button"></button>
        </div>
      </template>
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

test("renderPlaylistSidebarItemContent escapes name and shows export marker", () => {
  const html = renderPlaylistSidebarItemContent(
    { id: "p1", name: `<Mix & Match>`, lastExportedAt: "2026-04-07T10:00:00Z" },
    { escapeHtml }
  );

  assert.ok(html.includes("&lt;Mix &amp; Match&gt;"));
  assert.ok(html.includes("nav-playlist-status exported"));
  assert.ok(html.includes("Exported to USB"));
});

test("renderUsbRecentRoots toggles row visibility and renders buttons", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = {
    usbRecentRow: document.getElementById("usbRecentRow"),
    usbRecentList: document.getElementById("usbRecentList")
  };

  renderUsbRecentRoots(el, ["/USB/A", "", " /USB/B "], document);

  assert.equal(el.usbRecentRow.classList.contains("hidden"), false);
  assert.equal(el.usbRecentList.querySelectorAll("button").length, 2);
  assert.equal(el.usbRecentList.querySelector("button")?.dataset.usbRecentPath, "/USB/A");
});

test("updatePlaylistPanelTitle summarizes track count and duration", () => {
  const dom = makeDom();
  const el = { playlistPanelTitle: dom.window.document.getElementById("playlistPanelTitle") };

  updatePlaylistPanelTitle(el, {
    name: "Set A",
    tracks: [{ durationMs: 60000 }, { durationMs: 125000 }]
  }, {
    formatDurationMs: (ms) => `fmt:${ms}`
  });

  assert.equal(el.playlistPanelTitle.textContent, "Set A (2 tracks, Total time: fmt:185000)");
});

test("populatePlaylistPanel fills export status and search input", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = {
    playlistPanelTitle: document.getElementById("playlistPanelTitle"),
    playlistExportStatus: document.getElementById("playlistExportStatus"),
    playlistSearchInput: document.getElementById("playlistSearchInput")
  };
  let exportButtonUpdates = 0;

  populatePlaylistPanel(
    el,
    { playlistTrackSearch: "acid" },
    { name: "Set A", tracks: [] },
    {
      updatePlaylistPanelTitle: (playlist) => updatePlaylistPanelTitle(el, playlist, { formatDurationMs: String }),
      formatPlaylistExportStatus: () => "Last exported recently.",
      updatePlaylistExportButtons: () => { exportButtonUpdates += 1; }
    }
  );

  assert.equal(el.playlistExportStatus.textContent, "Last exported recently.");
  assert.equal(el.playlistSearchInput.value, "acid");
  assert.equal(exportButtonUpdates, 1);
});

test("updateUsbRootText renders disconnected and connected states", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = {
    usbConnectionBar: document.getElementById("usbConnectionBar"),
    usbRootPathText: document.getElementById("usbRootPathText")
  };

  updateUsbRootText(el, null, false);
  assert.equal(el.usbConnectionBar.classList.contains("hidden"), false);
  assert.equal(el.usbRootPathText.textContent, "No USB selected");
  assert.equal(el.usbRootPathText.classList.contains("usb-path-valid"), false);

  updateUsbRootText(el, "/media/USB", true);
  assert.equal(el.usbRootPathText.textContent, "/media/USB");
  assert.equal(el.usbRootPathText.classList.contains("usb-path-valid"), true);
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

  renderEmptyState(document, container, {
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
