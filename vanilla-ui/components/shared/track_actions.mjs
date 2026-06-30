// Resolves the emitStatus function from ctx, falling back to setStatus or a no-op.
export function resolveEmitStatus(ctx) {
  return typeof ctx.emitStatus === "function"
    ? ctx.emitStatus
    : (typeof ctx.setStatus === "function" ? ctx.setStatus : () => {});
}

// Returns a .catch() handler that logs and emits the error message.
export function catchErr(emitStatus) {
  return (err) => {
    console.error(err);
    emitStatus(err?.message || String(err));
  };
}

// Stable fingerprint of the metadata fields that hydration may populate.
export function trackMetaFingerprint(track) {
  return `${Array.isArray(track?.waveformPreview) ? track.waveformPreview.join(",") : ""}|${String(track?.artworkUrl || track?.artworkDataUrl || "")}|${track?.artworkChecked === true ? "art-ok" : ""}|${String(track?.bpm || "")}|${String(track?.key || "")}`;
}

// Handles add / analyze / play / scrub track table actions.
// Returns true if the action was handled (caller should return after).
export function handleTrackAction({ action, track, origin, target, event, state, rowKey, ctx }) {
  const {
    addTracksToCurrentPlaylist,
    analyzeSingleTrack,
    getPlaybackUiStateHelpers,
    isTrackCurrentlyPlaying,
    stopPlaybackFromUi,
    playTrackFromOrigin,
    scrubRatioFromPointer,
  } = ctx;
  const emitStatus = resolveEmitStatus(ctx);

  if (action === "add-library" || action === "add-usb" || action === "add-history") {
    addTracksToCurrentPlaylist([track]).catch(catchErr(emitStatus));
    return true;
  }

  if (action === "analyze-track") {
    analyzeSingleTrack(track).catch((err) => {
      console.error(err);
      emitStatus(`Analyze failed: ${err?.message || err}`);
    });
    return true;
  }

  if (action === "play-library" || action === "play-usb" || action === "play-history") {
    const helpers = getPlaybackUiStateHelpers();
    const stopRequested = helpers?.shouldToggleStop
      ? helpers.shouldToggleStop(state, rowKey, isTrackCurrentlyPlaying(track))
      : ((rowKey && state.playbackRowKey === rowKey) || isTrackCurrentlyPlaying(track));
    if (stopRequested) {
      stopPlaybackFromUi().catch((err) => {
        console.error(err);
        emitStatus(`Stop failed: ${err?.message}`);
      });
      return true;
    }
    const waveformEl = target.closest(".track-grid-row")?.querySelector(".waveform");
    playTrackFromOrigin(track, origin, { waveformEl, rowKey }).catch((err) => {
      console.error(err);
      emitStatus(`Playback failed: ${err?.message}`);
    });
    return true;
  }

  if (action === "scrub-play") {
    const waveformEl = target.closest(".waveform");
    const startRatio = scrubRatioFromPointer(event, waveformEl);
    playTrackFromOrigin(track, target?.dataset?.origin || origin, { startRatio, waveformEl, rowKey }).catch((err) => {
      console.error(err);
      emitStatus(`Playback failed: ${err?.message}`);
    });
    return true;
  }

  return false;
}
