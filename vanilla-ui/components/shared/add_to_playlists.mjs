// The selection bar's Add to…: a dialog listing every playlist with how much
// of the selection it already holds (backend-owned: get_playlist_membership),
// plus a field to create a new playlist. The ticked playlists all get the
// selection in one call, which resolves the tracks once.

import { t, tCount } from "../../ui_texts.mjs";
import { cloneTemplate } from "../../ui_utils.mjs";
import { createDialogLifecycle } from "./dialog_lifecycle.mjs";
import { addTracksProgressTexts, applyAddResolutions, toAddTrackCandidatePayload } from "../playlist/actions.mjs";

const USB_VIEWS = new Set(["usbPlaylist", "usbHistory"]);

export async function addSelectionToPlaylists(ctx, view, ids, rows) {
  const { state, command, emitStatus } = ctx;
  const usb = USB_VIEWS.has(view);
  const count = usb ? rows.length : ids.length;
  // One entry per selected track: its local id. A USB row carries the one
  // the backend matched it to, or none before it was ever added.
  const membershipIds = usb ? rows.map((row) => row.localTrackId || null) : ids;
  const playlistIds = await ctx.addToDialog.open({
    count,
    load: () => command("get_playlist_membership", { trackIds: membershipIds }),
    create: async (name) => {
      try {
        const created = await command("create_playlist", { name });
        await ctx.loadPlaylists();
        return created?.playlistId || null;
      } catch (err) {
        emitStatus(t("ui-add-to-create-failed", { error: err.message || err }));
        return null;
      }
    },
  });
  if (!playlistIds?.length) return;

  const result = await ctx.withProgress(addTracksProgressTexts(), async (progress) => {
    progress(30, t("ui-playlist-add-adding"));
    if (view === "library") {
      return command("add_library_selection_to_playlists", {
        playlistIds,
        query: String(state.libraryQuery || "").trim(),
        trackIds: ids,
      });
    }
    const add = await command("add_track_candidates_to_playlists", {
      playlistIds,
      // An app playlist's tracks are local already: their ids are enough.
      tracks: usb ? rows.map(toAddTrackCandidatePayload) : ids.map((trackId) => ({ trackId })),
      usbRoot: state.usbRoot || null,
      usbRootValid: !!state.usbRootValid,
    });
    if (usb) applyAddResolutions(ctx, rows, add.resolutions);
    return add;
  });

  if (!Number(result?.resolved || 0)) {
    emitStatus(t("ui-playlist-add-none-resolved"));
    return;
  }
  // The add cleared the playlists' "exported to USB" marks.
  await ctx.loadPlaylists();
  if (playlistIds.includes(state.currentPlaylistId)) await ctx.refreshCurrentPlaylistTracks();
  ctx.pushEventLog({
    level: "info",
    source: "playlist-add",
    code: "playlist_add.result",
    message: result.statusText,
    details: `requested=${result.requested} | resolved=${result.resolved} | added=${result.added} | skipped=${result.skipped}`,
  });
  emitStatus(result.statusText);
}

// The Add to… dialog. `open({ count, load, create })` lists what `load()`
// returns (the membership data); `create(name)` makes a playlist and returns
// its id, which is then ticked. Resolves with the ticked playlist ids, or
// null on cancel. A playlist holding the whole selection can't be ticked;
// one holding part of it shows as partly ticked until it is.
export function createAddToDialogController(el, document) {
  let request = null;
  let checked = new Set();
  const dialog = createDialogLifecycle(el.addToOverlay, { onClose: () => { request = null; } });
  const { close } = dialog;

  function sync() {
    el.addToApplyBtn.disabled = checked.size === 0;
    el.addToCreateBtn.disabled = !el.addToNewName.value.trim();
  }

  function render(data) {
    const items = data?.items || [];
    const rows = items.map((item) => {
      const row = cloneTemplate(document, "tplAddToItem");
      const box = row.querySelector("input");
      const all = item.state === "all";
      box.value = item.playlistId;
      box.checked = all || checked.has(item.playlistId);
      box.disabled = all;
      box.indeterminate = item.state === "some" && !checked.has(item.playlistId);
      row.querySelector(".add-to-name").textContent = item.name;
      row.querySelector(".add-to-note").textContent = item.noteText || "";
      return row;
    });
    if (!rows.length) {
      const empty = cloneTemplate(document, "tplMutedListItem");
      empty.textContent = t("ui-add-to-empty");
      rows.push(empty);
    }
    el.addToList.replaceChildren(...rows);
    sync();
  }

  async function createNew() {
    const name = el.addToNewName.value.trim();
    if (!name || !request) return;
    el.addToCreateBtn.disabled = true;
    const { load, create } = request;
    const id = await create(name);
    if (!id || !dialog.isOpen()) {
      sync();
      return;
    }
    el.addToNewName.value = "";
    checked.add(id);
    render(await load());
  }

  el.addToList.addEventListener("change", (event) => {
    const box = event.target.closest?.("input[type='checkbox']");
    if (!box) return;
    if (box.checked) checked.add(box.value);
    else checked.delete(box.value);
    sync();
  });
  el.addToNewName.addEventListener("input", sync);
  el.addToCreateBtn.addEventListener("click", () => createNew());
  // In list order.
  el.addToApplyBtn.addEventListener("click", () => close(
    [...el.addToList.querySelectorAll("input:checked:not(:disabled)")].map((box) => box.value),
  ));
  el.addToCancelBtn.addEventListener("click", () => close(null));
  document.addEventListener("keydown", (event) => {
    if (!dialog.isOpen()) return;
    if (event.key === "Enter" && event.target === el.addToNewName) {
      event.preventDefault();
      createNew();
    }
  });

  return {
    isOpen: dialog.isOpen,
    close,
    async open({ count = 0, load, create }) {
      close();
      checked = new Set();
      el.addToNewName.value = "";
      el.addToTitle.textContent = tCount("ui-add-to-title-count", count);
      render(await load());
      request = { load, create };
      return dialog.open(el.addToList.querySelector("input:not(:disabled)") || el.addToNewName);
    },
  };
}
