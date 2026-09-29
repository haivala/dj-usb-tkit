import { resolveEmitStatus } from "../shared/track_actions.mjs";
import { createTrackListController } from "../shared/track_list_controller.mjs";
import { renderEmptyState } from "../shell/actions.mjs";
import { cssEscape, cloneTemplate } from "../../ui_utils.mjs";
import { fillBpmCell, fillKeyCell, coverElement } from "../../track_table.mjs";
import {
  formatDurationMs,
  renderTrackListDurationSummary,
  normalizeDurationMs,
  loadMoreIfNearBottom,
} from "../../track_utils.mjs";
import { drawWaveformCanvas, invalidateWaveformCache, setWaveformColorData } from "../../waveform.mjs";

export const LIBRARY_LOAD_LIMIT_DEFAULT = 200;
const LIBRARY_LOAD_LIMIT_POST_SCAN = 1000;
const LIBRARY_SEARCH_DEBOUNCE_MS = 180;
const LIBRARY_SCROLL_FETCH_THRESHOLD_PX = 120;
const PREVIEW_HYDRATION_BATCH_SIZE = 48;

export function warningEntryText(entry) {
  return String(entry?.message ?? entry ?? "").trim();
}

// The backend tags this specific warning with a stable code (analysis.rs
// `analysis.auto-select-limit`); match on that, not the English message.
function findAnalysisAutoLimitWarning(warnings) {
  if (!Array.isArray(warnings)) return null;
  const hit = warnings.find((w) => w?.code === "analysis.auto-select-limit");
  return hit ? warningEntryText(hit) : null;
}

export function trackArtworkChecked(track) {
  return track?.artworkChecked === true;
}

// Whether a track still needs core analysis (`analysisReady`) or a deeper USB
// metadata fetch (`needsHydration`, on USB rows) is owned entirely by the
// backend -- see `has_core_analysis_fields` / `hydrate_usb_track_in_place`.
// The frontend reads the fields directly and never recomputes them.

function clampWaveformPreview(value) {
  if (!Array.isArray(value)) return [];
  return value.map((v) => Math.max(0, Math.min(100, Number(v) || 0)));
}

function toFiniteOrNull(value) {
  const n = Number(value);
  return Number.isFinite(n) ? n : null;
}

const FORMAT_COMPAT_SEVERITIES = new Set(["ok", "autofix", "warn"]);

// Coerce the backend `formatCompat` object into a stable `{ severity, warning }`
// shape. The rule itself lives in Rust (service::format_compat) -- this only
// guards against a missing/legacy field.
function normalizeFormatCompat(value) {
  const severityRaw = String(value?.severity || "ok").toLowerCase();
  const severity = FORMAT_COMPAT_SEVERITIES.has(severityRaw) ? severityRaw : "ok";
  const warning = typeof value?.warning === "string" && value.warning.trim()
    ? value.warning
    : null;
  return { severity, warning };
}

const randomId = () => Math.random().toString(36).slice(2, 9);

export function normalizeTrack(ctx, track, fallbackIdPrefix = "t") {
  const rawArtwork = track?.artworkDataUrl || track?.artworkUrl || track?.artworkPath || "";
  const artworkRevision = track?.updatedAt || "";
  const convertedArtwork = appendUrlRevision(ctx.toPlayableUrl(rawArtwork) || "", artworkRevision);
  const filePath = String(track?.filePath || "").trim();
  const waveformPreview = clampWaveformPreview(track?.waveformPreview);
  const title = track?.title || "Unknown Title";
  const artist = track?.artist || "Unknown Artist";
  const album = track?.album || "";
  const durationMs = normalizeDurationMs(track);

  return {
    id: track?.id || `${fallbackIdPrefix}-${randomId()}`,
    localTrackId: track?.localTrackId || null,
    title,
    artist,
    album,
    trackNumber: toFiniteOrNull(track?.trackNumber),
    bpm: toFiniteOrNull(track?.bpm),
    bpmAnalyzer: track?.bpmAnalyzer || "",
    key: track?.key || "",
    // Backend-owned: `key` in the user's notation (Classic/Camelot) and the
    // key pill's colour group 0..11 (service::key_notation).
    keyDisplay: track?.keyDisplay || "",
    keyColor: Number.isInteger(track?.keyColor) ? track.keyColor : null,
    artworkUrl: convertedArtwork,
    artworkDataUrl: track?.artworkDataUrl || "",
    artworkPath: track?.artworkPath || "",
    artworkChecked: trackArtworkChecked(track),
    filePath,
    durationMs,
    waveformPeaksPath: track?.waveformPeaksPath || "",
    usbAnalysisPath: track?.usbAnalysisPath || "",
    // USB rows only: the un-resolved on-device ANLZ / media paths the cue
    // editor needs to open + save straight onto the stick.
    usbAnalysisPathRaw: track?.usbAnalysisPathRaw || "",
    usbMediaPath: track?.usbMediaPath || "",
    // Backend-owned on every track-returning command (derived from the file
    // path server-side when the DB column / PDB row omits it).
    formatExt: track?.formatExt || "",
    sampleRateHz: toFiniteOrNull(track?.sampleRateHz),
    bitDepth: toFiniteOrNull(track?.bitDepth),
    bitrateKbps: toFiniteOrNull(track?.bitrateKbps),
    // Backend-owned CDJ format-compatibility verdict (service::format_compat).
    // `{ severity: "ok" | "autofix" | "warn", warning: string | null }`.
    formatCompat: normalizeFormatCompat(track?.formatCompat),
    waveformPreview,
    waveformColorData: Array.isArray(track?.waveformColorData) ? track.waveformColorData : null,
    createdAt: track?.createdAt || "",
    updatedAt: track?.updatedAt || "",
    masterDbSource: !!track?.masterDbSource,
    isUsbPath: !!track?.isUsbPath,
    analysisReady: !!track?.analysisReady,
    // USB rows only: backend flag for "still missing display data an
    // inspect_usb_track could fill" (service::usb::hydrate_usb_track_in_place).
    // Cleared to false by hydrateUsbTrackMetadata after it inspects the row.
    needsHydration: track?.needsHydration === true
  };
}

export function normalizeUsbPlaylist(ctx, playlist) {
  const rawTracks = Array.isArray(playlist?.tracks)
    ? playlist.tracks
    : Array.isArray(playlist?.items)
      ? playlist.items
      : [];
  const tracks = rawTracks.map((track) => normalizeTrack(ctx, track, "usb"));
  const declared = Number(playlist?.trackCount ?? 0);
  return {
    ...playlist,
    source: String(playlist?.source || "unknown"),
    tracks,
    trackCount: Math.max(Number.isFinite(declared) ? declared : 0, tracks.length)
  };
}
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
  if (!normalized.bpm && existing.bpm) merged.bpm = existing.bpm;
  if (!normalized.key && existing.key) {
    merged.key = existing.key;
    merged.keyDisplay = existing.keyDisplay;
    merged.keyColor = existing.keyColor;
  }
  // Backend-owned readiness only ever moves false -> true; a partial merge that
  // lacks the flag must not drop a previously-analyzed row back to "needs analysis".
  merged.analysisReady = !!(existing.analysisReady || normalized.analysisReady);
  // Once a row has been hydrated (backend flag cleared, or an inspect ran),
  // it stays hydrated across a re-fetch/merge.
  merged.needsHydration = existing.needsHydration === false
    ? false
    : normalized.needsHydration === true;
  return merged;
}

export function applySourceRootAnalysisFromBrowseData(state, data) {
  const rows = Array.isArray(data?.sourceRootAnalysis) ? data.sourceRootAnalysis : [];
  if (!rows.length) return;
  if (!state.sourceRootAnalysisStatus || typeof state.sourceRootAnalysisStatus !== "object") {
    state.sourceRootAnalysisStatus = {};
  }
  for (const row of rows) {
    const root = String(row?.sourceRoot ?? "").trim();
    if (!root) continue;
    state.sourceRootAnalysisStatus[root] = !!row?.fullyAnalyzed;
  }
}

export async function refreshSourceRootAnalysisStatus(ctx) {
  const { state } = ctx;
  // Analysis status is a property of the folder itself, not of whether it's
  // currently toggled on in the library filter, so this intentionally
  // queries every configured root (enabled or not) rather than reusing the
  // enabled-only filter the visible library's browse_source_files query uses.
  // Missing roots are skipped since there's nothing on
  // disk to scan.
  const roots = (state.sourceRoots || []).filter((root) => !sourceRootIsMissing(state, root));
  if (!roots.length) return;
  try {
    const data = await ctx.command("browse_source_files", {
      sourceRoots: roots,
      includeMasterDb: false,
      query: "",
      limit: 1,
      cursor: null
    });
    applySourceRootAnalysisFromBrowseData(state, data);
    renderSourceChips(ctx);
  } catch (err) {
    console.warn("Failed to refresh source folder analysis status:", err);
  }
}

export function missingSourceRootsArray(state) {
  const roots = state?.missingSourceRoots;
  if (roots instanceof Set) return Array.from(roots);
  if (Array.isArray(roots)) return roots;
  return [];
}

export function setMissingSourceRoots(state, roots) {
  const normalized = Array.isArray(roots)
    ? roots.map((root) => String(root || "").trim()).filter(Boolean)
    : [];
  state.missingSourceRoots = new Set(normalized);
  return normalized;
}

export function sourceRootIsMissing(state, root) {
  const key = normalizePath(root).replace(/\/+$/, "");
  if (!key) return false;
  return missingSourceRootsArray(state)
    .some((candidate) => normalizePath(candidate).replace(/\/+$/, "") === key);
}

export async function refreshMissingSourceRoots(ctx, { silent = true } = {}) {
  const { state } = ctx;
  const roots = Array.isArray(state.sourceRoots) ? state.sourceRoots : [];
  if (!roots.length) {
    setMissingSourceRoots(state, []);
    renderSourceChips(ctx);
    return [];
  }
  try {
    const data = await ctx.command("check_source_roots", { sourceRoots: roots });
    const missing = setMissingSourceRoots(state, Array.isArray(data?.missing) ? data.missing : []);
    renderSourceChips(ctx);
    if (!silent && missing.length) {
      ctx.emitStatus(`Source folder missing: ${missing[0]}${missing.length > 1 ? ` (+${missing.length - 1} more)` : ""}. Relocate or remove it.`);
    }
    return missing;
  } catch (err) {
    console.warn("Failed to check source folders:", err);
    return missingSourceRootsArray(state);
  }
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
  const normalized = normalizeTrack(ctx, rawTrack, "lib");
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

export async function applyRealtimeAnalyzedTrackUpdate(ctx, payload) {
  const { state } = ctx;
  const trackId = String(payload?.trackId || "").trim();
  if (!trackId) return;

  let libraryChanged = false;
  let patchedTrack = null;
  for (const track of state.tracks) {
    if (String(track.id) !== trackId) continue;
    libraryChanged = patchTrackAnalysisFields(ctx, track, payload) || libraryChanged;
    patchedTrack = track;
  }
  if (libraryChanged) {
    ctx.debugFrontendLog("row-update", {
      trackId,
      bpm: payload?.bpm ?? null,
      key: payload?.key ?? null
    });
    const label = patchedTrack
      ? [patchedTrack.artist, patchedTrack.title].filter(Boolean).join(" - ") || trackId
      : trackId;
    ctx.log("[analysis-ui] patched state for", label, "bpm:", payload?.bpm, "key:", payload?.key);
    patchLibraryRowByTrackId(ctx, trackId);
  } else {
    const bpm = Number(payload?.bpm);
    const hasBpm = payload?.bpm !== undefined
      && payload?.bpm !== null
      && Number.isFinite(bpm)
      && bpm > 0;
    const hasKey = typeof payload?.key === "string" && payload.key.trim();
    const hasAnalyzer = typeof payload?.bpmAnalyzer === "string" && payload.bpmAnalyzer.trim();
    const isBpmKeyPayload = hasBpm || hasKey || hasAnalyzer;
    if (isBpmKeyPayload) {
      const foundTrack = state.tracks.find((t) => String(t.id) === trackId);
      const warnLabel = foundTrack
        ? [foundTrack.artist, foundTrack.title].filter(Boolean).join(" - ") || trackId
        : trackId;
      ctx.warn("[analysis-ui] no state change for", warnLabel);
    }
  }

  for (const playlist of state.playlists) {
    for (const track of playlist.tracks || []) {
      const localTrackId = String(track.localTrackId || track.id || "").trim();
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
    renderSourceChips(ctx);
  }
}

export async function relocateSourceRoot(ctx, oldRoot) {
  const { state } = ctx;
  const emitStatus = resolveEmitStatus(ctx);
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

  const oldIndex = state.sourceRoots.findIndex((root) => normalizePath(root) === normalizePath(sourceRoot));
  const alreadyHasNewRoot = state.sourceRoots.some((root) => normalizePath(root) === normalizePath(newRoot));
  const unresolved = Number(result?.missingAtNewRoot || 0) + Number(result?.conflicts || 0);
  if (!state.sourceRootEnabled || typeof state.sourceRootEnabled !== "object") {
    state.sourceRootEnabled = {};
  }
  const oldEnabled = state.sourceRootEnabled?.[sourceRoot];
  if (oldIndex >= 0 && unresolved === 0) {
    if (alreadyHasNewRoot) {
      state.sourceRoots.splice(oldIndex, 1);
    } else {
      state.sourceRoots[oldIndex] = newRoot;
    }
  } else if (!alreadyHasNewRoot) {
    state.sourceRoots.push(newRoot);
  }
  if (oldEnabled !== undefined && state.sourceRootEnabled?.[newRoot] === undefined) {
    state.sourceRootEnabled[newRoot] = oldEnabled;
  } else if (state.sourceRootEnabled?.[newRoot] === undefined) {
    state.sourceRootEnabled[newRoot] = true;
  }
  if (unresolved === 0) {
    delete state.sourceRootEnabled[sourceRoot];
  }
  ctx.persistSourceRoots(state.sourceRoots);
  ctx.persistSourceRootEnabled(state.sourceRootEnabled);
  renderSourceChips(ctx);
  await ctx.syncAssetScopePaths();
  await refreshMissingSourceRoots(ctx, { silent: true });
  await resetAndLoadLibraryTracks(ctx, state.libraryQuery, LIBRARY_LOAD_LIMIT_DEFAULT);
  await ctx.refreshCurrentPlaylistTracks();

  const updated = Number(result?.updated || 0);
  const partial = unresolved > 0
    ? ` | ${unresolved} track(s) still unresolved`
    : "";
  emitStatus(`Source relocated: ${updated} track path(s) updated${partial}`);
}

export function renderSourceChips(ctx) {
  const { state, el, document } = ctx;
  el.sourceChipsContainer.replaceChildren();
  if (!state.sourceRootAnalysisStatus || typeof state.sourceRootAnalysisStatus !== "object") {
    state.sourceRootAnalysisStatus = {};
  }
  const activeRoots = new Set(state.sourceRoots || []);
  for (const cachedRoot of Object.keys(state.sourceRootAnalysisStatus)) {
    if (!activeRoots.has(cachedRoot)) delete state.sourceRootAnalysisStatus[cachedRoot];
  }
  const activeRootKeys = new Set((state.sourceRoots || []).map((root) => normalizePath(root).replace(/\/+$/, "")));
  const retainedMissingRoots = missingSourceRootsArray(state)
    .filter((root) => activeRootKeys.has(normalizePath(root).replace(/\/+$/, "")));
  if (retainedMissingRoots.length !== missingSourceRootsArray(state).length) {
    setMissingSourceRoots(state, retainedMissingRoots);
  }
  // master.db chip - shown when detected, positioned before filesystem chips
  if (state.externalMasterDbPath) {
    const chip = cloneTemplate(document, "tplSourceChipMasterDb");
    chip.querySelector(".source-chip-toggle").checked = !!state.masterDbEnabled;
    el.sourceChipsContainer.appendChild(chip);
  }

  state.sourceRoots.forEach((path, index) => {
    if (state.sourceRootEnabled[path] === undefined) {
      state.sourceRootEnabled[path] = true;
    }
    const missing = sourceRootIsMissing(state, path);
    // Per-root "fully analyzed" is computed by the backend over the complete,
    // unfiltered track set and delivered as `sourceRootAnalysis` (see
    // applySourceRootAnalysisFromBrowseData). Just render what it told us.
    const fullyAnalyzed = state.sourceRootAnalysisStatus[path] === true;

    const chip = cloneTemplate(document, "tplSourceChip");
    chip.classList.toggle("source-chip-analyzed", fullyAnalyzed && !missing);
    chip.classList.toggle("source-chip-missing", missing);
    if (missing) {
      chip.dataset.sourceRelocateIndex = String(index);
      chip.dataset.tooltip = "Source folder missing. Click to relocate.";
    }
    const toggle = chip.querySelector(".source-chip-toggle");
    toggle.dataset.sourceToggleIndex = String(index);
    toggle.checked = !missing && state.sourceRootEnabled[path] !== false;
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

  ctx.persistSourceRootEnabled(state.sourceRootEnabled);
  if (el.importMasterDbBtn) {
    el.importMasterDbBtn.classList.toggle("hidden", !state.externalMasterDbPath);
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
  state.libraryDurationUnknownCount = Math.max(0, Number(unknownCount) || 0);
  renderTrackListDurationSummary(
    el?.libraryTotalDuration,
    { totalDurationMs: state.libraryDurationTotalMs, unknownCount: state.libraryDurationUnknownCount },
    formatDurationMs
  );
}

// The library track table is fetched/paginated/searched/sorted by the shared
// TrackListController (createLibraryTracksController). This renders only the
// chrome the controller does not own: the "add a folder" empty state /
// onboarding mode, and re-applying the transient `is-analyzing` row class
// after a (re)render. The table itself is drawn by the controller.
export function renderLibraryChrome(ctx) {
  const { state, el } = ctx;
  const noSources = !state.sourcesEverConfigured;
  if (el.libraryEmptyState) {
    el.libraryEmptyState.replaceChildren();
    if (noSources) {
      const extraActions = state.externalMasterDbPath
        ? [{ label: "RB master.db", onAction: () => scanMasterDb(ctx) }]
        : [];
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
    fetchPage: ({ query, sortBy, sortDir, cursor, limit }) => {
      const enabledRoots = (ctx.state.sourceRoots || []).filter(
        (root) => ctx.state.sourceRootEnabled?.[root] !== false && !sourceRootIsMissing(ctx.state, root),
      );
      const includeMasterDb = ctx.state.masterDbEnabled === true;
      ctx.state.libraryQuery = String(query || "").trim();
      if (!enabledRoots.length && !includeMasterDb) {
        return { total: 0, items: [], nextCursor: null, hasMore: false, totalDurationMs: 0, durationKnownCount: 0 };
      }
      return ctx.command("browse_source_files", {
        sourceRoots: enabledRoots,
        includeMasterDb,
        query: ctx.state.libraryQuery,
        sortBy: sortBy || null,
        sortDir: sortDir || null,
        cursor: cursor || null,
        limit,
      });
    },
    normalize: (track) => {
      const normalized = normalizeTrack(ctx, track, "lib");
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
    renderTrackTable: ctx.renderTrackTable,
    renderDurationSummary: (_target, summary) =>
      applyLibraryDurationSummary(
        ctx,
        summary.totalDurationMs,
        Number(summary.trackCount || 0) - Number(summary.durationKnownCount || 0),
      ),
    getTableSortState: () => ctx.tableSortState,
    onResponse: (data) => {
      // On a re-query (search / sort) ctx.state.tracks still holds the previous list;
      // snapshot it so surviving tracks keep their lazily hydrated previews. On a
      // full reload (source-filter change) ctx.state.tracks is already cleared and
      // resetAndLoadLibraryTracks took the snapshot before clearing.
      if ((ctx.state.tracks || []).length) {
        ctl.prevById = new Map(ctx.state.tracks.map((t) => [String(t.id), t]));
      }
      applySourceRootAnalysisFromBrowseData(ctx.state, data);
      renderSourceChips(ctx);
    },
    onPage: (_page, { first }) => {
      if (first) {
        // On a fresh query/sort (not a scroll-append), narrow the selection to
        // what the new result set shows -- filtering the library is the user's
        // way of scoping "add selected" / "analyze selected". Backend "Select
        // all" sets `selectedTrackIds` directly and only re-renders (no `first`
        // page load), so its larger-than-one-page selection is preserved until
        // the query actually changes.
        const loadedIds = new Set((ctx.state.tracks || []).map((t) => t.id));
        ctx.state.selectedTrackIds = new Set(
          [...ctx.state.selectedTrackIds].filter((id) => loadedIds.has(id)),
        );
        ctx.updateSelectionCount();
      }
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
let librarySearchDebounceTimer = null;
export function scheduleLibrarySearch(ctx) {
  if (librarySearchDebounceTimer) clearTimeout(librarySearchDebounceTimer);
  librarySearchDebounceTimer = setTimeout(() => {
    librarySearchDebounceTimer = null;
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
  libraryTracksCtl.prevById = new Map((state.tracks || []).map((t) => [String(t.id), t]));
  const opts = Number.isFinite(limit) && limit > LIBRARY_LOAD_LIMIT_DEFAULT ? { limit } : {};
  await libraryTracksCtl.load(opts);
}

export function handleLibraryTableWrapScroll(ctx) {
  const ctl = ctx.libraryTracksCtl;
  loadMoreIfNearBottom(
    ctx.el.libraryTableWrap,
    LIBRARY_SCROLL_FETCH_THRESHOLD_PX,
    () => ctl.loading,
    () => ctl.hasMore,
    () => ctl.loadMore().catch((err) => {
      console.error(err);
      ctx.emitStatus(err.message || String(err));
    }),
  );
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

export function enabledLibrarySourceRoots(ctx) {
  const { state } = ctx;
  return enabledSourceRoots(state.sourceRoots, state.sourceRootEnabled, state.missingSourceRoots);
}

export async function scanLibrary(ctx) {
  const { state } = ctx;
  const emitStatus = resolveEmitStatus(ctx);
  if (!state.sourceRoots.length) {
    emitStatus("Set at least one source root path before scanning");
    return;
  }

  ctx.persistSourceRoots(state.sourceRoots);
  const knownMissing = missingSourceRootsArray(state);
  if (knownMissing.length) {
    emitStatus(`Source folder missing: ${knownMissing[0]}${knownMissing.length > 1 ? ` (+${knownMissing.length - 1} more)` : ""}. Relocate or remove it.`);
  }
  const activeScanRoots = enabledLibrarySourceRoots(ctx);
  if (!activeScanRoots.length) {
    emitStatus(knownMissing.length
      ? "No available source folders to scan. Relocate or remove missing source folders."
      : "Enable at least one source folder before scanning");
    return;
  }
  emitStatus("Scanning library files...");
  const result = await ctx.command("scan_library", {
    sourceRoots: activeScanRoots,
    incremental: true
  });
  if (Array.isArray(result?.notFound) && result.notFound.length) {
    setMissingSourceRoots(state, [...missingSourceRootsArray(state), ...result.notFound]);
    renderSourceChips(ctx);
    emitStatus(`Source folder missing: ${result.notFound[0]}${result.notFound.length > 1 ? ` (+${result.notFound.length - 1} more)` : ""}. Relocate or remove it.`);
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
    : { analyzed: 0, failed: 0, warnings: [] };
  const analyzed = Number(analysis?.analyzed || 0);
  const failed = Number(analysis?.failed || 0);
  const warnings = Array.isArray(analysis?.warnings) ? analysis.warnings : [];
  await ctx.refreshCurrentPlaylistTracks();

  const warningCount = ctx.countWarningsForStatus(warnings);
  const warningSuffix = warningCount ? ` (${warningCount} warning(s))` : "";
  const autoLimitWarning = findAnalysisAutoLimitWarning(warnings);
  const autoLimitSuffix = autoLimitWarning ? ` | ${autoLimitWarning}` : "";
  const missingCount = missingSourceRootsArray(state).length;
  const missingSuffix = missingCount ? ` | ${missingCount} source folder(s) missing` : "";
  emitStatus(
    `Scan done: ${scopedTrackCount} tracks / ${albumCount} albums | analyzed ${analyzed}, failed ${failed}${warningSuffix}${autoLimitSuffix}${missingSuffix}`,
    { warningCount }
  );
}

export async function analyzeSelectedTracks(ctx) {
  const emitStatus = resolveEmitStatus(ctx);
  const trackIds = Array.from(ctx.state.selectedTrackIds || []).filter(Boolean);
  if (!trackIds.length) {
    emitStatus("Select at least one track to analyze");
    return;
  }
  await analyzeTrackIds(ctx, trackIds, "Analyze selected");
  await ctx.refreshCurrentPlaylistTracks();
}

export async function scanMasterDb(ctx) {
  const { state, logWarnings } = ctx;
  const emitStatus = resolveEmitStatus(ctx);
  const path = state.externalMasterDbPath || undefined;

  emitStatus("Importing from desktop library...");
  let result;
  try {
    result = await ctx.command("scan_master_db", { path });
  } catch (err) {
    emitStatus(`Desktop library import failed: ${err?.message || err}`);
    return;
  }

  // Mark master.db as enabled and configured so the library's browse query
  // includes it alongside the folder sources.
  state.masterDbEnabled = true;
  state.sourcesEverConfigured = true;
  ctx.persistMasterDbEnabled(true);
  ctx.persistSourcesEverConfigured(true);
  renderSourceChips(ctx);

  await resetAndLoadLibraryTracks(ctx, "", LIBRARY_LOAD_LIMIT_POST_SCAN);

  await ctx.refreshCurrentPlaylistTracks();

  const notFound = Array.isArray(result.notFound) ? result.notFound : [];
  if (notFound.length > 0) {
    logWarnings(
      "master.db",
      notFound.map((p) => ({ level: "warn", message: p, code: "master_db.file_not_found" })),
      "desktop library import"
    );
  }

  const scanWarnings = Array.isArray(result.warnings) ? result.warnings : [];
  if (scanWarnings.length > 0) {
    logWarnings(
      "master.db",
      scanWarnings.map((entry) => (entry && typeof entry === "object"
        ? entry
        : { level: "info", message: entry, code: "master_db.scan_diag" })),
      "desktop library import"
    );
  }

  const suffix = notFound.length > 0 ? ` | ${notFound.length} file(s) not found (see event log)` : "";
  emitStatus(`Desktop library import done: ${result.indexed} new, ${result.updated} updated${suffix}`);
}

export async function analyzeTrackIds(ctx, trackIds, modeLabel = "Analyze", options = {}) {
  const { state } = ctx;
  const emitStatus = resolveEmitStatus(ctx);
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
  let failed = 0;
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
      sourceRoots: (state.sourceRoots || []).filter(
        (root) => state.sourceRootEnabled?.[root] !== false && !sourceRootIsMissing(state, root)
      ),
      includeMasterDb: state.masterDbEnabled === true,
      query: String(state.libraryQuery || "").trim()
    });
    analyzed = Math.max(0, Number(batch?.analyzed || 0));
    failed = Math.max(0, Number(batch?.failed || 0));
    const batchWarnings = Array.isArray(batch?.warnings) ? batch.warnings : [];
    warnings.push(...batchWarnings);
    hydratedItems = Array.isArray(batch?.items) ? batch.items : [];
    emitStatus(`${modeLabel}: ${analyzed + failed} track(s) processed...`);
  } catch (err) {
    failed = ids.length;
    warnings.push(`batch analysis failed: ${err.message || err}`);
  } finally {
    for (const id of ids) {
      setTrackAnalyzingState(ctx, String(id), false);
    }
  }
  const changedIds = [];
  for (const item of hydratedItems) {
    const changed = mergeHydratedTrackIntoState(ctx, item);
    if (changed) {
      const id = String(item?.id || "").trim();
      if (id) changedIds.push(id);
    }
  }
  if (changedIds.length) {
    if (ids.length === 1) {
      await ctx.nextPaint();
      await ctx.nextPaint();
    }
    for (const id of changedIds) {
      patchLibraryRowByTrackId(ctx, id);
      patchPlaylistRowByTrackId(ctx, id);
    }
    renderLibraryRows(ctx);
    renderSourceChips(ctx);
  }

  Promise.resolve(refreshSourceRootAnalysisStatus(ctx)).catch(() => {});
  await ctx.refreshCurrentPlaylistTracks();
  if (warnings.length) {
    ctx.logWarnings("analysis", warnings, modeLabel);
  }
  const warningCount = ctx.countWarningsForStatus(warnings);
  const warningSuffix = warningCount ? ` | (${warningCount} warning(s))` : "";
  const autoLimitWarning = findAnalysisAutoLimitWarning(warnings);
  const autoLimitSuffix = autoLimitWarning ? ` | ${autoLimitWarning}` : "";
  emitStatus(`${modeLabel} done: analyzed ${analyzed}, failed ${failed}${warningSuffix}${autoLimitSuffix}`, { warningCount });
  return { analyzed, failed, warnings };
}

export async function analyzeSingleTrack(ctx, track, modeLabel = null) {
  const emitStatus = resolveEmitStatus(ctx);
  const localId = await ctx.resolveLocalTrackIdAsync(track);
  if (!localId) {
    emitStatus("Track is not in local library yet. Scan library first, then analyze.");
    return;
  }
  const localTrack = ctx.state.tracks.find((t) => t.id === localId) || track;
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
  const track = state.tracks.find((t) => String(t.id) === id);
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

export function parseProgressWaveformPreview(value) {
  if (!Array.isArray(value)) return null;
  return value
    .map((v) => Math.max(0, Math.min(100, Number(v) || 0)))
    .filter((v) => Number.isFinite(v));
}

export function patchTrackAnalysisFields(ctx, track, payload) {
  if (!track || !payload || typeof payload !== "object") return false;
  let changed = false;
  const setIfChanged = (key, next) => {
    if (next === undefined) return;
    if (track[key] === next) return;
    track[key] = next;
    changed = true;
  };
  const bpm = Number(payload.bpm);
  if (payload.bpm !== undefined && payload.bpm !== null && Number.isFinite(bpm) && bpm > 0) {
    // Store the raw number, same as normalizeTrack -- display formatting is
    // formatBpm's job at render time, not state's.
    setIfChanged("bpm", bpm);
  }
  if (typeof payload.bpmAnalyzer === "string" && payload.bpmAnalyzer.trim()) {
    setIfChanged("bpmAnalyzer", payload.bpmAnalyzer.trim());
  }
  if (typeof payload.key === "string" && payload.key.trim()) {
    setIfChanged("key", payload.key.trim());
    setIfChanged("keyDisplay", typeof payload.keyDisplay === "string" ? payload.keyDisplay : "");
    setIfChanged("keyColor", Number.isInteger(payload.keyColor) ? payload.keyColor : null);
  }
  if (typeof payload.filePath === "string" && payload.filePath.trim()) {
    setIfChanged("filePath", payload.filePath.trim());
  }
  if (typeof payload.artworkPath === "string" && payload.artworkPath.trim()) {
    const artworkPath = payload.artworkPath.trim();
    setIfChanged("artworkPath", artworkPath);
    setIfChanged("artworkUrl", appendUrlRevision(ctx.toPlayableUrl(artworkPath) || "", payload.updatedAt || Date.now()));
    setIfChanged("artworkChecked", true);
  } else if (payload.artworkChecked === true) {
    setIfChanged("artworkChecked", true);
  }
  if (typeof payload.waveformPeaksPath === "string" && payload.waveformPeaksPath.trim()) {
    setIfChanged("waveformPeaksPath", payload.waveformPeaksPath.trim());
  }
  const durationMs = Number(payload.durationMs);
  if (payload.durationMs !== undefined && payload.durationMs !== null
      && Number.isFinite(durationMs) && durationMs > 0) {
    setIfChanged("durationMs", Math.round(durationMs));
  }
  const waveformPreview = parseProgressWaveformPreview(payload.waveformPreview);
  if (waveformPreview && waveformPreview.length) {
    const samePreview = Array.isArray(track.waveformPreview)
      && track.waveformPreview.length === waveformPreview.length
      && track.waveformPreview.every((value, index) => Number(value) === Number(waveformPreview[index]));
    if (!samePreview) {
      track.waveformPreview = waveformPreview;
      changed = true;
    }
  }
  // Backend-owned readiness; only ever flips false -> true on a progress event.
  if (payload.analysisReady === true) {
    setIfChanged("analysisReady", true);
  }
  return changed;
}

export function patchLibraryRowCells(ctx, row, track) {
  if (!row || !track) return false;

  const cells = row.querySelectorAll('[role="cell"]');
  if (cells.length < 3) return false;

  const durationTd = row.querySelector(".td-length");
  if (durationTd) {
    durationTd.textContent = formatDurationMsInternal(track.durationMs);
  }

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
        coverTd.replaceChildren(coverElement(row.ownerDocument, coverCandidates));
        attachCoverFallbackHandlers(coverTd);
      }
    } else if (coverTd.querySelector("img.cover-thumb")) {
      coverTd.replaceChildren(coverElement(row.ownerDocument, []));
    }
  }

  const waveformTd = row.querySelector(".td-waveform");
  if (waveformTd) {
    const waveformDiv = waveformTd.querySelector(".waveform");
    if (waveformDiv) {
      const colorData = Array.isArray(track.waveformColorData) && track.waveformColorData.length >= 6
        ? track.waveformColorData : null;
      const peaks = Array.isArray(track.waveformPreview)
        ? track.waveformPreview.map((v) => Math.max(0, Math.min(100, Number(v) || 0))).filter((v) => Number.isFinite(v))
        : [];
      const hasRenderableWaveform = colorData !== null || (peaks.length > 0 && peaks.some((v) => v > 0));
      if (hasRenderableWaveform) {
        invalidateWaveformCache(waveformDiv);
        if (colorData) {
          setWaveformColorData(waveformDiv, colorData);
          delete waveformDiv.dataset.peaks;
        } else {
          waveformDiv.dataset.peaks = peaks.join(",");
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
    }
  }

  const actionTd = row.querySelector(".td-action");
  if (actionTd) {
    const analyzeBtn = actionTd.querySelector("[data-action='analyze-track']");
    if (analyzeBtn && track.analysisReady) {
      analyzeBtn.textContent = "Reanalyze";
      analyzeBtn.dataset.tooltip = "Recompute waveform/BPM/key";
    }
  }

  return true;
}

function formatDurationMsInternal(value) {
  const rawMs = Number(value);
  return Number.isFinite(rawMs) && rawMs > 0 ? formatDurationMs(rawMs) : "-";
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

// --- source_root_filter.mjs ---

export function normalizePath(value) {
  return String(value || "").replace(/\\/g, "/").trim().toLowerCase();
}

export function enabledSourceRoots(sourceRoots, sourceRootEnabled = {}, missingSourceRoots = null) {
  const roots = Array.isArray(sourceRoots) ? sourceRoots : [];
  const missingKeys = missingSourceRoots instanceof Set
    ? Array.from(missingSourceRoots).map((root) => normalizePath(root).replace(/\/+$/, ""))
    : (Array.isArray(missingSourceRoots)
      ? missingSourceRoots.map((root) => normalizePath(root).replace(/\/+$/, ""))
      : []);
  return roots.filter((root) => {
    const rootKey = normalizePath(root).replace(/\/+$/, "");
    return sourceRootEnabled[root] !== false && !missingKeys.includes(rootKey);
  });
}

export function scanLibraryButtonLabel(sourceRoots, selectedCount = 0) {
  if (Number(selectedCount) > 0) return "Analyze Selected";
  const count = Array.isArray(sourceRoots) ? sourceRoots.length : 0;
  return count > 1 ? "Scan Libraries" : "Scan Library";
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
