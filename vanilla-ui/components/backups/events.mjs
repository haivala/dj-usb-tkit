export function bindBackupsEvents(ctx) {
  const { el } = ctx;

  el.backupsRefreshBtn?.addEventListener("click", () => ctx.renderBackups());

  el.backupsList?.addEventListener("click", async (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;

    const restoreBtn = target.closest(".backups-restore-btn");
    if (restoreBtn) {
      await ctx.restoreUsbBackup(restoreBtn.dataset.timestamp);
      return;
    }

    const deleteBtn = target.closest(".backups-delete-btn");
    if (deleteBtn) {
      await ctx.deleteUsbBackup(deleteBtn.dataset.timestamp);
    }
  });
}
