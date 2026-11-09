import test from "node:test";
import assert from "node:assert/strict";

if (typeof globalThis.window === "undefined") {
  const listeners = {};
  globalThis.window = {
    addEventListener(event, handler) {
      if (!listeners[event]) listeners[event] = [];
      listeners[event].push(handler);
    },
    _fire(event, payload) {
      for (const fn of listeners[event] || []) fn(payload);
    }
  };
}

import { setupConsoleFileLogging, setupRuntimeErrorLogging } from "../components/event-log/actions.mjs";
import { withSilencedConsole } from "./test_helpers.mjs";

test("setupConsoleFileLogging sends warnings and errors to the Event Log, log and info lines to the file only", async () => {
  await withSilencedConsole(async () => {
    const logged = [];
    const fileOnly = [];
    const invoked = [];

    await setupConsoleFileLogging({
      isTauriRuntime: () => true,
      invoke: (cmd, payload) => { invoked.push({ cmd, payload }); },
      pushEventLog: (entry) => { logged.push(entry); },
      logToFile: (entry) => { fileOnly.push(entry); }
    });

    console.log("hello");
    console.info("detail");
    console.warn("danger");
    console.error("bad");

    assert.deepEqual(fileOnly.map((e) => [e.level, e.message, e.source]), [["log", "hello", "console"], ["info", "detail", "console"]]);
    assert.deepEqual(logged.map((e) => [e.level, e.message, e.source]), [["warn", "danger", "console"], ["error", "bad", "console"]]);
    // The Event Log's entries reach the file through storeEventLogEntry, not the console bridge.
    assert.equal(invoked.length, 0);
  });
});

test("setupConsoleFileLogging serializes non-string args", async () => {
  await withSilencedConsole(async () => {
    const logged = [];

    await setupConsoleFileLogging({
      isTauriRuntime: () => false,
      invoke: () => {},
      pushEventLog: (entry) => { logged.push(entry); },
      logToFile: () => {}
    });

    console.warn("count:", 42, { key: "val" });

    assert.equal(logged.length, 1);
    assert.ok(logged[0].message.includes("count:"));
    assert.ok(logged[0].message.includes("42"));
    assert.ok(logged[0].message.includes('"key"'));
  });
});

test("setupRuntimeErrorLogging captures unhandled errors", () => {
  const logged = [];
  setupRuntimeErrorLogging({ window: globalThis.window, pushEventLog: (entry) => { logged.push(entry); } });

  window._fire("error", { message: "TypeError: x is not a function" });

  const err = logged.find((e) => e.message.includes("TypeError"));
  assert.ok(err);
  assert.equal(err.level, "error");
});

test("setupRuntimeErrorLogging captures unhandled rejections", () => {
  const logged = [];
  setupRuntimeErrorLogging({ window: globalThis.window, pushEventLog: (entry) => { logged.push(entry); } });

  window._fire("unhandledrejection", { reason: new Error("async failure") });

  const rejection = logged.find((e) => e.message.includes("async failure"));
  assert.ok(rejection);
  assert.equal(rejection.level, "error");
});
