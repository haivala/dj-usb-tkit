import {
  missingSourceRootsArray,
  playlistTracksAffectedByMissingRoots,
  warningEntryText
} from "../library/actions.mjs";
import { resolveEmitStatus } from "../shared/track_actions.mjs";

// Job types that scope a Tauri command to state.usbRoot -- while one of
// these is running, the currently-selected root must not change underneath
// it, or an in-flight response (e.g. a parity report) can land after the
// user has already switched to a different drive and get rendered as if it
// belonged to the new one.
export const USB_ROOT_LOCKING_JOB_TYPES = new Set(["usb_read", "usb_write", "diagnostics", "export"]);

export function isUsbRootChangeBlocked(state) {
  return !!state.activeJobId && USB_ROOT_LOCKING_JOB_TYPES.has(state.activeJobType);
}

export function setUsbRootControlsLocked(state, el, locked, deps = {}) {
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
    deps.updatePlaylistExportButtons?.();
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
export async function refreshPlaylistExportStatus(state, deps = {}) {
  const { command } = deps;
  const data = await command("refresh_playlist_export_status", {
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
  currentPlaylistName,
  playlistUsbExportStatusById: statusById
}) {
  const enabled = !!usbRoot && !!usbRootValid;
  const currentName = String(currentPlaylistName || "").trim();
  const status = statusById instanceof Map ? statusById.get(String(currentPlaylistId || "")) : null;
  // `locksReorder` is exactly "additive export mode AND same-named playlist
  // already on USB" (see PlaylistUsbExportStatus) -- the same condition that
  // makes an export here an append rather than a fresh write.
  const appendModeToExisting = enabled && !!status?.locksReorder;

  const lastDir = enabled
    ? String(usbRoot).replace(/[\\/]+$/, "").split(/[\\/]/).pop() || ""
    : "";

  let text;
  if (!enabled) {
    text = "Select USB first";
  } else if (appendModeToExisting) {
    text = `Append to (${currentName}) on USB: (${lastDir})`;
  } else {
    text = lastDir ? `Export to USB: ${lastDir}` : "Export to USB";
  }

  return {
    enabled,
    text,
    title: enabled
      ? (appendModeToExisting
        ? `Append current playlist tracks to existing USB playlist "${currentName}"`
        : "Export current playlist to selected USB")
      : "Select a valid USB folder first"
  };
}
// Backend-owned: `UsbParityPlaylistDetail.issueLabels` is built in Rust
// (service::diagnostics::parity_issue_labels). The frontend renders them.
export function formatParityIssues(pd) {
  return Array.isArray(pd?.issueLabels) ? pd.issueLabels : [];
}
export function diagStatusIcon(status) {
  if (status === "PASS") return "\u2713";
  if (status === "WARN") return "\u26A0";
  return "\u2717";
}

function renderDiagCheckRow(container, check, deps = {}) {
  const { escapeHtml, documentObj, switchView } = deps;
  const doc = documentObj || document;
  const row = doc.createElement("div");
  row.className = `diag-check diag-check-${check.status.toLowerCase()}`;
  row.innerHTML = `<span class="diag-indicator">${diagStatusIcon(check.status)}</span> <strong>${escapeHtml(check.label)}</strong>: ${escapeHtml(check.detail)}`;
  if (check.link === "event-log" && typeof switchView === "function") {
    const btn = doc.createElement("button");
    btn.className = "diag-log-link";
    btn.textContent = "→ event log";
    btn.addEventListener("click", () => switchView("event-log").catch((err) => console.error(err)));
    row.appendChild(btn);
  }
  container.appendChild(row);
}

export function renderDiagnosticsReport(el, data, deps = {}) {
  const { escapeHtml, showDiagReportView: showReport, updateUsbHealthDot, switchView } = deps;
  el.usbDiagnosticsCard.classList.remove("hidden");
  showReport();
  el.previewRepairsBtn.disabled = false;
  updateUsbHealthDot(data.overallStatus);

  const healthCard = (deps.documentObj || document).getElementById("usbHealthCard");
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

  el.diagSections.innerHTML = "";
  for (const sec of sections) {
    const div = (deps.documentObj || document).createElement("div");
    div.className = "diag-section";

    const header = (deps.documentObj || document).createElement("h3");
    header.innerHTML = `<span class="diag-dot diag-${sec.status.toLowerCase()}"></span> ${escapeHtml(sec.title)}`;
    div.appendChild(header);

    for (const check of (sec.checks || [])) {
      renderDiagCheckRow(div, check, { escapeHtml, documentObj: deps.documentObj, switchView });
    }

    el.diagSections.appendChild(div);
  }

  if (data.playlistDetails?.length) {
    el.diagPlaylistDetails.classList.remove("hidden");
    const summary = el.diagPlaylistDetails.querySelector("summary");
    if (summary) summary.textContent = "Playlist Resolution Details";
    const thead = el.diagPlaylistDetails.querySelector("thead tr");
    if (thead) thead.innerHTML = "<th>Status</th><th>Playlist</th><th>Resolved</th><th>Total</th><th>Rate</th>";
    el.diagPlaylistTableBody.innerHTML = "";
    for (const pd of data.playlistDetails) {
      const tr = (deps.documentObj || document).createElement("tr");
      tr.innerHTML = `<td><span class="diag-dot diag-${pd.status.toLowerCase()}"></span></td><td>${escapeHtml(pd.name)}</td><td>${pd.resolvedEntries}</td><td>${pd.totalEntries}</td><td>${(pd.resolutionRate * 100).toFixed(1)}%</td>`;
      el.diagPlaylistTableBody.appendChild(tr);
    }
  } else {
    el.diagPlaylistDetails.classList.add("hidden");
  }
}

export function renderParityReport(el, data, deps = {}) {
  const { escapeHtml, showDiagReportView: showReport, formatParityIssues } = deps;
  el.usbDiagnosticsCard.classList.remove("hidden");
  showReport();
  el.previewRepairsBtn.disabled = false;
  el.diagOverallStatus.textContent = data.overallStatus;
  el.diagOverallStatus.className = `diag-badge diag-${data.overallStatus.toLowerCase()}`;
  el.diagDuration.textContent = `Completed in ${data.durationMs}ms`;

  const section = {
    title: "USB Strict Parity Report",
    status: data.overallStatus,
    checks: data.checks || []
  };
  el.diagSections.innerHTML = "";
  const div = (deps.documentObj || document).createElement("div");
  div.className = "diag-section";
  const header = (deps.documentObj || document).createElement("h3");
  header.innerHTML = `<span class="diag-dot diag-${section.status.toLowerCase()}"></span> ${escapeHtml(section.title)}`;
  div.appendChild(header);
  if (Array.isArray(data.summaryRows) && data.summaryRows.length) {
    const summaryTitle = (deps.documentObj || document).createElement("h4");
    summaryTitle.textContent = "Parity Summary";
    div.appendChild(summaryTitle);
    const table = (deps.documentObj || document).createElement("table");
    table.className = "diag-table";
    table.innerHTML = "<thead><tr><th>Status</th><th>Metric</th><th>Count</th></tr></thead>";
    const tbody = (deps.documentObj || document).createElement("tbody");
    for (const row of data.summaryRows) {
      const tr = (deps.documentObj || document).createElement("tr");
      tr.innerHTML = `<td><span class="diag-dot diag-${String(row.status || "PASS").toLowerCase()}"></span> ${escapeHtml(String(row.status || "PASS"))}</td><td>${escapeHtml(row.label || "")}</td><td>${Number(row.count || 0)}</td>`;
      tbody.appendChild(tr);
    }
    table.appendChild(tbody);
    div.appendChild(table);
  }
  for (const check of section.checks) {
    renderDiagCheckRow(div, check, { escapeHtml, documentObj: deps.documentObj });
  }
  el.diagSections.appendChild(div);

  if (data.playlistDetails?.length) {
    el.diagPlaylistDetails.classList.remove("hidden");
    const summary = el.diagPlaylistDetails.querySelector("summary");
    if (summary) summary.textContent = "Strict Parity Playlist Details";
    const thead = el.diagPlaylistDetails.querySelector("thead tr");
    if (thead) thead.innerHTML = "<th>Status</th><th>Playlist</th><th class=\"num\">PDB</th><th class=\"num\">eDB</th><th class=\"num\">Matched</th><th>Issues</th>";
    el.diagPlaylistTableBody.innerHTML = "";
    for (const pd of data.playlistDetails) {
      const issues = formatParityIssues(pd);
      const issueText = issues.length
        ? `<span class="muted">${escapeHtml(issues.join(", "))}</span>`
        : "";
      const tr = (deps.documentObj || document).createElement("tr");
      tr.innerHTML = [
        `<td><span class="diag-dot diag-${String(pd.status || "PASS").toLowerCase()}"></span></td>`,
        `<td>${escapeHtml(pd.name)}</td>`,
        `<td class="num">${Number(pd.pdbTracks || 0)}</td>`,
        `<td class="num">${Number(pd.edbTracks || 0)}</td>`,
        `<td class="num">${pd.matchedTracks}</td>`,
        `<td>${issueText}</td>`,
      ].join("");
      el.diagPlaylistTableBody.appendChild(tr);
    }
  } else {
    el.diagPlaylistDetails.classList.add("hidden");
  }
}

export function showDiagReportView(el) {
  el.diagReportView.classList.remove("hidden");
  el.diagRepairPanel.classList.add("hidden");
}

export function showDiagRepairView(el) {
  el.diagReportView.classList.add("hidden");
  el.diagRepairPanel.classList.remove("hidden");
}

// Blanks the diagnostics report content back to an empty/unknown state
// without touching whether the panel itself is shown or collapsed. Use this
// when the USB DBs changed underneath an on-screen report (repair, playlist
// edit, export, backup restore, ...) but the same drive is still selected --
// the stale report should disappear, not the whole panel.
function resetDiagnosticsContent(el) {
  [el.usbHealthDot, el.usbHeaderHealthDot].filter(Boolean).forEach((dot) => {
    dot.classList.remove("health-pass", "health-warn", "health-fail");
    dot.dataset.tooltip = "USB health: unknown";
    dot.setAttribute("aria-label", "USB health: unknown");
  });
  if (el.diagSections) {
    el.diagSections.innerHTML = "";
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
    el.diagPlaylistTableBody.innerHTML = "";
  }
  if (el.diagRepairSummary) {
    el.diagRepairSummary.textContent = "";
    el.diagRepairSummary.className = "diag-repair-summary";
  }
  if (el.diagRepairFixes) {
    el.diagRepairFixes.innerHTML = "";
  }
  if (el.previewRepairsBtn) {
    el.previewRepairsBtn.disabled = true;
  }
  if (el.applyRepairsBtn) {
    el.applyRepairsBtn.disabled = true;
  }
  if (el.diagReportView && el.diagRepairPanel) {
    showDiagReportView(el);
  }
}

// Clears a stale diagnostics report in place -- the DBs changed but the same
// USB drive is still selected, so leave the panel's open/closed state alone.
export function clearUsbDiagnostics(el) {
  resetDiagnosticsContent(el);
}

// Full hide: the diagnostics report no longer applies to anything on screen
// (USB root cleared or switched to a different drive), so collapse the panel
// too, not just its content.
export function hideUsbDiagnostics(el) {
  resetDiagnosticsContent(el);
  if (el.usbDiagnosticsCard) {
    el.usbDiagnosticsCard.classList.add("hidden");
  }
  const healthCard = el.usbDiagnosticsCard?.closest?.("#usbHealthCard");
  if (healthCard) {
    healthCard.removeAttribute("open");
    healthCard.classList.remove("is-loading");
  }
}

export function renderRepairPreview(el, data, deps = {}) {
  const {
    documentObj = document,
    showDiagRepairView = () => showDiagRepairView(el),
    getSelectedFixIds = () => new Set(),
    setSelectedFixIds = () => {},
    onToggleFixSelection = () => {}
  } = deps;

  if (!el.diagRepairPanel) return;
  el.usbDiagnosticsCard.classList.remove("hidden");

  const issueCount = Array.isArray(data.detectedIssues) ? data.detectedIssues.length : 0;
  const fixes = data.proposedFixes || [];
  const unsupportedItems = data.unsupportedItems || [];
  const fixCount = fixes.length;
  const supportedFixes = fixes.filter((f) => f.supported);
  const supportedFixIds = supportedFixes
    .map((f) => String(f?.id || ""))
    .filter(Boolean);
  setSelectedFixIds(new Set(supportedFixIds));
  const selectedFixIds = getSelectedFixIds();
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
      el.diagRepairSummary.textContent = parts.join(" \u00b7 ");
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

    el.diagRepairFixes.innerHTML = "";
    for (const fix of fixesToRender) {
      const li = documentObj.createElement("li");
      li.className = fix.supported ? "diag-repair-fix-supported" : "diag-repair-fix-unsupported";
      if (fix.supported) {
        li.classList.add("diag-repair-fix-with-select");
      }

      const content = documentObj.createElement("div");
      content.className = "diag-repair-fix-content";

      if (fix.supported) {
        const fixId = String(fix.id || "");
        const alwaysApplied = fix.alwaysApplied === true;
        const checkbox = documentObj.createElement("input");
        checkbox.type = "checkbox";
        checkbox.className = "diag-repair-fix-check";
        checkbox.checked = alwaysApplied || selectedFixIds.has(fixId);
        checkbox.dataset.fixId = fixId;
        if (alwaysApplied) {
          checkbox.disabled = true;
          checkbox.title = "Always applied — required for other repairs to complete safely";
        } else {
          checkbox.addEventListener("change", (event) => {
            onToggleFixSelection(fixId, !!event?.target?.checked);
          });
        }
        li.appendChild(checkbox);

        const titleWrap = documentObj.createElement("div");
        titleWrap.className = "diag-repair-fix-title";
        const title = documentObj.createElement("strong");
        title.textContent = fix.title;
        titleWrap.appendChild(title);
        content.appendChild(titleWrap);
      } else {
        const titleWrap = documentObj.createElement("div");
        titleWrap.className = "diag-repair-fix-title";
        const title = documentObj.createElement("strong");
        title.textContent = fix.title;
        titleWrap.appendChild(title);
        content.appendChild(titleWrap);
      }

      const desc = documentObj.createElement("span");
      desc.className = "diag-repair-fix-desc";
      desc.textContent = fix.description;
      content.appendChild(desc);

      const meta = documentObj.createElement("span");
      meta.className = "diag-repair-fix-meta";
      const support = fix.supported ? "\u2713 supported" : "\u2717 preview-only";
      const mode = fix.destructive ? "destructive" : "safe";
      const metaParts = [support, mode];
      if (fix.estimatedWrites) metaParts.push(`${fix.estimatedWrites} writes`);
      if (fix.estimatedDeletes) metaParts.push(`${fix.estimatedDeletes} deletes`);
      if (fix.alwaysApplied === true) metaParts.push("always applied");
      meta.textContent = metaParts.join(" \u00b7 ");
      content.appendChild(meta);

      li.appendChild(content);

      el.diagRepairFixes.appendChild(li);
    }
  }

  showDiagRepairView();
  const selectedCount = getSelectedFixIds().size;
  el.applyRepairsBtn.disabled = selectedCount === 0;
  if (supportedFixes.length === 0 && fixCount === 0) {
    el.previewRepairsBtn.disabled = true;
  }
}
export function loadUsbRootFromStorage(state, el, deps = {}) {
  const {
    localStorageObj = typeof localStorage !== "undefined" ? localStorage : null,
    storageKeyUsbRoot = "usbRoot",
    updateUsbRootText = () => {},
    updateUsbConfigControlsVisibility = () => {},
    updatePlaylistExportButtons = () => {}
  } = deps;

  try {
    const raw = localStorageObj?.getItem?.(storageKeyUsbRoot);
    state.usbRoot = raw ? String(raw).trim() || null : null;
  } catch {
    state.usbRoot = null;
  }
  state.usbRootValid = false;
  state.usbNeedsInit = false;
  updateUsbRootText(state.usbRoot, false);
  if (el.usbInitRow) {
    el.usbInitRow.classList.add("hidden");
  }
  updateUsbConfigControlsVisibility();
  updatePlaylistExportButtons();
}

export function resetUsbStateViews(state, el, deps = {}) {
  const {
    renderUsbPlaylists = () => {},
    clearUsbPlaylistTracks = () => {},
    renderHistoryList = () => {},
    clearHistoryTracks = () => {},
    renderUsbPlayerMenuEditor = () => {},
    hideDiagnostics = true
  } = deps;

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
  if (hideDiagnostics) hideUsbDiagnostics(el);

  renderUsbPlaylists();
  clearUsbPlaylistTracks();
  renderHistoryList();
  clearHistoryTracks();
  renderUsbPlayerMenuEditor();
}

export async function syncAssetScopePaths(state, deps = {}) {
  const {
    invoke = async () => {},
    warn = () => {}
  } = deps;

  const paths = [];
  for (const root of state.sourceRoots || []) {
    const value = String(root || "").trim();
    if (value) paths.push(value);
  }
  const usbRoot = String(state.usbRoot || "").trim();
  if (usbRoot) paths.push(usbRoot);
  if (!paths.length) return;

  try {
    await invoke("allow_asset_paths", { paths });
  } catch (err) {
    warn("allow_asset_paths failed:", err);
  }
}

export async function pickSourceFolders(deps = {}) {
  const { invoke = async () => null } = deps;
  const selected = await invoke("pick_source_folders");
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
export function updateUsbConfigControlsVisibility(state, el) {
  const hasValidRoot = !!state.usbRoot && !!state.usbRootValid;
  if (el.usbSelectedControls) {
    el.usbSelectedControls.classList.toggle("hidden", !hasValidRoot);
  }
  if (!hasValidRoot && el.usbDiagnosticsCard) {
    el.usbDiagnosticsCard.classList.add("hidden");
  }
}

export async function detectExternalMasterDb(state, el, deps) {
  const { command, warn, renderSourceChips } = deps;
  try {
    const data = await command("detect_external_master_db");
    const found = !!data?.found && !!data?.path;
    state.externalMasterDbPath = found ? data.path : null;
    if (!found) state.masterDbEnabled = false;
  } catch (err) {
    state.externalMasterDbPath = null;
    state.masterDbEnabled = false;
    warn("External master DB detection failed:", err);
  }
  // Hide the legacy toggle element; the chip in renderSourceChips is the control
  el.externalMasterDbToggle?.classList.add("hidden");
  renderSourceChips?.();
}

// Prompts for (and saves) a name for `state.usbRoot` if it doesn't have one
// yet. A name is this app's only notion of stable drive identity (see
// `usb_identity` on the backend) -- it's what lets backups and local
// staging caching correctly recognize "this is the same drive" again after
// a replug or a different computer, where the OS-assigned mount path can't.
// Best-effort: any failure to even check/show the prompt just lets the user
// continue unnamed rather than blocking USB use entirely.
async function promptDriveNameIfUnset(state, el, deps = {}) {
  const { command, documentObj, updateUsbNameBadge = () => {} } = deps;
  const emitStatus = resolveEmitStatus(deps);
  // Reset first: a stale name from whatever drive was connected before must
  // never linger on screen while this one's actual name is still unknown.
  state.usbDeviceName = null;
  updateUsbNameBadge();
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
    updateUsbNameBadge();
    return;
  }

  const doc = documentObj ?? (typeof document !== "undefined" ? document : null);
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
        updateUsbNameBadge();
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

export async function validateAndSetUsbRoot(state, el, path, silent = false, deps) {
  const {
    command,
    persistUsbRoot,
    updateUsbRootText,
    resetUsbStateViews,
    updateUsbConfigControlsVisibility,
    updateUsbSubNavDisabledState,
    updatePlaylistExportButtons,
    setStatus,
    runUsbDiagnostics,
    warn,
    scheduler
  } = deps;
  const emitStatus = resolveEmitStatus(deps);

  if (isUsbRootChangeBlocked(state)) {
    if (!silent) emitStatus("Please wait for the current USB operation to finish before switching drives");
    return false;
  }

  const input = String(path || "").trim();
  const previousRoot = state.usbRoot;
  if (input && previousRoot && input !== previousRoot) {
    hideUsbDiagnostics(el);
  }
  if (!input) {
    state.usbRoot = null;
    state.usbRootValid = false;
    state.usbNeedsInit = false;
    state.usbDeviceName = null;
    deps.updateUsbNameBadge?.();
    persistUsbRoot(null);
    updateUsbRootText(null, false);
    el.usbInitRow.classList.add("hidden");
    resetUsbStateViews();
    updateUsbConfigControlsVisibility();
    updateUsbSubNavDisabledState();
    updatePlaylistExportButtons();
    if (!silent) emitStatus("USB root cleared");
    return false;
  }

  const result = await command("validate_usb_root", { path: input });
  const normalized = String(result?.normalizedRoot || "").trim();
  const valid = !!result?.valid && !!normalized;
  const hasStructureWarning = !result?.hasVendorRoot || !result?.hasContents || !result?.hasPdb;
  const canInitialize = !!normalized && !valid && !!result?.hasWriteAccess && hasStructureWarning;
  state.usbWritable = !!result?.hasWriteAccess;
  state.usbRootValid = valid;
  state.usbNeedsInit = canInitialize;
  state.usbRoot = normalized || input;
  persistUsbRoot(state.usbRoot);
  updateUsbRootText(state.usbRoot, valid);
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
    resetUsbStateViews();
  }
  updateUsbConfigControlsVisibility();
  updateUsbSubNavDisabledState();
  updatePlaylistExportButtons();
  if (valid) {
    await promptDriveNameIfUnset(state, el, deps);
  } else {
    // Invalid/uninitialized root: clear any name badge left over from
    // whatever drive was previously connected -- it no longer applies.
    state.usbDeviceName = null;
    deps.updateUsbNameBadge?.();
  }
  if (!silent) {
    if (valid) {
      const selectedWarningText = joinWarningTexts(result?.warnings);
      const reason = selectedWarningText ? ` (${selectedWarningText})` : "";
      emitStatus(`USB root selected: ${state.usbRoot}${reason}. Running diagnostics...`);
      const _docObj = deps.documentObj ?? (typeof document !== "undefined" ? document : null);
      const _healthCard = _docObj?.getElementById?.("usbHealthCard") ?? null;
      if (_healthCard) {
        _healthCard.removeAttribute("open");
        _healthCard.classList.add("is-loading");
      }
      scheduler(() => {
        runUsbDiagnostics().catch((err) => {
          warn("Auto-diagnostics failed:", err);
          emitStatus(`Auto-diagnostics failed: ${err?.message || err}`);
        });
      }, 50);
    } else if (canInitialize) {
      emitStatus('USB selected but not initialized. Click "Initialize USB Structure" to continue.');
    } else {
      const invalidWarningText = joinWarningTexts(result?.warnings) || "invalid USB root";
      emitStatus(`USB root invalid: ${invalidWarningText}`);
    }
  }
  return valid;
}

export async function removeUsbPlaylist(state, playlist, deps) {
  const {
    setStatus,
    openConfirmDialog,
    command,
    refreshUsb,
    countWarningsForStatus,
    clearUsbDiagnostics = () => {}
  } = deps;
  const emitStatus = resolveEmitStatus(deps);

  if (!state.usbRoot) {
    emitStatus("Select USB folder first");
    return;
  }
  if (!playlist) {
    emitStatus("USB playlist not found");
    return;
  }

  const confirmed = await openConfirmDialog({
    title: "Remove USB Playlist",
    message: `Remove USB playlist "${playlist.name}" from the stick?`,
    confirmLabel: "Remove"
  });
  if (!confirmed) return;

  const data = await command("remove_usb_playlist", {
    usbRoot: state.usbRoot,
    playlistId: playlist.id,
    playlistName: playlist.name
  });
  clearUsbDiagnostics();
  await refreshUsb();
  const warningCount = typeof countWarningsForStatus === "function"
    ? countWarningsForStatus(data.warnings)
    : ((data.warnings || []).length || 0);
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

export async function reorderUsbPlaylists(state, el, deps) {
  const { command, refreshUsb, clearUsbDiagnostics = () => {} } = deps;
  const emitStatus = resolveEmitStatus(deps);

  if (!state.usbRoot || !state.usbRootValid) {
    emitStatus("Select USB folder first");
    return;
  }

  try {
    await command("reorder_usb_playlists", {
      usbRoot: state.usbRoot,
      orderedPlaylistIds: state.usbPlaylists.map((p) => p.id)
    });
    clearUsbDiagnostics();
    emitStatus("Playlist order saved");
  } catch (err) {
    emitStatus(`Failed to save playlist order: ${err.message || err}`);
  } finally {
    await refreshUsb();
  }
}
// USB workflow orchestration extracted from main.js.

export async function refreshUsb(state, el, deps) {
  const {
    setStatus,
    command,
    setProgress,
    startProgressHeartbeat,
    stopProgressHeartbeat,
    normalizeUsbPlaylist,
    renderUsbPlaylists,
    clearUsbPlaylistTracks = () => {},
    renderCurrentPlaylistTracksFromState,
    updatePlaylistExportButtons,
    countWarningsForStatus,
    logWarnings
  } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (!state.usbRoot) {
    emitStatus("Select USB folder first");
    return;
  }
  emitStatus("Loading USB playlists...");
  setProgress(true, 5, "Reading USB database...");
  startProgressHeartbeat();
  let data;
  try {
    data = await command("fetch_usb_playlists", {
      usbRoot: state.usbRoot
    });
  } catch (err) {
    stopProgressHeartbeat();
    setProgress(true, 100, "USB load failed", { error: true, dismissable: true });
    throw err;
  }
  stopProgressHeartbeat();

  const rawItems = data.items || [];
  const total = rawItems.length;
  setProgress(true, 40, `Loaded ${total} playlists, normalizing...`);
  await new Promise((r) => setTimeout(r, 30));

  state.usbPlaylists = [];
  for (let i = 0; i < total; i += 1) {
    state.usbPlaylists.push(normalizeUsbPlaylist(rawItems[i]));
    if ((i + 1) % 3 === 0 || i === total - 1) {
      const pct = 40 + Math.round(((i + 1) / total) * 35);
      setProgress(true, pct, `Processing playlist ${i + 1}/${total}: ${rawItems[i].name || "..."}`);
      await new Promise((r) => setTimeout(r, 0));
    }
  }
  state.playlistUsbExportStatusById = playlistUsbExportStatusById(data.playlistUsbExportStatus);

  setProgress(true, 80, "Computing stats...");
  await new Promise((r) => setTimeout(r, 20));

  const usbTrackTotal = Number(data.playlistTrackTotal) || 0;
  el.usbCountsText.textContent = `${state.usbPlaylists.length} playlists, ${usbTrackTotal} tracks`;
  setProgress(true, 90, "Rendering playlists...");
  await new Promise((r) => setTimeout(r, 20));
  renderUsbPlaylists();
  clearUsbPlaylistTracks();
  updatePlaylistExportButtons();
  // The freshly scanned status may flip an open local playlist's reorder lock.
  await renderCurrentPlaylistTracksFromState?.();

  const warningCount = countWarningsForStatus(data.warnings);
  const warningSuffix = warningCount ? ` | (${warningCount} warning(s))` : "";
  logWarnings("usb-import", data.warnings, "fetch_usb_playlists");
  setProgress(true, 100, `Done — ${state.usbPlaylists.length} playlists, ${usbTrackTotal} tracks`);
  emitStatus(`USB playlists loaded: ${state.usbPlaylists.length}${warningSuffix}`, { warningCount });
  setTimeout(() => setProgress(false, 0, "Idle"), 1200);
}

export async function runUsbDiagnostics(state, deps) {
  const {
    setStatus,
    command,
    updatePlaylistExportButtons,
    renderCurrentPlaylistTracksFromState,
    renderDiagnosticsReport,
    logWarnings
  } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (!state.usbRoot) {
    emitStatus("Select USB folder first");
    return;
  }
  const _diagDocObj = deps.documentObj ?? (typeof document !== "undefined" ? document : null);
  const _diagHealthCard = _diagDocObj?.getElementById?.("usbHealthCard") ?? null;
  if (_diagHealthCard) {
    _diagHealthCard.removeAttribute("open");
    _diagHealthCard.classList.add("is-loading");
  }
  emitStatus("Running USB diagnostics...");
  const data = await command("run_usb_diagnostics", {
    usbRoot: state.usbRoot
  });
  state.playlistUsbExportStatusById = playlistUsbExportStatusById(data?.playlistUsbExportStatus);
  updatePlaylistExportButtons();
  await renderCurrentPlaylistTracksFromState?.();
  renderDiagnosticsReport(data);
  logWarnings("usb-diagnostics", data.warnings, "run_usb_diagnostics");
  emitStatus(`Diagnostics complete (${data.durationMs}ms)`);
}

export async function runUsbParityReport(state, deps) {
  const {
    setStatus,
    command,
    renderParityReport,
    logWarnings
  } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (!state.usbRoot) {
    emitStatus("Select USB folder first");
    return;
  }
  emitStatus("Running USB parity report...");
  const data = await command("run_usb_parity_report", {
    usbRoot: state.usbRoot
  });
  renderParityReport(data);
  logWarnings("usb-diagnostics", data.warnings, "run_usb_parity_report");
  emitStatus(`Parity report complete (${data.durationMs}ms)`);
}

export async function previewUsbRepairs(state, deps) {
  const {
    setStatus,
    command,
    renderRepairPreview,
    logWarnings
  } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (!state.usbRoot) {
    emitStatus("Select USB folder first");
    return;
  }
  emitStatus("Previewing USB repair fixes...");
  const data = await command("repair_usb_diagnostics", {
    usbRoot: state.usbRoot
  });
  renderRepairPreview(data);
  logWarnings("usb-diagnostics", data.warnings, "repair_usb_diagnostics preview");
  emitStatus(`Repair preview ready (${data.durationMs}ms)`);
}

export async function applyUsbRepairs(state, deps) {
  const {
    setStatus,
    command,
    logWarnings,
    resetUsbStateViews = () => {},
    updatePlaylistExportButtons = () => {},
    renderCurrentPlaylistTracksFromState = () => {},
    renderDiagnosticsReport = () => {}
  } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (!state.usbRoot) {
    emitStatus("Select USB folder first");
    return;
  }
  emitStatus("Applying supported USB repair fixes...");
  let data;
  const selectedFixIds = Array.from(state.selectedRepairFixIds);
  if (selectedFixIds.length === 0) {
    emitStatus("Select at least one fix to apply.");
    return;
  }
  data = await command("repair_usb_diagnostics", {
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
  if (applied > 0) resetUsbStateViews({ hideDiagnostics: false });
  logWarnings("usb-diagnostics", data.warnings, "repair_usb_diagnostics apply");
  if (data.diagnostics) {
    state.playlistUsbExportStatusById = playlistUsbExportStatusById(
      data.diagnostics.playlistUsbExportStatus
    );
    updatePlaylistExportButtons();
    await renderCurrentPlaylistTracksFromState();
    renderDiagnosticsReport(data.diagnostics);
    logWarnings("usb-diagnostics", data.diagnostics.warnings, "run_usb_diagnostics");
  }
  emitStatus(`Repair apply complete: ${applied} applied, ${failed} failed (${data.durationMs}ms)${data.diagnostics ? ". Diagnostics refreshed." : ""}`);
}

export async function refreshHistory(state, el, deps) {
  const { setStatus, command, normalizeTrack, countWarningsForStatus, logWarnings, renderHistoryList, clearHistoryTracks = () => {} } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (!state.usbRoot) {
    emitStatus("Select USB folder first");
    return;
  }
  emitStatus("Loading USB history...");
  const data = await command("fetch_usb_histories", { usbRoot: state.usbRoot });

  state.histories = (data.items || []).map((history) => ({
    ...history,
    tracks: (history.tracks || []).map((track) => normalizeTrack(track, "hist"))
  }));
  // Backend-owned: `fetch_usb_histories` always returns `counts` computed over
  // the full import -- the frontend renders them, never re-tallies.
  const counts = data.counts || {};
  el.historyCountsText.textContent = `${counts.importedPlaylists || 0} sessions, ${counts.importedTracks || 0} tracks`;
  state.selectedHistoryIndex = null;
  state.historyTracks = [];
  if (el.exportHistoryTracklistBtn) el.exportHistoryTracklistBtn.disabled = true;
  renderHistoryList();
  clearHistoryTracks();
  const warningCount = countWarningsForStatus(data.warnings);
  const warningSuffix = warningCount ? ` | (${warningCount} warning(s))` : "";
  logWarnings("usb-import", data.warnings, "fetch_usb_histories");
  emitStatus(`USB histories loaded: ${state.histories.length}${warningSuffix}`, { warningCount });
}

export function sanitizeTracklistFileName(name) {
  const cleaned = String(name || "")
    .replace(/[\\/:*?"<>|]+/g, "-")
    .replace(/\s+/g, " ")
    .trim();
  return `${cleaned || "tracklist"}.txt`;
}

export async function exportHistoryTracklist(state, el, deps = {}) {
  const { invoke = async () => false, tracklistExportDialog, buildTracklistText = () => "" } = deps;
  const emitStatus = resolveEmitStatus(deps);

  const history = state.histories[state.selectedHistoryIndex];
  if (!history || !state.historyTracks.length) {
    emitStatus("Select a history session first");
    return;
  }

  const choice = await tracklistExportDialog.open({
    tracks: state.historyTracks,
    defaultTimesEnabled: true,
    defaultPlacement: "before"
  });
  if (!choice) return;

  const startIndex = Math.min(Math.max(Number(choice.startIndex) || 0, 0), state.historyTracks.length - 1);
  const text = buildTracklistText(state.historyTracks.slice(startIndex), choice.timeMode);
  const saved = await invoke("save_text_file", {
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
  const button = documentObj.createElement("button");
  button.type = "button";
  button.className = "player-menu-item";
  button.dataset.menuKind = String(kind);
  button.dataset.menuSide = side;
  button.dataset.menuOrigin = origin;
  button.setAttribute("role", "option");

  const label = documentObj.createElement("span");
  label.className = "player-menu-item-label";
  label.textContent = toMenuOptionLabel(item);
  button.appendChild(label);

  if (side === "current" && origin !== "both") {
    const tag = documentObj.createElement("span");
    tag.className = `player-menu-item-origin is-${origin}`;
    tag.textContent = origin === "pdb_only" ? "PDB" : "eDB";
    tag.dataset.tooltip = origin === "pdb_only"
      ? "Only in PDB t16 (eDB missing this kind)"
      : "Only in eDB menuItem (not in PDB t16)";
    button.appendChild(tag);
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

export function selectUsbPlayerMenuItem(state, el, side, kind, deps = {}) {
  const normalized = normalizeMenuKind(kind);
  if (side === "available") {
    state.usbPlayerMenuAvailableSelectedKind = normalized;
    state.usbPlayerMenuCurrentSelectedKind = null;
  } else {
    state.usbPlayerMenuCurrentSelectedKind = normalized;
    state.usbPlayerMenuAvailableSelectedKind = null;
  }
  renderUsbPlayerMenuEditor(state, el, deps);
}

export function handleUsbPlayerMenuListClick(state, el, deps, side, event) {
  const target = event?.target?.closest?.(".player-menu-item");
  if (!target) return;
  const kind = normalizeMenuKind(target.dataset.menuKind);
  if (kind === null) return;
  selectUsbPlayerMenuItem(state, el, side, kind, deps);
}

export function renderUsbPlayerMenuEditor(state, el, deps = {}) {
  const { documentObj = document } = deps;
  const availableEl = el.usbPlayerMenuAvailable;
  const currentEl = el.usbPlayerMenuCurrent;
  if (!availableEl || !currentEl) return;

  ensureValidPlayerMenuSelections(state);

  availableEl.innerHTML = "";
  for (const item of state.usbPlayerMenuAvailable || []) {
    const row = buildPlayerMenuItemButton(
      documentObj,
      item,
      state.usbPlayerMenuAvailableSelectedKind,
      "available",
    );
    availableEl.appendChild(row);
  }

  currentEl.innerHTML = "";
  for (const item of state.usbPlayerMenuCurrent || []) {
    const row = buildPlayerMenuItemButton(
      documentObj,
      item,
      state.usbPlayerMenuCurrentSelectedKind,
      "current",
    );
    currentEl.appendChild(row);
  }

  renderUsbPlayerMenuDivergence(state, el);
  syncUsbPlayerMenuEditorControls(state, el);
}

function renderUsbPlayerMenuDivergence(state, el) {
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

export async function syncUsbPlayerMenusEdbToPdb(state, el, deps) {
  const { command, clearUsbDiagnostics = () => {} } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (!state.usbRoot || !state.usbRootValid) {
    emitStatus("Select USB folder first");
    return;
  }
  emitStatus("Fixing PDB sync...");
  const data = await command("sync_usb_player_menu_edb_to_pdb", { usbRoot: state.usbRoot });
  state.usbPlayerMenuCurrent = Array.isArray(data?.currentItems) ? data.currentItems : [];
  state.usbPlayerMenuAvailable = Array.isArray(data?.availableItems) ? data.availableItems : [];
  state.usbPlayerMenuDivergence = normalizeDivergence(data?.divergence);
  state.usbPlayerMenuCurrentSelectedKind = null;
  state.usbPlayerMenuAvailableSelectedKind = null;
  renderUsbPlayerMenuEditor(state, el, deps);
  if (data?.updated) clearUsbDiagnostics();
  emitStatus(data?.updated ? "PDB categories restored" : "PDB already complete");
}

// Whether the currently-selected `current` menu item may be removed is a
// backend-owned fact (`UsbPlayerMenuItem.removable`, see
// backend/src/service/repair.rs). The frontend just reads it.
function currentPlayerMenuItemByKind(state, kind) {
  return (state.usbPlayerMenuCurrent || []).find((item) => Number(item.kind) === kind) || null;
}

export function syncUsbPlayerMenuEditorControls(state, el) {
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

function normalizeDivergence(raw) {
  return {
    inEdbVisibleOnly: Array.isArray(raw?.inEdbVisibleOnly) ? raw.inEdbVisibleOnly : [],
    inPdbOnly: Array.isArray(raw?.inPdbOnly) ? raw.inPdbOnly : [],
    orderMismatch: !!raw?.orderMismatch,
    pdbMissingKinds: Array.isArray(raw?.pdbMissingKinds) ? raw.pdbMissingKinds : [],
    summary: String(raw?.summary || ""),
    canSync: !!raw?.canSync,
    canRestore: !!raw?.canRestore,
  };
}

export async function loadUsbPlayerMenuConfig(state, el, deps) {
  const { command } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (!state.usbRoot || !state.usbRootValid) {
    emitStatus("Select USB folder first");
    renderUsbPlayerMenuEditor(state, el, deps);
    return;
  }
  emitStatus("Loading player menu configuration...");
  const data = await command("get_usb_player_menu_config", { usbRoot: state.usbRoot });
  state.usbPlayerMenuCurrent = Array.isArray(data?.currentItems) ? data.currentItems : [];
  state.usbPlayerMenuAvailable = Array.isArray(data?.availableItems) ? data.availableItems : [];
  state.usbPlayerMenuDivergence = normalizeDivergence(data?.divergence);
  state.usbPlayerMenuCurrentSelectedKind = null;
  state.usbPlayerMenuAvailableSelectedKind = null;
  renderUsbPlayerMenuEditor(state, el, deps);
  emitStatus("Player menu loaded");
}

export async function updateUsbPlayerMenuConfig(state, el, deps, currentKinds, preferredSelection = null) {
  const { command, clearUsbDiagnostics = () => {} } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (!state.usbRoot || !state.usbRootValid) {
    emitStatus("Select USB folder first");
    return;
  }
  const data = await command("update_usb_player_menu_config", {
    usbRoot: state.usbRoot,
    currentKinds,
  });
  if (data?.updated) clearUsbDiagnostics();
  state.usbPlayerMenuCurrent = Array.isArray(data?.currentItems) ? data.currentItems : [];
  state.usbPlayerMenuAvailable = Array.isArray(data?.availableItems) ? data.availableItems : [];
  state.usbPlayerMenuDivergence = normalizeDivergence(data?.divergence);
  if (preferredSelection?.side === "current") {
    state.usbPlayerMenuCurrentSelectedKind = normalizeMenuKind(preferredSelection.kind);
    state.usbPlayerMenuAvailableSelectedKind = null;
  } else if (preferredSelection?.side === "available") {
    state.usbPlayerMenuAvailableSelectedKind = normalizeMenuKind(preferredSelection.kind);
    state.usbPlayerMenuCurrentSelectedKind = null;
  } else {
    state.usbPlayerMenuCurrentSelectedKind = null;
    state.usbPlayerMenuAvailableSelectedKind = null;
  }
  renderUsbPlayerMenuEditor(state, el, deps);
  emitStatus(data?.updated ? "Player menu updated" : "Player menu unchanged");
}

export async function addUsbPlayerMenuItems(state, el, deps) {
  const selected = normalizeMenuKind(state.usbPlayerMenuAvailableSelectedKind);
  if (selected === null) return;
  const currentKinds = (state.usbPlayerMenuCurrent || []).map((item) => Number(item.kind));
  if (!currentKinds.includes(selected)) {
    currentKinds.push(selected);
  }
  await updateUsbPlayerMenuConfig(state, el, deps, currentKinds, {
    side: "current",
    kind: selected,
  });
}

export async function removeUsbPlayerMenuItems(state, el, deps) {
  const selected = normalizeMenuKind(state.usbPlayerMenuCurrentSelectedKind);
  if (selected === null) return;
  // Belt-and-suspenders: the Remove button is already disabled for these, and
  // update_usb_player_menu_config rejects the request backend-side.
  if (currentPlayerMenuItemByKind(state, selected)?.removable === false) return;
  const currentKinds = (state.usbPlayerMenuCurrent || [])
    .map((item) => Number(item.kind))
    .filter((kind) => kind !== selected);
  await updateUsbPlayerMenuConfig(state, el, deps, currentKinds, {
    side: "available",
    kind: selected,
  });
}

export async function moveUsbPlayerMenuItems(state, el, deps, direction) {
  const selected = normalizeMenuKind(state.usbPlayerMenuCurrentSelectedKind);
  if (selected === null) return;
  const currentKinds = (state.usbPlayerMenuCurrent || []).map((item) => Number(item.kind));
  const selectedIdx = currentKinds.indexOf(selected);
  if (selectedIdx < 0) return;

  if (direction < 0) {
    if (selectedIdx > 0) {
      const tmp = currentKinds[selectedIdx - 1];
      currentKinds[selectedIdx - 1] = currentKinds[selectedIdx];
      currentKinds[selectedIdx] = tmp;
    }
  } else {
    if (selectedIdx >= 0 && selectedIdx < currentKinds.length - 1) {
      const tmp = currentKinds[selectedIdx + 1];
      currentKinds[selectedIdx + 1] = currentKinds[selectedIdx];
      currentKinds[selectedIdx] = tmp;
    }
  }
  await updateUsbPlayerMenuConfig(state, el, deps, currentKinds, {
    side: "current",
    kind: selected,
  });
}

export async function exportPlaylistToUsb(state, el, playlistId, deps) {
  const {
    setStatus,
    setProgress,
    startProgressHeartbeat,
    nextPaint,
    command,
    stopProgressHeartbeat,
    countWarningsForStatus,
    warningEntryLevel,
    logWarnings,
    emitMessage,
    pushEventLog,
    loadPlaylists,
    updateModeText,
    switchView,
    renderUsbPlaylists,
    clearUsbPlaylistTracks = () => {},
    refreshMissingSourceRoots = async () => [],
    clearUsbDiagnostics = () => {},
    commitActivePlaylistSort = async () => {}
  } = deps;
  const emitStatus = resolveEmitStatus(deps);
  const emitErrorEvent = (text, details = null, coalesceKey = "export.failure") => {
    if (typeof emitMessage === "function") {
      emitMessage({
        level: "error",
        source: "export",
        code: "export.failure",
        eventLog: { text, details, coalesceKey }
      });
      return;
    }
    if (typeof pushEventLog === "function") {
      pushEventLog({
        level: "error",
        source: "export",
        code: "export.failure",
        message: text,
        details,
        coalesceKey
      });
    }
  };
  const playlist = state.playlists.find((item) => item.id === playlistId);
  if (!playlist) return;
  try {
    await commitActivePlaylistSort(playlistId);
  } catch (err) {
    emitStatus(`Export blocked: couldn't save the current sort order (${err.message || err})`);
    return;
  }
  if (!playlist.tracks?.length) {
    emitStatus("Playlist must contain tracks before export");
    return;
  }
  await refreshMissingSourceRoots({ silent: true });
  const affectedMissingTracks = playlistTracksAffectedByMissingRoots(playlist.tracks, state);
  if (affectedMissingTracks.length) {
    const missingRoots = missingSourceRootsArray(state);
    const suffix = missingRoots.length ? `: ${missingRoots[0]}` : "";
    emitStatus(`Export blocked: source folder is missing${suffix}. Relocate or remove it first.`);
    return;
  }
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
  startProgressHeartbeat();
  await nextPaint();
  let data;
  try {
    data = await command("export_to_usb", {
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
    } else {
      const msg = String(error?.message || "USB export failed").trim() || "USB export failed";
      emitStatus(`Export failed: ${msg}. See Event Log for details.`);
      emitErrorEvent(msg, "context: export_to_usb", "export.failure.export_to_usb");
    }
    throw error;
  } finally {
    if (!state.activeJobId) {
      setProgress(false, 0, "Idle");
      stopProgressHeartbeat();
    }
  }
  clearUsbDiagnostics();
  const warningCount = countWarningsForStatus(data.warnings);
  const warningSuffix = warningCount ? ` | (${warningCount} warning(s))` : "";
  const warningList = Array.isArray(data.warnings) ? data.warnings : [];
  if (warningList.length) {
    const infoCount = warningList.filter((entry) => warningEntryLevel(entry) === "info").length;
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
  logWarnings("export", data.warnings, "export_to_usb");
  emitStatus(
    `Export complete: ${playlist.name} - ${data.exportedTracks || 0} track(s), ${data.skippedTracks || 0} skipped${warningSuffix}${state.exportPruneStale ? " [sync: mirror]" : " [sync: additive]"}`,
    { warningCount }
  );
  await loadPlaylists();
  state.currentPlaylistId = playlistId;
  updateModeText();
  await switchView(playlistId);

  state.usbPlaylists = [];
  renderUsbPlaylists();
  clearUsbPlaylistTracks();
}

export function renderUsbPlaylists(state, el, deps = {}) {
  const { escapeHtml = (v) => String(v || "") } = deps;
  el.usbPlaylists.innerHTML = "";
  const usbRight = el.usbPlaylists.closest(".split")?.querySelector(".right");
  if (!state.usbPlaylists.length) {
    el.usbPlaylists.innerHTML = '<li class="muted">No playlists imported yet. Click "Import Playlists" to load from USB.</li>';
    usbRight?.classList.add("hidden");
    return;
  }
  usbRight?.classList.remove("hidden");
  state.usbPlaylists.forEach((playlist, index) => {
    const count = Number(playlist.trackCount ?? playlist.tracks?.length ?? 0);
    el.usbPlaylists.insertAdjacentHTML(
      "beforeend",
      `<li data-usb-playlist-li="${index}"><button data-usb-playlist-index="${index}" data-usb-playlist="${escapeHtml(playlist.id)}"><span class="drag-handle" data-usb-drag-handle draggable="true" data-tooltip="Drag to reorder" aria-label="Drag to reorder">&#10495;</span><span class="playlist-label">${escapeHtml(playlist.name)} (${count})</span><span class="playlist-remove" data-usb-remove-playlist="${escapeHtml(playlist.id)}" data-tooltip="Remove" aria-label="Remove">&times;</span></button></li>`
    );
  });
}

export function usbPlaylistRowOptions() {
  return {
    withCheckbox: false,
    actionLabel: "+",
    actionType: "add-usb",
    compactAddButton: true,
    enableAnalyzeActions: false,
    origin: "usb",
    secondaryActionLabel: "Play",
    secondaryActionType: "play-usb"
  };
}

export function renderHistoryList(state, el, deps = {}) {
  const {
    escapeHtml = (v) => String(v || ""),
    getHistoryDateValue = () => ""
  } = deps;
  el.historyList.innerHTML = "";
  const histRight = el.historyList.closest(".split")?.querySelector(".right");
  if (!state.histories.length) {
    el.historyList.innerHTML = '<li class="muted">No history imported yet. Click "Import History" to load from USB.</li>';
    histRight?.classList.add("hidden");
    return;
  }
  histRight?.classList.remove("hidden");

  // Render newest first — keep original index so click handler resolves state.histories[index]
  state.histories.map((history, index) => ({ history, index })).reverse().forEach(({ history, index }) => {
    const dateText = getHistoryDateValue(history);
    el.historyList.insertAdjacentHTML(
      "beforeend",
      `<li><button data-history-index="${index}"><span class="playlist-label">${escapeHtml(history.name)}${dateText ? ` (${escapeHtml(dateText)})` : ""}</span></button></li>`
    );
  });
}

export function usbHistoryRowOptions() {
  return {
    withCheckbox: false,
    actionLabel: "+",
    actionType: "add-history",
    compactAddButton: true,
    enableAnalyzeActions: false,
    origin: "usb",
    secondaryActionLabel: "Play",
    secondaryActionType: "play-history"
  };
}

export async function initializeUsb(state, el, deps = {}) {
  const {
    command = async () => {},
    setStatus = () => {},
    validateAndSetUsbRoot = async () => {},
    logError = () => {}
  } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (!state.usbRoot) return;
  try {
    await command("initialize_usb", { usbRoot: state.usbRoot });
    emitStatus("USB initialized");
    el.usbInitRow?.classList?.add("hidden");
    await validateAndSetUsbRoot(state.usbRoot, false);
  } catch (err) {
    logError("Initialize USB failed:", err);
    emitStatus(`Initialize failed: ${err.message || err}`);
  }
}

export async function pickUsbFolder(deps = {}) {
  const {
    invoke = async () => null,
    validateAndSetUsbRoot = async () => {},
    state = {},
    emitStatus
  } = deps;
  if (isUsbRootChangeBlocked(state)) {
    resolveEmitStatus({ emitStatus })("Please wait for the current USB operation to finish before switching drives");
    return null;
  }
  const selected = await invoke("pick_usb_folder");
  if (!selected) return null;
  await validateAndSetUsbRoot(String(selected), false);
  return selected;
}

export async function hydrateUsbTrackMetadata(state, track, deps = {}) {
  const {
    command = async () => ({}),
    normalizeTrack = (t) => t
  } = deps;
  // Backend-owned: `needsHydration` (service::usb::hydrate_usb_track_in_place)
  // says whether an inspect could still fill anything in.
  if (!track || track.needsHydration !== true) return track;
  const trackId = String(track.id || "").trim();
  if (!/^\d+$/.test(trackId)) return track;
  try {
    const inspected = await command("inspect_usb_track", {
      usbRoot: state.usbRoot,
      trackId,
      filePath: track.filePath || "",
      title: track.title || "",
      artist: track.artist || ""
    });
    applyHydratedTrackResult(track, inspected?.track, normalizeTrack);
  } catch (err) {
    console.warn(`inspect_usb_track failed for ${trackId}:`, err);
  }
  // We've inspected this row -- don't ask again even if fields are still blank.
  track.needsHydration = false;
  return track;
}

function applyHydratedTrackResult(track, inspectedTrack, normalizeTrack) {
  if (!inspectedTrack || typeof inspectedTrack !== "object") {
    track.artworkChecked = true;
    return;
  }
  const normalized = normalizeTrack({ ...track, ...inspectedTrack }, "usb") || {};
  if (!normalized.localTrackId && track.localTrackId) {
    normalized.localTrackId = track.localTrackId;
  }
  normalized.artworkChecked = true;
  Object.assign(track, normalized);
}

// Replaces the old localStorage/app_settings "recent USB roots" list with a
// live query against the usb_devices table -- so mount state and pruning
// are always accurate, not a client-side cache that can drift from it.
export async function loadUsbDevices(state, command) {
  try {
    const data = await command("list_usb_devices");
    const items = Array.isArray(data?.items) ? data.items : [];
    state.usbDevices = items;
    state.usbRecentRoots = items.map((item) => String(item?.rootPath || "").trim()).filter(Boolean);
  } catch (err) {
    console.warn("Failed to load USB devices:", err);
    state.usbDevices = [];
    state.usbRecentRoots = [];
  }
  return state.usbRecentRoots;
}

export async function pruneUsbDevice(state, id, deps = {}) {
  const { command, reload = () => {} } = deps;
  if (!id) return;
  try {
    await command("prune_usb_device", { id });
  } catch (err) {
    console.warn(`Failed to prune USB device ${id}:`, err);
  }
  await reload();
}

export function renderUsbRecentRoots(el, rows, document, state = {}) {
  if (!el?.usbRecentRow || !el?.usbRecentList) return;
  el.usbRecentList.innerHTML = "";
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
    const row = document.createElement("span");
    row.className = "usb-cfg-recent-item";
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "usb-cfg-recent-btn";
    btn.dataset.usbRecentPath = path;
    btn.dataset.tooltip = path;
    btn.style.direction = "rtl";
    btn.style.textAlign = "left";
    btn.textContent = path;
    btn.disabled = locked;
    row.appendChild(btn);

    const device = devicesByPath.get(path);
    if (device?.id) {
      const pruneBtn = document.createElement("button");
      pruneBtn.type = "button";
      pruneBtn.className = "usb-cfg-recent-prune-btn";
      pruneBtn.dataset.usbPruneDeviceId = device.id;
      pruneBtn.dataset.tooltip = "Forget this USB device";
      pruneBtn.setAttribute("aria-label", `Forget ${path}`);
      pruneBtn.textContent = "×";
      pruneBtn.disabled = locked;
      row.appendChild(pruneBtn);
    }
    el.usbRecentList.appendChild(row);
  });
}

export function updateUsbRootText(el, path, valid = false) {
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
