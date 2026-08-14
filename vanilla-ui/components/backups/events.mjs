import { restoreUsbBackup, deleteUsbBackup } from "./actions.mjs";

export function bindBackupsEvents(ctx) {
  const { state, el, command, openConfirmDialog, setStatus, renderBackups } = ctx;

  el.backupsRefreshBtn?.addEventListener("click", () => renderBackups());

  el.backupsList?.addEventListener("click", async (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const reload = () => renderBackups();

    const restoreBtn = target.closest(".backups-restore-btn");
    if (restoreBtn) {
      await restoreUsbBackup(state, restoreBtn.dataset.stem, restoreBtn.dataset.filename, {
        command,
        openConfirmDialog,
        setStatus,
        reload
      });
      return;
    }

    const deleteBtn = target.closest(".backups-delete-btn");
    if (deleteBtn) {
      await deleteUsbBackup(state, deleteBtn.dataset.stem, deleteBtn.dataset.filename, {
        command,
        openConfirmDialog,
        setStatus,
        reload
      });
    }
  });
}
