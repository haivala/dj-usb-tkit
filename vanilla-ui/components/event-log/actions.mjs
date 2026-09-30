// Event log UI rendering helpers.

import { cloneTemplate } from "../../ui_utils.mjs";

// Stores an already-normalized entry (see message_bus.mjs emitMessage). With
// the Event Log open, only that entry's row changes -- a busy job can log
// many entries a second, too many for a full re-render each.
export function storeEventLogEntry(ctx, entry = {}) {
  const { state, eventLogStore } = ctx;
  const pushed = eventLogStore.push(entry);
  if (!pushed) return;
  if (state.activeTab === "event-log") {
    showEventLogEntry(ctx, pushed);
  }
}

// Offer each of `sources` in the source filter (once), keeping the selection.
function ensureEventLogSourceOptions(ctx, sources) {
  const select = ctx.el.eventLogSourceFilter;
  if (!select) return;
  const current = String(select.value || "all");
  const known = new Set([...select.options].map((opt) => String(opt.value || "")));
  for (const src of [...new Set(sources)].sort()) {
    if (known.has(src)) continue;
    select.add(new select.ownerDocument.defaultView.Option(src, src));
  }
  select.value = [...select.options].some((opt) => opt.value === current) ? current : "all";
}

// The entries the level/source filters let through, oldest first.
function filteredEventLogEntries(ctx) {
  const { el } = ctx;
  const levelFilter = String(el.eventLogLevelFilter?.value || "all");
  const sourceFilter = String(el.eventLogSourceFilter?.value || "all");
  return ctx.eventLogStore.list().filter((item) => {
    const levelMatch = levelFilter === "all" || item.level === levelFilter;
    const sourceMatch = sourceFilter === "all" || item.source === sourceFilter;
    return levelMatch && sourceMatch;
  });
}

function renderEventLogSummary(ctx, entries) {
  const totalOccurrences = entries.reduce((sum, item) => sum + Math.max(1, Number(item.count) || 1), 0);
  ctx.el.eventLogSummary.textContent = totalOccurrences === entries.length
    ? `${entries.length} event(s)`
    : `${entries.length} event(s) (${totalOccurrences} occurrences)`;
}

function eventLogRow(doc, item) {
  const date = new Date(item.ts);
  const hh = String(date.getHours()).padStart(2, "0");
  const mm = String(date.getMinutes()).padStart(2, "0");
  const ss = String(date.getSeconds()).padStart(2, "0");
  const rawCode = String(item.code || "unknown");
  const sourceCodePrefix = String(item.source || "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, ".")
    .replace(/^\.+|\.+$/g, "");
  const collapsedCode = sourceCodePrefix && rawCode.startsWith(`${sourceCodePrefix}.`)
    ? rawCode.slice(sourceCodePrefix.length + 1)
    : rawCode;
  const count = Math.max(1, Number(item.count) || 1);
  const details = String(item.details || "").trim();

  const row = cloneTemplate(doc, "tplEventLogRow");
  row.dataset.entryId = String(item.id);
  row.querySelector(".event-log-time").textContent = `${hh}:${mm}:${ss}`;
  const level = row.querySelector(".event-log-level");
  level.classList.add(`level-${item.level}`);
  level.textContent = item.level;
  row.querySelector(".event-log-source").textContent = item.source;
  const message = row.querySelector(".event-log-message");
  if (details) message.dataset.tooltip = details;
  row.querySelector(".event-log-code").textContent = `[${collapsedCode || "unknown"}]`;
  row.querySelector(".event-log-text").textContent = item.message;
  const countBadge = row.querySelector(".event-log-count");
  if (count > 1) countBadge.textContent = `x${count}`;
  else countBadge.remove();
  return row;
}

export function renderEventLog(ctx) {
  const { el } = ctx;
  if (!el.eventLogList || !el.eventLogSummary) return;
  ensureEventLogSourceOptions(ctx, ctx.eventLogStore.list().map((item) => item.source));
  const entries = filteredEventLogEntries(ctx);
  renderEventLogSummary(ctx, entries);
  const doc = el.eventLogList.ownerDocument;
  if (!entries.length) {
    const row = cloneTemplate(doc, "tplLogMessageRow");
    row.firstElementChild.textContent = "No events";
    el.eventLogList.replaceChildren(row);
    return;
  }
  // Newest first.
  el.eventLogList.replaceChildren(...entries.reverse().map((item) => eventLogRow(doc, item)));
}

// Put the just-stored `item` (new, or a repeat moved up with its new count)
// at the top of the open list, without touching the other rows.
function showEventLogEntry(ctx, item) {
  const { el } = ctx;
  const list = el.eventLogList;
  if (!list || !el.eventLogSummary) return;
  ensureEventLogSourceOptions(ctx, [item.source]);
  const entries = filteredEventLogEntries(ctx);
  renderEventLogSummary(ctx, entries);
  list.querySelector(`.event-log-row[data-entry-id="${item.id}"]`)?.remove();
  if (!entries.includes(item)) return;
  list.querySelector(".event-log-row:not([data-entry-id])")?.remove(); // "No events"
  list.prepend(eventLogRow(list.ownerDocument, item));
  // The store drops its oldest entries past its cap; they're the bottom rows.
  while (list.children.length > entries.length) list.lastElementChild.remove();
}

// Console and runtime error logging setup.

export async function setupConsoleFileLogging({ isTauriRuntime, invoke, pushEventLog }) {
  const original = {
    log: console.log.bind(console),
    info: console.info.bind(console),
    warn: console.warn.bind(console),
    error: console.error.bind(console)
  };
  const canForwardToFile = isTauriRuntime();

  if (canForwardToFile) {
    try {
      await invoke("clear_frontend_log");
    } catch (_) {
      // keep console interception even when file logging fails
    }
  }

  const forward = (level, args) => {
    const message = args.map((v) => {
      if (typeof v === "string") return v;
      try {
        return JSON.stringify(v);
      } catch {
        return String(v);
      }
    }).join(" ");
    pushEventLog({ level, source: "console", message });
    if (canForwardToFile) invoke("append_frontend_log", { level, message }).catch(() => {});
  };

  console.log = (...args) => {
    original.log(...args);
    forward("log", args);
  };
  console.info = (...args) => {
    original.info(...args);
    forward("info", args);
  };
  console.warn = (...args) => {
    original.warn(...args);
    forward("warn", args);
  };
  console.error = (...args) => {
    original.error(...args);
    forward("error", args);
  };
}

export function setupRuntimeErrorLogging({ pushEventLog, window }) {
  window.addEventListener("securitypolicyviolation", (event) => {
    const directive = String(event?.violatedDirective || "unknown");
    const blocked = String(event?.blockedURI || "").trim();
    const message = blocked
      ? `CSP violation: ${directive} (blocked: ${blocked})`
      : `CSP violation: ${directive}`;
    pushEventLog({
      level: "error",
      source: "browser",
      message
    });
  });

  window.addEventListener("error", (event) => {
    const message = String(event?.message || "Unhandled window error");
    pushEventLog({
      level: "error",
      source: "browser",
      message
    });
  });

  window.addEventListener("unhandledrejection", (event) => {
    const reason = event?.reason;
    const message = typeof reason === "string"
      ? reason
      : String(reason?.message || "Unhandled promise rejection");
    pushEventLog({
      level: "error",
      source: "browser",
      message
    });
  });
}

// --- warning_utils.mjs ---

// Backend warnings are always typed `WarningEntry { level, code, message, source }`
// (built through `logging::log` in Rust -- see backend/src/logging.rs). The
// frontend reads `level` verbatim and never guesses severity from the message
// text.
export function warningEntryLevel(entry) {
  const level = String(entry?.level || "").toLowerCase().trim();
  return level === "error" || level === "warn" || level === "info" ? level : "info";
}

export function countWarningsForStatus(warnings) {
  const list = Array.isArray(warnings) ? warnings : [];
  return list.filter((entry) => {
    const level = warningEntryLevel(entry);
    return level === "warn" || level === "error";
  }).length;
}

export function logWarnings(ctx, source, warnings, context = "") {
  const list = Array.isArray(warnings) ? warnings : [];
  if (!list.length) return;
  for (const warning of list) {
    const text = String(warning?.message ?? warning ?? "").trim();
    if (!text) continue;
    const level = warningEntryLevel(warning);
    const warningSource = String(warning?.source || source || "ui").trim() || "ui";
    const code = String(warning?.code || "").trim() || `${warningSource}.event`;
    const detailParts = [];
    if (context) detailParts.push(`context: ${context}`);
    if (typeof warning?.details === "string" && warning.details.trim()) {
      detailParts.push(warning.details.trim());
    }
    const detailsJoined = detailParts.length ? detailParts.join(" | ") : null;
    const coalesceKeyParts = [
      warningSource.toLowerCase(),
      code.toLowerCase(),
      text.toLowerCase(),
      String(detailsJoined || "").toLowerCase()
    ];
    ctx.pushEventLog({
      level,
      source: warningSource,
      code,
      message: text,
      details: detailsJoined,
      coalesceKey: coalesceKeyParts.join("|")
    });
  }
}
