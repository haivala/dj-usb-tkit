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

test("setupConsoleFileLogging intercepts console methods and pushes to event log", async () => {
  await withSilencedConsole(async () => {
    const logged = [];
    const invoked = [];

    await setupConsoleFileLogging({
      isTauriRuntime: () => true,
      invoke: (cmd, payload) => { invoked.push({ cmd, payload }); },
      pushEventLog: (entry) => { logged.push(entry); }
    });

    console.log("hello");
    console.warn("danger");
    console.error("bad");

    assert.equal(logged.length, 3);
    assert.equal(logged[0].level, "log");
    assert.equal(logged[0].message, "hello");
    assert.equal(logged[0].source, "console");
    assert.equal(logged[1].level, "warn");
    assert.equal(logged[2].level, "error");
    // The file is written by storeEventLogEntry, not the console bridge.
    assert.equal(invoked.length, 0);
  });
});

test("setupConsoleFileLogging serializes non-string args", async () => {
  await withSilencedConsole(async () => {
    const logged = [];

    await setupConsoleFileLogging({
      isTauriRuntime: () => false,
      invoke: () => {},
      pushEventLog: (entry) => { logged.push(entry); }
    });

    console.log("count:", 42, { key: "val" });

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
