// The Export playlists page: every app playlist, in the app's order,
// to export to the connected USB as one job, in a table like the track
// lists' (a row per playlist, the select-all box in the header). The backend
// says what each playlist holds, what it is on the stick and whether it can
// be exported (`get_batch_export_overview`); the page keeps only the selection.
import { t, tCount } from "../../ui_texts.mjs";
import { cloneTemplate } from "../../ui_utils.mjs";
import { exportPlaylistsToUsb, isUsbRootChangeBlocked } from "./actions.mjs";

function page(ctx) {
  ctx.state.exportPlaylistsPage ??= { overview: null, selected: new Set(), spaceSeq: 0, space: null, spaceTimer: null };
  return ctx.state.exportPlaylistsPage;
}

function hasUsb(state) {
  return !!state.usbRoot && !!state.usbRootValid;
}

function selectableRows(overview) {
  return (overview?.rows || []).filter((row) => !row.disabledReason || row.needsAnalysis);
}

function allSelected(ctx) {
  const { overview, selected } = page(ctx);
  const rows = selectableRows(overview);
  return rows.length > 0 && rows.every((row) => selected.has(row.playlistId));
}

// Opened from the sidebar: everything that can be exported is chosen.
export async function openExportPlaylistsPage(ctx) {
  const { state } = ctx;
  const current = page(ctx);
  current.overview = null;
  current.selected = new Set();
  current.space = null;
  if (hasUsb(state)) {
    try {
      current.overview = await ctx.command("get_batch_export_overview", { usbRoot: state.usbRoot });
    } catch (err) {
      ctx.emitStatus(t("ui-export-playlists-load-failed", { error: err?.message || String(err) }));
    }
    current.selected = new Set(selectableRows(current.overview).map((row) => row.playlistId));
  }
  renderExportPlaylistsPage(ctx);
  refreshExportPlaylistsSpace(ctx);
}

// After an analysis: the overview again, keeping the choice and adding the
// playlists it made exportable.
export async function refreshExportPlaylistsPage(ctx) {
  const { state } = ctx;
  const current = page(ctx);
  if (!hasUsb(state) || !current.overview) return;
  const wasSelectable = new Set(selectableRows(current.overview).map((row) => row.playlistId));
  let overview;
  try {
    overview = await ctx.command("get_batch_export_overview", { usbRoot: state.usbRoot });
  } catch (err) {
    ctx.emitStatus(t("ui-export-playlists-load-failed", { error: err?.message || String(err) }));
    return;
  }
  current.overview = overview;
  current.selected = new Set(selectableRows(overview)
    .map((row) => row.playlistId)
    .filter((id) => current.selected.has(id) || !wasSelectable.has(id)));
  renderExportPlaylistsPage(ctx);
  refreshExportPlaylistsSpace(ctx);
}

// Analyze Missing Tracks: the backend picks the tracks of the page's
// selected playlists that still need analysis.
export async function analyzeExportPlaylistsMissing(ctx) {
  const { overview, selected } = page(ctx);
  const playlistIds = (overview?.rows || []).filter((row) => selected.has(row.playlistId)).map((row) => row.playlistId);
  if (playlistIds.length === 0) return;
  await ctx.analyzeTrackIds([], { playlistIds });
}

export function renderExportPlaylistsPage(ctx) {
  const { el, document } = ctx;
  const { overview, selected } = page(ctx);
  const rows = overview?.rows || [];
  el.exportPlaylistsTitle.textContent = overview?.usbName
    ? t("ui-export-playlists-title-usb", { name: overview.usbName })
    : t("ui-export-playlists-title");
  el.exportPlaylistsEmpty.classList.toggle("hidden", rows.length > 0);
  el.exportPlaylistsList.replaceChildren(...rows.map((row, index) => {
    const item = cloneTemplate(document, "tplExportPlaylistsRow");
    item.dataset.playlistId = row.playlistId;
    item.dataset.status = row.status;
    item.classList.toggle("is-alt", index % 2 === 1);
    item.classList.toggle("is-disabled", !!row.disabledReason && !row.needsAnalysis);
    const check = item.querySelector(".export-playlists-check");
    check.value = row.playlistId;
    check.checked = selected.has(row.playlistId);
    check.disabled = !!row.disabledReason && !row.needsAnalysis;
    const cells = {
      ".export-playlists-name": row.name,
      ".export-playlists-tracks": String(row.trackCount),
      ".export-playlists-length": row.durationText,
      ".export-playlists-size": row.sizeText,
      ".export-playlists-copy": row.copyText,
      ".export-playlists-status": row.disabledReason || row.statusText,
      ".export-playlists-last": row.lastExportedText,
    };
    for (const [selector, text] of Object.entries(cells)) item.querySelector(selector).textContent = text;
    item.querySelector(".export-playlists-name").dataset.tooltip = row.name;
    if (row.disabledReason) item.dataset.tooltip = row.disabledReason;
    return item;
  }));
  el.exportPlaylistsForceTrackOrder.checked = false;
  el.exportPlaylistsForceOrder.checked = false;
  renderExportPlaylistsControls(ctx);
}

// Everything the selection drives: the count, the ticks, the warning, the
// size line and the export button.
export function renderExportPlaylistsControls(ctx) {
  const { el, state } = ctx;
  const { overview, selected, space } = page(ctx);
  const needsAnalysis = (overview?.rows || []).some((row) => selected.has(row.playlistId) && row.needsAnalysis);
  el.exportPlaylistsAnalyzeBtn.hidden = !needsAnalysis;
  el.exportPlaylistsRunBtn.hidden = needsAnalysis;
  el.exportPlaylistsAnalyzeBtn.textContent = t("ui-playlist-analyze-missing");
  el.exportPlaylistsAnalyzeBtn.dataset.tooltip = overview?.analyzeMissingTooltip || "";
  const total = selectableRows(overview).length;
  const count = selected.size;
  const everything = allSelected(ctx);
  el.exportPlaylistsCount.textContent = overview ? t("ui-export-playlists-count", { count, total }) : "";
  // The header box, as in the track lists: ticked with everything chosen,
  // half-ticked with some.
  el.exportPlaylistsSelectAll.checked = everything;
  el.exportPlaylistsSelectAll.indeterminate = count > 0 && !everything;
  el.exportPlaylistsSelectAll.disabled = total === 0;
  el.exportPlaylistsSelectChangedBtn.disabled = total === 0;

  el.exportPlaylistsForceOrder.disabled = !everything;
  if (!everything) el.exportPlaylistsForceOrder.checked = false;
  el.exportPlaylistsForceOrderHint.textContent = everything
    ? t("ui-export-playlists-force-order-hint")
    : t("ui-export-playlists-force-order-needs-all");

  const trackOrderAvailable = !!overview?.forceTrackOrderAvailable;
  el.exportPlaylistsForceTrackOrder.disabled = !trackOrderAvailable;
  if (!trackOrderAvailable) el.exportPlaylistsForceTrackOrder.checked = false;
  if (overview?.forceTrackOrderTooltip) {
    el.exportPlaylistsForceTrackOrderRow.dataset.tooltip = overview.forceTrackOrderTooltip;
  } else {
    delete el.exportPlaylistsForceTrackOrderRow.dataset.tooltip;
  }

  el.exportPlaylistsWarning.classList.toggle("hidden", !everything || total < 2);

  el.exportPlaylistsSpace.classList.toggle("hidden", !space);
  el.exportPlaylistsSpace.textContent = space?.summaryText || "";
  el.exportPlaylistsSpace.classList.toggle("is-warning", !!space && !space.fits);

  el.exportPlaylistsRunBtn.textContent = overview?.usbName
    ? tCount("ui-export-playlists-run-usb", count, { name: overview.usbName })
    : tCount("ui-export-playlists-run", count);
  el.exportPlaylistsRunBtn.disabled = count === 0 || !hasUsb(state) || !!state.exportStarting
    || isUsbRootChangeBlocked(state) || (!!space && !space.fits);
}

// The size line for the chosen playlists, asked for once the choice settles;
// an answer overtaken by a newer choice is dropped.
export function refreshExportPlaylistsSpace(ctx) {
  const { state } = ctx;
  const current = page(ctx);
  clearTimeout(current.spaceTimer);
  const seq = ++current.spaceSeq;
  current.space = null;
  renderExportPlaylistsControls(ctx);
  if (!hasUsb(state) || current.selected.size === 0) return;
  const playlistIds = [...current.selected];
  current.spaceTimer = setTimeout(() => {
    ctx.command("get_playlists_export_space", { playlistIds, usbRoot: state.usbRoot })
      .then((data) => {
        if (seq !== current.spaceSeq) return;
        current.space = data?.exportSpace || null;
        renderExportPlaylistsControls(ctx);
      })
      .catch((err) => console.warn("get_playlists_export_space failed:", err));
  }, 250);
}

function setSelection(ctx, playlistIds) {
  const current = page(ctx);
  current.selected = new Set(playlistIds);
  ctx.el.exportPlaylistsList.querySelectorAll(".export-playlists-check").forEach((check) => {
    check.checked = current.selected.has(check.value);
  });
  refreshExportPlaylistsSpace(ctx);
}

// The header box: everything, or (with everything already chosen) nothing.
// "changed": the playlists the stick doesn't have up to date.
export function selectExportPlaylists(ctx, which) {
  const rows = selectableRows(page(ctx).overview);
  if (which === "all") setSelection(ctx, allSelected(ctx) ? [] : rows.map((row) => row.playlistId));
  else setSelection(ctx, rows.filter((row) => row.status !== "onStick").map((row) => row.playlistId));
}

export function toggleExportPlaylist(ctx, check) {
  const current = page(ctx);
  if (check.checked) current.selected.add(check.value);
  else current.selected.delete(check.value);
  refreshExportPlaylistsSpace(ctx);
}

// A click anywhere on a row ticks or unticks it, like its box.
export function clickExportPlaylistRow(ctx, target) {
  if (target.closest("input")) return;
  const check = target.closest(".export-playlists-row")?.querySelector(".export-playlists-check");
  if (!check || check.disabled) return;
  check.checked = !check.checked;
  toggleExportPlaylist(ctx, check);
}

export async function runExportPlaylists(ctx) {
  const { el } = ctx;
  const { overview, selected } = page(ctx);
  // In the app's order; the backend keeps that order whatever it's sent.
  const playlistIds = (overview?.rows || []).map((row) => row.playlistId).filter((id) => selected.has(id));
  if (playlistIds.length === 0) return;
  const exported = await exportPlaylistsToUsb(ctx, {
    playlistIds,
    forcePlaylistOrder: el.exportPlaylistsForceOrder.checked && allSelected(ctx),
    forceTrackOrder: el.exportPlaylistsForceTrackOrder.checked
  });
  // What's on the stick changed: the statuses follow, the choice starts over.
  if (exported && ctx.state.activeTab === "export-playlists") await openExportPlaylistsPage(ctx);
  else renderExportPlaylistsControls(ctx);
}
