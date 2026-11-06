import { formatDurationMs, formatBpm, applyAnalyzeButtonState } from "./track_utils.mjs";
import { cloneTemplate } from "./ui_utils.mjs";
import { coverUrl, attachCoverFallbackHandlers } from "./components/library/actions.mjs";
import { isTrackCurrentlyPlaying, moveActiveWaveform, updateTransportButtonsInDom } from "./components/playback/actions.mjs";
import { renderWaveforms, setWaveformColorData } from "./waveform.mjs";

// The BPM cell: the formatted value in a pill (tooltip = which analyzer set
// it), or "-" before analysis.
// Where a BPM came from when it wasn't this app's analysis.
const BPM_SOURCE_TOOLTIPS = {
  user: "Manually set",
  mixxx: "From Mixxx",
  rekordbox: "From rekordbox"
};

export function fillBpmCell(cell, track) {
  const bpmText = formatBpm(track.bpm);
  if (!bpmText) {
    cell.textContent = "-";
    return;
  }
  const pill = cloneTemplate(cell.ownerDocument, "tplBpmPill");
  pill.textContent = bpmText;
  const tooltip = BPM_SOURCE_TOOLTIPS[track.bpmAnalyzer]
    || (track.bpmAnalyzer ? `Analyzed with: ${track.bpmAnalyzer}` : "");
  if (tooltip) pill.dataset.tooltip = tooltip;
  cell.replaceChildren(pill);
}

// The year cell: the release year, or blank.
export function fillYearCell(cell, track) {
  cell.textContent = track.year ? String(track.year) : "";
}

// The cue editor button: enabled once the track is analyzed (`ready`) and
// its row has a waveform to edit on -- a reanalysis clears the waveform, so
// the button waits for the new one. `ready` is kept on the button, because
// live patches of USB rows get the library track, which has no USB analysis.
export function setCueButtonState(button, { ready = button.dataset.cueReady === "true", hasWaveform }) {
  button.dataset.cueReady = String(!!ready);
  const enabled = !!ready && !!hasWaveform;
  button.disabled = !enabled;
  button.dataset.tooltip = enabled ? "Edit cue points & beat grid" : "Analyze this track first";
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

// Whether a row has a waveform to draw: the colour data, or a preview (the
// backend's 0-100 levels) with any level above zero.
export function trackHasWaveform(track) {
  return track.waveformColorData?.length >= 6 || !!track.waveformPreview?.some((v) => v > 0);
}

// A cover <img> of `src` (see attachCoverFallbackHandlers), or the empty
// placeholder when there is none.
export function coverElement(doc, src) {
  if (!src) return cloneTemplate(doc, "tplCoverPlaceholder");
  const img = cloneTemplate(doc, "tplCoverImg");
  img.src = src;
  return img;
}

export function transportIcon(doc, playing) {
  return cloneTemplate(doc, playing ? "tplIconStop" : "tplIconPlay");
}

// The source icon next to the checkbox (backend-owned `track.fileSource`, sent
// with an app playlist's rows): which icon, red when the track isn't
// available, and the tooltip.
function fillTrackSource(row, track) {
  const source = track.fileSource;
  const cell = row.querySelector(".td-select");
  if (!source || !cell) return;
  const icon = cloneTemplate(row.ownerDocument, source.kind === "usb" ? "tplTrackSourceUsb" : "tplTrackSourceLocal");
  icon.classList.toggle("is-missing", !!source.missing);
  icon.dataset.tooltip = source.tooltip;
  icon.setAttribute("aria-label", source.tooltip);
  cell.append(icon);
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

  // The play button sits over the cover (see .cover-play).
  cell("cover").firstElementChild.prepend(coverElement(doc, coverUrl(track)));

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
  const hasWaveform = trackHasWaveform(track);
  if (hasWaveform) {
    waveform.classList.add("waveform-canvas");
    waveform.dataset.peaks = (track.waveformPreview || []).join(",");
    waveform.prepend(cloneTemplate(doc, "tplWaveformCanvas"));
  }

  const cueButton = row.querySelector(".waveform-detail-btn");
  if (options.enableAnalyzeActions) {
    // Cue editing readiness is per-origin: local rows carry `analysisReady`;
    // USB rows (playlist/history) carry the serialized on-USB ANLZ path instead.
    const cueReady = options.origin === "usb" ? !!track.usbAnalysisPath : !!track.analysisReady;
    cueButton.dataset.index = String(options.index);
    cueButton.dataset.id = renderTrackId;
    setCueButtonState(cueButton, { ready: cueReady, hasWaveform });
  } else {
    cueButton.remove();
  }

  row.querySelector(".track-title").textContent = track.title ?? "";
  row.querySelector(".track-artist").textContent = track.artist ?? "";
  fillTrackSource(row, track);
  cell("album").textContent = track.album ?? "";
  fillYearCell(cell("year"), track);

  fillFormatCell(row.querySelector(".td-format"), track);
  cell("length").textContent = formatTrackDuration(track);
  fillBpmCell(cell("bpm"), track);
  fillKeyCell(cell("key"), track);

  const actionCell = cell("action");
  if (options.actionLabel || options.enableAnalyzeActions) {
    const [primary, analyze] = actionCell.querySelectorAll("button");
    if (options.actionLabel) {
      const isRemoveAction = options.actionType === "remove-playlist-track";
      primary.dataset.action = options.actionType;
      primary.dataset.index = String(options.index);
      primary.dataset.id = track.id;
      if (isRemoveAction) {
        const playlistName = state.playlists?.find((p) => p.id === state.currentPlaylistId)?.name;
        primary.dataset.tooltip = playlistName ? `Remove from ${playlistName}` : "Remove from playlist";
      } else {
        setAddButtonState(primary, state.playlists?.find((p) => p.id === state.currentPlaylistId));
      }
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
      analyze.textContent = track.analysisReady ? "Reanalyze" : "Analyze";
      applyAnalyzeButtonState(state, analyze, renderTrackId, track.analysisReady);
    } else {
      analyze.remove();
    }
  } else {
    actionCell.textContent = "-";
  }

  return row;
}

// The skeleton row's shimmer cycle; matches the animation in styles.css.
const SKELETON_SHIMMER_MS = 1600;

// One row of a track table, at `options.index` in the whole list: the
// track's row, or a skeleton while its page isn't loaded (`track` undefined).
// Rows are striped by that index, not by DOM position, since the table only
// holds the rows around the viewport.
// A row's "+": enabled, and saying where it adds, only with an active
// `playlist` (as the header shows it). Also redrawn on rows already shown
// when the active playlist or the playlist list changes
// (`updateAddToPlaylistButtons`).
export function setAddButtonState(button, playlist) {
  button.disabled = !playlist;
  button.dataset.tooltip = playlist
    ? `Add to ${playlist.name}`
    : "Create and activate a playlist first, then add tracks to it.";
}

export function buildTrackTableRow(ctx, track, options) {
  let row;
  if (!track) {
    row = cloneTemplate(ctx.document, "tplTrackRowSkeleton");
    // Join the shimmer where the rows already on screen are, not at its start.
    row.style.setProperty("--skeleton-phase", `${-(performance.now() % SKELETON_SHIMMER_MS)}ms`);
  } else {
    try {
      row = createTrackRow(ctx, track, options);
      if (Array.isArray(track.waveformColorData) && track.waveformColorData.length >= 6) {
        setWaveformColorData(row.querySelector(".waveform"), track.waveformColorData);
      }
    } catch (error) {
      row = cloneTemplate(ctx.document, "tplTrackRowFallback");
      if (!options.reservesDragColumn) row.querySelector(".td-drag").remove();
      if (!options.withCheckbox) row.querySelector(".td-select").remove();
      row.querySelector(".td-track").textContent = track.title ?? "";
      ctx.emitStatus(`Track render fallback used: ${error?.message || "unknown render error"}`);
    }
  }
  row.dataset.trackIndex = String(options.index);
  row.classList.toggle("is-alt", options.index % 2 === 1);
  return row;
}

export function renderEmptyTrackTable(ctx, tbody) {
  tbody.replaceChildren(cloneTemplate(tbody.ownerDocument, "tplTrackRowEmpty"));
}

// Mark the row the cue editor was just closed on (`is-last-edited`) in every
// list, and unmark the previous one.
export function markLastEditedRow(ctx, rowKey) {
  ctx.state.lastEditedRowKey = rowKey;
  for (const row of ctx.document.querySelectorAll(".track-grid-row")) {
    row.classList.toggle("is-last-edited", row.dataset.playbackRow === rowKey);
  }
}

// Wire up rows just put in the table: cover placeholders, transport state,
// waveforms. A rebuilt row of the playing track takes the playhead back --
// the row it had was dropped when it left the rendered window.
export async function finishTrackRows(ctx, rows) {
  const { state } = ctx;
  const waveforms = [];
  for (const row of rows) {
    attachCoverFallbackHandlers(row);
    updateTransportButtonsInDom(ctx, row);
    row.classList.toggle("is-last-edited", !!state.lastEditedRowKey && row.dataset.playbackRow === state.lastEditedRowKey);
    waveforms.push(...row.querySelectorAll(".waveform.waveform-canvas"));
    const playingHere = state.playbackActive
      && state.playbackRowKey
      && row.dataset.playbackRow === state.playbackRowKey
      && !state.activeWaveform?.isConnected;
    if (playingHere) {
      moveActiveWaveform(ctx, row.querySelector(".waveform"))
        .catch((err) => ctx.warn("Playhead re-attach failed:", err));
    }
  }
  await renderWaveforms(waveforms);
}

export function formatTrackDuration(track) {
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

// The FMT cell: the format badge (with the backend's compatibility warning),
// then the track's transcodes. Also redraws a row already shown.
//
// Backend-owned `track.exportFile` (an app playlist read for the selected
// USB): what export puts on that USB when it isn't the track's own file. The
// badge and its tooltip are then the backend's: the format that goes, and
// the track's own file as its source.
export function fillFormatCell(cell, track) {
  const badge = cell?.querySelector(".format-badge");
  if (!badge) return;
  badge.classList.remove("autofix", "warn", "export-convert");
  cell.querySelectorAll(".format-transcode").forEach((line) => line.remove());
  const exportFile = track?.exportFile || null;
  if (exportFile) {
    badge.textContent = exportFile.badge;
    badge.dataset.tooltip = exportFile.tooltip;
    badge.classList.toggle("export-convert", exportFile.kind === "convert");
    if (exportFile.kind === "refused") {
      badge.classList.add("warn");
      badge.textContent += " ⚠";
    }
  } else {
    const formatInfo = describeTrackFormat(track);
    const tooltip = formatTrackFormatTooltip(formatInfo);
    if (tooltip) badge.dataset.tooltip = tooltip;
    else delete badge.dataset.tooltip;
    badge.textContent = formatInfo.label;
    if (formatInfo.warning) {
      const autofix = formatInfo.kind === "autofix";
      badge.classList.add(autofix ? "autofix" : "warn");
      badge.textContent += autofix ? " ⟳" : " ⚠";
    }
  }
  appendTranscodeLines(badge, track);
}

// Backend-owned `track.transcodes` (the track's other files, made from the
// library): one small line each under the format badge, except the one
// export puts on the USB (`exported`), which the badge itself shows.
function appendTranscodeLines(badge, track) {
  const transcodes = Array.isArray(track?.transcodes) ? track.transcodes : [];
  const doc = badge.ownerDocument;
  for (const transcode of transcodes) {
    if (transcode.exported) continue;
    const line = cloneTemplate(doc, "tplFormatTranscode");
    line.textContent = transcode.badge;
    line.dataset.tooltip = transcode.tooltip;
    line.classList.toggle("outdated", !!transcode.outdated);
    badge.after(line);
    badge = line;
  }
}

function formatTrackFormatTooltip(formatInfo) {
  if (formatInfo.warning && formatInfo.detail) return `${formatInfo.detail} — ${formatInfo.warning}`;
  return formatInfo.warning || formatInfo.detail || "";
}

