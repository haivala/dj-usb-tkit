import test from "node:test";
import assert from "node:assert/strict";

if (typeof globalThis.window === "undefined") {
  globalThis.window = {};
}

import { createApiClient } from "../api_client.mjs";

function makeClient(overrides = {}) {
  return createApiClient({
    tauriInvoke: overrides.tauriInvoke || (() => { throw new Error("not tauri"); }),
    tauriIsTauri: overrides.tauriIsTauri || (() => false),
    tauriListen: overrides.tauriListen || (() => {}),
  });
}

test("runtime detection handles false, true, and thrown checks", () => {
  for (const [tauriIsTauri, expected] of [
    [() => false, false],
    [() => true, true],
    [() => { throw new Error("no window"); }, false],
  ]) {
    assert.equal(makeClient({ tauriIsTauri }).isTauriRuntime(), expected);
  }
});

test("invoke delegates to tauriInvoke in Tauri", async () => {
  const calls = [];
  const client = makeClient({
    tauriIsTauri: () => true,
    tauriInvoke: (cmd, payload) => {
      calls.push({ cmd, payload });
      return { ok: true, data: "real" };
    },
  });
  assert.equal((await client.invoke("scan_library", { foo: 1 })).data, "real");
  assert.deepEqual(calls, [{ cmd: "scan_library", payload: { foo: 1 } }]);
});

test("invoke routes through window.__TAURI__.core.invoke when present outside Tauri", async () => {
  const calls = [];
  const prev = window.__TAURI__;
  window.__TAURI__ = {
    core: {
      invoke: (cmd, payload) => {
        calls.push({ cmd, payload });
        return { ok: true, data: "routed" };
      },
    },
  };
  try {
    const client = makeClient();
    assert.equal((await client.invoke("list_playlists", {})).data, "routed");
    assert.deepEqual(calls, [{ cmd: "list_playlists", payload: {} }]);
  } finally {
    window.__TAURI__ = prev;
  }
});

test("invoke returns an INTERNAL_ERROR envelope when no backend is available", async () => {
  const prev = window.__TAURI__;
  delete window.__TAURI__;
  try {
    const response = await makeClient().invoke("scan_library");
    assert.equal(response.ok, false);
    assert.equal(response.error.code, "INTERNAL_ERROR");
  } finally {
    if (prev !== undefined) window.__TAURI__ = prev;
  }
});

test("command() unwraps ok envelopes and throws on failure", async () => {
  const ok = makeClient({
    tauriIsTauri: () => true,
    tauriInvoke: () => ({ ok: true, data: { items: [1, 2] } }),
  });
  assert.deepEqual(await ok.command("list_playlists"), { items: [1, 2] });

  const bad = makeClient({
    tauriIsTauri: () => true,
    tauriInvoke: () => ({ ok: false, error: { message: "boom" } }),
  });
  await assert.rejects(() => bad.command("list_playlists"), /boom/);
});

test("command() carries the backend error's code and details onto the thrown Error", async () => {
  const client = makeClient({
    tauriIsTauri: () => true,
    tauriInvoke: () => ({
      ok: false,
      error: {
        code: "NOT_FOUND",
        message: "track not found in Library or selected USB",
        details: { validationType: "missing_analysis", missingTrackCount: 2 },
      },
    }),
  });
  const err = await client.command("play_resolved_track").then(
    () => { throw new Error("expected rejection"); },
    (e) => e,
  );
  assert.equal(err.code, "NOT_FOUND");
  assert.deepEqual(err.details, { validationType: "missing_analysis", missingTrackCount: 2 });
});
