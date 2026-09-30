import { createTrackListController } from "../shared/track_list_controller.mjs";
import { applyPlaylistReorderLockToGrid } from "../shared/export_reorder_lock.mjs";
import { clearTrackSort, renderEmptyState } from "../shell/actions.mjs";
import { computeExportButtonState, isUsbRootChangeBlocked } from "../usb/actions.mjs";
import { cssEscape, cloneTemplate } from "../../ui_utils.mjs";
import { formatDurationMs, formatTimestampLocal, renderTrackListDurationSummary } from "../../track_utils.mjs";

const PLAYLIST_LOAD_LIMIT_DEFAULT = 150;

// Builds the wire payload for add_track_candidates_to_playlist from a UI track object,
// keeping only the fields AddTrackCandidate (backend/src/models.rs) declares so UI-display
// state/sentinels (e.g. bpm: "" for "not analyzed yet") never reach the backend's strict types.
function toAddTrackCandidatePayload(track) {
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
  el.navPlaylistList.querySelectorAll(".nav-playlist-item").forEach((item) => item.closest("li")?.remove());
  el.navPlaylistList.querySelectorAll(".nav-new-input-wrap").forEach((wrap) => wrap.remove());

  [...state.playlists].reverse().forEach((playlist) => {
    const li = cloneTemplate(document, "tplNavPlaylistItem");
    const btn = li.firstElementChild;
    btn.dataset.playlistId = playlist.id;
    fillPlaylistSidebarItem(btn, playlist);
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
        playlist.name = data.name;
        playlist.lastExportedAt = null;
        playlist.lastExportedUsbRoot = null;
        playlist.lastExportedTrackCount = null;
      } catch (err) {
        console.error("rename failed:", err);
        emitStatus(`Rename failed: ${err.message || err}`);
      }
    }
    item.replaceChildren(...itemContent);
    fillPlaylistSidebarItem(item, playlist);
    if (state.activeTab === playlistId) {
      el.playlistPanelTitle.textContent = playlist.name;
      renderPlaylistExportStatus(el, playlist);
    }
    const badge = getCurrentPlaylist(ctx);
    if (badge?.id === playlistId) {
      el.badgeLabel.textContent = playlist.name;
    }
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

export function formatPlaylistExportStatus(playlist) {
  const when = String(playlist?.lastExportedAt || "").trim();
  if (!when) return "Not exported yet.";
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
function renderPlaylistExportStatus(el, playlist) {
  const status = el.playlistExportStatus;
  status.textContent = formatPlaylistExportStatus(playlist);
  const root = String(playlist?.lastExportedUsbRoot || "").trim();
  if (playlist?.lastExportedAt && playlist?.lastExportedUsbName && root) {
    status.dataset.tooltip = root;
  } else {
    delete status.dataset.tooltip;
  }
}

export async function loadPlaylists(ctx) {
  const data = await ctx.command("list_playlists");
  ctx.state.playlists = (data.items || []).map((playlist) => ({ ...playlist, tracks: [] }));
  renderPlaylistList(ctx);
  updatePlaylistExportButtons(ctx);
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

  el.exportPlaylistBtn.disabled = isUsbRootChangeBlocked(state);
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
      ? "Analyze missing waveform, BPM, and duration for tracks in this playlist"
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
      || loadedPlaylists[loadedPlaylists.length - 1]
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
    pathKey: "externalMasterDbPath",
    listCommand: "list_rekordbox_playlists",
    importCommand: "import_rekordbox_playlist",
    enable: (ctx) => {
      ctx.state.masterDbEnabled = true;
      ctx.persistMasterDbEnabled(true);
    },
    detect: (ctx) => ctx.detectExternalMasterDb()
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

// [kind, group label, one list of that kind]
const PLAYLIST_IMPORT_KINDS = [
  ["playlist", "playlists", "playlist"],
  ["crate", "crates", "crate"],
  ["history", "history", "history session"]
];

// Pick one playlist / crate / history session from a detected rekordbox or
// Mixxx library and import it as a new local playlist (the backend imports
// its tracks too), then open it.
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
  const groups = sources.flatMap((source, index) => PLAYLIST_IMPORT_KINDS.map(([kind, kindLabel, oneLabel]) => ({
    label: `${source.label} ${kindLabel}`,
    items: (listed[index] || [])
      .filter((item) => item.kind === kind)
      .map((item) => ({
        ...item,
        sourceLabel: source.label,
        description: [
          `${source.label} ${oneLabel}`,
          `${item.trackCount} ${item.trackCount === 1 ? "track" : "tracks"}`,
          // Importing a list again updates the playlist its earlier import made.
          ...(item.existingPlaylist ? [`updates your playlist "${item.existingPlaylist.name}"`] : [])
        ].join(" · ")
      }))
  })));
  if (!groups.some((group) => group.items.length)) {
    // Keep a listing error on the status line rather than hiding it.
    if (!listed.includes(null)) emitStatus("No playlists with tracks found");
    return;
  }

  const chosen = await ctx.playlistImportDialog.open({ groups });
  if (!chosen) return;
  const { source, path } = chosen;

  let result;
  try {
    result = await withProgress(`Importing ${source.label} playlist`, async (progress) => {
      progress(30, `Importing ${chosen.name}...`);
      const imported = await command(source.importCommand, {
        path, kind: chosen.kind, id: chosen.id, force: chosen.force === true
      });
      progress(70, "Refreshing playlists...");
      // The imported tracks belong to that library: show them in the library too.
      source.enable(ctx);
      state.sourcesEverConfigured = true;
      ctx.persistSourcesEverConfigured(true);
      // Re-detect: the library now counts as imported, unlocking its chip.
      await source.detect(ctx);
      await ctx.resetAndLoadLibraryTracks(state.libraryQuery || "");
      await loadPlaylists(ctx);
      state.currentPlaylistId = imported.playlistId;
      updateModeText();
      await switchView(imported.playlistId);
      return imported;
    });
  } catch (err) {
    emitStatus(`${source.label} playlist import failed: ${err?.message || err}`);
    return;
  }

  const logLabel = `${source.label} playlist import`;
  const notFound = Array.isArray(result.notFound) ? result.notFound : [];
  if (notFound.length > 0) {
    logWarnings(
      source.label,
      notFound.map((p) => ({ level: "warn", message: p, code: "playlist_import.file_not_found" })),
      logLabel
    );
  }
  const warnings = Array.isArray(result.warnings) ? result.warnings : [];
  if (warnings.length > 0) {
    logWarnings(source.label, warnings, logLabel);
  }
  const skipped = Math.max(0, Number(chosen.trackCount || 0) - Number(result.added || 0));
  const suffix = skipped > 0 ? ` | ${skipped} track(s) skipped (see event log)` : "";
  const verb = result.updatedExisting ? "Updated" : "Imported";
  emitStatus(`${verb} playlist ${result.name}: ${result.added} track(s)${suffix}`);
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
      state.currentPlaylistId = state.playlists.at(-1)?.id || null;
    }
    updateModeText();
    await switchView(state.currentPlaylistId || "library");
    emitStatus(`Playlist deleted: ${playlist.name}`);
  } finally {
    state.deletingPlaylistId = null;
  }
}

export async function addTracksToCurrentPlaylist(ctx, tracks) {
  const { state, pushEventLog, withProgress, command, promoteTrackIdentity } = ctx;
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
      dedupe: "skip",
      usbRoot: state.usbRoot || null,
      usbRootValid: !!state.usbRootValid
    });
    for (let i = 0; i < (add.resolutions || []).length; i += 1) {
      const resolution = add.resolutions[i] || {};
      const resolvedId = String(resolution.trackId || "").trim();
      if (!resolvedId) continue;
      if (tracks?.[i]) tracks[i].localTrackId = resolvedId;
      const previousId = String(resolution.previousId || "").trim();
      if (previousId && previousId !== resolvedId) {
        promoteTrackIdentity(previousId, resolvedId);
      }
    }
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
      includeMasterDb: state.masterDbEnabled === true,
      includeMixxxDb: state.mixxxDbEnabled === true,
      query: String(state.libraryQuery || "").trim(),
      trackIds: allMatching ? [] : trackIds,
      allMatching,
      dedupe: "skip"
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
export function fillPlaylistSidebarItem(item, playlist) {
  item.querySelector(".nav-playlist-name").textContent = playlist.name;
  const status = item.querySelector(".nav-playlist-status");
  const exported = !!playlist.lastExportedAt;
  status.classList.toggle("exported", exported);
  status.textContent = exported ? "✓" : "";
  if (exported) {
    status.dataset.tooltip = "Exported to USB";
    status.setAttribute("aria-label", "Exported to USB");
  } else {
    delete status.dataset.tooltip;
    status.removeAttribute("aria-label");
  }
  item.querySelector(".nav-playlist-delete").dataset.deletePlaylist = playlist.id;
}

// The playlist's track count is computed by the backend over the whole
// playlist (not just whatever page(s) happen to be loaded client-side) and
// pushed here via playlist.trackCount -- see GetPlaylistTracksData::total.
// This is a pure setter, no track iteration.
export function updatePlaylistPanelTitle(ctx, playlist) {
  const { el } = ctx;
  if (!el?.playlistPanelTitle || !playlist) return;
  const loaded = Array.isArray(playlist.tracks) ? playlist.tracks.length : 0;
  // Prefer the backend's whole-playlist total; fall back to the loaded count
  // only before the first page response has set it.
  const count = Number(playlist.trackCount) > 0 ? Number(playlist.trackCount) : loaded;
  const parts = [playlist.name];
  // The total time is under the table (playlistTotalDuration), not repeated here.
  if (count > 0) {
    parts.push(`(${count} track${count !== 1 ? "s" : ""})`);
  }
  // Backend-derived from where the playlist was imported (see list_playlists).
  if (playlist.importedFrom) parts.push(`· Imported from ${playlist.importedFrom}`);
  el.playlistPanelTitle.textContent = parts.join(" ");
}

export function populatePlaylistPanel(ctx, playlist) {
  const { state, el } = ctx;
  if (!playlist) return;
  updatePlaylistPanelTitle(ctx, playlist);
  renderPlaylistExportStatus(el, playlist);
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
    fetchPage: ({ scopeId, query, sortBy, sortDir, cursor, limit }) =>
      ctx.command("get_playlist_tracks", {
        playlistId: scopeId,
        query,
        sortBy: sortBy || null,
        sortDir: sortDir || null,
        cursor: cursor || null,
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
        withCheckbox: false,
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
    renderTrackTable: ctx.renderTrackTable,
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
      }
      // Reveal the table wrap *before* renderTrackTable paints the rows: a
      // waveform canvas measured while an ancestor is `display:none` (the
      // empty-state chrome still applied from a previously-selected empty
      // playlist) sizes to 1x1 and is never repainted when the wrap is later
      // shown, leaving every waveform blank. renderPlaylistPanelChrome() in
      // onPage still owns the authoritative empty/non-empty state afterwards.
      const hasTracks = Number(data.total) > 0 || (data.items || []).length > 0;
      ctx.el.playlistTableWrap?.classList.toggle("hidden", !hasTracks);
      ctx.el.playlistTotalDuration?.classList.toggle("hidden", !hasTracks);
    },
    onPage: () => renderPlaylistPanelChrome(ctx),
  });
  return ctl;
}

// Panel glue around the playlist track table that the controller does not own:
// empty state, section visibility, search-input restore, is-analyzing pulse,
// title, export buttons, sidebar. Run after every (re)render of the list.
export function renderPlaylistPanelChrome(ctx) {
  const { state, el, playlistTracksCtl } = ctx;
  const playlist = getCurrentPlaylist(ctx);
  if (!playlist) return;
  const empty = playlistTracksCtl.total === 0 && !playlistTracksCtl.loading;
  if (el.playlistEmptyState) {
    el.playlistEmptyState.replaceChildren();
    if (empty) {
      renderEmptyState(el.playlistEmptyState, {
        icon: "♫",
        heading: "Browse Library or USB to add tracks",
      });
    }
  }
  el.playlistTableWrap?.classList.toggle("hidden", empty);
  el.playlistTotalDuration?.classList.toggle("hidden", empty);
  el.playlistSearchInput?.closest(".search-row")?.classList.toggle("hidden", empty);
  el.exportPlaylistBtn?.closest(".playlist-actions")?.classList.toggle("hidden", empty);
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
  await playlistTracksCtl.load({ scopeId: playlist.id });
  renderPlaylistPanelChrome(ctx);
}
