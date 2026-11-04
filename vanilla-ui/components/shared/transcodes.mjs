// Transcodes: extra files of a library track in another format (AIFF, WAV,
// MP3 320), for players that can't read FLAC or ALAC. The backend decides
// which selected tracks need one and keeps the list on each track
// (`track.transcodes`); this module only asks and shows.

import { counted } from "../../track_utils.mjs";
import { cloneTemplate } from "../../ui_utils.mjs";
import { createDialogLifecycle } from "./dialog_lifecycle.mjs";

// The Convert… dialog. `open({ count, folder, formats })` (the backend's
// get_transcode_formats choices) resolves with
// `{ action: "convert", format }`, `{ action: "remove" }`, or null on cancel.
export function createConvertDialogController(el, document) {
  const dialog = createDialogLifecycle(el.convertOverlay);
  const { close } = dialog;

  function chosenFormat() {
    return el.convertFormatGroup.querySelector("input[name='convertFormat']:checked")?.value || "";
  }

  el.convertApplyBtn.addEventListener("click", () => close({ action: "convert", format: chosenFormat() }));
  el.convertRemoveBtn.addEventListener("click", () => close({ action: "remove" }));
  el.convertCancelBtn.addEventListener("click", () => close(null));

  return {
    isOpen: dialog.isOpen,
    close,
    open({ count = 0, folder = "", formats = [] }) {
      close();
      el.convertTitle.textContent = `Convert ${counted(count, "track")}`;
      el.convertFolderText.textContent = folder;
      el.convertFormatGroup.querySelectorAll("label").forEach((row) => row.remove());
      for (const choice of formats) {
        const row = cloneTemplate(document, "tplConvertFormatChoice");
        const input = row.querySelector("input");
        input.value = choice.format;
        input.checked = !!choice.isDefault;
        row.querySelector("span").textContent = choice.label;
        el.convertFormatGroup.appendChild(row);
      }
      return dialog.open(el.convertApplyBtn);
    },
  };
}

// Shows each returned track in every table holding it, in place of the row
// the backend says it replaces (a listed file it indexed: `replaced`).
function showTracks(ctx, tracks, replaced = []) {
  const previous = new Map(replaced.map((row) => [row.trackId, row.previousId]));
  for (const track of tracks || []) {
    ctx.showTrackRow(track, { replaces: previous.get(track.id) || null });
  }
}

// The selection bars' Convert…: ask for the format, then run the job
// (progress and cancel come through `job:event`), or remove the transcodes.
export async function convertTrackSelection(ctx, ids) {
  const trackIds = (ids || []).filter(Boolean);
  if (!trackIds.length) return;
  const [folder, offered] = await Promise.all([
    ctx.command("get_transcode_folder"),
    ctx.command("get_transcode_formats"),
  ]);
  const choice = await ctx.convertDialog.open({
    count: trackIds.length,
    folder: folder?.path || "",
    formats: offered?.formats || [],
  });
  if (!choice) return;

  if (choice.action === "remove") {
    const result = await ctx.command("remove_track_transcodes", { trackIds });
    showTracks(ctx, result.tracks);
    // What export puts on the USB may have changed.
    ctx.reloadPlaylistForUsb();
    ctx.emitStatus(result.summary);
    return;
  }

  const result = await ctx.command("transcode_tracks", { trackIds, format: choice.format });
  showTracks(ctx, result.tracks, result.replaced);
  ctx.reloadPlaylistForUsb();
  // Skipped and failed tracks are in the Event Log already (the backend's).
  ctx.emitStatus(result.summary);
}
