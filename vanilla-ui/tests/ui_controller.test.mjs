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

function makeDom(body = "") {
  return new JSDOM(`<!doctype html><body>${body || `
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
  `}</body>`);
}

function elements(document, ids) {
  return Object.fromEntries(ids.map((id) => [id, document.getElementById(id)]));
}

test("setStatusText turns warning suffixes into event-log links", () => {
  const { document } = makeDom().window;
  const statusText = document.getElementById("statusText");

  setStatusText({ statusText }, "USB playlists loaded: 1 | (2 warning(s))", 2);

  assert.equal(statusText.textContent, "USB playlists loaded: 1 | (2 warning(s))");
  assert.equal(statusText.querySelector(".status-warning-link")?.textContent, "(2 warning(s))");
  assert.equal(statusText.querySelector(".status-warning-link")?.getAttribute("href"), "#");
});

test("playlist mode and selection helpers derive DOM state from app state", () => {
  const { document } = makeDom().window;
  const el = elements(document, [
    "playlistBadge",
    "badgeLabel",
    "navPlaylistList",
    "selectionCount",
    "selectionActions",
    "addSelectedBtn"
  ]);
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
  for (const action of ["add-library", "add-usb", "add-history"]) {
    assert.equal(document.querySelector(`[data-action="${action}"]`).disabled, false);
  }
  assert.equal(el.selectionCount.textContent, "2 selected");
  assert.equal(el.selectionActions.classList.contains("hidden"), false);
  assert.equal(el.addSelectedBtn.disabled, false);
});

test("USB nav and empty-state helpers follow the selected-root state", () => {
  const { document } = makeDom().window;
  const el = elements(document, ["navSidebar", "refreshUsbBtn", "refreshHistoryBtn"]);
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
  const { document } = makeDom().window;
  const el = elements(document, [
    "sourceFilterIndicator",
    "scanLibraryBtn",
    "settingsDrawer",
    "settingsBackdrop",
    "usbHealthDot",
    "usbHeaderHealthDot",
    "usbNameBadge",
    "usbNameBadgeLabel"
  ]);

  updateSourceFilterIndicator({
    sourceRoots: ["/a"],
    sourceRootEnabled: { "/a": true },
    externalMasterDbPath: "/path/to/master.db",
    masterDbEnabled: false
  }, el);
  updateScanLibraryButtonLabel({ sourceRoots: ["/a"], selectedTrackIds: new Set() }, el, {
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

test("initTooltips handles hover delay, focus escape, and viewport clamping", () => {
  const hoverDom = makeDom('<button id="target" data-tooltip="Full detail text">Hover me</button>');
  const hoverDoc = hoverDom.window.document;
  const hoverTarget = hoverDoc.getElementById("target");
  let scheduled = null;
  initTooltips({
    document: hoverDoc,
    window: {
      setTimeout: (fn) => { scheduled = fn; return 1; },
      clearTimeout: () => { scheduled = null; },
      innerWidth: 1024
    }
  });
  hoverTarget.dispatchEvent(new hoverDom.window.MouseEvent("mouseover", { bubbles: true }));
  assert.equal(typeof scheduled, "function");
  scheduled();
  const hoverTooltip = hoverDoc.getElementById("app-tooltip");
  assert.equal(hoverTooltip.textContent, "Full detail text");
  assert.equal(hoverTooltip.classList.contains("app-tooltip--visible"), true);
  assert.equal(hoverTarget.getAttribute("aria-describedby"), "app-tooltip");
  hoverTarget.dispatchEvent(new hoverDom.window.MouseEvent("mouseout", { bubbles: true, relatedTarget: hoverDoc.body }));
  assert.equal(hoverTooltip.classList.contains("app-tooltip--visible"), false);
  assert.equal(hoverTarget.hasAttribute("aria-describedby"), false);

  const focusDom = makeDom('<button id="target" data-tooltip="Focus detail">Focus me</button>');
  const focusDoc = focusDom.window.document;
  initTooltips({
    document: focusDoc,
    window: { setTimeout: () => 1, clearTimeout: () => {}, innerWidth: 1024 }
  });
  focusDoc.getElementById("target").dispatchEvent(new focusDom.window.FocusEvent("focusin", { bubbles: true }));
  const focusTooltip = focusDoc.getElementById("app-tooltip");
  assert.equal(focusTooltip.classList.contains("app-tooltip--visible"), true);
  focusDoc.dispatchEvent(new focusDom.window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  assert.equal(focusTooltip.classList.contains("app-tooltip--visible"), false);

  const clampDom = makeDom('<span id="target" data-tooltip="x"></span>');
  const clampDoc = clampDom.window.document;
  const clampTarget = clampDoc.getElementById("target");
  clampTarget.getBoundingClientRect = () => ({ top: 5, bottom: 25, left: 2, right: 20, width: 18, height: 20 });
  initTooltips({
    document: clampDoc,
    window: { setTimeout: (fn) => { fn(); return 1; }, clearTimeout: () => {}, innerWidth: 100 }
  });
  clampTarget.dispatchEvent(new clampDom.window.MouseEvent("mouseover", { bubbles: true }));
  const clampTooltip = clampDoc.getElementById("app-tooltip");
  clampTooltip.getBoundingClientRect = () => ({ width: 150, height: 24 });
  clampTarget.dispatchEvent(new clampDom.window.MouseEvent("mouseout", { bubbles: true, relatedTarget: clampDoc.body }));
  clampTarget.dispatchEvent(new clampDom.window.MouseEvent("mouseover", { bubbles: true }));
  assert.equal(clampTooltip.style.left, "8px");
  assert.equal(clampTooltip.style.top, "31px");
});
