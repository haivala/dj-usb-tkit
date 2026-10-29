import { createTrackListController } from "../shared/track_list_controller.mjs";
import { narrowTrackSelectionToLoaded } from "../shared/track_selection.mjs";
import { renderEmptyState } from "../shell/actions.mjs";
import { cssEscape, cloneTemplate } from "../../ui_utils.mjs";
import { fillBpmCell, fillKeyCell, fillYearCell, coverElement, setCueButtonState, formatTrackDuration, trackHasWaveform } from "../../track_table.mjs";
import { REANALYZE_TOOLTIP } from "../../track_utils.mjs";
import {
  formatDurationMs,
  renderTrackListDurationSummary,
  formatTrackListSummary,
} from "../../track_utils.mjs";
import { drawWaveformCanvas, invalidateWaveformCache, setWaveformColorData } from "../../waveform.mjs";

export const LIBRARY_LOAD_LIMIT_DEFAULT = 200;
const LIBRARY_LOAD_LIMIT_POST_SCAN = 1000;
const LIBRARY_SEARCH_DEBOUNCE_MS = 180;
const PREVIEW_HYDRATION_BATCH_SIZE = 48;

export function warningEntryText(entry) {
  return String(entry?.message ?? entry ?? "").trim();
}

export function trackArtworkChecked(track) {
  return track?.artworkChecked === true;
}

// The backend sends every track fully typed (models.rs `Track` / `UsbTrack`):
// analysis readiness, format compatibility, key display, the 0-100 waveform
// preview. This adds only what the frontend owns: the cover's asset URL
// (Tauri's convertFileSrc), the display defaults for an untitled row, and the
// UI-side flags.
export function normalizeTrack(ctx, track) {
  const rawArtwork = track.artworkDataUrl || track.artworkUrl || track.artworkPath || "";
  return {
    ...track,
    title: track.title || "Unknown Title",
    artist: track.artist || "Unknown Artist",
    // Revision-tagged so an updated cover isn't served from cache.
    artworkUrl: appendUrlRevision(ctx.toPlayableUrl(rawArtwork) || "", track.updatedAt || ""),
    artworkChecked: trackArtworkChecked(track),
    // USB rows: "an inspect_usb_track could still fill display data in"; cleared
    // by hydrateUsbTrackMetadata once it has inspected the row.
    needsHydration: track.needsHydration === true
  };
}

export function normalizeUsbPlaylist(ctx, playlist) {
  return { ...playlist, tracks: playlist.tracks.map((track) => normalizeTrack(ctx, track)) };
}

// A re-fetched row is the backend's word on everything it carries; only what
// is loaded lazily (waveform preview, colour data, artwork) is kept from the
// row it replaces while the new one lacks it.
export function mergeTrackPreservingBestFields(existing, normalized) {
  const merged = { ...existing, ...normalized };
  if ((!Array.isArray(normalized.waveformPreview) || normalized.waveformPreview.length === 0)
    && Array.isArray(existing.waveformPreview)
    && existing.waveformPreview.length > 0) {
    merged.waveformPreview = [...existing.waveformPreview];
  }
  if ((!Array.isArray(normalized.waveformColorData) || normalized.waveformColorData.length === 0)
    && Array.isArray(existing.waveformColorData)
    && existing.waveformColorData.length > 0) {
    merged.waveformColorData = existing.waveformColorData;
  }
  if (!normalized.artworkDataUrl && existing.artworkDataUrl) merged.artworkDataUrl = existing.artworkDataUrl;
  if (!normalized.artworkUrl && existing.artworkUrl) merged.artworkUrl = existing.artworkUrl;
  if (!normalized.artworkPath && existing.artworkPath) merged.artworkPath = existing.artworkPath;
  if (normalized.artworkChecked || existing.artworkChecked) merged.artworkChecked = true;
  if (!normalized.waveformPeaksPath && existing.waveformPeaksPath) merged.waveformPeaksPath = existing.waveformPeaksPath;
  // Once a row has been hydrated (backend flag cleared, or an inspect ran),
  // it stays hydrated across a re-fetch/merge.
  merged.needsHydration = existing.needsHydration === false
    ? false
    : normalized.needsHydration === true;
  return merged;
}

// The library's sources are the backend's (`get_library_sources`): the
// folders with their switch and missing / analyzed state, and the imported
// libraries' switches. Every change goes through a command that answers with
// the new sources; they're stored here, drawn as chips, and the folders are
// let into the asset scope (covers).
export function applyLibrarySources(ctx, sources) {
  ctx.state.librarySources = sources;
  renderSourceChips(ctx);
  return ctx.syncAssetScopePaths().catch(() => {});
}

export async function refreshLibrarySources(ctx) {
  try {
    await applyLibrarySources(ctx, await ctx.command("get_library_sources"));
  } catch (err) {
    console.warn("Failed to load library sources:", err);
  }
}

// "Source folder missing: /a (+2 more). Relocate or remove it."
function missingFoldersStatus(paths) {
  const more = paths.length > 1 ? ` (+${paths.length - 1} more)` : "";
  return `Source folder missing: ${paths[0]}${more}. Relocate or remove it.`;
}

export function trackNeedsPreviewHydration(track) {
  if (!track) return false;
  const missingWaveformPreview = !Array.isArray(track.waveformPreview) || track.waveformPreview.length === 0;
  const missingColorData = !Array.isArray(track.waveformColorData) || track.waveformColorData.length === 0;
  const hasWaveformPath = typeof track.waveformPeaksPath === "string" && track.waveformPeaksPath.trim().length > 0;
  return (missingWaveformPreview && missingColorData) && hasWaveformPath;
}

export function mergeHydratedTrackIntoState(ctx, rawTrack) {
  const { state } = ctx;
  const normalized = normalizeTrack(ctx, rawTrack);
  const trackId = String(normalized.id || "").trim();
  if (!trackId) return false;
  let changed = false;

  for (let i = 0; i < state.tracks.length; i += 1) {
    const existing = state.tracks[i];
    if (String(existing?.id || "") !== trackId) continue;
    state.tracks[i] = mergeTrackPreservingBestFields(existing, normalized);
    changed = true;
    break;
  }

  for (const playlist of state.playlists) {
    for (let i = 0; i < (playlist.tracks || []).length; i += 1) {
      const existing = playlist.tracks[i];
      const localTrackId = String(existing?.localTrackId || existing?.id || "").trim();
      if (localTrackId !== trackId) continue;
      playlist.tracks[i] = mergeTrackPreservingBestFields(existing, normalized);
      changed = true;
    }
  }

  return changed;
}

// A command that changed a track answers with its whole row -- the library's
// `Track`, or a USB list's `UsbTrack` (which carries `usbMediaPath`) -- and
// the tables show it as it is: every loaded copy of that track takes the
// row's contents (kept as the same object, so the lists holding it stay put)
// and its drawn rows are redrawn. Nothing is merged from the old copy.
export function showTrackRow(ctx, rawTrack) {
  if (!rawTrack) return;
  const track = normalizeTrack(ctx, rawTrack);
  const replace = (copy) => {
    for (const key of Object.keys(copy)) delete copy[key];
    Object.assign(copy, track);
  };
  if ("usbMediaPath" in rawTrack) {
    // The same file can sit in several USB playlists and the history.
    const path = String(track.usbMediaPath || "").trim();
    for (const ctl of [ctx.usbPlaylistTracksCtl, ctx.usbHistoryTracksCtl]) {
      const copies = (ctl?.items || []).filter((item) => item && String(item.usbMediaPath || "").trim() === path);
      copies.forEach(replace);
      for (const copy of copies) ctl.patchRow(copy, (body, t) => patchUsbRowsInContainer(ctx, body, t));
    }
    return;
  }
  const { state } = ctx;
  const id = String(track.id);
  for (const copy of state.tracks) {
    if (copy && String(copy.id) === id) replace(copy);
  }
  for (const playlist of state.playlists) {
    for (const copy of playlist.tracks || []) {
      if (copy && String(copy.localTrackId || copy.id || "").trim() === id) replace(copy);
    }
  }
  patchLibraryRowByTrackId(ctx, id);
  patchPlaylistRowByTrackId(ctx, id);
}

// An analysis job event. A track-ready event carries the track's saved row
// (`track`), shown whole; the pieces before it (`trackStarted`, then
// artwork, waveform, length and BPM / key as each is found) fill the row in
// column order while the track is analyzed.
export async function applyRealtimeAnalyzedTrackUpdate(ctx, payload) {
  const { state } = ctx;
  const trackId = String(payload?.trackId || "").trim();
  if (!trackId) return;
  if (payload.track) {
    showTrackRow(ctx, payload.track);
    return;
  }

  let libraryChanged = false;
  for (const track of state.tracks) {
    if (String(track?.id) !== trackId) continue;
    libraryChanged = patchTrackAnalysisFields(ctx, track, payload) || libraryChanged;
  }
  if (libraryChanged) {
    ctx.debugFrontendLog("row-update", {
      trackId,
      bpm: payload?.bpm ?? null,
      key: payload?.key ?? null
    });
    patchLibraryRowByTrackId(ctx, trackId);
  } else {
    // Expected, not a problem: the track isn't on a loaded library page, or
    // the BPM/key piece and the track-ready event carry the same values.
    ctx.debugFrontendLog("row-update-skipped", { trackId });
  }

  for (const playlist of state.playlists) {
    for (const track of playlist.tracks || []) {
      const localTrackId = String(track?.localTrackId || track?.id || "").trim();
      if (localTrackId !== trackId) continue;
      patchTrackAnalysisFields(ctx, track, payload);
    }
  }
  // The open app playlist shows the same library track: redraw its row too,
  // or it keeps the old BPM/key until the playlist is reloaded.
  patchPlaylistRowByTrackId(ctx, trackId);

  const payloadHasPreview = Array.isArray(payload?.waveformPreview) && payload.waveformPreview.length > 0;
  const payloadHasWaveformPath = typeof payload?.waveformPeaksPath === "string"
    && payload.waveformPeaksPath.trim().length > 0;
  if (!payloadHasPreview && payloadHasWaveformPath) {
    hydrateTrackPreviewFromBackend(ctx, trackId).catch(() => {});
  }
}

export async function hydrateTrackPreviewFromBackend(ctx, trackId) {
  const { state } = ctx;
  const id = String(trackId || "").trim();
  if (!id) return;
  if (state.trackPreviewHydrateInFlight.has(id)) return;
  state.trackPreviewHydrateInFlight.add(id);
  try {
    const data = await ctx.command("get_tracks_by_ids_with_previews", { trackIds: [id] });
    let changed = false;
    for (const item of data?.items || []) {
      changed = mergeHydratedTrackIntoState(ctx, item) || changed;
    }
    if (changed) {
      patchLibraryRowByTrackId(ctx, id);
    }
  } finally {
    state.trackPreviewHydrateInFlight.delete(id);
  }
}

export async function hydrateLoadedTracksPreviewsInBackground(ctx) {
  const { state } = ctx;
  const hydrationSeq = ++state.loadedPreviewHydrationSeq;
  const pendingIds = (state.tracks || [])
    .filter((track) => trackNeedsPreviewHydration(track))
    .map((track) => String(track.id || "").trim())
    .filter(Boolean);
  if (!pendingIds.length) return;

  let anyChanged = false;
  for (let i = 0; i < pendingIds.length; i += PREVIEW_HYDRATION_BATCH_SIZE) {
    if (hydrationSeq !== state.loadedPreviewHydrationSeq) return;
    const batch = pendingIds.slice(i, i + PREVIEW_HYDRATION_BATCH_SIZE);
    try {
      const data = await ctx.command("get_tracks_by_ids_with_previews", { trackIds: batch });
      const changedIds = [];
      for (const item of data?.items || []) {
        const changed = mergeHydratedTrackIntoState(ctx, item);
        if (!changed) continue;
        anyChanged = true;
        const id = String(item?.id || "").trim();
        if (id) changedIds.push(id);
      }
      for (const id of changedIds) {
        patchLibraryRowByTrackId(ctx, id);
      }
      await ctx.nextPaint();
    } catch (_) {
      return;
    }
  }

  if (hydrationSeq !== state.loadedPreviewHydrationSeq) return;
  if (anyChanged) {
    renderLibraryRows(ctx);
    ctx.renderCurrentPlaylistTracksFromState();
  }
}

export async function relocateSourceRoot(ctx, oldRoot) {
  const { state } = ctx;
  const { emitStatus } = ctx;
  const sourceRoot = String(oldRoot || "").trim();
  if (!sourceRoot) return;

  emitStatus(`Relocate source folder: ${sourceRoot}`);
  const picked = await ctx.pickSourceFolders();
  const newRoot = Array.isArray(picked)
    ? String(picked[0] || "").trim()
    : String(picked || "").trim();
  if (!newRoot) return;

  const result = await ctx.command("relocate_source_root", {
    oldRoot: sourceRoot,
    newRoot
  });
  await applyLibrarySources(ctx, result.sources);
  await resetAndLoadLibraryTracks(ctx, state.libraryQuery, LIBRARY_LOAD_LIMIT_DEFAULT);
  await ctx.refreshCurrentPlaylistTracks();

  const unresolved = Number(result.missingAtNewRoot || 0) + Number(result.conflicts || 0);
  const updated = Number(result?.updated || 0);
  const partial = unresolved > 0
    ? ` | ${unresolved} track(s) still unresolved`
    : "";
  emitStatus(`Source relocated: ${updated} track path(s) updated${partial}`);
}

// The detected DJ libraries' chips, in their own "Libraries" row. A library's
// checkbox (show / hide its tracks) only works once it has been imported --
// `imported` comes from the backend's detect call; the chip's ↻ imports it.
function renderLibraryChips(ctx) {
  const { state, el, document } = ctx;
  if (!el.libraryChipsContainer) return;
  const libraries = [
    {
      path: state.externalRekordboxDbPath, imported: state.rekordboxDbImported,
      enabled: state.librarySources.rekordboxDbEnabled,
      template: "tplSourceChipRekordboxDb", label: "rekordbox"
    },
    {
      path: state.externalMixxxDbPath, imported: state.mixxxDbImported,
      enabled: state.librarySources.mixxxDbEnabled,
      template: "tplSourceChipMixxxDb", label: "Mixxx"
    }
  ].filter((library) => library.path);
  el.libraryChipsContainer.replaceChildren();
  for (const library of libraries) {
    const chip = cloneTemplate(document, library.template);
    const toggle = chip.querySelector(".source-chip-toggle");
    toggle.disabled = !library.imported;
    toggle.checked = !!library.imported && !!library.enabled;
    chip.querySelector(".source-chip-path").dataset.tooltip = library.imported
      ? library.path
      : `Not imported yet: click ↻ to import from ${library.label}`;
    el.libraryChipsContainer.appendChild(chip);
  }
  el.libraryChipsRow?.classList.toggle("hidden", !libraries.length);
}

export function renderSourceChips(ctx) {
  const { state, el, document } = ctx;
  el.sourceChipsContainer.replaceChildren();
  renderLibraryChips(ctx);

  state.librarySources.folders.forEach(({ path, enabled, missing, fullyAnalyzed }, index) => {
    const chip = cloneTemplate(document, "tplSourceChip");
    chip.classList.toggle("source-chip-analyzed", fullyAnalyzed);
    chip.classList.toggle("source-chip-missing", missing);
    if (missing) {
      chip.dataset.sourceRelocateIndex = String(index);
      chip.dataset.tooltip = "Source folder missing. Click to relocate.";
    }
    const toggle = chip.querySelector(".source-chip-toggle");
    toggle.dataset.sourceToggleIndex = String(index);
    toggle.checked = !missing && enabled;
    toggle.disabled = missing;
    if (missing) toggle.setAttribute("aria-label", "Source folder missing");
    const pathEl = chip.querySelector(".source-chip-path");
    const pathTitle = missing ? `Folder missing. Click to relocate: ${path}` : path;
    pathEl.textContent = path;
    pathEl.dataset.tooltip = pathTitle;
    pathEl.setAttribute("aria-label", pathTitle);
    chip.querySelector(".source-chip-remove").dataset.sourceIndex = String(index);
    el.sourceChipsContainer.appendChild(chip);
  });

  if (el.importPlaylistItem) {
    el.importPlaylistItem.classList.toggle("hidden", !state.externalRekordboxDbPath && !state.externalMixxxDbPath);
  }
  ctx.updateScanLibraryButtonLabel();
  ctx.updateSourceFilterIndicator();
}

// The library's duration total/unknown-count are computed entirely by the
// backend (which knows the true filtered library, not just whatever page
// happens to be loaded client-side) and pushed here: once per fresh
// browse_source_files response (the library TrackListController), and live per
// track via job.progress events during an analysis batch (job_manager.mjs). This is
// a pure setter -- no track iteration, no countability logic, no filtering.
export function applyLibraryDurationSummary(ctx, totalMs, unknownCount) {
  const { state, el } = ctx;
  state.libraryDurationTotalMs = Number(totalMs) || 0;
  state.libraryDurationUnknownCount = Number(unknownCount) || 0;
  renderTrackListDurationSummary(el?.libraryTotalDuration, {
    totalDurationMs: state.libraryDurationTotalMs,
    durationUnknownCount: state.libraryDurationUnknownCount,
  });
}

// The library track table is fetched/paginated/searched/sorted by the shared
// TrackListController (createLibraryTracksController). This renders only the
// chrome the controller does not own: the "add a folder" empty state /
// onboarding mode, and re-applying the transient `is-analyzing` row class
// after a (re)render. The table itself is drawn by the controller.
export function renderLibraryChrome(ctx) {
  const { state, el } = ctx;
  const noSources = !state.librarySources.everConfigured;
  if (el.libraryEmptyState) {
    el.libraryEmptyState.replaceChildren();
    if (noSources) {
      const extraActions = [
        ...(state.externalRekordboxDbPath ? [{ label: "rekordbox library", onAction: () => scanRekordboxDb(ctx) }] : []),
        ...(state.externalMixxxDbPath ? [{ label: "Mixxx library", onAction: () => scanMixxxDb(ctx) }] : []),
      ];
      renderEmptyState(el.libraryEmptyState, {
        icon: "♫",
        heading: "Add a music folder to get started",
        actionLabel: "Add Folder",
        onAction: () => el.addSourceBtn?.click(),
        extraActions
      });
    }
  }
  if (el.libraryContent) {
    el.libraryContent.classList.toggle("hidden", noSources);
  }
  ctx.syncLibraryOnboardingMode();

  for (const id of state.analyzingTrackIds) {
    const selector = `.track-grid-row[data-track-id="${cssEscape(id)}"][data-track-origin="local"]`;
    const row = el.libraryTableBody?.querySelector(selector);
    if (row) row.classList.add("is-analyzing");
  }
}

// The library track table's data layer: server-paginated + searched + sorted
// via `browse_source_files`, rendered through the shared TrackListController.
// `ctl.items` is backed by `state.tracks` (read by playback resolution,
// analysis patching, and selection) so those stay consistent as pages load.
// `ctl.prevById` is rebuilt before every page's normalize() so a lazily
// hydrated waveform preview survives a reload / filter / sort change.
export function createLibraryTracksController(ctx) {
  const ctl = createTrackListController({
    bodyId: "libraryTableBody",
    pageSize: LIBRARY_LOAD_LIMIT_DEFAULT,
    getElements: () => ({
      body: ctx.el.libraryTableBody,
      wrap: ctx.el.libraryTableWrap,
      durationTarget: ctx.el.libraryTotalDuration,
    }),
    // The backend lists the library as its sources are set.
    fetchPage: ({ query, sortBy, sortDir, offset, limit }) => {
      ctx.state.libraryQuery = String(query || "").trim();
      return ctx.command("browse_source_files", {
        query: ctx.state.libraryQuery,
        sortBy: sortBy || null,
        sortDir: sortDir || null,
        offset,
        limit,
      });
    },
    normalize: (track) => {
      const normalized = normalizeTrack(ctx, track);
      const prev = ctl.prevById.get(String(normalized.id));
      return prev ? mergeTrackPreservingBestFields(prev, normalized) : normalized;
    },
    getItems: () => ctx.state.tracks,
    setItems: (value) => { ctx.state.tracks = value; },
    rowOptions: () => ({
      withCheckbox: true,
      selectedIds: ctx.state.selectedTrackIds,
      actionLabel: "+",
      actionType: "add-library",
      compactAddButton: true,
      enableAnalyzeActions: true,
      origin: "local",
      secondaryActionLabel: "Play",
      secondaryActionType: "play-library",
    }),
    buildRow: ctx.buildTrackTableRow,
    renderEmpty: ctx.renderEmptyTrackTable,
    finishRows: ctx.finishTrackRows,
    renderDurationSummary: (_target, summary) =>
      applyLibraryDurationSummary(ctx, summary.totalDurationMs, summary.durationUnknownCount),
    getTableSortState: () => ctx.tableSortState,
    onError: (err) => {
      console.error(err);
      ctx.emitStatus(err.message || String(err));
    },
    onResponse: (data) => {
      // On a re-query (search / sort) ctx.state.tracks still holds the previous list;
      // snapshot it so surviving tracks keep their lazily hydrated previews. On a
      // full reload (source-filter change) ctx.state.tracks is already cleared and
      // resetAndLoadLibraryTracks took the snapshot before clearing.
      const loaded = (ctx.state.tracks || []).filter(Boolean);
      if (loaded.length) {
        ctl.prevById = new Map(loaded.map((t) => [String(t.id), t]));
      }
      // The enabled sources' tracks, ignoring the search.
      if (ctx.el.libraryCountsText) {
        ctx.el.libraryCountsText.textContent = formatTrackListSummary(data.listTrackCount, data.listSizeBytes);
      }
    },
    onPage: (_page, { first }) => {
      if (first) narrowTrackSelectionToLoaded(ctx, "library", ctx.state.tracks || []);
      renderLibraryChrome(ctx);
      void hydrateLoadedTracksPreviewsInBackground(ctx);
    },
  });
  ctl.prevById = new Map();
  return ctl;
}

// Re-render the loaded library rows (no fetch) + chrome -- used after an
// in-place mutation of `state.tracks` (analysis patch, bg preview hydration)
// or a selection change.
export async function renderLibraryRows(ctx) {
  await ctx.libraryTracksCtl.rerender();
  renderLibraryChrome(ctx);
}

// Debounced library search. Goes through the controller's setSearch (a
// re-query of page 1 that does NOT pre-clear state.tracks), so lazily hydrated
// waveform previews on tracks that survive the filter aren't flashed away.
export function scheduleLibrarySearch(ctx) {
  const { state } = ctx;
  clearTimeout(state.librarySearchDebounceTimer);
  state.librarySearchDebounceTimer = setTimeout(() => {
    state.librarySearchDebounceTimer = null;
    Promise.resolve(ctx.libraryTracksCtl.setSearch(ctx.el.librarySearch?.value || ""))
      .then(() => renderLibraryChrome(ctx))
      .catch((err) => {
        console.error(err);
        ctx.emitStatus(err.message || String(err));
      });
  }, LIBRARY_SEARCH_DEBOUNCE_MS);
}

// Fetch the library from page 1 (source-filter / search / post-scan reload).
// `state.libraryQuery` is the persisted search text; a bigger `limit` (post
// scan) is a one-shot override, otherwise the controller's page size is used.
export async function resetAndLoadLibraryTracks(ctx, query = "", limit = LIBRARY_LOAD_LIMIT_DEFAULT) {
  const { state, libraryTracksCtl } = ctx;
  state.libraryQuery = String(query || "").trim();
  libraryTracksCtl.query = state.libraryQuery;
  // Snapshot for waveform-preview preservation before ctl.load() clears state.tracks.
  libraryTracksCtl.prevById = new Map((state.tracks || []).filter(Boolean).map((t) => [String(t.id), t]));
  const opts = Number.isFinite(limit) && limit > LIBRARY_LOAD_LIMIT_DEFAULT ? { limit } : {};
  await libraryTracksCtl.load(opts);
}

// Key labels are rendered by the backend in the user's notation, so a notation
// change re-fetches every loaded list (page 1) instead of relabelling rows here.
export async function reloadTrackListsForKeyNotation(ctx) {
  const reloads = [];
  if (ctx.libraryTracksCtl.items.length) reloads.push(resetAndLoadLibraryTracks(ctx, ctx.state.libraryQuery));
  if (ctx.getCurrentPlaylist()) reloads.push(ctx.refreshCurrentPlaylistTracks());
  for (const ctl of [ctx.usbPlaylistTracksCtl, ctx.usbHistoryTracksCtl]) {
    if (ctl.scopeId) reloads.push(ctl.reload());
  }
  await Promise.all(reloads);
}

// Scans the library's switched-on folders (the backend picks them, and says
// when there are none).
export async function scanLibrary(ctx) {
  const { state } = ctx;
  const { emitStatus } = ctx;
  emitStatus("Scanning library files...");
  const result = await ctx.command("scan_library", { incremental: true });
  if (result.notFound?.length) emitStatus(missingFoldersStatus(result.notFound));
  await refreshLibrarySources(ctx);
  // e.g. the same file found in two folders; the backend words them.
  const scanWarnings = Array.isArray(result?.warnings) ? result.warnings : [];
  if (scanWarnings.length) {
    ctx.logWarnings("library-scan", scanWarnings, "Library scan");
  }

  await resetAndLoadLibraryTracks(ctx, "", LIBRARY_LOAD_LIMIT_POST_SCAN);
  // Backend-owned: `scan_library` reports these over the whole scanned library,
  // not the page the frontend just loaded.
  const scopedTrackCount = Math.max(0, Number(result?.scopedTrackCount || 0));
  const albumCount = Math.max(0, Number(result?.albumCount || 0));
  const unanalyzedCount = Math.max(0, Number(result?.unanalyzedCount || 0));

  emitStatus(
    `Library scan: ${scopedTrackCount} tracks across ${albumCount} albums (local DB rows indexed ${result.indexed}, updated ${result.updated}, removed ${result.removed}). Resolving waveform/BPM/key...`
  );

  const analysis = unanalyzedCount > 0
    ? await analyzeTrackIds(ctx, [], "Scan analysis", { scopeToLibraryFilter: true })
    : { analyzed: 0, incomplete: 0, failed: 0, warnings: [] };
  const analyzed = Number(analysis?.analyzed || 0);
  const failed = Number(analysis?.failed || 0);
  const incompleteSuffix = analysis?.incomplete ? `, incomplete ${analysis.incomplete}` : "";
  const warnings = Array.isArray(analysis?.warnings) ? analysis.warnings : [];
  await ctx.refreshCurrentPlaylistTracks();

  const warningCount = ctx.countWarningsForStatus([...scanWarnings, ...warnings]);
  const warningSuffix = warningCount ? ` (${warningCount} warning(s))` : "";
  const missingCount = state.librarySources.folders.filter((folder) => folder.missing).length;
  const missingSuffix = missingCount ? ` | ${missingCount} source folder(s) missing` : "";
  emitStatus(
    `Scan done: ${scopedTrackCount} tracks / ${albumCount} albums | analyzed ${analyzed}, failed ${failed}${incompleteSuffix}${warningSuffix}${missingSuffix}`,
    { warningCount }
  );
}

// The selection bars' "Reanalyze": explicitly picked tracks are always
// analyzed afresh, replacing what they had (cues are kept). Filling in only
// what's missing is the header's job (Scan Library / Analyze Missing Tracks).
export async function reanalyzeTracks(ctx, trackIds) {
  const ids = (trackIds || []).filter(Boolean);
  if (!ids.length) return;
  await analyzeTrackIds(ctx, ids, "Reanalyze selected", { reanalyze: true });
  await ctx.refreshCurrentPlaylistTracks();
}

export async function scanRekordboxDb(ctx) {
  await importExternalLibrary(ctx, {
    command: "scan_rekordbox_db",
    path: ctx.state.externalRekordboxDbPath,
    label: "Desktop library",
    logSource: "master.db",
    codePrefix: "rekordbox_db",
    library: "rekordbox",
    detect: () => ctx.detectExternalRekordboxDb(),
  });
}

export async function scanMixxxDb(ctx) {
  await importExternalLibrary(ctx, {
    command: "scan_mixxx_db",
    path: ctx.state.externalMixxxDbPath,
    label: "Mixxx library",
    logSource: "mixxx",
    codePrefix: "mixxx_db",
    library: "mixxx",
    detect: () => ctx.detectExternalMixxxDb(),
  });
}

// An imported library's tracks join the library (its chip switched on).
export async function showExternalLibrary(ctx, library) {
  await applyLibrarySources(
    ctx,
    await ctx.command("set_external_library_enabled", { library, enabled: true }),
  );
}

// Shared by the rekordbox master.db and Mixxx imports: run the backend
// import, turn the library's source chip on, and report the outcome.
async function importExternalLibrary(ctx, { command, path, label, logSource, codePrefix, library, detect }) {
  const { logWarnings } = ctx;
  const { emitStatus } = ctx;
  const logLabel = `${label.toLowerCase()} import`;

  emitStatus(`Importing from ${label.toLowerCase()}...`);
  let result;
  try {
    result = await ctx.command(command, { path: path || undefined });
  } catch (err) {
    emitStatus(`${label} import failed: ${err?.message || err}`);
    return;
  }

  // Show the library's tracks alongside the folder sources, and re-detect it:
  // the backend now reports it as imported, which unlocks its chip.
  await showExternalLibrary(ctx, library);
  await detect();

  await resetAndLoadLibraryTracks(ctx, "", LIBRARY_LOAD_LIMIT_POST_SCAN);

  await ctx.refreshCurrentPlaylistTracks();

  const notFound = Array.isArray(result.notFound) ? result.notFound : [];
  if (notFound.length > 0) {
    logWarnings(
      logSource,
      notFound.map((p) => ({ level: "warn", message: p, code: `${codePrefix}.file_not_found` })),
      logLabel
    );
  }

  const scanWarnings = Array.isArray(result.warnings) ? result.warnings : [];
  if (scanWarnings.length > 0) {
    logWarnings(
      logSource,
      scanWarnings.map((entry) => (entry && typeof entry === "object"
        ? entry
        : { level: "info", message: entry, code: `${codePrefix}.scan_diag` })),
      logLabel
    );
  }

  const suffix = notFound.length > 0 ? ` | ${notFound.length} file(s) not found (see event log)` : "";
  emitStatus(`${label} import done: ${result.indexed} new, ${result.updated} updated${suffix}`);
}

export async function analyzeTrackIds(ctx, trackIds, modeLabel = "Analyze", options = {}) {
  const { state } = ctx;
  const { emitStatus } = ctx;
  if (state.analysisEnginePersistPromise) {
    try {
      await state.analysisEnginePersistPromise;
    } catch {
      // If persistence fails, proceed with current backend setting.
    }
  }
  const ids = Array.isArray(trackIds) ? trackIds.filter(Boolean) : [];
  // With `playlistId` / `scopeToLibraryFilter` the backend picks the tracks that
  // still need analysis (over the whole playlist / library filter, not just the
  // page the frontend has loaded), so an empty `ids` list is a valid request.
  const backendScoped = !!(options.playlistId || options.scopeToLibraryFilter);
  if (!ids.length && !backendScoped) return;
  const countLabel = ids.length ? `${ids.length}` : "matching";

  let analyzed = 0;
  // Decoded fine, but analysis left something out (no BPM detected, ...).
  let incomplete = 0;
  let failed = 0;
  let elapsedMs = null;
  const warnings = [];
  let hydratedItems = [];

  emitStatus(`${modeLabel}: preparing ${countLabel} track(s)...`);

  // Per-row "analyzing" state is driven by job:event (job_manager.mjs), which
  // only marks a track analyzing once the backend actually starts working on
  // it -- this reflects the real, memory/CPU-capped worker count instead of
  // marking every requested track as "analyzing" for the whole batch call,
  // which would look identical regardless of how many workers are actually
  // running concurrently.
  try {
    const batch = await ctx.command("analyze_new_tracks", {
      trackIds: ids,
      playlistId: options.playlistId || null,
      scopeToLibraryFilter: !!options.scopeToLibraryFilter,
      // Backend parses + validates the range string (service::analysis).
      bpmRange: String(state.analysisBpmRange || ""),
      analysisEngine: state.analysisEngine,
      query: String(state.libraryQuery || "").trim(),
      reanalyze: !!options.reanalyze
    });
    analyzed = Math.max(0, Number(batch?.analyzed || 0));
    incomplete = Math.max(0, Number(batch?.incomplete || 0));
    failed = Math.max(0, Number(batch?.failed || 0));
    elapsedMs = Number.isFinite(Number(batch?.elapsedMs)) ? Number(batch.elapsedMs) : null;
    const batchWarnings = Array.isArray(batch?.warnings) ? batch.warnings : [];
    warnings.push(...batchWarnings);
    hydratedItems = Array.isArray(batch?.items) ? batch.items : [];
    emitStatus(`${modeLabel}: ${analyzed + incomplete + failed} track(s) processed...`);
  } catch (err) {
    failed = ids.length;
    warnings.push(`batch analysis failed: ${err.message || err}`);
  } finally {
    for (const id of ids) {
      setTrackAnalyzingState(ctx, String(id), false);
    }
  }
  // The batch's tracks as saved (each also came on its track-ready event).
  for (const item of hydratedItems) showTrackRow(ctx, item);

  refreshLibrarySources(ctx);
  await ctx.refreshCurrentPlaylistTracks();
  if (warnings.length) {
    ctx.logWarnings("analysis", warnings, modeLabel);
  }
  const warningCount = ctx.countWarningsForStatus(warnings);
  const warningSuffix = warningCount ? ` | (${warningCount} warning(s))` : "";
  const incompleteSuffix = incomplete ? `, incomplete ${incomplete}` : "";
  emitStatus(`${modeLabel} done: analyzed ${analyzed}, failed ${failed}${incompleteSuffix}${warningSuffix}`, { warningCount });
  const processed = analyzed + incomplete + failed;
  if (processed > 0 && elapsedMs !== null) {
    ctx.pushEventLog({
      level: "info",
      source: "analysis",
      code: "analysis.batch_done",
      message: `${modeLabel}: ${processed} track(s) in ${formatDurationMs(elapsedMs)} (analyzed ${analyzed}, failed ${failed}${incompleteSuffix})`
    });
  }
  return { analyzed, incomplete, failed, warnings };
}

export async function analyzeSingleTrack(ctx, track, modeLabel = null) {
  const { emitStatus } = ctx;
  const localId = await ctx.resolveLocalTrackIdAsync(track);
  if (!localId) {
    emitStatus("Track is not in local library yet. Scan library first, then analyze.");
    return;
  }
  const localTrack = ctx.state.tracks.find((t) => t?.id === localId) || track;
  const label = modeLabel || (localTrack.analysisReady ? "Reanalyze" : "Analyze missing");
  await analyzeTrackIds(ctx, [localId], label);
}


export function patchLibraryRowByTrackId(ctx, trackId) {
  const { state, el } = ctx;
  const id = String(trackId || "").trim();
  if (!id) return false;
  const selector = `.track-grid-row[data-track-id="${cssEscape(id)}"][data-track-origin="local"]`;
  const row = el.libraryTableBody?.querySelector(selector);
  if (!row) return false;
  const track = state.tracks.find((t) => String(t?.id) === id);
  if (!track) return false;
  const patched = patchLibraryRowCells(ctx, row, track);
  const analyzing = state.analyzingTrackIds.has(id);
  row.classList.toggle("is-analyzing", analyzing);
  const analyzeBtn = row.querySelector("[data-action='analyze-track']");
  if (analyzeBtn) analyzeBtn.disabled = analyzing;
  return patched;
}

export function patchPlaylistRowByTrackId(ctx, trackId) {
  const { state, el } = ctx;
  const id = String(trackId || "").trim();
  if (!id) return false;
  const selector = `.track-grid-row[data-track-id="${cssEscape(id)}"][data-track-origin="local"]`;
  const row = el.playlistTracksBody?.querySelector(selector);
  if (!row) return false;
  const playlist = ctx.getCurrentPlaylist();
  const track = (playlist?.tracks || []).find((t) => {
    const localTrackId = String(t?.localTrackId || t?.id || "").trim();
    return localTrackId === id;
  });
  if (!track) return false;
  const patched = patchLibraryRowCells(ctx, row, track);
  const analyzing = state.analyzingTrackIds.has(id);
  row.classList.toggle("is-analyzing", analyzing);
  return patched;
}

// Patch every already-rendered USB row of `track` inside `container`.
export function patchUsbRowsInContainer(ctx, container, track) {
  const trackId = String(track?.id || "").trim();
  if (!trackId) return false;
  const selector = `.track-grid-row[data-track-origin="usb"][data-track-id="${cssEscape(trackId)}"]`;
  const rows = container?.querySelectorAll?.(selector) || [];
  let patched = false;
  rows.forEach((row) => {
    if (patchLibraryRowCells(ctx, row, track)) patched = true;
  });
  return patched;
}

export function setTrackAnalyzingState(ctx, trackId, active) {
  const { state } = ctx;
  const id = String(trackId || "").trim();
  if (!id) return;
  if (active) state.analyzingTrackIds.add(id);
  else state.analyzingTrackIds.delete(id);
  patchLibraryRowByTrackId(ctx, id);
  patchPlaylistRowByTrackId(ctx, id);
}

export function promoteTrackIdentity(ctx, oldId, newId) {
  const { state, el } = ctx;
  const fromId = String(oldId || "").trim();
  const toId = String(newId || "").trim();
  if (!fromId || !toId || fromId === toId) return;

  for (const track of state.tracks) {
    if (String(track?.id || "") !== fromId) continue;
    track.id = toId;
    track.localTrackId = toId;
  }

  if (state.selectedTrackIds.has(fromId)) {
    state.selectedTrackIds.delete(fromId);
    state.selectedTrackIds.add(toId);
  }

  for (const playlist of state.playlists) {
    for (const track of playlist.tracks || []) {
      const localTrackId = String(track?.localTrackId || track?.id || "").trim();
      if (localTrackId !== fromId) continue;
      track.localTrackId = toId;
      if (String(track.id || "") === fromId) {
        track.id = toId;
      }
    }
  }

  const row = el.libraryTableBody?.querySelector(
    `.track-grid-row[data-track-id="${cssEscape(fromId)}"][data-track-origin="local"]`
  );
  if (row) {
    row.dataset.trackId = toId;
    row.querySelectorAll("[data-id]").forEach((node) => {
      if (String(node.dataset.id || "") === fromId) {
        node.dataset.id = toId;
      }
    });
  }
}

// --- analysis_patch.mjs ---

// An analysis piece (job event) on a row while its track is analyzed: a
// field that is absent or null leaves the row's value alone.
export function patchTrackAnalysisFields(ctx, track, payload) {
  if (!track || !payload) return false;
  let changed = false;
  const put = (key, next) => {
    if (track[key] === next) return;
    track[key] = next;
    changed = true;
  };
  const setIfChanged = (key, next) => {
    if (next != null) put(key, next);
  };
  // A reanalysis replaces the BPM / key: clear them as the track starts so the
  // row visibly refills with the fresh values (the track-ready event's row
  // brings the stored ones back if the analysis fails).
  if (payload.trackStarted && payload.reanalysis) {
    put("bpm", null);
    put("bpmAnalyzer", null);
    put("key", null);
    put("keyDisplay", "");
    put("keyColor", null);
  }
  setIfChanged("bpm", payload.bpm);
  setIfChanged("bpmAnalyzer", payload.bpmAnalyzer);
  if (payload.key) {
    put("key", payload.key);
    put("keyDisplay", payload.keyDisplay ?? "");
    put("keyColor", payload.keyColor ?? null);
  }
  setIfChanged("filePath", payload.filePath);
  if (payload.artworkPath) {
    setIfChanged("artworkPath", payload.artworkPath);
    setIfChanged("artworkUrl", appendUrlRevision(ctx.toPlayableUrl(payload.artworkPath) || "", Date.now()));
    setIfChanged("artworkChecked", true);
  }
  setIfChanged("waveformPeaksPath", payload.waveformPeaksPath);
  setIfChanged("durationMs", payload.durationMs);
  const preview = payload.waveformPreview;
  if (preview?.length && String(track.waveformPreview) !== String(preview)) {
    track.waveformPreview = preview;
    changed = true;
  }
  return changed;
}

export function patchLibraryRowCells(ctx, row, track) {
  if (!row || !track) return false;

  const cells = row.querySelectorAll('[role="cell"]');
  if (cells.length < 3) return false;

  const durationTd = row.querySelector(".td-length");
  if (durationTd) {
    durationTd.textContent = formatTrackDuration(track);
  }

  const yearTd = row.querySelector(".td-year");
  if (yearTd) fillYearCell(yearTd, track);

  const bpmTd = row.querySelector(".td-bpm");
  if (bpmTd) fillBpmCell(bpmTd, track);

  const keyTd = row.querySelector(".td-key");
  if (keyTd) fillKeyCell(keyTd, track);

  const coverTd = row.querySelector(".td-cover");
  if (coverTd) {
    const coverCandidates = buildCoverSrcCandidates(ctx, track);
    if (coverCandidates.length) {
      const img = coverTd.querySelector("img.cover-thumb");
      if (img) {
        img.src = coverCandidates[0];
        img.dataset.fallbacks = coverCandidates.slice(1).join("|");
      } else {
        // Swap only the cover: the play button shares the cell (.cover-play).
        coverTd.querySelector(".cover-thumb")?.replaceWith(coverElement(row.ownerDocument, coverCandidates));
        attachCoverFallbackHandlers(coverTd);
      }
    } else {
      coverTd.querySelector("img.cover-thumb")?.replaceWith(coverElement(row.ownerDocument, []));
    }
  }

  const waveformTd = row.querySelector(".td-waveform");
  if (waveformTd) {
    const waveformDiv = waveformTd.querySelector(".waveform");
    if (waveformDiv) {
      const hasRenderableWaveform = trackHasWaveform(track);
      if (hasRenderableWaveform) {
        invalidateWaveformCache(waveformDiv);
        if (track.waveformColorData?.length >= 6) {
          setWaveformColorData(waveformDiv, track.waveformColorData);
          delete waveformDiv.dataset.peaks;
        } else {
          waveformDiv.dataset.peaks = track.waveformPreview.join(",");
        }
        if (!waveformDiv.classList.contains("waveform-canvas")) {
          waveformDiv.classList.add("waveform-canvas");
          waveformDiv.prepend(cloneTemplate(row.ownerDocument, "tplWaveformCanvas"));
        }
        drawWaveformCanvas(waveformDiv);
      } else {
        delete waveformDiv.dataset.peaks;
        setWaveformColorData(waveformDiv, null);
        invalidateWaveformCache(waveformDiv);
        waveformDiv.classList.remove("waveform-canvas");
        const canvas = waveformDiv.querySelector("canvas");
        if (canvas) canvas.remove();
      }
      const cueButton = waveformTd.querySelector(".waveform-detail-btn");
      if (cueButton) {
        const ready = row.dataset.trackOrigin === "local" ? !!track.analysisReady : undefined;
        setCueButtonState(cueButton, { ready, hasWaveform: hasRenderableWaveform });
      }
    }
  }

  const actionTd = row.querySelector(".td-action");
  if (actionTd) {
    const analyzeBtn = actionTd.querySelector("[data-action='analyze-track']");
    if (analyzeBtn && track.analysisReady) {
      analyzeBtn.textContent = "Reanalyze";
      analyzeBtn.dataset.tooltip = REANALYZE_TOOLTIP;
    }
  }

  return true;
}

// --- analysis_settings.mjs ---

export const DEFAULT_ANALYSIS_BPM_RANGE = "70-180";

// Format guard for the persisted analysis-BPM-range setting (a corrupt
// localStorage value shouldn't reach the dropdown). The range string is sent
// as-is to `analyze_new_tracks`, which parses and validates it backend-side
// (service::analysis::resolve_analysis_bpm_range); the dropdown's `<option>`
// list in index.html is the source of truth for the offered presets.
export function normalizeAnalysisBpmRange(range) {
  return /^\d{1,3}\s*-\s*\d{1,3}$/.test(String(range || "").trim())
    ? String(range).trim()
    : DEFAULT_ANALYSIS_BPM_RANGE;
}

export function scanLibraryButtonLabel(folders) {
  return folders.length > 1 ? "Scan Libraries" : "Scan Library";
}

// --- cover_url.mjs ---

export function convertFileSrcLocal(filePath) {
  const normalized = String(filePath || "").replace(/\\/g, "/").trim();
  if (!normalized) return null;
  if (/^(?:asset|tauri|https?|blob|data|file):/i.test(normalized)) return normalized;
  const encoded = normalized.split("/").map(encodeURIComponent).join("/");
  return `asset://localhost${encoded}`;
}

export function appendUrlRevision(url, revision) {
  const normalized = String(url || "").trim();
  if (!normalized) return "";
  if (normalized.startsWith("data:")) return normalized;
  const rev = String(revision || "").trim();
  if (!rev) return normalized;
  const joiner = normalized.includes("?") ? "&" : "?";
  return `${normalized}${joiner}rev=${encodeURIComponent(rev)}`;
}

export function buildCoverSrcCandidates(ctx, track) {
  const toPlayableUrl = ctx?.toPlayableUrl;
  const out = [];
  const seen = new Set();
  const addPathVariants = (value) => {
    const raw = String(value || "").trim();
    if (!raw) return;
    if (/^(?:asset|tauri|https?|blob|data|file):/i.test(raw)) {
      push(raw);
      return;
    }
    if (typeof toPlayableUrl === "function") {
      push(toPlayableUrl(raw));
      const normalizedPath = raw.replace(/^file:\/\//i, "");
      if (normalizedPath && normalizedPath !== raw) {
        push(toPlayableUrl(normalizedPath));
      }
      if (raw.startsWith("/")) {
        push(toPlayableUrl(raw.replace(/^\/+/, "")));
      } else {
        push(toPlayableUrl(`/${raw}`));
      }
    }
    push(convertFileSrcLocal(raw));
  };
  const push = (value) => {
    const v = String(value || "").trim();
    if (!v || seen.has(v)) return;
    seen.add(v);
    out.push(v);
  };

  push(track?.artworkDataUrl);
  addPathVariants(track?.artworkPath);
  push(track?.artworkUrl);
  return out;
}

// --- cover_fallback.mjs ---

export function attachCoverFallbackHandlers(root) {
  root.querySelectorAll("img.cover-thumb").forEach((img) => {
    if (img.dataset.fallbackBound === "1") return;
    img.dataset.fallbackBound = "1";
    img.addEventListener("error", () => {
      const queue = String(img.dataset.fallbacks || "")
        .split("|")
        .filter(Boolean);
      const next = queue.shift();
      if (next) {
        img.dataset.fallbacks = queue.join("|");
        img.src = next;
        return;
      }
      img.replaceWith(coverElement(img.ownerDocument, []));
    });
  });
}
