// Settings persistence and hydration helpers.

import {
  STORAGE_KEY_SOURCE_ROOTS, STORAGE_KEY_SOURCE_ROOT_ENABLED,
  STORAGE_KEY_USB_ROOT, STORAGE_KEY_USB_RECENT_ROOTS,
  STORAGE_KEY_MASTER_DB_ENABLED, STORAGE_KEY_SOURCES_EVER_CONFIGURED,
  FRONTEND_DB_KEY_SOURCE_ROOTS, FRONTEND_DB_KEY_SOURCE_ROOT_ENABLED,
  FRONTEND_DB_KEY_USB_ROOT, FRONTEND_DB_KEY_USB_RECENT_ROOTS,
  FRONTEND_DB_KEY_MASTER_DB_ENABLED, FRONTEND_DB_KEY_SOURCES_EVER_CONFIGURED,
  FRONTEND_SETTING_BINDINGS
} from "../../settings_keys.mjs";

export function persistSetting(command, storageKey, dbKey, value) {
  if (!storageKey || !dbKey) return;
  try {
    if (value === null || value === undefined || value === "") {
      localStorage.removeItem(storageKey);
    } else {
      localStorage.setItem(storageKey, String(value));
    }
  } catch {}
  return command("set_frontend_setting", {
    key: dbKey,
    value: value === null || value === undefined || value === "" ? null : String(value)
  }).catch((err) => {
    console.warn(`Failed to persist setting ${dbKey}:`, err);
    throw err;
  });
}

export async function hydrateLocalStorageFromFrontendSettingsDb(command, state) {
  let values = {};
  let nodeAvailable = false;
  let essentiaInstalled = false;
  try {
    const data = await command("get_frontend_settings");
    values = data && typeof data.values === "object" && data.values
      ? data.values
      : {};
    nodeAvailable = !!data?.nodeAvailable;
    essentiaInstalled = !!data?.essentiaInstalled;
  } catch (err) {
    console.warn("Failed to hydrate frontend settings from DB:", err);
    return;
  }

  if (state) {
    state.nodeAvailable = nodeAvailable;
    state.essentiaInstalled = essentiaInstalled;
  }

  for (const binding of FRONTEND_SETTING_BINDINGS) {
    const hasValue = Object.prototype.hasOwnProperty.call(values, binding.dbKey);
    if (!hasValue) continue;
    try {
      localStorage.setItem(binding.storageKey, String(values[binding.dbKey] ?? ""));
    } catch {}
  }
}

export function loadUsbRecentRootsFromStorage(state) {
  try {
    const raw = localStorage.getItem(STORAGE_KEY_USB_RECENT_ROOTS);
    if (!raw) {
      state.usbRecentRoots = [];
      return;
    }
    const parsed = JSON.parse(raw);
    state.usbRecentRoots = Array.isArray(parsed)
      ? parsed
        .map((entry) => String(entry || "").trim())
        .filter((entry, index, arr) => entry.length > 0 && arr.indexOf(entry) === index)
      : [];
  } catch {
    state.usbRecentRoots = [];
  }
}

export function persistUsbRecentRoots(state, command) {
  const rows = Array.isArray(state.usbRecentRoots) ? state.usbRecentRoots.slice(0, 8) : [];
  state.usbRecentRoots = rows;
  persistSetting(
    command,
    STORAGE_KEY_USB_RECENT_ROOTS,
    FRONTEND_DB_KEY_USB_RECENT_ROOTS,
    JSON.stringify(rows)
  );
}

export function rememberUsbRecentRoot(state, command, path, renderCallback) {
  const normalized = String(path || "").trim();
  if (!normalized) return;
  const without = state.usbRecentRoots.filter((row) => row !== normalized);
  state.usbRecentRoots = [normalized, ...without].slice(0, 8);
  persistUsbRecentRoots(state, command);
  if (renderCallback) renderCallback();
}

export function persistSourceRoots(command, roots) {
  persistSetting(command, STORAGE_KEY_SOURCE_ROOTS, FRONTEND_DB_KEY_SOURCE_ROOTS, JSON.stringify(roots || []));
}

export function persistUsbRoot(command, path) {
  if (!path) {
    persistSetting(command, STORAGE_KEY_USB_ROOT, FRONTEND_DB_KEY_USB_ROOT, null);
    return;
  }
  persistSetting(command, STORAGE_KEY_USB_ROOT, FRONTEND_DB_KEY_USB_ROOT, String(path));
}

export function loadSourceRootsFromStorage(state) {
  const raw = localStorage.getItem(STORAGE_KEY_SOURCE_ROOTS);
  if (!raw) {
    state.sourceRoots = [];
    return;
  }

  try {
    const parsed = JSON.parse(raw);
    state.sourceRoots = Array.isArray(parsed)
      ? parsed.filter((x) => typeof x === "string" && x.trim())
      : [];
  } catch {
    state.sourceRoots = [];
  }
}

export function loadSourceRootEnabledFromStorage(state) {
  const raw = localStorage.getItem(STORAGE_KEY_SOURCE_ROOT_ENABLED);
  if (!raw) {
    state.sourceRootEnabled = {};
    return;
  }
  try {
    const parsed = JSON.parse(raw);
    state.sourceRootEnabled = parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    state.sourceRootEnabled = {};
  }
}

export function persistSourceRootEnabled(command, enabledMap) {
  persistSetting(
    command,
    STORAGE_KEY_SOURCE_ROOT_ENABLED,
    FRONTEND_DB_KEY_SOURCE_ROOT_ENABLED,
    JSON.stringify(enabledMap || {})
  );
}

export function persistMasterDbEnabled(command, enabled) {
  persistSetting(command, STORAGE_KEY_MASTER_DB_ENABLED, FRONTEND_DB_KEY_MASTER_DB_ENABLED, enabled ? "1" : "0");
}

export function loadMasterDbEnabledFromStorage(state) {
  state.masterDbEnabled = localStorage.getItem(STORAGE_KEY_MASTER_DB_ENABLED) === "1";
}

export function persistSourcesEverConfigured(command, value) {
  persistSetting(command, STORAGE_KEY_SOURCES_EVER_CONFIGURED, FRONTEND_DB_KEY_SOURCES_EVER_CONFIGURED, value ? "1" : "0");
}

export function loadSourcesEverConfiguredFromStorage(state) {
  state.sourcesEverConfigured =
    localStorage.getItem(STORAGE_KEY_SOURCES_EVER_CONFIGURED) === "1" ||
    (Array.isArray(state.sourceRoots) && state.sourceRoots.length > 0) ||
    state.masterDbEnabled === true;
}

const ACCENT_DEFAULT_HUE = 270;

export function createThemeManager(deps) {
  const {
    persistSetting: persist,
    invoke,
    deriveWaveformColors,
    WAVEFORM_COLORS,
    renderWaveformsIn,
    STORAGE_KEY_THEME,
    FRONTEND_DB_KEY_THEME,
    STORAGE_KEY_ACCENT_HUE
  } = deps;

  let accentManager = null;

  const mgr = {
    _mediaQuery: window.matchMedia("(prefers-color-scheme: dark)"),

    setAccentManager(am) { accentManager = am; },

    preference() {
      try { return localStorage.getItem(STORAGE_KEY_THEME) || "auto"; }
      catch { return "auto"; }
    },

    resolved() {
      const pref = mgr.preference();
      if (pref === "light" || pref === "dark") return pref;
      return mgr._mediaQuery.matches ? "dark" : "light";
    },

    apply() {
      const pref = mgr.preference();
      const resolved = mgr.resolved();

      if (pref === "auto") {
        document.documentElement.removeAttribute("data-theme");
      } else {
        document.documentElement.setAttribute("data-theme", pref);
      }

      updateWaveformColorsInternal();

      const selector = document.getElementById("themeSelector");
      if (selector) {
        selector.querySelectorAll(".theme-option").forEach((opt) => {
          opt.classList.toggle("active", opt.dataset.themeChoice === pref);
        });
      }

      if (typeof invoke === "function") {
        invoke("set_theme_background", { dark: resolved === "dark" }).catch(() => {});
      }
    },

    init() {
      mgr.apply();
      mgr._mediaQuery.addEventListener("change", () => {
        if (mgr.preference() === "auto") mgr.apply();
      });
      const selector = document.getElementById("themeSelector");
      if (selector) {
        selector.addEventListener("click", (event) => {
          const opt = event.target.closest(".theme-option");
          if (!opt) return;
          const choice = opt.dataset.themeChoice;
          if (choice) {
            persist(STORAGE_KEY_THEME, FRONTEND_DB_KEY_THEME, choice);
            mgr.apply();
          }
        });
      }
    }
  };

  function updateWaveformColorsInternal() {
    const hue = accentManager ? accentManager.hue() : ACCENT_DEFAULT_HUE;
    const isDark = mgr.resolved() === "dark";
    const wc = deriveWaveformColors(hue, isDark);
    WAVEFORM_COLORS.base = wc.base;
    WAVEFORM_COLORS.mid = wc.mid;
    WAVEFORM_COLORS.peak = wc.peak;
    if (typeof renderWaveformsIn === "function") {
      renderWaveformsIn(document);
    }
  }

  mgr.updateWaveformColors = updateWaveformColorsInternal;

  return mgr;
}

export function createAccentManager(deps) {
  const {
    el,
    persistSetting: persist,
    themeManager,
    STORAGE_KEY_ACCENT_HUE,
    FRONTEND_DB_KEY_ACCENT_HUE
  } = deps;

  const mgr = {
    hue() {
      try {
        const stored = localStorage.getItem(STORAGE_KEY_ACCENT_HUE);
        if (stored !== null) {
          const parsed = Number(stored);
          if (Number.isFinite(parsed)) return Math.round(parsed) % 360;
        }
      } catch {}
      return ACCENT_DEFAULT_HUE;
    },

    apply(hue) {
      const h = Number.isFinite(hue) ? Math.round(hue) % 360 : mgr.hue();
      document.documentElement.style.setProperty("--accent-h", String(h));
      persist(STORAGE_KEY_ACCENT_HUE, FRONTEND_DB_KEY_ACCENT_HUE, String(h));
      if (el.accentSwatch) el.accentSwatch.style.background = `hsl(${h}, 82%, 55%)`;
      if (el.accentHueSlider) el.accentHueSlider.value = String(h);
      themeManager.updateWaveformColors();
    },

    reset() {
      mgr.apply(ACCENT_DEFAULT_HUE);
    },

    init() {
      mgr.apply();
      if (el.accentHueSlider) {
        el.accentHueSlider.addEventListener("input", () => {
          mgr.apply(Number(el.accentHueSlider.value));
        });
      }
      if (el.accentResetBtn) {
        el.accentResetBtn.addEventListener("click", () => mgr.reset());
      }
    }
  };

  return mgr;
}
