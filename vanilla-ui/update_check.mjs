// In-app update notice rendering.
//
// The check itself -- fetching GitHub Releases, comparing versions, and
// deciding whether a release is "critical" (its notes body carries a
// `**Severity:** critical` line) -- is done in Rust (the `check_for_update`
// command / `backend/src/service/update_check.rs`). This module only renders
// the `state.updateCheck` verdict:
//   { updateAvailable, severity: "none"|"normal"|"critical",
//     currentVersion, latestVersion, releaseUrl }

import { STORAGE_KEY_UPDATE_DISMISSED } from "./settings_keys.mjs";

export const RELEASES_PAGE_URL = "https://github.com/haivala/dj-usb-tkit/releases";

export function renderUpdateNotice(state, el, deps = {}) {
  const { openUrl = () => {} } = deps;
  if (!el.settingsUpdateNote) return;

  const info = state.updateCheck;
  if (!info || !info.updateAvailable) {
    el.settingsUpdateNote.classList.add("hidden");
    el.settingsUpdateNote.textContent = "";
    return;
  }

  el.settingsUpdateNote.classList.remove("hidden");
  el.settingsUpdateNote.innerHTML =
    `<a href="#" class="update-note-link">Update available: ${info.latestVersion}</a>`;
  el.settingsUpdateNote
    .querySelector(".update-note-link")
    ?.addEventListener("click", (event) => {
      event.preventDefault();
      openUrl(info.releaseUrl || RELEASES_PAGE_URL);
    });
}

export function renderCriticalUpdateBanner(state, el, deps = {}) {
  const {
    localStorageObj = typeof localStorage !== "undefined" ? localStorage : null,
    openUrl = () => {}
  } = deps;
  if (!el.criticalUpdateBanner) return;

  const info = state.updateCheck;
  if (!info || info.severity !== "critical") {
    el.criticalUpdateBanner.classList.add("hidden");
    return;
  }

  let dismissedVersion = null;
  try {
    dismissedVersion = localStorageObj?.getItem?.(STORAGE_KEY_UPDATE_DISMISSED) || null;
  } catch {
    dismissedVersion = null;
  }
  if (dismissedVersion === info.latestVersion) {
    el.criticalUpdateBanner.classList.add("hidden");
    return;
  }

  el.criticalUpdateBanner.classList.remove("hidden");
  if (el.criticalUpdateText) {
    el.criticalUpdateText.innerHTML =
      `Critical update available: ${info.latestVersion} — ` +
      `<a href="#" class="critical-update-link">view release</a>`;
    el.criticalUpdateText
      .querySelector(".critical-update-link")
      ?.addEventListener("click", (event) => {
        event.preventDefault();
        openUrl(info.releaseUrl || RELEASES_PAGE_URL);
      });
  }
}

export function dismissCriticalUpdateBanner(state, el, deps = {}) {
  const { localStorageObj = typeof localStorage !== "undefined" ? localStorage : null } = deps;
  if (el.criticalUpdateBanner) {
    el.criticalUpdateBanner.classList.add("hidden");
  }
  try {
    const latestVersion = state.updateCheck?.latestVersion;
    if (latestVersion) {
      localStorageObj?.setItem?.(STORAGE_KEY_UPDATE_DISMISSED, latestVersion);
    }
  } catch {
    // Best-effort persistence only.
  }
}
