import test from "node:test";
import assert from "node:assert/strict";
import {
  debugFrontendLog,
  handleBackendLogEvent,
  registerBackendJobEvents,
  unregisterBackendJobEvents
} from "../startup_bootstrap.mjs";

test("debugFrontendLog writes only in tauri runtime", async () => {
  const calls = [];
  debugFrontendLog("hello", { a: 1 }, {
    isTauriRuntime: () => false,
    invoke: async (...args) => { calls.push(args); }
  });
  assert.equal(calls.length, 0);

  debugFrontendLog("hello", { a: 1 }, {
    isTauriRuntime: () => true,
    invoke: async (...args) => { calls.push(args); }
  });
  assert.equal(calls.length, 1);
  assert.equal(calls[0][0], "append_frontend_log");
});

test("handleBackendLogEvent normalizes payload into event log entry", () => {
  const logged = [];
  handleBackendLogEvent({
    level: "warn",
    source: "backend",
    code: "X1",
    message: "Something",
    details: "details"
  }, {
    pushEventLog: (entry) => logged.push(entry)
  });

  assert.equal(logged.length, 1);
  assert.equal(logged[0].level, "warn");
  assert.equal(logged[0].code, "X1");
});

test("register/unregister bridge delegates to playback_events core", async () => {
  const state = {
    unlistenJobEvent: async () => {},
    unlistenPlaybackEvent: async () => {},
    unlistenBackendLogEvent: async () => {}
  };
  const listens = [];
  const unlistenCalls = [];

  await registerBackendJobEvents(state, {
    isTauriRuntime: () => true,
    unregisterBackendJobEvents: async () => {
      await unregisterBackendJobEvents(state, {
        warn: (...args) => unlistenCalls.push(args)
      });
    },
    getTauriEventListen: () => async (_name, _handler) => {
      listens.push(_name);
      return async () => {};
    },
    handleJobEvent: () => {},
    handlePlaybackEvent: () => {},
    handleBackendLogEvent: () => {}
  });

  assert.deepEqual(listens.sort(), ["backend:log", "job:event", "playback:event"].sort());
  await unregisterBackendJobEvents(state, { warn: (...args) => unlistenCalls.push(args) });
  assert.equal(state.unlistenJobEvent, null);
  assert.equal(state.unlistenPlaybackEvent, null);
  assert.equal(state.unlistenBackendLogEvent, null);
});
