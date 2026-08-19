import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import {
  closeSettingsDrawer,
  setStatusText,
  syncLibraryOnboardingMode,
  updateActivePlaylistIndicators,
  updateAddToPlaylistButtons,
  updateModeText,
  updateScanLibraryButtonLabel,
  updateSelectionCount,
  updateSourceFilterIndicator,
  updateUsbEmptyState,
  updateUsbHealthDot,
  updateUsbNameBadge,
  updateUsbSubNavDisabledState
} from "../ui_controller.mjs";
import { initTooltips } from "../tooltip.mjs";

function makeDom() {
  return new JSDOM(`
    <!doctype html>
    <body>
      <div id="statusText"></div>
      <div id="playlistBadge" class="playlist-badge inactive"></div>
      <div id="badgeLabel"></div>
      <div id="usbNameBadge" class="usb-name-badge"></div>
      <div id="usbNameBadgeLabel"></div>
      <ul id="navPlaylistList">
        <li><button class="nav-playlist-item" data-playlist-id="p1"></button></li>
        <li><button class="nav-playlist-item" data-playlist-id="p2"></button></li>
      </ul>
      <button data-action="add-library"></button>
      <button data-action="add-usb"></button>
      <button data-action="add-history"></button>
      <button id="addSelectedBtn"></button>
      <div id="selectionCount"></div>
      <div id="selectionActions" class="hidden"></div>
      <nav id="navSidebar">
        <button class="nav-sub-item" data-view="usb-playlists"></button>
        <button class="nav-sub-item" data-view="usb-history"></button>
        <button class="nav-sub-item" data-view="usb-player-menu"></button>
      </nav>
      <button id="refreshUsbBtn"></button>
      <button id="refreshHistoryBtn"></button>
      <button id="selectUsbFolderBtn"></button>
      <div id="usbEmptyState"></div>
      <div id="sourceFilterIndicator"></div>
      <button id="scanLibraryBtn"></button>
      <div id="settingsDrawer"></div>
      <div id="settingsBackdrop"></div>
      <div id="usbHealthDot"></div>
      <div id="usbHeaderHealthDot"></div>
    </body>
  `);
}

test("setStatusText turns warning suffixes into event-log links", () => {
  const dom = makeDom();
  const statusText = dom.window.document.getElementById("statusText");

  setStatusText({ statusText }, "USB playlists loaded: 1 | (2 warning(s))", 2);

  assert.equal(statusText.textContent, "USB playlists loaded: 1 | (2 warning(s))");
  const link = statusText.querySelector(".status-warning-link");
  assert.equal(link?.textContent, "(2 warning(s))");
  assert.equal(link?.getAttribute("href"), "#");
});

test("playlist mode and selection helpers derive DOM state from app state", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = {
    playlistBadge: document.getElementById("playlistBadge"),
    badgeLabel: document.getElementById("badgeLabel"),
    navPlaylistList: document.getElementById("navPlaylistList"),
    selectionCount: document.getElementById("selectionCount"),
    selectionActions: document.getElementById("selectionActions"),
    addSelectedBtn: document.getElementById("addSelectedBtn")
  };
  let addButtonUpdates = 0;
  let activeIndicatorUpdates = 0;

  updateModeText({ currentPlaylistId: "p1" }, el, {
    getCurrentPlaylist: () => ({ id: "p1", name: "House" }),
    updateAddToPlaylistButtons: () => { addButtonUpdates += 1; },
    updateActivePlaylistIndicators: () => { activeIndicatorUpdates += 1; }
  });
  updateActivePlaylistIndicators({ currentPlaylistId: "p2" }, el);
  updateAddToPlaylistButtons({ currentPlaylistId: "p1" }, document);
  updateSelectionCount({ currentPlaylistId: "p1", selectedTrackIds: new Set(["a", "b"]) }, el);

  assert.equal(el.playlistBadge.className, "playlist-badge active");
  assert.equal(el.badgeLabel.textContent, "House");
  assert.equal(addButtonUpdates, 1);
  assert.equal(activeIndicatorUpdates, 1);
  assert.equal(document.querySelector('[data-playlist-id="p1"]').classList.contains("playlist-active-mode"), false);
  assert.equal(document.querySelector('[data-playlist-id="p2"]').classList.contains("playlist-active-mode"), true);
  assert.equal(document.querySelector('[data-action="add-library"]').disabled, false);
  assert.equal(document.querySelector('[data-action="add-usb"]').disabled, false);
  assert.equal(document.querySelector('[data-action="add-history"]').disabled, false);
  assert.equal(el.selectionCount.textContent, "2 selected");
  assert.equal(el.selectionActions.classList.contains("hidden"), false);
  assert.equal(el.addSelectedBtn.disabled, false);
});

test("USB nav and empty-state helpers follow the selected-root state", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = {
    navSidebar: document.getElementById("navSidebar"),
    refreshUsbBtn: document.getElementById("refreshUsbBtn"),
    refreshHistoryBtn: document.getElementById("refreshHistoryBtn")
  };
  const switched = [];
  const renderPayloads = [];

  updateUsbSubNavDisabledState(
    { usbRoot: null, usbRootValid: false, activeTab: "usb-player-menu" },
    el,
    { switchView: async (view) => { switched.push(view); } }
  );
  updateUsbEmptyState(
    { usbRoot: null, usbRootValid: false, usbRecentRoots: [] },
    document,
    { renderEmptyState: (_container, payload) => renderPayloads.push(payload) }
  );

  assert.equal(el.refreshUsbBtn.disabled, true);
  assert.equal(el.refreshHistoryBtn.disabled, true);
  assert.deepEqual(switched, ["usb"]);
  assert.equal(renderPayloads[0].heading, "Connect a USB drive to browse and export");

  updateUsbSubNavDisabledState(
    { usbRoot: "/USB", usbRootValid: true, activeTab: "usb" },
    el,
    { switchView: async () => {} }
  );
  assert.equal(document.querySelector('.nav-sub-item[data-view="usb-playlists"]').classList.contains("revealed"), true);
  assert.equal(el.refreshUsbBtn.disabled, false);
});

test("source, settings, health, name badge, and onboarding helpers update compact UI state", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const el = {
    sourceFilterIndicator: document.getElementById("sourceFilterIndicator"),
    scanLibraryBtn: document.getElementById("scanLibraryBtn"),
    settingsDrawer: document.getElementById("settingsDrawer"),
    settingsBackdrop: document.getElementById("settingsBackdrop"),
    usbHealthDot: document.getElementById("usbHealthDot"),
    usbHeaderHealthDot: document.getElementById("usbHeaderHealthDot"),
    usbNameBadge: document.getElementById("usbNameBadge"),
    usbNameBadgeLabel: document.getElementById("usbNameBadgeLabel")
  };

  updateSourceFilterIndicator({
    sourceRoots: ["/a"],
    sourceRootEnabled: { "/a": true },
    externalMasterDbPath: "/path/to/master.db",
    masterDbEnabled: false
  }, el);
  updateScanLibraryButtonLabel({ sourceRoots: ["/a"] }, el, {
    scanLibraryButtonLabel: (roots) => `Scan ${roots.length}`
  });
  closeSettingsDrawer(el);
  updateUsbHealthDot(el, "WARN");
  updateUsbNameBadge({ usbDeviceName: "Club Stick" }, el);
  syncLibraryOnboardingMode({ activeTab: "library", sourceRoots: [] }, document);

  assert.equal(el.sourceFilterIndicator.classList.contains("active"), true);
  assert.equal(el.scanLibraryBtn.textContent, "Scan 1");
  assert.equal(el.settingsDrawer.classList.contains("hidden"), true);
  assert.equal(el.settingsBackdrop.classList.contains("hidden"), true);
  assert.equal(el.usbHealthDot.classList.contains("health-warn"), true);
  assert.equal(el.usbHealthDot.dataset.tooltip, "USB health: warnings");
  assert.equal(el.usbHeaderHealthDot.classList.contains("health-warn"), true);
  assert.equal(el.usbNameBadgeLabel.textContent, "Club Stick");
  assert.equal(document.body.classList.contains("library-onboarding"), true);
});

test("initTooltips shows a custom tooltip after delay and hides on mouseout", () => {
  const dom = new JSDOM(`
    <!doctype html>
    <body>
      <button id="target" data-tooltip="Full detail text">Hover me</button>
    </body>
  `);
  const { document } = dom.window;
  const target = document.getElementById("target");
  let scheduled = null;

  initTooltips({
    document,
    window: {
      setTimeout: (fn) => {
        scheduled = fn;
        return 1;
      },
      clearTimeout: () => {
        scheduled = null;
      },
      innerWidth: 1024
    }
  });

  target.dispatchEvent(new dom.window.MouseEvent("mouseover", { bubbles: true }));
  assert.equal(typeof scheduled, "function");
  scheduled();

  const tooltipEl = document.getElementById("app-tooltip");
  assert.equal(tooltipEl.textContent, "Full detail text");
  assert.equal(tooltipEl.classList.contains("app-tooltip--visible"), true);
  assert.equal(target.getAttribute("aria-describedby"), "app-tooltip");

  target.dispatchEvent(new dom.window.MouseEvent("mouseout", { bubbles: true, relatedTarget: document.body }));
  assert.equal(tooltipEl.classList.contains("app-tooltip--visible"), false);
  assert.equal(target.hasAttribute("aria-describedby"), false);
});

test("initTooltips shows immediately on focus and hides on Escape", () => {
  const dom = new JSDOM(`
    <!doctype html>
    <body>
      <button id="target" data-tooltip="Focus detail">Focus me</button>
    </body>
  `);
  const { document } = dom.window;
  const target = document.getElementById("target");

  initTooltips({
    document,
    window: { setTimeout: () => 1, clearTimeout: () => {}, innerWidth: 1024 }
  });

  target.dispatchEvent(new dom.window.FocusEvent("focusin", { bubbles: true }));
  const tooltipEl = document.getElementById("app-tooltip");
  assert.equal(tooltipEl.classList.contains("app-tooltip--visible"), true);

  document.dispatchEvent(new dom.window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  assert.equal(tooltipEl.classList.contains("app-tooltip--visible"), false);
});

test("initTooltips clamps tooltip position within the viewport", () => {
  const dom = new JSDOM(`
    <!doctype html>
    <body>
      <span id="target" data-tooltip="x"></span>
    </body>
  `);
  const { document } = dom.window;
  const target = document.getElementById("target");
  target.getBoundingClientRect = () => ({ top: 5, bottom: 25, left: 2, right: 20, width: 18, height: 20 });

  initTooltips({
    document,
    window: { setTimeout: (fn) => { fn(); return 1; }, clearTimeout: () => {}, innerWidth: 100 }
  });
  target.dispatchEvent(new dom.window.MouseEvent("mouseover", { bubbles: true }));

  const tooltipEl = document.getElementById("app-tooltip");
  tooltipEl.getBoundingClientRect = () => ({ width: 150, height: 24 });

  target.dispatchEvent(new dom.window.MouseEvent("mouseout", { bubbles: true, relatedTarget: document.body }));
  target.dispatchEvent(new dom.window.MouseEvent("mouseover", { bubbles: true }));

  assert.equal(tooltipEl.style.left, "8px");
  assert.equal(tooltipEl.style.top, "31px");
});
