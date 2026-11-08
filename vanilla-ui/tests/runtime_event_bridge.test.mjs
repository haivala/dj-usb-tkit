import test from "node:test";
import assert from "node:assert/strict";
import { debugFrontendLog, handleBackendLogEvent } from "../startup_bootstrap.mjs";

test("debugFrontendLog writes only in tauri runtime", async () => {
  const calls = [];
  debugFrontendLog({
    isTauriRuntime: () => false,
    invoke: async (...args) => { calls.push(args); }
  }, "hello", { a: 1 });
  assert.equal(calls.length, 0);

  debugFrontendLog({
    isTauriRuntime: () => true,
    invoke: async (...args) => { calls.push(args); }
  }, "hello", { a: 1 });
  assert.equal(calls.length, 1);
  assert.equal(calls[0][0], "append_frontend_log");
});

test("handleBackendLogEvent stores the entry without writing the log file again", () => {
  const logged = [];
  handleBackendLogEvent({
    storeEventLogEntry: (entry) => logged.push(entry)
  }, {
    level: "warn",
    source: "backend",
    message: "Something"
  });

  assert.equal(logged.length, 1);
  assert.equal(logged[0].level, "warn");
  assert.equal(logged[0].message, "Something");
  assert.equal(logged[0].inLogFile, true);
});
