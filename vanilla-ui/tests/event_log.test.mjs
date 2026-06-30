import test from "node:test";
import assert from "node:assert/strict";

import {
  buildEventLogCoalesceKey,
  createEventLogStore,
  normalizeEventLogEntry
} from "../event_log.mjs";

test("normalizeEventLogEntry normalizes level/source/code", () => {
  const entry = normalizeEventLogEntry({
    level: "WARNING",
    source: "USB-Diagnostics",
    code: "USB.DIAGNOSTICS.EMPTY_ANALYSIS",
    message: "  analysis file appears empty  "
  });
  assert.equal(entry.level, "warn");
  assert.equal(entry.source, "usb-diagnostics");
  assert.equal(entry.code, "usb.diagnostics.empty_analysis");
  assert.equal(entry.message, "analysis file appears empty");
});

test("normalizeEventLogEntry defaults code when missing", () => {
  const entry = normalizeEventLogEntry({
    source: "export",
    message: "missing source file: /tmp/x.mp3"
  });
  assert.equal(entry.code, "export.event");
  assert.equal(entry.level, "info");
});

test("buildEventLogCoalesceKey is stable for normalized duplicates", () => {
  const a = normalizeEventLogEntry({
    source: "export",
    message: "missing source file: /tmp/x.mp3"
  });
  const b = normalizeEventLogEntry({
    source: "EXPORT",
    message: "  missing   source file: /tmp/x.mp3  "
  });
  assert.equal(buildEventLogCoalesceKey(a), buildEventLogCoalesceKey(b));
});

test("createEventLogStore coalesces repeated warnings and tracks count/timestamps", () => {
  const store = createEventLogStore({ maxEntries: 1000 });
  store.push({
    source: "export",
    message: "missing source file: /tmp/x.mp3",
    ts: 10
  });
  store.push({
    source: "export",
    message: "missing source file: /tmp/x.mp3",
    ts: 99
  });
  const items = store.list();
  assert.equal(items.length, 1);
  assert.equal(items[0].count, 2);
  assert.equal(items[0].firstTs, 10);
  assert.equal(items[0].lastTs, 99);
});

test("createEventLogStore does not coalesce different messages when details are identical", () => {
  const store = createEventLogStore({ maxEntries: 1000 });
  store.push({
    source: "usb-diagnostics",
    code: "usb.diagnostics.unindexed-audio",
    level: "warn",
    message: "unindexed audio file: /Contents/A/one.mp3",
    details: "context: repair_usb_diagnostics preview"
  });
  store.push({
    source: "usb-diagnostics",
    code: "usb.diagnostics.unindexed-audio",
    level: "warn",
    message: "unindexed audio file: /Contents/B/two.mp3",
    details: "context: repair_usb_diagnostics preview"
  });
  const items = store.list();
  assert.equal(items.length, 2);
});

test("createEventLogStore keeps newest entries within max cap", () => {
  const store = createEventLogStore({ maxEntries: 3 });
  store.push({ source: "ui", message: "a", ts: 1 });
  store.push({ source: "ui", message: "b", ts: 2 });
  store.push({ source: "ui", message: "c", ts: 3 });
  store.push({ source: "ui", message: "d", ts: 4 });
  const items = store.list();
  assert.deepEqual(items.map((x) => x.message), ["b", "c", "d"]);
});

test("createEventLogStore clear removes all entries", () => {
  const store = createEventLogStore();
  store.push({ source: "ui", message: "hello" });
  assert.equal(store.list().length, 1);
  store.clear();
  assert.equal(store.list().length, 0);
});

test("createEventLogStore uses explicit coalesce key when provided", () => {
  const store = createEventLogStore({ maxEntries: 1000 });
  store.push({
    source: "analysis",
    code: "analysis.track_failed",
    level: "error",
    message: "Track analysis failed: A",
    coalesceKey: "analysis.track_failed.1",
  });
  store.push({
    source: "analysis",
    code: "analysis.track_failed",
    level: "error",
    message: "Track analysis failed: B",
    coalesceKey: "analysis.track_failed.1",
  });
  assert.equal(store.list().length, 1);
  assert.equal(store.list()[0].count, 2);
});
