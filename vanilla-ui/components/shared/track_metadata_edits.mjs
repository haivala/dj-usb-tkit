// Genre / rating / release-date saves -- the cue editor's (one track) and
// the selection bar's Edit (many) -- and the track-table updates after them.
// Library and app-playlist tracks save by local id; USB playlist / history
// rows save to the stick (and the matched library tracks) by their media
// path. Each save answers with the saved tracks' whole rows, which the tables
// show as they are.

import { t, tCount } from "../../ui_texts.mjs";
import { createDialogLifecycle } from "./dialog_lifecycle.mjs";
import { cloneTemplate } from "../../ui_utils.mjs";

// The 1-5 star rating the cue editor and the selection bar's Edit share:
// `mountRatingStars` fills an empty radiogroup with the stars,
// `renderRatingStars` lights the stars up to `rating` (0 = none) and
// `toggledRating` is the rating a star click sets -- the current rating's own
// star clears it.
export function mountRatingStars(container) {
  container.replaceChildren(...cloneTemplate(container.ownerDocument, "tplRatingStars").children);
}

export function renderRatingStars(container, rating) {
  for (const star of container.querySelectorAll("[data-rating]")) {
    const n = Number(star.dataset.rating);
    star.classList.toggle("is-on", n <= rating);
    star.setAttribute("aria-checked", String(n === rating));
  }
}

export function toggledRating(current, clicked) {
  const n = Math.max(0, Math.min(5, Math.round(Number(clicked) || 0)));
  return n === current ? 0 : n;
}

// The shape of a release date the editors send: YYYY, YYYY-MM or YYYY-MM-DD
// (month and day may have one digit). The backend checks it is a real date
// in range.
export const RELEASE_DATE_PATTERN = /^\d{4}(-\d{1,2}(-\d{1,2})?)?$/;

// A USB row as the save takes it: the on-stick path plus the row's library
// hints, as the cue editor has always sent them.
function usbEditTrack(row) {
  return {
    usbMediaPathRaw: row.usbMediaPath,
    localTrackId: row.localTrackId || null,
    title: row.title || null,
    artist: row.artist || null,
    album: row.album || null,
  };
}

// `edits`: { genre?, rating?, releaseDate? } -- only the fields to change;
// an empty genre or release date clears it.
export async function saveLibraryMetadataEdits(ctx, trackIds, edits) {
  const saved = await ctx.command("save_track_metadata_edits", { trackIds, ...edits });
  for (const track of saved.tracks) ctx.showTrackRow(track);
  return saved;
}

export async function saveUsbMetadataEdits(ctx, rows, edits) {
  // A running job blocks USB changes (an export, diagnostics, another save):
  // wait for it instead of dropping the edits.
  if (ctx.actionBlockedReason("usb")) {
    ctx.emitStatus(t("ui-group-edit-waiting-for-usb-job"));
    await ctx.whenActionUnblocked("usb");
  }
  const saved = await ctx.command("save_usb_track_metadata_edits", {
    usbRoot: ctx.state.usbRoot,
    tracks: rows.map(usbEditTrack),
    ...edits,
  });
  // Every loaded row of these files, and the library tracks also written.
  for (const track of [...saved.tracks, ...saved.localTracks]) ctx.showTrackRow(track);
  return saved;
}

// The selection bar's Edit: what the selected tracks share (backend-owned,
// over the whole selection, not just loaded rows), the dialog, then one save
// for all of them.
export async function editTrackSelection(ctx, view, ids, rows) {
  const usb = view === "usbPlaylist" || view === "usbHistory";
  const summary = usb
    ? await ctx.command("get_usb_tracks_metadata_summary", {
      usbRoot: ctx.state.usbRoot,
      usbMediaPathsRaw: rows.map((row) => row.usbMediaPath).filter(Boolean),
    })
    : await ctx.command("get_tracks_metadata_summary", { trackIds: ids });
  const edits = await ctx.groupEditDialog.open(summary);
  if (!edits || !Object.keys(edits).length) return;
  try {
    const saved = usb
      ? await saveUsbMetadataEdits(ctx, rows, edits)
      : await saveLibraryMetadataEdits(ctx, ids, edits);
    ctx.emitStatus(saved.statusText);
  } catch (err) {
    ctx.emitStatus(err?.message || String(err));
  }
}

// The Edit dialog. `open(summary)` shows each field's shared value, or
// "Mixed"; it resolves with only the fields the user changed (`{ genre?,
// releaseDate?, rating? }`; an empty genre / release date clears it), or null
// on cancel.
export function createGroupEditDialogController(el, document) {
  const dialog = createDialogLifecycle(el.groupEditOverlay);
  const { close } = dialog;
  let touched = new Set();
  let rating = 0;

  mountRatingStars(el.groupEditRating);
  const renderRating = () => renderRatingStars(el.groupEditRating, rating);

  function apply() {
    const edits = {};
    if (touched.has("genre")) edits.genre = el.groupEditGenre.value.trim();
    if (touched.has("releaseDate")) {
      const text = el.groupEditReleaseDate.value.trim();
      if (text && !RELEASE_DATE_PATTERN.test(text)) {
        el.groupEditReleaseDate.setAttribute("aria-invalid", "true");
        el.groupEditReleaseDate.focus();
        return;
      }
      edits.releaseDate = text;
    }
    if (touched.has("rating")) edits.rating = rating;
    close(edits);
  }

  el.groupEditGenre.addEventListener("input", () => touched.add("genre"));
  el.groupEditReleaseDate.addEventListener("input", () => {
    touched.add("releaseDate");
    el.groupEditReleaseDate.removeAttribute("aria-invalid");
  });
  el.groupEditRating.addEventListener("click", (event) => {
    const star = event.target.closest("[data-rating]");
    if (!star) return;
    rating = toggledRating(rating, star.dataset.rating);
    touched.add("rating");
    el.groupEditRatingMixed.hidden = true;
    renderRating();
  });
  el.groupEditApplyBtn.addEventListener("click", apply);
  el.groupEditCancelBtn.addEventListener("click", () => close(null));
  document.addEventListener("keydown", (event) => {
    if (!dialog.isOpen()) return;
    if (event.key === "Enter" && event.target?.tagName === "INPUT") {
      event.preventDefault();
      apply();
    }
  });

  return {
    isOpen: dialog.isOpen,
    close,
    open(summary) {
      close();
      touched = new Set();
      const count = Number(summary?.count || 0);
      el.groupEditTitle.textContent = tCount("ui-group-edit-title-count", count);
      const list = el.groupEditGenreOptions;
      const { Option } = list.ownerDocument.defaultView;
      list.replaceChildren(...(summary?.genreOptions || []).map((name) => new Option(name)));
      const field = (input, shared, emptyHint = "") => {
        input.value = shared?.mixed ? "" : String(shared?.value ?? "");
        input.placeholder = shared?.mixed ? t("ui-group-edit-mixed") : emptyHint;
        input.removeAttribute("aria-invalid");
      };
      field(el.groupEditGenre, summary?.genre);
      field(el.groupEditReleaseDate, summary?.releaseDate, t("ui-group-edit-release-date-placeholder"));
      rating = summary?.rating?.mixed ? 0 : Number(summary?.rating?.value || 0);
      el.groupEditRatingMixed.hidden = !summary?.rating?.mixed;
      renderRating();
      return dialog.open(el.groupEditGenre);
    },
  };
}
