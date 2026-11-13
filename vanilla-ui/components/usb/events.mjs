import { catchErr, handleTrackAction, resolveRowActionTrack } from "../shared/track_actions.mjs";
import { bindListDragReorder } from "../shared/list_drag_reorder.mjs";
import { bindTrackDragReorder } from "../shared/track_drag_reorder.mjs";
import {
  beginUsbOrderEdit, cancelUsbOrderEdit, commitUsbViewSort, isUsbOrderEditing, isUsbViewSortActive,
  moveUsbPlaylistInOrder, moveUsbPlaylistTrackInOrder, saveUsbOrderEdit
} from "./order_edit.mjs";
import { bindUsbAudioFormatEvents } from "./audio_formats.mjs";
import { bindTrackSelection } from "../shared/track_selection.mjs";
import { addSelectionToPlaylists } from "../shared/add_to_playlists.mjs";
import { editTrackSelection } from "../shared/track_metadata_edits.mjs";

export function bindUsbEvents(ctx) {
  const {
    state,
    el,
    pickUsbFolder,
    validateAndSetUsbRoot,
    initializeUsb,
    runUsbParityReport,
    runUsbDiagnostics,
    previewUsbRepairs,
    applyUsbRepairs,
    showDiagReportView,
    exportHistoryTracklist,
    tracklistExportDialog,
    syncUsbPlayerMenuEditorControls,
    handleUsbPlayerMenuListClick,
    addUsbPlayerMenuItems,
    removeUsbPlayerMenuItems,
    moveUsbPlayerMenuItems,
    setUsbBackgroundColor,
    usbPlaylistTracksCtl,
    usbHistoryTracksCtl,
    removeUsbPlaylist,
    stopPlaybackIfActive,
    hydrateUsbTrackMetadata,
    pruneUsbDevice,
  } = ctx;
  const { emitStatus } = ctx;
  // USB-playlist and USB-history track tables: fetch / paginate / search /
  // sort / scroll-load are owned by their shared track-list controllers (see
  // main.js), which fetch pre-hydrated pages from the backend. Scroll-load
  // wires itself here.
  usbPlaylistTracksCtl.attachScroll();
  usbHistoryTracksCtl.attachScroll();

  // Selection: "+ Playlist" sends the rows themselves, as each row's "+" does.
  // Select all fetches the whole list (`limit: 0`) under the current search.
  for (const [view, body, ctl, fetchCommand] of [
    ["usbPlaylist", el.usbPlaylistTracks, usbPlaylistTracksCtl, "fetch_usb_playlist_tracks"],
    ["usbHistory", el.historyTracks, usbHistoryTracksCtl, "fetch_usb_history_tracks"],
  ]) {
    bindTrackSelection(ctx, {
      view,
      body,
      ctl,
      selectAll: async () => {
        if (ctl.scopeId == null) return [];
        const data = await ctx.command(fetchCommand, {
          usbRoot: state.usbRoot || null,
          id: ctl.scopeId,
          query: ctl.query || "",
          limit: 0,
        });
        return (data?.items || []).map((track) => ctx.normalizeTrack(track));
      },
      actions: {
        add: (_ids, rows) => ctx.addTracksToCurrentPlaylist(rows),
        "add-to": (ids, rows) => addSelectionToPlaylists(ctx, view, ids, rows),
        edit: (ids, rows) => editTrackSelection(ctx, view, ids, rows),
      },
    });
  }

  el.selectUsbFolderBtn.addEventListener("click", () => {
    pickUsbFolder().catch(catchErr(emitStatus));
  });

  // The header's drive name renames the connected drive.
  el.usbNameBadge.addEventListener("click", (event) => {
    // The health dot inside it has its own tooltip, not a rename.
    if (event.target.closest(".usb-health-dot")) return;
    if (!el.usbNameBadge.classList.contains("is-renamable")) return;
    ctx.renameDrive().catch(catchErr(emitStatus));
  });
  el.usbNameBadge.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    ctx.renameDrive().catch(catchErr(emitStatus));
  });

  bindUsbAudioFormatEvents(ctx);

  // Recent USBs accordion, like the library's Sources.
  el.usbRecentHeader.addEventListener("click", () => {
    el.usbRecentRow.classList.toggle("collapsed");
  });

  el.usbRecentList.addEventListener("click", (event) => {
    const pruneBtn = event.target.closest("[data-usb-prune-device-id]");
    if (pruneBtn) {
      const deviceId = String(pruneBtn.dataset.usbPruneDeviceId || "").trim();
      if (deviceId) pruneUsbDevice(deviceId).catch(catchErr(emitStatus));
      return;
    }
    const btn = event.target.closest("[data-usb-recent-path]");
    if (!btn) return;
    const selectedPath = String(btn.dataset.usbRecentPath || "").trim();
    if (!selectedPath) return;
    validateAndSetUsbRoot(selectedPath).catch(catchErr(emitStatus));
  });

  el.initializeUsbBtn.addEventListener("click", () => {
    initializeUsb().catch(catchErr(emitStatus));
  });

  el.runUsbParityBtn.addEventListener("click", () => {
    runUsbParityReport().catch(catchErr(emitStatus));
  });

  el.reDiagnoseBtn.addEventListener("click", () => {
    runUsbDiagnostics({ openReport: true }).catch(catchErr(emitStatus));
  });

  el.previewRepairsBtn.addEventListener("click", () => {
    previewUsbRepairs().catch(catchErr(emitStatus));
  });

  el.usbBackupsBtn.addEventListener("click", () => {
    ctx.switchView("backups").catch(catchErr(emitStatus));
  });

  el.applyRepairsBtn.addEventListener("click", () => {
    applyUsbRepairs().catch(catchErr(emitStatus));
  });

  el.diagBackToReportBtn.addEventListener("click", () => {
    showDiagReportView();
  });

  el.exportHistoryTracklistBtn.addEventListener("click", () => {
    exportHistoryTracklist().catch(catchErr(emitStatus));
  });

  el.tracklistExportTimesToggle.addEventListener("change", () => {
    tracklistExportDialog?.syncPlacementVisibility();
  });
  el.tracklistExportOkBtn.addEventListener("click", () => {
    const timesOn = !!el.tracklistExportTimesToggle.checked;
    tracklistExportDialog?.close({
      timeMode: timesOn ? el.tracklistExportPlacement.value : "off",
      startIndex: Number(el.tracklistExportStartTrack.value) || 0
    });
  });
  el.tracklistExportCancelBtn.addEventListener("click", () => {
    tracklistExportDialog?.close(null);
  });

  el.usbPlayerMenuAddBtn.addEventListener("click", () => {
    addUsbPlayerMenuItems().catch(catchErr(emitStatus));
  });

  el.usbPlayerMenuRemoveBtn.addEventListener("click", () => {
    removeUsbPlayerMenuItems().catch(catchErr(emitStatus));
  });

  el.usbPlayerMenuUpBtn.addEventListener("click", () => {
    moveUsbPlayerMenuItems(-1).catch(catchErr(emitStatus));
  });

  el.usbPlayerMenuDownBtn.addEventListener("click", () => {
    moveUsbPlayerMenuItems(1).catch(catchErr(emitStatus));
  });

  el.usbBackgroundColorChips.addEventListener("click", (event) => {
    const chip = event.target.closest(".bg-color-chip");
    if (!chip) return;
    setUsbBackgroundColor(Number(chip.dataset.color)).catch(catchErr(emitStatus));
  });

  el.usbPlayerMenuAvailable.addEventListener("click", (event) => {
    handleUsbPlayerMenuListClick("available", event);
    syncUsbPlayerMenuEditorControls();
  });
  el.usbPlayerMenuCurrent.addEventListener("click", (event) => {
    handleUsbPlayerMenuListClick("current", event);
    syncUsbPlayerMenuEditorControls();
  });

  el.usbTrackSearch.addEventListener("input", () => {
    usbPlaylistTracksCtl.setSearch(el.usbTrackSearch.value)
      .catch((err) => console.warn("USB playlist search failed:", err));
  });

  el.historyTrackSearch.addEventListener("input", () => {
    usbHistoryTracksCtl.setSearch(el.historyTrackSearch.value)
      .catch((err) => console.warn("USB history search failed:", err));
  });

  el.usbPlaylists.addEventListener("click", (event) => {
    const removeBtn = event.target.closest("[data-usb-remove-playlist]");
    if (removeBtn) {
      const removeId = removeBtn.dataset.usbRemovePlaylist;
      const playlist = state.usbPlaylists.find((item) => String(item.id) === String(removeId));
      removeUsbPlaylist(playlist).catch((err) => {
        console.error(err);
        emitStatus(err?.message || String(err));
      });
      return;
    }

    const btn = event.target.closest("[data-usb-playlist]");
    if (!btn) return;
    stopPlaybackIfActive().catch((err) => {
      console.warn("Failed stopping playback on USB playlist change:", err);
    });
    emitStatus(ctx.openUsbPlaylistRow(btn).selectedText);
  });

  // Edit order: the handles show only while editing (CSS on the list's
  // `is-order-editing`).
  el.usbEditOrderBtn.addEventListener("click", () => {
    beginUsbOrderEdit(ctx).catch(catchErr(emitStatus));
  });
  el.usbCancelOrderBtn.addEventListener("click", () => {
    cancelUsbOrderEdit(ctx).catch(catchErr(emitStatus));
  });
  el.usbSaveOrderBtn.addEventListener("click", () => {
    saveUsbOrderEdit(ctx).catch(catchErr(emitStatus));
  });

  bindListDragReorder(el.usbPlaylists, {
    handleSelector: "[data-usb-drag-handle]",
    itemSelector: "li",
    // It reports its own failure (and re-reads the real order).
    onMove: (fromIndex, toIndex) => moveUsbPlaylistInOrder(ctx, fromIndex, toIndex),
  });

  bindTrackDragReorder(el.usbPlaylistTracks, usbPlaylistTracksCtl, {
    scrollContainer: el.usbPlaylistTracks.closest(".table-wrap"),
    canDrag: () => isUsbOrderEditing(state) && !usbPlaylistTracksCtl.query,
    sortActive: () => isUsbViewSortActive(ctx),
    commitSort: () => commitUsbViewSort(ctx),
    onMove: (move) => moveUsbPlaylistTrackInOrder(ctx, move),
    // Re-read in place: the dropped row stays where the user is.
    onSettled: () => usbPlaylistTracksCtl.load(),
    emitStatus,
  });

  el.usbPlaylistTracks.addEventListener("click", async (event) => {
    const target = event.target.closest("[data-action]");
    const action = target?.dataset?.action;
    const row = target?.closest(".track-grid-row") || event.target.closest(".track-grid-row");
    const rowKey = row?.dataset?.playbackRow || null;
    const track = resolveRowActionTrack(usbPlaylistTracksCtl.view, target || event.target);
    if (!track) return;

    if (!action) {
      // Rows arrive hydrated from fetch_usb_playlist_tracks; this is the
      // belt-and-suspenders re-hydrate for a row that somehow still isn't.
      await hydrateUsbTrackMetadata(track);
      return;
    }

    await hydrateUsbTrackMetadata(track);
    handleTrackAction({ action, track, origin: "usb", target, event, state, rowKey, ctx });
  });

  el.historyList.addEventListener("click", (event) => {
    const btn = event.target.closest("[data-history-id]");
    if (!btn) return;
    stopPlaybackIfActive().catch((err) => {
      console.warn("Failed stopping playback on history change:", err);
    });
    ctx.openHistoryRow(btn);
  });

  el.historyTracks.addEventListener("click", async (event) => {
    const target = event.target.closest("[data-action]");
    const action = target?.dataset?.action;
    const row = target?.closest(".track-grid-row") || event.target.closest(".track-grid-row");
    const rowKey = row?.dataset?.playbackRow || null;
    const track = resolveRowActionTrack(usbHistoryTracksCtl.view, target || event.target);
    if (!track) return;

    if (!action) {
      await hydrateUsbTrackMetadata(track);
      return;
    }

    await hydrateUsbTrackMetadata(track);
    handleTrackAction({ action, track, origin: "usb", target, event, state, rowKey, ctx });
  });

  // The Export playlists page.
  el.exportPlaylistsList.addEventListener("change", (event) => {
    if (event.target.matches(".export-playlists-check")) ctx.toggleExportPlaylist(event.target);
  });
  el.exportPlaylistsList.addEventListener("click", (event) => ctx.clickExportPlaylistRow(event.target));
  el.exportPlaylistsSelectAll.addEventListener("change", () => ctx.selectExportPlaylists("all"));
  el.exportPlaylistsSelectChangedBtn.addEventListener("click", () => ctx.selectExportPlaylists("changed"));
  el.exportPlaylistsRunBtn.addEventListener("click", () => {
    ctx.runExportPlaylists().catch(catchErr(emitStatus));
  });
}
