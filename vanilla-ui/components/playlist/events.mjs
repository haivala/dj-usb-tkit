import { counted } from "../../track_utils.mjs";
import { catchErr, handleTrackAction, resolveRowActionTrack } from "../shared/track_actions.mjs";
import { bindTrackSelection, clearTrackSelection } from "../shared/track_selection.mjs";
import { addSelectionToPlaylists } from "../shared/add_to_playlists.mjs";
import { editTrackSelection } from "../shared/track_metadata_edits.mjs";
import { createDragAutoScroller } from "../../dnd_autoscroll.mjs";
import { bindListDragReorder } from "../shared/list_drag_reorder.mjs";

export function bindPlaylistEvents(ctx) {
  const {
    state,
    el,
    switchView,
    deletePlaylist,
    startPlaylistRename,
    reorderPlaylists,
    promptNewPlaylist,
    importExternalPlaylist,
    playlistImportDialog,
    command,
    getCurrentPlaylist,
    loadPlaylists,
    updateModeText,
    exportPlaylistToUsb,
    analyzeTrackIds,
    reanalyzeTracks,
    refreshCurrentPlaylistTracks,
    playlistTracksCtl,
    commitActivePlaylistSort,
    isPlaylistSortActive
  } = ctx;
  const { emitStatus } = ctx;

  // Scroll-load more of the (now paginated) playlist track list.
  playlistTracksCtl.attachScroll();

  el.navPlaylistList.addEventListener("mousedown", (event) => {
    const iconBtn = event.target.closest("[data-delete-playlist], [data-rename-playlist]");
    if (!iconBtn) return;
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation();
  });

  el.navPlaylistList.addEventListener("click", (event) => {
    const deleteBtn = event.target.closest("[data-delete-playlist]");
    if (deleteBtn) {
      event.preventDefault();
      event.stopPropagation();
      event.stopImmediatePropagation();
      deletePlaylist(deleteBtn.dataset.deletePlaylist).catch(catchErr(emitStatus));
      return;
    }
    const renameBtn = event.target.closest("[data-rename-playlist]");
    if (renameBtn) {
      event.preventDefault();
      event.stopPropagation();
      startPlaylistRename(renameBtn.dataset.renamePlaylist);
      return;
    }
    // Clicks inside an open rename field (placing the caret) aren't selections.
    if (event.target.closest(".nav-rename-input")) return;
    const item = event.target.closest(".nav-playlist-item");
    if (!item) return;
    if (el.navPlaylistList.querySelector(".nav-new-input-wrap")) return;
    switchView(item.dataset.playlistId).catch(catchErr(emitStatus));
  });

  el.navPlaylistList.addEventListener("dblclick", (event) => {
    if (event.target.closest(".nav-rename-input, [data-delete-playlist], [data-nav-playlist-drag-handle]")) return;
    const item = event.target.closest(".nav-playlist-item");
    if (!item) return;
    event.preventDefault();
    startPlaylistRename(item.dataset.playlistId);
  });

  bindListDragReorder(el.navPlaylistList, {
    handleSelector: "[data-nav-playlist-drag-handle]",
    itemSelector: "li[data-nav-playlist-li]",
    onMove: (fromIndex, toIndex) => {
      reorderPlaylists(fromIndex, toIndex).catch(catchErr(emitStatus));
    }
  });

  el.addPlaylistBtn.addEventListener("click", () => {
    promptNewPlaylist();
  });

  el.importPlaylistBtn?.addEventListener("click", () => {
    importExternalPlaylist().catch(catchErr(emitStatus));
  });
  el.playlistImportList?.addEventListener("change", () => playlistImportDialog?.sync());
  el.playlistImportOkBtn?.addEventListener("click", () => {
    playlistImportDialog?.close(true);
  });
  el.playlistImportCancelBtn?.addEventListener("click", () => {
    playlistImportDialog?.close(false);
  });
  el.playlistImportOverlay?.addEventListener("click", (event) => {
    if (event.target === el.playlistImportOverlay) playlistImportDialog?.close(false);
  });

  let playlistSearchTimer = null;
  el.playlistSearchInput?.addEventListener("input", () => {
    state.playlistTrackSearch = String(el.playlistSearchInput.value || "");
    // Debounced backend re-query of the playlist (get_playlist_tracks query
    // param) -- same as the library search.
    if (playlistSearchTimer) clearTimeout(playlistSearchTimer);
    playlistSearchTimer = setTimeout(() => {
      playlistSearchTimer = null;
      Promise.resolve(playlistTracksCtl.setSearch(state.playlistTrackSearch)).catch(catchErr(emitStatus));
    }, 180);
  });

  // Remove tracks from the open playlist (the row's "×" and the selection
  // bar's Remove); the tracks stay in the library.
  const removeFromCurrentPlaylist = async (trackIds) => {
    const playlist = getCurrentPlaylist();
    if (!playlist || !trackIds.length) return;
    try {
      const res = await command("remove_tracks_from_playlist", {
        playlistId: playlist.id,
        trackIds
      });
      await clearTrackSelection(ctx, "playlist");
      await loadPlaylists();
      state.currentPlaylistId = playlist.id;
      updateModeText();
      await switchView(playlist.id);
      emitStatus(`Removed ${counted(res.removed || 0, "track")} from ${playlist.name}`);
    } catch (err) {
      console.error(err);
      emitStatus(`Remove failed: ${err.message || err}`);
    }
  };

  bindTrackSelection(ctx, {
    view: "playlist",
    body: el.playlistTracksBody,
    ctl: playlistTracksCtl,
    // `limit: 0` returns the whole playlist under the current search.
    selectAll: async () => {
      const playlist = getCurrentPlaylist();
      if (!playlist) return [];
      const data = await command("get_playlist_tracks", {
        playlistId: playlist.id,
        query: playlistTracksCtl.query || "",
        limit: 0,
      });
      return data?.items || [];
    },
    actions: {
      "add-to": (ids) => addSelectionToPlaylists(ctx, "playlist", ids, []),
      remove: (ids) => removeFromCurrentPlaylist(ids),
      reanalyze: (ids) => reanalyzeTracks(ids),
      edit: (ids, rows) => editTrackSelection(ctx, "playlist", ids, rows),
    },
  });

  el.panels.playlist.addEventListener("click", (event) => {
    const actionTarget = event.target.closest("[data-action]");
    if (actionTarget?.dataset?.action === "remove-playlist-track") {
      const track = resolveRowActionTrack(playlistTracksCtl.view, actionTarget);
      if (track?.id) removeFromCurrentPlaylist([track.id]);
      return;
    }

    const action = actionTarget?.dataset?.action;
    if (
      action === "play-library" ||
      action === "scrub-play" ||
      action === "edit-track-detail" ||
      action === "analyze-track"
    ) {
      const track = resolveRowActionTrack(playlistTracksCtl.view, actionTarget);
      if (!track) return;
      const rowKey = actionTarget?.closest(".track-grid-row")?.dataset?.playbackRow || null;
      handleTrackAction({ action, track, origin: "local", target: actionTarget, event, state, rowKey, ctx });
    }
  });

  el.exportPlaylistBtn?.addEventListener("click", () => {
    if (!state.usbRoot || !state.usbRootValid) {
      switchView("usb").catch((err) => console.error(err));
      return;
    }
    const playlist = getCurrentPlaylist();
    if (!playlist) return;
    exportPlaylistToUsb(playlist.id).catch((error) => {
      console.error(error);
      emitStatus(`Export failed: ${error?.message || String(error || "unknown error")}`);
    });
  });

  el.analyzePlaylistMissingBtn?.addEventListener("click", async () => {
    const playlist = getCurrentPlaylist();
    if (!playlist) return;
    // Backend-owned: `analyze_new_tracks` with a `playlistId` selects the tracks
    // in this playlist that still need analysis, over the whole playlist -- no
    // need to force-load every page here or resolve ids client-side.
    analyzeTrackIds([], "Analyze Missing Tracks", { playlistId: playlist.id }).catch((err) => {
      console.error(err);
      emitStatus(`Analyze failed: ${err.message || err}`);
    });
  });

  let dragSourceRow = null;
  let dragOriginalOrder = null;
  let dragSortCommit = null; // Promise while a sorted-list drag commits its order, else null
  const autoScroller = createDragAutoScroller(el.playlistTableWrap);

  el.playlistTracksBody?.addEventListener("dragstart", (event) => {
    const handle = event.target.closest("[data-playlist-track-drag-handle]");
    const row = handle?.closest(".track-grid-row[data-track-id]");
    if (!handle || !row) {
      event.preventDefault();
      return;
    }
    dragSourceRow = row;
    dragOriginalOrder = Array.from(
      el.playlistTracksBody.querySelectorAll(".track-grid-row[data-track-id]")
    ).map((r) => r.dataset.trackId);
    // WYSIWYG: a column sort on this list is a view-only op until committed. The
    // instant a drag starts we persist that exact sorted order as the playlist's
    // real order and drop the sort indicator, so the drop below reorders the list
    // the user is actually looking at (commitActivePlaylistSort clears the sort
    // hint synchronously, then awaits the backend sort-commit).
    const dragPlaylist = getCurrentPlaylist();
    dragSortCommit = (dragPlaylist && isPlaylistSortActive())
      ? Promise.resolve(commitActivePlaylistSort(dragPlaylist.id)).catch((err) => {
          console.error(err);
          emitStatus(`Save track order failed: ${err.message || err}`);
        })
      : null;
    row.classList.add("dragging");
    // Only the rows around the viewport are rendered; hold them still while
    // this row is moved among them (autoscroll only adds rows).
    playlistTracksCtl.freezeWindow(true);
    autoScroller.attachWheel();
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", row.dataset.trackId);
    }
  });

  el.playlistTracksBody?.addEventListener("dragover", (event) => {
    if (!dragSourceRow) return;
    event.preventDefault();
    autoScroller.update(event.clientY);
    const targetRow = event.target.closest(".track-grid-row[data-track-id]");
    if (!targetRow || targetRow === dragSourceRow) return;
    const rect = targetRow.getBoundingClientRect();
    const before = event.clientY < rect.top + rect.height / 2;
    if (before) {
      targetRow.before(dragSourceRow);
    } else {
      targetRow.after(dragSourceRow);
    }
  });

  el.playlistTracksBody?.addEventListener("drop", (event) => {
    if (dragSourceRow) event.preventDefault();
  });

  el.playlistTracksBody?.addEventListener("dragend", () => {
    autoScroller.stop();
    autoScroller.detachWheel();
    if (!dragSourceRow) return;
    playlistTracksCtl.freezeWindow(false);
    dragSourceRow.classList.remove("dragging");
    const playlist = getCurrentPlaylist();
    const movedRow = dragSourceRow;
    const newOrder = Array.from(
      el.playlistTracksBody.querySelectorAll(".track-grid-row[data-track-id]")
    ).map((r) => r.dataset.trackId);
    const originalOrder = dragOriginalOrder;
    const pendingCommit = dragSortCommit;
    dragSourceRow = null;
    dragOriginalOrder = null;
    dragSortCommit = null;
    if (!playlist || !originalOrder || newOrder.join("\u0000") === originalOrder.join("\u0000")) {
      // No positional change. Any active sort was still committed on dragstart;
      // reconcile the loaded page with the newly persisted order.
      if (pendingCommit) {
        pendingCommit.finally(() => refreshCurrentPlaylistTracks().catch((e) => console.error(e)));
      }
      return;
    }
    // Single-move: the backend repositions this one track relative to its new
    // neighbour, over the whole playlist -- the DOM only holds loaded rows. When
    // a sort was active, `pendingCommit` first persists the sorted view as the
    // playlist order so `beforeTrackId` (a neighbour in that view) lines up.
    // The next row can be a skeleton (its page not loaded yet): resolve it
    // through its slot in the list.
    const nextRow = movedRow.nextElementSibling;
    const beforeTrackId = nextRow?.classList?.contains("track-grid-row")
      ? (nextRow.dataset.trackId || playlistTracksCtl.view[Number(nextRow.dataset.trackIndex)]?.id || null)
      : null;
    if (nextRow && !beforeTrackId) {
      emitStatus("Track order not saved: the list was still loading. Try again.");
      refreshCurrentPlaylistTracks().catch((e) => console.error(e));
      return;
    }
    const moveTrackId = movedRow.dataset.trackId;

    (async () => {
      try {
        if (pendingCommit) await pendingCommit; // sorted order must land before the single move
        await command("reorder_playlist_tracks", {
          playlistId: playlist.id,
          moveTrackId,
          beforeTrackId
        });
        // The reorder cleared the playlist's last export.
        await loadPlaylists();
      } catch (err) {
        console.error(err);
        emitStatus(`Save track order failed: ${err.message || err}`);
      } finally {
        refreshCurrentPlaylistTracks().catch((e) => console.error(e));
      }
    })();
  });
}
