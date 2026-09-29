// USB DB backup snapshot list/restore/delete UI logic.

import { cloneTemplate } from "../../ui_utils.mjs";

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

function showBackupsMessage(list, text) {
  const row = cloneTemplate(list.ownerDocument, "tplLogMessageRow");
  row.firstElementChild.textContent = text;
  list.replaceChildren(row);
}

export async function renderBackups(ctx) {
  const { state, el, command } = ctx;
  if (!el.backupsList || !el.backupsSummary) return;

  if (!state.usbRoot) {
    el.backupsSummary.textContent = "No USB connected";
    showBackupsMessage(el.backupsList, "Connect a USB to see its backups.");
    return;
  }

  let items = [];
  try {
    const data = await command("list_usb_backups", { usbRoot: state.usbRoot });
    items = Array.isArray(data?.items) ? data.items : [];
  } catch (err) {
    el.backupsSummary.textContent = "Failed to load backups";
    showBackupsMessage(el.backupsList, err?.message || String(err));
    return;
  }

  state.usbBackups = items;
  el.backupsSummary.textContent = `${items.length} backup(s)`;
  if (!items.length) {
    showBackupsMessage(el.backupsList, "No backups yet.");
    return;
  }

  el.backupsList.replaceChildren(...items.map((item) => {
    const row = cloneTemplate(el.backupsList.ownerDocument, "tplBackupRow");
    const location = item.location === "usb" ? "On USB" : "On this computer";
    const playlistCount = Number.isFinite(item.playlistCount)
      ? ` · ${item.playlistCount} playlist${item.playlistCount === 1 ? "" : "s"}`
      : "";
    row.dataset.timestamp = item.timestamp;
    row.querySelector(".event-log-time").textContent = formatBackupTimestamp(item.timestamp);
    row.querySelector(".event-log-source").textContent = item.reason || "—";
    row.querySelector(".event-log-message").textContent = `${formatBackupSize(item.sizeBytes)} · ${location}${playlistCount}`;
    row.querySelectorAll("button").forEach((btn) => { btn.dataset.timestamp = item.timestamp; });
    return row;
  }));
}

export async function restoreUsbBackup(ctx, timestamp) {
  const { state, command, openConfirmDialog, emitStatus } = ctx;
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
    emitStatus(`Restored ${label} from backup`);
    // The restored files may no longer match whatever diagnostics report,
    // playlists, histories, or player-menu state are on screen -- clear
    // them rather than show stale results; the user can reload if they
    // want fresh ones. The same drive is still selected, so only the
    // diagnostics report is cleared, not the whole panel.
    ctx.clearUsbDiagnostics();
    ctx.resetUsbStateViews({ hideDiagnostics: false });
  } catch (err) {
    emitStatus(`Restore failed: ${err?.message || err}`);
  }
  await renderBackups(ctx);
}

export async function deleteUsbBackup(ctx, timestamp) {
  const { state, command, openConfirmDialog, emitStatus } = ctx;
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
    emitStatus(`Deleted ${label} backup`);
  } catch (err) {
    emitStatus(`Delete failed: ${err?.message || err}`);
  }
  await renderBackups(ctx);
}
