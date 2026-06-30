export function bindShellEvents(ctx) {
  const {
    state,
    el,
    document,
    window,
    sidebarExpandBtn,
    confirmDialog,
    constants,
    persistSetting,
    setStatus,
    switchView,
    handleSortHeaderClick,
    stopPlaybackIfActive
  } = ctx;
  const {
    STORAGE_KEY_SIDEBAR_COLLAPSED,
    FRONTEND_DB_KEY_SIDEBAR_COLLAPSED
  } = constants;

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

  el.navSidebar.addEventListener("click", (event) => {
    const navItem = event.target.closest(".nav-item[data-view]");
    if (!navItem) return;
    switchView(navItem.dataset.view).catch((err) => {
      console.error(err);
      setStatus(err.message);
    });
  });

  document.addEventListener("click", (event) => {
    if (!state.playbackActive) return;
    const inActiveRow = event.target?.closest?.(`.track-grid-row[data-playback-row="${state.playbackRowKey}"]`);
    if (inActiveRow) return;
    stopPlaybackIfActive().catch((err) => {
      console.warn("Failed stopping playback on outside interaction:", err);
    });
  }, true);

  document.addEventListener("click", handleSortHeaderClick);
}
