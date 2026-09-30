// UI coordination helpers extracted from main.js.
import { bindLibraryEvents } from "./components/library/events.mjs";
import { bindPlaylistEvents } from "./components/playlist/events.mjs";
import { bindUsbEvents } from "./components/usb/events.mjs";
import { bindSettingsEvents } from "./components/settings/events.mjs";
import { bindEventLogEvents } from "./components/event-log/events.mjs";
import { bindBackupsEvents } from "./components/backups/events.mjs";
import { bindShellEvents } from "./components/shell/events.mjs";
import { bindTrackDetailEvents } from "./components/track-detail/events.mjs";
import { initTooltips } from "./tooltip.mjs";
import { cloneTemplate } from "./ui_utils.mjs";
import { renderEmptyState } from "./components/shell/actions.mjs";
import { scanLibraryButtonLabel } from "./components/library/actions.mjs";

export function updateActivePlaylistIndicators(ctx) {
  const { state, el } = ctx;
  el.navPlaylistList.querySelectorAll(".nav-playlist-item").forEach((item) => {
    item.classList.toggle("playlist-active-mode", item.dataset.playlistId === state.currentPlaylistId);
  });
}

export function updateAddToPlaylistButtons(ctx) {
  const { state, document } = ctx;
  const hasPlaylist = !!state.currentPlaylistId;
  document.querySelectorAll('[data-action="add-library"], [data-action="add-usb"], [data-action="add-history"]').forEach((btn) => {
    btn.disabled = !hasPlaylist;
  });
  const addSelectedBtn = document.getElementById("addSelectedBtn");
  if (addSelectedBtn) addSelectedBtn.disabled = !hasPlaylist;
}

export function updateModeText(ctx) {
  const { el } = ctx;
  const current = ctx.getCurrentPlaylist();
  el.playlistBadge.className = current ? "playlist-badge active" : "playlist-badge inactive";
  el.badgeLabel.textContent = current ? current.name : "No active playlist";
  updateAddToPlaylistButtons(ctx);
  updateActivePlaylistIndicators(ctx);
}

export function updateUsbNameBadge(ctx) {
  const { state, el } = ctx;
  if (!el?.usbNameBadge || !el?.usbNameBadgeLabel) return;
  const name = String(state.usbDeviceName || "").trim();
  el.usbNameBadgeLabel.textContent = name || "Not connected";
}

export function updateSelectionCount(ctx) {
  const { state, el } = ctx;
  const count = state.selectedTrackIds.size;
  el.selectionCount.textContent = count > 0 ? `${count} selected` : "";
  el.selectionActions.classList.toggle("hidden", count === 0);
  el.addSelectedBtn.disabled = !state.currentPlaylistId || count === 0;
  updateScanLibraryButtonLabel(ctx);
}

export function updateUsbSubNavDisabledState(ctx) {
  const { state, el } = ctx;
  const hasRoot = !!state.usbRoot && !!state.usbRootValid;
  el.navSidebar.querySelectorAll('.nav-sub-item[data-view^="usb-"]').forEach((btn) => {
    btn.classList.toggle("revealed", hasRoot);
  });
  if (el.refreshUsbBtn) el.refreshUsbBtn.disabled = !hasRoot;
  if (el.refreshHistoryBtn) el.refreshHistoryBtn.disabled = !hasRoot;
  if (el.backupsRefreshBtn) el.backupsRefreshBtn.disabled = !hasRoot;
  if (el.openBackupsBtn) el.openBackupsBtn.disabled = !hasRoot;
  if (!hasRoot && (state.activeTab === "usb-playlists" || state.activeTab === "usb-history" || state.activeTab === "usb-player-menu")) {
    ctx.switchView("usb").catch(() => {});
  }
}

export function updateUsbEmptyState(ctx) {
  const { state, document } = ctx;
  const container = document.getElementById("usbEmptyState");
  if (!container) return;
  const hasValidRoot = !!state.usbRoot && !!state.usbRootValid;
  const hasRecents = Array.isArray(state.usbRecentRoots) && state.usbRecentRoots.length > 0;
  container.replaceChildren();
  if (!hasValidRoot && !hasRecents) {
    renderEmptyState(container, {
      icon: "\u2B58",
      heading: "Connect a USB drive to browse and export",
      actionLabel: "Select USB Folder",
      onAction: () => document.getElementById("selectUsbFolderBtn")?.click()
    });
  }
}

export function updateSourceFilterIndicator(ctx) {
  const { state, el } = ctx;
  if (!el.sourceFilterIndicator) return;
  const anyUnchecked = state.sourceRoots.some((root) => state.sourceRootEnabled[root] === false);
  const masterDbFiltered = !!(state.externalMasterDbPath && !state.masterDbEnabled);
  const mixxxDbFiltered = !!(state.externalMixxxDbPath && !state.mixxxDbEnabled);
  const missingRoots = state.missingSourceRoots instanceof Set
    ? state.missingSourceRoots.size
    : (Array.isArray(state.missingSourceRoots) ? state.missingSourceRoots.length : 0);
  el.sourceFilterIndicator.classList.toggle("active", anyUnchecked || masterDbFiltered || mixxxDbFiltered || missingRoots > 0);
}

export function updateScanLibraryButtonLabel(ctx) {
  const { state, el } = ctx;
  if (!el.scanLibraryBtn) return;
  el.scanLibraryBtn.textContent = scanLibraryButtonLabel(state.sourceRoots, state.selectedTrackIds.size);
}

export function closeSettingsDrawer(ctx) {
  ctx.el.settingsDrawer.classList.add("hidden");
  ctx.el.settingsBackdrop.classList.add("hidden");
}

export function updateUsbHealthDot(ctx, status) {
  const { el } = ctx;
  const dots = [el.usbHealthDot, el.usbHeaderHealthDot].filter(Boolean);
  if (!dots.length) return;
  const className =
    status === "PASS" ? "health-pass" :
    status === "WARN" ? "health-warn" :
    status === "FAIL" ? "health-fail" :
    null;
  const tooltip =
    status === "PASS" ? "USB health: good" :
    status === "WARN" ? "USB health: warnings" :
    status === "FAIL" ? "USB health: issues found" :
    "USB health: unknown";
  dots.forEach((dot) => {
    dot.classList.remove("health-pass", "health-warn", "health-fail");
    if (className) dot.classList.add(className);
    dot.dataset.tooltip = tooltip;
    dot.setAttribute("aria-label", tooltip);
  });
}

export function syncLibraryOnboardingMode(ctx) {
  const { state, document } = ctx;
  document.body.classList.toggle(
    "library-onboarding",
    state.activeTab === "library" && !state.sourceRoots.length
  );
}

export function createConfirmDialogController(el) {
  let confirmResolve = null;
  let confirmOpen = false;

  return {
    isOpen() {
      return confirmOpen;
    },
    close(result) {
      if (!confirmOpen) return;
      confirmOpen = false;
      el.confirmOverlay.hidden = true;
      const resolver = confirmResolve;
      confirmResolve = null;
      if (resolver) resolver(!!result);
    },
    open({ title, message, confirmLabel = "Confirm" }) {
      if (confirmOpen) {
        this.close(false);
      }
      confirmOpen = true;
      el.confirmTitle.textContent = title || "Confirm";
      el.confirmMessage.textContent = message || "";
      el.confirmOkBtn.textContent = confirmLabel;
      el.confirmOverlay.hidden = false;
      el.confirmOkBtn.focus();
      return new Promise((resolve) => {
        confirmResolve = resolve;
      });
    }
  };
}

// Picker for importing one external playlist. `open({ groups })` takes
// `[{ label, items }]` (one <optgroup> each) and resolves with the chosen
// item, or null when cancelled.
export function createPlaylistImportDialogController(el, document) {
  let resolveFn = null;
  let isOpen = false;
  let items = [];

  function populate(groups) {
    const select = el.playlistImportSelect;
    select.textContent = "";
    items = [];
    const Option = select.ownerDocument.defaultView.Option;
    for (const { label, items: groupItems } of groups) {
      if (!groupItems?.length) continue;
      const group = cloneTemplate(document, "tplSelectOptgroup");
      group.label = label;
      for (const item of groupItems) {
        group.appendChild(new Option(`${item.name} (${item.trackCount})`, String(items.length)));
        items.push(item);
      }
      select.appendChild(group);
    }
    select.selectedIndex = 0;
  }

  return {
    isOpen() {
      return isOpen;
    },
    close(confirmed) {
      if (!isOpen) return;
      isOpen = false;
      el.playlistImportOverlay.hidden = true;
      const chosen = confirmed ? items[Number(el.playlistImportSelect.value)] || null : null;
      const resolver = resolveFn;
      resolveFn = null;
      if (resolver) resolver(chosen);
    },
    open({ groups = [] } = {}) {
      if (isOpen) {
        this.close(false);
      }
      isOpen = true;
      populate(groups);
      el.playlistImportOverlay.hidden = false;
      el.playlistImportSelect.focus();
      return new Promise((resolve) => {
        resolveFn = resolve;
      });
    }
  };
}

export function createTracklistExportDialogController(el) {
  let resolveFn = null;
  let isOpen = false;

  function syncPlacementVisibility() {
    const enabled = !!el.tracklistExportTimesToggle?.checked;
    if (el.tracklistExportPlacementRow) {
      el.tracklistExportPlacementRow.classList.toggle("hidden", !enabled);
    }
  }

  function populateStartTrackOptions(tracks) {
    const select = el.tracklistExportStartTrack;
    if (!select) return;
    select.textContent = "";
    (tracks || []).forEach((track, index) => {
      const label = `${index + 1}. ${track?.artist || ""} - ${track?.title || ""}`;
      select.add(new select.ownerDocument.defaultView.Option(label.length > 64 ? `${label.slice(0, 63)}…` : label, String(index)));
    });
    select.value = "0";
  }

  return {
    isOpen() {
      return isOpen;
    },
    close(result) {
      if (!isOpen) return;
      isOpen = false;
      el.tracklistExportOverlay.hidden = true;
      const resolver = resolveFn;
      resolveFn = null;
      if (resolver) resolver(result || null);
    },
    open({ tracks = [], defaultTimesEnabled = true, defaultPlacement = "before" } = {}) {
      if (isOpen) {
        this.close(null);
      }
      isOpen = true;
      populateStartTrackOptions(tracks);
      el.tracklistExportTimesToggle.checked = defaultTimesEnabled;
      el.tracklistExportPlacement.value = defaultPlacement;
      syncPlacementVisibility();
      el.tracklistExportOverlay.hidden = false;
      el.tracklistExportOkBtn.focus();
      return new Promise((resolve) => {
        resolveFn = resolve;
      });
    },
    syncPlacementVisibility
  };
}

export function bindEvents(ctx) {
  const { el } = ctx;

  if (el.progressDismiss) {
    el.progressDismiss.addEventListener("click", ctx.dismissProgress);
  }
  if (el.progressPauseBtn) {
    el.progressPauseBtn.addEventListener("click", ctx.toggleAnalysisPause);
  }
  if (el.progressCancelAnalysisBtn) {
    el.progressCancelAnalysisBtn.addEventListener("click", ctx.cancelAnalysis);
  }

  initTooltips(ctx);
  bindShellEvents(ctx);
  bindSettingsEvents(ctx);
  bindEventLogEvents(ctx);
  bindBackupsEvents(ctx);
  bindLibraryEvents(ctx);
  bindUsbEvents(ctx);
  bindPlaylistEvents(ctx);
  bindTrackDetailEvents(ctx);
}
