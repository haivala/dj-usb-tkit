import {
  missingSourceRootsArray,
  playlistTracksAffectedByMissingRoots
} from "../library/actions.mjs";

export function normalizePlaylistNameForCompare(value) {
  return String(value || "").trim().toLowerCase();
}

function resolveEmitStatus(deps = {}) {
  if (typeof deps.emitStatus === "function") return deps.emitStatus;
  if (typeof deps.setStatus === "function") return deps.setStatus;
  return () => {};
}

export function knownUsbPlaylistNamesFromPlaylists(playlists) {
  const names = new Set();
  for (const playlist of playlists || []) {
    const key = normalizePlaylistNameForCompare(playlist?.name);
    if (key) names.add(key);
  }
  return names;
}

export function computeExportButtonState({
  usbRoot,
  usbRootValid,
  exportPruneStale,
  currentPlaylistName,
  knownUsbPlaylistNames
}) {
  const enabled = !!usbRoot && !!usbRootValid;
  const currentName = String(currentPlaylistName || "").trim();
  const normalized = normalizePlaylistNameForCompare(currentName);
  const sameNameUsbPlaylistExists =
    !!currentName && !!normalized && knownUsbPlaylistNames instanceof Set && knownUsbPlaylistNames.has(normalized);
  const appendModeToExisting = enabled && !exportPruneStale && sameNameUsbPlaylistExists;

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
export function formatParityIssues(pd) {
  const pdbTracks = Number(pd?.pdbTracks || 0);
  const edbTracks = Number(pd?.edbTracks || 0);
  const onlyInPdb = Number(pd?.onlyInPdb || 0);
  const onlyInEdb = Number(pd?.onlyInEdb || 0);
  return [
    (onlyInPdb > 0 && edbTracks > 0) ? `+PDB ${onlyInPdb}` : "",
    (onlyInEdb > 0 && pdbTracks > 0) ? `+eDB ${onlyInEdb}` : "",
    pd?.playlistIdMatch === false ? "id mismatch" : "",
    pd?.sortOrderMatch === false ? "sort mismatch" : "",
    pd?.orderMismatch ? "order mismatch" : "",
    Number(pd?.pdbDuplicateEntries || 0) ? `dup PDB ${pd.pdbDuplicateEntries}` : "",
    Number(pd?.pdbMissingCoreMetadata || 0) ? `PDB gaps ${pd.pdbMissingCoreMetadata}` : "",
    Number(pd?.edbMissingCoreMetadata || 0) ? `eDB gaps ${pd.edbMissingCoreMetadata}` : "",
    Number(pd?.pathMismatchTracks || 0) ? `path mismatch ${pd.pathMismatchTracks}` : "",
    Number(pd?.dictionaryIdIssueTracks || 0) ? `dict issues ${pd.dictionaryIdIssueTracks}` : "",
    Number(pd?.artworkMismatchTracks || 0) ? `art mismatch ${pd.artworkMismatchTracks}` : "",
  ].filter(Boolean);
}
export function diagStatusIcon(status) {
  if (status === "PASS") return "\u2713";
  if (status === "WARN") return "\u26A0";
  return "\u2717";
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
  ].filter(Boolean);

  const cdj = data.cdjCounterSnapshot;
  if (cdj) {
    const dataPage = cdj.t19?.dataPage || null;
    const t19Line = dataPage
      ? `t19 ec=${Number(cdj.t19.ec || 0)} chain=${Number(cdj.t19.chainLen || 0)} p${Number(dataPage.page || 0)} nrs=${Number(dataPage.nrs || 0)} num_rl=${Number(dataPage.numRl || 0)} rowpf0=0x${Number(dataPage.rowpf0 || 0).toString(16).padStart(4, "0")} tranrf0=0x${Number(dataPage.tranrf0 || 0).toString(16).padStart(4, "0")}`
      : `t19 ec=${Number(cdj.t19?.ec || 0)} chain=${Number(cdj.t19?.chainLen || 0)}`;
    sections.push({
      title: "Player Counter Snapshot",
      status: String(cdj.confidence || "low").toLowerCase() === "high" ? "PASS" : "WARN",
      checks: [
        {
          label: "Predicted player counter",
          status: "PASS",
          detail: `playlists ${Number(cdj.playlistCountCandidate || 0)}, songs ${Number(cdj.songCountCandidate || 0)} (confidence: ${String(cdj.confidence || "low")})`
        },
        {
          label: "Shape mode",
          status: "PASS",
          detail: `${String(cdj.shapeMode || "unknown")} (baseline-init-like: ${cdj.baselineInitLike ? "yes" : "no"})`
        },
        {
          label: "Cross-check",
          status: "PASS",
          detail: `t00 tracks ${Number(cdj.t00Tracks || 0)}, t08 entries ${Number(cdj.t08Entries || 0)}`
        },
        {
          label: "History pointers",
          status: "PASS",
          detail: `t11 ${Number(cdj.t11?.first || 0)}-${Number(cdj.t11?.last || 0)} ec=${Number(cdj.t11?.ec || 0)} | t12 ${Number(cdj.t12?.first || 0)}-${Number(cdj.t12?.last || 0)} ec=${Number(cdj.t12?.ec || 0)} | t17 ${Number(cdj.t17?.first || 0)}-${Number(cdj.t17?.last || 0)} ec=${Number(cdj.t17?.ec || 0)} | t18 ${Number(cdj.t18?.first || 0)}-${Number(cdj.t18?.last || 0)} ec=${Number(cdj.t18?.ec || 0)}`
        },
        {
          label: "Primitive signal",
          status: "PASS",
          detail: t19Line
        }
      ]
    });
  }

  el.diagSections.innerHTML = "";
  for (const sec of sections) {
    const div = (deps.documentObj || document).createElement("div");
    div.className = "diag-section";

    const header = (deps.documentObj || document).createElement("h3");
    header.innerHTML = `<span class="diag-dot diag-${sec.status.toLowerCase()}"></span> ${escapeHtml(sec.title)}`;
    div.appendChild(header);

    for (const check of (sec.checks || [])) {
      const row = (deps.documentObj || document).createElement("div");
      row.className = `diag-check diag-check-${check.status.toLowerCase()}`;
      row.innerHTML = `<span class="diag-indicator">${diagStatusIcon(check.status)}</span> <strong>${escapeHtml(check.label)}</strong>: ${escapeHtml(check.detail)}`;
      if (check.link === "event-log" && typeof switchView === "function") {
        const btn = (deps.documentObj || document).createElement("button");
        btn.className = "diag-log-link";
        btn.textContent = "→ event log";
        btn.addEventListener("click", () => switchView("event-log").catch((err) => console.error(err)));
        row.appendChild(btn);
      }
      div.appendChild(row);
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
    const row = (deps.documentObj || document).createElement("div");
    row.className = `diag-check diag-check-${check.status.toLowerCase()}`;
    row.innerHTML = `<span class="diag-indicator">${diagStatusIcon(check.status)}</span> <strong>${escapeHtml(check.label)}</strong>: ${escapeHtml(check.detail)}`;
    div.appendChild(row);
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

export function hideUsbDiagnostics(el) {
  if (el.usbDiagnosticsCard) {
    el.usbDiagnosticsCard.classList.add("hidden");
  }
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
    const fixesToRender = fixes.map((f) => ({ ...f }));
    const unindexedPattern = /unindexed audio file\(s\) under contents/i;
    const missingAudioPattern = /missing-audio reference\(s\) require manual review/i;
    const unindexedIdx = fixesToRender.findIndex((f) =>
      /manual re-import unindexed audio/i.test(String(f?.title || ""))
    );
    const missingAudioIdx = fixesToRender.findIndex((f) =>
      /remove missing audio references/i.test(String(f?.title || ""))
      && !f?.supported
    );
    if (unindexedIdx >= 0) {
      const unindexedItem = unsupportedItems.find((item) =>
        unindexedPattern.test(String(item?.issue || ""))
      );
      if (unindexedItem) {
        const note = `${unindexedItem.reason} See Event Log for full path list/details.`;
        fixesToRender[unindexedIdx].description = note;
      }
    }
    if (missingAudioIdx >= 0) {
      const missingAudioItem = unsupportedItems.find((item) =>
        missingAudioPattern.test(String(item?.issue || ""))
      );
      if (missingAudioItem) {
        fixesToRender[missingAudioIdx].description = `${missingAudioItem.issue}. ${missingAudioItem.reason}`;
      }
    }
    for (const item of unsupportedItems) {
      if (unindexedPattern.test(String(item?.issue || "")) && unindexedIdx >= 0) continue;
      if (missingAudioPattern.test(String(item?.issue || "")) && missingAudioIdx >= 0) continue;
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
        const checkbox = documentObj.createElement("input");
        checkbox.type = "checkbox";
        checkbox.className = "diag-repair-fix-check";
        checkbox.checked = selectedFixIds.has(String(fix.id || ""));
        checkbox.dataset.fixId = String(fix.id || "");
        checkbox.addEventListener("change", (event) => {
          onToggleFixSelection(String(fix.id || ""), !!event?.target?.checked);
        });
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
    renderUsbPlaylistTracks = () => {},
    renderHistoryList = () => {},
    renderHistoryTracks = () => {},
    renderUsbPlayerMenuEditor = () => {}
  } = deps;

  state.usbPlaylists = [];
  state.usbKnownPlaylistNames = new Set();
  state.usbPlaylistTracks = [];
  state.usbPlaylistTracksView = [];
  state.histories = [];
  state.historyTracks = [];
  state.historyTracksView = [];
  state.usbPlayerMenuCurrent = [];
  state.usbPlayerMenuAvailable = [];
  state.usbPlayerMenuCurrentSelectedKind = null;
  state.usbPlayerMenuAvailableSelectedKind = null;

  el.usbCountsText.textContent = "";
  el.historyCountsText.textContent = "";
  hideUsbDiagnostics(el);

  renderUsbPlaylists();
  renderUsbPlaylistTracks();
  renderHistoryList();
  renderHistoryTracks();
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

  const input = String(path || "").trim();
  const previousRoot = state.usbRoot;
  if (input && previousRoot && input !== previousRoot) {
    hideUsbDiagnostics(el);
  }
  if (!input) {
    state.usbRoot = null;
    state.usbRootValid = false;
    state.usbNeedsInit = false;
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
    const warnings = Array.isArray(result?.warnings) ? result.warnings : [];
    if (canInitialize) {
      const reason = warnings.length ? ` (${warnings.join(" | ")})` : "";
      el.usbInitHint.textContent = `USB folder is writable but missing External library structure${reason}`;
    } else if (!valid) {
      const reason = warnings.length ? ` (${warnings.join(" | ")})` : "";
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
  if (!silent) {
    if (valid) {
      const warningText = Array.isArray(result?.warnings) && result.warnings.length
        ? ` (${result.warnings.join(" | ")})`
        : "";
      emitStatus(`USB root selected: ${state.usbRoot}${warningText}. Running diagnostics...`);
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
      const warnings = Array.isArray(result?.warnings) ? result.warnings.join(" | ") : "invalid USB root";
      emitStatus(`USB root invalid: ${warnings}`);
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
    countWarningsForStatus
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
  await refreshUsb();
  const warningCount = typeof countWarningsForStatus === "function"
    ? countWarningsForStatus(data.warnings)
    : ((data.warnings || []).length || 0);
  const warningSuffix = warningCount ? ` (${warningCount} warning(s))` : "";
  emitStatus(
    `Removed USB playlist: ${playlist.name} [db ${data.removedFromEdb || 0}, pdb ${data.removedFromPdb || 0}]${warningSuffix}`
  );
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
    rebuildKnownUsbPlaylistNames,
    renderUsbPlaylists,
    renderUsbPlaylistTracks,
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
  rebuildKnownUsbPlaylistNames();

  setProgress(true, 80, "Computing stats...");
  await new Promise((r) => setTimeout(r, 20));

  const usbTrackTotal = state.usbPlaylists.reduce((sum, playlist) => sum + (playlist.tracks?.length || 0), 0);
  el.usbCountsText.textContent = `${state.usbPlaylists.length} playlists, ${usbTrackTotal} tracks`;
  state.usbPlaylistTracks = [];
  setProgress(true, 90, "Rendering playlists...");
  await new Promise((r) => setTimeout(r, 20));
  renderUsbPlaylists();
  renderUsbPlaylistTracks();
  updatePlaylistExportButtons();

  const warningCount = countWarningsForStatus(data.warnings);
  const warningSuffix = warningCount ? ` (${warningCount} warning(s))` : "";
  logWarnings("usb-import", data.warnings, "fetch_usb_playlists");
  setProgress(true, 100, `Done — ${state.usbPlaylists.length} playlists, ${usbTrackTotal} tracks`);
  emitStatus(`USB playlists loaded: ${state.usbPlaylists.length}${warningSuffix}`);
  setTimeout(() => setProgress(false, 0, "Idle"), 1200);
}

export async function runUsbDiagnostics(state, deps) {
  const {
    setStatus,
    command,
    normalizePlaylistNameForCompare,
    updatePlaylistExportButtons,
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
  state.usbKnownPlaylistNames = new Set(
    (data?.playlistDetails || [])
      .map((entry) => normalizePlaylistNameForCompare(entry?.name))
      .filter(Boolean)
  );
  updatePlaylistExportButtons();
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
    runUsbDiagnostics
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
  logWarnings("usb-diagnostics", data.warnings, "repair_usb_diagnostics apply");
  emitStatus(`Repair apply complete: ${applied} applied, ${failed} failed (${data.durationMs}ms). Re-diagnosing...`);
  try {
    await runUsbDiagnostics();
  } catch (err) {
    console.warn("Post-repair diagnostics failed:", err);
  }
}

export async function refreshHistory(state, el, deps) {
  const { setStatus, command, normalizeTrack, countWarningsForStatus, logWarnings, renderHistoryList, renderHistoryTracks } = deps;
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
  const counts = data.counts || {};
  const importedPlaylists = Number.isFinite(counts.importedPlaylists) ? counts.importedPlaylists : state.histories.length;
  const importedTracks = Number.isFinite(counts.importedTracks)
    ? counts.importedTracks
    : state.histories.reduce((sum, history) => sum + (history.tracks?.length || 0), 0);
  el.historyCountsText.textContent = `${importedPlaylists} sessions, ${importedTracks} tracks`;
  state.historyTracks = [];
  renderHistoryList();
  renderHistoryTracks();
  const warningCount = countWarningsForStatus(data.warnings);
  const warningSuffix = warningCount ? ` (${warningCount} warning(s))` : "";
  logWarnings("usb-import", data.warnings, "fetch_usb_histories");
  emitStatus(`USB histories loaded: ${state.histories.length}${warningSuffix}`);
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
    tag.title = origin === "pdb_only"
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
  const div = state.usbPlayerMenuDivergence || {};
  const inEdbOnly = Array.isArray(div.inEdbVisibleOnly) ? div.inEdbVisibleOnly : [];
  const pdbMissingKinds = Array.isArray(div.pdbMissingKinds) ? div.pdbMissingKinds : [];
  const canFix = !!(state.usbRoot && state.usbRootValid);
  const noProblems = inEdbOnly.length === 0 && pdbMissingKinds.length === 0;
  if (!canFix || noProblems) {
    node.classList.add("hidden");
    if (el.usbPlayerMenuDivergenceMessage) el.usbPlayerMenuDivergenceMessage.textContent = "";
    if (el.usbPlayerMenuSyncBtn) el.usbPlayerMenuSyncBtn.disabled = true;
    if (el.usbPlayerMenuRestoreBtn) el.usbPlayerMenuRestoreBtn.disabled = true;
    return;
  }
  const parts = [];
  if (inEdbOnly.length > 0) {
    parts.push(`${inEdbOnly.length} active menu items missing from PDB`);
  }
  if (pdbMissingKinds.length > 0) {
    parts.push(`PDB missing ${pdbMissingKinds.length} browse categories`);
  }
  node.classList.remove("hidden");
  const msg = parts.join("; ") + ".";
  if (el.usbPlayerMenuDivergenceMessage) {
    el.usbPlayerMenuDivergenceMessage.textContent = msg;
  } else {
    node.textContent = msg;
  }
  if (el.usbPlayerMenuSyncBtn) {
    el.usbPlayerMenuSyncBtn.disabled = inEdbOnly.length === 0 || !canFix;
  }
  if (el.usbPlayerMenuRestoreBtn) {
    el.usbPlayerMenuRestoreBtn.disabled = pdbMissingKinds.length === 0 || !canFix;
  }
}

export async function syncUsbPlayerMenusEdbToPdb(state, el, deps) {
  const { command } = deps;
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
  emitStatus(data?.updated ? "PDB categories restored" : "PDB already complete");
}

const PROTECTED_PLAYER_MENU_KINDS = new Set([131, 132, 144, 145, 149]);

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
      !hasRoot || !hasCurrent || PROTECTED_PLAYER_MENU_KINDS.has(currentSelected);
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
  const { command } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (!state.usbRoot || !state.usbRootValid) {
    emitStatus("Select USB folder first");
    return;
  }
  const data = await command("update_usb_player_menu_config", {
    usbRoot: state.usbRoot,
    currentKinds,
  });
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
  if (PROTECTED_PLAYER_MENU_KINDS.has(selected)) return;
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
    renderUsbPlaylistTracks,
    refreshMissingSourceRoots = async () => []
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
  const warningCount = countWarningsForStatus(data.warnings);
  const warningSuffix = warningCount ? ` (${warningCount} warning(s))` : "";
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
    `Export complete: ${playlist.name} - ${data.exportedTracks || 0} track(s), ${data.skippedTracks || 0} skipped${warningSuffix}${state.exportPruneStale ? " [sync: mirror]" : " [sync: additive]"}`
  );
  await loadPlaylists();
  state.currentPlaylistId = playlistId;
  updateModeText();
  await switchView(playlistId);

  state.usbPlaylists = [];
  state.usbPlaylistTracks = [];
  renderUsbPlaylists();
  renderUsbPlaylistTracks();
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
    const source = String(playlist.source || "unknown");
    el.usbPlaylists.insertAdjacentHTML(
      "beforeend",
      `<li><button data-usb-playlist-index="${index}" data-usb-playlist="${escapeHtml(playlist.id)}" title="Source: ${escapeHtml(source)}"><span class="playlist-label">${escapeHtml(playlist.name)} (${count})</span><span class="playlist-remove" data-usb-remove-playlist="${escapeHtml(playlist.id)}" title="Remove">&times;</span></button></li>`
    );
  });
}

export function renderUsbPlaylistTracks(state, el, deps = {}) {
  const {
    filterTracksByQuery = (tracks) => tracks,
    applySortToTracks = (tracks) => tracks,
    renderTrackTable = () => {},
    updateTrackListDurationSummary = () => {}
  } = deps;
  state.usbPlaylistTracksView = filterTracksByQuery(state.usbPlaylistTracks, state.usbTrackSearch);
  const sortedUsb = applySortToTracks(state.usbPlaylistTracksView, "usbPlaylistTracks");
  renderTrackTable(el.usbPlaylistTracks, sortedUsb, {
    withCheckbox: false,
    actionLabel: "+",
    actionType: "add-usb",
    compactAddButton: true,
    enableAnalyzeActions: false,
    origin: "usb",
    secondaryActionLabel: "Play",
    secondaryActionType: "play-usb"
  });
  updateTrackListDurationSummary(el.usbPlaylistTotalDuration, state.usbPlaylistTracksView);
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

export function renderHistoryTracks(state, el, deps = {}) {
  const {
    filterTracksByQuery = (tracks) => tracks,
    applySortToTracks = (tracks) => tracks,
    renderTrackTable = () => {},
    updateTrackListDurationSummary = () => {}
  } = deps;
  state.historyTracksView = filterTracksByQuery(state.historyTracks, state.historyTrackSearch);
  const sortedHistory = applySortToTracks(state.historyTracksView, "historyTracks");
  renderTrackTable(el.historyTracks, sortedHistory, {
    withCheckbox: false,
    actionLabel: "+",
    actionType: "add-history",
    compactAddButton: true,
    enableAnalyzeActions: false,
    origin: "usb",
    secondaryActionLabel: "Play",
    secondaryActionType: "play-history"
  });
  updateTrackListDurationSummary(el.historyTotalDuration, state.historyTracksView);
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
    validateAndSetUsbRoot = async () => {}
  } = deps;
  const selected = await invoke("pick_usb_folder");
  if (!selected) return null;
  await validateAndSetUsbRoot(String(selected), false);
  return selected;
}

export async function hydrateUsbTrackMetadata(state, track, deps = {}) {
  const {
    usbTrackNeedsHydration = () => false,
    command = async () => ({}),
    normalizeTrack = (t) => t
  } = deps;
  if (!track || !usbTrackNeedsHydration(track)) return track;
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
    const normalized = normalizeTrack(inspected?.track || {}, "usb");
    if (!normalized.localTrackId && track.localTrackId) {
      normalized.localTrackId = track.localTrackId;
    }
    normalized.artworkChecked = true;
    Object.assign(track, normalized);
  } catch (err) {
    console.warn(`inspect_usb_track failed for ${trackId}:`, err);
  }
  return track;
}

export function rebuildKnownUsbPlaylistNames(state) {
  state.usbKnownPlaylistNames = knownUsbPlaylistNamesFromPlaylists(state.usbPlaylists);
}

export function renderUsbRecentRoots(el, rows, document) {
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
  normalizedRows.forEach((path) => {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "usb-cfg-recent-btn";
    btn.dataset.usbRecentPath = path;
    btn.title = path;
    btn.style.direction = "rtl";
    btn.style.textAlign = "left";
    btn.textContent = path;
    el.usbRecentList.appendChild(btn);
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
