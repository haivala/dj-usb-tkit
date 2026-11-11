import { t } from "../../ui_texts.mjs";
import { catchErr, handleTrackAction, resolveRowActionTrack } from "../shared/track_actions.mjs";
import { bindTrackSelection, clearTrackSelection } from "../shared/track_selection.mjs";
import { addSelectionToPlaylists } from "../shared/add_to_playlists.mjs";
import { editTrackSelection } from "../shared/track_metadata_edits.mjs";
import { convertTrackSelection } from "../shared/transcodes.mjs";
import { bindTrackDragReorder } from "../shared/track_drag_reorder.mjs";
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
    // A new playlist's name still being typed is saved first (leaving the
    // field submits it); the clicked playlist opens once that save is done.
    el.navPlaylistList.querySelector(".nav-new-input-wrap .nav-new-input")?.blur();
    const playlistId = item.dataset.playlistId;
    Promise.resolve(state.playlistCreating)
      .then(() => switchView(playlistId))
      .catch(catchErr(emitStatus));
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

  el.importPlaylistBtn.addEventListener("click", () => {
    importExternalPlaylist().catch(catchErr(emitStatus));
  });
  el.playlistImportList.addEventListener("change", (event) => playlistImportDialog?.changed(event.target));
  el.playlistImportOkBtn.addEventListener("click", () => {
    playlistImportDialog?.close(true);
  });
  el.playlistImportCancelBtn.addEventListener("click", () => {
    playlistImportDialog?.close(false);
  });

  let playlistSearchTimer = null;
  el.playlistSearchInput.addEventListener("input", () => {
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
      ctx.applyPlaylistsState(res);
      state.currentPlaylistId = playlist.id;
      updateModeText();
      await switchView(playlist.id);
      emitStatus(res.statusText);
    } catch (err) {
      console.error(err);
      emitStatus(err?.message || String(err));
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
      convert: (ids) => convertTrackSelection(ctx, ids),
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

  el.exportPlaylistBtn.addEventListener("click", () => {
    if (!state.usbRoot || !state.usbRootValid) {
      switchView("usb").catch((err) => console.error(err));
      return;
    }
    const playlist = getCurrentPlaylist();
    if (!playlist) return;
    exportPlaylistToUsb(playlist.id).catch((error) => {
      console.error(error);
      emitStatus(t("ui-playlist-export-failed", { error: error?.message || String(error || t("ui-playlist-export-failed-unknown")) }));
    });
  });

  el.analyzePlaylistMissingBtn.addEventListener("click", async () => {
    const playlist = getCurrentPlaylist();
    if (!playlist) return;
    // Backend-owned: `analyze_new_tracks` with a `playlistId` selects the tracks
    // in this playlist that still need analysis, over the whole playlist -- no
    // need to force-load every page here or resolve ids client-side.
    analyzeTrackIds([], { playlistId: playlist.id }).catch((err) => {
      console.error(err);
      emitStatus(t("ui-common-analyze-failed", { error: err.message || err }));
    });
  });

  bindTrackDragReorder(el.playlistTracksBody, playlistTracksCtl, {
    scrollContainer: el.playlistTableWrap,
    sortActive: () => !!getCurrentPlaylist() && isPlaylistSortActive(),
    // Clears the sort hint synchronously, then awaits the backend sort-commit.
    commitSort: () => commitActivePlaylistSort(getCurrentPlaylist().id),
    onMove: async ({ moveTrackId, beforeTrackId }) => {
      const playlist = getCurrentPlaylist();
      if (!playlist) return;
      // The reorder cleared the playlist's last export.
      ctx.applyPlaylistsState(await command("reorder_playlist_tracks", {
        playlistId: playlist.id,
        moveTrackId,
        beforeTrackId
      }));
    },
    onSettled: refreshCurrentPlaylistTracks,
    emitStatus,
  });
}
