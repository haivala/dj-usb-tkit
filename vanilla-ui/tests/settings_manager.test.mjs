import test from "node:test";
import assert from "node:assert/strict";

import {
  hydrateLocalStorageFromFrontendSettingsDb,
  persistSetting
} from "../components/settings/actions.mjs";
import {
  FRONTEND_DB_KEY_ANALYSIS_ENGINE,
  FRONTEND_DB_KEY_THEME,
  STORAGE_KEY_ANALYSIS_ENGINE,
  STORAGE_KEY_THEME
} from "../settings_keys.mjs";

function installLocalStorage() {
  const store = new Map();
  globalThis.localStorage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => { store.set(key, String(value)); },
    removeItem: (key) => { store.delete(key); },
    clear: () => { store.clear(); }
  };
}

test.beforeEach(() => {
  installLocalStorage();
});

test("persistence helpers mirror localStorage values to the frontend settings DB", async () => {
  const calls = [];
  const command = async (name, payload) => { calls.push({ name, payload }); };
  const ctx = { command, localStorage };

  await persistSetting(ctx, STORAGE_KEY_THEME, FRONTEND_DB_KEY_THEME, "dark");
  await persistSetting(ctx, STORAGE_KEY_THEME, FRONTEND_DB_KEY_THEME, "");

  assert.equal(localStorage.getItem(STORAGE_KEY_THEME), null);
  assert.deepEqual(calls.map((call) => call.payload), [
    { key: FRONTEND_DB_KEY_THEME, value: "dark" },
    { key: FRONTEND_DB_KEY_THEME, value: null }
  ]);
});

test("hydrateLocalStorageFromFrontendSettingsDb copies DB-backed values into localStorage", async () => {
  await hydrateLocalStorageFromFrontendSettingsDb({
    localStorage,
    command: async (name) => {
      assert.equal(name, "get_frontend_settings");
      return {
        values: {
          [FRONTEND_DB_KEY_THEME]: "light",
          [FRONTEND_DB_KEY_ANALYSIS_ENGINE]: "stratum"
        }
      };
    }
  });

  assert.equal(localStorage.getItem(STORAGE_KEY_THEME), "light");
  assert.equal(localStorage.getItem(STORAGE_KEY_ANALYSIS_ENGINE), "stratum");
});
