// Shared helpers for the vanilla-ui node:test suite.

import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { createAppContext } from "../app_context.mjs";

const INDEX_HTML = readFileSync(new URL("../index.html", import.meta.url), "utf8");

// Every <template> from index.html, for tests that build their own small DOM
// around code which clones them.
export const APP_TEMPLATES = (INDEX_HTML.match(/<template id="[^"]+">[\s\S]*?<\/template>/g) || []).join("\n");

// Runs `fn` with console.log/info/warn/error replaced by no-ops, restoring
// the real methods afterward (even if `fn` throws). For tests that
// deliberately exercise a code path which logs to console on purpose (e.g.
// console-interception setup, or an error handler's `console.error(err)`),
// so the test run's own terminal output isn't spammed with expected,
// already-asserted-on output.
export async function withSilencedConsole(fn) {
  const original = {
    log: console.log,
    info: console.info,
    warn: console.warn,
    error: console.error
  };
  console.log = () => {};
  console.info = () => {};
  console.warn = () => {};
  console.error = () => {};
  try {
    return await fn();
  } finally {
    Object.assign(console, original);
  }
}

// The real app context (every module action bound, exactly as main.js builds
// it) on a jsdom copy of index.html, with a stubbed backend. `overrides`
// replaces ctx entries afterwards -- `command`, `state`, or any action a test
// wants to observe (only calls made *through ctx* see an overridden action).
export function makeTestCtx(overrides = {}) {
  const dom = new JSDOM(INDEX_HTML);
  const stored = new Map();
  const localStorage = {
    getItem: (key) => (stored.has(key) ? stored.get(key) : null),
    setItem: (key, value) => stored.set(key, String(value)),
    removeItem: (key) => stored.delete(key),
  };
  const ctx = createAppContext({
    document: dom.window.document,
    window: dom.window,
    navigator: {},
    localStorage,
    command: async () => ({}),
    invoke: async () => null,
    isTauriRuntime: () => false,
    getTauriEventListen: async () => null,
    tauriConvertFileSrc: null,
    tauriIsTauri: () => false,
    tauriGetVersion: async () => "0.0.0",
  });
  ctx.dom = dom;
  ctx.requestAnimationFrameFn = (cb) => setTimeout(cb, 0);
  ctx.cancelAnimationFrameFn = (handle) => clearTimeout(handle);
  ctx.nextPaint = async () => {};
  ctx.log = () => {};
  ctx.warn = () => {};
  return Object.assign(ctx, overrides);
}
