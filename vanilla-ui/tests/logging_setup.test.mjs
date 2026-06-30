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

test("setupConsoleFileLogging intercepts console methods and pushes to event log", async () => {
  const originalLog = console.log;
  const originalInfo = console.info;
  const originalWarn = console.warn;
  const originalError = console.error;
  const logged = [];
  const invoked = [];

  try {
    await setupConsoleFileLogging({
      isTauriRuntime: () => false,
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
    // Non-tauri: no file forwarding
    assert.equal(invoked.length, 0);
  } finally {
    console.log = originalLog;
    console.info = originalInfo;
    console.warn = originalWarn;
    console.error = originalError;
  }
});

test("setupConsoleFileLogging forwards to file in tauri mode", async () => {
  const originalLog = console.log;
  const originalInfo = console.info;
  const originalWarn = console.warn;
  const originalError = console.error;
  const invoked = [];

  try {
    await setupConsoleFileLogging({
      isTauriRuntime: () => true,
      invoke: (cmd, payload) => { invoked.push({ cmd, payload }); return Promise.resolve(); },
      pushEventLog: () => {}
    });

    console.info("test message");

    // clear_frontend_log + append_frontend_log
    const appendCalls = invoked.filter((c) => c.cmd === "append_frontend_log");
    assert.ok(appendCalls.length >= 1);
    assert.equal(appendCalls[0].payload.level, "info");
    assert.equal(appendCalls[0].payload.message, "test message");
  } finally {
    console.log = originalLog;
    console.info = originalInfo;
    console.warn = originalWarn;
    console.error = originalError;
  }
});

test("setupConsoleFileLogging serializes non-string args", async () => {
  const originalLog = console.log;
  const originalInfo = console.info;
  const originalWarn = console.warn;
  const originalError = console.error;
  const logged = [];

  try {
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
  } finally {
    console.log = originalLog;
    console.info = originalInfo;
    console.warn = originalWarn;
    console.error = originalError;
  }
});

test("setupRuntimeErrorLogging captures CSP violations", () => {
  const logged = [];
  setupRuntimeErrorLogging({ pushEventLog: (entry) => { logged.push(entry); } });

  window._fire("securitypolicyviolation", {
    violatedDirective: "img-src",
    blockedURI: "https://evil.com/img.png"
  });

  const csp = logged.find((e) => e.message.includes("CSP violation"));
  assert.ok(csp);
  assert.equal(csp.level, "error");
  assert.equal(csp.source, "browser");
  assert.ok(csp.message.includes("img-src"));
  assert.ok(csp.message.includes("evil.com"));
});

test("setupRuntimeErrorLogging captures unhandled errors", () => {
  const logged = [];
  setupRuntimeErrorLogging({ pushEventLog: (entry) => { logged.push(entry); } });

  window._fire("error", { message: "TypeError: x is not a function" });

  const err = logged.find((e) => e.message.includes("TypeError"));
  assert.ok(err);
  assert.equal(err.level, "error");
});

test("setupRuntimeErrorLogging captures unhandled rejections", () => {
  const logged = [];
  setupRuntimeErrorLogging({ pushEventLog: (entry) => { logged.push(entry); } });

  window._fire("unhandledrejection", { reason: new Error("async failure") });

  const rejection = logged.find((e) => e.message.includes("async failure"));
  assert.ok(rejection);
  assert.equal(rejection.level, "error");
});
