// DOM event wiring for the track-detail (cues + beat grid) modal.

import { scrubRatioFromPointer } from "../playback/actions.mjs";
import { HOTCUE_PALETTE } from "./actions.mjs";

export function bindTrackDetailEvents(ctx) {
  const { el, trackDetailDialog } = ctx;
  const overlay = el.trackDetailOverlay;
  if (!overlay || !trackDetailDialog) return;

  let playbackStartedHere = false;

  const stopIfOwned = () => {
    if (playbackStartedHere && ctx.stopPlaybackFromUi) {
      playbackStartedHere = false;
      ctx.stopPlaybackFromUi().catch(() => {});
    }
  };
  const close = () => {
    stopIfOwned();
    trackDetailDialog.close(null);
  };

  el.trackDetailCloseBtn?.addEventListener("click", close);
  el.trackDetailCancelBtn?.addEventListener("click", close);
  // An outside click or Escape closes the dialog only when there is nothing
  // to lose; with unsaved edits it just points at Save (Cancel still discards).
  const closeIfClean = () => {
    if (!trackDetailDialog.hasUnsavedChanges()) {
      close();
      return;
    }
    const save = el.trackDetailSaveBtn;
    if (!save) return;
    save.classList.remove("is-attention");
    void save.offsetWidth; // restart the animation on repeated attempts
    save.classList.add("is-attention");
  };
  overlay.addEventListener("mousedown", (event) => {
    if (event.target === overlay) closeIfClean();
  });
  el.trackDetailSaveBtn?.addEventListener("animationend", (event) =>
    event.currentTarget.classList.remove("is-attention")
  );
  // On the document, like the shell's dialogs: deleting a cue removes the
  // focused button, leaving focus on <body>, outside the overlay.
  overlay.ownerDocument.addEventListener("keydown", (event) => {
    if (event.key !== "Escape" || overlay.hidden) return;
    if (el.confirmOverlay && !el.confirmOverlay.hidden) return; // its own Escape
    event.preventDefault();
    closeIfClean();
  });

  el.trackDetailSaveBtn?.addEventListener("click", () => {
    stopIfOwned();
    trackDetailDialog.close(trackDetailDialog.toSavePayload());
  });

  // A drag (pan, cue marker, overview) must never select text, whichever
  // engine renders the app (WebKitGTK in the Tauri build): drop any selection
  // on press, mark the page unselectable, and cancel `selectstart` until the
  // button is released anywhere.
  const doc = overlay.ownerDocument;
  let dragNoSelect = false;
  const endDragNoSelect = () => {
    if (!dragNoSelect) return;
    dragNoSelect = false;
    doc.documentElement.classList.remove("is-ui-dragging");
    window.removeEventListener("pointerup", endDragNoSelect, true);
    window.removeEventListener("pointercancel", endDragNoSelect, true);
  };
  const beginDragNoSelect = () => {
    doc.defaultView?.getSelection?.()?.removeAllRanges();
    if (dragNoSelect) return;
    dragNoSelect = true;
    doc.documentElement.classList.add("is-ui-dragging");
    window.addEventListener("pointerup", endDragNoSelect, true);
    window.addEventListener("pointercancel", endDragNoSelect, true);
  };
  doc.addEventListener("selectstart", (event) => {
    if (dragNoSelect) event.preventDefault();
  });

  // --- Waveform: click to play, double-click to add a cue, wheel to zoom, drag to pan,
  // drag a cue marker to move it (Shift snaps to the beat grid) ---
  const wf = el.trackDetailWaveform;
  const PAN_THRESHOLD_PX = 4;
  let pan = null; // { startX, startViewMs, moved }
  let cueDrag = null; // { tempId, startX, moved }
  let suppressMarkerClick = false;
  let pendingPlay = null;

  const playFromRatio = (startRatio) => {
    const track = trackDetailDialog.getWorking().track;
    if (!track || !ctx.playTrackFromOrigin) return;
    playbackStartedHere = true;
    ctx
      .playTrackFromOrigin(track, "local", { startRatio, waveformEl: wf })
      .then(() => trackDetailDialog.notePlaybackStarted())
      .catch(() => {});
  };
  const playFromCue = (cue) => {
    const dur = trackDetailDialog.getWorking().durationMs;
    if (cue && dur) playFromRatio(cue.positionMs / dur);
  };
  const playFromPointer = (clientX) =>
    playFromRatio(
      trackDetailDialog.viewRatioToTrackRatio(scrubRatioFromPointer({ clientX }, wf))
    );

  wf?.addEventListener("wheel", (event) => {
    event.preventDefault();
    const ratio = scrubRatioFromPointer(event, wf);
    const factor = Math.exp((event.deltaY || 0) * 0.0015);
    trackDetailDialog.zoomAt(ratio, factor);
  }, { passive: false });

  // Marker drag tracks the pointer on the window rather than via pointer
  // capture: capture would retarget the `click` that a plain (un-moved) press
  // on a marker must still deliver to it for click-to-play.
  const onCueDragMove = (event) => {
    if (!cueDrag.moved && Math.abs(event.clientX - cueDrag.startX) < PAN_THRESHOLD_PX) return;
    if (!cueDrag.moved) {
      cueDrag.moved = true;
      wf.classList.add("is-dragging-cue");
      trackDetailDialog.setDraggingCue(cueDrag.tempId);
    }
    trackDetailDialog.moveCueToViewRatio(
      cueDrag.tempId,
      scrubRatioFromPointer(event, wf),
      { snap: event.shiftKey }
    );
  };
  const endCueDrag = () => {
    if (!cueDrag) return;
    const { moved } = cueDrag;
    suppressMarkerClick = moved;
    cueDrag = null;
    window.removeEventListener("pointermove", onCueDragMove);
    window.removeEventListener("pointerup", endCueDrag);
    window.removeEventListener("pointercancel", endCueDrag);
    // A plain press must leave the marker DOM alone: re-rendering here would
    // detach the marker before its `click` fires.
    if (!moved) return;
    wf.classList.remove("is-dragging-cue");
    trackDetailDialog.setDraggingCue(null);
  };

  wf?.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    beginDragNoSelect();
    suppressMarkerClick = false;
    const marker = event.target.closest(".cue-marker");
    if (marker) {
      cueDrag = { tempId: marker.dataset.tempId, startX: event.clientX, moved: false };
      window.addEventListener("pointermove", onCueDragMove);
      window.addEventListener("pointerup", endCueDrag);
      window.addEventListener("pointercancel", endCueDrag);
      return;
    }
    pan = {
      startX: event.clientX,
      startViewMs: trackDetailDialog.getView().startMs,
      moved: false,
    };
    wf.classList.add("is-panning");
  });
  wf?.addEventListener("pointermove", (event) => {
    if (!pan) return;
    const dx = event.clientX - pan.startX;
    if (!pan.moved && Math.abs(dx) < PAN_THRESHOLD_PX) return;
    pan.moved = true;
    const width = wf.clientWidth || 1;
    const view = trackDetailDialog.getView();
    const span = Math.max(1, view.endMs - view.startMs);
    trackDetailDialog.setView(pan.startViewMs - (dx / width) * span, span);
  });
  const endPan = (event) => {
    if (!pan) return;
    const wasMove = pan.moved;
    pan = null;
    wf.classList.remove("is-panning");
    if (wasMove || event.target.closest(".cue-marker")) return;
    // Defer the play so a following double-click can cancel it and add a cue instead.
    const { clientX } = event;
    clearTimeout(pendingPlay);
    pendingPlay = setTimeout(() => {
      pendingPlay = null;
      playFromPointer(clientX);
    }, 230);
  };
  wf?.addEventListener("pointerup", endPan);
  wf?.addEventListener("pointercancel", () => {
    pan = null;
    wf.classList.remove("is-panning");
  });
  wf?.addEventListener("dblclick", (event) => {
    if (event.target.closest(".cue-marker")) return;
    clearTimeout(pendingPlay);
    pendingPlay = null;
    const trackRatio = trackDetailDialog.viewRatioToTrackRatio(
      scrubRatioFromPointer(event, wf)
    );
    if (!trackDetailDialog.addCueAtRatio(trackRatio)) {
      ctx.emitStatus?.("Maximum 8 cue points.");
    }
  });

  el.trackDetailPlayPause?.addEventListener("click", () => {
    clearTimeout(pendingPlay);
    pendingPlay = null;
    if (trackDetailDialog.isPlaying()) {
      ctx.pausePlaybackFromUi?.().catch(() => {});
    } else if (trackDetailDialog.isPaused()) {
      ctx
        .resumePlaybackFromUi?.()
        .then(() => trackDetailDialog.notePlaybackStarted())
        .catch(() => {});
    } else {
      // Nothing loaded here yet: start at the left edge of the visible window.
      playFromRatio(trackDetailDialog.viewRatioToTrackRatio(0));
    }
  });

  // Overview strip: press or drag to centre the view there (zoom kept).
  const overview = el.trackDetailOverview;
  let overviewDragging = false;
  const overviewTo = (event) =>
    trackDetailDialog.centerViewAtRatio(scrubRatioFromPointer(event, overview));
  overview?.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    beginDragNoSelect();
    overviewDragging = true;
    overview.setPointerCapture?.(event.pointerId);
    overview.classList.add("is-dragging");
    overviewTo(event);
  });
  overview?.addEventListener("pointermove", (event) => {
    if (overviewDragging) overviewTo(event);
  });
  const endOverviewDrag = () => {
    overviewDragging = false;
    overview?.classList.remove("is-dragging");
  };
  overview?.addEventListener("pointerup", endOverviewDrag);
  overview?.addEventListener("pointercancel", endOverviewDrag);

  el.trackDetailZoomIn?.addEventListener("click", () => trackDetailDialog.zoomAt(0.5, 0.5));
  el.trackDetailZoomOut?.addEventListener("click", () => trackDetailDialog.zoomAt(0.5, 2));
  el.trackDetailZoomFit?.addEventListener("click", () => trackDetailDialog.fitView());

  // Click a marker → scroll its row into view and play from that cue
  // (not when the click ends a marker drag).
  el.trackDetailCueMarkers?.addEventListener("click", (event) => {
    const marker = event.target.closest(".cue-marker");
    if (!marker || suppressMarkerClick) return;
    const row = el.trackDetailCueList?.querySelector(
      `.cue-row[data-temp-id="${marker.dataset.tempId}"]`
    );
    row?.scrollIntoView({ block: "nearest" });
    const cue = trackDetailDialog
      .getWorking()
      .cues.find((c) => c.tempId === marker.dataset.tempId);
    playFromCue(cue);
  });

  el.trackDetailAddCue?.addEventListener("click", () => {
    if (!trackDetailDialog.addCue()) {
      ctx.emitStatus?.("Maximum 8 cue points.");
    }
  });

  el.trackDetailFirstBeatMinus?.addEventListener("click", () =>
    trackDetailDialog.nudgeFirstBeat(-1)
  );
  el.trackDetailFirstBeatPlus?.addEventListener("click", () =>
    trackDetailDialog.nudgeFirstBeat(1)
  );
  el.trackDetailFirstBeatMs?.addEventListener("change", (event) => {
    trackDetailDialog.setFirstBeatMs(Number(event.target.value) || 0);
  });

  el.trackDetailGridLevel?.addEventListener("input", (event) =>
    trackDetailDialog.setBeatgridLevel(event.target.value)
  );
  el.trackDetailGridLevel?.addEventListener("change", (event) =>
    trackDetailDialog.setBeatgridLevel(event.target.value, { remember: true })
  );

  el.trackDetailStartFirstCue?.addEventListener("click", () =>
    trackDetailDialog.setStartOnFirstBeat(false, { remember: true })
  );
  el.trackDetailStartFirstBeat?.addEventListener("click", () =>
    trackDetailDialog.setStartOnFirstBeat(true, { remember: true })
  );

  el.trackDetailBpmMinus?.addEventListener("click", () =>
    trackDetailDialog.nudgeBpm(-1)
  );
  el.trackDetailBpmPlus?.addEventListener("click", () =>
    trackDetailDialog.nudgeBpm(1)
  );
  el.trackDetailBpm?.addEventListener("change", (event) => {
    trackDetailDialog.setBpm(Number.parseFloat(event.target.value));
  });

  el.trackDetailKeyMinus?.addEventListener("click", () =>
    trackDetailDialog.nudgeKey(-1)
  );
  el.trackDetailKeyPlus?.addEventListener("click", () =>
    trackDetailDialog.nudgeKey(1)
  );
  el.trackDetailKey?.addEventListener("change", (event) => {
    trackDetailDialog.setKey(event.target.value);
  });

  // Cue list: play / name / colour / delete (event-delegated).
  el.trackDetailCueList?.addEventListener("input", (event) => {
    const target = event.target.closest("[data-action='cue-name']");
    if (!target) return;
    const tempId = target.closest(".cue-row")?.dataset.tempId;
    if (tempId) trackDetailDialog.renameCue(tempId, target.value);
  });
  el.trackDetailCueList?.addEventListener("click", (event) => {
    const target = event.target.closest("[data-action]");
    if (!target) return;
    const tempId = target.closest(".cue-row")?.dataset.tempId;
    if (!tempId) return;
    if (target.dataset.action === "cue-delete") {
      trackDetailDialog.deleteCue(tempId);
    } else if (target.dataset.action === "cue-color") {
      openColorPopover(ctx, target, tempId);
    } else if (target.dataset.action === "cue-play") {
      playFromCue(trackDetailDialog.getWorking().cues.find((c) => c.tempId === tempId));
    }
  });
}

function openColorPopover(ctx, anchor, tempId) {
  const { el, trackDetailDialog } = ctx;
  const pop = el.trackDetailColorPopover;
  if (!pop) return;
  pop.textContent = "";
  for (const entry of HOTCUE_PALETTE) {
    const swatch = pop.ownerDocument.createElement("button");
    swatch.type = "button";
    swatch.className = "cue-color-swatch";
    swatch.style.background = entry.css;
    swatch.addEventListener("click", () => {
      trackDetailDialog.updateCue(tempId, { colorId: entry.id });
      pop.hidden = true;
    });
    pop.appendChild(swatch);
  }
  const overlayRect = el.trackDetailOverlay.getBoundingClientRect();
  const rect = anchor.getBoundingClientRect();
  pop.style.left = `${rect.left - overlayRect.left}px`;
  pop.style.top = `${rect.bottom - overlayRect.top + 4}px`;
  pop.hidden = false;

  const dismiss = (event) => {
    if (!pop.contains(event.target) && event.target !== anchor) {
      pop.hidden = true;
      document.removeEventListener("mousedown", dismiss, true);
    }
  };
  document.addEventListener("mousedown", dismiss, true);
}
