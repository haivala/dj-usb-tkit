import { catchErr, handleTrackAction, trackMetaFingerprint, resolveEmitStatus } from "../shared/track_actions.mjs";

export function bindUsbEvents(ctx) {
  const {
    state,
    el,
    setStatus,
    refreshUsb,
    pickUsbFolder,
    validateAndSetUsbRoot,
    initializeUsb,
    runUsbParityReport,
    runUsbDiagnostics,
    previewUsbRepairs,
    applyUsbRepairs,
    showDiagReportView,
    refreshHistory,
    exportHistoryTracklist,
    tracklistExportDialog,
    loadUsbPlayerMenuConfig,
    syncUsbPlayerMenuEditorControls,
    handleUsbPlayerMenuListClick,
    addUsbPlayerMenuItems,
    removeUsbPlayerMenuItems,
    moveUsbPlayerMenuItems,
    syncUsbPlayerMenusEdbToPdb,
    renderUsbPlaylistTracks,
    renderHistoryTracks,
    removeUsbPlaylist,
    reorderUsbPlaylists,
    moveArrayItem,
    stopPlaybackIfActive,
    hydrateUsbTrackMetadata,
    setActiveListItem,
    getHistoryDateDisplay,
    addTracksToCurrentPlaylist,
  } = ctx;
  const patchUsbTrackRow = typeof ctx.patchUsbTrackRow === "function"
    ? ctx.patchUsbTrackRow
    : () => false;
  const patchHistoryTrackRow = typeof ctx.patchHistoryTrackRow === "function"
    ? ctx.patchHistoryTrackRow
    : () => false;
  const emitStatus = resolveEmitStatus(ctx);
  const syncPlayerMenuControls = typeof syncUsbPlayerMenuEditorControls === "function"
    ? syncUsbPlayerMenuEditorControls
    : () => {};
  const onPlayerMenuListClick = typeof handleUsbPlayerMenuListClick === "function"
    ? handleUsbPlayerMenuListClick
    : () => {};
  let usbSelectionHydrationToken = 0;
  let historySelectionHydrationToken = 0;

  const hydrateSelectionTracks = async (tracks, isSelectionCurrent, patchRow, renderFallback) => {
    let missedPatch = false;
    for (let i = 0; i < (tracks || []).length; i += 1) {
      if (!isSelectionCurrent()) return;
      const track = tracks[i];
      const before = trackMetaFingerprint(track);
      await hydrateUsbTrackMetadata(track);
      const after = trackMetaFingerprint(track);
      if (after !== before) {
        const patched = patchRow(track);
        if (!patched) missedPatch = true;
      }
      if ((i + 1) % 8 === 0) {
        await Promise.resolve();
      }
    }
    if (missedPatch) renderFallback();
  };

  el.refreshUsbBtn.addEventListener("click", () => {
    refreshUsb().catch(catchErr(emitStatus));
  });

  el.selectUsbFolderBtn?.addEventListener("click", () => {
    pickUsbFolder().catch(catchErr(emitStatus));
  });

  el.usbRecentList?.addEventListener("click", (event) => {
    const btn = event.target.closest("[data-usb-recent-path]");
    if (!btn) return;
    const selectedPath = String(btn.dataset.usbRecentPath || "").trim();
    if (!selectedPath) return;
    validateAndSetUsbRoot(selectedPath, false).catch(catchErr(emitStatus));
  });

  el.initializeUsbBtn.addEventListener("click", () => {
    initializeUsb().catch(catchErr(emitStatus));
  });

  el.runUsbParityBtn.addEventListener("click", () => {
    runUsbParityReport().catch(catchErr(emitStatus));
  });

  el.reDiagnoseBtn?.addEventListener("click", () => {
    runUsbDiagnostics().catch(catchErr(emitStatus));
  });

  el.previewRepairsBtn?.addEventListener("click", () => {
    previewUsbRepairs().catch(catchErr(emitStatus));
  });

  el.applyRepairsBtn?.addEventListener("click", () => {
    applyUsbRepairs().catch(catchErr(emitStatus));
  });

  el.diagBackToReportBtn?.addEventListener("click", () => {
    showDiagReportView();
  });

  el.refreshHistoryBtn.addEventListener("click", () => {
    refreshHistory().catch(catchErr(emitStatus));
  });

  el.exportHistoryTracklistBtn?.addEventListener("click", () => {
    exportHistoryTracklist().catch(catchErr(emitStatus));
  });

  el.tracklistExportTimesToggle?.addEventListener("change", () => {
    tracklistExportDialog?.syncPlacementVisibility();
  });
  el.tracklistExportOkBtn?.addEventListener("click", () => {
    const timesOn = !!el.tracklistExportTimesToggle?.checked;
    tracklistExportDialog?.close({
      timeMode: timesOn ? el.tracklistExportPlacement.value : "off",
      startIndex: Number(el.tracklistExportStartTrack?.value) || 0
    });
  });
  el.tracklistExportCancelBtn?.addEventListener("click", () => {
    tracklistExportDialog?.close(null);
  });
  el.tracklistExportOverlay?.addEventListener("click", (event) => {
    if (event.target === el.tracklistExportOverlay) {
      tracklistExportDialog?.close(null);
    }
  });

  el.usbPlayerMenuAddBtn?.addEventListener("click", () => {
    addUsbPlayerMenuItems().catch(catchErr(emitStatus));
  });

  el.usbPlayerMenuRemoveBtn?.addEventListener("click", () => {
    removeUsbPlayerMenuItems().catch(catchErr(emitStatus));
  });

  el.usbPlayerMenuUpBtn?.addEventListener("click", () => {
    moveUsbPlayerMenuItems(-1).catch(catchErr(emitStatus));
  });

  el.usbPlayerMenuDownBtn?.addEventListener("click", () => {
    moveUsbPlayerMenuItems(1).catch(catchErr(emitStatus));
  });

  el.usbPlayerMenuSyncBtn?.addEventListener("click", () => {
    syncUsbPlayerMenusEdbToPdb().catch(catchErr(emitStatus));
  });

  el.usbPlayerMenuRestoreBtn?.addEventListener("click", () => {
    syncUsbPlayerMenusEdbToPdb().catch(catchErr(emitStatus));
  });

  el.usbPlayerMenuAvailable?.addEventListener("click", (event) => {
    onPlayerMenuListClick("available", event);
    syncPlayerMenuControls();
  });
  el.usbPlayerMenuCurrent?.addEventListener("click", (event) => {
    onPlayerMenuListClick("current", event);
    syncPlayerMenuControls();
  });

  el.usbTrackSearch?.addEventListener("input", () => {
    state.usbTrackSearch = String(el.usbTrackSearch.value || "");
    renderUsbPlaylistTracks();
  });

  el.historyTrackSearch?.addEventListener("input", () => {
    state.historyTrackSearch = String(el.historyTrackSearch.value || "");
    renderHistoryTracks();
  });

  el.usbPlaylists.addEventListener("click", (event) => {
    const removeBtn = event.target.closest("[data-usb-remove-playlist]");
    if (removeBtn) {
      const removeId = removeBtn.dataset.usbRemovePlaylist;
      const playlist = state.usbPlaylists.find((item) => String(item.id) === String(removeId));
      removeUsbPlaylist(playlist).catch((err) => {
        console.error(err);
        emitStatus(`Remove USB playlist failed: ${err.message}`);
      });
      return;
    }

    const btn = event.target.closest("[data-usb-playlist-index]");
    if (!btn) return;
    stopPlaybackIfActive().catch((err) => {
      console.warn("Failed stopping playback on USB playlist change:", err);
    });
    const index = Number(btn.dataset.usbPlaylistIndex);
    const id = btn.dataset.usbPlaylist;
    const playlist = state.usbPlaylists[index]
      || state.usbPlaylists.find((item) => String(item.id) === String(id));
    state.usbPlaylistTracks = playlist?.tracks || [];
    setActiveListItem(el.usbPlaylists, btn);
    renderUsbPlaylistTracks();
    usbSelectionHydrationToken += 1;
    const token = usbSelectionHydrationToken;
    hydrateSelectionTracks(
      state.usbPlaylistTracks,
      () => usbSelectionHydrationToken === token,
      patchUsbTrackRow,
      renderUsbPlaylistTracks
    ).catch((err) => {
      console.warn("USB playlist hydration failed:", err);
    });
    if (!playlist) {
      emitStatus("Failed to resolve selected USB playlist");
      return;
    }
    emitStatus(`USB playlist selected: ${playlist.name} (${state.usbPlaylistTracks.length} tracks)`);
  });

  let dragSourceLi = null;
  let dragSourceIndex = null;

  el.usbPlaylists.addEventListener("dragstart", (event) => {
    const handle = event.target.closest("[data-usb-drag-handle]");
    const li = handle?.closest("li[data-usb-playlist-li]");
    if (!handle || !li) {
      event.preventDefault();
      return;
    }
    dragSourceLi = li;
    dragSourceIndex = Number(li.dataset.usbPlaylistLi);
    li.classList.add("dragging");
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", String(dragSourceIndex));
    }
  });

  el.usbPlaylists.addEventListener("dragover", (event) => {
    if (!dragSourceLi) return;
    event.preventDefault();
    const targetLi = event.target.closest("li[data-usb-playlist-li]");
    if (!targetLi || targetLi === dragSourceLi) return;
    const rect = targetLi.getBoundingClientRect();
    const before = event.clientY < rect.top + rect.height / 2;
    if (before) {
      targetLi.before(dragSourceLi);
    } else {
      targetLi.after(dragSourceLi);
    }
  });

  el.usbPlaylists.addEventListener("drop", (event) => {
    if (dragSourceLi) event.preventDefault();
  });

  el.usbPlaylists.addEventListener("dragend", () => {
    if (!dragSourceLi) return;
    dragSourceLi.classList.remove("dragging");
    const lis = Array.from(el.usbPlaylists.querySelectorAll("li[data-usb-playlist-li]"));
    const toIndex = lis.indexOf(dragSourceLi);
    const fromIndex = dragSourceIndex;
    dragSourceLi = null;
    dragSourceIndex = null;
    if (toIndex < 0 || toIndex === fromIndex) return;
    state.usbPlaylists = moveArrayItem(state.usbPlaylists, fromIndex, toIndex);
    reorderUsbPlaylists().catch((err) => {
      console.error(err);
      emitStatus(`Save playlist order failed: ${err.message}`);
    });
  });

  el.usbPlaylistTracks.addEventListener("click", async (event) => {
    const target = event.target.closest("[data-action]");
    const action = target?.dataset?.action;
    const row = target?.closest(".track-grid-row") || event.target.closest(".track-grid-row");
    const rowIndex = Number(row?.dataset?.trackIndex);
    const index = Number.isFinite(Number(target?.dataset?.index))
      ? Number(target?.dataset?.index)
      : rowIndex;
    const rowKey = row?.dataset?.playbackRow || null;
    const track = state.usbPlaylistTracksView[index];
    if (!track) return;

    if (!action) {
      const before = trackMetaFingerprint(track);
      await hydrateUsbTrackMetadata(track);
      if (trackMetaFingerprint(track) !== before && !patchUsbTrackRow(track)) {
        renderUsbPlaylistTracks();
      }
      return;
    }

    await hydrateUsbTrackMetadata(track);
    handleTrackAction({ action, track, origin: "usb", target, event, state, rowKey, ctx });
  });

  el.historyList.addEventListener("click", (event) => {
    const btn = event.target.closest("[data-history-index]");
    if (!btn) return;
    stopPlaybackIfActive().catch((err) => {
      console.warn("Failed stopping playback on history change:", err);
    });
    const index = btn.dataset.historyIndex;

    state.selectedHistoryIndex = Number(index);
    const history = state.histories[Number(index)];
    state.historyTracks = history?.tracks || [];
    if (el.exportHistoryTracklistBtn) el.exportHistoryTracklistBtn.disabled = !state.historyTracks.length;
    setActiveListItem(el.historyList, btn);
    renderHistoryTracks();
    historySelectionHydrationToken += 1;
    const token = historySelectionHydrationToken;
    hydrateSelectionTracks(
      state.historyTracks,
      () => historySelectionHydrationToken === token,
      patchHistoryTrackRow,
      renderHistoryTracks
    ).catch((err) => {
      console.warn("USB history hydration failed:", err);
    });
  });

  el.historyTracks.addEventListener("click", async (event) => {
    const target = event.target.closest("[data-action]");
    const action = target?.dataset?.action;
    const row = target?.closest(".track-grid-row") || event.target.closest(".track-grid-row");
    const rowIndex = Number(row?.dataset?.trackIndex);
    const index = Number.isFinite(Number(target?.dataset?.index))
      ? Number(target?.dataset?.index)
      : rowIndex;
    const rowKey = row?.dataset?.playbackRow || null;
    const track = state.historyTracksView[index];
    if (!track) return;

    if (!action) {
      const before = trackMetaFingerprint(track);
      await hydrateUsbTrackMetadata(track);
      if (trackMetaFingerprint(track) !== before && !patchHistoryTrackRow(track)) {
        renderHistoryTracks();
      }
      return;
    }

    await hydrateUsbTrackMetadata(track);
    handleTrackAction({ action, track, origin: "usb", target, event, state, rowKey, ctx });
  });
}
