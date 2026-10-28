import { createTrackListController } from "../shared/track_list_controller.mjs";
import { narrowTrackSelectionToLoaded } from "../shared/track_selection.mjs";
import { applyPlaylistReorderLockToGrid } from "../shared/export_reorder_lock.mjs";
import { moveArrayItem } from "../shared/list_drag_reorder.mjs";
import { clearTrackSort, renderEmptyState } from "../shell/actions.mjs";
import { computeExportButtonState, isUsbRootChangeBlocked } from "../usb/actions.mjs";
import { cssEscape, cloneTemplate } from "../../ui_utils.mjs";
import {
  formatDurationMs,
  formatTimestampLocal,
  renderTrackListDurationSummary,
  formatTrackListSummary,
  formatByteSize,
  ANALYZE_PLAYLIST_MISSING_TOOLTIP,
} from "../../track_utils.mjs";

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
      btn.dataset.tooltip = "Active playlist";
    }
    el.navPlaylistList.appendChild(li);
  });
}

export function promptNewPlaylist(ctx) {
  const { el, document, requestAnimationFrameFn } = ctx;
  const { emitStatus } = ctx;
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
  const { state, el, document, requestAnimationFrameFn, command } = ctx;
  const { emitStatus } = ctx;
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
        const data = await command("rename_playlist", { playlistId, name: newName });
        // loadPlaylists may have replaced the list while the field was open.
        const current = state.playlists.find((item) => item.id === playlistId) || playlist;
        current.name = data.name;
        current.lastExportedAt = null;
        current.lastExportedUsbRoot = null;
        current.lastExportedTrackCount = null;
        refreshUsbExportStatusInBackground(ctx);
      } catch (err) {
        console.error("rename failed:", err);
        emitStatus(`Rename failed: ${err.message || err}`);
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

// `usbStatus` is the playlist's backend PlaylistUsbExportStatus for the
// connected USB (see fillPlaylistSidebarItem): with one, the line leads with
// whether the playlist is on that stick, then the last export, if any.
export function formatPlaylistExportStatus(playlist, usbStatus) {
  const lastExport = formatLastExport(playlist);
  if (usbStatus) {
    const onUsb = usbStatus.sameNameExistsOnUsb ? "On this USB." : "Not on this USB.";
    return lastExport ? `${onUsb} ${lastExport}` : onUsb;
  }
  return lastExport || "Not exported yet.";
}

function formatLastExport(playlist) {
  const when = String(playlist?.lastExportedAt || "").trim();
  if (!when) return null;
  const formattedWhen = formatTimestampLocal(when);
  const root = String(playlist?.lastExportedUsbRoot || "").trim();
  // The drive's name (backend-resolved) reads better than its mount path.
  const drive = String(playlist?.lastExportedUsbName || "").trim() || root;
  const count = Number(playlist?.lastExportedTrackCount);
  const countText = Number.isFinite(count) && count >= 0 ? `${count} track(s)` : "unknown track count";
  const driveText = drive ? ` to ${drive}` : "";
  return `Last exported ${formattedWhen}${driveText} (${countText}).`;
}

// The export status line, with the drive's mount path as its tooltip when
// the line names the drive instead.
function renderPlaylistExportStatus(ctx, playlist) {
  const status = ctx.el.playlistExportStatus;
  const usbStatus = ctx.state.playlistUsbExportStatusById?.get(playlist?.id);
  status.textContent = formatPlaylistExportStatus(playlist, usbStatus);
  const root = String(playlist?.lastExportedUsbRoot || "").trim();
  if (playlist?.lastExportedAt && playlist?.lastExportedUsbName && root) {
    status.dataset.tooltip = root;
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
    ctx.emitStatus(`Save playlist order failed: ${err.message || err}`);
    await loadPlaylists(ctx);
  }
}

export async function loadPlaylists(ctx) {
  const data = await ctx.command("list_playlists");
  ctx.state.playlists = (data.items || []).map((playlist) => ({ ...playlist, tracks: [] }));
  renderPlaylistList(ctx);
  updatePlaylistExportButtons(ctx);
  refreshUsbExportStatusInBackground(ctx);
}

// The USB export status matches playlists by name, so a create, rename,
// delete or export leaves it stale; the backend recompute is cheap (staged
// PDB/eDB only). Fire-and-forget: a failed refresh only leaves the marks stale.
function refreshUsbExportStatusInBackground(ctx) {
  const { state } = ctx;
  if (!state.usbRoot || !state.usbRootValid || !ctx.refreshPlaylistExportStatus) return;
  Promise.resolve()
    .then(() => ctx.refreshPlaylistExportStatus())
    .catch((err) => console.warn("refresh_playlist_export_status failed:", err));
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
  await command("reorder_playlist_tracks", {
    playlistId,
    sortBy: sort.key,
    sortDir: sort.dir || "asc",
  });
}

export function updatePlaylistExportButtons(ctx) {
  const { state, el } = ctx;
  const current = getCurrentPlaylist(ctx);
  const buttonState = computeExportButtonState({
    usbRoot: state.usbRoot,
    usbRootValid: state.usbRootValid,
    currentPlaylistId: current?.id,
    playlistUsbExportStatusById: state.playlistUsbExportStatusById
  });

  el.exportPlaylistBtn.disabled = isUsbRootChangeBlocked(state) || !!state.exportStarting;
  el.exportPlaylistBtn.textContent = buttonState.text;
  el.exportPlaylistBtn.dataset.tooltip = buttonState.title;

  // Offer analysis for any playlist track the backend flags as not yet
  // analysis-ready, regardless of where the file lives (a library track
  // imported from a folder on a USB stick is `isUsbPath` but still analyzable).
  // The backend export gate (ensure_playlist_tracks_analysis_ready) is the
  // hard safety net.
  // Count of not-yet-analysis-ready tracks over the WHOLE playlist, computed
  // server-side (get_playlist_tracks -> unanalyzedCount) -- the client only
  // holds a page, so it can't count them itself.
  const unanalyzedCount = Number(current?.unanalyzedCount) || 0;
  const showAnalyzeMissing = unanalyzedCount > 0;
  if (el.analyzePlaylistMissingBtn) {
    el.analyzePlaylistMissingBtn.disabled = !showAnalyzeMissing;
    el.analyzePlaylistMissingBtn.hidden = !showAnalyzeMissing;
    el.analyzePlaylistMissingBtn.textContent = showAnalyzeMissing
      ? `Analyze Missing Tracks (${unanalyzedCount})`
      : "Analyze Missing Tracks";
    el.analyzePlaylistMissingBtn.dataset.tooltip = showAnalyzeMissing
      ? ANALYZE_PLAYLIST_MISSING_TOOLTIP
      : "No tracks in this playlist need analysis";
  }
  if (el.exportPlaylistBtn) {
    el.exportPlaylistBtn.hidden = showAnalyzeMissing;
  }
}

export async function createPlaylist(ctx, name) {
  const { state, withProgress, command, updateModeText, switchView } = ctx;
  const { emitStatus } = ctx;
  if (!name) {
    emitStatus("Playlist name is required");
    return;
  }

  const created = await withProgress("Creating playlist", async (progress) => {
    progress(35, "Saving playlist...");
    const playlist = await command("create_playlist", { name });
    progress(70, "Refreshing playlists...");
    await loadPlaylists(ctx);
    const loadedPlaylists = Array.isArray(state.playlists) ? state.playlists : [];
    const createdPlaylistId = String(playlist?.playlistId || "").trim();
    const selectedPlaylist = loadedPlaylists.find((item) => String(item?.id || "") === createdPlaylistId)
      || (createdPlaylistId
        ? null
        : loadedPlaylists.find((item) => String(item?.name || "") === String(playlist?.name || "")))
      || loadedPlaylists[0]
      || null;
    const selectedPlaylistId = selectedPlaylist?.id || createdPlaylistId;
    state.currentPlaylistId = selectedPlaylistId;
    updateModeText();
    if (selectedPlaylistId) {
      await switchView(selectedPlaylistId);
    }
    return playlist;
  });

  emitStatus(`Playlist created: ${created.name}`);
}

// External libraries a playlist can be imported from. Each is offered when
// its database was detected; importing turns that library's source chip on.
const PLAYLIST_IMPORT_SOURCES = [
  {
    label: "rekordbox",
    pathKey: "externalRekordboxDbPath",
    listCommand: "list_rekordbox_playlists",
    importCommand: "import_rekordbox_playlist",
    enable: (ctx) => {
      ctx.state.rekordboxDbEnabled = true;
      ctx.persistRekordboxDbEnabled(true);
    },
    detect: (ctx) => ctx.detectExternalRekordboxDb()
  },
  {
    label: "Mixxx",
    pathKey: "externalMixxxDbPath",
    listCommand: "list_mixxx_playlists",
    importCommand: "import_mixxx_playlist",
    enable: (ctx) => {
      ctx.state.mixxxDbEnabled = true;
      ctx.persistMixxxDbEnabled(true);
    },
    detect: (ctx) => ctx.detectExternalMixxxDb()
  }
];

// [kind, group label]
const PLAYLIST_IMPORT_KINDS = [
  ["playlist", "Playlists"],
  ["crate", "Crates"],
  ["history", "History"]
];

// Pick any number of playlists / crates / history sessions from a detected
// rekordbox or Mixxx library and import each as a local playlist (the backend
// imports its tracks too), then open the first one.
export async function importExternalPlaylist(ctx) {
  const { state, withProgress, command, updateModeText, switchView, logWarnings } = ctx;
  const { emitStatus } = ctx;

  const sources = PLAYLIST_IMPORT_SOURCES.filter((source) => state[source.pathKey]);
  const listed = await Promise.all(sources.map(async (source) => {
    const path = state[source.pathKey];
    try {
      const data = await command(source.listCommand, { path });
      return (Array.isArray(data?.items) ? data.items : []).map((item) => ({ ...item, source, path }));
    } catch (err) {
      emitStatus(`Could not read ${source.label} playlists: ${err?.message || err}`);
      return null;
    }
  }));
  // A column per library, its lists grouped by kind.
  const columns = sources.map((source, index) => ({
    label: source.label,
    groups: PLAYLIST_IMPORT_KINDS.map(([kind, kindLabel]) => ({
      label: kindLabel,
      items: (listed[index] || [])
        .filter((item) => item.kind === kind)
        .map((item) => ({
          ...item,
          sourceLabel: source.label,
          note: [
            `${item.trackCount} ${item.trackCount === 1 ? "track" : "tracks"}`,
            // Importing a list again updates the playlist its earlier import made.
            ...(item.existingPlaylist ? [`updates "${item.existingPlaylist.name}"`] : [])
          ].join(" · ")
        }))
    }))
  }));
  if (!columns.some((column) => column.groups.some((group) => group.items.length))) {
    // Keep a listing error on the status line rather than hiding it.
    if (!listed.includes(null)) emitStatus("No playlists with tracks found");
    return;
  }

  const chosen = await ctx.playlistImportDialog.open({ columns });
  if (!chosen?.length) return;

  // One import after another; a failed one doesn't stop the rest.
  const done = [];
  const failed = [];
  const logFailed = () => {
    for (const { item, message } of failed) {
      logWarnings(
        item.source.label,
        [{ level: "error", message: `${item.name}: ${message}`, code: "playlist_import.failed" }],
        `${item.source.label} playlist import`
      );
    }
  };
  try {
    await withProgress(
      chosen.length === 1 ? `Importing ${chosen[0].source.label} playlist` : `Importing ${chosen.length} playlists`,
      async (progress) => {
        for (const [index, item] of chosen.entries()) {
          progress(5 + Math.round((index / chosen.length) * 70), `Importing ${item.name}...`);
          try {
            const result = await command(item.source.importCommand, {
              path: item.path, kind: item.kind, id: item.id, force: item.force === true
            });
            done.push({ item, result });
          } catch (err) {
            failed.push({ item, message: err?.message || String(err) });
          }
        }
        if (!done.length) throw new Error(failed[0].message);
        progress(80, "Refreshing playlists...");
        // The imported tracks belong to those libraries: show them in the library too.
        const imported = [...new Set(done.map(({ item }) => item.source))];
        for (const source of imported) source.enable(ctx);
        state.sourcesEverConfigured = true;
        ctx.persistSourcesEverConfigured(true);
        // Re-detect: each library now counts as imported, unlocking its chip.
        for (const source of imported) await source.detect(ctx);
        await ctx.resetAndLoadLibraryTracks(state.libraryQuery || "");
        await loadPlaylists(ctx);
        state.currentPlaylistId = done[0].result.playlistId;
        updateModeText();
        await switchView(done[0].result.playlistId);
      }
    );
  } catch (err) {
    logFailed();
    const what = chosen.length === 1 ? `${chosen[0].source.label} playlist` : "Playlist";
    emitStatus(`${what} import failed: ${err?.message || err}`);
    return;
  }

  let skipped = 0;
  for (const { item, result } of done) {
    const logLabel = `${item.source.label} playlist import`;
    const notFound = Array.isArray(result.notFound) ? result.notFound : [];
    if (notFound.length > 0) {
      logWarnings(
        item.source.label,
        notFound.map((p) => ({ level: "warn", message: p, code: "playlist_import.file_not_found" })),
        logLabel
      );
    }
    const warnings = Array.isArray(result.warnings) ? result.warnings : [];
    if (warnings.length > 0) {
      logWarnings(item.source.label, warnings, logLabel);
    }
    skipped += Math.max(0, Number(item.trackCount || 0) - Number(result.added || 0));
  }
  logFailed();

  const suffix = skipped > 0 ? ` | ${skipped} track(s) skipped (see event log)` : "";
  if (chosen.length === 1) {
    const { result } = done[0];
    const verb = result.updatedExisting ? "Updated" : "Imported";
    emitStatus(`${verb} playlist ${result.name}: ${result.added} track(s)${suffix}`);
    return;
  }
  const added = done.reduce((sum, { result }) => sum + Number(result.added || 0), 0);
  const failedSuffix = failed.length
    ? ` | ${failed.length} failed: ${failed.map(({ item }) => item.name).join(", ")} (see event log)`
    : "";
  emitStatus(`Imported ${done.length} of ${chosen.length} playlists: ${added} track(s)${suffix}${failedSuffix}`);
}

export async function deletePlaylist(ctx, playlistId) {
  const { state, openConfirmDialog, command, updateModeText, switchView } = ctx;
  const { emitStatus } = ctx;

  if (!playlistId || state.deletingPlaylistId === playlistId) return;
  const playlist = state.playlists.find((p) => p.id === playlistId);
  if (!playlist) return;

  const exportedHint = playlist.lastExportedAt
    ? "\n\nThis playlist was exported before. It is easy to recreate by importing playlists from USB."
    : "";
  const confirmed = await openConfirmDialog({
    title: "Delete App Playlist",
    message: `Delete "${playlist.name}"?\n\nThis removes the app playlist and its app playlist-track links.${exportedHint}`,
    confirmLabel: "Delete"
  });
  if (!confirmed) return;

  state.deletingPlaylistId = playlistId;
  try {
    const data = await command("delete_playlist", { playlistId });
    if (!data?.deleted) {
      emitStatus(`Delete failed: ${playlist.name}`);
      return;
    }
    await loadPlaylists(ctx);
    if (state.currentPlaylistId === playlistId) {
      state.currentPlaylistId = state.playlists[0]?.id || null;
    }
    updateModeText();
    await switchView(state.currentPlaylistId || "library");
    emitStatus(`Playlist deleted: ${playlist.name}`);
  } finally {
    state.deletingPlaylistId = null;
  }
}

// An add's per-candidate resolutions, onto the rows it was sent from: each
// row learns its local track id, and a row that had another id hands its
// identity over to the resolved one.
export function applyAddResolutions(ctx, tracks, resolutions) {
  (resolutions || []).forEach((resolution, i) => {
    const resolvedId = String(resolution?.trackId || "").trim();
    if (!resolvedId) return;
    if (tracks?.[i]) tracks[i].localTrackId = resolvedId;
    const previousId = String(resolution.previousId || "").trim();
    if (previousId && previousId !== resolvedId) ctx.promoteTrackIdentity(previousId, resolvedId);
  });
}

export async function addTracksToCurrentPlaylist(ctx, tracks) {
  const { state, pushEventLog, withProgress, command } = ctx;
  const { emitStatus } = ctx;

  const playlist = requireCurrentPlaylist(ctx);
  if (!playlist) return;
  const candidates = Array.isArray(tracks) ? tracks : [];
  if (!candidates.length) {
    emitStatus("No imported track IDs found to add");
    return;
  }

  const result = await withProgress("Adding tracks", async (progress) => {
    progress(25, "Resolving tracks...");
    pushEventLog({
      level: "info",
      source: "playlist-add",
      code: "playlist_add.request",
      message: `Adding ${candidates.length} track(s) to ${playlist.name}`,
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
    // Backend unconditionally nulls out last_exported_* on every
    // add-track call (mod.rs) -- mirror that here so the
    // sidebar's "exported to USB" checkmark doesn't keep showing stale
    // state until an unrelated list_playlists refetch happens.
    playlist.lastExportedAt = null;
    playlist.lastExportedUsbRoot = null;
    playlist.lastExportedTrackCount = null;
    progress(80, "Refreshing playlist...");
    await refreshCurrentPlaylistTracks(ctx);
    pushEventLog({
      level: "info",
      source: "playlist-add",
      code: "playlist_add.result",
      message: `Added ${add.added} track(s) to ${playlist.name}`,
      details: `requested=${add.requested || candidates.length} | resolved=${add.resolved || 0} | added=${add.added} | skipped=${add.skipped}`
    });
    return add;
  });
  if (!Number(result.resolved || 0)) {
    emitStatus("No imported track IDs found to add");
    return;
  }
  emitStatus(`Added ${result.added} tracks (skipped ${result.skipped}) to ${playlist.name}`);
}

// Library "Add selected" / "Add all matching": the backend enumerates the
// selection against the current library filter (so a selection spanning pages
// the user scrolled past is not lost), materializes any browse-only rows, and
// appends -- the frontend just forwards the ids / `allMatching` flag.
export async function addLibrarySelectionToCurrentPlaylist(ctx, { trackIds = [], allMatching = false } = {}) {
  const { state, pushEventLog, withProgress, command } = ctx;
  const { emitStatus } = ctx;

  const playlist = requireCurrentPlaylist(ctx);
  if (!playlist) return;
  if (!allMatching && !trackIds.length) {
    emitStatus("Select at least one track to add");
    return;
  }

  const add = await withProgress("Adding tracks", async (progress) => {
    progress(30, "Adding tracks...");
    return command("add_library_selection_to_playlist", {
      playlistId: playlist.id,
      sourceRoots: ctx.enabledLibrarySourceRoots(),
      includeRekordboxDb: state.rekordboxDbEnabled === true,
      includeMixxxDb: state.mixxxDbEnabled === true,
      query: String(state.libraryQuery || "").trim(),
      trackIds: allMatching ? [] : trackIds,
      allMatching
    });
  });

  // Mirror the backend unconditionally clearing last_exported_* on add.
  playlist.lastExportedAt = null;
  playlist.lastExportedUsbRoot = null;
  playlist.lastExportedTrackCount = null;
  await refreshCurrentPlaylistTracks(ctx);
  pushEventLog({
    level: "info",
    source: "playlist-add",
    code: "playlist_add.result",
    message: `Added ${add?.added || 0} track(s) to ${playlist.name}`,
    details: `added=${add?.added || 0} | skipped=${add?.skipped || 0} | allMatching=${allMatching}`
  });
  emitStatus(`Added ${add?.added || 0} tracks (skipped ${add?.skipped || 0}) to ${playlist.name}`);
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
// `usbStatus` is the playlist's backend PlaylistUsbExportStatus for the
// connected USB: with one, the mark says whether the playlist is on that
// stick; without (no USB scanned), it falls back to "ever exported".
export function fillPlaylistSidebarItem(item, playlist, usbStatus) {
  item.querySelector(".nav-playlist-name").textContent = playlist.name;
  const status = item.querySelector(".nav-playlist-status");
  const exported = usbStatus ? !!usbStatus.sameNameExistsOnUsb : !!playlist.lastExportedAt;
  status.classList.toggle("exported", exported);
  status.textContent = exported ? "✓" : "";
  // No tooltip: hovering the row swaps the mark for the rename/delete buttons.
  if (exported) {
    status.setAttribute("aria-label", usbStatus ? "On this USB" : "Exported to USB");
  } else {
    status.removeAttribute("aria-label");
  }
  item.querySelector(".nav-playlist-rename").dataset.renamePlaylist = playlist.id;
  item.querySelector(".nav-playlist-delete").dataset.deletePlaylist = playlist.id;
}

// The playlist's track count and size are computed by the backend over the
// whole playlist, ignoring the search (what an export copies), and pushed here
// via playlist.listTrackCount / listSizeBytes -- see
// GetPlaylistTracksData::list_track_count. This is a pure setter, no track
// iteration.
export function updatePlaylistPanelTitle(ctx, playlist) {
  const { el } = ctx;
  if (!el?.playlistPanelTitle || !playlist) return;
  const loaded = Array.isArray(playlist.tracks) ? playlist.tracks.length : 0;
  // Prefer the backend's whole-playlist count; before the first page response
  // has set it, fall back to the sidebar listing's count, then the loaded one.
  const count = Number(playlist.listTrackCount) || Number(playlist.trackCount) || loaded;
  const parts = [playlist.name];
  // The total time is under the table (playlistTotalDuration), not repeated here.
  if (count > 0) {
    parts.push(`(${formatTrackListSummary(count, playlist.listSizeBytes)})`);
  }
  // Backend-derived from where the playlist was imported (see list_playlists).
  if (playlist.importedFrom) parts.push(`· Imported from ${playlist.importedFrom}`);
  el.playlistPanelTitle.textContent = parts.join(" ");
  renderPlaylistExportSpace(ctx, playlist);
}

// Ask for the export estimate once the track list is shown, without awaiting
// it, so it never holds the list up. Only the newest answer lands (a per-
// playlist request count), and it is kept with the USB it was made for, so a
// switched stick never shows another stick's numbers. A failed or missing
// estimate just leaves the line hidden.
function refreshPlaylistExportSpaceInBackground(ctx) {
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
// decides whether it fits). Hidden without a USB or an estimate.
function renderPlaylistExportSpace(ctx, playlist) {
  const { state, el } = ctx;
  if (!el?.playlistExportSpace) return;
  const space = state.usbRoot && state.usbRootValid && Number(playlist.listTrackCount) > 0
    && playlist.exportSpace?.usbRoot === state.usbRoot
    ? playlist.exportSpace
    : null;
  el.playlistExportSpace.classList.toggle("hidden", !space);
  if (!space) return;
  const free = space.availableBytes == null ? null : `${formatByteSize(space.availableBytes)} free on USB`;
  const tracks = Number(space.tracksToCopy) || 0;
  const parts = tracks > 0
    ? [`${tracks} of ${Number(playlist.listTrackCount)} tracks to copy`, formatByteSize(space.bytesToCopy)]
    : ["All tracks on USB"];
  if (free) parts.push(free);
  const text = `${space.fits ? "" : "Doesn't fit: "}${parts.join(" · ")}`;
  el.playlistExportSpace.textContent = text;
  el.playlistExportSpace.classList.toggle("is-warning", !space.fits);
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
  ctx.emitStatus("Create and activate a playlist first");
  return null;
}

export function clearPlaylistTrackSort(ctx) {
  clearTrackSort(
    ctx.tableSortState,
    "playlistTracksBody",
    ctx.el.playlistTracksBody?.closest("[data-track-grid]")
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
          playlist,
          { searchActive: !!ctl.query },
          ctx.state.playlistUsbExportStatusById,
        )
        : {};
      return {
        withCheckbox: true,
        selectedIds: ctx.state.trackSelections.playlist.ids,
        origin: "local",
        secondaryActionLabel: "Play",
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
    renderDurationSummary: (target, summary) =>
      renderTrackListDurationSummary(target, summary, formatDurationMs),
    getTableSortState: () => ctx.tableSortState,
    onResponse: (data) => {
      const p = getCurrentPlaylist(ctx);
      if (p) {
        p.totalDurationMs = Number(data.totalDurationMs) || 0;
        p.durationKnownCount = Number(data.durationKnownCount) || 0;
        p.unanalyzedCount = Number(data.unanalyzedCount) || 0;
        // Whole-playlist count from the backend, not just the loaded page(s).
        p.trackCount = Number(data.total) || 0;
        p.listTrackCount = Number(data.listTrackCount) || 0;
        p.listSizeBytes = Number(data.listSizeBytes) || 0;
      }
      // Reveal the table wrap *before* the rows are built: a
      // waveform canvas measured while an ancestor is `display:none` (the
      // empty-state chrome still applied from a previously-selected empty
      // playlist) sizes to 1x1 and is never repainted when the wrap is later
      // shown, leaving every waveform blank. renderPlaylistPanelChrome() in
      // onPage still owns the authoritative empty/non-empty state afterwards.
      const hasTracks = Number(data.listTrackCount) > 0 || Number(data.total) > 0
        || (data.items || []).length > 0;
      ctx.el.playlistTableWrap?.classList.toggle("hidden", !hasTracks);
      ctx.el.playlistTotalDuration?.classList.toggle("hidden", !hasTracks);
    },
    onPage: (_page, { first }) => {
      if (first) narrowTrackSelectionToLoaded(ctx, "playlist", getCurrentPlaylist(ctx)?.tracks || []);
      renderPlaylistPanelChrome(ctx);
      if (first) refreshPlaylistExportSpaceInBackground(ctx);
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
  if (el.playlistEmptyState) {
    el.playlistEmptyState.replaceChildren();
    if (empty) {
      // With a USB selected, its playlists are the tracks to add; without
      // one, the USB view is where to select it.
      const hasUsb = !!state.usbRoot && !!state.usbRootValid;
      renderEmptyState(el.playlistEmptyState, {
        icon: "♫",
        heading: "Browse Library or USB to add tracks",
        actionLabel: "Media Library",
        onAction: () => ctx.switchView("library").catch((err) => console.error(err)),
        extraActions: [{
          label: hasUsb ? "USB Playlists" : "USB",
          onAction: () => ctx.switchView(hasUsb ? "usb-playlists" : "usb").catch((err) => console.error(err)),
        }],
      });
    }
  }
  el.playlistTableWrap?.classList.toggle("hidden", empty);
  el.playlistTotalDuration?.classList.toggle("hidden", empty);
  // "Not exported yet." says nothing useful about a playlist with no tracks.
  el.playlistExportStatus?.classList.toggle("hidden", empty && !playlist.lastExportedAt);
  el.playlistSearchInput?.closest(".track-toolbar")?.classList.toggle("hidden", empty);
  el.exportPlaylistBtn?.closest(".playlist-actions")?.classList.toggle("hidden", empty);
}

// Panel glue around the playlist track table that the controller does not own:
// empty state, section visibility, search-input restore, is-analyzing pulse,
// title, export buttons, sidebar. Run after every (re)render of the list.
export function renderPlaylistPanelChrome(ctx) {
  const { state, el } = ctx;
  const playlist = getCurrentPlaylist(ctx);
  if (!playlist) return;
  renderPlaylistEmptyState(ctx, playlist);
  if (el.playlistSearchInput && el.playlistSearchInput.value !== (state.playlistTrackSearch || "")) {
    el.playlistSearchInput.value = state.playlistTrackSearch || "";
  }
  for (const id of state.analyzingTrackIds) {
    const row = el.playlistTracksBody?.querySelector(
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
