import { catchErr, handleTrackAction, resolveRowActionTrack } from "../shared/track_actions.mjs";
import { bindTrackSelection } from "../shared/track_selection.mjs";
import { editTrackSelection } from "../shared/track_metadata_edits.mjs";
import { enabledSourceRoots, LIBRARY_LOAD_LIMIT_DEFAULT } from "./actions.mjs";

export function bindLibraryEvents(ctx) {
  const {
    state,
    el,
    renderSourceChips,
    syncAssetScopePaths,
    renderLibraryRows,
    updateSelectionCount,
    updateSourceFilterIndicator,
    openConfirmDialog,
    command,
    resetAndLoadLibraryTracks,
    refreshCurrentPlaylistTracks,
    withProgress,
    persistSourceRoots,
    persistSourceRootEnabled,
    persistRekordboxDbEnabled,
    persistMixxxDbEnabled,
    persistSourcesEverConfigured,
    pickSourceFolders,
    relocateSourceRoot,
    scanLibrary,
    scanRekordboxDb,
    scanMixxxDb,
    reanalyzeTracks,
    addLibrarySelectionToCurrentPlaylist,
    libraryTracksCtl,
  } = ctx;
  const { emitStatus } = ctx;

  // "enabled/total" over the folder chips plus each imported library chip.
  const emitSourceFilterCounts = () => {
    const libraries = [
      [state.externalRekordboxDbPath && state.rekordboxDbImported, state.rekordboxDbEnabled],
      [state.externalMixxxDbPath && state.mixxxDbImported, state.mixxxDbEnabled],
    ].filter(([available]) => !!available);
    const enabled = enabledSourceRoots(state.sourceRoots, state.sourceRootEnabled, state.missingSourceRoots).length
      + libraries.filter(([, on]) => on === true).length;
    emitStatus(`Source filters: ${enabled}/${state.sourceRoots.length + libraries.length} enabled`);
  };

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
        renderLibraryRows();
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

    const index = Number(checkbox.dataset.sourceToggleIndex);
    if (!Number.isInteger(index) || index < 0 || index >= state.sourceRoots.length) return;
    const path = state.sourceRoots[index];
    state.sourceRootEnabled[path] = !!checkbox.checked;
    persistSourceRootEnabled(state.sourceRootEnabled);
    resetAndLoadLibraryTracks(state.libraryQuery, LIBRARY_LOAD_LIMIT_DEFAULT)
      .catch(catchErr(emitStatus));
    updateSourceFilterIndicator();
    emitSourceFilterCounts();
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
    scanLibrary().catch(catchErr(emitStatus));
  });

  // Library chips: the checkbox is a pure filter (never an import); ↻ imports.
  el.libraryChipsContainer?.addEventListener("change", (event) => {
    const checkbox = event.target.closest(".source-chip-toggle");
    if (!checkbox) return;
    const enabling = checkbox.checked;
    if (checkbox.dataset.rekordboxDb === "true") {
      state.rekordboxDbEnabled = enabling;
      persistRekordboxDbEnabled(enabling);
    } else if (checkbox.dataset.mixxxDb === "true") {
      state.mixxxDbEnabled = enabling;
      persistMixxxDbEnabled(enabling);
    } else {
      return;
    }
    resetAndLoadLibraryTracks(state.libraryQuery, LIBRARY_LOAD_LIMIT_DEFAULT)
      .catch(catchErr(emitStatus));
    updateSourceFilterIndicator();
    emitSourceFilterCounts();
  });
  el.libraryChipsContainer?.addEventListener("click", (event) => {
    const importBtn = event.target.closest(".source-chip-import");
    if (!importBtn) return;
    const scan = importBtn.dataset.importLibrary === "rekordbox" ? scanRekordboxDb : scanMixxxDb;
    scan().catch(catchErr(emitStatus));
  });

  libraryTracksCtl.attachScroll();
  el.librarySearch.addEventListener("input", () => ctx.scheduleLibrarySearch());

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

  bindTrackSelection(ctx, {
    view: "library",
    body: el.libraryTableBody,
    ctl: libraryTracksCtl,
    // Backend enumerates every id matching the current filter in one call --
    // no need to page the whole list into the client to collect ids.
    selectAll: async () => {
      const data = await command("list_matching_track_ids", {
        sourceRoots: ctx.enabledLibrarySourceRoots(),
        includeRekordboxDb: state.rekordboxDbEnabled === true,
        includeMixxxDb: state.mixxxDbEnabled === true,
        query: String(state.libraryQuery || "").trim(),
      });
      return (data?.trackIds || []).filter(Boolean);
    },
    actions: {
      // Backend-owned: it enumerates the ticked ids against the current
      // library filter and materializes any browse-only rows -- so a
      // selection spanning pages that have since been dropped isn't lost.
      add: (ids) => addLibrarySelectionToCurrentPlaylist({ trackIds: ids }),
      reanalyze: (ids) => reanalyzeTracks(ids),
      edit: (ids, rows) => editTrackSelection(ctx, "library", ids, rows),
    },
  });

  el.libraryTableBody.addEventListener("click", (event) => {
    const target = event.target.closest("[data-action]");
    const action = target?.dataset?.action;
    const rowKey = target?.closest(".track-grid-row")?.dataset?.playbackRow || null;
    const track = resolveRowActionTrack(libraryTracksCtl.view, target);
    if (!track) return;

    handleTrackAction({ action, track, origin: "local", target, event, state, rowKey, ctx });
  });
}
