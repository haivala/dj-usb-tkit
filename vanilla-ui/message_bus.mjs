import { cloneTemplate } from "./ui_utils.mjs";

function normalizeText(value) {
  return String(value ?? "").trim();
}

function normalizeLevel(value) {
  const raw = String(value || "").toLowerCase().trim();
  if (raw === "error") return "error";
  if (raw === "warn" || raw === "warning") return "warn";
  return "info";
}

// A warning or error should never be visible only in the transient status bar — this
// decides whether a status update should also persist to the Event Log. Startup-phase
// status is always mirrored too (coalesced, for reviewing a failed startup afterward).
export function shouldPersistStatusToEventLog(level, isStartupPhase) {
  if (isStartupPhase) return true;
  return level === "warn" || level === "error";
}

function normalizeSource(value) {
  const raw = String(value || "").trim();
  return raw || "ui";
}

function normalizeCode(value) {
  const raw = String(value || "").trim();
  return raw || null;
}

function normalizeTs(value) {
  const ts = Number(value);
  return Number.isFinite(ts) && ts > 0 ? ts : Date.now();
}

function normalizeProgress(progress) {
  if (!progress || typeof progress !== "object") return null;
  const text = normalizeText(progress.text);
  if (!text) return null;
  const percentRaw = Number(progress.percent);
  const percent = Number.isFinite(percentRaw)
    ? Math.max(0, Math.min(100, percentRaw))
    : null;
  return { text, percent };
}

function normalizeStatus(status) {
  if (!status || typeof status !== "object") return null;
  const text = normalizeText(status.text);
  if (!text) return null;
  const warningCountRaw = Number(status.warningCount);
  const warningCount = Number.isFinite(warningCountRaw) ? Math.max(0, warningCountRaw) : 0;
  return { text, warningCount };
}

function normalizeEventLog(eventLog) {
  if (!eventLog || typeof eventLog !== "object") return null;
  const text = normalizeText(eventLog.text);
  if (!text) return null;
  const detailsText = normalizeText(eventLog.details);
  const coalesceKey = normalizeText(eventLog.coalesceKey);
  return {
    text,
    details: detailsText || null,
    coalesceKey: coalesceKey || null
  };
}

export function normalizeUiMessage(input = {}) {
  if (!input || typeof input !== "object") return null;
  const level = normalizeLevel(input.level);
  const source = normalizeSource(input.source);
  const code = normalizeCode(input.code);
  const ts = normalizeTs(input.ts);
  const progress = normalizeProgress(input.progress);
  const status = normalizeStatus(input.status);
  const eventLog = normalizeEventLog(input.eventLog);
  if (!progress && !status && !eventLog) return null;
  return {
    level,
    source,
    code,
    ts,
    progress,
    status,
    eventLog
  };
}

export function setStatusText(el, text, warningCount = 0) {
  const target = el.statusText;
  const doc = target.ownerDocument;
  target.textContent = "";
  const str = String(text ?? "");
  const n = Math.max(0, Number(warningCount) || 0);
  const pipeIdx = n > 0 ? str.indexOf("|") : -1;
  if (pipeIdx === -1) {
    target.append(doc.createTextNode(str));
    return;
  }
  const trailing = str.slice(pipeIdx + 1);
  const leadingSpace = trailing.match(/^\s*/)[0];
  target.append(doc.createTextNode(str.slice(0, pipeIdx + 1) + leadingSpace));
  const link = cloneTemplate(doc, "tplStatusWarningLink");
  link.textContent = trailing.slice(leadingSpace.length);
  target.append(link);
}

// The one sink every UI message goes through: status bar, progress footer
// and the event log.
export function emitMessage(ctx, input = {}) {
  const message = normalizeUiMessage(input);
  if (!message) return null;

  if (message.progress) {
    const percent = Number(message.progress.percent);
    ctx.setProgress(true, Number.isFinite(percent) ? percent : ctx.state.progressPercent, message.progress.text);
  }

  if (message.status) {
    setStatusText(ctx.el, message.status.text, message.status.warningCount);
  }

  if (message.eventLog) {
    ctx.storeEventLogEntry({
      level: message.level,
      source: message.source,
      code: message.code,
      message: message.eventLog.text,
      details: message.eventLog.details,
      coalesceKey: message.eventLog.coalesceKey,
      ts: message.ts
    });
  }

  return message;
}

export function setStatus(ctx, text, meta = {}) {
  const statusText = String(text || "");
  const level = meta.level || "info";
  const startupPhase = ctx.state.startupPhase;
  const eventLog = shouldPersistStatusToEventLog(level, startupPhase)
    ? {
      text: statusText,
      details: meta.details ?? null,
      coalesceKey: meta.coalesceKey ?? (startupPhase ? "startup.status" : null)
    }
    : null;
  emitMessage(ctx, {
    level,
    source: meta.source || "ui",
    code: meta.code || null,
    status: { text: statusText, warningCount: meta.warningCount || 0 },
    eventLog,
  });
}

export function pushEventLog(ctx, entry = {}) {
  const text = String(entry.message ?? entry.text ?? "").trim();
  if (!text) return null;
  return emitMessage(ctx, {
    level: entry.level,
    source: entry.source,
    code: entry.code,
    ts: entry.ts,
    eventLog: {
      text,
      details: entry.details ?? null,
      coalesceKey: entry.coalesceKey ?? null
    }
  });
}
