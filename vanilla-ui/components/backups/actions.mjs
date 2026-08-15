// USB DB backup snapshot list/restore/delete UI logic.

function formatBackupTimestamp(raw) {
  // Stored as "%Y-%m-%d_%H-%M-%S" (usb_vendor_compat::backup_usb_databases).
  const match = /^(\d{4}-\d{2}-\d{2})_(\d{2})-(\d{2})-(\d{2})$/.exec(String(raw || ""));
  if (!match) return String(raw || "");
  const [, date, hh, mm, ss] = match;
  return `${date} ${hh}:${mm}:${ss}`;
}

function formatBackupSize(bytes) {
  const n = Number(bytes) || 0;
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

// PDB and eDB are always backed up together under one timestamp, so a
// bundle's label lists whichever of the two files it actually contains.
const STEM_LABELS = { exportLibrary: "eDB", export: "PDB" };
const STEM_LABEL_ORDER = ["exportLibrary", "export"];

function labelForFiles(files) {
  const present = STEM_LABEL_ORDER.filter((stem) => (files || []).some((f) => f.stem === stem));
  return present.length ? present.map((stem) => STEM_LABELS[stem]).join(" and ") : "Backup";
}

export async function renderBackups(state, el, document, deps = {}) {
  const { command, escapeHtml = (s) => String(s) } = deps;
  if (!el.backupsList || !el.backupsSummary) return;

  if (!state.usbRoot) {
    el.backupsSummary.textContent = "No USB connected";
    el.backupsList.innerHTML = `<div class="event-log-row"><div class="event-log-message muted">Connect a USB to see its backups.</div></div>`;
    return;
  }

  let items = [];
  try {
    const data = await command("list_usb_backups", { usbRoot: state.usbRoot });
    items = Array.isArray(data?.items) ? data.items : [];
  } catch (err) {
    el.backupsSummary.textContent = "Failed to load backups";
    el.backupsList.innerHTML = `<div class="event-log-row"><div class="event-log-message muted">${escapeHtml(err?.message || String(err))}</div></div>`;
    return;
  }

  state.usbBackups = items;
  el.backupsSummary.textContent = `${items.length} backup(s)`;
  if (!items.length) {
    el.backupsList.innerHTML = `<div class="event-log-row"><div class="event-log-message muted">No backups yet.</div></div>`;
    return;
  }

  el.backupsList.innerHTML = items.map((item) => {
    const label = escapeHtml(labelForFiles(item.files));
    const rawTimestamp = escapeHtml(item.timestamp);
    const timestamp = escapeHtml(formatBackupTimestamp(item.timestamp));
    const size = escapeHtml(formatBackupSize(item.sizeBytes));
    const location = item.location === "usb" ? "On USB" : "On this computer";
    return `<div class="event-log-row" data-timestamp="${rawTimestamp}">
      <div class="event-log-time">${timestamp}</div>
      <div class="event-log-source">${label}</div>
      <div class="event-log-message">${size} · ${escapeHtml(location)}</div>
      <div class="backups-row-actions">
        <button type="button" class="backups-restore-btn" data-timestamp="${rawTimestamp}">Restore</button>
        <button type="button" class="backups-delete-btn" data-timestamp="${rawTimestamp}">Delete</button>
      </div>
    </div>`;
  }).join("");
}

export async function restoreUsbBackup(state, timestamp, deps = {}) {
  const {
    command,
    openConfirmDialog = async () => true,
    setStatus = () => {},
    reload = async () => {},
    clearUsbDiagnostics = () => {}
  } = deps;
  if (!state.usbRoot) return;

  const known = (state.usbBackups || []).find((b) => b.timestamp === timestamp);
  const label = known ? labelForFiles(known.files) : "backup";
  const whenText = formatBackupTimestamp(timestamp);
  const confirmed = await openConfirmDialog({
    title: "Restore Backup",
    message: `Restore ${label} from the backup taken at ${whenText}? The current files will themselves be backed up first.`,
    confirmLabel: "Restore"
  });
  if (!confirmed) return;

  try {
    await command("restore_usb_backup", { usbRoot: state.usbRoot, timestamp });
    setStatus(`Restored ${label} from backup`);
    // The restored files may no longer match whatever diagnostics report is
    // on screen -- clear it rather than show a stale result; the user can
    // re-run diagnostics if they want a fresh one. The same drive is still
    // selected, so only the report is cleared, not the whole panel.
    clearUsbDiagnostics();
  } catch (err) {
    setStatus(`Restore failed: ${err?.message || err}`);
  }
  await reload();
}

export async function deleteUsbBackup(state, timestamp, deps = {}) {
  const { command, openConfirmDialog = async () => true, setStatus = () => {}, reload = async () => {} } = deps;
  if (!state.usbRoot) return;

  const known = (state.usbBackups || []).find((b) => b.timestamp === timestamp);
  const label = known ? labelForFiles(known.files) : "backup";
  const confirmed = await openConfirmDialog({
    title: "Delete Backup",
    message: `Delete this ${label} backup permanently?`,
    confirmLabel: "Delete"
  });
  if (!confirmed) return;

  try {
    await command("delete_usb_backup", { usbRoot: state.usbRoot, timestamp });
    setStatus(`Deleted ${label} backup`);
  } catch (err) {
    setStatus(`Delete failed: ${err?.message || err}`);
  }
  await reload();
}
