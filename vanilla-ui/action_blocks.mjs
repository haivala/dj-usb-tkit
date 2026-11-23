// What a running job blocks. The table is the backend's (`job_conflicts.rs`, documented in
// docs/COMMANDS.md "What a running job blocks"): whenever a job starts or ends it sends the
// blocked action groups with the reason for each (`jobs:blocked`). Here, and only here, the
// UI follows it: every control marked `data-action-group="<group>"` in index.html (also
// ones drawn later from a <template>) is off while its group is blocked -- `aria-disabled`,
// the reason as its tooltip (`data-blocked-reason`), clicks, presses (a <select> opens on
// one) and keys stopped, with the reason in the status line. A control's own `disabled`
// (nothing selected, nothing to do) stays its own.
//
// Code that acts without a click (switching to a plugged-in USB, a save waiting for a USB
// job) asks `actionBlockedReason` / `whenActionUnblocked`.

const MARKED = "[data-action-group]";
const BLOCKED = "[data-blocked-reason]";

// A control's own `aria-disabled` (the USB name badge has one) is kept aside while
// it's blocked and put back after.
function syncControl(state, node) {
  const reason = state.blockedActions[node.dataset.actionGroup];
  const blocked = "blockedReason" in node.dataset;
  if (reason) {
    if (!blocked) node.dataset.ownAriaDisabled = node.getAttribute("aria-disabled") ?? "";
    node.setAttribute("aria-disabled", "true");
    node.dataset.blockedReason = reason;
  } else if (blocked) {
    const own = node.dataset.ownAriaDisabled;
    if (own) node.setAttribute("aria-disabled", own);
    else node.removeAttribute("aria-disabled");
    delete node.dataset.blockedReason;
    delete node.dataset.ownAriaDisabled;
  }
}

function syncTree(state, root) {
  if (root.matches?.(MARKED)) syncControl(state, root);
  root.querySelectorAll?.(MARKED).forEach((node) => syncControl(state, node));
}

// Once at startup: marks controls drawn later and stops input to blocked ones.
export function initActionBlocks(ctx) {
  const { document, window, state } = ctx;
  new window.MutationObserver((records) => {
    for (const record of records) record.addedNodes.forEach((node) => syncTree(state, node));
  }).observe(document.body, { childList: true, subtree: true });
  const stop = (event) => {
    const control = event.target?.closest?.(BLOCKED);
    if (!control) return;
    if (event.type === "keydown" && (event.key === "Tab" || event.key === "Escape")) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (event.type !== "mousedown") ctx.emitStatus(control.dataset.blockedReason);
  };
  for (const type of ["click", "mousedown", "keydown"]) document.addEventListener(type, stop, true);
}

// `jobs:blocked`: the groups the running jobs block now, `{ group: reason }`.
export function applyBlockedActions(ctx, blockedActions) {
  const { state, document } = ctx;
  state.blockedActions = blockedActions || {};
  syncTree(state, document.body);
  for (const [group, waiters] of state.actionBlockWaiters) {
    if (state.blockedActions[group]) continue;
    state.actionBlockWaiters.delete(group);
    waiters.forEach((resolve) => resolve());
  }
}

// Why `group` is blocked now, or null.
export function actionBlockedReason(ctx, group) {
  return ctx.state.blockedActions[group] || null;
}

// Resolves once `group` isn't blocked: at once when it isn't.
export function whenActionUnblocked(ctx, group) {
  const { state } = ctx;
  if (!state.blockedActions[group]) return Promise.resolve();
  return new Promise((resolve) => {
    if (!state.actionBlockWaiters.has(group)) state.actionBlockWaiters.set(group, []);
    state.actionBlockWaiters.get(group).push(resolve);
  });
}
