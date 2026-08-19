import test from "node:test";
import assert from "node:assert/strict";

import {
  hydrateLocalStorageFromFrontendSettingsDb,
  loadSourceRootEnabledFromStorage,
  loadSourceRootsFromStorage,
  loadSourcesEverConfiguredFromStorage,
  persistSetting,
  persistSourceRootEnabled,
  persistSourceRoots,
  persistUsbRoot
} from "../components/settings/actions.mjs";
import {
  FRONTEND_DB_KEY_ANALYSIS_ENGINE,
  FRONTEND_DB_KEY_SOURCE_ROOT_ENABLED,
  FRONTEND_DB_KEY_SOURCE_ROOTS,
  FRONTEND_DB_KEY_THEME,
  FRONTEND_DB_KEY_USB_ROOT,
  STORAGE_KEY_ANALYSIS_ENGINE,
  STORAGE_KEY_SOURCE_ROOT_ENABLED,
  STORAGE_KEY_SOURCE_ROOTS,
  STORAGE_KEY_SOURCES_EVER_CONFIGURED,
  STORAGE_KEY_THEME,
  STORAGE_KEY_USB_ROOT
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

function makeState(overrides = {}) {
  return {
    usbRecentRoots: [],
    sourceRoots: [],
    sourceRootEnabled: {},
    masterDbEnabled: false,
    sourcesEverConfigured: false,
    ...overrides
  };
}

test.beforeEach(() => {
  installLocalStorage();
});

test("persistence helpers mirror localStorage values to the frontend settings DB", async () => {
  const calls = [];
  const command = async (name, payload) => { calls.push({ name, payload }); };

  await persistSetting(command, STORAGE_KEY_THEME, FRONTEND_DB_KEY_THEME, "dark");
  localStorage.setItem(STORAGE_KEY_USB_ROOT, "/tmp/usb");
  await persistSetting(command, STORAGE_KEY_USB_ROOT, FRONTEND_DB_KEY_USB_ROOT, "");
  persistSourceRoots(command, ["/music/a", "/music/b"]);
  persistUsbRoot(command, "/usb/root");
  persistSourceRootEnabled(command, { "/music/a": true, "/music/b": false });

  assert.equal(localStorage.getItem(STORAGE_KEY_THEME), "dark");
  assert.equal(localStorage.getItem(STORAGE_KEY_USB_ROOT), "/usb/root");
  assert.equal(localStorage.getItem(STORAGE_KEY_SOURCE_ROOTS), "[\"/music/a\",\"/music/b\"]");
  assert.equal(localStorage.getItem(STORAGE_KEY_SOURCE_ROOT_ENABLED), "{\"/music/a\":true,\"/music/b\":false}");
  assert.deepEqual(calls.map((call) => call.payload), [
    { key: FRONTEND_DB_KEY_THEME, value: "dark" },
    { key: FRONTEND_DB_KEY_USB_ROOT, value: null },
    { key: FRONTEND_DB_KEY_SOURCE_ROOTS, value: "[\"/music/a\",\"/music/b\"]" },
    { key: FRONTEND_DB_KEY_USB_ROOT, value: "/usb/root" },
    { key: FRONTEND_DB_KEY_SOURCE_ROOT_ENABLED, value: "{\"/music/a\":true,\"/music/b\":false}" }
  ]);
});

test("hydrateLocalStorageFromFrontendSettingsDb copies DB-backed values into localStorage", async () => {
  await hydrateLocalStorageFromFrontendSettingsDb(async (name) => {
    assert.equal(name, "get_frontend_settings");
    return {
      values: {
        [FRONTEND_DB_KEY_THEME]: "light",
        [FRONTEND_DB_KEY_ANALYSIS_ENGINE]: "stratum"
      }
    };
  });

  assert.equal(localStorage.getItem(STORAGE_KEY_THEME), "light");
  assert.equal(localStorage.getItem(STORAGE_KEY_ANALYSIS_ENGINE), "stratum");
});

test("storage loaders recover invalid JSON and derive sources-ever-configured", () => {
  const invalid = makeState();
  localStorage.setItem(STORAGE_KEY_SOURCE_ROOTS, "{");
  localStorage.setItem(STORAGE_KEY_SOURCE_ROOT_ENABLED, "{");
  loadSourceRootsFromStorage(invalid);
  loadSourceRootEnabledFromStorage(invalid);
  assert.deepEqual(invalid.sourceRoots, []);
  assert.deepEqual(invalid.sourceRootEnabled, {});

  const migrated = makeState({ sourceRoots: ["/music"] });
  loadSourcesEverConfiguredFromStorage(migrated);
  assert.equal(migrated.sourcesEverConfigured, true);

  const persisted = makeState();
  localStorage.setItem(STORAGE_KEY_SOURCES_EVER_CONFIGURED, "1");
  loadSourcesEverConfiguredFromStorage(persisted);
  assert.equal(persisted.sourcesEverConfigured, true);
});
