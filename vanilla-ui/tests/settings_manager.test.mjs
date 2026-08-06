import test from "node:test";
import assert from "node:assert/strict";

import {
  persistSetting,
  hydrateLocalStorageFromFrontendSettingsDb,
  persistSourceRoots,
  persistUsbRoot,
  loadSourceRootsFromStorage,
  loadSourceRootEnabledFromStorage,
  loadSourcesEverConfiguredFromStorage,
  persistSourceRootEnabled
} from "../components/settings/actions.mjs";
import {
  STORAGE_KEY_THEME,
  STORAGE_KEY_SOURCE_ROOTS,
  STORAGE_KEY_SOURCE_ROOT_ENABLED,
  STORAGE_KEY_SOURCES_EVER_CONFIGURED,
  STORAGE_KEY_USB_ROOT,
  STORAGE_KEY_ANALYSIS_ENGINE,
  FRONTEND_DB_KEY_THEME,
  FRONTEND_DB_KEY_SOURCE_ROOTS,
  FRONTEND_DB_KEY_SOURCE_ROOT_ENABLED,
  FRONTEND_DB_KEY_USB_ROOT,
  FRONTEND_DB_KEY_ANALYSIS_ENGINE
} from "../settings_keys.mjs";

function installLocalStorage() {
  const store = new Map();
  globalThis.localStorage = {
    getItem(key) {
      return store.has(key) ? store.get(key) : null;
    },
    setItem(key, value) {
      store.set(key, String(value));
    },
    removeItem(key) {
      store.delete(key);
    },
    clear() {
      store.clear();
    }
  };
  return store;
}

function makeState() {
  return {
    usbRecentRoots: [],
    sourceRoots: [],
    sourceRootEnabled: {},
    masterDbEnabled: false,
    sourcesEverConfigured: false
  };
}

test.beforeEach(() => {
  installLocalStorage();
});

test("persistSetting writes localStorage and frontend settings DB", async () => {
  const calls = [];
  const command = async (name, payload) => {
    calls.push({ name, payload });
  };

  persistSetting(command, STORAGE_KEY_THEME, FRONTEND_DB_KEY_THEME, "dark");

  assert.equal(localStorage.getItem(STORAGE_KEY_THEME), "dark");
  assert.deepEqual(calls, [{
    name: "set_frontend_setting",
    payload: { key: FRONTEND_DB_KEY_THEME, value: "dark" }
  }]);
});

test("persistSetting removes empty values from localStorage and persists null", async () => {
  localStorage.setItem(STORAGE_KEY_USB_ROOT, "/tmp/usb");
  const calls = [];
  const command = async (name, payload) => {
    calls.push({ name, payload });
  };

  persistSetting(command, STORAGE_KEY_USB_ROOT, FRONTEND_DB_KEY_USB_ROOT, "");

  assert.equal(localStorage.getItem(STORAGE_KEY_USB_ROOT), null);
  assert.deepEqual(calls, [{
    name: "set_frontend_setting",
    payload: { key: FRONTEND_DB_KEY_USB_ROOT, value: null }
  }]);
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

test("persistSourceRoots and persistUsbRoot encode values through persistSetting", () => {
  const calls = [];
  const command = async (name, payload) => { calls.push({ name, payload }); };

  persistSourceRoots(command, ["/music/a", "/music/b"]);
  persistUsbRoot(command, "/usb/root");

  assert.equal(localStorage.getItem(STORAGE_KEY_SOURCE_ROOTS), "[\"/music/a\",\"/music/b\"]");
  assert.equal(localStorage.getItem(STORAGE_KEY_USB_ROOT), "/usb/root");
  assert.deepEqual(calls, [
    {
      name: "set_frontend_setting",
      payload: { key: FRONTEND_DB_KEY_SOURCE_ROOTS, value: "[\"/music/a\",\"/music/b\"]" }
    },
    {
      name: "set_frontend_setting",
      payload: { key: FRONTEND_DB_KEY_USB_ROOT, value: "/usb/root" }
    }
  ]);
});

test("loadSourceRootsFromStorage and loadSourceRootEnabledFromStorage recover from invalid JSON", () => {
  const state = makeState();
  localStorage.setItem(STORAGE_KEY_SOURCE_ROOTS, "{");
  localStorage.setItem(STORAGE_KEY_SOURCE_ROOT_ENABLED, "{");

  loadSourceRootsFromStorage(state);
  loadSourceRootEnabledFromStorage(state);

  assert.deepEqual(state.sourceRoots, []);
  assert.deepEqual(state.sourceRootEnabled, {});
});

test("loadSourcesEverConfiguredFromStorage migrates existing configured sources", () => {
  const state = makeState();
  state.sourceRoots = ["/music"];

  loadSourcesEverConfiguredFromStorage(state);

  assert.equal(state.sourcesEverConfigured, true);
});

test("loadSourcesEverConfiguredFromStorage respects persisted configured flag", () => {
  const state = makeState();
  localStorage.setItem(STORAGE_KEY_SOURCES_EVER_CONFIGURED, "1");

  loadSourcesEverConfiguredFromStorage(state);

  assert.equal(state.sourcesEverConfigured, true);
});

test("persistSourceRootEnabled stores JSON map in localStorage and DB", () => {
  const calls = [];
  const command = async (name, payload) => { calls.push({ name, payload }); };

  persistSourceRootEnabled(command, { "/music/a": true, "/music/b": false });

  assert.equal(
    localStorage.getItem(STORAGE_KEY_SOURCE_ROOT_ENABLED),
    "{\"/music/a\":true,\"/music/b\":false}"
  );
  assert.deepEqual(calls, [{
    name: "set_frontend_setting",
    payload: {
      key: FRONTEND_DB_KEY_SOURCE_ROOT_ENABLED,
      value: "{\"/music/a\":true,\"/music/b\":false}"
    }
  }]);
});
