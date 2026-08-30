import { catchErr, handleTrackAction, trackMetaFingerprint, resolveEmitStatus } from "../shared/track_actions.mjs";
import { loadMoreIfNearBottom } from "../../track_utils.mjs";
import { createDragAutoScroller } from "../../dnd_autoscroll.mjs";

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
    usbPlaylistTracksCtl,
    renderHistoryTracks,
    loadMoreHistoryTracks,
    removeUsbPlaylist,
    reorderUsbPlaylists,
    moveArrayItem,
    stopPlaybackIfActive,
    hydrateUsbTrackMetadata,
    setActiveListItem,
    getHistoryDateDisplay,
    addTracksToCurrentPlaylist,
    pruneUsbDevice,
    applyUsbDurationSummary,
    formatDurationMs,
  } = ctx;
  const patchUsbTrackRow = typeof ctx.patchUsbTrackRow === "function"
    ? ctx.patchUsbTrackRow
    : () => false;
  const patchHistoryTrackRow = typeof ctx.patchHistoryTrackRow === "function"
    ? ctx.patchHistoryTrackRow
    : () => false;
  const applyDurationSummary = typeof applyUsbDurationSummary === "function"
    ? applyUsbDurationSummary
    : () => {};
  const emitStatus = resolveEmitStatus(ctx);
  const syncPlayerMenuControls = typeof syncUsbPlayerMenuEditorControls === "function"
    ? syncUsbPlayerMenuEditorControls
    : () => {};
  const onPlayerMenuListClick = typeof handleUsbPlayerMenuListClick === "function"
    ? handleUsbPlayerMenuListClick
    : () => {};
  let historySelectionHydrationToken = 0;

  // USB_SELECTION_PAGE_SIZE does double duty as both the batched-hydration
  // chunk size (how many ids one inspect_usb_tracks call resolves) and the
  // DOM pagination page size (how many rows one render pass builds) -- see
  // LARGE_USB_SELECTION_THRESHOLD below for why the two were merged into one
  // constant. Sizing it balances two costs that pull in opposite directions:
  //  - A FIXED per-call cost (re-parsing the PDB, re-opening and
  //    re-decrypting the SQLCipher eDB, loading its full track index) paid
  //    once per inspect_usb_tracks call no matter how many ids are in it.
  //    A large playlist (rekordbox caps these at 9999 tracks) used to pay
  //    this ~250 times over at a chunk size of 40 -- visible as ~250
  //    repeated "eDB unlocked"/"loaded N track metadata rows" log lines --
  //    which argues for making chunks as large as possible.
  //  - A PER-ITEM cost proportional to chunk size: each track's
  //    waveform-preview analysis file has to be read from the USB device,
  //    and (for DOM pagination) each row is a real DOM node with a cover
  //    image, waveform canvas, etc. (Cover art used to also be read+
  //    base64-encoded per item here, which at a chunk size of 2000 produced
  //    IPC responses bloated enough to block the frontend's main thread for
  //    a very long time while parsing a single response -- that's been
  //    removed; see `include_artwork_data_url` in
  //    `resolve_usb_track_from_sources`.) This argues for keeping chunks
  //    small enough that one round trip's I/O and render finish and paint
  //    in a reasonable time, so both hydration and DOM growth stay visibly
  //    progressive instead of one long silent wait.
  // 150 balances the two: a 9993-track playlist needs ~67 round trips
  // (still far fewer than the original 40-per-chunk's ~250) while keeping
  // each round trip's file I/O, DOM growth, and response size bounded.
  const USB_SELECTION_PAGE_SIZE = 150;

  // Very large selections (rekordbox caps a playlist at 9999 tracks; a
  // typical playlist is ~80) render/hydrate one page at a time instead of
  // all at once -- rendering ~10k DOM rows in one go is what caused a
  // reported total UI freeze, including the table becoming unresponsive
  // just from moving the mouse over it (the browser's hit-testing/style/
  // paint cost scales with total DOM node count, independent of how well
  // the data-fetching side is optimized). Selections at/under the threshold
  // are entirely unaffected -- see usbPlaylistPagedCount/historyPagedCount
  // in renderUsbPlaylistTracks/renderHistoryTracks (components/usb/actions.mjs) --
  // but still hydrate in USB_SELECTION_PAGE_SIZE-sized chunks below this
  // threshold too, same as they always have.
  const LARGE_USB_SELECTION_THRESHOLD = 300;
  const USB_SCROLL_FETCH_THRESHOLD_PX = 120;

  // Renders+hydrates the next page of an already-paginated large selection
  // on scroll-near-bottom (loadMoreFn -- loadMoreUsbPlaylistTracks/
  // loadMoreHistoryTracks in usb/actions.mjs -- appends to the table and
  // hydrates what it just appended in one step; see hydrateUsbTrackPage
  // there). Shared between USB playlist and USB history scroll handlers
  // below -- only which state/elements/loader they use differs.
  // isSelectionCurrent gates against issuing a wasted round trip for a
  // selection the user has since switched away from -- actual correctness
  // against a stale in-flight call finishing late is independently handled
  // by loadMoreFn's own per-container render/hydration tokens.
  const loadNextPage = async (loadMoreFn, isSelectionCurrent, setLoading) => {
    if (!isSelectionCurrent()) return;
    setLoading(true);
    try {
      await loadMoreFn(USB_SELECTION_PAGE_SIZE);
    } finally {
      setLoading(false);
    }
  };

  // USB-playlist tracks: fetch/paginate/search/sort/scroll are owned by the
  // shared track-list controller (see main.js). Scroll-load wires itself here.
  usbPlaylistTracksCtl?.attachScroll?.();

  const historyTableWrap = el.historyTracks?.closest?.(".table-wrap");
  historyTableWrap?.addEventListener("scroll", () => {
    const token = historySelectionHydrationToken;
    loadMoreIfNearBottom(
      historyTableWrap,
      USB_SCROLL_FETCH_THRESHOLD_PX,
      () => state.historyLoadingMore,
      () => state.historyPagedCount > 0 && state.historyPagedCount < state.historyTracksView.length,
      () => loadNextPage(
        loadMoreHistoryTracks,
        () => historySelectionHydrationToken === token,
        (loading) => { state.historyLoadingMore = loading; }
      ).catch((err) => console.warn("USB history page load failed:", err))
    );
  }, { passive: true });

  el.refreshUsbBtn.addEventListener("click", () => {
    refreshUsb().catch(catchErr(emitStatus));
  });

  el.selectUsbFolderBtn?.addEventListener("click", () => {
    pickUsbFolder().catch(catchErr(emitStatus));
  });

  el.usbRecentList?.addEventListener("click", (event) => {
    const pruneBtn = event.target.closest("[data-usb-prune-device-id]");
    if (pruneBtn) {
      const deviceId = String(pruneBtn.dataset.usbPruneDeviceId || "").trim();
      if (deviceId) pruneUsbDevice?.(deviceId).catch(catchErr(emitStatus));
      return;
    }
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
    usbPlaylistTracksCtl.setSearch(el.usbTrackSearch.value)
      .catch((err) => console.warn("USB playlist search failed:", err));
  });

  el.historyTrackSearch?.addEventListener("input", () => {
    state.historyTrackSearch = String(el.historyTrackSearch.value || "");
    if (state.historyPagedCount > 0) state.historyPagedCount = USB_SELECTION_PAGE_SIZE;
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
    setActiveListItem(el.usbPlaylists, btn);
    if (!playlist) {
      usbPlaylistTracksCtl.clear();
      emitStatus("Failed to resolve selected USB playlist");
      return;
    }
    // The controller fetches page 1 (paginated + hydrated server-side),
    // renders it, and sets the "Total time" footer from the response.
    usbPlaylistTracksCtl.load({ scopeId: playlist.id }).catch((err) => {
      console.warn("USB playlist load failed:", err);
      emitStatus(`Failed to load USB playlist: ${err.message}`);
    });
    emitStatus(`USB playlist selected: ${playlist.name} (${playlist.trackCount ?? "?"} tracks)`);
  });

  let dragSourceLi = null;
  let dragSourceIndex = null;
  const usbPlaylistAutoScroller = createDragAutoScroller(el.usbPlaylists);

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
    usbPlaylistAutoScroller.attachWheel();
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", String(dragSourceIndex));
    }
  });

  el.usbPlaylists.addEventListener("dragover", (event) => {
    if (!dragSourceLi) return;
    event.preventDefault();
    usbPlaylistAutoScroller.update(event.clientY);
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
    usbPlaylistAutoScroller.stop();
    usbPlaylistAutoScroller.detachWheel();
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
    const track = usbPlaylistTracksCtl.items[index];
    if (!track) return;

    if (!action) {
      // Rows arrive hydrated from fetch_usb_playlist_tracks; this is the
      // belt-and-suspenders re-hydrate for a row that somehow still isn't.
      const before = trackMetaFingerprint(track);
      await hydrateUsbTrackMetadata(track);
      if (trackMetaFingerprint(track) !== before && !patchUsbTrackRow(track)) {
        usbPlaylistTracksCtl.rerender();
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
    state.historyPagedCount = state.historyTracks.length > LARGE_USB_SELECTION_THRESHOLD
      ? USB_SELECTION_PAGE_SIZE
      : 0;
    state.historyLoadingMore = false;
    if (el.exportHistoryTracklistBtn) el.exportHistoryTracklistBtn.disabled = !state.historyTracks.length;
    setActiveListItem(el.historyList, btn);
    // renderHistoryTracks() renders AND hydrates whatever page it renders
    // (see components/usb/actions.mjs) -- no separate hydration call
    // needed here. Bumping the token still matters: it's what lets the
    // historyTableWrap scroll listener below tell a stale load-more
    // continuation from a previous selection to stop.
    renderHistoryTracks().catch((err) => {
      console.warn("USB history render/hydration failed:", err);
    });
    const historyTrackCount = history?.tracks?.length ?? state.historyTracks.length;
    const historyKnownCount = history?.durationKnownCount ?? 0;
    applyDurationSummary(
      el.historyTotalDuration,
      history?.totalDurationMs ?? 0,
      Math.max(0, historyTrackCount - historyKnownCount),
      { formatDurationMs }
    );
    historySelectionHydrationToken += 1;
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
