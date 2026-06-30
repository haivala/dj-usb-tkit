function normalizeText(value) {
  return String(value ?? "").trim();
}

function normalizeLevel(value) {
  const raw = String(value || "").toLowerCase().trim();
  if (raw === "error") return "error";
  if (raw === "warn" || raw === "warning") return "warn";
  return "info";
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
  return { text };
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

export function createMessageBus(deps = {}) {
  const {
    setStatusText = () => {},
    setProgressText = () => {},
    pushEventLog = () => {}
  } = deps;

  function emitMessage(input = {}) {
    const message = normalizeUiMessage(input);
    if (!message) return null;

    if (message.progress) {
      setProgressText(message.progress);
    }

    if (message.status) {
      setStatusText(message.status.text);
    }

    if (message.eventLog) {
      pushEventLog({
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

  function emitStatus(text, meta = {}) {
    return emitMessage({
      ...meta,
      status: { text }
    });
  }

  function emitEventLog(text, meta = {}) {
    return emitMessage({
      ...meta,
      eventLog: {
        text,
        details: meta.details ?? null,
        coalesceKey: meta.coalesceKey ?? null
      }
    });
  }

  return {
    emitMessage,
    emitStatus,
    emitEventLog
  };
}
