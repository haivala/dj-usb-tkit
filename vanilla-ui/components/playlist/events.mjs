import { catchErr, handleTrackAction, resolveEmitStatus, resolveRowActionTrack } from "../shared/track_actions.mjs";
import { createDragAutoScroller } from "../../dnd_autoscroll.mjs";

export function bindPlaylistEvents(ctx) {
  const {
    state,
    el,
    setStatus,
    switchView,
    deletePlaylist,
    startPlaylistRename,
    promptNewPlaylist,
    command,
    getCurrentPlaylist,
    loadPlaylists,
    updateModeText,
    exportPlaylistToUsb,
    analyzeTrackIds,
    refreshCurrentPlaylistTracks,
    playlistTracksCtl,
    clearPlaylistTrackSort = () => {},
    commitActivePlaylistSort = async () => {},
    isPlaylistSortActive = () => false
  } = ctx;
  const emitStatus = resolveEmitStatus(ctx);

  // Scroll-load more of the (now paginated) playlist track list.
  playlistTracksCtl?.attachScroll?.();

  el.navPlaylistList.addEventListener("mousedown", (event) => {
    const deleteBtn = event.target.closest("[data-delete-playlist]");
    if (!deleteBtn) return;
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
    const item = event.target.closest(".nav-playlist-item");
    if (!item) return;
    if (el.navPlaylistList.querySelector(".nav-new-input-wrap")) return;
    switchView(item.dataset.playlistId).catch(catchErr(emitStatus));
  });

  el.navPlaylistList.addEventListener("dblclick", (event) => {
    const item = event.target.closest(".nav-playlist-item");
    if (!item) return;
    event.preventDefault();
    startPlaylistRename(item.dataset.playlistId);
  });

  el.addPlaylistBtn.addEventListener("click", () => {
    promptNewPlaylist();
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

  el.panels.playlist.addEventListener("click", (event) => {
    const actionTarget = event.target.closest("[data-action]");
    if (actionTarget?.dataset?.action === "remove-playlist-track") {
      const playlist = getCurrentPlaylist();
      const track = resolveRowActionTrack(playlistTracksCtl.view, actionTarget);
      if (!playlist || !track?.id) return;
      command("remove_tracks_from_playlist", {
        playlistId: playlist.id,
        trackIds: [track.id]
      })
        .then(async (res) => {
          await loadPlaylists();
          state.currentPlaylistId = playlist.id;
          updateModeText();
          await switchView(playlist.id);
          emitStatus(`Removed ${res.removed || 0} track(s) from ${playlist.name}`);
        })
        .catch((err) => {
          console.error(err);
          emitStatus(`Remove failed: ${err.message || err}`);
        });
      return;
    }

    const action = actionTarget?.dataset?.action;
    if (action === "play-library" || action === "scrub-play") {
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
    const nextRow = movedRow.nextElementSibling;
    const beforeTrackId = nextRow?.classList?.contains("track-grid-row")
      ? (nextRow.dataset.trackId || null)
      : null;
    const moveTrackId = movedRow.dataset.trackId;

    (async () => {
      try {
        if (pendingCommit) await pendingCommit; // sorted order must land before the single move
        await command("reorder_playlist_tracks", {
          playlistId: playlist.id,
          moveTrackId,
          beforeTrackId
        });
      } catch (err) {
        console.error(err);
        emitStatus(`Save track order failed: ${err.message || err}`);
      } finally {
        refreshCurrentPlaylistTracks().catch((e) => console.error(e));
      }
    })();
  });
}
