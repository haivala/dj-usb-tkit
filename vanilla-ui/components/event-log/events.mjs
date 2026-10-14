export function bindEventLogEvents(ctx) {
  const { el, eventLogStore, renderEventLog } = ctx;

  el.eventLogBtn?.addEventListener("click", () => {
    ctx.switchView("event-log").catch((err) => {
      console.error(err);
      ctx.emitStatus(err.message || String(err));
    });
  });

  el.eventLogLevelFilter?.addEventListener("change", () => renderEventLog());
  el.eventLogSourceFilter?.addEventListener("change", () => renderEventLog());
  el.eventLogClearBtn?.addEventListener("click", () => {
    eventLogStore.clear();
    ctx.clearEventLogBadge();
    renderEventLog();
  });
  el.eventLogOpenFileBtn?.addEventListener("click", () => ctx.openEventLogFile());
}
