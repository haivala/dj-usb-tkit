import test from "node:test";
import assert from "node:assert/strict";

import { createMessageBus, normalizeUiMessage } from "../message_bus.mjs";

test("normalizeUiMessage returns canonical preformatted payload", () => {
  const msg = normalizeUiMessage({
    level: "WARNING",
    source: " usb ",
    code: "usb.import",
    status: { text: "  USB playlists loaded  " },
    eventLog: { text: "  USB playlists loaded  ", details: "  x  " },
    ts: 42
  });
  assert.equal(msg.level, "warn");
  assert.equal(msg.source, "usb");
  assert.equal(msg.code, "usb.import");
  assert.equal(msg.status.text, "USB playlists loaded");
  assert.equal(msg.eventLog.text, "USB playlists loaded");
  assert.equal(msg.eventLog.details, "x");
  assert.equal(msg.ts, 42);
});

test("message bus routes preformatted text without consumer-side rewriting", () => {
  let statusText = "";
  const entries = [];
  const bus = createMessageBus({
    setStatusText: (text) => { statusText = text; },
    pushEventLog: (entry) => entries.push(entry)
  });

  bus.emitMessage({
    level: "info",
    source: "library",
    status: { text: "scan:stage - exact text" },
    eventLog: {
      text: "scan:stage - exact text",
      details: "ctx: test",
      coalesceKey: "library.scan.status"
    }
  });

  assert.equal(statusText, "scan:stage - exact text");
  assert.equal(entries.length, 1);
  assert.equal(entries[0].message, "scan:stage - exact text");
  assert.equal(entries[0].details, "ctx: test");
  assert.equal(entries[0].coalesceKey, "library.scan.status");
});

