import { createTrackListController } from "../shared/track_list_controller.mjs";
import { narrowTrackSelectionToLoaded } from "../shared/track_selection.mjs";
import { cloneTemplate } from "../../ui_utils.mjs";
import { t, tCount } from "../../ui_texts.mjs";
import { createDialogLifecycle } from "../shared/dialog_lifecycle.mjs";
import { catchErr } from "../shared/track_actions.mjs";
import { setActiveListItem } from "../shell/actions.mjs";
import { STORAGE_KEY_USB_ROOT, STORAGE_KEY_EXPORT_ENGINE_LIBRARY } from "../../settings_keys.mjs";
import {
  applyUsbOrderEdit, applyUsbOrderSortClick, forgetUsbOrderSort, isUsbOrderEditing,
  refuseWhileUsbOrderEditing, renderUsbOrderEditControls
} from "./order_edit.mjs";

// Job types that scope a Tauri command to state.usbRoot -- while one of
// these is running, the currently-selected root must not change underneath
// it, or an in-flight response (e.g. a parity report) can land after the
// user has already switched to a different drive and get rendered as if it
// belonged to the new one.
export const USB_ROOT_LOCKING_JOB_TYPES = new Set(["usb_read", "usb_write", "diagnostics", "export"]);

export function isUsbRootChangeBlocked(state) {
  for (const job of state.runningJobs.values()) {
    if (USB_ROOT_LOCKING_JOB_TYPES.has(job.jobType)) return true;
  }
  return false;
}

export function setUsbRootControlsLocked(ctx, locked) {
  const { el } = ctx;
  el.selectUsbFolderBtn.disabled = !!locked;
  el.selectUsbFolderBtn.title = locked ? t("ui-usb-select-usb-folder-busy-title") : "";
  el.usbRecentList.querySelectorAll("button").forEach((btn) => { btn.disabled = !!locked; });
  renderUsbOrderEditControls(ctx);
  if (locked) {
    el.exportPlaylistBtn.disabled = true;
  } else {
    // Don't just flip disabled=false here -- the export button's disabled
    // state is normally owned by playlist/usbRootValid logic (see below),
    // so hand back to that recompute instead of overriding it.
    ctx.updatePlaylistExportButtons();
  }
}

// Index the backend's per-playlist `playlistUsbExportStatus` (see
// PlaylistUsbExportStatus in backend/src/models.rs) by playlist id, for O(1)
// lookup while rendering. The backend computes same-name-on-USB and
// export-mode-locks-reorder itself; the frontend only looks the answer up.
export function playlistUsbExportStatusById(statusList) {
  return new Map(statusList.map((entry) => [entry.playlistId, entry]));
}

// Store a fresh backend status list and re-render what shows it: the export
// button label, the sidebar's "on this USB" marks and the open playlist's
// export status line.
export function applyPlaylistUsbExportStatus(ctx, statusList) {
  ctx.state.playlistUsbExportStatusById = playlistUsbExportStatusById(statusList);
  ctx.updatePlaylistExportButtons();
  ctx.renderPlaylistList();
  ctx.renderOpenPlaylistExportStatus();
  return ctx.state.playlistUsbExportStatusById;
}

export function diagStatusIcon(status) {
  if (status === "PASS") return "✓";
  if (status === "WARN") return "⚠";
  return "✗";
}

// "→ event log" after `anchor`, for an item whose details the Event Log lists
// (backend-set `link`); also puts the dot on the header Event Log button.
function appendEventLogLink(ctx, anchor, link) {
  if (link !== "event-log") return;
  const btn = cloneTemplate(anchor.ownerDocument, "tplDiagLogLink");
  btn.addEventListener("click", () => ctx.switchView("event-log").catch((err) => console.error(err)));
  anchor.after(btn);
  ctx.markEventLogLinkedDetails();
}

function renderDiagCheckRow(ctx, container, check, { withLogLink = false } = {}) {
  const doc = container.ownerDocument;
  const row = cloneTemplate(doc, "tplDiagCheck");
  row.classList.add(`diag-check-${check.status.toLowerCase()}`);
  row.querySelector(".diag-indicator").textContent = diagStatusIcon(check.status);
  row.querySelector("strong").textContent = check.label;
  const detail = row.querySelector(".diag-check-detail");
  detail.textContent = check.detail;
  if (withLogLink) appendEventLogLink(ctx, detail, check.link);
  container.appendChild(row);
}

// A report section: status dot + title heading, filled in by the caller.
function diagSection(doc, status, title) {
  const section = cloneTemplate(doc, "tplDiagSection");
  section.querySelector(".diag-dot").classList.add(`diag-${status.toLowerCase()}`);
  section.querySelector(".diag-section-title").textContent = title;
  return section;
}

// Fill a row's cells in order; the first cell holds the status dot.
function fillDiagRow(tr, status, values) {
  tr.querySelector(".diag-dot").classList.add(`diag-${status.toLowerCase()}`);
  values.forEach((value, i) => {
    const td = tr.cells[i + 1];
    (td.firstElementChild || td).textContent = value;
  });
  return tr;
}

export function renderDiagnosticsReport(ctx, data) {
  const { el, document } = ctx;
  el.usbDiagnosticsCard.classList.remove("hidden");
  showDiagReportView(ctx);
  el.previewRepairsBtn.disabled = false;
  ctx.updateUsbHealthDot(data.overallStatus, data.healthText);

  el.usbHealthCard.classList.remove("is-loading");
  if (data.overallStatus !== "PASS") el.usbHealthCard.open = true;

  el.diagOverallStatus.textContent = data.statusLabel;
  el.diagOverallStatus.className = `diag-badge diag-${data.overallStatus.toLowerCase()}`;
  clearParityBadge(el);
  el.diagDuration.textContent = data.durationText;

  el.diagSections.replaceChildren();
  renderReportBody(ctx, data);
}

// The backend's plain-language summary rows (`DiagOverviewItem`), shared by
// the diagnostics, parity and repair-preview views.
function renderOverviewRows(container, overview) {
  for (const entry of overview) {
    const row = cloneTemplate(container.ownerDocument, "tplDiagOverviewItem");
    const status = entry.status ? entry.status.toLowerCase() : "na";
    row.classList.add(`diag-check-${status}`);
    row.querySelector(".diag-indicator").textContent = entry.status ? diagStatusIcon(entry.status) : "–";
    row.querySelector(".diag-overview-title").textContent = entry.title;
    row.querySelector(".diag-overview-detail").textContent = entry.detail;
    container.appendChild(row);
  }
}

// Plain-language summary rows, then the checks grouped into boxes under a
// collapsed "Technical details". Both are backend-owned
// (service::diagnostics::{diagnostics_overview, parity_overview, regroup_sections}).
// Returns the technical body so a report can append its own extras.
function renderReportBody(ctx, data) {
  const { el, document } = ctx;
  renderOverviewRows(el.diagSections, data.overview);

  const technical = cloneTemplate(document, "tplDiagTechnical");
  const technicalBody = technical.querySelector(".diag-technical-body");
  el.diagSections.appendChild(technical);
  for (const group of data.sectionGroups) {
    const box = cloneTemplate(document, "tplDiagGroup");
    box.querySelector(".diag-group-title").textContent = group.title;
    technicalBody.appendChild(box);
    const parent = box.querySelector(".diag-group-sections");
    for (const sec of group.sections) {
      const div = diagSection(document, sec.status, sec.title);
      for (const check of sec.checks) {
        renderDiagCheckRow(ctx, div, check, { withLogLink: true });
      }
      parent.appendChild(div);
    }
  }
  return technicalBody;
}

export function renderParityReport(ctx, data) {
  const { el, document } = ctx;
  el.usbDiagnosticsCard.classList.remove("hidden");
  showDiagReportView(ctx);
  el.previewRepairsBtn.disabled = false;
  // Its own badge next to the health one: the health badge keeps matching
  // the USB health dot.
  el.diagParityStatus.textContent = data.statusLabel;
  el.diagParityStatus.className = `diag-badge diag-${data.overallStatus.toLowerCase()}`;
  el.diagDuration.textContent = data.durationText;

  el.diagSections.replaceChildren(cloneTemplate(document, "tplDiagParityIntro"));
  const technicalBody = renderReportBody(ctx, data);

  if (data.playlistDetails.length) {
    const box = cloneTemplate(document, "tplDiagParityPlaylists");
    const tbody = box.querySelector("tbody");
    for (const pd of data.playlistDetails) {
      // Backend-owned: `issueLabels` is built in Rust
      // (service::diagnostics::parity_issue_labels). The frontend renders them.
      tbody.appendChild(fillDiagRow(cloneTemplate(document, "tplDiagParityRow"), pd.status, [
        pd.name,
        pd.pdbTracks,
        pd.edbTracks,
        pd.matchedTracks,
        pd.issueLabels.join(", "),
      ]));
    }
    technicalBody.appendChild(box);
  }
}

function clearParityBadge(el) {
  el.diagParityStatus.textContent = "";
  el.diagParityStatus.className = "diag-badge";
}

export function showDiagReportView(ctx) {
  ctx.el.diagReportView.classList.remove("hidden");
  ctx.el.diagRepairPanel.classList.add("hidden");
}

function showDiagRepairView(ctx) {
  ctx.el.diagReportView.classList.add("hidden");
  ctx.el.diagRepairPanel.classList.remove("hidden");
}

// Blanks the diagnostics report content back to an empty/unknown state
// without touching whether the panel itself is shown or collapsed. Use this
// when the USB DBs changed underneath an on-screen report (repair, playlist
// edit, export, backup restore, ...) but the same drive is still selected --
// the stale report should disappear, not the whole panel.
function resetDiagnosticsContent(ctx) {
  const { el } = ctx;
  ctx.updateUsbHealthDot(null);
  el.diagSections.replaceChildren();
  el.diagOverallStatus.textContent = "";
  el.diagOverallStatus.className = "diag-badge";
  clearParityBadge(el);
  el.diagDuration.textContent = "";
  el.diagRepairSummary.textContent = "";
  el.diagRepairSummary.className = "diag-repair-summary";
  el.diagRepairOverview.replaceChildren();
  el.diagRepairFixes.replaceChildren();
  el.previewRepairsBtn.disabled = true;
  el.applyRepairsBtn.disabled = true;
  showDiagReportView(ctx);
}

// Clears a stale diagnostics report in place -- the DBs changed but the same
// USB drive is still selected, so leave the panel's open/closed state alone.
export function clearUsbDiagnostics(ctx) {
  resetDiagnosticsContent(ctx);
}

// Full hide: the diagnostics report no longer applies to anything on screen
// (USB root cleared or switched to a different drive), so collapse the panel
// too, not just its content.
export function hideUsbDiagnostics(ctx) {
  const { el } = ctx;
  resetDiagnosticsContent(ctx);
  el.usbDiagnosticsCard.classList.add("hidden");
  el.usbHealthCard.removeAttribute("open");
  el.usbHealthCard.classList.remove("is-loading");
}

export function renderRepairPreview(ctx, data) {
  const { state, el, document } = ctx;
  el.usbDiagnosticsCard.classList.remove("hidden");

  el.diagRepairSummary.textContent = data.summary;
  el.diagRepairSummary.className = data.overview.length === 0
    ? "diag-repair-summary diag-repair-summary-clean"
    : "diag-repair-summary";
  el.diagRepairOverview.replaceChildren();
  renderOverviewRows(el.diagRepairOverview, data.overview);
  el.diagRepairTechnical.open = false;
  el.diagRepairTechnical.classList.toggle("hidden", data.rows.length === 0);

  state.selectedRepairFixIds = new Set(data.rows.filter((row) => row.checked).map((row) => row.id));
  el.diagRepairFixes.replaceChildren();
  for (const row of data.rows) {
    const li = cloneTemplate(document, "tplRepairFix");
    li.className = row.selectable
      ? "diag-repair-fix-supported diag-repair-fix-with-select"
      : "diag-repair-fix-unsupported";
    const checkbox = li.querySelector(".diag-repair-fix-check");
    if (row.selectable) {
      checkbox.checked = row.checked;
      checkbox.disabled = row.locked;
      if (row.lockedTooltip) checkbox.title = row.lockedTooltip;
      checkbox.addEventListener("change", () => {
        if (checkbox.checked) state.selectedRepairFixIds.add(row.id);
        else state.selectedRepairFixIds.delete(row.id);
        el.applyRepairsBtn.disabled = state.selectedRepairFixIds.size === 0;
      });
    } else {
      checkbox.remove();
    }
    li.querySelector(".diag-repair-fix-title strong").textContent = row.title;
    const desc = li.querySelector(".diag-repair-fix-desc");
    desc.textContent = row.description;
    appendEventLogLink(ctx, desc, row.link);
    li.querySelector(".diag-repair-fix-meta").textContent = row.metaText;
    el.diagRepairFixes.appendChild(li);
  }

  showDiagRepairView(ctx);
  el.applyRepairsBtn.disabled = state.selectedRepairFixIds.size === 0;
  if (data.nothingToFix) el.previewRepairsBtn.disabled = true;
}
export function loadUsbRootFromStorage(ctx) {
  const { state, el, localStorage } = ctx;
  try {
    const raw = localStorage.getItem(STORAGE_KEY_USB_ROOT);
    state.usbRoot = raw ? String(raw).trim() || null : null;
  } catch {
    state.usbRoot = null;
  }
  state.usbRootValid = false;
  updateUsbRootText(ctx, state.usbRoot, false);
  el.usbInitRow.classList.add("hidden");
  updateUsbConfigControlsVisibility(ctx);
  ctx.updatePlaylistExportButtons();
}

// Each clear covers one list and its panel; the next read fills it again.
export function clearUsbPlaylistsView(ctx) {
  const { state, el } = ctx;
  forgetUsbListRead(ctx, "usb-playlists");
  state.usbPlaylists = [];
  state.usbOrderEditBlockedText = null;
  el.usbCountsText.textContent = "";
  renderUsbPlaylists(ctx);
  renderUsbOrderEditControls(ctx);
  ctx.usbPlaylistTracksCtl.clear();
}

function clearUsbHistoryView(ctx) {
  const { state, el } = ctx;
  forgetUsbListRead(ctx, "usb-history");
  state.histories = [];
  el.historyCountsText.textContent = "";
  renderHistoryList(ctx);
  ctx.usbHistoryTracksCtl.clear();
}

function clearUsbMenuViews(ctx) {
  const { state } = ctx;
  state.usbPlayerMenuCurrent = [];
  state.usbPlayerMenuAvailable = [];
  state.usbPlayerMenuCurrentSelectedKind = null;
  state.usbPlayerMenuAvailableSelectedKind = null;
  state.usbBackgroundColor = null;
  renderUsbPlayerMenuEditor(ctx);
  renderUsbBackgroundColor(ctx);
}

// The same USB stays selected but its databases changed (repair, restore).
// The export status and the diagnostics panel stay: the caller updates them.
export function usbContentChanged(ctx) {
  clearUsbPlaylistsView(ctx);
  clearUsbHistoryView(ctx);
  clearUsbMenuViews(ctx);
}

// The playlists and their export status against the USB now selected (none
// for one that isn't valid). Fire-and-forget: a failed read only leaves the
// marks stale.
function refreshExportStatusInBackground(ctx) {
  ctx.loadPlaylists().catch((err) => console.warn("list_playlists failed:", err));
}

// A USB was selected, re-selected or cleared: nothing on screen applies any more.
export function usbSelectionChanged(ctx) {
  ctx.state.usbDeviceName = null;
  ctx.state.usbNameTakenBy = null;
  ctx.updateUsbNameBadge();
  usbContentChanged(ctx);
  applyPlaylistUsbExportStatus(ctx, []);
  hideUsbDiagnostics(ctx);
  // The Export playlists page shows the newly selected USB, or nothing.
  if (ctx.state.activeTab === "export-playlists") {
    ctx.openExportPlaylistsPage().catch((err) => console.error(err));
  }
}

// The folders picked in the system dialog; none when it's cancelled.
export async function pickSourceFolders(ctx) {
  return ctx.invoke("pick_source_folders");
}
export function updateUsbConfigControlsVisibility(ctx) {
  const { state, el } = ctx;
  const hasValidRoot = !!state.usbRoot && !!state.usbRootValid;
  el.usbSelectedControls.classList.toggle("hidden", !hasValidRoot);
  el.usbDiagToolbar.classList.toggle("hidden", !hasValidRoot);
  if (!hasValidRoot) el.usbDiagnosticsCard.classList.add("hidden");
  ctx.updateUsbEmptyState();
}

export async function detectExternalRekordboxDb(ctx) {
  const { state, el } = ctx;
  try {
    const data = await ctx.command("detect_external_rekordbox_db");
    const found = data.found && !!data.path;
    state.externalRekordboxDbPath = found ? data.path : null;
    state.rekordboxDbImported = found && data.imported;
  } catch (err) {
    state.externalRekordboxDbPath = null;
    state.rekordboxDbImported = false;
    ctx.warn("External rekordbox DB detection failed:", err);
  }
  // Hide the legacy toggle element; the chip in renderSourceChips is the control
  el.externalRekordboxDbToggle.classList.add("hidden");
  ctx.renderSourceChips();
}

export async function detectExternalMixxxDb(ctx) {
  const { state } = ctx;
  try {
    const data = await ctx.command("detect_external_mixxx_db");
    const found = data.found && !!data.path;
    state.externalMixxxDbPath = found ? data.path : null;
    state.mixxxDbImported = found && data.imported;
  } catch (err) {
    state.externalMixxxDbPath = null;
    state.mixxxDbImported = false;
    ctx.warn("Mixxx library detection failed:", err);
  }
  ctx.renderSourceChips();
}

// Prompts for (and saves) a name for `state.usbRoot` if it doesn't have one
// yet. A name is this app's only notion of stable drive identity (see
// `usb_identity` on the backend) -- it's what lets backups and local
// staging caching correctly recognize "this is the same drive" again after
// a replug or a different computer, where the OS-assigned mount path can't.
// Best-effort: any failure to even check/show the prompt just lets the user
// continue unnamed rather than blocking USB use entirely.
// `data` is select_usb's `deviceName`, null when it couldn't be read (the
// backend logged why).
async function promptDriveNameIfUnset(ctx, data) {
  const { state } = ctx;
  if (!data) return;
  state.usbNameTakenBy = data.nameTakenBy;
  if (data.name) {
    state.usbDeviceName = data.name;
    ctx.updateUsbNameBadge();
    return;
  }
  await openDriveNameDialog(ctx, data.suggestedName);
}

// Renames the selected drive (or names it, if the first-time prompt was
// skipped). The backend moves the drive's backups and cache to the new name;
// everything showing the name is refreshed from there.
export async function renameDrive(ctx) {
  const { state } = ctx;
  if (isUsbRootChangeBlocked(state) || refuseWhileUsbOrderEditing(ctx)) return;
  const saved = await openDriveNameDialog(ctx, state.usbDeviceName);
  if (!saved) return;
  ctx.emitStatus(saved.statusText);
  if (!saved.updated) return;
  try {
    if (state.currentPlaylistId) await ctx.refreshCurrentPlaylistTracks();
    await loadUsbDevices(ctx);
  } catch (err) {
    ctx.warn("[usb] refresh after rename failed:", err);
  }
}

// The name dialog: "rename", "name" (first time) or "conflict" (the drive
// carries another drive's name, `state.usbNameTakenBy`) picks which of the
// template's texts show. Resolves to the save's response, or null if closed
// without saving.
async function openDriveNameDialog(ctx, initial) {
  const { state, el, command } = ctx;
  const overlay = el.driveNameOverlay;
  const mode = state.usbDeviceName ? "rename" : state.usbNameTakenBy ? "conflict" : "name";
  for (const node of overlay.querySelectorAll("[data-drive-name-mode]")) {
    node.hidden = !node.dataset.driveNameMode.split(" ").includes(mode);
  }
  el.driveNameConflict.textContent = t("ui-drive-name-conflict", { name: state.usbNameTakenBy });

  // This overlay is full-viewport and intercepts every click while open, so
  // it must never be able to get stuck there -- Escape, a backdrop click,
  // and an explicit "Not now"/"Cancel" button all close it without a name,
  // in addition to Save. A prompt with no way out would silently freeze the
  // entire app if anything about it ever misbehaves.
  const dialog = createDialogLifecycle(overlay, {
    onClose() {
      el.driveNameOkBtn.removeEventListener("click", onSave);
      el.driveNameSkipBtn.removeEventListener("click", onSkip);
      el.driveNameInput.removeEventListener("keydown", onEnter);
    },
  });
  const onSave = async () => {
    try {
      const data = await command("set_usb_device_name", { usbRoot: state.usbRoot, name: el.driveNameInput.value });
      // Playlists carry the name of the drive they were exported to.
      ctx.applyPlaylistsState(data);
      state.usbDeviceName = data.name;
      state.usbNameTakenBy = null;
      ctx.updateUsbNameBadge();
      dialog.close(data);
    } catch (err) {
      el.driveNameError.textContent = err?.message || String(err);
      el.driveNameError.hidden = false;
    }
  };
  const onSkip = () => dialog.close();
  const onEnter = (event) => {
    if (event.key === "Enter") onSave();
  };
  el.driveNameInput.value = initial ?? "";
  el.driveNameError.hidden = true;
  el.driveNameOkBtn.addEventListener("click", onSave);
  el.driveNameSkipBtn.addEventListener("click", onSkip);
  el.driveNameInput.addEventListener("keydown", onEnter);
  const result = dialog.open(el.driveNameInput);
  el.driveNameInput.select();
  return result;
}

export async function validateAndSetUsbRoot(ctx, path) {
  const { state, el, command, emitStatus } = ctx;

  if (isUsbRootChangeBlocked(state)) {
    emitStatus(t("ui-usb-switch-busy"));
    return false;
  }

  const input = String(path || "").trim();
  const previousRoot = state.usbRoot;
  if (input && previousRoot && input !== previousRoot) {
    hideUsbDiagnostics(ctx);
  }

  // Checks the folder, makes it the backend's selected USB (none for an empty
  // path) and saves it as the folder the app reopens with.
  const data = await command("select_usb", { path: input });
  const { valid, canInitialize } = data;
  state.usbRootValid = valid;
  state.usbRoot = input ? data.normalizedRoot || input : null;
  updateUsbRootText(ctx, state.usbRoot, valid);
  el.usbInitRow.classList.toggle("hidden", !canInitialize);
  el.usbInitHint.textContent = data.initHintText;
  el.initializeUsbBtn.disabled = !canInitialize;
  // Also when it's the same USB: selecting it again is how changes made on a
  // player show up (the backend just re-staged its databases).
  usbSelectionChanged(ctx);
  ctx.applyPlaylistsState(data);
  applyUsbDevices(ctx, data.devices);
  ctx.applyUsbAudioFormats(data.audioFormats);
  updateUsbConfigControlsVisibility(ctx);
  ctx.updateUsbSubNavDisabledState();
  ctx.updatePlaylistExportButtons();
  if (valid) await promptDriveNameIfUnset(ctx, data.deviceName);
  emitStatus(data.statusText);
  if (valid) {
    el.usbHealthCard.removeAttribute("open");
    el.usbHealthCard.classList.add("is-loading");
    // Next frame, so the "Running diagnostics..." state paints first.
    ctx.requestAnimationFrameFn(() => {
      ctx.runUsbDiagnostics().catch((err) => {
        ctx.warn("Auto-diagnostics failed:", err);
        emitStatus(t("ui-usb-auto-diagnostics-failed", { error: err?.message || err }));
      });
    });
  }
  // The recent list is for picking a drive: close it once one is selected
  // (its header reopens it), open it when none is.
  if (valid) el.usbRecentRow.classList.add("collapsed");
  if (!input) el.usbRecentRow.classList.remove("collapsed");
  return valid;
}

export async function removeUsbPlaylist(ctx, playlist) {
  const { state, emitStatus } = ctx;
  const confirmed = await ctx.openConfirmDialog({
    title: t("ui-usb-playlists-remove-title"),
    message: playlist.removeConfirmText,
    confirmLabel: t("ui-common-remove")
  });
  if (!confirmed) return;

  const data = await ctx.command("remove_usb_playlist", {
    usbRoot: state.usbRoot,
    playlistId: playlist.id,
    playlistName: playlist.name
  });
  clearUsbDiagnostics(ctx);
  await refreshUsb(ctx);
  emitStatus(data.statusText, { warningText: data.warningText });
}

// USB workflow orchestration.

// Each USB list page: its <ul> (which carries the `data-loaded` mark), its read,
// its track table and how a row opens.
const USB_LISTS = {
  "usb-playlists": { list: "usbPlaylists", read: readUsbPlaylists, tracksCtl: "usbPlaylistTracksCtl", openRow: openUsbPlaylistRow },
  "usb-history": { list: "historyList", read: readUsbHistories, tracksCtl: "usbHistoryTracksCtl", openRow: openHistoryRow }
};

// `state.usbListReads[viewId]` is the list's running read. Clearing the list or
// starting a newer read replaces it, and a read that is no longer the running
// one drops its response: the stick may have changed since it started.
function startUsbListRead(ctx, viewId) {
  const { state, el } = ctx;
  const { list, read } = USB_LISTS[viewId];
  delete el[list].dataset.loaded;
  const run = {};
  state.usbListReads[viewId] = run;
  run.promise = read(ctx, () => state.usbListReads[viewId] === run).finally(() => {
    if (state.usbListReads[viewId] === run) state.usbListReads[viewId] = null;
  });
  return run.promise;
}

function forgetUsbListRead(ctx, viewId) {
  ctx.state.usbListReads[viewId] = null;
  delete ctx.el[USB_LISTS[viewId].list].dataset.loaded;
}

// Opening a list page reads the list unless it's already loaded, then opens the
// first item if nothing is open. The page doesn't wait for it, and a failed read
// leaves no mark, so the next open retries. Clicking the page already on screen
// only retries a failed read: it doesn't reopen what a removal closed.
export function ensureUsbListLoaded(ctx, viewId, { viewChanged }) {
  const { state, el } = ctx;
  const running = state.usbListReads[viewId];
  const loaded = "loaded" in el[USB_LISTS[viewId].list].dataset;
  if (!viewChanged && (running || loaded)) return;
  let read;
  if (running) read = running.promise.catch(() => {}); // whoever started it reports its failure
  else if (!loaded) read = startUsbListRead(ctx, viewId).catch(catchErr(ctx.emitStatus));
  Promise.resolve(read).then(() => openFirstUsbListItem(ctx, viewId)).catch(catchErr(ctx.emitStatus));
}

// Only for a page still on screen: one the user left opens its item on return.
function openFirstUsbListItem(ctx, viewId) {
  const { list, tracksCtl, openRow } = USB_LISTS[viewId];
  const listEl = ctx.el[list];
  if (ctx.state.activeTab !== viewId || ctx[tracksCtl].scopeId != null || !("loaded" in listEl.dataset)) return;
  const firstRow = listEl.firstElementChild?.firstElementChild;
  if (firstRow) openRow(ctx, firstRow);
}

// Highlights a playlist row and loads its table. Returns the playlist.
export function openUsbPlaylistRow(ctx, btn) {
  const { state, el, usbPlaylistTracksCtl, emitStatus } = ctx;
  const playlist = findByScopeId(state.usbPlaylists, btn.dataset.usbPlaylist);
  setActiveListItem(el.usbPlaylists, btn);
  if (isUsbOrderEditing(state) && String(playlist.id) !== String(usbPlaylistTracksCtl.scopeId)) {
    forgetUsbOrderSort(ctx);
  }
  // The controller fetches page 1 (paginated + hydrated server-side),
  // renders it, and sets the "Total time" footer from the response.
  usbPlaylistTracksCtl.load({ scopeId: playlist.id }).catch((err) => {
    console.warn("USB playlist load failed:", err);
    emitStatus(t("ui-usb-playlists-load-failed", { error: err.message }));
  });
  return playlist;
}

// Highlights a history row and loads its table.
export function openHistoryRow(ctx, btn) {
  const { state, el, usbHistoryTracksCtl, emitStatus } = ctx;
  const history = findByScopeId(state.histories, btn.dataset.historyId);
  setActiveListItem(el.historyList, btn);
  usbHistoryTracksCtl.load({ scopeId: history.id }).catch((err) => {
    console.warn("USB history load failed:", err);
    emitStatus(t("ui-usb-history-load-failed", { error: err.message }));
  });
}

// A read that must see the stick as it is now (after a write), so it doesn't
// join one that started earlier.
export function refreshUsb(ctx) {
  return startUsbListRead(ctx, "usb-playlists");
}

async function readUsbPlaylists(ctx, isCurrent) {
  const { state, el, setProgress, startProgressHeartbeat, stopProgressHeartbeat, emitStatus } = ctx;
  emitStatus(t("ui-usb-playlists-loading"));
  setProgress(true, 5, t("ui-usb-playlists-reading"));
  startProgressHeartbeat();
  let data;
  try {
    data = await ctx.command("fetch_usb_playlists", {
      usbRoot: state.usbRoot
    });
  } catch (err) {
    stopProgressHeartbeat();
    if (!isCurrent()) {
      ctx.scheduleProgressIdle(0);
      return;
    }
    setProgress(true, 100, t("ui-usb-playlists-read-failed"), { error: true, dismissable: true });
    throw err;
  }
  stopProgressHeartbeat();
  if (!isCurrent()) {
    ctx.scheduleProgressIdle(0);
    return;
  }

  el.usbPlaylists.dataset.loaded = "";
  // The open playlist's heading keeps its count until its table has loaded again.
  const tracksCtl = ctx.usbPlaylistTracksCtl;
  const openSummary = findByScopeId(state.usbPlaylists, tracksCtl.scopeId)?.listSummaryText;
  state.usbPlaylists = data.items;
  const stillOpen = findByScopeId(state.usbPlaylists, tracksCtl.scopeId);
  if (stillOpen) stillOpen.listSummaryText = openSummary;
  ctx.applyPlaylistsState(data);

  el.usbCountsText.textContent = data.summaryText;
  // Renders the list too.
  applyUsbOrderEdit(ctx, data.orderEdit, data.orderEditBlockedText);
  keepOpenUsbPlaylist(ctx);
  // The freshly scanned status may flip an open local playlist's reorder lock.
  await ctx.renderCurrentPlaylistTracksFromState();
  setProgress(true, 100, data.doneText);
  emitStatus(data.statusText, { warningText: data.warningText });
  ctx.scheduleProgressIdle(1200);
}

// After a read, the open playlist stays open if it still exists. Its table loads
// again in place.
function keepOpenUsbPlaylist(ctx) {
  const { state, usbPlaylistTracksCtl: ctl, emitStatus } = ctx;
  if (!findByScopeId(state.usbPlaylists, ctl.scopeId)) {
    ctl.clear();
    return;
  }
  ctl.load().catch((err) => {
    console.warn("USB playlist load failed:", err);
    emitStatus(t("ui-usb-playlists-load-failed", { error: err.message }));
  });
}

// `openReport`: a run the user asked for opens Health & Diagnostics whatever
// the result; the automatic run on USB select opens it only when not PASS.
export async function runUsbDiagnostics(ctx, { openReport = false } = {}) {
  const { state, emitStatus } = ctx;
  const healthCard = ctx.el.usbHealthCard;
  healthCard.removeAttribute("open");
  healthCard.classList.add("is-loading");
  emitStatus(t("ui-usb-diagnostics-running"));
  try {
    const data = await ctx.command("run_usb_diagnostics", {
      usbRoot: state.usbRoot
    });
    ctx.applyPlaylistsState(data);
    await ctx.renderCurrentPlaylistTracksFromState();
    renderDiagnosticsReport(ctx, data);
    emitStatus(data.statusText);
  } finally {
    healthCard.classList.remove("is-loading");
    if (openReport) healthCard.open = true;
  }
}

export async function runUsbParityReport(ctx) {
  const { state, emitStatus } = ctx;
  emitStatus(t("ui-usb-parity-running"));
  try {
    const data = await ctx.command("run_usb_parity_report", {
      usbRoot: state.usbRoot
    });
    renderParityReport(ctx, data);
    emitStatus(data.statusText);
  } finally {
    // Only ever run from its button: show the result, whatever it is.
    ctx.el.usbHealthCard.open = true;
  }
}

export async function previewUsbRepairs(ctx) {
  const { state, emitStatus } = ctx;
  emitStatus(t("ui-usb-repair-previewing"));
  const data = await ctx.command("repair_usb_diagnostics", {
    usbRoot: state.usbRoot
  });
  renderRepairPreview(ctx, data);
  emitStatus(data.statusText);
}

export async function applyUsbRepairs(ctx) {
  const { state, emitStatus } = ctx;
  emitStatus(t("ui-usb-repair-applying"));
  const selectedFixIds = Array.from(state.selectedRepairFixIds);
  const data = await ctx.command("repair_usb_diagnostics", {
    usbRoot: state.usbRoot,
    apply: true,
    selectedFixIds
  });
  if (data.usbContentChanged) usbContentChanged(ctx);
  // The Engine library fix turns the setting on backend-side.
  syncEngineLibrarySetting(ctx, data.engineLibraryEnabled);
  ctx.applyPlaylistsState(data);
  await ctx.renderCurrentPlaylistTracksFromState();
  if (data.diagnostics) {
    renderDiagnosticsReport(ctx, data.diagnostics);
  }
  emitStatus(data.statusText);
}

// The backend already saved it; this only brings the open UI in line.
function syncEngineLibrarySetting(ctx, enabled) {
  ctx.state.exportEngineLibrary = enabled;
  ctx.el.exportEngineLibraryCheckbox.checked = enabled;
  try {
    ctx.localStorage?.setItem(STORAGE_KEY_EXPORT_ENGINE_LIBRARY, enabled ? "1" : "0");
  } catch {}
}

async function readUsbHistories(ctx, isCurrent) {
  const { state, el, emitStatus } = ctx;
  emitStatus(t("ui-usb-history-loading"));
  let data;
  try {
    data = await ctx.command("fetch_usb_histories", { usbRoot: state.usbRoot });
  } catch (err) {
    if (!isCurrent()) return;
    throw err;
  }
  if (!isCurrent()) return;

  el.historyList.dataset.loaded = "";
  state.histories = data.items;
  el.historyCountsText.textContent = data.summaryText;
  renderHistoryList(ctx);
  ctx.usbHistoryTracksCtl.clear();
  emitStatus(data.statusText, { warningText: data.warningText });
}

export async function exportHistoryTracklist(ctx) {
  const { state, emitStatus } = ctx;

  const history = findByScopeId(state.histories, ctx.usbHistoryTracksCtl.scopeId);
  const usbRoot = state.usbRoot;
  const { startTracks } = await ctx.command("get_usb_history_tracklist_options", { usbRoot, id: history.id });
  const choice = await ctx.tracklistExportDialog.open({
    startTracks,
    defaultTimesEnabled: true,
    defaultPlacement: "before"
  });
  if (!choice) return;

  const tracklist = await ctx.command("build_usb_history_tracklist", {
    usbRoot,
    id: history.id,
    startIndex: choice.startIndex,
    timeMode: choice.timeMode
  });
  const saved = await ctx.invoke("save_text_file", {
    suggestedFileName: tracklist.suggestedFileName,
    contents: tracklist.contents
  });
  emitStatus(saved ? tracklist.exportedText : tracklist.cancelledText);
}

// The selected kinds are a menu item's `kind` (a number) or null.
function ensureValidPlayerMenuSelections(state) {
  const has = (items, kind) => items.some((item) => item.kind === kind);
  if (!has(state.usbPlayerMenuAvailable, state.usbPlayerMenuAvailableSelectedKind)) {
    state.usbPlayerMenuAvailableSelectedKind = null;
  }
  if (!has(state.usbPlayerMenuCurrent, state.usbPlayerMenuCurrentSelectedKind)) {
    state.usbPlayerMenuCurrentSelectedKind = null;
  }
}

function buildPlayerMenuItemButton(documentObj, item, selectedKind, side) {
  const button = cloneTemplate(documentObj, "tplPlayerMenuItem");
  button.dataset.menuKind = String(item.kind);
  button.dataset.menuSide = side;
  button.querySelector(".player-menu-item-label").textContent = item.label;
  // Only muted: it stays selectable, because it can still be moved. On the
  // available side, `removable` means nothing.
  button.classList.toggle("is-locked", side === "current" && !item.removable);

  const selected = selectedKind === item.kind;
  button.classList.toggle("is-selected", selected);
  button.setAttribute("aria-selected", String(selected));
  return button;
}

export function handleUsbPlayerMenuListClick(ctx, side, event) {
  const { state } = ctx;
  const target = event.target.closest(".player-menu-item");
  if (!target) return;
  const kind = Number(target.dataset.menuKind);
  if (side === "available") {
    state.usbPlayerMenuAvailableSelectedKind = kind;
    state.usbPlayerMenuCurrentSelectedKind = null;
  } else {
    state.usbPlayerMenuCurrentSelectedKind = kind;
    state.usbPlayerMenuAvailableSelectedKind = null;
  }
  renderUsbPlayerMenuEditor(ctx);
}

export function renderUsbPlayerMenuEditor(ctx) {
  const { state, el, document } = ctx;
  const availableEl = el.usbPlayerMenuAvailable;
  const currentEl = el.usbPlayerMenuCurrent;

  ensureValidPlayerMenuSelections(state);

  availableEl.replaceChildren();
  for (const item of state.usbPlayerMenuAvailable) {
    const row = buildPlayerMenuItemButton(
      document,
      item,
      state.usbPlayerMenuAvailableSelectedKind,
      "available",
    );
    availableEl.appendChild(row);
  }

  currentEl.replaceChildren();
  for (const item of state.usbPlayerMenuCurrent) {
    const row = buildPlayerMenuItemButton(
      document,
      item,
      state.usbPlayerMenuCurrentSelectedKind,
      "current",
    );
    currentEl.appendChild(row);
  }

  syncUsbPlayerMenuEditorControls(ctx);
}

// Store a player-menu config response and redraw the editor.
function applyPlayerMenuConfig(ctx, data, selection = null) {
  const { state } = ctx;
  state.usbPlayerMenuCurrent = data.currentItems;
  state.usbPlayerMenuAvailable = data.availableItems;
  state.usbPlayerMenuCurrentSelectedKind = selection?.side === "current" ? selection.kind : null;
  state.usbPlayerMenuAvailableSelectedKind = selection?.side === "available" ? selection.kind : null;
  renderUsbPlayerMenuEditor(ctx);
}

export function syncUsbPlayerMenuEditorControls(ctx) {
  const { state, el } = ctx;
  const selected = state.usbPlayerMenuCurrent.find((item) => item.kind === state.usbPlayerMenuCurrentSelectedKind);
  el.usbPlayerMenuAddBtn.disabled = state.usbPlayerMenuAvailableSelectedKind === null;
  el.usbPlayerMenuRemoveBtn.disabled = !selected?.removable;
  el.usbPlayerMenuUpBtn.disabled = !selected?.canMoveUp;
  el.usbPlayerMenuDownBtn.disabled = !selected?.canMoveDown;
}

export async function loadUsbPlayerMenuConfig(ctx) {
  const { state, emitStatus } = ctx;
  emitStatus(t("ui-usb-player-menu-loading"));
  const data = await ctx.command("get_usb_player_menu_config", { usbRoot: state.usbRoot });
  applyPlayerMenuConfig(ctx, data);
  emitStatus(data.statusText);
}

// One editor edit (add, remove, moveUp, moveDown) on `kind`: the backend
// applies it to the stick's menu and answers with the new config. The edited
// item stays selected, in whichever list it is now in.
async function editUsbPlayerMenu(ctx, action, kind, side) {
  const { state, emitStatus } = ctx;
  const data = await ctx.command("update_usb_player_menu_config", {
    usbRoot: state.usbRoot,
    action,
    kind,
  });
  if (data.updated) clearUsbDiagnostics(ctx);
  applyPlayerMenuConfig(ctx, data, { side, kind });
  emitStatus(data.statusText);
}

export async function addUsbPlayerMenuItems(ctx) {
  await editUsbPlayerMenu(ctx, "add", ctx.state.usbPlayerMenuAvailableSelectedKind, "current");
}

export async function removeUsbPlayerMenuItems(ctx) {
  await editUsbPlayerMenu(ctx, "remove", ctx.state.usbPlayerMenuCurrentSelectedKind, "available");
}

export async function moveUsbPlayerMenuItems(ctx, direction) {
  const kind = ctx.state.usbPlayerMenuCurrentSelectedKind;
  await editUsbPlayerMenu(ctx, direction < 0 ? "moveUp" : "moveDown", kind, "current");
}

// The chips and the note are the backend's, ready to show.
export function renderUsbBackgroundColor(ctx) {
  const { state, el, document } = ctx;
  const data = state.usbBackgroundColor;

  el.usbBackgroundColorChips.replaceChildren(...(data?.chips || []).map((color) => {
    const chip = cloneTemplate(document, "tplBackgroundColorChip");
    chip.dataset.color = String(color.color);
    chip.querySelector(".bg-color-chip-label").textContent = color.label;
    chip.classList.toggle("is-selected", color.selected);
    chip.setAttribute("aria-checked", String(color.selected));
    chip.disabled = !data.writable;
    return chip;
  }));

  const text = data?.noteText || "";
  el.usbBackgroundColorNote.textContent = text;
  el.usbBackgroundColorNote.classList.toggle("hidden", !text);
}

export async function loadUsbBackgroundColor(ctx) {
  const { state } = ctx;
  try {
    state.usbBackgroundColor = await ctx.command("get_usb_background_color", { usbRoot: state.usbRoot });
  } catch (err) {
    state.usbBackgroundColor = null;
    ctx.warn("get_usb_background_color failed:", err);
    ctx.emitStatus(err?.message || String(err));
  }
  renderUsbBackgroundColor(ctx);
}

export async function setUsbBackgroundColor(ctx, color) {
  const { state, emitStatus } = ctx;
  const data = await ctx.command("set_usb_background_color", { usbRoot: state.usbRoot, color });
  state.usbBackgroundColor = data;
  if (data.updated) clearUsbDiagnostics(ctx);
  renderUsbBackgroundColor(ctx);
  emitStatus(data.statusText);
}

// One export at a time: the buttons are disabled from the click on, not only
// once the backend's job.started event arrives (the backend refuses a second
// export too).
async function withExportStarting(ctx, run) {
  const { state } = ctx;
  if (state.exportStarting || isUsbRootChangeBlocked(state)) return;
  state.exportStarting = true;
  ctx.updatePlaylistExportButtons();
  try {
    await run();
  } finally {
    state.exportStarting = false;
    ctx.updatePlaylistExportButtons();
  }
}

// What every export sends from Settings.
function exportOptions(state) {
  return {
    includeArtwork: true,
    includeAnalysis: true,
    pruneStale: !!state.exportPruneStale,
    backupBeforeExport: !!state.exportBackup
  };
}

// Runs an export command (`export_to_usb`, `export_playlists_to_usb`) with the
// progress footer up. Its response, or null once its failure is shown; a
// success's status line is shown, its playlists state is the caller's.
async function runExportCommand(ctx, command, request, runningText) {
  const { state, el, setProgress, stopProgressHeartbeat, emitStatus } = ctx;
  emitStatus(runningText);
  el.donateBtn.classList.add("exporting");
  setProgress(true, 8, t("ui-usb-export-starting"));
  ctx.startProgressHeartbeat();
  await ctx.nextPaint();
  let data;
  try {
    data = await ctx.command(command, request);
  } catch (error) {
    const msg = error.message.trim() || t("ui-usb-export-failed-unknown");
    emitStatus(error.details?.statusText || t("ui-usb-export-failed", { error: msg }));
    // The failed export may still have saved the sort or changed the USB.
    refreshExportStatusInBackground(ctx);
    // Reported: the backend already put the reason in the Event Log, so the
    // caller must not log it again.
    return null;
  } finally {
    if (!state.activeJobId) {
      setProgress(false, 0, t("ui-common-idle"));
      stopProgressHeartbeat();
    }
  }
  clearUsbDiagnostics(ctx);
  emitStatus(data.statusText, { warningText: data.warningText });
  return data;
}

export async function exportPlaylistToUsb(ctx, playlistId) {
  await withExportStarting(ctx, () => runPlaylistExport(ctx, playlistId));
}

async function runPlaylistExport(ctx, playlistId) {
  const { state } = ctx;
  const playlist = state.playlists.find((item) => item.id === playlistId);
  if (!playlist) return;
  // An active column sort becomes the exported order: the backend saves it
  // first (unless the export can't change the order on the USB).
  const sort = state.activeTab === playlistId ? ctx.tableSortState.playlistTracksBody : null;
  const data = await runExportCommand(ctx, "export_to_usb", {
    usbRoot: state.usbRoot,
    playlistId: playlist.id,
    options: exportOptions(state),
    sort: sort?.key ? { sortBy: sort.key, sortDir: sort.dir || "asc" } : null
  }, t("ui-usb-export-running", { name: playlist.name }));
  if (!data) return;
  // The sort is the playlist's order now (or, locked, stays only a view the
  // export didn't use): the page shows the order it exported.
  if (sort) ctx.clearPlaylistTrackSort();
  ctx.applyPlaylistsState(data);
  state.currentPlaylistId = playlistId;
  ctx.updateModeText();
  await ctx.switchView(playlistId);

  clearUsbPlaylistsView(ctx);
}

// The Export playlists page's export: the chosen playlists as one job. True
// once it succeeded.
export async function exportPlaylistsToUsb(ctx, { playlistIds, forcePlaylistOrder, forceTrackOrder }) {
  let exported = false;
  await withExportStarting(ctx, async () => {
    const data = await runExportCommand(ctx, "export_playlists_to_usb", {
      usbRoot: ctx.state.usbRoot,
      playlistIds,
      forcePlaylistOrder,
      forceTrackOrder,
      options: exportOptions(ctx.state)
    }, tCount("ui-usb-export-playlists-running", playlistIds.length));
    if (!data) return;
    ctx.applyPlaylistsState(data);
    clearUsbPlaylistsView(ctx);
    exported = true;
  });
  return exported;
}

export function renderUsbPlaylists(ctx) {
  const { state, el, document } = ctx;
  const openId = String(ctx.usbPlaylistTracksCtl.scopeId);
  el.usbPlaylists.replaceChildren(...state.usbPlaylists.map((playlist) => {
    const li = cloneTemplate(document, "tplUsbPlaylistItem");
    const btn = li.firstElementChild;
    btn.dataset.usbPlaylist = playlist.id;
    btn.classList.toggle("active", String(playlist.id) === openId);
    btn.querySelector(".playlist-label").textContent = playlist.name;
    btn.querySelector(".playlist-order-edited").classList.toggle("hidden", !playlist.orderEdited);
    btn.querySelector(".playlist-remove").dataset.usbRemovePlaylist = playlist.id;
    return li;
  }));
  renderUsbPlaylistHeading(ctx);
}

// Name, track count and size of the list shown in a USB track table, above
// it. `item` is the open playlist/history (null hides the heading). The count
// and size come with the first page (`listSummaryText`, stored on the item).
function renderTrackListHeading(heading, item) {
  heading.classList.toggle("hidden", !item);
  if (!item) return;
  heading.querySelector(".track-list-heading-name").textContent = item.name;
  heading.querySelector(".track-list-heading-count").textContent = item.listSummaryText || "";
}

function findByScopeId(items, scopeId) {
  return scopeId == null ? null : (items || []).find((item) => String(item.id) === String(scopeId)) || null;
}

export function renderUsbPlaylistHeading(ctx) {
  const playlist = findByScopeId(ctx.state.usbPlaylists, ctx.usbPlaylistTracksCtl.scopeId);
  renderTrackListHeading(ctx.el.usbPlaylistHeading, playlist);
}

export function renderHistoryHeading(ctx) {
  const history = findByScopeId(ctx.state.histories, ctx.usbHistoryTracksCtl.scopeId);
  renderTrackListHeading(ctx.el.historyHeading, history);
  ctx.el.exportHistoryTracklistBtn.disabled = !history?.canExportTracklist;
}

export function renderHistoryList(ctx) {
  const { state, el, document } = ctx;
  const items = state.histories.map((history) => {
    const li = cloneTemplate(document, "tplHistoryItem");
    li.firstElementChild.dataset.historyId = history.id;
    li.querySelector(".playlist-label").textContent = history.name;
    li.querySelector(".history-date").textContent = history.createdAt;
    return li;
  });
  el.historyList.replaceChildren(...items);
  renderHistoryHeading(ctx);
}

// The USB-playlist and USB-history track tables' data layer: paginated +
// searched + sorted + per-page-hydrated by the backend, rendered via the shared
// controller. Selection/search/sort/scroll all go through it.
function createUsbTracksController(ctx, { bodyId, selectionView, fetchCommand, secondaryActionType, actionType, getItems, onScopeChange, dragOptions = () => ({}) }) {
  const ctl = createTrackListController({
    bodyId,
    onScopeChange,
    onResponse: (data) => {
      const item = findByScopeId(getItems(), ctl.scopeId);
      if (!item) return;
      item.listSummaryText = data.listSummaryText;
      onScopeChange();
    },
    getElements: () => ({
      body: ctx.el[bodyId],
      wrap: ctx.el[bodyId].closest(".table-wrap"),
      durationTarget: bodyId === "historyTracks" ? ctx.el.historyTotalDuration : ctx.el.usbPlaylistTotalDuration,
    }),
    fetchPage: ({ scopeId, query, sortBy, sortDir, offset, limit }) =>
      ctx.command(fetchCommand, {
        usbRoot: ctx.state.usbRoot || null,
        id: scopeId,
        query,
        sortBy: sortBy || null,
        sortDir: sortDir || null,
        offset,
        limit,
      }),
    normalize: (track) => ctx.normalizeTrack(track),
    rowOptions: () => ({
      selectedIds: ctx.state.trackSelections[selectionView].ids,
      actionLabel: "+",
      actionType,
      origin: "usb",
      secondaryActionType,
      ...dragOptions(),
    }),
    buildRow: ctx.buildTrackTableRow,
    renderEmpty: ctx.renderEmptyTrackTable,
    finishRows: ctx.finishTrackRows,
    getTableSortState: () => ctx.tableSortState,
    onPage: (_page, { first }) => {
      if (first) narrowTrackSelectionToLoaded(ctx, selectionView, ctl.items);
    },
  });
  return ctl;
}

export function createUsbPlaylistTracksController(ctx) {
  const ctl = createUsbTracksController(ctx, {
    bodyId: "usbPlaylistTracks",
    selectionView: "usbPlaylist",
    fetchCommand: "fetch_usb_playlist_tracks",
    actionType: "add-usb",
    secondaryActionType: "play-usb",
    getItems: () => ctx.state.usbPlaylists,
    onScopeChange: () => renderUsbPlaylistHeading(ctx),
    // The drag handles show only during Edit order, disabled while searching.
    dragOptions: () => {
      const editing = isUsbOrderEditing(ctx.state);
      return {
        reservesDragColumn: true,
        enableDragReorder: editing && !ctl.query,
        dragDisabledTooltip: editing && ctl.query ? t("ui-usb-playlists-drag-search-active") : null,
      };
    },
  });
  // While editing, a column sort is the playlist's new (pending) order.
  const applyViewSort = ctl.applyHeaderSort;
  ctl.applyHeaderSort = () => (isUsbOrderEditing(ctx.state) ? applyUsbOrderSortClick(ctx) : applyViewSort());
  return ctl;
}

export function createUsbHistoryTracksController(ctx) {
  return createUsbTracksController(ctx, {
    bodyId: "historyTracks",
    selectionView: "usbHistory",
    fetchCommand: "fetch_usb_history_tracks",
    actionType: "add-history",
    secondaryActionType: "play-history",
    getItems: () => ctx.state.histories,
    onScopeChange: () => renderHistoryHeading(ctx),
  });
}

export async function initializeUsb(ctx) {
  const { state, el, emitStatus } = ctx;
  try {
    const data = await ctx.command("initialize_usb", { usbRoot: state.usbRoot });
    emitStatus(data.statusText);
    el.usbInitRow.classList.add("hidden");
    await validateAndSetUsbRoot(ctx, state.usbRoot);
  } catch (err) {
    ctx.logError("Initialize USB failed:", err);
    emitStatus(err?.message || String(err));
  }
}

export async function pickUsbFolder(ctx) {
  if (isUsbRootChangeBlocked(ctx.state)) {
    ctx.emitStatus(t("ui-usb-switch-busy"));
    return null;
  }
  const selected = await ctx.invoke("pick_usb_folder");
  if (!selected) return null;
  await validateAndSetUsbRoot(ctx, String(selected));
  return selected;
}

// A clicked USB row the page fetch couldn't fill in completely: the backend
// inspects the track and answers with its whole row, shown on every loaded
// copy of that file.
export async function hydrateUsbTrackMetadata(ctx, track) {
  // Backend-owned: `needsHydration` (service::usb::hydrate_usb_track_in_place)
  // says whether an inspect could still fill anything in.
  if (!track || track.needsHydration !== true) return;
  const trackId = String(track.id || "").trim();
  if (!/^\d+$/.test(trackId)) return;
  // Asked once: the inspected row comes back with the flag cleared, and a
  // failed inspect isn't retried.
  track.needsHydration = false;
  try {
    const inspected = await ctx.command("inspect_usb_track", {
      usbRoot: ctx.state.usbRoot,
      trackId,
      filePath: track.filePath || "",
      title: track.title || "",
      artist: track.artist || ""
    });
    ctx.showTrackRow(inspected.track);
  } catch (err) {
    console.warn(`inspect_usb_track failed for ${trackId}:`, err);
  }
}

// Replaces the old localStorage/app_settings "recent USB roots" list with a
// live query against the usb_devices table -- so mount state and pruning
// are always accurate, not a client-side cache that can drift from it.
export async function loadUsbDevices(ctx) {
  let items = [];
  try {
    ({ items } = await ctx.command("list_usb_devices"));
  } catch (err) {
    console.warn("Failed to load USB devices:", err);
  }
  applyUsbDevices(ctx, items);
}

// The known drives, for the recent list.
function applyUsbDevices(ctx, devices) {
  const { state } = ctx;
  state.usbDevices = devices || [];
  renderUsbRecentRoots(ctx);
}

export async function pruneUsbDevice(ctx, id) {
  if (!id) return;
  try {
    const data = await ctx.command("prune_usb_device", { id });
    applyUsbDevices(ctx, data.devices);
  } catch (err) {
    console.warn(`Failed to prune USB device ${id}:`, err);
  }
}

export function renderUsbRecentRoots(ctx) {
  const { state, el, document } = ctx;
  el.usbRecentList.replaceChildren();
  const devices = state.usbDevices;
  el.usbRecentRow.classList.toggle("hidden", !devices.length);
  if (!devices.length) return;
  const locked = isUsbRootChangeBlocked(state);
  devices.forEach((device) => {
    const path = device.rootPath;
    const row = cloneTemplate(document, "tplUsbRecentItem");
    const btn = row.querySelector(".usb-cfg-recent-btn");
    btn.dataset.usbRecentPath = path;
    btn.dataset.tooltip = path;
    btn.textContent = path;
    btn.disabled = locked;

    const pruneBtn = row.querySelector(".usb-cfg-recent-prune-btn");
    pruneBtn.dataset.usbPruneDeviceId = device.id;
    pruneBtn.setAttribute("aria-label", t("ui-usb-forget-path-aria-label", { path }));
    pruneBtn.disabled = locked;
    el.usbRecentList.appendChild(row);
  });
}

export function updateUsbRootText(ctx, path, valid = false) {
  const { el } = ctx;
  el.usbConnectionBar.classList.remove("hidden");
  if (!valid) {
    el.usbRootPathText.textContent = t("ui-usb-root-path");
    el.usbRootPathText.classList.remove("usb-path-valid", "usb-path-invalid");
    delete el.usbRootPathText.dataset.tooltip;
    return;
  }
  // Show the drive's name; the full folder is in the tooltip. The path is the
  // label only until the name has been read (or for a drive without one).
  el.usbRootPathText.textContent = ctx.state.usbDeviceName || path;
  el.usbRootPathText.dataset.tooltip = path;
  el.usbRootPathText.classList.add("usb-path-valid");
  el.usbRootPathText.classList.remove("usb-path-invalid");
}
