import { formatDurationMs, formatBpm } from "./track_utils.mjs";
import { cloneTemplate } from "./ui_utils.mjs";
import { buildCoverSrcCandidates, attachCoverFallbackHandlers } from "./components/library/actions.mjs";
import { isTrackCurrentlyPlaying, updateTransportButtonsInDom } from "./components/playback/actions.mjs";
import { renderWaveformsIn, setWaveformColorData } from "./waveform.mjs";

// The BPM cell: the formatted value in a pill (tooltip = which analyzer set
// it), or "-" before analysis.
export function fillBpmCell(cell, track) {
  const bpmText = formatBpm(track.bpm);
  if (!bpmText) {
    cell.textContent = "-";
    return;
  }
  const pill = cloneTemplate(cell.ownerDocument, "tplBpmPill");
  pill.textContent = bpmText;
  const tooltip = track.bpmAnalyzer === "user"
    ? "Manually set"
    : track.bpmAnalyzer
      ? `Analyzed with: ${track.bpmAnalyzer}`
      : "";
  if (tooltip) pill.dataset.tooltip = tooltip;
  cell.replaceChildren(pill);
}

// The key cell. The backend sends the label in the user's key notation
// (`keyDisplay`, Classic or Camelot) and the wheel colour group (`keyColor`,
// 0..11); this only renders them. `key` itself is the classic value the
// backend stores and exports.
export function fillKeyCell(cell, track) {
  const label = track?.keyDisplay || track?.key;
  if (!label) {
    cell.textContent = "-";
    return;
  }
  const pill = cloneTemplate(cell.ownerDocument, "tplKeyPill");
  pill.textContent = label;
  const color = Number(track?.keyColor);
  if (Number.isInteger(color) && color >= 0 && color < 12) pill.classList.add(`key-pill--h${color}`);
  cell.replaceChildren(pill);
}

// A cover <img> trying `candidates` in order (see attachCoverFallbackHandlers),
// or the empty placeholder when there are none.
export function coverElement(doc, candidates) {
  if (!candidates.length) return cloneTemplate(doc, "tplCoverPlaceholder");
  const img = cloneTemplate(doc, "tplCoverImg");
  img.src = candidates[0];
  img.dataset.fallbacks = candidates.slice(1).join("|");
  return img;
}

export function transportIcon(doc, playing) {
  return cloneTemplate(doc, playing ? "tplIconStop" : "tplIconPlay");
}

export function createTrackRow(ctx, track, options) {
  const { state } = ctx;
  const doc = ctx.document;
  const row = cloneTemplate(doc, "tplTrackRow");
  const cell = (name) => row.querySelector(`.td-${name}`);

  const localRenderId = options.origin === "local"
    ? String(track.localTrackId || track.id || "")
    : String(track.id || "");
  const renderTrackId = localRenderId || String(track.id || options.index || "row");
  const rowKey = `${options.origin || "unknown"}:${renderTrackId || options.index || "row"}`;
  const origin = options.origin || "usb";
  row.dataset.playbackRow = rowKey;
  row.dataset.trackId = renderTrackId;
  row.dataset.trackIndex = String(options.index);
  row.dataset.trackOrigin = options.origin || "unknown";

  const dragCell = cell("drag");
  if (!options.reservesDragColumn) {
    dragCell.remove();
  } else if (!options.enableDragReorder) {
    const handle = dragCell.firstElementChild;
    if (options.dragDisabledTooltip) {
      handle.classList.add("disabled");
      handle.removeAttribute("draggable");
      delete handle.dataset.playlistTrackDragHandle;
      handle.dataset.tooltip = options.dragDisabledTooltip;
      handle.setAttribute("aria-label", options.dragDisabledTooltip);
    } else {
      handle.remove();
    }
  }

  const selectCell = cell("select");
  if (options.withCheckbox) {
    const checkbox = selectCell.firstElementChild;
    checkbox.dataset.id = track.id;
    checkbox.checked = !!options.selectedIds?.has(track.id);
  } else {
    selectCell.remove();
  }

  cell("cover").append(coverElement(doc, buildCoverSrcCandidates(ctx, track)));

  const transport = row.querySelector(".transport-btn");
  if (options.secondaryActionLabel) {
    const playing = isTrackCurrentlyPlaying(ctx, track);
    const label = playing ? "Stop" : "Play";
    transport.classList.toggle("is-playing", playing);
    Object.assign(transport.dataset, {
      action: options.secondaryActionType,
      index: String(options.index),
      id: renderTrackId,
      rowKey,
      origin,
      tooltip: label,
    });
    transport.setAttribute("aria-label", label);
    transport.append(transportIcon(doc, playing));
  } else {
    transport.remove();
  }

  const waveform = row.querySelector(".waveform");
  Object.assign(waveform.dataset, { index: String(options.index), id: renderTrackId, origin });
  const peaks = Array.isArray(track.waveformPreview)
    ? track.waveformPreview
      .map((v) => Math.max(0, Math.min(100, Number(v) || 0)))
      .filter((v) => Number.isFinite(v))
    : [];
  const hasColorWaveform = Array.isArray(track.waveformColorData) && track.waveformColorData.length >= 6;
  if (hasColorWaveform || (peaks.length > 0 && peaks.some((v) => v > 0))) {
    waveform.classList.add("waveform-canvas");
    waveform.dataset.peaks = peaks.join(",");
    waveform.prepend(cloneTemplate(doc, "tplWaveformCanvas"));
  }

  const cueButton = row.querySelector(".waveform-detail-btn");
  if (options.enableAnalyzeActions) {
    // Cue editing readiness is per-origin: local rows carry `analysisReady`;
    // USB rows (playlist/history) carry the serialized on-USB ANLZ path instead.
    const cueReady = options.origin === "usb" ? !!track.usbAnalysisPath : !!track.analysisReady;
    cueButton.dataset.index = String(options.index);
    cueButton.dataset.id = renderTrackId;
    cueButton.disabled = !cueReady;
    cueButton.dataset.tooltip = cueReady ? "Edit cue points & beat grid" : "Analyze this track first";
  } else {
    cueButton.remove();
  }

  row.querySelector(".track-title").textContent = track.title ?? "";
  row.querySelector(".track-artist").textContent = track.artist ?? "";
  cell("album").textContent = track.album ?? "";

  const formatInfo = describeTrackFormat(track);
  const badge = row.querySelector(".format-badge");
  const tooltip = formatTrackFormatTooltip(formatInfo);
  if (tooltip) badge.dataset.tooltip = tooltip;
  badge.textContent = formatInfo.label;
  if (formatInfo.warning) {
    const autofix = formatInfo.kind === "autofix";
    badge.classList.add(autofix ? "autofix" : "warn");
    badge.textContent += autofix ? " ⟳" : " ⚠";
  }
  cell("length").textContent = formatTrackDuration(track);
  fillBpmCell(cell("bpm"), track);
  fillKeyCell(cell("key"), track);

  const actionCell = cell("action");
  if (options.actionLabel || options.enableAnalyzeActions) {
    const [primary, analyze] = actionCell.querySelectorAll("button");
    if (options.actionLabel) {
      const isRemoveAction = options.actionType === "remove-playlist-track";
      const playlistName = state.playlists?.find((p) => p.id === state.currentPlaylistId)?.name;
      primary.dataset.action = options.actionType;
      primary.dataset.index = String(options.index);
      primary.dataset.id = track.id;
      primary.disabled = !isRemoveAction && !state.currentPlaylistId;
      primary.dataset.tooltip = isRemoveAction
        ? (playlistName ? `Remove from ${playlistName}` : "Remove from playlist")
        : (playlistName
          ? `Add to ${playlistName}`
          : "Create and activate a playlist first, then add tracks to it.");
      primary.textContent = options.actionLabel;
      if (!options.compactAddButton) primary.removeAttribute("class");
    } else {
      primary.remove();
    }
    // USB playlist / history rows (`origin: "usb"`) keep `enableAnalyzeActions`
    // on for the cue-editor button, but per-track analyze acts only on a local
    // library copy, so it doesn't belong on those lists.
    if (options.enableAnalyzeActions && options.origin !== "usb") {
      analyze.dataset.id = renderTrackId;
      analyze.dataset.tooltip = track.analysisReady ? "Recompute waveform/BPM/key" : "Analyze missing waveform/BPM/key";
      analyze.textContent = track.analysisReady ? "Reanalyze" : "Analyze";
    } else {
      analyze.remove();
    }
  } else {
    actionCell.textContent = "-";
  }

  return row;
}

const trackTableRenderTokens = new WeakMap();
const ROW_BUILD_CHUNK_SIZE = 300;

export async function renderTrackTable(ctx, tbody, tracks, options = {}) {

  // `options.append`: add `tracks` onto the end of what's already in `tbody`
  // instead of rebuilding it (used for paginated large-selection loading,
  // see hydrateSelectionTracks/paginateUsbSelection in usb/events.mjs).
  // `options.indexOffset`: since an appended page's `tracks` array is only
  // that page's slice, row indices (used for data-index/data-track-index,
  // which click handlers resolve back into the *full* view array) need to
  // be offset by how many rows already precede this page.
  const isAppend = !!options.append;
  const indexOffset = Number(options.indexOffset) || 0;

  // Bump the token for this tbody so a slower, still-running render started
  // by a previous call (e.g. the user rapidly re-selecting playlists) knows
  // to stop appending rows once a newer render has taken over. An append
  // call doesn't bump it -- it's a continuation of the render that already
  // holds the current token, and gets invalidated the same way if a fresh
  // (non-append) render for this tbody starts before it finishes.
  let myToken;
  if (isAppend) {
    myToken = trackTableRenderTokens.get(tbody) ?? 0;
  } else {
    myToken = (trackTableRenderTokens.get(tbody) || 0) + 1;
    trackTableRenderTokens.set(tbody, myToken);
  }
  const isStale = () => trackTableRenderTokens.get(tbody) !== myToken;

  if (!isAppend) {
    tbody.replaceChildren();
  }

  if (!tracks.length) {
    if (!isAppend) {
      tbody.append(cloneTemplate(tbody.ownerDocument, "tplTrackRowEmpty"));
    }
    return;
  }

  try {
    for (let i = 0; i < tracks.length; i += 1) {
      if (isStale()) return;
      const track = tracks[i];
      const index = i + indexOffset;
      const row = createTrackRow(ctx, track, { ...options, index });
      if (Array.isArray(track.waveformColorData) && track.waveformColorData.length >= 6) {
        setWaveformColorData(row.querySelector(".waveform"), track.waveformColorData);
      }
      tbody.append(row);
      if ((i + 1) % ROW_BUILD_CHUNK_SIZE === 0 && i + 1 < tracks.length) {
        await new Promise((resolve) => setTimeout(resolve, 0));
      }
    }
    if (isStale()) return;
    attachCoverFallbackHandlers(tbody);
    await renderWaveformsIn(tbody);
    if (isStale()) return;
    updateTransportButtonsInDom(ctx, tbody);
  } catch (error) {
    if (isStale()) return;
    if (!isAppend) {
      tbody.replaceChildren();
    }
    tracks.forEach((track) => {
      const row = cloneTemplate(tbody.ownerDocument, "tplTrackRowFallback");
      if (!options.reservesDragColumn) row.querySelector(".td-drag").remove();
      if (!options.withCheckbox) row.querySelector(".td-select").remove();
      row.querySelector(".td-track").textContent = track.title ?? "";
      tbody.append(row);
    });
    ctx.emitStatus(`Track render fallback used: ${error?.message || "unknown render error"}`);
  }
}

function formatTrackDuration(track) {
  const rawMs = Number(track?.durationMs);
  if (!Number.isFinite(rawMs) || rawMs <= 0) return "-";
  return formatDurationMs(rawMs);
}

function formatTrackFormatDetail(sampleRate, bitDepth, bitrate) {
  const parts = [];
  if (sampleRate) {
    parts.push(`${(sampleRate / 1000).toLocaleString(undefined, { maximumFractionDigits: 1 })} kHz`);
  }
  if (bitDepth) {
    parts.push(`${bitDepth}-bit`);
  } else if (bitrate) {
    parts.push(`${bitrate} kbps`);
  }
  return parts.join(" · ");
}

function describeTrackFormat(track) {
  // Backend-owned: every track-returning command populates `formatExt` (falling
  // back to the file-path extension server-side) and `formatCompat` (severity +
  // warning, computed in Rust by service::format_compat). The frontend renders
  // the badge and never re-derives the CDJ-compatibility rule.
  const ext = String(track?.formatExt || "").trim().toLowerCase();
  const label = ext ? ext.toUpperCase() : "Unknown";
  const sampleRate = Number(track?.sampleRateHz || 0) || null;
  const bitDepth = Number(track?.bitDepth || 0) || null;
  const bitrate = Number(track?.bitrateKbps || 0) || null;
  const detail = formatTrackFormatDetail(sampleRate, bitDepth, bitrate);

  const compat = track?.formatCompat || {};
  const severity = String(compat.severity || "ok");
  return {
    label,
    detail,
    kind: severity === "ok" ? null : severity,
    warning: compat.warning || null,
  };
}

function formatTrackFormatTooltip(formatInfo) {
  if (formatInfo.warning && formatInfo.detail) return `${formatInfo.detail} — ${formatInfo.warning}`;
  return formatInfo.warning || formatInfo.detail || "";
}

