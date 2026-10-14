import { dismissUpdateBanner } from "../../update_check.mjs";
import {
  STORAGE_KEY_SIDEBAR_COLLAPSED,
  FRONTEND_DB_KEY_SIDEBAR_COLLAPSED,
  STORAGE_KEY_SIDEBAR_WIDTH,
  FRONTEND_DB_KEY_SIDEBAR_WIDTH
} from "../../settings_keys.mjs";
import { applySidebarWidth, clampSidebarWidth, SIDEBAR_WIDTH_DEFAULT } from "./actions.mjs";

export function bindShellEvents(ctx) {
  const {
    state,
    el,
    document,
    window,
    confirmDialog,
    tracklistExportDialog,
    playlistImportDialog,
    persistSetting,
    emitStatus,
    switchView,
    handleSortHeaderClick,
    stopPlaybackIfActive
  } = ctx;
  const { sidebarExpandBtn } = el;
  sidebarExpandBtn?.addEventListener("click", () => {
    state.sidebarCollapsed = false;
    el.navSidebar.classList.remove("collapsed");
    document.body.classList.remove("sidebar-collapsed");
    sidebarExpandBtn.classList.remove("visible");
    persistSetting(STORAGE_KEY_SIDEBAR_COLLAPSED, FRONTEND_DB_KEY_SIDEBAR_COLLAPSED, "0");
  });

  el.sidebarCollapseBtn?.addEventListener("click", () => {
    state.sidebarCollapsed = true;
    el.navSidebar.classList.add("collapsed");
    document.body.classList.add("sidebar-collapsed");
    sidebarExpandBtn.classList.add("visible");
    persistSetting(STORAGE_KEY_SIDEBAR_COLLAPSED, FRONTEND_DB_KEY_SIDEBAR_COLLAPSED, "1");
  });

  // Drag the sidebar's right edge to resize it; double-click resets it.
  const resizer = el.navSidebarResizer;
  const saveSidebarWidth = (width) => {
    state.sidebarWidth = clampSidebarWidth(width);
    applySidebarWidth(document, state.sidebarWidth);
    persistSetting(STORAGE_KEY_SIDEBAR_WIDTH, FRONTEND_DB_KEY_SIDEBAR_WIDTH, String(state.sidebarWidth));
  };
  resizer?.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const left = el.navSidebar.getBoundingClientRect().left;
    resizer.setPointerCapture?.(event.pointerId);
    el.navSidebar.classList.add("resizing");
    document.body.classList.add("sidebar-resizing");
    const onMove = (moveEvent) => {
      state.sidebarWidth = clampSidebarWidth(moveEvent.clientX - left);
      applySidebarWidth(document, state.sidebarWidth);
    };
    const onUp = () => {
      resizer.removeEventListener("pointermove", onMove);
      resizer.removeEventListener("pointerup", onUp);
      resizer.removeEventListener("pointercancel", onUp);
      el.navSidebar.classList.remove("resizing");
      document.body.classList.remove("sidebar-resizing");
      saveSidebarWidth(state.sidebarWidth);
    };
    resizer.addEventListener("pointermove", onMove);
    resizer.addEventListener("pointerup", onUp);
    resizer.addEventListener("pointercancel", onUp);
  });
  resizer?.addEventListener("dblclick", () => saveSidebarWidth(SIDEBAR_WIDTH_DEFAULT));

  el.confirmOkBtn?.addEventListener("click", () => confirmDialog.close(true));
  el.confirmCancelBtn?.addEventListener("click", () => confirmDialog.close(false));
  el.confirmOverlay?.addEventListener("click", (event) => {
    if (event.target === el.confirmOverlay) confirmDialog.close(false);
  });
  document.addEventListener("keydown", (event) => {
    if (!confirmDialog.isOpen()) return;
    if (event.key === "Escape") {
      event.preventDefault();
      confirmDialog.close(false);
    }
    if (event.key === "Enter" && document.activeElement === el.confirmOkBtn) {
      event.preventDefault();
      confirmDialog.close(true);
    }
  });

  document.addEventListener("keydown", (event) => {
    if (!playlistImportDialog?.isOpen()) return;
    if (event.key === "Escape") {
      event.preventDefault();
      playlistImportDialog.close(false);
    }
  });

  document.addEventListener("keydown", (event) => {
    if (!tracklistExportDialog?.isOpen()) return;
    if (event.key === "Escape") {
      event.preventDefault();
      tracklistExportDialog.close(null);
    }
  });

  el.navSidebar.addEventListener("click", (event) => {
    const navItem = event.target.closest(".nav-item[data-view]");
    if (!navItem) return;
    switchView(navItem.dataset.view).catch((err) => {
      console.error(err);
      emitStatus(err.message);
    });
  });

  el.statusText?.addEventListener("click", (event) => {
    const link = event.target.closest(".status-warning-link");
    if (!link) return;
    event.preventDefault();
    switchView("event-log").catch((err) => {
      console.error(err);
      emitStatus(err.message);
    });
  });

  document.addEventListener("click", (event) => {
    if (!state.playbackActive) return;
    if (event.target?.closest?.('[data-action="play-library"],[data-action="play-usb"],[data-action="play-history"],[data-action="scrub-play"]')) return;
    // A surface that drives its own transport (the cue editor: play/pause,
    // + Cue at the playhead, zoom) manages -- and stops -- playback itself.
    if (event.target?.closest?.("[data-playback-scope]")) return;
    const inActiveRow = event.target?.closest?.(`.track-grid-row[data-playback-row="${state.playbackRowKey}"]`);
    if (inActiveRow) return;
    stopPlaybackIfActive().catch((err) => {
      console.warn("Failed stopping playback on outside interaction:", err);
    });
  }, true);

  document.addEventListener("click", handleSortHeaderClick);

  el.updateBannerDismissBtn?.addEventListener("click", () => {
    dismissUpdateBanner(ctx);
  });
}
