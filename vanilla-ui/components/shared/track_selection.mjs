// Track selection, shared by the four track-list views (library, app
// playlist, USB playlist, USB history): the row checkboxes, the select-all
// box in the header and the selection bar beside the search.
//
// The bar is always shown, so the table never moves when a selection starts
// or ends; until something is selected it shows only a hint (index.html,
// styled by `is-empty`) and its actions stay disabled. Each view
// has its own selection (`state.trackSelections[view]`: ticked ids, plus the
// rows by id for views whose actions need the row itself) and its own set of
// actions -- the bar's buttons in index.html say which.
//
// Beside the count, the ticked tracks' total time, summed by the backend
// (`get_selection_duration`) by the same rule as the list's "Total time".

import { catchErr } from "./track_actions.mjs";
import { formatDurationMs } from "../../track_utils.mjs";

// Per app context and view: the view's `durationRequest` and the last request
// sent, so an unchanged selection isn't asked for again and a late answer for
// an older selection is dropped.
const selectionTimes = new WeakMap();

function selectionTimeOf(ctx, view) {
  if (!selectionTimes.has(ctx)) selectionTimes.set(ctx, {});
  const views = selectionTimes.get(ctx);
  views[view] ||= { request: null, sent: "" };
  return views[view];
}

export function trackSelection(ctx, view) {
  return ctx.state.trackSelections[view];
}

function barOf(ctx, view) {
  return ctx.document.querySelector(`[data-selection-bar="${view}"]`);
}

function selectAllBoxOf(ctx, view) {
  return ctx.document.querySelector(`[data-select-all="${view}"]`);
}

// Count, button states and the header box, from the view's selection.
export function renderSelectionBar(ctx, view) {
  const bar = barOf(ctx, view);
  if (!bar) return;
  const { ids } = trackSelection(ctx, view);
  const count = ids.size;
  bar.classList.toggle("is-empty", count === 0);
  bar.querySelector(".selection-count").textContent = `${count} selected`;
  renderSelectionTime(ctx, view, bar, ids);
  for (const button of bar.querySelectorAll("[data-selection-action]")) {
    button.disabled = count === 0;
  }
  // "+ Add to ‹active playlist›" names its target; with no active playlist
  // it's gone, and Add to… picks one.
  const add = bar.querySelector('[data-selection-action="add"]');
  if (add) {
    const playlist = ctx.getCurrentPlaylist();
    add.hidden = !playlist;
    add.querySelector(".selection-target").textContent = playlist?.name || "";
    add.dataset.tooltip = playlist ? `Add the selected tracks to ${playlist.name}` : "";
  }
  const box = selectAllBoxOf(ctx, view);
  if (box) {
    const total = Number(barCtl(ctx, view)?.total || 0);
    box.checked = count > 0 && count >= total;
    box.indeterminate = count > 0 && count < total;
  }
}

// "· 52:14" once the backend has summed the selection; "+" and a tooltip when
// some ticked tracks have no length yet. Blank while asking, or when none of
// them has a length.
function renderSelectionTime(ctx, view, bar, ids) {
  const target = bar.querySelector(".selection-time");
  const time = selectionTimeOf(ctx, view);
  const args = ids.size && time.request ? time.request() : null;
  const key = args ? JSON.stringify({ ...args, trackIds: [...ids] }) : "";
  if (key === time.sent) return;
  time.sent = key;
  if (!target) return;
  target.textContent = "";
  delete target.dataset.tooltip;
  if (!args) return;
  ctx
    .command("get_selection_duration", { ...args, trackIds: [...ids] })
    .then((data) => {
      if (time.sent !== key) return;
      const known = Number(data?.durationKnownCount) || 0;
      if (!known) return;
      const unknown = Math.max(0, (Number(data?.trackCount) || 0) - known);
      target.textContent = `· ${formatDurationMs(data.totalDurationMs)}${unknown ? "+" : ""}`;
      if (unknown) target.dataset.tooltip = `${unknown} of the selected tracks have no length yet`;
    })
    .catch(() => {});
}

export function renderAllSelectionBars(ctx) {
  for (const view of Object.keys(ctx.state.trackSelections)) renderSelectionBar(ctx, view);
}

function barCtl(ctx, view) {
  return {
    library: ctx.libraryTracksCtl,
    playlist: ctx.playlistTracksCtl,
    usbPlaylist: ctx.usbPlaylistTracksCtl,
    usbHistory: ctx.usbHistoryTracksCtl,
  }[view];
}

async function rerender(ctx, view) {
  await barCtl(ctx, view)?.rerender();
  renderSelectionBar(ctx, view);
}

export async function clearTrackSelection(ctx, view) {
  const selection = trackSelection(ctx, view);
  selection.ids = new Set();
  selection.rows = new Map();
  await rerender(ctx, view);
}

// A fresh first page (new list, search or sort) narrows the selection to the
// tracks it shows: changing the list is how the user scopes the selection.
// "Select all" only re-renders, so its larger-than-a-page selection stays
// until the list itself changes.
export function narrowTrackSelectionToLoaded(ctx, view, loaded) {
  const selection = trackSelection(ctx, view);
  if (!selection.ids.size) return;
  const loadedIds = new Set(loaded.filter(Boolean).map((track) => String(track.id)));
  selection.ids = new Set([...selection.ids].filter((id) => loadedIds.has(id)));
  selection.rows = new Map([...selection.rows].filter(([id]) => loadedIds.has(id)));
  renderSelectionBar(ctx, view);
}

// Wire a view's checkboxes, header box and bar.
//   body:       the rows' container (row checkboxes carry `data-id`)
//   ctl:        the view's TrackListController (rows resolve against `view`)
//   selectAll:  async () => tracks (or ids) -- every track the list shows
//               under its current search, not just the loaded pages
//   actions:    { add, "add-to", remove, reanalyze, edit }
//               -- async (ids, rows) => void
//   durationRequest: () => the `get_selection_duration` request naming this
//               view's list (its `view` and list fields, not the ids), or
//               null while there is no list
export function bindTrackSelection(ctx, { view, body, ctl, selectAll, actions, durationRequest }) {
  const { emitStatus } = ctx;
  selectionTimeOf(ctx, view).request = durationRequest || null;

  body?.addEventListener("change", (event) => {
    const box = event.target.closest?.(".td-select input[type='checkbox']");
    if (!box) return;
    const id = String(box.dataset.id || "");
    if (!id) return;
    const selection = trackSelection(ctx, view);
    if (box.checked) {
      selection.ids.add(id);
      const index = Number(box.closest(".track-grid-row")?.dataset?.trackIndex);
      const track = ctl.view[index];
      if (track) selection.rows.set(id, track);
    } else {
      selection.ids.delete(id);
      selection.rows.delete(id);
    }
    renderSelectionBar(ctx, view);
  });

  const box = selectAllBoxOf(ctx, view);
  box?.addEventListener("change", () => {
    if (!box.checked) {
      clearTrackSelection(ctx, view).catch(catchErr(emitStatus));
      return;
    }
    (async () => {
      const all = await ctx.withProgress("Selecting all matching tracks", async (progress) => {
        progress(40, "Enumerating tracks...");
        return selectAll();
      });
      if (!box.checked) return;
      const selection = trackSelection(ctx, view);
      selection.ids = new Set();
      selection.rows = new Map();
      for (const item of all || []) {
        const id = String(typeof item === "object" ? item?.id ?? "" : item ?? "");
        if (!id) continue;
        selection.ids.add(id);
        if (typeof item === "object") selection.rows.set(id, item);
      }
      await rerender(ctx, view);
    })().catch(catchErr(emitStatus));
  });

  barOf(ctx, view)?.addEventListener("click", (event) => {
    const button = event.target.closest("[data-selection-action]");
    if (!button || button.disabled) return;
    const name = button.dataset.selectionAction;
    if (name === "clear") {
      clearTrackSelection(ctx, view).catch(catchErr(emitStatus));
      return;
    }
    const action = actions[name];
    if (!action) return;
    const selection = trackSelection(ctx, view);
    // In list order where the rows are loaded (an add keeps the list's
    // order), then the rest in the order they were ticked.
    const position = new Map();
    ctl.view.forEach((track, index) => {
      if (track) position.set(String(track.id), index);
    });
    const ids = [...selection.ids].sort(
      (a, b) => (position.get(a) ?? Infinity) - (position.get(b) ?? Infinity),
    );
    const rows = ids.map((id) => selection.rows.get(id)).filter(Boolean);
    Promise.resolve(action(ids, rows)).catch(catchErr(emitStatus));
  });

  renderSelectionBar(ctx, view);
}
