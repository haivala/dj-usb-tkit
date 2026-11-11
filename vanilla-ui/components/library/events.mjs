import { t } from "../../ui_texts.mjs";
import { catchErr, handleTrackAction, resolveRowActionTrack } from "../shared/track_actions.mjs";
import { bindTrackSelection } from "../shared/track_selection.mjs";
import { addSelectionToPlaylists } from "../shared/add_to_playlists.mjs";
import { editTrackSelection } from "../shared/track_metadata_edits.mjs";
import { convertTrackSelection } from "../shared/transcodes.mjs";
import { applyLibrarySources } from "./actions.mjs";

export function bindLibraryEvents(ctx) {
  const {
    state,
    el,
    openConfirmDialog,
    command,
    resetAndLoadLibraryTracks,
    refreshCurrentPlaylistTracks,
    withProgress,
    pickSourceFolders,
    relocateSourceRoot,
    analyzeLibrary,
    scanRekordboxDb,
    scanMixxxDb,
    reanalyzeTracks,
    addLibrarySelectionToCurrentPlaylist,
    libraryTracksCtl,
  } = ctx;
  const { emitStatus } = ctx;

  const folders = () => state.librarySources.folders;
  const folderAt = (index) => (Number.isInteger(index) ? folders()[index]?.path : undefined);

  // A switch changed: the backend saved it and lists the library with it.
  const applySwitchedSources = async (sources) => {
    applyLibrarySources(ctx, sources);
    emitStatus(sources.filterStatusText);
    await resetAndLoadLibraryTracks(state.libraryQuery);
  };

  el.sourceChipsContainer.addEventListener("click", (event) => {
    const removeBtn = event.target.closest(".source-chip-remove");
    if (!removeBtn) return;
    const removedRoot = folderAt(Number(removeBtn.dataset.sourceIndex));
    if (!removedRoot) return;
    (async () => {
      const confirmed = await openConfirmDialog({
        title: t("ui-library-remove-folder-title"),
        message: t("ui-library-remove-folder-confirm", { path: removedRoot }),
        confirmLabel: t("ui-common-remove")
      });
      if (!confirmed) return;

      const result = await command("remove_source_folder", { path: removedRoot });
      applyLibrarySources(ctx, result.sources);
      await resetAndLoadLibraryTracks(state.libraryQuery);
      await refreshCurrentPlaylistTracks();
      emitStatus(result.statusText);
    })().catch(catchErr(emitStatus));
  });

  el.sourceChipsContainer.addEventListener("click", (event) => {
    if (event.target.closest(".source-chip-remove")) return;
    const chip = event.target.closest(".source-chip-missing[data-source-relocate-index]");
    if (!chip) return;
    const path = folderAt(Number(chip.dataset.sourceRelocateIndex));
    if (!path) return;
    relocateSourceRoot(path).catch(catchErr(emitStatus));
  });

  el.sourceChipsContainer.addEventListener("change", (event) => {
    const checkbox = event.target.closest(".source-chip-toggle");
    if (!checkbox) return;
    const path = folderAt(Number(checkbox.dataset.sourceToggleIndex));
    if (!path) return;
    command("set_source_folder_enabled", { path, enabled: checkbox.checked })
      .then(applySwitchedSources)
      .catch(catchErr(emitStatus));
  });

  el.addSourceBtn.addEventListener("click", () => {
    pickSourceFolders()
      .then(async (picked) => {
        if (!picked.length) return;
        let statusText = "";
        const texts = {
          working: t("ui-library-add-folders-progress"),
          done: t("ui-library-add-folders-progress-done"),
          failed: t("ui-library-add-folders-progress-failed")
        };
        await withProgress(texts, async (progress) => {
          // Adding the folders indexes them too.
          progress(20, t("ui-library-add-folders-indexing"));
          const result = await command("add_source_folders", { paths: picked });
          applyLibrarySources(ctx, result.sources);
          statusText = result.statusText;
          if (!result.added.length) return;

          progress(70, t("ui-library-add-folders-building"));
          await resetAndLoadLibraryTracks(state.libraryQuery);

          progress(85, t("ui-library-add-folders-refreshing"));
          await refreshCurrentPlaylistTracks();
        });
        emitStatus(statusText);
      })
      .catch(catchErr(emitStatus));
  });

  el.analyzeLibraryBtn.addEventListener("click", () => {
    analyzeLibrary().catch(catchErr(emitStatus));
  });

  // Library chips: the checkbox is a pure filter (never an import); ↻ imports.
  el.libraryChipsContainer.addEventListener("change", (event) => {
    const checkbox = event.target.closest(".source-chip-toggle");
    if (!checkbox) return;
    const library = checkbox.dataset.rekordboxDb === "true" ? "rekordbox"
      : checkbox.dataset.mixxxDb === "true" ? "mixxx"
      : null;
    if (!library) return;
    command("set_external_library_enabled", { library, enabled: checkbox.checked })
      .then(applySwitchedSources)
      .catch(catchErr(emitStatus));
  });
  el.libraryChipsContainer.addEventListener("click", (event) => {
    const importBtn = event.target.closest(".source-chip-import");
    if (!importBtn) return;
    const scan = importBtn.dataset.importLibrary === "rekordbox" ? scanRekordboxDb : scanMixxxDb;
    scan().catch(catchErr(emitStatus));
  });

  libraryTracksCtl.attachScroll();
  el.librarySearch.addEventListener("input", () => ctx.scheduleLibrarySearch());

  // Source bar accordion
  el.sourceFilterHeader.addEventListener("click", () => {
    el.sourceBar.classList.toggle("collapsed");
  });
  el.librarySearch.addEventListener("focus", () => {
    if (folders().length > 6) el.sourceBar.classList.add("collapsed");
  }, { passive: true });
  el.libraryTableWrap.addEventListener("scroll", () => {
    if (folders().length > 6) el.sourceBar.classList.add("collapsed");
  }, { passive: true });

  bindTrackSelection(ctx, {
    view: "library",
    body: el.libraryTableBody,
    ctl: libraryTracksCtl,
    // Backend enumerates every id matching the current filter in one call --
    // no need to page the whole list into the client to collect ids.
    selectAll: async () => {
      const data = await command("list_matching_track_ids", {
        query: String(state.libraryQuery || "").trim(),
      });
      return (data?.trackIds || []).filter(Boolean);
    },
    actions: {
      // Backend-owned: it enumerates the ticked ids against the current
      // library filter and materializes any browse-only rows -- so a
      // selection spanning pages that have since been dropped isn't lost.
      add: (ids) => addLibrarySelectionToCurrentPlaylist({ trackIds: ids }),
      "add-to": (ids) => addSelectionToPlaylists(ctx, "library", ids, []),
      reanalyze: (ids) => reanalyzeTracks(ids),
      convert: (ids) => convertTrackSelection(ctx, ids),
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
