// Transcodes: extra files of a library track in another format (AIFF, WAV,
// MP3 320), for players that can't read FLAC or ALAC. The backend decides
// which selected tracks need one and keeps the list on each track
// (`track.transcodes`); this module only asks and shows.

import { counted } from "../../track_utils.mjs";

// The Convert… dialog. `open({ count, folder })` resolves with
// `{ action: "convert", format }`, `{ action: "remove" }`, or null on cancel.
export function createConvertDialogController(el, document) {
  let resolveFn = null;

  function close(result) {
    if (!resolveFn) return;
    el.convertOverlay.hidden = true;
    const resolve = resolveFn;
    resolveFn = null;
    resolve(result);
  }

  function chosenFormat() {
    return el.convertFormatGroup.querySelector("input[name='convertFormat']:checked")?.value || "mp3";
  }

  el.convertApplyBtn.addEventListener("click", () => close({ action: "convert", format: chosenFormat() }));
  el.convertRemoveBtn.addEventListener("click", () => close({ action: "remove" }));
  el.convertCancelBtn.addEventListener("click", () => close(null));
  el.convertOverlay.addEventListener("click", (event) => {
    if (event.target === el.convertOverlay) close(null);
  });
  document.addEventListener("keydown", (event) => {
    if (!resolveFn || event.key !== "Escape") return;
    event.preventDefault();
    close(null);
  });

  return {
    isOpen: () => !!resolveFn,
    close,
    open({ count = 0, folder = "" }) {
      if (resolveFn) close(null);
      el.convertTitle.textContent = `Convert ${counted(count, "track")}`;
      el.convertFolderText.textContent = folder;
      el.convertOverlay.hidden = false;
      el.convertApplyBtn.focus();
      return new Promise((resolve) => {
        resolveFn = resolve;
      });
    },
  };
}

// Shows each returned track in every table holding it. A browse-only row
// (`id` is its file path) that the backend indexed comes back with its new id.
function showTracks(ctx, ids, tracks) {
  const asked = new Set(ids.map(String));
  for (const track of tracks || []) {
    const replaces = asked.has(String(track.filePath)) ? track.filePath : null;
    ctx.showTrackRow(track, { replaces });
  }
}

// The selection bars' Convert…: ask for the format, then run the job
// (progress and cancel come through `job:event`), or remove the transcodes.
export async function convertTrackSelection(ctx, ids) {
  const trackIds = (ids || []).filter(Boolean);
  if (!trackIds.length) return;
  const folder = await ctx.command("get_transcode_folder");
  const choice = await ctx.convertDialog.open({ count: trackIds.length, folder: folder?.path || "" });
  if (!choice) return;

  if (choice.action === "remove") {
    const result = await ctx.command("remove_track_transcodes", { trackIds });
    showTracks(ctx, trackIds, result.tracks);
    ctx.emitStatus(`Removed ${counted(result.removed, "transcode")}`);
    return;
  }

  const result = await ctx.command("transcode_tracks", { trackIds, format: choice.format });
  showTracks(ctx, trackIds, result.tracks);
  for (const message of result.messages || []) {
    ctx.pushEventLog({
      level: "info",
      source: "transcode",
      code: "transcode.result",
      message,
    });
  }
  const parts = [`${counted(result.transcoded, "track")} converted`];
  if (result.skipped) parts.push(`${result.skipped} skipped`);
  if (result.failed) parts.push(`${result.failed} failed`);
  ctx.emitStatus(`${result.cancelled ? "Converting cancelled" : "Converting done"}: ${parts.join(", ")}`);
}
