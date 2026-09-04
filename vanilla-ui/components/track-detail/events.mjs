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
  overlay.addEventListener("mousedown", (event) => {
    if (event.target === overlay) close();
  });
  overlay.addEventListener("keydown", (event) => {
    if (event.key === "Escape") close();
  });

  el.trackDetailSaveBtn?.addEventListener("click", () => {
    stopIfOwned();
    trackDetailDialog.close(trackDetailDialog.toSavePayload());
  });

  // --- Waveform: click to play, double-click to add a cue, wheel to zoom, drag to pan ---
  const wf = el.trackDetailWaveform;
  const PAN_THRESHOLD_PX = 4;
  let pan = null; // { startX, startViewMs, moved }
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

  wf?.addEventListener("pointerdown", (event) => {
    if (event.button !== 0 || event.target.closest(".cue-marker")) return;
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

  el.trackDetailZoomIn?.addEventListener("click", () => trackDetailDialog.zoomAt(0.5, 0.5));
  el.trackDetailZoomOut?.addEventListener("click", () => trackDetailDialog.zoomAt(0.5, 2));
  el.trackDetailZoomFit?.addEventListener("click", () => trackDetailDialog.fitView());

  // Click a marker → scroll its row into view and play from that cue.
  el.trackDetailCueMarkers?.addEventListener("click", (event) => {
    const marker = event.target.closest(".cue-marker");
    if (!marker) return;
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
