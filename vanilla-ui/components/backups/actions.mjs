// USB DB backup snapshot list/restore/delete UI logic.

import { cloneTemplate } from "../../ui_utils.mjs";
import { t } from "../../ui_texts.mjs";

function showBackupsMessage(list, text) {
  const row = cloneTemplate(list.ownerDocument, "tplLogMessageRow");
  row.firstElementChild.textContent = text;
  list.replaceChildren(row);
}

export async function renderBackups(ctx) {
  const { state, el, command } = ctx;

  let data;
  try {
    data = await command("list_usb_backups", { usbRoot: state.usbRoot });
  } catch (err) {
    el.backupsSummary.textContent = t("ui-backups-load-failed");
    showBackupsMessage(el.backupsList, err?.message || String(err));
    return;
  }
  applyBackups(ctx, data);
}

// A backups list as list_usb_backups, a restore or a delete answers it.
function applyBackups(ctx, data) {
  const { state, el } = ctx;
  const { items } = data;
  state.usbBackups = items;
  el.backupsSummary.textContent = data.summaryText;
  if (!items.length) {
    showBackupsMessage(el.backupsList, data.emptyText);
    return;
  }

  el.backupsList.replaceChildren(...items.map((item) => {
    const row = cloneTemplate(el.backupsList.ownerDocument, "tplBackupRow");
    row.dataset.timestamp = item.timestamp;
    row.querySelector(".event-log-time").textContent = item.timeText;
    row.querySelector(".event-log-source").textContent = item.reasonText;
    row.querySelector(".event-log-message").textContent = item.detailText;
    row.querySelectorAll("button").forEach((btn) => { btn.dataset.timestamp = item.timestamp; });
    return row;
  }));
}

export async function restoreUsbBackup(ctx, timestamp) {
  const { state, command, openConfirmDialog, emitStatus } = ctx;
  const backup = state.usbBackups.find((b) => b.timestamp === timestamp);
  const confirmed = await openConfirmDialog({
    title: t("ui-backups-restore-title"),
    message: backup.restoreConfirmText,
    confirmLabel: t("ui-backups-restore")
  });
  if (!confirmed) return;

  let data;
  try {
    data = await command("restore_usb_backup", { usbRoot: state.usbRoot, timestamp });
  } catch (err) {
    emitStatus(err?.message || String(err));
    await renderBackups(ctx);
    return;
  }
  emitStatus(data.statusText);
  // The restored files may no longer match the diagnostics report, lists or
  // player menu on screen. The same drive is still selected, so the lists read
  // again when their page opens.
  ctx.clearUsbDiagnostics();
  ctx.usbContentChanged();
  ctx.applyPlaylistsState(data);
  await ctx.renderCurrentPlaylistTracksFromState();
  applyBackups(ctx, data.backups);
}

export async function deleteUsbBackup(ctx, timestamp) {
  const { state, command, openConfirmDialog, emitStatus } = ctx;
  const backup = state.usbBackups.find((b) => b.timestamp === timestamp);
  const confirmed = await openConfirmDialog({
    title: t("ui-backups-delete-title"),
    message: backup.deleteConfirmText,
    confirmLabel: t("ui-common-delete")
  });
  if (!confirmed) return;

  let data;
  try {
    data = await command("delete_usb_backup", { usbRoot: state.usbRoot, timestamp });
  } catch (err) {
    emitStatus(err?.message || String(err));
    // Some of its files may be gone: show the list as it is now.
    await renderBackups(ctx);
    return;
  }
  emitStatus(data.statusText);
  applyBackups(ctx, data.backups);
}
