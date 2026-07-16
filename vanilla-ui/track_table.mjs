export function createTrackRow(track, options, deps) {
  const {
    state,
    buildCoverSrcCandidates,
    isTrackCurrentlyPlaying,
    escapeHtml,
    trackHasCoreAnalysis,
    getKeyHue,
  } = deps;

  const localRenderId = options.origin === "local"
    ? String(track.localTrackId || track.id || "")
    : String(track.id || "");
  const renderTrackId = localRenderId || String(track.id || options.index || "row");
  const rowKey = `${options.origin || "unknown"}:${renderTrackId || options.index || "row"}`;
  const coverCandidates = buildCoverSrcCandidates(track);
  const coverCell = coverCandidates.length
    ? `<img class="cover-thumb" alt="cover" loading="lazy" src="${escapeHtml(coverCandidates[0])}" data-fallbacks="${escapeHtml(coverCandidates.slice(1).join("|"))}" />`
    : `<div class="cover-thumb" aria-hidden="true"></div>`;
  const isPlayingTrack = isTrackCurrentlyPlaying(track);
  const transportLabel = isPlayingTrack ? "Stop" : "Play";
  const transportIcon = isPlayingTrack
    ? `<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="7" y="7" width="10" height="10" rx="1"></rect></svg>`
    : `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 6v12l10-6z"></path></svg>`;
  const waveformAttrs = `data-action="scrub-play" data-index="${options.index}" data-id="${escapeHtml(renderTrackId)}" data-origin="${escapeHtml(options.origin || "usb")}"`;
  const peaks = Array.isArray(track.waveformPreview)
    ? track.waveformPreview
      .map((v) => Math.max(0, Math.min(100, Number(v) || 0)))
      .filter((v) => Number.isFinite(v))
    : [];
  const hasColorWaveform = Array.isArray(track.waveformColorData) && track.waveformColorData.length >= 6;
  const hasRenderableWaveform = hasColorWaveform || (peaks.length > 0 && peaks.some((v) => v > 0));
  const peaksData = peaks.length ? escapeHtml(peaks.join(",")) : "";
  const waveformCell = hasRenderableWaveform
    ? `<div class="waveform waveform-canvas" ${waveformAttrs} data-peaks="${peaksData}" aria-label="waveform preview" title="${escapeHtml(track.waveformPeaksPath || track.usbAnalysisPath || "")}"><canvas class="waveform-canvas-el" aria-hidden="true"></canvas><i class="waveform-playhead" aria-hidden="true"></i></div>`
    : `<div class="waveform" ${waveformAttrs} aria-label="waveform preview" title="${escapeHtml(track.waveformPeaksPath || track.usbAnalysisPath || "")}"><i class="waveform-playhead" aria-hidden="true"></i></div>`;
  const playInWaveform = options.secondaryActionLabel
    ? `<button class="transport-btn ${isPlayingTrack ? "is-playing" : ""}" data-action="${options.secondaryActionType}" data-index="${options.index}" data-id="${escapeHtml(renderTrackId)}" data-row-key="${escapeHtml(rowKey)}" data-origin="${escapeHtml(options.origin || "usb")}" aria-label="${transportLabel}" title="${transportLabel}">${transportIcon}</button>`
    : "";
  const waveformWithAction = `<div class="waveform-cell">${playInWaveform}${waveformCell}</div>`;

  const selectCell = options.withCheckbox
    ? `<div role="cell" class="track-grid-cell td-select"><input type="checkbox" data-id="${track.id}" ${options.selectedIds?.has(track.id) ? "checked" : ""} /></div>`
    : "";

  let actionCell = `<div role="cell" class="track-grid-cell td-action">-</div>`;
  if (options.actionLabel || options.enableAnalyzeActions) {
    const isRemoveAction = options.actionType === "remove-playlist-track";
    const playlistName = state.playlists?.find((p) => p.id === state.currentPlaylistId)?.name;
    const disabledAttr = !isRemoveAction && !state.currentPlaylistId ? " disabled" : "";
    const actionTitle = isRemoveAction
      ? (playlistName ? `Remove from ${playlistName}` : "Remove from playlist")
      : (playlistName
        ? `Add to ${playlistName}`
        : "Create and activate a playlist first, then add tracks to it.");
    const primary = options.actionLabel
      ? (options.compactAddButton
        ? `<button class="track-add-btn" data-action="${options.actionType}" data-index="${options.index}" data-id="${track.id}"${disabledAttr} title="${escapeHtml(actionTitle)}">${options.actionLabel}</button>`
        : `<button data-action="${options.actionType}" data-index="${options.index}" data-id="${track.id}"${disabledAttr} title="${escapeHtml(actionTitle)}">${options.actionLabel}</button>`)
      : "";
    const analysisButtons = options.enableAnalyzeActions
      ? `<button data-action="analyze-track" data-id="${escapeHtml(renderTrackId)}" title="${trackHasCoreAnalysis(track) ? "Recompute waveform/BPM/key" : "Analyze missing waveform/BPM/key"}">${trackHasCoreAnalysis(track) ? "Reanalyze" : "Analyze"}</button>`
      : "";
    actionCell = `<div role="cell" class="track-grid-cell td-action"><div class="action-buttons">${primary}${analysisButtons}</div></div>`;
  }

  const bpmTitle = track.bpmAnalyzer ? ` title="${escapeHtml(`Analyzed with: ${track.bpmAnalyzer}`)}"` : "";
  const bpmCell = track.bpm
    ? `<span class="bpm-pill"${bpmTitle}>${escapeHtml(track.bpm)}</span>`
    : "-";

  const keyHue = getKeyHue(track.key);
  const keyHueClass = `key-pill--h${((Math.round(Number(keyHue) / 30) % 12) + 12) % 12}`;
  const keyCell = track.key
    ? `<span class="key-pill ${keyHueClass}">${escapeHtml(track.key)}</span>`
    : "-";
  const formatInfo = describeTrackFormat(track);
  const formatCell = formatInfo.warning
    ? (formatInfo.kind === "autofix"
      ? `<div role="cell" class="track-grid-cell td-format"><span class="format-badge autofix" title="${escapeHtml(formatInfo.warning)}">${escapeHtml(formatInfo.label)} ⟳</span></div>`
      : `<div role="cell" class="track-grid-cell td-format"><span class="format-badge warn" title="${escapeHtml(formatInfo.warning)}">${escapeHtml(formatInfo.label)} ⚠</span></div>`)
    : `<div role="cell" class="track-grid-cell td-format"><span class="format-badge">${escapeHtml(formatInfo.label)}</span></div>`;
  const durationCell = `<div role="cell" class="track-grid-cell td-length">${escapeHtml(formatTrackDuration(track))}</div>`;

  return `
    <div role="row" class="track-grid-row" data-playback-row="${escapeHtml(rowKey)}" data-track-id="${escapeHtml(renderTrackId)}" data-track-index="${options.index}" data-track-origin="${escapeHtml(options.origin || "unknown")}">
      ${selectCell}
      <div role="cell" class="track-grid-cell td-cover">${coverCell}</div>
      <div role="cell" class="track-grid-cell td-waveform">${waveformWithAction}</div>
      <div role="cell" class="track-grid-cell td-track"><div class="track-info-cell">
        <span class="track-title">${escapeHtml(track.title)}</span>
        <span class="track-artist">${escapeHtml(track.artist)}</span>
      </div></div>
      <div role="cell" class="track-grid-cell td-album">${escapeHtml(track.album)}</div>
      ${formatCell}
      ${durationCell}
      <div role="cell" class="track-grid-cell td-bpm">${bpmCell}</div>
      <div role="cell" class="track-grid-cell td-key">${keyCell}</div>
      ${actionCell}
    </div>
  `;
}

export function renderTrackTable(tbody, tracks, options = {}, deps) {
  const {
    createTrackRow,
    attachCoverFallbackHandlers,
    renderWaveformsIn,
    setWaveformColorData,
    updateTransportButtonsInDom,
    escapeHtml,
    setStatus
  } = deps;

  tbody.innerHTML = "";

  if (!tracks.length) {
    tbody.innerHTML = `<div role="row" class="track-grid-row track-grid-row-empty"><div role="cell" class="track-grid-cell track-grid-empty">No tracks available.</div></div>`;
    return;
  }

  try {
    tracks.forEach((track, index) => {
      tbody.insertAdjacentHTML("beforeend", createTrackRow(track, { ...options, index }));
      const colorData = Array.isArray(track.waveformColorData) && track.waveformColorData.length >= 6
        ? track.waveformColorData
        : null;
      if (colorData && typeof setWaveformColorData === "function") {
        const row = tbody.lastElementChild;
        const waveform = row?.querySelector?.(".waveform");
        if (waveform) {
          setWaveformColorData(waveform, colorData);
        }
      }
    });
    attachCoverFallbackHandlers(tbody);
    renderWaveformsIn(tbody);
    updateTransportButtonsInDom();
  } catch (error) {
    tbody.innerHTML = "";
    tracks.forEach((track) => {
      const maybeSelectCell = options.withCheckbox
        ? `<div role="cell" class="track-grid-cell td-select">-</div>`
        : "";
      tbody.insertAdjacentHTML(
        "beforeend",
        `<div role="row" class="track-grid-row">${maybeSelectCell}<div role="cell" class="track-grid-cell td-cover">-</div><div role="cell" class="track-grid-cell td-waveform">-</div><div role="cell" class="track-grid-cell td-track">${escapeHtml(track.title)}</div><div role="cell" class="track-grid-cell td-album">-</div><div role="cell" class="track-grid-cell td-format">-</div><div role="cell" class="track-grid-cell td-length">-</div><div role="cell" class="track-grid-cell td-bpm">-</div><div role="cell" class="track-grid-cell td-key">-</div><div role="cell" class="track-grid-cell td-action">-</div></div>`
      );
    });
    setStatus(`Track render fallback used: ${error?.message || "unknown render error"}`);
  }
}

function formatTrackDuration(track) {
  const rawMs = Number(track?.durationMs);
  if (!Number.isFinite(rawMs) || rawMs <= 0) return "-";
  return formatDurationMs(rawMs);
}

function formatDurationMs(value) {
  const ms = Math.max(0, Math.round(Number(value) || 0));
  const totalSeconds = Math.floor(ms / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

function describeTrackFormat(track) {
  const direct = String(track?.formatExt || "").trim().toLowerCase();
  const inferred = (() => {
    const filePath = String(track?.filePath || "").trim();
    if (!filePath) return "";
    const match = filePath.match(/\.([a-zA-Z0-9]+)$/);
    return match ? String(match[1] || "").toLowerCase() : "";
  })();
  const ext = direct || inferred;
  const label = ext ? ext.toUpperCase() : "Unknown";

  if (ext === "wav") {
    const wavExtensibleKind = track?.wavExtensibleKind || null;
    if (wavExtensibleKind === "extensible_pcm") {
      return {
        label,
        kind: "autofix",
        warning: "Uses an extended WAV header (WAVE_FORMAT_EXTENSIBLE) that some CDJs reject. Will be automatically converted to standard PCM on export.",
      };
    }
    if (wavExtensibleKind === "extensible_other") {
      return {
        label,
        kind: "warn",
        warning: "Uses an extended WAV header with a non-standard subformat - cannot be safely converted and may not play on CDJ hardware.",
      };
    }
  }

  const sampleRate = Number(track?.sampleRateHz || 0) || null;
  const bitDepth = Number(track?.bitDepth || 0) || null;
  const bitrate = Number(track?.bitrateKbps || 0) || null;
  const warning = validateFormatCompatibility(ext, sampleRate, bitDepth, bitrate);
  return { label, kind: warning ? "warn" : null, warning };
}

function validateFormatCompatibility(ext, sampleRate, bitDepth, bitrate) {
  const inSet = (value, set) => value != null && set.includes(value);
  const inRange = (value, min, max) => value != null && value >= min && value <= max;

  const ratesA = [44100, 48000, 96000];
  const ratesB = [44100, 48000, 88200, 96000];
  const ratesAac = [16000, 22050, 24000, 32000, 44100, 48000];
  const bitsLossless = [16, 24];

  switch (ext) {
    case "wav":
      if (sampleRate == null || bitDepth == null) return null;
      if (!inSet(sampleRate, ratesA) || !inSet(bitDepth, bitsLossless)) {
        return "Outside WAV support (16/24-bit, 44.1/48/96 kHz)";
      }
      return null;
    case "mp3":
      if (sampleRate == null || bitrate == null) return null;
      if (!inSet(sampleRate, ratesAac)) {
        return "Outside MP3 sample-rate support";
      }
      if (inSet(sampleRate, [16000, 22050, 24000]) && !inRange(bitrate, 8, 160)) {
        return "Outside MP3 bitrate support for 16/22.05/24 kHz";
      }
      if (inSet(sampleRate, [32000, 44100, 48000]) && !inRange(bitrate, 32, 320)) {
        return "Outside MP3 bitrate support for 32/44.1/48 kHz";
      }
      return null;
    case "aac":
    case "mp4":
      if (sampleRate == null || bitrate == null) return null;
      if (!inSet(sampleRate, ratesAac) || !inRange(bitrate, 16, 320)) {
        return "Outside AAC support (16-320 kbps, 16/22.05/24/32/44.1/48 kHz)";
      }
      return null;
    case "m4a":
      if (sampleRate == null || (bitrate == null && bitDepth == null)) return null;
      // Could be AAC or ALAC. Pass if either profile matches.
      if (
        inSet(sampleRate, ratesAac)
        && inRange(bitrate, 16, 320)
      ) {
        return null;
      }
      if (inSet(sampleRate, ratesB) && inSet(bitDepth, bitsLossless)) {
        return null;
      }
      return "Outside AAC/ALAC support for .m4a";
    case "flac":
    case "fla":
      if (sampleRate == null || bitDepth == null) return null;
      if (!inSet(sampleRate, ratesB) || !inSet(bitDepth, bitsLossless)) {
        return "Outside FLAC support (16/24-bit, 44.1/48/88.2/96 kHz)";
      }
      return null;
    case "aif":
    case "aiff":
      if (sampleRate == null || bitDepth == null) return null;
      if (!inSet(sampleRate, ratesB) || !inSet(bitDepth, bitsLossless)) {
        return "Outside AIFF support (16/24-bit, 44.1/48/88.2/96 kHz)";
      }
      return null;
    default:
      return "Unlisted format";
  }
}

export function sortTracks(tracks, key, dir) {
  if (!key) return tracks;
  const sorted = [...tracks];
  const m = dir === "desc" ? -1 : 1;
  sorted.sort((a, b) => {
    switch (key) {
      case "title": return m * (a.title || "").localeCompare(b.title || "", undefined, { sensitivity: "base" });
      case "artist": {
        const byArtist = (a.artist || "").localeCompare(b.artist || "", undefined, { sensitivity: "base" });
        if (byArtist !== 0) return m * byArtist;
        return m * (a.title || "").localeCompare(b.title || "", undefined, { sensitivity: "base" });
      }
      case "album": return m * (a.album || "").localeCompare(b.album || "", undefined, { sensitivity: "base" });
      case "format": {
        const fa = String(a.formatExt || "").toLowerCase();
        const fb = String(b.formatExt || "").toLowerCase();
        return m * fa.localeCompare(fb, undefined, { sensitivity: "base" });
      }
      case "bpm": return m * ((Number(a.bpm) || 0) - (Number(b.bpm) || 0));
      case "durationMs": return m * ((Number(a.durationMs) || 0) - (Number(b.durationMs) || 0));
      case "key": return m * (a.key || "").localeCompare(b.key || "", undefined, { sensitivity: "base" });
      default: return 0;
    }
  });
  return sorted;
}
