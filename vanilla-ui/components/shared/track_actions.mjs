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

// Resolve the track a row action/click targets, from the array the table
// currently shows (`controller.view`). Every rendered row carries both its
// position in that view (`data-index` on the control, `data-track-index` on
// the row) and a stable identity (`data-id` / `data-track-id`). All four
// track-list views (library, app playlist, USB playlist, USB history) render
// `view` in exact order, so the index is the primary key; identity is the
// fallback for a row that was patched in place out of order.
export function resolveRowActionTrack(view, el) {
  const list = Array.isArray(view) ? view : [];
  const row = el?.closest?.(".track-grid-row") || null;
  const attrEl = el?.closest?.("[data-index],[data-id]") || null;

  const idx = Number(attrEl?.dataset?.index ?? row?.dataset?.trackIndex);
  if (Number.isInteger(idx) && idx >= 0 && idx < list.length) return list[idx];

  const id = String(attrEl?.dataset?.id ?? row?.dataset?.trackId ?? "").trim();
  if (id) {
    return list.find(
      (t) => String(t?.id) === id || String(t?.localTrackId ?? "") === id,
    ) ?? null;
  }
  return null;
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
      : (
        state?.playbackPendingKind === "stop"
          ? false
          : state?.playbackPendingKind === "play"
            ? ((rowKey && state.playbackPendingRowKey === rowKey) || isTrackCurrentlyPlaying(track))
            : ((rowKey && state.playbackRowKey === rowKey) || isTrackCurrentlyPlaying(track))
      );
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
