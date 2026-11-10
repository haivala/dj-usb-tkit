import { createTrackListController } from "../shared/track_list_controller.mjs";
import { narrowTrackSelectionToLoaded } from "../shared/track_selection.mjs";
import { applyPlaylistReorderLockToGrid } from "../shared/export_reorder_lock.mjs";
import { moveArrayItem } from "../shared/list_drag_reorder.mjs";
import { clearTrackSort, renderEmptyState } from "../shell/actions.mjs";
import { isUsbRootChangeBlocked } from "../usb/actions.mjs";
import { cssEscape, cloneTemplate } from "../../ui_utils.mjs";
import { t, tCount } from "../../ui_texts.mjs";

const PLAYLIST_LOAD_LIMIT_DEFAULT = 150;

// Builds the wire payload for add_track_candidates_to_playlist from a UI track object,
// keeping only the fields AddTrackCandidate (backend/src/models.rs) declares so UI-display
// state/sentinels (e.g. bpm: "" for "not analyzed yet") never reach the backend's strict types.
export function toAddTrackCandidatePayload(track) {
  // > 0, not just isFinite: an empty-string/undefined "not analyzed yet" sentinel coerces to 0
  // via Number(), and 0 is a valid f64 that would otherwise get materialized as a real BPM value.
  const bpm = Number(track?.bpm);
  return {
    // Backend's AddTrackCandidate.track_id declares `id` as a serde alias of
    // `trackId` -- sending both keys at once (even one as null) makes serde
    // reject the request with "duplicate field trackId", so only one may go out.
    trackId: track?.trackId ?? track?.id ?? null,
    localTrackId: track?.localTrackId ?? null,
    title: track?.title ?? "",
    artist: track?.artist ?? "",
    album: track?.album ?? null,
    bpm: Number.isFinite(bpm) && bpm > 0 ? bpm : null,
    filePath: track?.filePath ?? null,
    fileSizeBytes: track?.fileSizeBytes ?? null,
    trackNumber: track?.trackNumber ?? null,
    key: track?.key ?? null,
    formatExt: track?.formatExt ?? null,
    sampleRateHz: track?.sampleRateHz ?? null,
    bitDepth: track?.bitDepth ?? null,
    bitrateKbps: track?.bitrateKbps ?? null,
    usbAnalysisPath: track?.usbAnalysisPath ?? null,
    usbRoot: track?.usbRoot ?? null,
    usbRootValid: !!track?.usbRootValid
  };
}

export function renderPlaylistList(ctx) {
  const { state, el, document } = ctx;
  // A rebuild would drop an open rename field (a double-click's first click
  // switches view, which re-renders); the rename re-renders when it closes.
  if (el.navPlaylistList.querySelector(".nav-rename-input")) return;
  el.navPlaylistList.querySelectorAll(".nav-playlist-item").forEach((item) => item.closest("li")?.remove());
  el.navPlaylistList.querySelectorAll(".nav-new-input-wrap").forEach((wrap) => wrap.remove());

  // list_playlists returns sidebar order, top first.
  state.playlists.forEach((playlist) => {
    const li = cloneTemplate(document, "tplNavPlaylistItem");
    const btn = li.firstElementChild;
    btn.dataset.playlistId = playlist.id;
    fillPlaylistSidebarItem(btn, playlist, state.playlistUsbExportStatusById?.get(playlist.id));
    if (state.activeTab === playlist.id) btn.classList.add("active");
    if (state.currentPlaylistId === playlist.id) {
      btn.classList.add("playlist-active-mode");
      btn.dataset.tooltip = t("ui-playlist-active-tooltip");
    }
    el.navPlaylistList.appendChild(li);
  });
}

export function promptNewPlaylist(ctx) {
  const { el, document, requestAnimationFrameFn, emitStatus } = ctx;
  const existing = el.navPlaylistList.querySelector(".nav-new-input-wrap");
  if (existing) {
    existing.querySelector(".nav-new-input")?.focus();
    return;
  }

  el.addPlaylistBtn.classList.add("hidden");

  const wrap = cloneTemplate(document, "tplNavNewPlaylist");
  const input = wrap.querySelector(".nav-new-input");
  const cancel = wrap.querySelector(".nav-new-cancel");
  const addItem = el.navPlaylistList.querySelector(".nav-playlist-add-item");
  if (addItem?.nextSibling) {
    el.navPlaylistList.insertBefore(wrap, addItem.nextSibling);
  } else {
    el.navPlaylistList.appendChild(wrap);
  }

  let closed = false;
  const cleanup = () => {
    closed = true;
    wrap.remove();
    el.addPlaylistBtn.classList.remove("hidden");
  };

  const submit = createSingleSubmit(() => {
    const name = input.value.trim();
    cleanup();
    if (name) {
      createPlaylist(ctx, name).catch((err) => {
        console.error(err);
        emitStatus(err.message || String(err));
      });
    }
  });

  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") { event.preventDefault(); submit(); }
    if (event.key === "Escape") { event.preventDefault(); cleanup(); }
  });
  // Leaving the field submits -- except towards ×, which cancels. Clicking ×
  // must not take focus (WebKit doesn't focus buttons on click), so its
  // mousedown is suppressed and the input never blurs.
  input.addEventListener("blur", (event) => {
    if (!closed && event.relatedTarget !== cancel) submit();
  });
  cancel.addEventListener("mousedown", (event) => event.preventDefault());
  cancel.addEventListener("click", (event) => {
    event.preventDefault();
    cleanup();
  });

  requestAnimationFrameFn(() => input.focus());
}

export function startPlaylistRename(ctx, playlistId) {
  const { state, el, document, requestAnimationFrameFn, command, emitStatus } = ctx;
  const playlist = state.playlists.find((item) => item.id === playlistId);
  if (!playlist) return;

  const item = el.navPlaylistList.querySelector(`.nav-playlist-item[data-playlist-id="${playlistId}"]`);
  if (!item) return;

  const originalName = playlist.name;
  const input = cloneTemplate(document, "tplNavRenameInput");
  input.value = originalName;
  const itemContent = [...item.childNodes];
  item.replaceChildren(input);

  let finished = false;
  const finish = async (save) => {
    if (finished) return;
    finished = true;
    const newName = input.value.trim();
    if (save && newName && newName !== originalName) {
      try {
        applyPlaylistsState(ctx, await command("rename_playlist", { playlistId, name: newName }));
      } catch (err) {
        console.error("rename failed:", err);
        emitStatus(err?.message || String(err));
      }
    }
    item.replaceChildren(...itemContent);
    renderPlaylistList(ctx);
    const current = state.playlists.find((item) => item.id === playlistId) || playlist;
    if (state.activeTab === playlistId) {
      el.playlistPanelTitle.textContent = current.name;
      renderPlaylistExportStatus(ctx, current);
    }
    // The badge and the selection bars' "+ Add to ‹name›".
    if (getCurrentPlaylist(ctx)?.id === playlistId) ctx.updateModeText();
  };

  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") { event.preventDefault(); finish(true); }
    if (event.key === "Escape") { event.preventDefault(); finish(false); }
  });
  input.addEventListener("blur", () => finish(true));

  requestAnimationFrameFn(() => {
    input.focus();
    input.select();
  });
}

// The export status line, the backend's: the playlist's PlaylistUsbExportStatus
// line while it has one (it says whether the playlist is on the selected USB),
// else the playlist's own. Its tooltip is the drive's mount path when the line
// names the drive instead.
function renderPlaylistExportStatus(ctx, playlist) {
  const status = ctx.el.playlistExportStatus;
  const usbStatus = ctx.state.playlistUsbExportStatusById?.get(playlist?.id);
  status.textContent = usbStatus?.exportStatusText || playlist?.exportStatusText || "";
  if (playlist?.exportStatusTooltip) {
    status.dataset.tooltip = playlist.exportStatusTooltip;
  } else {
    delete status.dataset.tooltip;
  }
}

// Re-render the open playlist's export status line, e.g. after a USB scan
// changed whether it is on the connected stick.
export function renderOpenPlaylistExportStatus(ctx) {
  const { state } = ctx;
  const playlist = state.playlists.find((item) => item.id === state.activeTab);
  if (playlist) renderPlaylistExportStatus(ctx, playlist);
}

// Sidebar drag-reorder: moves the playlist locally, then saves the whole
// order. The order is local only; it never changes anything on the USB.
export async function reorderPlaylists(ctx, fromIndex, toIndex) {
  const { state } = ctx;
  state.playlists = moveArrayItem(state.playlists, fromIndex, toIndex);
  renderPlaylistList(ctx);
  try {
    await ctx.command("reorder_playlists", {
      orderedPlaylistIds: state.playlists.map((playlist) => playlist.id)
    });
  } catch (err) {
    ctx.emitStatus(err?.message || String(err));
    await loadPlaylists(ctx);
  }
}

// The backend's rows replace every stored field (an unset one comes as null);
// what a playlist's track list loaded onto it (its tracks, totals) is kept, so
// a re-read after a change doesn't empty the open playlist.
export async function loadPlaylists(ctx) {
  applyPlaylistsState(ctx, await ctx.command("list_playlists"));
}

// The playlists as the backend has them, with their export status against the
// selected USB: what list_playlists answers, and every command that changes the
// playlists answers with them too.
export function applyPlaylistsState(ctx, data) {
  const loaded = new Map(ctx.state.playlists.map((playlist) => [playlist.id, playlist]));
  ctx.state.playlists = (data.playlists || []).map((playlist) => ({
    tracks: [],
    ...loaded.get(playlist.id),
    ...playlist,
  }));
  // Draws the sidebar, the export button and the open playlist's status line.
  ctx.applyPlaylistUsbExportStatus(data.playlistUsbExportStatus);
  // The active playlist's name is known now (or changed): the header and the
  // rows' "+" say it.
  ctx.updateModeText();
}

// Column-sort on the app playlist tracklist is a free, reversible view-only
// action while browsing (it never touches the backend on its own) -- the
// currently active sort only becomes the playlist's real (and thus exported)
// order at two points: navigating away from the playlist, and exporting it.
// Both call this with the playlist the sort belongs to.
export async function commitActivePlaylistSort(ctx, playlistId) {
  const { state, command } = ctx;
  // Read the active sort *before* clearing -- clearPlaylistTrackSort deletes
  // the same state.
  const sort = ctx.tableSortState.playlistTracksBody || null;
  clearPlaylistTrackSort(ctx);
  const playlist = (state.playlists || []).find((p) => p.id === playlistId);
  if (!playlistId || !playlist || !sort?.key) return;
  if (state.playlistUsbExportStatusById?.get(playlistId)?.locksReorder) return;
  // Sort-commit mode: the backend reorders the whole playlist by these keys
  // (same comparator get_playlist_tracks uses) and persists it as the new
  // position order -- no need for the client to hold every track.
  // The reorder cleared the playlist's last export.
  applyPlaylistsState(ctx, await command("reorder_playlist_tracks", {
    playlistId,
    sortBy: sort.key,
    sortDir: sort.dir || "asc",
  }));
}

export function updatePlaylistExportButtons(ctx) {
  const { state, el } = ctx;
  const current = getCurrentPlaylist(ctx);
  const status = state.playlistUsbExportStatusById.get(current?.id);

  el.exportPlaylistBtn.disabled = isUsbRootChangeBlocked(state) || !!state.exportStarting;
  if (status) {
    el.exportPlaylistBtn.textContent = status.exportButtonText;
    el.exportPlaylistBtn.dataset.tooltip = status.exportButtonTitle;
  }

  // Offer analysis for any playlist track the backend flags as not yet
  // analysis-ready, regardless of where the file lives (a library track
  // imported from a folder on a USB stick is `isUsbPath` but still analyzable).
  // The backend export gate (ensure_playlist_tracks_analysis_ready) is the
  // hard safety net.
  // Count of not-yet-analysis-ready tracks over the WHOLE playlist, computed
  // server-side (get_playlist_tracks -> unanalyzedCount, with the button's
  // label and tooltip) -- the client only holds a page, so it can't count them
  // itself.
  const unanalyzedCount = Number(current?.unanalyzedCount) || 0;
  const showAnalyzeMissing = unanalyzedCount > 0;
  el.analyzePlaylistMissingBtn.disabled = !showAnalyzeMissing;
  el.analyzePlaylistMissingBtn.hidden = !showAnalyzeMissing;
  el.analyzePlaylistMissingBtn.textContent = current?.analyzeMissingLabel || "";
  el.analyzePlaylistMissingBtn.dataset.tooltip = current?.analyzeMissingTooltip || "";
  el.exportPlaylistBtn.hidden = showAnalyzeMissing;
}

export async function createPlaylist(ctx, name) {
  const { state, withProgress, command, updateModeText, switchView, emitStatus } = ctx;
  if (!name) {
    emitStatus(t("ui-playlist-name-required"));
    return;
  }

  const texts = {
    working: t("ui-playlist-create-progress"),
    done: t("ui-playlist-create-progress-done"),
    failed: t("ui-playlist-create-progress-failed")
  };
  const created = await withProgress(texts, async (progress) => {
    progress(35, t("ui-playlist-create-saving"));
    const playlist = await command("create_playlist", { name });
    applyPlaylistsState(ctx, playlist);
    // The new playlist becomes the active one.
    state.currentPlaylistId = playlist.playlistId;
    updateModeText();
    await switchView(playlist.playlistId);
    return playlist;
  });

  emitStatus(created.statusText);
}

// External libraries a playlist can be imported from. Each is offered when
// its database was detected; importing turns that library's source chip on.
const PLAYLIST_IMPORT_SOURCES = [
  {
    label: "rekordbox",
    pathKey: "externalRekordboxDbPath",
    listCommand: "list_rekordbox_playlists",
    library: "rekordbox",
    detect: (ctx) => ctx.detectExternalRekordboxDb()
  },
  {
    label: "Mixxx",
    pathKey: "externalMixxxDbPath",
    listCommand: "list_mixxx_playlists",
    library: "mixxx",
    detect: (ctx) => ctx.detectExternalMixxxDb()
  }
];

// [kind, group label key]
const PLAYLIST_IMPORT_KINDS = [
  ["playlist", "ui-playlist-import-group-playlists"],
  ["crate", "ui-playlist-import-group-crates"],
  ["history", "ui-playlist-import-group-history"]
];

// Pick any number of playlists / crates / history sessions from a detected
// rekordbox or Mixxx library and import each as a local playlist (the backend
// imports their tracks too, one list after another, and totals the outcome),
// then open the first one imported.
export async function importExternalPlaylist(ctx) {
  const { state, withProgress, command, updateModeText, switchView, logWarnings, emitStatus } = ctx;

  const sources = PLAYLIST_IMPORT_SOURCES.filter((source) => state[source.pathKey]);
  const listed = await Promise.all(sources.map(async (source) => {
    const path = state[source.pathKey];
    try {
      const data = await command(source.listCommand, { path });
      return (Array.isArray(data?.items) ? data.items : []).map((item) => ({ ...item, source, path }));
    } catch (err) {
      emitStatus(err?.message || String(err));
      return null;
    }
  }));
  // A column per library, its lists grouped by kind.
  const columns = sources.map((source, index) => ({
    label: source.label,
    groups: PLAYLIST_IMPORT_KINDS.map(([kind, kindLabelKey]) => ({
      label: t(kindLabelKey),
      items: (listed[index] || [])
        .filter((item) => item.kind === kind)
        // The backend's note: the track count, and the playlist an earlier
        // import made, which importing the list again updates.
        .map((item) => ({ ...item, sourceLabel: source.label, note: item.noteText }))
    }))
  }));
  if (!columns.some((column) => column.groups.some((group) => group.items.length))) {
    // Keep a listing error on the status line rather than hiding it.
    if (!listed.includes(null)) emitStatus(t("ui-playlist-import-none-found"));
    return;
  }

  const chosen = await ctx.playlistImportDialog.open({ columns });
  if (!chosen?.length) return;

  // `results` follows `chosen`: each list's playlist, or why it failed.
  let data = null;
  const done = [];
  const failed = [];
  const logFailed = () => {
    for (const { item, message } of failed) {
      logWarnings(
        item.source.label,
        [{ level: "error", message: `${item.name}: ${message}`, code: "playlist_import.failed" }],
        t("ui-playlist-import-log-context", { library: item.source.label })
      );
    }
  };
  try {
    const library = chosen.length === 1 ? chosen[0].source.label : null;
    const texts = library != null
      ? {
          working: t("ui-playlist-import-one-progress", { library }),
          done: t("ui-playlist-import-one-progress-done", { library }),
          failed: t("ui-playlist-import-one-progress-failed", { library })
        }
      : {
          working: tCount("ui-playlist-import-many-progress", chosen.length),
          done: tCount("ui-playlist-import-many-progress-done", chosen.length),
          failed: tCount("ui-playlist-import-many-progress-failed", chosen.length)
        };
    await withProgress(
      texts,
      async (progress) => {
        progress(5, chosen.length === 1
          ? t("ui-playlist-import-one-starting", { name: chosen[0].name })
          : t("ui-playlist-import-many-starting"));
        data = await command("import_external_playlists", {
          items: chosen.map((item) => ({
            library: item.source.library,
            name: item.name,
            path: item.path,
            kind: item.kind,
            id: item.id,
            force: item.force === true
          }))
        });
        for (const [index, outcome] of (data.results || []).entries()) {
          const item = chosen[index];
          if (outcome.playlist) done.push({ item, result: outcome.playlist });
          else failed.push({ item, message: outcome.error || t("ui-playlist-import-failed-unknown") });
        }
        if (!done.length) throw new Error(failed[0]?.message || t("ui-playlist-import-failed-unknown"));
        applyPlaylistsState(ctx, data);
        // The imported tracks belong to those libraries: show them in the library too.
        const imported = [...new Set(done.map(({ item }) => item.source))];
        for (const source of imported) await ctx.showExternalLibrary(source.library);
        // Re-detect: each library now counts as imported, unlocking its chip.
        for (const source of imported) await source.detect(ctx);
        await ctx.resetAndLoadLibraryTracks(state.libraryQuery || "");
        state.currentPlaylistId = done[0].result.playlistId;
        updateModeText();
        await switchView(done[0].result.playlistId);
      }
    );
  } catch (err) {
    logFailed();
    const error = err?.message || err;
    emitStatus(chosen.length === 1
      ? t("ui-playlist-import-one-failed", { library: chosen[0].source.label, error })
      : t("ui-playlist-import-many-failed", { error }));
    return;
  }

  // The files it couldn't find are in the Event Log already: the backend logged them.
  logFailed();
  emitStatus(data.statusText);
}

export async function deletePlaylist(ctx, playlistId) {
  const { state, openConfirmDialog, command, updateModeText, switchView, emitStatus } = ctx;

  if (!playlistId || state.deletingPlaylistId === playlistId) return;
  const playlist = state.playlists.find((p) => p.id === playlistId);
  if (!playlist) return;

  const confirmed = await openConfirmDialog({
    title: t("ui-playlist-delete-title"),
    message: playlist.deleteConfirmText,
    confirmLabel: t("ui-playlist-delete")
  });
  if (!confirmed) return;

  state.deletingPlaylistId = playlistId;
  try {
    const data = await command("delete_playlist", { playlistId });
    if (!data?.deleted) {
      emitStatus(data?.statusText);
      return;
    }
    applyPlaylistsState(ctx, data);
    if (state.currentPlaylistId === playlistId) {
      state.currentPlaylistId = state.playlists[0]?.id || null;
    }
    updateModeText();
    await switchView(state.currentPlaylistId || "library");
    emitStatus(data.statusText);
  } finally {
    state.deletingPlaylistId = null;
  }
}

// An add's per-candidate resolutions, onto the rows it was sent from: each
// row learns its local track id, and a listed file given its library row
// shows that row in place of its own.
// The progress bar's texts for adding tracks to a playlist, shared by every add.
export function addTracksProgressTexts() {
  return {
    working: t("ui-playlist-add-progress"),
    done: t("ui-playlist-add-progress-done"),
    failed: t("ui-playlist-add-progress-failed")
  };
}

export function applyAddResolutions(ctx, tracks, resolutions) {
  (resolutions || []).forEach((resolution, i) => {
    const resolvedId = String(resolution?.trackId || "").trim();
    if (!resolvedId) return;
    if (resolution.track) ctx.showTrackRow(resolution.track, { replaces: resolution.previousId });
    if (tracks?.[i]) tracks[i].localTrackId = resolvedId;
  });
}

export async function addTracksToCurrentPlaylist(ctx, tracks) {
  const { state, pushEventLog, withProgress, command, emitStatus } = ctx;

  const playlist = requireCurrentPlaylist(ctx);
  if (!playlist) return;
  const candidates = Array.isArray(tracks) ? tracks : [];
  if (!candidates.length) {
    emitStatus(t("ui-playlist-add-none-resolved"));
    return;
  }

  const result = await withProgress(addTracksProgressTexts(), async (progress) => {
    progress(25, t("ui-playlist-add-resolving"));
    pushEventLog({
      level: "info",
      source: "playlist-add",
      code: "playlist_add.request",
      message: tCount("ui-playlist-adding-tracks", candidates.length, { playlist: playlist.name }),
      details: `candidateIds=${candidates.map((item) => item.trackId || item.id || item.filePath || "unknown").join(",")}`
    });
    const add = await command("add_track_candidates_to_playlist", {
      playlistId: playlist.id,
      tracks: candidates.map(toAddTrackCandidatePayload),
      usbRoot: state.usbRoot || null,
      usbRootValid: !!state.usbRootValid
    });
    applyAddResolutions(ctx, tracks, add.resolutions);
    if (!Number(add.resolved || 0)) {
      return add;
    }
    // The add cleared the playlist's last export.
    applyPlaylistsState(ctx, add);
    progress(80, t("ui-playlist-add-refreshing"));
    await refreshCurrentPlaylistTracks(ctx);
    pushEventLog({
      level: "info",
      source: "playlist-add",
      code: "playlist_add.result",
      message: add.statusText,
      details: `requested=${add.requested || candidates.length} | resolved=${add.resolved || 0} | added=${add.added} | skipped=${add.skipped}`
    });
    return add;
  });
  if (!Number(result.resolved || 0)) {
    emitStatus(t("ui-playlist-add-none-resolved"));
    return;
  }
  emitStatus(result.statusText);
}

// Library "Add selected" / "Add all matching": the backend enumerates the
// selection against the current library filter (so a selection spanning pages
// the user scrolled past is not lost), materializes any browse-only rows, and
// appends -- the frontend just forwards the ids / `allMatching` flag.
export async function addLibrarySelectionToCurrentPlaylist(ctx, { trackIds = [], allMatching = false } = {}) {
  const { state, pushEventLog, withProgress, command, emitStatus } = ctx;

  const playlist = requireCurrentPlaylist(ctx);
  if (!playlist) return;
  if (!allMatching && !trackIds.length) {
    emitStatus(t("ui-playlist-add-select-first"));
    return;
  }

  const add = await withProgress(addTracksProgressTexts(), async (progress) => {
    progress(30, t("ui-playlist-add-adding"));
    return command("add_library_selection_to_playlist", {
      playlistId: playlist.id,
      query: String(state.libraryQuery || "").trim(),
      trackIds: allMatching ? [] : trackIds,
      allMatching
    });
  });

  // The add cleared the playlist's last export.
  applyPlaylistsState(ctx, add);
  await refreshCurrentPlaylistTracks(ctx);
  pushEventLog({
    level: "info",
    source: "playlist-add",
    code: "playlist_add.result",
    message: add?.statusText,
    details: `added=${add?.added || 0} | skipped=${add?.skipped || 0} | allMatching=${allMatching}`
  });
  emitStatus(add?.statusText);
}

export function createSingleSubmit(handler) {
  let submitted = false;
  return () => {
    if (submitted) return false;
    submitted = true;
    handler();
    return true;
  };
}

// Name, "exported to USB" mark and delete button of a sidebar playlist item.
// `usbStatus` is the playlist's backend PlaylistUsbExportStatus, whose
// `markLabel` says whether there is a mark and what it means.
export function fillPlaylistSidebarItem(item, playlist, usbStatus) {
  item.querySelector(".nav-playlist-name").textContent = playlist.name;
  const status = item.querySelector(".nav-playlist-status");
  const label = usbStatus?.markLabel;
  status.classList.toggle("exported", !!label);
  status.textContent = label ? "✓" : "";
  // No tooltip: hovering the row swaps the mark for the rename/delete buttons.
  if (label) {
    status.setAttribute("aria-label", label);
  } else {
    status.removeAttribute("aria-label");
  }
  item.querySelector(".nav-playlist-rename").dataset.renamePlaylist = playlist.id;
  item.querySelector(".nav-playlist-delete").dataset.deletePlaylist = playlist.id;
}

// The playlist's track count and size come from the backend, over the whole
// playlist and ignoring the search (what an export copies), with its first page
// (`listSummaryText`). The total time is under the table, not repeated here.
export function updatePlaylistPanelTitle(ctx, playlist) {
  const { el } = ctx;
  if (!playlist) return;
  const parts = [playlist.name];
  if (playlist.listSummaryText && Number(playlist.listTrackCount) > 0) parts.push(`(${playlist.listSummaryText})`);
  // Backend-derived from where the playlist was imported (see list_playlists).
  if (playlist.importedFrom) parts.push(t("ui-playlist-imported-from", { library: playlist.importedFrom }));
  el.playlistPanelTitle.textContent = parts.join(" ");
  renderPlaylistExportSpace(ctx, playlist);
}

// Ask for the export estimate once the track list is shown, without awaiting
// it, so it never holds the list up. Only the newest answer lands (a per-
// playlist request count), and it is kept with the USB it was made for, so a
// switched stick never shows another stick's numbers. A failed or missing
// estimate just leaves the line hidden.
// What export puts on the selected USB changed (another USB, its Player
// formats, a track's transcodes): the open playlist is read again, so its
// FMT column and export line follow.
export function reloadPlaylistForUsb(ctx) {
  const playlist = getCurrentPlaylist(ctx);
  if (playlist && ctx.playlistTracksCtl.scopeId === playlist.id) ctx.playlistTracksCtl.reload();
}

export function refreshPlaylistExportSpace(ctx) {
  const { state } = ctx;
  const playlist = getCurrentPlaylist(ctx);
  if (!playlist || !state.usbRoot || !state.usbRootValid) return;
  const seq = (playlist.exportSpaceSeq = (playlist.exportSpaceSeq || 0) + 1);
  const usbRoot = state.usbRoot;
  Promise.resolve()
    .then(() => ctx.command("get_playlist_export_space", { playlistId: playlist.id, usbRoot }))
    .then((data) => {
      if (seq !== playlist.exportSpaceSeq) return;
      // A playlist list reload may have swapped the object; land on the open one.
      const current = getCurrentPlaylist(ctx);
      if (current?.id !== playlist.id) return;
      current.exportSpace = data?.exportSpace ? { ...data.exportSpace, usbRoot } : null;
      renderPlaylistExportSpace(ctx, current);
    })
    .catch((err) => console.warn("get_playlist_export_space failed:", err));
}

// What exporting the playlist to the connected USB would copy, against its
// free space -- `exportSpace` from get_playlist_export_space (the backend
// decides whether it fits and counts the tracks the USB audio formats
// don't allow). Hidden without a USB or an estimate.
function renderPlaylistExportSpace(ctx, playlist) {
  const { state, el } = ctx;
  const space = state.usbRoot && state.usbRootValid && Number(playlist.listTrackCount) > 0
    && playlist.exportSpace?.usbRoot === state.usbRoot
    ? playlist.exportSpace
    : null;
  el.playlistExportSpace.classList.toggle("hidden", !space);
  renderPlaylistFormatWarning(ctx, space);
  if (!space) return;
  el.playlistExportSpace.textContent = space.summaryText;
  el.playlistExportSpace.classList.toggle("is-warning", !space.fits);
}

// What the USB audio formats mean for this playlist's export: the
// backend's `formatNotice` ("2 tracks will be converted to MP3 320 on
// export"), a warning when it blocks the export.
function renderPlaylistFormatWarning(ctx, space) {
  const { el } = ctx;
  const notice = space?.formatNotice || "";
  el.playlistFormatWarning.classList.toggle("hidden", !notice);
  el.playlistFormatWarning.classList.toggle("is-warning", !!space?.formatNoticeBlocks);
  el.playlistFormatWarning.textContent = notice;
}

export function populatePlaylistPanel(ctx, playlist) {
  const { state, el } = ctx;
  if (!playlist) return;
  updatePlaylistPanelTitle(ctx, playlist);
  renderPlaylistExportStatus(ctx, playlist);
  updatePlaylistExportButtons(ctx);
  el.playlistSearchInput.value = state.playlistTrackSearch || "";
}

export function getCurrentPlaylist(ctx) {
  const { state } = ctx;
  return state.playlists.find((p) => p.id === state.currentPlaylistId) || null;
}

function requireCurrentPlaylist(ctx) {
  const p = getCurrentPlaylist(ctx);
  if (p) return p;
  ctx.emitStatus(t("ui-playlist-none-active"));
  return null;
}

export function clearPlaylistTrackSort(ctx) {
  clearTrackSort(
    ctx.tableSortState,
    "playlistTracksBody",
    ctx.el.playlistTracksBody.closest("[data-track-grid]")
  );
}

export function isPlaylistSortActive(ctx) {
  return !!ctx.tableSortState.playlistTracksBody;
}

// The app-playlist track table's data layer -- server-paginated + searched +
// sorted via `get_playlist_tracks`, same as the other three views. `ctl.items`
// is backed by `getCurrentPlaylist().tracks` (the loaded page(s)). A column
// sort is still a *reversible view op* while browsing -- it re-queries page 1
// sorted, and only becomes the playlist's persisted order when
// `commitActivePlaylistSort` fires on navigate-away/export (which sends the
// sort params to the backend, so it reorders the whole playlist, not just what
// was loaded). Drag-reorder sends a single-move to the backend for the same
// reason.
export function createPlaylistTracksController(ctx) {
  const ctl = createTrackListController({
    bodyId: "playlistTracksBody",
    pageSize: PLAYLIST_LOAD_LIMIT_DEFAULT,
    getElements: () => ({
      body: ctx.el.playlistTracksBody,
      wrap: ctx.el.playlistTableWrap,
      durationTarget: ctx.el.playlistTotalDuration,
    }),
    fetchPage: ({ scopeId, query, sortBy, sortDir, offset, limit }) =>
      ctx.command("get_playlist_tracks", {
        playlistId: scopeId,
        query,
        sortBy: sortBy || null,
        sortDir: sortDir || null,
        offset,
        limit,
        // Each track then says what export puts on this USB (FMT column).
        usbRoot: ctx.state.usbRootValid ? ctx.state.usbRoot : null,
      }),
    normalize: (track) => ctx.normalizeTrack(track),
    getItems: () => getCurrentPlaylist(ctx)?.tracks || [],
    setItems: (value) => {
      const p = getCurrentPlaylist(ctx);
      if (p) p.tracks = value;
    },
    rowOptions: () => {
      const playlist = getCurrentPlaylist(ctx);
      const lock = playlist
        ? applyPlaylistReorderLockToGrid(
          ctx.el,
          playlist.id,
          { searchActive: !!ctl.query },
          ctx.state.playlistUsbExportStatusById,
        )
        : {};
      return {
        withCheckbox: true,
        selectedIds: ctx.state.trackSelections.playlist.ids,
        origin: "local",
        secondaryActionType: "play-library",
        enableAnalyzeActions: true,
        actionLabel: "×",
        actionType: "remove-playlist-track",
        compactAddButton: true,
        reservesDragColumn: true,
        enableDragReorder: lock.enableDragReorder,
        dragDisabledTooltip: lock.dragDisabledTooltip,
      };
    },
    buildRow: ctx.buildTrackTableRow,
    renderEmpty: ctx.renderEmptyTrackTable,
    finishRows: ctx.finishTrackRows,
    getTableSortState: () => ctx.tableSortState,
    onResponse: (data) => {
      const p = getCurrentPlaylist(ctx);
      if (p) {
        p.totalDurationMs = data.totalDurationMs;
        p.durationUnknownCount = data.durationUnknownCount;
        p.unanalyzedCount = data.unanalyzedCount;
        p.analyzeMissingLabel = data.analyzeMissingLabel;
        p.analyzeMissingTooltip = data.analyzeMissingTooltip;
        // Whole-playlist count from the backend, not just the loaded page(s).
        p.trackCount = data.total;
        p.listTrackCount = data.listTrackCount;
        p.listSummaryText = data.listSummaryText;
      }
      // Reveal the table wrap *before* the rows are built: a
      // waveform canvas measured while an ancestor is `display:none` (the
      // empty-state chrome still applied from a previously-selected empty
      // playlist) sizes to 1x1 and is never repainted when the wrap is later
      // shown, leaving every waveform blank. renderPlaylistPanelChrome() in
      // onPage still owns the authoritative empty/non-empty state afterwards.
      const hasTracks = Number(data.listTrackCount) > 0 || Number(data.total) > 0
        || (data.items || []).length > 0;
      ctx.el.playlistTableWrap.classList.toggle("hidden", !hasTracks);
      ctx.el.playlistTotalDuration.classList.toggle("hidden", !hasTracks);
    },
    onPage: (_page, { first }) => {
      if (first) narrowTrackSelectionToLoaded(ctx, "playlist", getCurrentPlaylist(ctx)?.tracks || []);
      renderPlaylistPanelChrome(ctx);
      if (first) refreshPlaylistExportSpace(ctx);
    },
  });
  return ctl;
}

// The "add tracks" screen of an empty playlist, or the table and the
// controls that go with it. Empty only once this playlist's tracks have
// loaded and it has none at all: while they load the table shows "Loading
// tracks…", and a search that matches nothing still leaves tracks to search.
function renderPlaylistEmptyState(ctx, playlist) {
  const { state, el, playlistTracksCtl } = ctx;
  const loaded = playlistTracksCtl.scopeId === playlist.id && !playlistTracksCtl.loading;
  const empty = loaded && playlistTracksCtl.total === 0 && !(Number(playlist.listTrackCount) > 0);
  el.playlistEmptyState.replaceChildren();
  if (empty) {
    // With a USB selected, its playlists are the tracks to add; without
    // one, the USB view is where to select it.
    const hasUsb = !!state.usbRoot && !!state.usbRootValid;
    renderEmptyState(el.playlistEmptyState, {
      icon: "♫",
      heading: t("ui-playlist-empty-heading"),
      actionLabel: t("ui-playlist-empty-library"),
      onAction: () => ctx.switchView("library").catch((err) => console.error(err)),
      extraActions: [{
        label: t(hasUsb ? "ui-playlist-empty-usb-playlists" : "ui-playlist-empty-usb"),
        onAction: () => ctx.switchView(hasUsb ? "usb-playlists" : "usb").catch((err) => console.error(err)),
      }],
    });
  }
  el.playlistTableWrap.classList.toggle("hidden", empty);
  el.playlistTotalDuration.classList.toggle("hidden", empty);
  // Export status says nothing useful about a playlist with no tracks.
  el.playlistExportStatus.classList.toggle("hidden", empty);
  el.playlistSearchInput.closest(".track-toolbar")?.classList.toggle("hidden", empty);
  el.exportPlaylistBtn.closest(".panel-actions")?.classList.toggle("hidden", empty);
}

// Panel glue around the playlist track table that the controller does not own:
// empty state, section visibility, search-input restore, is-analyzing pulse,
// title, export buttons, sidebar. Run after every (re)render of the list.
export function renderPlaylistPanelChrome(ctx) {
  const { state, el } = ctx;
  const playlist = getCurrentPlaylist(ctx);
  if (!playlist) return;
  renderPlaylistEmptyState(ctx, playlist);
  if (el.playlistSearchInput.value !== (state.playlistTrackSearch || "")) {
    el.playlistSearchInput.value = state.playlistTrackSearch || "";
  }
  for (const id of state.analyzingTrackIds) {
    const row = el.playlistTracksBody.querySelector(
      `.track-grid-row[data-track-id="${cssEscape(id)}"][data-track-origin="local"]`,
    );
    if (row) row.classList.add("is-analyzing");
  }
  updatePlaylistPanelTitle(ctx, playlist);
  updatePlaylistExportButtons(ctx);
  renderPlaylistList(ctx);
}

// Re-render the open playlist's track table from the already-loaded
// `playlist.tracks` (no fetch) -- used when only the view changed: a
// reorder-lock flip after a USB scan, a cross-view analysis patch.
export async function renderCurrentPlaylistTracksFromState(ctx) {
  if (!getCurrentPlaylist(ctx)) return;
  await ctx.playlistTracksCtl.rerender();
  renderPlaylistPanelChrome(ctx);
}

// Fetch the open playlist's tracks from the backend and render. `ctl.load`
// replaces `getCurrentPlaylist().tracks` via setItems.
export async function refreshCurrentPlaylistTracks(ctx) {
  const { state, playlistTracksCtl } = ctx;
  const playlist = getCurrentPlaylist(ctx);
  if (!playlist) return;
  playlistTracksCtl.query = String(state.playlistTrackSearch || "");
  // Keep the controller's sort in lockstep with the header UI state -- a drag
  // clears the sort (deletes tableSortState) but leaves ctl.sortBy stale.
  const st = ctx.tableSortState.playlistTracksBody || null;
  playlistTracksCtl.sortBy = st?.key || null;
  playlistTracksCtl.sortDir = st?.dir || null;
  // `load` marks the list loading before its first await, so a previous
  // empty playlist's screen gives way to the loading table at once. Only
  // that part: the full chrome re-renders the sidebar, which would swallow
  // a double-click's second click.
  const loading = playlistTracksCtl.load({ scopeId: playlist.id });
  renderPlaylistEmptyState(ctx, playlist);
  await loading;
  renderPlaylistPanelChrome(ctx);
}
