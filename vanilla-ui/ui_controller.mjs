// UI coordination helpers extracted from main.js.
import { bindLibraryEvents } from "./components/library/events.mjs";
import { bindPlaylistEvents } from "./components/playlist/events.mjs";
import { bindUsbEvents } from "./components/usb/events.mjs";
import { bindSettingsEvents } from "./components/settings/events.mjs";
import { bindEventLogEvents } from "./components/event-log/events.mjs";
import { bindShellEvents } from "./components/shell/events.mjs";
import { initTooltips } from "./tooltip.mjs";

export function setStatusText(el, text, warningCount = 0) {
  const target = el.statusText;
  const doc = target.ownerDocument;
  target.textContent = "";
  const str = String(text ?? "");
  const n = Math.max(0, Number(warningCount) || 0);
  const pipeIdx = n > 0 ? str.indexOf("|") : -1;
  if (pipeIdx === -1) {
    target.append(doc.createTextNode(str));
    return;
  }
  const trailing = str.slice(pipeIdx + 1);
  const leadingSpace = trailing.match(/^\s*/)[0];
  target.append(doc.createTextNode(str.slice(0, pipeIdx + 1) + leadingSpace));
  const link = doc.createElement("a");
  link.href = "#";
  link.className = "status-warning-link";
  link.textContent = trailing.slice(leadingSpace.length);
  target.append(link);
}

export function setStatus(el, state, pushEventLog, text) {
  setStatusText(el, text);
}

export function updateActivePlaylistIndicators(state, el) {
  el.navPlaylistList.querySelectorAll(".nav-playlist-item").forEach((item) => {
    item.classList.toggle("playlist-active-mode", item.dataset.playlistId === state.currentPlaylistId);
  });
}

export function updateAddToPlaylistButtons(state, document) {
  const hasPlaylist = !!state.currentPlaylistId;
  document.querySelectorAll('[data-action="add-library"], [data-action="add-usb"], [data-action="add-history"]').forEach((btn) => {
    btn.disabled = !hasPlaylist;
  });
  const addSelectedBtn = document.getElementById("addSelectedBtn");
  if (addSelectedBtn) addSelectedBtn.disabled = !hasPlaylist;
}

export function updateModeText(state, el, deps) {
  const { getCurrentPlaylist, updateAddToPlaylistButtons, updateActivePlaylistIndicators } = deps;
  const current = getCurrentPlaylist();
  if (!current) {
    el.playlistBadge.className = "playlist-badge inactive";
    el.badgeLabel.textContent = "No active playlist";
    updateAddToPlaylistButtons();
    updateActivePlaylistIndicators();
    return;
  }

  el.playlistBadge.className = "playlist-badge active";
  el.badgeLabel.textContent = current.name;
  updateAddToPlaylistButtons();
  updateActivePlaylistIndicators();
}

export function updateSelectionCount(state, el) {
  const count = state.selectedTrackIds.size;
  el.selectionCount.textContent = count > 0 ? `${count} selected` : "";
  el.selectionActions.classList.toggle("hidden", count === 0);
  el.addSelectedBtn.disabled = !state.currentPlaylistId || count === 0;
}

export function updateUsbSubNavDisabledState(state, el, deps) {
  const { switchView } = deps;
  const hasRoot = !!state.usbRoot && !!state.usbRootValid;
  el.navSidebar.querySelectorAll('.nav-sub-item[data-view^="usb-"]').forEach((btn) => {
    btn.classList.toggle("revealed", hasRoot);
  });
  if (el.refreshUsbBtn) el.refreshUsbBtn.disabled = !hasRoot;
  if (el.refreshHistoryBtn) el.refreshHistoryBtn.disabled = !hasRoot;
  if (!hasRoot && (state.activeTab === "usb-playlists" || state.activeTab === "usb-history" || state.activeTab === "usb-player-menu")) {
    switchView("usb").catch(() => {});
  }
}

export function updateUsbEmptyState(state, document, deps) {
  const { renderEmptyState } = deps;
  const container = document.getElementById("usbEmptyState");
  if (!container) return;
  const hasValidRoot = !!state.usbRoot && !!state.usbRootValid;
  const hasRecents = Array.isArray(state.usbRecentRoots) && state.usbRecentRoots.length > 0;
  container.innerHTML = "";
  if (!hasValidRoot && !hasRecents) {
    renderEmptyState(container, {
      icon: "\u2B58",
      heading: "Connect a USB drive to browse and export",
      actionLabel: "Select USB Folder",
      onAction: () => document.getElementById("selectUsbFolderBtn")?.click()
    });
  }
}

export function updateSourceFilterIndicator(state, el) {
  if (!el.sourceFilterIndicator) return;
  const anyUnchecked = state.sourceRoots.some((root) => state.sourceRootEnabled[root] === false);
  const masterDbFiltered = !!(state.externalMasterDbPath && !state.masterDbEnabled);
  const missingRoots = state.missingSourceRoots instanceof Set
    ? state.missingSourceRoots.size
    : (Array.isArray(state.missingSourceRoots) ? state.missingSourceRoots.length : 0);
  el.sourceFilterIndicator.classList.toggle("active", anyUnchecked || masterDbFiltered || missingRoots > 0);
}

export function updateScanLibraryButtonLabel(state, el, deps) {
  const { scanLibraryButtonLabel } = deps;
  if (!el.scanLibraryBtn) return;
  el.scanLibraryBtn.textContent = scanLibraryButtonLabel(state.sourceRoots);
}

export function closeSettingsDrawer(el) {
  el.settingsDrawer.classList.add("hidden");
  el.settingsBackdrop.classList.add("hidden");
}

export function updateUsbHealthDot(el, status) {
  if (!el.usbHealthDot) return;
  el.usbHealthDot.classList.remove("health-pass", "health-warn", "health-fail");
  const setTooltip = (text) => {
    el.usbHealthDot.dataset.tooltip = text;
    el.usbHealthDot.setAttribute("aria-label", text);
  };
  if (status === "PASS") {
    el.usbHealthDot.classList.add("health-pass");
    setTooltip("USB health: good");
  } else if (status === "WARN") {
    el.usbHealthDot.classList.add("health-warn");
    setTooltip("USB health: warnings");
  } else if (status === "FAIL") {
    el.usbHealthDot.classList.add("health-fail");
    setTooltip("USB health: issues found");
  } else {
    setTooltip("USB health: unknown");
  }
}

export function syncLibraryOnboardingMode(state, document) {
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
    const doc = select.ownerDocument;
    select.textContent = "";
    (tracks || []).forEach((track, index) => {
      const option = doc.createElement("option");
      option.value = String(index);
      const label = `${index + 1}. ${track?.artist || ""} - ${track?.title || ""}`;
      option.textContent = label.length > 64 ? `${label.slice(0, 63)}…` : label;
      select.append(option);
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
  const { state, el } = ctx;

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
  bindLibraryEvents(ctx);
  bindUsbEvents(ctx);
  bindPlaylistEvents(ctx);
}

export function createBindEventsContext(state, el, deps = {}) {
  return {
    state,
    el,
    ...deps
  };
}
