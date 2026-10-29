// The selection bar's Add to…: a dialog listing every playlist with how much
// of the selection it already holds (backend-owned: get_playlist_membership),
// plus a field to create a new playlist. The ticked playlists all get the
// selection in one call, which resolves the tracks once.

import { cloneTemplate } from "../../ui_utils.mjs";
import { applyAddResolutions, toAddTrackCandidatePayload } from "../playlist/actions.mjs";

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
        emitStatus(`Could not create playlist: ${err.message || err}`);
        return null;
      }
    },
  });
  if (!playlistIds?.length) return;

  const result = await ctx.withProgress("Adding tracks", async (progress) => {
    progress(30, "Adding tracks...");
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
    emitStatus("No imported track IDs found to add");
    return;
  }
  // The add cleared the playlists' "exported to USB" marks.
  await ctx.loadPlaylists();
  if (playlistIds.includes(state.currentPlaylistId)) await ctx.refreshCurrentPlaylistTracks();
  const names = playlistIds
    .map((id) => state.playlists.find((playlist) => playlist.id === id)?.name)
    .filter(Boolean)
    .join(", ");
  ctx.pushEventLog({
    level: "info",
    source: "playlist-add",
    code: "playlist_add.result",
    message: `Added ${result.added} track(s) to ${names}`,
    details: `requested=${result.requested} | resolved=${result.resolved} | added=${result.added} | skipped=${result.skipped}`,
  });
  emitStatus(`Added ${result.added} tracks (skipped ${result.skipped}) to ${names}`);
}

// The Add to… dialog. `open({ count, load, create })` lists what `load()`
// returns (the membership data); `create(name)` makes a playlist and returns
// its id, which is then ticked. Resolves with the ticked playlist ids, or
// null on cancel. A playlist holding the whole selection can't be ticked;
// one holding part of it shows as partly ticked until it is.
export function createAddToDialogController(el, document) {
  let resolveFn = null;
  let request = null;
  let checked = new Set();

  function sync() {
    el.addToApplyBtn.disabled = checked.size === 0;
    el.addToCreateBtn.disabled = !el.addToNewName.value.trim();
  }

  function render(data) {
    const selected = Number(data?.selected || 0);
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
      row.querySelector(".add-to-note").textContent = all
        ? "Already in"
        : item.state === "some" ? `${item.contained} of ${selected} in` : "";
      return row;
    });
    if (!rows.length) {
      const empty = cloneTemplate(document, "tplMutedListItem");
      empty.textContent = "No playlists yet: create one below.";
      rows.push(empty);
    }
    el.addToList.replaceChildren(...rows);
    sync();
  }

  function close(result) {
    if (!resolveFn) return;
    el.addToOverlay.hidden = true;
    const resolve = resolveFn;
    resolveFn = null;
    request = null;
    resolve(result);
  }

  async function createNew() {
    const name = el.addToNewName.value.trim();
    if (!name || !request) return;
    el.addToCreateBtn.disabled = true;
    const { load, create } = request;
    const id = await create(name);
    if (!id || !resolveFn) {
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
  el.addToOverlay.addEventListener("click", (event) => {
    if (event.target === el.addToOverlay) close(null);
  });
  document.addEventListener("keydown", (event) => {
    if (!resolveFn) return;
    if (event.key === "Escape") {
      event.preventDefault();
      close(null);
    } else if (event.key === "Enter" && event.target === el.addToNewName) {
      event.preventDefault();
      createNew();
    }
  });

  return {
    isOpen: () => !!resolveFn,
    close,
    async open({ count = 0, load, create }) {
      if (resolveFn) close(null);
      checked = new Set();
      el.addToNewName.value = "";
      el.addToTitle.textContent = `Add ${count} track${count === 1 ? "" : "s"} to playlists`;
      render(await load());
      request = { load, create };
      el.addToOverlay.hidden = false;
      (el.addToList.querySelector("input:not(:disabled)") || el.addToNewName).focus();
      return new Promise((resolve) => {
        resolveFn = resolve;
      });
    },
  };
}
