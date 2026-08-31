import { catchErr, handleTrackAction, resolveEmitStatus, resolveRowActionTrack } from "../shared/track_actions.mjs";

export function bindLibraryEvents(ctx) {
  const {
    state,
    el,
    window,
    constants,
    setStatus,
    renderSourceChips,
    syncAssetScopePaths,
    applySearchLocalFilter,
    updateSelectionCount,
    updateSourceFilterIndicator = () => {},
    openConfirmDialog = async () => false,
    command,
    resetAndLoadLibraryTracks,
    refreshCurrentPlaylistTracks,
    withProgress,
    persistSourceRoots,
    persistSourceRootEnabled,
    persistMasterDbEnabled = () => {},
    persistSourcesEverConfigured = () => {},
    enabledSourceRoots,
    pickSourceFolders,
    relocateSourceRoot,
    scanLibrary,
    scanMasterDb,
    analyzeSelectedTracks,
    scheduleApplySearchLocalFilter,
    addTracksToCurrentPlaylist,
    addLibrarySelectionToCurrentPlaylist,
    getLibraryVisibleTracks,
    hydrateLoadedTracksPreviewsInBackground = async () => {},
    LIBRARY_LOAD_LIMIT_DEFAULT,
  } = ctx;
  const emitStatus = resolveEmitStatus(ctx);

  el.sourceChipsContainer.addEventListener("click", (event) => {
    const removeBtn = event.target.closest(".source-chip-remove");
    if (!removeBtn) return;
    const index = Number(removeBtn.dataset.sourceIndex);
    if (!Number.isInteger(index) || index < 0 || index >= state.sourceRoots.length) return;
    const removedRoot = state.sourceRoots[index];
    (async () => {
      const confirmed = await openConfirmDialog({
        title: "Remove Source Folder",
        message: `Remove source folder?\n\n${removedRoot}\n\nAll tracks from this folder will be removed from the library. Tracks that are in a local playlist will also be removed from those playlists.`,
        confirmLabel: "Remove"
      });
      if (!confirmed) return;

      state.sourceRoots.splice(index, 1);
      delete state.sourceRootEnabled[removedRoot];
      persistSourceRoots(state.sourceRoots);
      persistSourceRootEnabled(state.sourceRootEnabled);
      renderSourceChips();
      syncAssetScopePaths().catch(() => {});
      if (!removedRoot) {
        applySearchLocalFilter();
        updateSelectionCount();
        emitStatus(`Source folders: ${state.sourceRoots.length}`);
        return;
      }
      try {
        const result = await command("remove_tracks_by_source_roots", {
          sourceRoots: [removedRoot]
        });
        await resetAndLoadLibraryTracks(state.libraryQuery, LIBRARY_LOAD_LIMIT_DEFAULT);
        await refreshCurrentPlaylistTracks();
        emitStatus(
          `Source folders: ${state.sourceRoots.length} | removed ${Number(result?.removed || 0)} track(s) from library`
        );
      } catch (err) {
        console.error(err);
        emitStatus(`Source removed, but track pruning failed: ${err.message || err}`);
      }
    })();
  });

  el.sourceChipsContainer.addEventListener("click", (event) => {
    if (event.target.closest(".source-chip-remove")) return;
    const chip = event.target.closest(".source-chip-missing[data-source-relocate-index]");
    if (!chip) return;
    const index = Number(chip.dataset.sourceRelocateIndex);
    if (!Number.isInteger(index) || index < 0 || index >= state.sourceRoots.length) return;
    const path = state.sourceRoots[index];
    relocateSourceRoot(path).catch(catchErr(emitStatus));
  });

  el.sourceChipsContainer.addEventListener("change", (event) => {
    const checkbox = event.target.closest(".source-chip-toggle");
    if (!checkbox) return;

    // master.db chip toggle - pure filter, never triggers import
    if (checkbox.dataset.masterDb === "true") {
      const enabling = checkbox.checked;
      state.masterDbEnabled = enabling;
      persistMasterDbEnabled(enabling);
      resetAndLoadLibraryTracks(state.libraryQuery, LIBRARY_LOAD_LIMIT_DEFAULT)
        .catch(catchErr(emitStatus));
      updateSourceFilterIndicator();
      const total = state.sourceRoots.length + 1;
      const enabled = enabledSourceRoots(state.sourceRoots, state.sourceRootEnabled, state.missingSourceRoots).length + (enabling ? 1 : 0);
      emitStatus(`Source filters: ${enabled}/${total} enabled`);
      return;
    }

    const index = Number(checkbox.dataset.sourceToggleIndex);
    if (!Number.isInteger(index) || index < 0 || index >= state.sourceRoots.length) return;
    const path = state.sourceRoots[index];
    state.sourceRootEnabled[path] = !!checkbox.checked;
    persistSourceRootEnabled(state.sourceRootEnabled);
    resetAndLoadLibraryTracks(state.libraryQuery, LIBRARY_LOAD_LIMIT_DEFAULT)
      .catch(catchErr(emitStatus));
    updateSourceFilterIndicator();
    const masterDbTotal = state.externalMasterDbPath ? 1 : 0;
    const masterDbEnabled = state.externalMasterDbPath && state.masterDbEnabled ? 1 : 0;
    const enabledCount = enabledSourceRoots(state.sourceRoots, state.sourceRootEnabled, state.missingSourceRoots).length + masterDbEnabled;
    emitStatus(`Source filters: ${enabledCount}/${state.sourceRoots.length + masterDbTotal} enabled`);
  });

  el.addSourceBtn.addEventListener("click", () => {
    pickSourceFolders()
      .then(async (picked) => {
        if (!picked.length) return;

        const known = new Set(state.sourceRoots);
        const newRoots = [];
        for (const path of picked) {
          if (!known.has(path)) {
            state.sourceRoots.push(path);
            state.sourceRootEnabled[path] = true;
            known.add(path);
            newRoots.push(path);
          }
        }
        if (newRoots.length === 0) {
          emitStatus(`Source folders unchanged: ${state.sourceRoots.length}`);
          return;
        }
        let indexResult = null;
        await withProgress("Loading media library", async (progress) => {
          progress(20, "Saving source folders...");
          persistSourceRoots(state.sourceRoots);
          persistSourceRootEnabled(state.sourceRootEnabled);
          if (!state.sourcesEverConfigured) {
            state.sourcesEverConfigured = true;
            persistSourcesEverConfigured(true);
          }
          renderSourceChips();
          syncAssetScopePaths().catch(() => {});

          progress(45, "Indexing new files...");
          indexResult = await command("scan_library", {
            sourceRoots: newRoots,
            incremental: true
          });

          progress(70, "Building track list...");
          await resetAndLoadLibraryTracks(state.libraryQuery, LIBRARY_LOAD_LIMIT_DEFAULT);

          progress(85, "Refreshing playlist views...");
          await refreshCurrentPlaylistTracks();
        });
        emitStatus(`Source folders: ${state.sourceRoots.length} | indexed ${Number(indexResult?.indexed || 0)} track(s)`);
      })
      .catch(catchErr(emitStatus));
  });

  el.scanLibraryBtn.addEventListener("click", () => {
    const action = state.selectedTrackIds.size > 0 ? analyzeSelectedTracks : scanLibrary;
    action().catch(catchErr(emitStatus));
  });

  el.importMasterDbBtn?.addEventListener("click", () => {
    scanMasterDb?.().catch(catchErr(emitStatus));
  });

  el.libraryTableWrap?.addEventListener("scroll", ctx.handleLibraryTableWrapScroll, { passive: true });
  window.addEventListener("resize", ctx.handleLibraryTableWrapScroll);
  el.librarySearch.addEventListener("input", scheduleApplySearchLocalFilter);

  // Source bar accordion
  el.sourceFilterHeader?.addEventListener("click", () => {
    el.sourceBar?.classList.toggle("collapsed");
  });
  el.librarySearch.addEventListener("focus", () => {
    if (state.sourceRoots.length > 6) el.sourceBar?.classList.add("collapsed");
  }, { passive: true });
  el.libraryTableWrap?.addEventListener("scroll", () => {
    if (state.sourceRoots.length > 6) el.sourceBar?.classList.add("collapsed");
  }, { passive: true });

  el.addSelectedBtn.addEventListener("click", () => {
    // Backend-owned: it enumerates the ticked ids against the current library
    // filter and materializes any browse-only rows -- so a selection spanning
    // pages that have since been dropped isn't silently lost.
    addLibrarySelectionToCurrentPlaylist({
      trackIds: Array.from(state.selectedTrackIds || []),
    }).catch(catchErr(emitStatus));
  });

  el.selectAllTracks.addEventListener("change", (event) => {
    const checkbox = event.target;
    if (!checkbox.checked) {
      state.selectedTrackIds.clear();
      ctx.renderLibraryRows();
      updateSelectionCount();
      return;
    }
    (async () => {
      // Backend enumerates every id matching the current filter in one call --
      // no need to page the whole list into the client to collect ids.
      const data = await withProgress("Selecting all matching tracks", async (progress) => {
        progress(40, "Enumerating tracks...");
        return command("list_matching_track_ids", {
          sourceRoots: enabledSourceRoots(state.sourceRoots, state.sourceRootEnabled, state.missingSourceRoots),
          includeMasterDb: state.masterDbEnabled === true,
          query: String(state.libraryQuery || "").trim(),
        });
      });
      if (!checkbox.checked) return;
      state.selectedTrackIds = new Set((data?.trackIds || []).filter(Boolean));
      ctx.renderLibraryRows();
      updateSelectionCount();
    })().catch(catchErr(emitStatus));
  });

  el.libraryTableBody.addEventListener("change", (event) => {
    const id = event.target?.dataset?.id;
    if (!id) return;

    if (event.target.checked) state.selectedTrackIds.add(id);
    else state.selectedTrackIds.delete(id);
    updateSelectionCount();
  });

  el.libraryTableBody.addEventListener("click", (event) => {
    const target = event.target.closest("[data-action]");
    const action = target?.dataset?.action;
    const rowKey = target?.closest(".track-grid-row")?.dataset?.playbackRow || null;
    const track = resolveRowActionTrack(getLibraryVisibleTracks(), target);
    if (!track) return;

    handleTrackAction({ action, track, origin: "local", target, event, state, rowKey, ctx });
  });
}
