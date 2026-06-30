export function bindEventLogEvents(ctx) {
  const {
    state,
    el,
    eventLogStore,
    renderEventLog
  } = ctx;

  el.eventLogLevelFilter?.addEventListener("change", () => renderEventLog());
  el.eventLogSourceFilter?.addEventListener("change", () => renderEventLog());
  el.eventLogClearBtn?.addEventListener("click", () => {
    eventLogStore.clear();
    state.eventLogEntries = [];
    renderEventLog();
  });
}
