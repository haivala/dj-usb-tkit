import { catchErr, handleTrackAction, resolveEmitStatus } from "../shared/track_actions.mjs";
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
    resolveLocalTrackId,
    refreshCurrentPlaylistTracks,
    playlistTracksCtl,
    clearPlaylistTrackSort = () => {}
  } = ctx;
  const emitStatus = resolveEmitStatus(ctx);

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

  el.playlistSearchInput?.addEventListener("input", () => {
    state.playlistTrackSearch = String(el.playlistSearchInput.value || "");
    // Client-side filter of the already-loaded playlist -- no refetch.
    Promise.resolve(playlistTracksCtl.setSearch(state.playlistTrackSearch)).catch(catchErr(emitStatus));
  });

  el.panels.playlist.addEventListener("click", (event) => {
    const actionTarget = event.target.closest("[data-action]");
    if (actionTarget?.dataset?.action === "remove-playlist-track") {
      const id = actionTarget.dataset.id;
      const playlist = getCurrentPlaylist();
      const track = playlistTracksCtl.view.find((item) => String(item.id) === String(id))
        || playlist?.tracks?.find((item) => String(item.id) === String(id));
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
      const id = actionTarget.dataset.id;
      const playlist = getCurrentPlaylist();
      const track = playlistTracksCtl.view.find((item) => String(item.id) === String(id))
        || playlist?.tracks?.find((item) => String(item.id) === String(id));
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

  el.analyzePlaylistMissingBtn?.addEventListener("click", () => {
    const playlist = getCurrentPlaylist();
    if (!playlist) return;
    const trackIds = (playlist.tracks || [])
      .filter((track) => !track.analysisReady)
      .map((track) => String(resolveLocalTrackId(track) || track.localTrackId || track.id || "").trim())
      .filter(Boolean);
    if (!trackIds.length) {
      emitStatus("No tracks in this playlist need analysis.");
      return;
    }
    analyzeTrackIds(trackIds, "Analyze Missing Tracks").catch((err) => {
      console.error(err);
      emitStatus(`Analyze failed: ${err.message || err}`);
    });
  });

  let dragSourceRow = null;
  let dragOriginalOrder = null;
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
    const newOrder = Array.from(
      el.playlistTracksBody.querySelectorAll(".track-grid-row[data-track-id]")
    ).map((r) => r.dataset.trackId);
    const originalOrder = dragOriginalOrder;
    dragSourceRow = null;
    dragOriginalOrder = null;
    if (!playlist || !originalOrder || newOrder.join("\u0000") === originalOrder.join("\u0000")) {
      return;
    }
    clearPlaylistTrackSort();
    command("reorder_playlist_tracks", {
      playlistId: playlist.id,
      orderedTrackIds: newOrder
    })
      .catch((err) => {
        console.error(err);
        emitStatus(`Save track order failed: ${err.message || err}`);
      })
      .finally(() => {
        refreshCurrentPlaylistTracks().catch((err) => console.error(err));
      });
  });
}
