// Event log UI rendering helpers.

export function pushEventLog(state, eventLogStore, renderEventLog, entry = {}) {
  const pushed = eventLogStore.push(entry);
  if (!pushed) return;
  state.eventLogEntries = eventLogStore.list();
  if (state.activeTab === "event-log") {
    renderEventLog();
  }
}

export function ensureEventLogSourceOptions(state, el, document) {
  if (!el.eventLogSourceFilter) return;
  const current = String(el.eventLogSourceFilter.value || "all");
  const known = new Set(["all"]);
  for (const opt of el.eventLogSourceFilter.options) {
    known.add(String(opt.value || ""));
  }
  const sources = Array.from(new Set(state.eventLogEntries.map((x) => x.source))).sort();
  for (const src of sources) {
    if (known.has(src)) continue;
    const opt = document.createElement("option");
    opt.value = src;
    opt.textContent = src;
    el.eventLogSourceFilter.appendChild(opt);
  }
  if ([...el.eventLogSourceFilter.options].some((opt) => opt.value === current)) {
    el.eventLogSourceFilter.value = current;
  } else {
    el.eventLogSourceFilter.value = "all";
  }
}

export function renderEventLog(state, el, document, deps) {
  const { ensureEventLogSourceOptions, escapeHtml } = deps;
  if (!el.eventLogList || !el.eventLogSummary) return;
  ensureEventLogSourceOptions();
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
  if (!rows.length) {
    el.eventLogList.innerHTML = `<div class="event-log-row"><div class="event-log-message muted">No events</div></div>`;
    return;
  }
  el.eventLogList.innerHTML = rows.map((item) => {
    const date = new Date(item.ts);
    const hh = String(date.getHours()).padStart(2, "0");
    const mm = String(date.getMinutes()).padStart(2, "0");
    const ss = String(date.getSeconds()).padStart(2, "0");
    const level = escapeHtml(item.level);
    const source = escapeHtml(item.source);
    const message = escapeHtml(item.message);
    const rawCode = String(item.code || "unknown");
    const sourceCodePrefix = String(item.source || "")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, ".")
      .replace(/^\.+|\.+$/g, "");
    const collapsedCode = sourceCodePrefix && rawCode.startsWith(`${sourceCodePrefix}.`)
      ? rawCode.slice(sourceCodePrefix.length + 1)
      : rawCode;
    const code = escapeHtml(collapsedCode || "unknown");
    const count = Math.max(1, Number(item.count) || 1);
    const details = String(item.details || "").trim();
    const detailsAttr = details ? ` title="${escapeHtml(details)}"` : "";
    const countBadge = count > 1 ? `<span class="event-log-count" title="Coalesced occurrences">x${count}</span>` : "";
    return `<div class="event-log-row"><div class="event-log-time">${hh}:${mm}:${ss}</div><div class="event-log-level level-${level}">${level}</div><div class="event-log-source">${source}</div><div class="event-log-message"${detailsAttr}><span class="event-log-code">[${code}]</span> ${message} ${countBadge}</div></div>`;
  }).join("");
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

export function setupRuntimeErrorLogging({ pushEventLog }) {
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

export function classifyLogLevelFromText(text) {
  const t = String(text || "").toLowerCase();
  const errorHints = [
    "failed",
    "error",
    "timeout",
    "timed out",
    "permission denied",
    "denied",
    "unreadable",
    "corrupt",
    "invalid key",
  ];
  const warnHints = [
    "missing ",
    "not found",
    "appears empty",
    "zero static track entries",
    "skipped",
  ];
  if (errorHints.some((hint) => t.includes(hint))) return "error";
  if (warnHints.some((hint) => t.includes(hint))) return "warn";
  return "info";
}

export function warningEntryLevel(entry) {
  if (entry && typeof entry === "object") {
    const level = String(entry.level || "").toLowerCase().trim();
    if (level === "error" || level === "warn" || level === "info") return level;
  }
  return classifyLogLevelFromText(String(entry || "").trim());
}

export function countWarningsForStatus(warnings) {
  const list = Array.isArray(warnings) ? warnings : [];
  return list.filter((entry) => {
    const level = warningEntryLevel(entry);
    return level === "warn" || level === "error";
  }).length;
}

export function logWarnings(pushEventLog, source, warnings, context = "") {
  const list = Array.isArray(warnings) ? warnings : [];
  if (!list.length) return;
  for (const warning of list) {
    const isTyped = warning && typeof warning === "object";
    const text = isTyped
      ? String(warning.message || "").trim()
      : String(warning || "").trim();
    if (!text) continue;
    const levelRaw = isTyped ? String(warning.level || "").toLowerCase().trim() : "";
    const level = levelRaw === "error" || levelRaw === "warn" || levelRaw === "info"
      ? levelRaw
      : "info";
    const warningSource = isTyped
      ? String(warning.source || source || "ui").trim() || "ui"
      : source;
    const code = isTyped
      ? String(warning.code || "").trim()
      : `${String(source || "ui").toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "") || "ui"}.event`;
    const detailParts = [];
    if (context) detailParts.push(`context: ${context}`);
    if (isTyped && typeof warning.details === "string" && warning.details.trim()) {
      detailParts.push(warning.details.trim());
    }
    const detailsJoined = detailParts.length ? detailParts.join(" | ") : null;
    const coalesceKeyParts = [
      String(warningSource || "ui").trim().toLowerCase(),
      String(code || "").trim().toLowerCase() || "event",
      String(text || "").trim().toLowerCase(),
      String(detailsJoined || "").trim().toLowerCase()
    ];
    pushEventLog({
      level,
      source: warningSource,
      code,
      message: text,
      details: detailsJoined,
      coalesceKey: coalesceKeyParts.join("|")
    });
  }
}
