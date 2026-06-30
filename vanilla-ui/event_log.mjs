function normalizeText(value) {
  return String(value || "").replace(/\s+/g, " ").trim();
}

function normalizeToken(value) {
  return String(value || "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
}

function normalizeSource(value) {
  return String(value || "")
    .toLowerCase()
    .replace(/\s+/g, "-")
    .replace(/[^a-z0-9._-]+/g, "")
    .replace(/^-+|-+$/g, "");
}

function normalizeLevel(level, message = "") {
  const raw = String(level || "").toLowerCase().trim();
  if (raw === "warn" || raw === "warning") return "warn";
  if (raw === "error") return "error";
  if (raw === "info" || raw === "log" || raw === "debug") return "info";
  return "info";
}

function normalizeCode(source, code) {
  const normalizedSource = normalizeToken(source) || "ui";
  const rawCode = String(code || "").toLowerCase().trim();
  if (/^[a-z0-9]+(\.[a-z0-9_]+)+$/.test(rawCode)) {
    return rawCode;
  }
  return `${normalizedSource}.event`;
}

function normalizeSignatureText(value) {
  return normalizeText(value).toLowerCase();
}

export function normalizeEventLogEntry(input = {}) {
  const message = normalizeText(input.message);
  if (!message) return null;
  const source = normalizeSource(input.source) || "ui";
  const level = normalizeLevel(input.level, message);
  const code = normalizeCode(source, input.code);
  const details = normalizeText(input.details);
  const ts = Number(input.ts) > 0 ? Number(input.ts) : Date.now();
  const signature = `${normalizeSignatureText(message)}|${normalizeSignatureText(details)}`;
  const explicitCoalesceKey = normalizeText(input.coalesceKey);
  return {
    source,
    level,
    code,
    message,
    details: details || null,
    ts,
    signature,
    coalesceKey: explicitCoalesceKey || null
  };
}

export function buildEventLogCoalesceKey(entry) {
  if (entry?.coalesceKey) {
    return String(entry.coalesceKey);
  }
  return `${entry.level}|${entry.source}|${entry.code}|${entry.signature}`;
}

export function createEventLogStore({ maxEntries = 1000 } = {}) {
  let seq = 0;
  const entries = [];
  const byKey = new Map();

  const removeOldest = () => {
    while (entries.length > maxEntries) {
      const removed = entries.shift();
      if (!removed) break;
      const tracked = byKey.get(removed.coalesceKey);
      if (tracked && tracked.id === removed.id) {
        byKey.delete(removed.coalesceKey);
      }
    }
  };

  return {
    push(raw) {
      const normalized = normalizeEventLogEntry(raw);
      if (!normalized) return null;
      const coalesceKey = buildEventLogCoalesceKey(normalized);
      const existing = byKey.get(coalesceKey);
      if (existing) {
        existing.count += 1;
        existing.lastTs = normalized.ts;
        if (normalized.details) existing.details = normalized.details;
        const idx = entries.findIndex((item) => item.id === existing.id);
        if (idx >= 0) entries.splice(idx, 1);
        entries.push(existing);
        removeOldest();
        return existing;
      }

      seq += 1;
      const entry = {
        id: seq,
        ts: normalized.ts,
        firstTs: normalized.ts,
        lastTs: normalized.ts,
        level: normalized.level,
        source: normalized.source,
        code: normalized.code,
        message: normalized.message,
        details: normalized.details,
        count: 1,
        coalesceKey
      };
      entries.push(entry);
      byKey.set(coalesceKey, entry);
      removeOldest();
      return entry;
    },

    clear() {
      entries.length = 0;
      byKey.clear();
    },

    list() {
      return entries.slice();
    }
  };
}
