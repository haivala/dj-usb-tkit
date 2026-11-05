// USB DB backup snapshot list/restore/delete UI logic.

import { counted } from "../../track_utils.mjs";
import { cloneTemplate } from "../../ui_utils.mjs";

function formatBackupSize(bytes) {
  const n = Number(bytes) || 0;
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function showBackupsMessage(list, text) {
  const row = cloneTemplate(list.ownerDocument, "tplLogMessageRow");
  row.firstElementChild.textContent = text;
  list.replaceChildren(row);
}

export async function renderBackups(ctx) {
  const { state, el, command } = ctx;

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
  el.backupsSummary.textContent = counted(items.length, "backup");
  if (!items.length) {
    showBackupsMessage(el.backupsList, "No backups yet.");
    return;
  }

  el.backupsList.replaceChildren(...items.map((item) => {
    const row = cloneTemplate(el.backupsList.ownerDocument, "tplBackupRow");
    const location = item.location === "usb" ? "On USB" : "On this computer";
    const playlistCount = Number.isFinite(item.playlistCount)
      ? ` · ${counted(item.playlistCount, "playlist")}`
      : "";
    row.dataset.timestamp = item.timestamp;
    row.querySelector(".event-log-time").textContent = item.timeText;
    row.querySelector(".event-log-source").textContent = item.reason || "—";
    row.querySelector(".event-log-message").textContent = `${formatBackupSize(item.sizeBytes)} · ${location}${playlistCount}`;
    row.querySelectorAll("button").forEach((btn) => { btn.dataset.timestamp = item.timestamp; });
    return row;
  }));
}

export async function restoreUsbBackup(ctx, timestamp) {
  const { state, command, openConfirmDialog, emitStatus } = ctx;
  if (!state.usbRoot) return;

  const backup = state.usbBackups.find((b) => b.timestamp === timestamp);
  const confirmed = await openConfirmDialog({
    title: "Restore Backup",
    message: backup.restoreConfirmText,
    confirmLabel: "Restore"
  });
  if (!confirmed) return;

  let data;
  try {
    data = await command("restore_usb_backup", { usbRoot: state.usbRoot, timestamp });
  } catch (err) {
    emitStatus(`Restore failed: ${err?.message || err}`);
    await renderBackups(ctx);
    return;
  }
  emitStatus(data.statusText);
  // The restored files may no longer match the diagnostics report, lists or
  // player menu on screen. The same drive is still selected, so the lists read
  // again when their page opens.
  ctx.clearUsbDiagnostics();
  ctx.usbContentChanged();
  ctx.applyPlaylistUsbExportStatus(data.playlistUsbExportStatus);
  await ctx.renderCurrentPlaylistTracksFromState();
  await renderBackups(ctx);
}

export async function deleteUsbBackup(ctx, timestamp) {
  const { state, command, openConfirmDialog, emitStatus } = ctx;
  if (!state.usbRoot) return;

  const backup = state.usbBackups.find((b) => b.timestamp === timestamp);
  const confirmed = await openConfirmDialog({
    title: "Delete Backup",
    message: backup.deleteConfirmText,
    confirmLabel: "Delete"
  });
  if (!confirmed) return;

  try {
    const data = await command("delete_usb_backup", { usbRoot: state.usbRoot, timestamp });
    emitStatus(data.statusText);
  } catch (err) {
    emitStatus(`Delete failed: ${err?.message || err}`);
  }
  await renderBackups(ctx);
}
