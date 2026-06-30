import { catchErr, handleTrackAction, resolveEmitStatus } from "../shared/track_actions.mjs";

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
    isUsbOriginTrack,
    trackHasCoreAnalysis,
    analyzeTrackIds,
    resolveLocalTrackId
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
    ctx.refreshCurrentPlaylistTracks().catch(catchErr(emitStatus));
  });

  el.panels.playlist.addEventListener("click", (event) => {
    const actionTarget = event.target.closest("[data-action]");
    if (actionTarget?.dataset?.action === "remove-playlist-track") {
      const index = Number(actionTarget.dataset.index);
      const playlist = getCurrentPlaylist();
      const track = state.currentPlaylistTracksView[index];
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
      const index = Number(actionTarget.dataset.index);
      const track = state.currentPlaylistTracksView[index];
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
      .filter((track) => !isUsbOriginTrack(track) && !trackHasCoreAnalysis(track))
      .map((track) => String(resolveLocalTrackId(track) || track.localTrackId || track.id || "").trim())
      .filter(Boolean);
    if (!trackIds.length) {
      emitStatus("No local non-USB tracks in this playlist need analysis.");
      return;
    }
    analyzeTrackIds(trackIds, "Analyze Missing Tracks", { pieceMode: "missing" }).catch((err) => {
      console.error(err);
      emitStatus(`Analyze failed: ${err.message || err}`);
    });
  });
}
