// Event log UI rendering helpers.

import { cloneTemplate } from "../../ui_utils.mjs";

// Stores an already-normalized entry (see message_bus.mjs emitMessage).
export function storeEventLogEntry(ctx, entry = {}) {
  const { state, eventLogStore } = ctx;
  const pushed = eventLogStore.push(entry);
  if (!pushed) return;
  state.eventLogEntries = eventLogStore.list();
  if (state.activeTab === "event-log") {
    renderEventLog(ctx);
  }
}

function ensureEventLogSourceOptions(ctx) {
  const { state, el } = ctx;
  if (!el.eventLogSourceFilter) return;
  const current = String(el.eventLogSourceFilter.value || "all");
  const known = new Set(["all"]);
  for (const opt of el.eventLogSourceFilter.options) {
    known.add(String(opt.value || ""));
  }
  const sources = Array.from(new Set(state.eventLogEntries.map((x) => x.source))).sort();
  for (const src of sources) {
    if (known.has(src)) continue;
    el.eventLogSourceFilter.add(new el.eventLogSourceFilter.ownerDocument.defaultView.Option(src, src));
  }
  if ([...el.eventLogSourceFilter.options].some((opt) => opt.value === current)) {
    el.eventLogSourceFilter.value = current;
  } else {
    el.eventLogSourceFilter.value = "all";
  }
}

export function renderEventLog(ctx) {
  const { state, el } = ctx;
  if (!el.eventLogList || !el.eventLogSummary) return;
  ensureEventLogSourceOptions(ctx);
  const levelFilter = String(el.eventLogLevelFilter?.value || "all");
  const sourceFilter = String(el.eventLogSourceFilter?.value || "all");
  const filtered = state.eventLogEntries.filter((item) => {
    const levelMatch = levelFilter === "all" || item.level === levelFilter;
    const sourceMatch = sourceFilter === "all" || item.source === sourceFilter;
    return levelMatch && sourceMatch;
  });
  const rows = filtered.slice().reverse();
  const totalOccurrences = rows.reduce((sum, item) => sum + Math.max(1, Number(item.count) || 1), 0);
  el.eventLogSummary.textContent = totalOccurrences === rows.length
    ? `${rows.length} event(s)`
    : `${rows.length} event(s) (${totalOccurrences} occurrences)`;
  const doc = el.eventLogList.ownerDocument;
  if (!rows.length) {
    const row = cloneTemplate(doc, "tplLogMessageRow");
    row.firstElementChild.textContent = "No events";
    el.eventLogList.replaceChildren(row);
    return;
  }
  el.eventLogList.replaceChildren(...rows.map((item) => {
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
  }));
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
    pushEventLog({
      level,
      source: "console",
      message: args.map((v) => {
        if (typeof v === "string") return v;
        try {
          return JSON.stringify(v);
        } catch {
          return String(v);
        }
      }).join(" ")
    });
    if (!canForwardToFile) return;
    const message = args.map((v) => {
      if (typeof v === "string") return v;
      try {
        return JSON.stringify(v);
      } catch {
        return String(v);
      }
    }).join(" ");
    invoke("append_frontend_log", { level, message }).catch(() => {});
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
