import { warningEntryText, patchUsbRowsInContainer } from "../library/actions.mjs";
import { createTrackListController } from "../shared/track_list_controller.mjs";
import { cloneTemplate } from "../../ui_utils.mjs";
import { STORAGE_KEY_USB_ROOT } from "../../settings_keys.mjs";
import {
  formatDurationMs,
  renderTrackListDurationSummary,
  getHistoryDateValue,
  buildTracklistText,
} from "../../track_utils.mjs";

// Job types that scope a Tauri command to state.usbRoot -- while one of
// these is running, the currently-selected root must not change underneath
// it, or an in-flight response (e.g. a parity report) can land after the
// user has already switched to a different drive and get rendered as if it
// belonged to the new one.
export const USB_ROOT_LOCKING_JOB_TYPES = new Set(["usb_read", "usb_write", "diagnostics", "export"]);

export function isUsbRootChangeBlocked(state) {
  return !!state.activeJobId && USB_ROOT_LOCKING_JOB_TYPES.has(state.activeJobType);
}

export function setUsbRootControlsLocked(ctx, locked) {
  const { el } = ctx;
  if (el.selectUsbFolderBtn) {
    el.selectUsbFolderBtn.disabled = !!locked;
    el.selectUsbFolderBtn.title = locked ? "Please wait for the current USB operation to finish" : "";
  }
  el.usbRecentList?.querySelectorAll("button").forEach((btn) => { btn.disabled = !!locked; });
  if (locked) {
    if (el.exportPlaylistBtn) el.exportPlaylistBtn.disabled = true;
  } else {
    // Don't just flip disabled=false here -- the export button's disabled
    // state is normally owned by playlist/usbRootValid logic (see below),
    // so hand back to that recompute instead of overriding it.
    ctx.updatePlaylistExportButtons();
  }
}

function joinWarningTexts(warnings) {
  return (Array.isArray(warnings) ? warnings : [])
    .map(warningEntryText)
    .filter(Boolean)
    .join(" | ");
}

// Index the backend's per-playlist `playlistUsbExportStatus` (see
// PlaylistUsbExportStatus in backend/src/models.rs) by playlist id, for O(1)
// lookup while rendering. The backend computes same-name-on-USB and
// export-mode-locks-reorder itself; the frontend only looks the answer up.
export function playlistUsbExportStatusById(statusList) {
  const byId = new Map();
  for (const entry of statusList || []) {
    const id = String(entry?.playlistId || "");
    if (id) byId.set(id, entry);
  }
  return byId;
}

// Cheap backend recompute of every playlist's `PlaylistUsbExportStatus` (staged
// PDB/eDB only, no USB access) -- used after the export sync-mode setting
// changes so the reorder lock reflects the new mode without a full USB rescan
// and without the frontend re-deriving the rule.
export async function refreshPlaylistExportStatus(ctx) {
  const { state } = ctx;
  const data = await ctx.command("refresh_playlist_export_status", {
    usbRoot: state.usbRoot || null,
  });
  state.playlistUsbExportStatusById = playlistUsbExportStatusById(
    data?.playlistUsbExportStatus,
  );
  return state.playlistUsbExportStatusById;
}

export function computeExportButtonState({
  usbRoot,
  usbRootValid,
  currentPlaylistId,
  playlistUsbExportStatusById: statusById
}) {
  const enabled = !!usbRoot && !!usbRootValid;
  if (!enabled) {
    return { enabled: false, text: "Select USB first", title: "Select a valid USB folder first" };
  }
  // The label ("Append to (X) on USB: (dir)" vs "Export to USB: dir") and its
  // tooltip are backend-computed per playlist (PlaylistUsbExportStatus --
  // service::export::compute_playlist_usb_export_status), so the append rule
  // and the USB-path parsing both live in Rust. The fallbacks only apply
  // before the first USB scan has populated the status map.
  const status = statusById instanceof Map ? statusById.get(String(currentPlaylistId || "")) : null;
  return {
    enabled: true,
    text: status?.exportButtonText || "Export to USB",
    title: status?.exportButtonTitle || "Export current playlist to selected USB"
  };
}
export function diagStatusIcon(status) {
  if (status === "PASS") return "✓";
  if (status === "WARN") return "⚠";
  return "✗";
}

function renderDiagCheckRow(ctx, container, check, { withLogLink = false } = {}) {
  const doc = container.ownerDocument;
  const row = cloneTemplate(doc, "tplDiagCheck");
  row.classList.add(`diag-check-${check.status.toLowerCase()}`);
  row.querySelector(".diag-indicator").textContent = diagStatusIcon(check.status);
  row.querySelector("strong").textContent = check.label;
  row.querySelector(".diag-check-detail").textContent = check.detail;
  if (withLogLink && check.link === "event-log") {
    const btn = cloneTemplate(doc, "tplDiagLogLink");
    btn.addEventListener("click", () => ctx.switchView("event-log").catch((err) => console.error(err)));
    row.appendChild(btn);
  }
  container.appendChild(row);
}

// A report section: status dot + title heading, filled in by the caller.
function diagSection(doc, status, title) {
  const section = cloneTemplate(doc, "tplDiagSection");
  section.querySelector(".diag-dot").classList.add(`diag-${String(status).toLowerCase()}`);
  section.querySelector(".diag-section-title").textContent = title;
  return section;
}

// Swap the playlist-details table to `headTemplate`'s columns and return its
// emptied body.
function resetDiagPlaylistTable(el, summaryText, headTemplate) {
  el.diagPlaylistDetails.classList.remove("hidden");
  const summary = el.diagPlaylistDetails.querySelector("summary");
  if (summary) summary.textContent = summaryText;
  el.diagPlaylistDetails.querySelector("thead tr")
    ?.replaceWith(cloneTemplate(el.diagPlaylistDetails.ownerDocument, headTemplate));
  el.diagPlaylistTableBody.replaceChildren();
  return el.diagPlaylistTableBody;
}

// Fill a row's cells in order; the first cell holds the status dot.
function fillDiagRow(tr, status, values) {
  tr.querySelector(".diag-dot").classList.add(`diag-${String(status || "PASS").toLowerCase()}`);
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
  ctx.updateUsbHealthDot(data.overallStatus);

  const healthCard = document.getElementById("usbHealthCard");
  if (healthCard) {
    healthCard.classList.remove("is-loading");
    if (data.overallStatus !== "PASS") {
      healthCard.open = true;
    }
  }

  el.diagOverallStatus.textContent = data.overallStatus;
  el.diagOverallStatus.className = `diag-badge diag-${data.overallStatus.toLowerCase()}`;
  el.diagDuration.textContent = `Completed in ${data.durationMs}ms`;

  const sections = [
    data.pdbIntegrity,
    data.edbAccess,
    data.contentsIntegrity,
    data.analysisIntegrity,
    data.playlistResolution,
    // Backend-assembled (service::diagnostics::player_counter_snapshot_section).
    data.cdjCounterSection,
  ].filter(Boolean);

  el.diagSections.replaceChildren();
  for (const sec of sections) {
    const div = diagSection(document, sec.status, sec.title);
    for (const check of (sec.checks || [])) {
      renderDiagCheckRow(ctx, div, check, { withLogLink: true });
    }
    el.diagSections.appendChild(div);
  }

  if (data.playlistDetails?.length) {
    const tbody = resetDiagPlaylistTable(el, "Playlist Resolution Details", "tplDiagResolutionHead");
    for (const pd of data.playlistDetails) {
      tbody.appendChild(fillDiagRow(cloneTemplate(document, "tplDiagResolutionRow"), pd.status, [
        pd.name,
        pd.resolvedEntries,
        pd.totalEntries,
        `${(pd.resolutionRate * 100).toFixed(1)}%`,
      ]));
    }
  } else {
    el.diagPlaylistDetails.classList.add("hidden");
  }
}

export function renderParityReport(ctx, data) {
  const { el, document } = ctx;
  el.usbDiagnosticsCard.classList.remove("hidden");
  showDiagReportView(ctx);
  el.previewRepairsBtn.disabled = false;
  el.diagOverallStatus.textContent = data.overallStatus;
  el.diagOverallStatus.className = `diag-badge diag-${data.overallStatus.toLowerCase()}`;
  el.diagDuration.textContent = `Completed in ${data.durationMs}ms`;

  const section = {
    title: "USB Strict Parity Report",
    status: data.overallStatus,
    checks: data.checks || []
  };
  el.diagSections.replaceChildren();
  const div = diagSection(document, section.status, section.title);
  if (Array.isArray(data.summaryRows) && data.summaryRows.length) {
    const summary = cloneTemplate(document, "tplDiagParitySummary");
    const tbody = summary.querySelector("tbody");
    for (const row of data.summaryRows) {
      const tr = cloneTemplate(document, "tplDiagParitySummaryRow");
      tr.querySelector(".diag-summary-status").textContent = String(row.status || "PASS");
      tbody.appendChild(fillDiagRow(tr, row.status, [row.label || "", Number(row.count || 0)]));
    }
    div.append(...summary.children);
  }
  for (const check of section.checks) {
    renderDiagCheckRow(ctx, div, check);
  }
  el.diagSections.appendChild(div);

  if (data.playlistDetails?.length) {
    const tbody = resetDiagPlaylistTable(el, "Strict Parity Playlist Details", "tplDiagParityHead");
    for (const pd of data.playlistDetails) {
      // Backend-owned: `issueLabels` is built in Rust
      // (service::diagnostics::parity_issue_labels). The frontend renders them.
      const issues = Array.isArray(pd?.issueLabels) ? pd.issueLabels : [];
      tbody.appendChild(fillDiagRow(cloneTemplate(document, "tplDiagParityRow"), pd.status, [
        pd.name,
        Number(pd.pdbTracks || 0),
        Number(pd.edbTracks || 0),
        pd.matchedTracks,
        issues.join(", "),
      ]));
    }
  } else {
    el.diagPlaylistDetails.classList.add("hidden");
  }
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
  [el.usbHealthDot, el.usbHeaderHealthDot].filter(Boolean).forEach((dot) => {
    dot.classList.remove("health-pass", "health-warn", "health-fail");
    dot.dataset.tooltip = "USB health: unknown";
    dot.setAttribute("aria-label", "USB health: unknown");
  });
  if (el.diagSections) {
    el.diagSections.replaceChildren();
  }
  if (el.diagOverallStatus) {
    el.diagOverallStatus.textContent = "";
    el.diagOverallStatus.className = "diag-badge";
  }
  if (el.diagDuration) {
    el.diagDuration.textContent = "";
  }
  if (el.diagPlaylistDetails) {
    el.diagPlaylistDetails.classList.add("hidden");
  }
  if (el.diagPlaylistTableBody) {
    el.diagPlaylistTableBody.replaceChildren();
  }
  if (el.diagRepairSummary) {
    el.diagRepairSummary.textContent = "";
    el.diagRepairSummary.className = "diag-repair-summary";
  }
  if (el.diagRepairFixes) {
    el.diagRepairFixes.replaceChildren();
  }
  if (el.previewRepairsBtn) {
    el.previewRepairsBtn.disabled = true;
  }
  if (el.applyRepairsBtn) {
    el.applyRepairsBtn.disabled = true;
  }
  if (el.diagReportView && el.diagRepairPanel) {
    showDiagReportView(ctx);
  }
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
  if (el.usbDiagnosticsCard) {
    el.usbDiagnosticsCard.classList.add("hidden");
  }
  const healthCard = el.usbDiagnosticsCard?.closest?.("#usbHealthCard");
  if (healthCard) {
    healthCard.removeAttribute("open");
    healthCard.classList.remove("is-loading");
  }
}

export function renderRepairPreview(ctx, data) {
  const { state, el, document } = ctx;
  if (!el.diagRepairPanel) return;
  el.usbDiagnosticsCard.classList.remove("hidden");

  const issueCount = Array.isArray(data.detectedIssues) ? data.detectedIssues.length : 0;
  const fixes = data.proposedFixes || [];
  const unsupportedItems = data.unsupportedItems || [];
  const fixCount = fixes.length;
  const supportedFixes = fixes.filter((f) => f.supported);
  state.selectedRepairFixIds = new Set(
    supportedFixes.map((f) => String(f?.id || "")).filter(Boolean)
  );
  const writes = Number(data.estimatedFileWrites || 0);
  const deletes = Number(data.estimatedFileDeletes || 0);

  if (el.diagRepairSummary) {
    if (fixCount === 0 && issueCount === 0) {
      el.diagRepairSummary.textContent = "No issues found.";
      el.diagRepairSummary.className = "diag-repair-summary diag-repair-summary-clean";
    } else {
      const parts = [`${issueCount} issue(s)`, `${supportedFixes.length} fixable`];
      if (writes) parts.push(`${writes} writes`);
      if (deletes) parts.push(`${deletes} deletes`);
      el.diagRepairSummary.textContent = parts.join(" · ");
      el.diagRepairSummary.className = "diag-repair-summary";
    }
  }

  if (el.diagRepairFixes) {
    // Backend-owned: each fix's `description` is already the full text (the
    // reason a fix is manual-only is baked in server-side), and unsupported
    // items no longer duplicate a fix row -- so they render straight through.
    const fixesToRender = fixes.map((f) => ({ ...f }));
    for (const item of unsupportedItems) {
      fixesToRender.push({
        id: `unsupported:${item.issue}`,
        title: item.issue,
        description: item.reason,
        supported: false,
        destructive: false,
        estimatedWrites: 0,
        estimatedDeletes: 0
      });
    }

    el.diagRepairFixes.replaceChildren();
    for (const fix of fixesToRender) {
      const li = cloneTemplate(document, "tplRepairFix");
      li.className = fix.supported ? "diag-repair-fix-supported" : "diag-repair-fix-unsupported";
      if (fix.supported) {
        li.classList.add("diag-repair-fix-with-select");
      }
      const checkbox = li.querySelector(".diag-repair-fix-check");

      if (fix.supported) {
        const fixId = String(fix.id || "");
        const alwaysApplied = fix.alwaysApplied === true;
        checkbox.checked = alwaysApplied || state.selectedRepairFixIds.has(fixId);
        checkbox.dataset.fixId = fixId;
        if (alwaysApplied) {
          checkbox.disabled = true;
          checkbox.title = "Always applied — required for other repairs to complete safely";
        } else {
          checkbox.addEventListener("change", (event) => {
            if (!fixId) return;
            if (event?.target?.checked) state.selectedRepairFixIds.add(fixId);
            else state.selectedRepairFixIds.delete(fixId);
            el.applyRepairsBtn.disabled = state.selectedRepairFixIds.size === 0;
          });
        }
      } else {
        checkbox.remove();
      }

      li.querySelector(".diag-repair-fix-title strong").textContent = fix.title;
      li.querySelector(".diag-repair-fix-desc").textContent = fix.description;
      const meta = li.querySelector(".diag-repair-fix-meta");
      const support = fix.supported ? "✓ supported" : "✗ preview-only";
      const mode = fix.destructive ? "destructive" : "safe";
      const metaParts = [support, mode];
      if (fix.estimatedWrites) metaParts.push(`${fix.estimatedWrites} writes`);
      if (fix.estimatedDeletes) metaParts.push(`${fix.estimatedDeletes} deletes`);
      if (fix.alwaysApplied === true) metaParts.push("always applied");
      meta.textContent = metaParts.join(" · ");

      el.diagRepairFixes.appendChild(li);
    }
  }

  showDiagRepairView(ctx);
  el.applyRepairsBtn.disabled = state.selectedRepairFixIds.size === 0;
  if (supportedFixes.length === 0 && fixCount === 0) {
    el.previewRepairsBtn.disabled = true;
  }
}
export function loadUsbRootFromStorage(ctx) {
  const { state, el, localStorage } = ctx;
  try {
    const raw = localStorage?.getItem?.(STORAGE_KEY_USB_ROOT);
    state.usbRoot = raw ? String(raw).trim() || null : null;
  } catch {
    state.usbRoot = null;
  }
  state.usbRootValid = false;
  updateUsbRootText(ctx, state.usbRoot, false);
  if (el.usbInitRow) {
    el.usbInitRow.classList.add("hidden");
  }
  updateUsbConfigControlsVisibility(ctx);
  ctx.updatePlaylistExportButtons();
}

export function resetUsbStateViews(ctx, { hideDiagnostics = true } = {}) {
  const { state, el } = ctx;
  state.usbPlaylists = [];
  state.playlistUsbExportStatusById = new Map();
  state.histories = [];
  state.selectedHistoryIndex = null;
  state.historyTracks = [];
  state.usbPlayerMenuCurrent = [];
  state.usbPlayerMenuAvailable = [];
  state.usbPlayerMenuCurrentSelectedKind = null;
  state.usbPlayerMenuAvailableSelectedKind = null;

  el.usbCountsText.textContent = "";
  el.historyCountsText.textContent = "";
  if (el.exportHistoryTracklistBtn) el.exportHistoryTracklistBtn.disabled = true;
  // The USB may still be connected and selected (e.g. after a backup
  // restore or a repair apply) -- only a full disconnect/switch-drive
  // should collapse the diagnostics panel itself, not just blank its report.
  if (hideDiagnostics) hideUsbDiagnostics(ctx);

  renderUsbPlaylists(ctx);
  ctx.usbPlaylistTracksCtl.clear();
  renderHistoryList(ctx);
  ctx.usbHistoryTracksCtl.clear();
  renderUsbPlayerMenuEditor(ctx);
}

export async function syncAssetScopePaths(ctx) {
  const { state } = ctx;
  const paths = [];
  for (const root of state.sourceRoots || []) {
    const value = String(root || "").trim();
    if (value) paths.push(value);
  }
  const usbRoot = String(state.usbRoot || "").trim();
  if (usbRoot) paths.push(usbRoot);
  if (!paths.length) return;

  try {
    await ctx.invoke("allow_asset_paths", { paths });
  } catch (err) {
    ctx.warn("allow_asset_paths failed:", err);
  }
}

export async function pickSourceFolders(ctx) {
  const selected = await ctx.invoke("pick_source_folders");
  if (!selected) return [];

  const rawItems = Array.isArray(selected) ? selected : [selected];
  return rawItems
    .map((item) => {
      if (typeof item === "string") return item;
      if (!item || typeof item !== "object") return "";
      if (typeof item.path === "string") return item.path;
      if (typeof item.Path === "string") return item.Path;
      if (typeof item.url === "string") return item.url;
      if (typeof item.Url === "string") return item.Url;
      if (typeof item.filePath === "string") return item.filePath;
      return "";
    })
    .filter(Boolean);
}
export function updateUsbConfigControlsVisibility(ctx) {
  const { state, el } = ctx;
  const hasValidRoot = !!state.usbRoot && !!state.usbRootValid;
  if (el.usbSelectedControls) {
    el.usbSelectedControls.classList.toggle("hidden", !hasValidRoot);
  }
  if (!hasValidRoot && el.usbDiagnosticsCard) {
    el.usbDiagnosticsCard.classList.add("hidden");
  }
  ctx.updateUsbEmptyState();
}

export async function detectExternalMasterDb(ctx) {
  const { state, el } = ctx;
  try {
    const data = await ctx.command("detect_external_master_db");
    const found = !!data?.found && !!data?.path;
    state.externalMasterDbPath = found ? data.path : null;
    if (!found) state.masterDbEnabled = false;
  } catch (err) {
    state.externalMasterDbPath = null;
    state.masterDbEnabled = false;
    ctx.warn("External master DB detection failed:", err);
  }
  // Hide the legacy toggle element; the chip in renderSourceChips is the control
  el.externalMasterDbToggle?.classList.add("hidden");
  ctx.renderSourceChips();
}

// Prompts for (and saves) a name for `state.usbRoot` if it doesn't have one
// yet. A name is this app's only notion of stable drive identity (see
// `usb_identity` on the backend) -- it's what lets backups and local
// staging caching correctly recognize "this is the same drive" again after
// a replug or a different computer, where the OS-assigned mount path can't.
// Best-effort: any failure to even check/show the prompt just lets the user
// continue unnamed rather than blocking USB use entirely.
async function promptDriveNameIfUnset(ctx) {
  const { state, el, command, document: doc } = ctx;
  const { emitStatus } = ctx;
  // Reset first: a stale name from whatever drive was connected before must
  // never linger on screen while this one's actual name is still unknown.
  state.usbDeviceName = null;
  ctx.updateUsbNameBadge();
  if (!state.usbRoot || typeof command !== "function") return;

  let existingName;
  let suggestedName = "";
  try {
    const data = await command("get_usb_device_name", { usbRoot: state.usbRoot });
    // Fail closed: only trust an explicit "name" field as the real answer.
    // A response that doesn't even look like GetUsbDeviceNameData (e.g. an
    // unmocked test double, or a future API change) must never be read as
    // "definitely unnamed" -- that would pop a prompt that blocks the whole
    // UI (see below) based on a guess instead of a real answer.
    if (!data || typeof data !== "object" || !("name" in data)) {
      console.warn("[usb] get_usb_device_name returned an unexpected shape, skipping naming prompt:", data);
      return;
    }
    existingName = data.name;
    suggestedName = String(data.suggestedName || "").trim();
  } catch (err) {
    console.warn("[usb] get_usb_device_name failed, skipping naming prompt:", err);
    emitStatus(`Could not check this drive's name: ${err?.message || err}`);
    return;
  }
  if (existingName) {
    state.usbDeviceName = existingName;
    ctx.updateUsbNameBadge();
    return;
  }

  const overlay = el.driveNameOverlay;
  if (!doc || !overlay || !el.driveNameInput || !el.driveNameOkBtn) {
    console.warn("[usb] drive-naming prompt DOM elements missing, skipping prompt", {
      hasDoc: !!doc,
      hasOverlay: !!overlay,
      hasInput: !!el.driveNameInput,
      hasOkBtn: !!el.driveNameOkBtn
    });
    return;
  }

  // This overlay is full-viewport and intercepts every click while open, so
  // it must never be able to get stuck there -- Escape, a backdrop click,
  // and an explicit "Not now" button all close it without a name, in
  // addition to Save. A prompt with no way out would silently freeze the
  // entire app if anything about it ever misbehaves.
  await new Promise((resolve) => {
    el.driveNameInput.value = suggestedName;
    if (el.driveNameError) el.driveNameError.hidden = true;
    overlay.hidden = false;
    el.driveNameInput.focus();
    el.driveNameInput.select();

    const cleanup = () => {
      overlay.hidden = true;
      el.driveNameOkBtn.removeEventListener("click", onSave);
      el.driveNameSkipBtn?.removeEventListener("click", onSkip);
      el.driveNameInput.removeEventListener("keydown", onEnter);
      doc.removeEventListener("keydown", onEscape);
      overlay.removeEventListener("click", onOverlayClick);
    };
    const onSave = async () => {
      const name = String(el.driveNameInput.value || "").trim();
      if (!name) {
        if (el.driveNameError) {
          el.driveNameError.textContent = "Enter a name for this drive.";
          el.driveNameError.hidden = false;
        }
        return;
      }
      try {
        await command("set_usb_device_name", { usbRoot: state.usbRoot, name });
        state.usbDeviceName = name;
        ctx.updateUsbNameBadge();
        cleanup();
        resolve();
      } catch (err) {
        if (el.driveNameError) {
          el.driveNameError.textContent = err?.message || String(err);
          el.driveNameError.hidden = false;
        }
      }
    };
    const onSkip = () => {
      cleanup();
      resolve();
    };
    const onEnter = (event) => {
      if (event.key === "Enter") onSave();
    };
    const onEscape = (event) => {
      if (event.key === "Escape") onSkip();
    };
    const onOverlayClick = (event) => {
      if (event.target === overlay) onSkip();
    };
    el.driveNameOkBtn.addEventListener("click", onSave);
    el.driveNameSkipBtn?.addEventListener("click", onSkip);
    el.driveNameInput.addEventListener("keydown", onEnter);
    doc.addEventListener("keydown", onEscape);
    overlay.addEventListener("click", onOverlayClick);
  });
}

export async function validateAndSetUsbRoot(ctx, path, silent = false) {
  const { state, el, command } = ctx;
  const { emitStatus } = ctx;

  if (isUsbRootChangeBlocked(state)) {
    if (!silent) emitStatus("Please wait for the current USB operation to finish before switching drives");
    return false;
  }

  const input = String(path || "").trim();
  const previousRoot = state.usbRoot;
  if (input && previousRoot && input !== previousRoot) {
    hideUsbDiagnostics(ctx);
  }
  if (!input) {
    state.usbRoot = null;
    state.usbRootValid = false;
      state.usbDeviceName = null;
    ctx.updateUsbNameBadge();
    ctx.persistUsbRoot(null);
    updateUsbRootText(ctx, null, false);
    el.usbInitRow.classList.add("hidden");
    resetUsbStateViews(ctx);
    updateUsbConfigControlsVisibility(ctx);
    ctx.updateUsbSubNavDisabledState();
    ctx.updatePlaylistExportButtons();
    if (!silent) emitStatus("USB root cleared");
    await syncAssetScopePaths(ctx);
    return false;
  }

  const result = await command("validate_usb_root", { path: input });
  const normalized = String(result?.normalizedRoot || "").trim();
  const valid = !!result?.valid && !!normalized;
  const hasStructureWarning = !result?.hasVendorRoot || !result?.hasContents || !result?.hasPdb;
  const canInitialize = !!normalized && !valid && !!result?.hasWriteAccess && hasStructureWarning;
  state.usbWritable = !!result?.hasWriteAccess;
  state.usbRootValid = valid;
  state.usbRoot = normalized || input;
  ctx.persistUsbRoot(state.usbRoot);
  updateUsbRootText(ctx, state.usbRoot, valid);
  if (el.usbInitRow) {
    el.usbInitRow.classList.toggle("hidden", !canInitialize);
  }
  if (el.usbInitHint) {
    const warningText = joinWarningTexts(result?.warnings);
    if (canInitialize) {
      const reason = warningText ? ` (${warningText})` : "";
      el.usbInitHint.textContent = `USB folder is writable but missing External library structure${reason}`;
    } else if (!valid) {
      const reason = warningText ? ` (${warningText})` : "";
      el.usbInitHint.textContent = `USB folder is not ready for initialization${reason}`;
    }
  }
  if (el.initializeUsbBtn) {
    el.initializeUsbBtn.disabled = !canInitialize;
  }
  if (previousRoot !== state.usbRoot) {
    resetUsbStateViews(ctx);
  }
  updateUsbConfigControlsVisibility(ctx);
  ctx.updateUsbSubNavDisabledState();
  ctx.updatePlaylistExportButtons();
  if (valid) {
    await promptDriveNameIfUnset(ctx);
  } else {
    // Invalid/uninitialized root: clear any name badge left over from
    // whatever drive was previously connected -- it no longer applies.
    state.usbDeviceName = null;
    ctx.updateUsbNameBadge();
  }
  if (!silent) {
    if (valid) {
      const selectedWarningText = joinWarningTexts(result?.warnings);
      const reason = selectedWarningText ? ` (${selectedWarningText})` : "";
      emitStatus(`USB root selected: ${state.usbRoot}${reason}. Running diagnostics...`);
      const healthCard = ctx.document?.getElementById?.("usbHealthCard") ?? null;
      if (healthCard) {
        healthCard.removeAttribute("open");
        healthCard.classList.add("is-loading");
      }
      // Next frame, so the "Running diagnostics..." state paints first.
      ctx.requestAnimationFrameFn(() => {
        ctx.runUsbDiagnostics().catch((err) => {
          ctx.warn("Auto-diagnostics failed:", err);
          emitStatus(`Auto-diagnostics failed: ${err?.message || err}`);
        });
      });
    } else if (canInitialize) {
      emitStatus('USB selected but not initialized. Click "Initialize USB Structure" to continue.');
    } else {
      const invalidWarningText = joinWarningTexts(result?.warnings) || "invalid USB root";
      emitStatus(`USB root invalid: ${invalidWarningText}`);
    }
  }
  if (state.usbRoot) await loadUsbDevices(ctx);
  await syncAssetScopePaths(ctx);
  return valid;
}

export async function removeUsbPlaylist(ctx, playlist) {
  const { state } = ctx;
  const { emitStatus } = ctx;

  if (!state.usbRoot) {
    emitStatus("Select USB folder first");
    return;
  }
  if (!playlist) {
    emitStatus("USB playlist not found");
    return;
  }

  const confirmed = await ctx.openConfirmDialog({
    title: "Remove USB Playlist",
    message: `Remove USB playlist "${playlist.name}" from the stick?`,
    confirmLabel: "Remove"
  });
  if (!confirmed) return;

  const data = await ctx.command("remove_usb_playlist", {
    usbRoot: state.usbRoot,
    playlistId: playlist.id,
    playlistName: playlist.name
  });
  clearUsbDiagnostics(ctx);
  await refreshUsb(ctx);
  const warningCount = ctx.countWarningsForStatus(data.warnings);
  const warningSuffix = warningCount ? ` | (${warningCount} warning(s))` : "";
  emitStatus(
    `Removed USB playlist: ${playlist.name} [db ${data.removedFromEdb || 0}, pdb ${data.removedFromPdb || 0}]${warningSuffix}`,
    { warningCount }
  );
}

export function moveArrayItem(list, fromIndex, toIndex) {
  const copy = list.slice();
  const [item] = copy.splice(fromIndex, 1);
  copy.splice(toIndex, 0, item);
  return copy;
}

export async function reorderUsbPlaylists(ctx) {
  const { state } = ctx;
  const { emitStatus } = ctx;

  if (!state.usbRoot || !state.usbRootValid) {
    emitStatus("Select USB folder first");
    return;
  }

  try {
    await ctx.command("reorder_usb_playlists", {
      usbRoot: state.usbRoot,
      orderedPlaylistIds: state.usbPlaylists.map((p) => p.id)
    });
    clearUsbDiagnostics(ctx);
    emitStatus("Playlist order saved");
  } catch (err) {
    emitStatus(`Failed to save playlist order: ${err.message || err}`);
  } finally {
    await refreshUsb(ctx);
  }
}
// USB workflow orchestration.

export async function refreshUsb(ctx) {
  const { state, el, setProgress, startProgressHeartbeat, stopProgressHeartbeat } = ctx;
  const { emitStatus } = ctx;
  if (!state.usbRoot) {
    emitStatus("Select USB folder first");
    return;
  }
  emitStatus("Loading USB playlists...");
  setProgress(true, 5, "Reading USB database...");
  startProgressHeartbeat();
  let data;
  try {
    data = await ctx.command("fetch_usb_playlists", {
      usbRoot: state.usbRoot
    });
  } catch (err) {
    stopProgressHeartbeat();
    setProgress(true, 100, "USB load failed", { error: true, dismissable: true });
    throw err;
  }
  stopProgressHeartbeat();

  state.usbPlaylists = (data.items || []).map((item) => ctx.normalizeUsbPlaylist(item));
  state.playlistUsbExportStatusById = playlistUsbExportStatusById(data.playlistUsbExportStatus);

  const usbTrackTotal = Number(data.playlistTrackTotal) || 0;
  el.usbCountsText.textContent = `${state.usbPlaylists.length} playlists, ${usbTrackTotal} tracks`;
  renderUsbPlaylists(ctx);
  ctx.usbPlaylistTracksCtl.clear();
  ctx.updatePlaylistExportButtons();
  // The freshly scanned status may flip an open local playlist's reorder lock.
  await ctx.renderCurrentPlaylistTracksFromState();

  const warningCount = ctx.countWarningsForStatus(data.warnings);
  const warningSuffix = warningCount ? ` | (${warningCount} warning(s))` : "";
  ctx.logWarnings("usb-import", data.warnings, "fetch_usb_playlists");
  setProgress(true, 100, `Done — ${state.usbPlaylists.length} playlists, ${usbTrackTotal} tracks`);
  emitStatus(`USB playlists loaded: ${state.usbPlaylists.length}${warningSuffix}`, { warningCount });
  ctx.scheduleProgressIdle(1200);
}

export async function runUsbDiagnostics(ctx) {
  const { state } = ctx;
  const { emitStatus } = ctx;
  if (!state.usbRoot) {
    emitStatus("Select USB folder first");
    return;
  }
  const healthCard = ctx.document?.getElementById?.("usbHealthCard") ?? null;
  if (healthCard) {
    healthCard.removeAttribute("open");
    healthCard.classList.add("is-loading");
  }
  emitStatus("Running USB diagnostics...");
  const data = await ctx.command("run_usb_diagnostics", {
    usbRoot: state.usbRoot
  });
  state.playlistUsbExportStatusById = playlistUsbExportStatusById(data?.playlistUsbExportStatus);
  ctx.updatePlaylistExportButtons();
  await ctx.renderCurrentPlaylistTracksFromState();
  renderDiagnosticsReport(ctx, data);
  ctx.logWarnings("usb-diagnostics", data.warnings, "run_usb_diagnostics");
  emitStatus(`Diagnostics complete (${data.durationMs}ms)`);
}

export async function runUsbParityReport(ctx) {
  const { state } = ctx;
  const { emitStatus } = ctx;
  if (!state.usbRoot) {
    emitStatus("Select USB folder first");
    return;
  }
  emitStatus("Running USB parity report...");
  const data = await ctx.command("run_usb_parity_report", {
    usbRoot: state.usbRoot
  });
  renderParityReport(ctx, data);
  ctx.logWarnings("usb-diagnostics", data.warnings, "run_usb_parity_report");
  emitStatus(`Parity report complete (${data.durationMs}ms)`);
}

export async function previewUsbRepairs(ctx) {
  const { state } = ctx;
  const { emitStatus } = ctx;
  if (!state.usbRoot) {
    emitStatus("Select USB folder first");
    return;
  }
  emitStatus("Previewing USB repair fixes...");
  const data = await ctx.command("repair_usb_diagnostics", {
    usbRoot: state.usbRoot
  });
  renderRepairPreview(ctx, data);
  ctx.logWarnings("usb-diagnostics", data.warnings, "repair_usb_diagnostics preview");
  emitStatus(`Repair preview ready (${data.durationMs}ms)`);
}

export async function applyUsbRepairs(ctx) {
  const { state } = ctx;
  const { emitStatus } = ctx;
  if (!state.usbRoot) {
    emitStatus("Select USB folder first");
    return;
  }
  emitStatus("Applying supported USB repair fixes...");
  const selectedFixIds = Array.from(state.selectedRepairFixIds);
  if (selectedFixIds.length === 0) {
    emitStatus("Select at least one fix to apply.");
    return;
  }
  const data = await ctx.command("repair_usb_diagnostics", {
    usbRoot: state.usbRoot,
    apply: true,
    selectedFixIds
  });
  const applied = Array.isArray(data.appliedFixes) ? data.appliedFixes.length : 0;
  const failed = Array.isArray(data.failedFixes) ? data.failedFixes.length : 0;
  // Some repair fixes rewrite the PDB playlist tree -- rather than track
  // which specific fix IDs touch playlists, treat any successful apply as
  // potentially invalidating whatever's loaded, same coarse-grained
  // "DB changed, clear it" reasoning diagnostics-clearing already uses.
  if (applied > 0) resetUsbStateViews(ctx, { hideDiagnostics: false });
  ctx.logWarnings("usb-diagnostics", data.warnings, "repair_usb_diagnostics apply");
  if (data.diagnostics) {
    state.playlistUsbExportStatusById = playlistUsbExportStatusById(
      data.diagnostics.playlistUsbExportStatus
    );
    ctx.updatePlaylistExportButtons();
    await ctx.renderCurrentPlaylistTracksFromState();
    renderDiagnosticsReport(ctx, data.diagnostics);
    ctx.logWarnings("usb-diagnostics", data.diagnostics.warnings, "run_usb_diagnostics");
  }
  emitStatus(`Repair apply complete: ${applied} applied, ${failed} failed (${data.durationMs}ms)${data.diagnostics ? ". Diagnostics refreshed." : ""}`);
}

export async function refreshHistory(ctx) {
  const { state, el } = ctx;
  const { emitStatus } = ctx;
  if (!state.usbRoot) {
    emitStatus("Select USB folder first");
    return;
  }
  emitStatus("Loading USB history...");
  const data = await ctx.command("fetch_usb_histories", { usbRoot: state.usbRoot });

  state.histories = (data.items || []).map((history) => ({
    ...history,
    tracks: (history.tracks || []).map((track) => ctx.normalizeTrack(track))
  }));
  // Backend-owned: `fetch_usb_histories` always returns `counts` computed over
  // the full import -- the frontend renders them, never re-tallies.
  const counts = data.counts || {};
  el.historyCountsText.textContent = `${counts.importedPlaylists || 0} sessions, ${counts.importedTracks || 0} tracks`;
  state.selectedHistoryIndex = null;
  state.historyTracks = [];
  if (el.exportHistoryTracklistBtn) el.exportHistoryTracklistBtn.disabled = true;
  renderHistoryList(ctx);
  ctx.usbHistoryTracksCtl.clear();
  const warningCount = ctx.countWarningsForStatus(data.warnings);
  const warningSuffix = warningCount ? ` | (${warningCount} warning(s))` : "";
  ctx.logWarnings("usb-import", data.warnings, "fetch_usb_histories");
  emitStatus(`USB histories loaded: ${state.histories.length}${warningSuffix}`, { warningCount });
}

export function sanitizeTracklistFileName(name) {
  const cleaned = String(name || "")
    .replace(/[\\/:*?"<>|]+/g, "-")
    .replace(/\s+/g, " ")
    .trim();
  return `${cleaned || "tracklist"}.txt`;
}

export async function exportHistoryTracklist(ctx) {
  const { state } = ctx;
  const { emitStatus } = ctx;

  const history = state.histories[state.selectedHistoryIndex];
  if (!history || !state.historyTracks.length) {
    emitStatus("Select a history session first");
    return;
  }

  const choice = await ctx.tracklistExportDialog.open({
    tracks: state.historyTracks,
    defaultTimesEnabled: true,
    defaultPlacement: "before"
  });
  if (!choice) return;

  const startIndex = Math.min(Math.max(Number(choice.startIndex) || 0, 0), state.historyTracks.length - 1);
  const text = buildTracklistText(state.historyTracks.slice(startIndex), choice.timeMode);
  const saved = await ctx.invoke("save_text_file", {
    suggestedFileName: sanitizeTracklistFileName(history.name),
    contents: text
  });
  emitStatus(saved ? `Tracklist exported: ${history.name}` : "Tracklist export cancelled");
}

function toMenuOptionLabel(item) {
  return String(item?.name || "").trim() || `Menu ${item?.kind ?? item?.menuItemId ?? ""}`;
}

function normalizeMenuKind(value) {
  if (value === null || value === undefined || value === "") return null;
  const n = Number(value);
  return Number.isFinite(n) && n >= 0 ? n : null;
}

function ensureValidPlayerMenuSelections(state) {
  const availableKinds = new Set(
    (state.usbPlayerMenuAvailable || []).map((item) => Number(item.kind)),
  );
  const currentKinds = new Set(
    (state.usbPlayerMenuCurrent || []).map((item) => Number(item.kind)),
  );
  if (!availableKinds.has(Number(state.usbPlayerMenuAvailableSelectedKind))) {
    state.usbPlayerMenuAvailableSelectedKind = null;
  }
  if (!currentKinds.has(Number(state.usbPlayerMenuCurrentSelectedKind))) {
    state.usbPlayerMenuCurrentSelectedKind = null;
  }
}

function buildPlayerMenuItemButton(documentObj, item, selectedKind, side) {
  const kind = Number(item?.kind);
  const origin = item?.origin || "both";
  const button = cloneTemplate(documentObj, "tplPlayerMenuItem");
  button.dataset.menuKind = String(kind);
  button.dataset.menuSide = side;
  button.dataset.menuOrigin = origin;
  button.querySelector(".player-menu-item-label").textContent = toMenuOptionLabel(item);

  const tag = button.querySelector(".player-menu-item-origin");
  if (side === "current" && origin !== "both") {
    tag.classList.add(`is-${origin}`);
    tag.textContent = origin === "pdb_only" ? "PDB" : "eDB";
    tag.dataset.tooltip = origin === "pdb_only"
      ? "Only in PDB t16 (eDB missing this kind)"
      : "Only in eDB menuItem (not in PDB t16)";
  } else {
    tag.remove();
  }

  const selected = Number(selectedKind) === kind;
  if (selected) {
    button.classList.add("is-selected");
    button.setAttribute("aria-selected", "true");
  } else {
    button.setAttribute("aria-selected", "false");
  }
  return button;
}

export function handleUsbPlayerMenuListClick(ctx, side, event) {
  const { state } = ctx;
  const target = event?.target?.closest?.(".player-menu-item");
  if (!target) return;
  const kind = normalizeMenuKind(target.dataset.menuKind);
  if (kind === null) return;
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
  if (!availableEl || !currentEl) return;

  ensureValidPlayerMenuSelections(state);

  availableEl.replaceChildren();
  for (const item of state.usbPlayerMenuAvailable || []) {
    const row = buildPlayerMenuItemButton(
      document,
      item,
      state.usbPlayerMenuAvailableSelectedKind,
      "available",
    );
    availableEl.appendChild(row);
  }

  currentEl.replaceChildren();
  for (const item of state.usbPlayerMenuCurrent || []) {
    const row = buildPlayerMenuItemButton(
      document,
      item,
      state.usbPlayerMenuCurrentSelectedKind,
      "current",
    );
    currentEl.appendChild(row);
  }

  renderUsbPlayerMenuDivergence(ctx);
  syncUsbPlayerMenuEditorControls(ctx);
}

function renderUsbPlayerMenuDivergence(ctx) {
  const { state, el } = ctx;
  const node = el.usbPlayerMenuDivergence;
  if (!node) return;
  // Backend-owned: `summary` / `canSync` / `canRestore` come from
  // service::repair (load_usb_player_menu_config). `canFix` is frontend state
  // (a valid USB must be selected to run either action).
  const div = state.usbPlayerMenuDivergence || {};
  const canFix = !!(state.usbRoot && state.usbRootValid);
  if (!canFix || (!div.canSync && !div.canRestore)) {
    node.classList.add("hidden");
    if (el.usbPlayerMenuDivergenceMessage) el.usbPlayerMenuDivergenceMessage.textContent = "";
    if (el.usbPlayerMenuSyncBtn) el.usbPlayerMenuSyncBtn.disabled = true;
    if (el.usbPlayerMenuRestoreBtn) el.usbPlayerMenuRestoreBtn.disabled = true;
    return;
  }
  node.classList.remove("hidden");
  const msg = String(div.summary || "");
  if (el.usbPlayerMenuDivergenceMessage) {
    el.usbPlayerMenuDivergenceMessage.textContent = msg;
  } else {
    node.textContent = msg;
  }
  if (el.usbPlayerMenuSyncBtn) {
    el.usbPlayerMenuSyncBtn.disabled = !div.canSync;
  }
  if (el.usbPlayerMenuRestoreBtn) {
    el.usbPlayerMenuRestoreBtn.disabled = !div.canRestore;
  }
}

// Store a player-menu config response and redraw the editor.
function applyPlayerMenuConfig(ctx, data, selection = null) {
  const { state } = ctx;
  state.usbPlayerMenuCurrent = Array.isArray(data?.currentItems) ? data.currentItems : [];
  state.usbPlayerMenuAvailable = Array.isArray(data?.availableItems) ? data.availableItems : [];
  state.usbPlayerMenuDivergence = data?.divergence ?? null;
  state.usbPlayerMenuCurrentSelectedKind = selection?.side === "current" ? normalizeMenuKind(selection.kind) : null;
  state.usbPlayerMenuAvailableSelectedKind = selection?.side === "available" ? normalizeMenuKind(selection.kind) : null;
  renderUsbPlayerMenuEditor(ctx);
}

export async function syncUsbPlayerMenusEdbToPdb(ctx) {
  const { state } = ctx;
  const { emitStatus } = ctx;
  if (!state.usbRoot || !state.usbRootValid) {
    emitStatus("Select USB folder first");
    return;
  }
  emitStatus("Fixing PDB sync...");
  const data = await ctx.command("sync_usb_player_menu_edb_to_pdb", { usbRoot: state.usbRoot });
  applyPlayerMenuConfig(ctx, data);
  if (data?.updated) clearUsbDiagnostics(ctx);
  emitStatus(data?.updated ? "PDB categories restored" : "PDB already complete");
}

// Whether the currently-selected `current` menu item may be removed is a
// backend-owned fact (`UsbPlayerMenuItem.removable`, see
// backend/src/service/repair.rs). The frontend just reads it.
function currentPlayerMenuItemByKind(state, kind) {
  return (state.usbPlayerMenuCurrent || []).find((item) => Number(item.kind) === kind) || null;
}

export function syncUsbPlayerMenuEditorControls(ctx) {
  const { state, el } = ctx;
  const availableEl = el.usbPlayerMenuAvailable;
  const currentEl = el.usbPlayerMenuCurrent;
  if (!availableEl || !currentEl) return;

  const hasRoot = !!state.usbRoot && !!state.usbRootValid;

  const availableSelected = normalizeMenuKind(state.usbPlayerMenuAvailableSelectedKind);
  const currentSelected = normalizeMenuKind(state.usbPlayerMenuCurrentSelectedKind);
  const currentKinds = (state.usbPlayerMenuCurrent || []).map((item) => Number(item.kind));
  const currentIdx = currentSelected !== null ? currentKinds.indexOf(currentSelected) : -1;

  const hasAvailable = availableSelected !== null;
  const hasCurrent = currentSelected !== null;
  if (el.usbPlayerMenuAddBtn) el.usbPlayerMenuAddBtn.disabled = !hasRoot || !hasAvailable;
  if (el.usbPlayerMenuRemoveBtn)
    el.usbPlayerMenuRemoveBtn.disabled =
      !hasRoot || !hasCurrent
      || currentPlayerMenuItemByKind(state, currentSelected)?.removable === false;
  if (el.usbPlayerMenuUpBtn) el.usbPlayerMenuUpBtn.disabled = !hasRoot || currentIdx <= 0;
  if (el.usbPlayerMenuDownBtn) {
    el.usbPlayerMenuDownBtn.disabled = !hasRoot || currentIdx < 0 || currentIdx >= currentKinds.length - 1;
  }
}

export async function loadUsbPlayerMenuConfig(ctx) {
  const { state } = ctx;
  const { emitStatus } = ctx;
  if (!state.usbRoot || !state.usbRootValid) {
    emitStatus("Select USB folder first");
    renderUsbPlayerMenuEditor(ctx);
    return;
  }
  emitStatus("Loading player menu configuration...");
  const data = await ctx.command("get_usb_player_menu_config", { usbRoot: state.usbRoot });
  applyPlayerMenuConfig(ctx, data);
  emitStatus("Player menu loaded");
}

async function updateUsbPlayerMenuConfig(ctx, currentKinds, preferredSelection = null) {
  const { state } = ctx;
  const { emitStatus } = ctx;
  if (!state.usbRoot || !state.usbRootValid) {
    emitStatus("Select USB folder first");
    return;
  }
  const data = await ctx.command("update_usb_player_menu_config", {
    usbRoot: state.usbRoot,
    currentKinds,
  });
  if (data?.updated) clearUsbDiagnostics(ctx);
  applyPlayerMenuConfig(ctx, data, preferredSelection);
  emitStatus(data?.updated ? "Player menu updated" : "Player menu unchanged");
}

export async function addUsbPlayerMenuItems(ctx) {
  const { state } = ctx;
  const selected = normalizeMenuKind(state.usbPlayerMenuAvailableSelectedKind);
  if (selected === null) return;
  const currentKinds = (state.usbPlayerMenuCurrent || []).map((item) => Number(item.kind));
  if (!currentKinds.includes(selected)) {
    currentKinds.push(selected);
  }
  await updateUsbPlayerMenuConfig(ctx, currentKinds, {
    side: "current",
    kind: selected,
  });
}

export async function removeUsbPlayerMenuItems(ctx) {
  const { state } = ctx;
  const selected = normalizeMenuKind(state.usbPlayerMenuCurrentSelectedKind);
  if (selected === null) return;
  // Belt-and-suspenders: the Remove button is already disabled for these, and
  // update_usb_player_menu_config rejects the request backend-side.
  if (currentPlayerMenuItemByKind(state, selected)?.removable === false) return;
  const currentKinds = (state.usbPlayerMenuCurrent || [])
    .map((item) => Number(item.kind))
    .filter((kind) => kind !== selected);
  await updateUsbPlayerMenuConfig(ctx, currentKinds, {
    side: "available",
    kind: selected,
  });
}

export async function moveUsbPlayerMenuItems(ctx, direction) {
  const { state } = ctx;
  const selected = normalizeMenuKind(state.usbPlayerMenuCurrentSelectedKind);
  if (selected === null) return;
  const currentKinds = (state.usbPlayerMenuCurrent || []).map((item) => Number(item.kind));
  const selectedIdx = currentKinds.indexOf(selected);
  if (selectedIdx < 0) return;

  const swapIdx = direction < 0 ? selectedIdx - 1 : selectedIdx + 1;
  if (swapIdx >= 0 && swapIdx < currentKinds.length) {
    [currentKinds[swapIdx], currentKinds[selectedIdx]] = [currentKinds[selectedIdx], currentKinds[swapIdx]];
  }
  await updateUsbPlayerMenuConfig(ctx, currentKinds, {
    side: "current",
    kind: selected,
  });
}

export async function exportPlaylistToUsb(ctx, playlistId) {
  const { state, el, setProgress, stopProgressHeartbeat } = ctx;
  const { emitStatus } = ctx;
  const playlist = state.playlists.find((item) => item.id === playlistId);
  if (!playlist) return;
  try {
    await ctx.commitActivePlaylistSort(playlistId);
  } catch (err) {
    emitStatus(`Export blocked: couldn't save the current sort order (${err.message || err})`);
    return;
  }
  if (!playlist.tracks?.length) {
    emitStatus("Playlist must contain tracks before export");
    return;
  }
  // A missing source folder that a playlist track lives under is caught by the
  // backend export gate (validationType "source_root_missing"), which sees the
  // whole playlist rather than just the loaded page -- handled below.
  if (!state.usbRoot || !state.usbRootValid) {
    emitStatus("Select a valid USB folder first");
    return;
  }
  if (!state.usbWritable) {
    emitStatus("USB is read-only. Remount as read-write before export.");
    return;
  }

  emitStatus(`Exporting ${playlist.name} to USB...`);
  el.donateBtn?.classList.add("exporting");
  setProgress(true, 8, "Starting USB export...");
  ctx.startProgressHeartbeat();
  await ctx.nextPaint();
  let data;
  try {
    data = await ctx.command("export_to_usb", {
      usbRoot: state.usbRoot,
      playlistId: playlist.id,
      options: {
        includeArtwork: true,
        includeAnalysis: true,
        pruneStale: !!state.exportPruneStale,
        backupBeforeExport: !!state.exportBackup
      }
    });
  } catch (error) {
    const details = error?.details || null;
    if (details?.validationType === "missing_analysis") {
      const missing = Number(details.missingTrackCount || 0);
      const total = Number(details.totalTrackCount || 0);
      emitStatus(`Export blocked: ${missing}/${total} track(s) need analysis. Use Analyze Missing Tracks.`);
    } else if (details?.validationType === "source_root_missing") {
      const missingRoot = Array.isArray(details.missingRoots) ? details.missingRoots[0] : "";
      const suffix = missingRoot ? `: ${missingRoot}` : "";
      emitStatus(`Export blocked: source folder is missing${suffix}. Relocate or remove it first.`);
    } else {
      const msg = String(error?.message || "USB export failed").trim() || "USB export failed";
      emitStatus(`Export failed: ${msg}. See Event Log for details.`);
      ctx.emitMessage({
        level: "error",
        source: "export",
        code: "export.failure",
        eventLog: { text: msg, details: "context: export_to_usb", coalesceKey: "export.failure.export_to_usb" }
      });
    }
    throw error;
  } finally {
    if (!state.activeJobId) {
      setProgress(false, 0, "Idle");
      stopProgressHeartbeat();
    }
  }
  clearUsbDiagnostics(ctx);
  const warningCount = ctx.countWarningsForStatus(data.warnings);
  const warningSuffix = warningCount ? ` | (${warningCount} warning(s))` : "";
  const warningList = Array.isArray(data.warnings) ? data.warnings : [];
  if (warningList.length) {
    const infoCount = warningList.filter((entry) => ctx.warningEntryLevel(entry) === "info").length;
    if (warningCount > 0) {
      console.warn(
        `Export completed with ${warningCount} warning/error entr${warningCount === 1 ? "y" : "ies"}${infoCount ? ` (+${infoCount} info)` : ""}.`
      );
    } else {
      console.info(
        `Export completed with ${infoCount} informational entr${infoCount === 1 ? "y" : "ies"}.`
      );
    }
  }
  ctx.logWarnings("export", data.warnings, "export_to_usb");
  emitStatus(
    `Export complete: ${playlist.name} - ${data.exportedTracks || 0} track(s), ${data.skippedTracks || 0} skipped${warningSuffix}${state.exportPruneStale ? " [sync: mirror]" : " [sync: additive]"}`,
    { warningCount }
  );
  await ctx.loadPlaylists();
  state.currentPlaylistId = playlistId;
  ctx.updateModeText();
  await ctx.switchView(playlistId);

  state.usbPlaylists = [];
  renderUsbPlaylists(ctx);
  ctx.usbPlaylistTracksCtl.clear();
}

// A USB side list (playlists / history): the items, or `emptyText` with the
// right-hand track pane hidden when there are none.
function renderUsbSideList(list, items, emptyText) {
  const doc = list.ownerDocument;
  const right = list.closest(".split")?.querySelector(".right");
  right?.classList.toggle("hidden", !items.length);
  if (!items.length) {
    const empty = cloneTemplate(doc, "tplMutedListItem");
    empty.textContent = emptyText;
    list.replaceChildren(empty);
    return;
  }
  list.replaceChildren(...items);
}

export function renderUsbPlaylists(ctx) {
  const { state, el, document } = ctx;
  renderUsbSideList(el.usbPlaylists, state.usbPlaylists.map((playlist, index) => {
    const count = Number(playlist.trackCount ?? playlist.tracks?.length ?? 0);
    const li = cloneTemplate(document, "tplUsbPlaylistItem");
    li.dataset.usbPlaylistLi = String(index);
    const btn = li.firstElementChild;
    btn.dataset.usbPlaylistIndex = String(index);
    btn.dataset.usbPlaylist = playlist.id;
    btn.querySelector(".playlist-label").textContent = `${playlist.name} (${count})`;
    btn.querySelector(".playlist-remove").dataset.usbRemovePlaylist = playlist.id;
    return li;
  }), 'No playlists imported yet. Click "Import Playlists" to load from USB.');
}

export function renderHistoryList(ctx) {
  const { state, el, document } = ctx;
  // Render newest first — keep original index so click handler resolves state.histories[index]
  const items = state.histories.map((history, index) => {
    const dateText = getHistoryDateValue(history);
    const li = cloneTemplate(document, "tplHistoryItem");
    li.firstElementChild.dataset.historyIndex = String(index);
    li.querySelector(".playlist-label").textContent = `${history.name}${dateText ? ` (${dateText})` : ""}`;
    return li;
  }).reverse();
  renderUsbSideList(el.historyList, items, 'No history imported yet. Click "Import History" to load from USB.');
}

// The USB-playlist and USB-history track tables' data layer: paginated +
// searched + sorted + per-page-hydrated by the backend, rendered via the shared
// controller. Selection/search/sort/scroll all go through it.
function createUsbTracksController(ctx, { bodyId, fetchCommand, secondaryActionType, actionType }) {
  return createTrackListController({
    bodyId,
    getElements: () => ({
      body: ctx.el[bodyId],
      wrap: ctx.el[bodyId]?.closest?.(".table-wrap"),
      durationTarget: bodyId === "historyTracks" ? ctx.el.historyTotalDuration : ctx.el.usbPlaylistTotalDuration,
    }),
    fetchPage: ({ scopeId, query, sortBy, sortDir, cursor, limit }) =>
      ctx.command(fetchCommand, {
        usbRoot: ctx.state.usbRoot || null,
        id: scopeId,
        query,
        sortBy: sortBy || null,
        sortDir: sortDir || null,
        cursor: cursor || null,
        limit,
      }),
    normalize: (track) => ctx.normalizeTrack(track),
    rowOptions: () => ({
      withCheckbox: false,
      actionLabel: "+",
      actionType,
      compactAddButton: true,
      enableAnalyzeActions: true,
      origin: "usb",
      secondaryActionLabel: "Play",
      secondaryActionType,
    }),
    renderTrackTable: ctx.renderTrackTable,
    renderDurationSummary: (target, summary) =>
      renderTrackListDurationSummary(target, summary, formatDurationMs),
    getTableSortState: () => ctx.tableSortState,
  });
}

export function createUsbPlaylistTracksController(ctx) {
  return createUsbTracksController(ctx, {
    bodyId: "usbPlaylistTracks",
    fetchCommand: "fetch_usb_playlist_tracks",
    actionType: "add-usb",
    secondaryActionType: "play-usb",
  });
}

export function createUsbHistoryTracksController(ctx) {
  return createUsbTracksController(ctx, {
    bodyId: "historyTracks",
    fetchCommand: "fetch_usb_history_tracks",
    actionType: "add-history",
    secondaryActionType: "play-history",
  });
}

export function patchUsbTrackRow(ctx, track) {
  return patchUsbRowsInContainer(ctx, ctx.el.usbPlaylistTracks, track);
}

export function patchHistoryTrackRow(ctx, track) {
  return patchUsbRowsInContainer(ctx, ctx.el.historyTracks, track);
}

export async function initializeUsb(ctx) {
  const { state, el } = ctx;
  const { emitStatus } = ctx;
  if (!state.usbRoot) return;
  try {
    await ctx.command("initialize_usb", { usbRoot: state.usbRoot });
    emitStatus("USB initialized");
    el.usbInitRow?.classList?.add("hidden");
    await validateAndSetUsbRoot(ctx, state.usbRoot, false);
  } catch (err) {
    ctx.logError("Initialize USB failed:", err);
    emitStatus(`Initialize failed: ${err.message || err}`);
  }
}

export async function pickUsbFolder(ctx) {
  if (isUsbRootChangeBlocked(ctx.state)) {
    ctx.emitStatus("Please wait for the current USB operation to finish before switching drives");
    return null;
  }
  const selected = await ctx.invoke("pick_usb_folder");
  if (!selected) return null;
  await validateAndSetUsbRoot(ctx, String(selected), false);
  return selected;
}

export async function hydrateUsbTrackMetadata(ctx, track) {
  // Backend-owned: `needsHydration` (service::usb::hydrate_usb_track_in_place)
  // says whether an inspect could still fill anything in.
  if (!track || track.needsHydration !== true) return track;
  const trackId = String(track.id || "").trim();
  if (!/^\d+$/.test(trackId)) return track;
  try {
    const inspected = await ctx.command("inspect_usb_track", {
      usbRoot: ctx.state.usbRoot,
      trackId,
      filePath: track.filePath || "",
      title: track.title || "",
      artist: track.artist || ""
    });
    applyHydratedTrackResult(ctx, track, inspected?.track);
  } catch (err) {
    console.warn(`inspect_usb_track failed for ${trackId}:`, err);
  }
  // We've inspected this row -- don't ask again even if fields are still blank.
  track.needsHydration = false;
  return track;
}

function applyHydratedTrackResult(ctx, track, inspectedTrack) {
  if (!inspectedTrack || typeof inspectedTrack !== "object") {
    track.artworkChecked = true;
    return;
  }
  const normalized = ctx.normalizeTrack({ ...track, ...inspectedTrack });
  if (!normalized.localTrackId && track.localTrackId) {
    normalized.localTrackId = track.localTrackId;
  }
  normalized.artworkChecked = true;
  Object.assign(track, normalized);
}

// Replaces the old localStorage/app_settings "recent USB roots" list with a
// live query against the usb_devices table -- so mount state and pruning
// are always accurate, not a client-side cache that can drift from it.
export async function loadUsbDevices(ctx) {
  const { state } = ctx;
  try {
    const data = await ctx.command("list_usb_devices");
    const items = Array.isArray(data?.items) ? data.items : [];
    state.usbDevices = items;
    state.usbRecentRoots = items.map((item) => String(item?.rootPath || "").trim()).filter(Boolean);
  } catch (err) {
    console.warn("Failed to load USB devices:", err);
    state.usbDevices = [];
    state.usbRecentRoots = [];
  }
  renderUsbRecentRoots(ctx);
  return state.usbRecentRoots;
}

export async function pruneUsbDevice(ctx, id) {
  if (!id) return;
  try {
    await ctx.command("prune_usb_device", { id });
  } catch (err) {
    console.warn(`Failed to prune USB device ${id}:`, err);
  }
  await loadUsbDevices(ctx);
}

export function renderUsbRecentRoots(ctx) {
  const { state, el, document } = ctx;
  if (!el?.usbRecentRow || !el?.usbRecentList) return;
  el.usbRecentList.replaceChildren();
  const rows = state.usbRecentRoots;
  const normalizedRows = Array.isArray(rows)
    ? rows.filter((row) => String(row || "").trim().length > 0)
    : [];
  if (!normalizedRows.length) {
    el.usbRecentRow.classList.add("hidden");
    return;
  }
  el.usbRecentRow.classList.remove("hidden");
  const locked = isUsbRootChangeBlocked(state);
  const devicesByPath = new Map(
    (Array.isArray(state.usbDevices) ? state.usbDevices : []).map((d) => [String(d?.rootPath || "").trim(), d])
  );
  normalizedRows.forEach((path) => {
    const row = cloneTemplate(document, "tplUsbRecentItem");
    const btn = row.querySelector(".usb-cfg-recent-btn");
    btn.dataset.usbRecentPath = path;
    btn.dataset.tooltip = path;
    btn.textContent = path;
    btn.disabled = locked;

    const pruneBtn = row.querySelector(".usb-cfg-recent-prune-btn");
    const device = devicesByPath.get(path);
    if (device?.id) {
      pruneBtn.dataset.usbPruneDeviceId = device.id;
      pruneBtn.setAttribute("aria-label", `Forget ${path}`);
      pruneBtn.disabled = locked;
    } else {
      pruneBtn.remove();
    }
    el.usbRecentList.appendChild(row);
  });
}

export function updateUsbRootText(ctx, path, valid = false) {
  const { el } = ctx;
  if (!el?.usbRootPathText) return;
  if (el.usbConnectionBar) {
    el.usbConnectionBar.classList.remove("hidden");
  }
  if (!valid) {
    el.usbRootPathText.textContent = "No USB selected";
    el.usbRootPathText.classList.remove("usb-path-valid", "usb-path-invalid");
    return;
  }
  el.usbRootPathText.textContent = path;
  el.usbRootPathText.classList.add("usb-path-valid");
  el.usbRootPathText.classList.remove("usb-path-invalid");
}
