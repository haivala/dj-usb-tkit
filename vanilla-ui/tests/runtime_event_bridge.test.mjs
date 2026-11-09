import test from "node:test";
import assert from "node:assert/strict";
import { debugFrontendLog, handleBackendLogEvent } from "../startup_bootstrap.mjs";
import { logToFile } from "../components/event-log/actions.mjs";

test("debugFrontendLog writes a file-only debug line, only in tauri runtime", async () => {
  const calls = [];
  const ctxFor = (tauri) => {
    const ctx = {
      isTauriRuntime: () => tauri,
      invoke: async (...args) => { calls.push(args); }
    };
    ctx.logToFile = (entry) => logToFile(ctx, entry);
    return ctx;
  };
  debugFrontendLog(ctxFor(false), "hello", { a: 1 });
  assert.equal(calls.length, 0);

  debugFrontendLog(ctxFor(true), "hello", { a: 1 });
  assert.equal(calls.length, 1);
  assert.equal(calls[0][0], "append_frontend_log");
  // `debug` in the file: never shown in the Event Log, like the backend's developer lines.
  assert.deepEqual(calls[0][1], { level: "debug", message: '[analysis-ui] [analysis_ui.event] hello {"a":1}' });
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
