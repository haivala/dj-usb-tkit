// Shared helpers for the vanilla-ui node:test suite.

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
