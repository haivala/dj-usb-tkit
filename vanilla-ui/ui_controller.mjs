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
import { renderAllSelectionBars, renderSelectionBar } from "./components/shared/track_selection.mjs";
import { clearManualUpdateCheck } from "./update_check.mjs";

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
  // The selection bars' "+ Playlist" needs an active playlist too.
  renderAllSelectionBars(ctx);
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
  const connected = !!state.usbRoot && !!state.usbRootValid;
  el.usbNameBadgeLabel.textContent = name || (connected ? "Unnamed" : "Not connected");
  // Clicking the badge names or renames the connected drive.
  const badge = el.usbNameBadge;
  badge.classList.toggle("is-renamable", connected);
  badge.tabIndex = connected ? 0 : -1;
  badge.setAttribute("aria-disabled", connected ? "false" : "true");
  if (connected) badge.dataset.tooltip = name ? "Rename this drive" : "Name this drive";
  else delete badge.dataset.tooltip;
  // The USB Browser heading shows the same name.
  if (state.usbRoot && state.usbRootValid) ctx.updateUsbRootText?.(state.usbRoot, true);
}

export function updateSelectionCount(ctx) {
  renderSelectionBar(ctx, "library");
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
  if (el.usbBackupsBtn) el.usbBackupsBtn.disabled = !hasRoot;
  if (!hasRoot && (state.activeTab === "usb-playlists" || state.activeTab === "usb-history" || state.activeTab === "usb-player-menu" || state.activeTab === "backups")) {
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
  // Only an imported library can be filtered out; one never imported isn't.
  const rekordboxDbFiltered = !!(state.externalRekordboxDbPath && state.rekordboxDbImported && !state.rekordboxDbEnabled);
  const mixxxDbFiltered = !!(state.externalMixxxDbPath && state.mixxxDbImported && !state.mixxxDbEnabled);
  const missingRoots = state.missingSourceRoots instanceof Set
    ? state.missingSourceRoots.size
    : (Array.isArray(state.missingSourceRoots) ? state.missingSourceRoots.length : 0);
  el.sourceFilterIndicator.classList.toggle("active", anyUnchecked || rekordboxDbFiltered || mixxxDbFiltered || missingRoots > 0);
}

export function updateScanLibraryButtonLabel(ctx) {
  const { state, el } = ctx;
  if (!el.scanLibraryBtn) return;
  el.scanLibraryBtn.textContent = scanLibraryButtonLabel(state.sourceRoots);
}

export function closeSettingsDrawer(ctx) {
  ctx.el.settingsDrawer.classList.add("hidden");
  ctx.el.settingsBackdrop.classList.add("hidden");
  clearManualUpdateCheck(ctx);
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
// item plus `force` (the "force update" checkbox), or null when cancelled.
// The line under the select shows the chosen item's `description` -- the
// group isn't visible once the list is closed -- and the checkbox names its
// `sourceLabel`.
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
    syncDescription();
  }

  function syncDescription() {
    const item = items[Number(el.playlistImportSelect.value)];
    el.playlistImportSource.textContent = item?.description || "";
    el.playlistImportForceLabel.textContent = item?.sourceLabel
      ? `Force update track data from ${item.sourceLabel}`
      : "Force update track data";
  }

  return {
    syncDescription,
    isOpen() {
      return isOpen;
    },
    close(confirmed) {
      if (!isOpen) return;
      isOpen = false;
      el.playlistImportOverlay.hidden = true;
      const item = confirmed ? items[Number(el.playlistImportSelect.value)] : null;
      const chosen = item ? { ...item, force: !!el.playlistImportForce.checked } : null;
      const resolver = resolveFn;
      resolveFn = null;
      if (resolver) resolver(chosen);
    },
    open({ groups = [] } = {}) {
      if (isOpen) {
        this.close(false);
      }
      isOpen = true;
      el.playlistImportForce.checked = false;
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
