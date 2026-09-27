// In-app update notice rendering.
//
// The check itself -- fetching GitHub Releases, comparing versions, and
// deciding how urgently a release is flagged (its notes body carries a
// `**Severity:** critical` or `**Severity:** feature` line) -- is done in Rust
// (the `check_for_update` command / `backend/src/service/update_check.rs`).
// This module only renders the `state.updateCheck` verdict:
//   { updateAvailable, severity: "none"|"normal"|"feature"|"critical",
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

// The banner's wording per severity; any other severity gets no banner (just
// the quiet settings note).
const BANNER_TEXT = {
  critical: (version) => `Critical update available: ${version}`,
  feature: (version) => `New features available: ${version}`,
};

export function renderUpdateBanner(state, el, deps = {}) {
  const {
    localStorageObj = typeof localStorage !== "undefined" ? localStorage : null,
    openUrl = () => {}
  } = deps;
  if (!el.updateBanner) return;

  const info = state.updateCheck;
  const bannerText = BANNER_TEXT[info?.severity];
  if (!info || !bannerText) {
    el.updateBanner.classList.add("hidden");
    return;
  }

  let dismissedVersion = null;
  try {
    dismissedVersion = localStorageObj?.getItem?.(STORAGE_KEY_UPDATE_DISMISSED) || null;
  } catch {
    dismissedVersion = null;
  }
  if (dismissedVersion === info.latestVersion) {
    el.updateBanner.classList.add("hidden");
    return;
  }

  el.updateBanner.classList.remove("hidden");
  el.updateBanner.classList.toggle("is-critical", info.severity === "critical");
  el.updateBanner.classList.toggle("is-feature", info.severity === "feature");
  el.updateBanner.setAttribute("role", info.severity === "critical" ? "alert" : "status");
  if (el.updateBannerText) {
    el.updateBannerText.innerHTML =
      `${bannerText(info.latestVersion)} — ` +
      `<a href="#" class="update-banner-link">view release</a>`;
    el.updateBannerText
      .querySelector(".update-banner-link")
      ?.addEventListener("click", (event) => {
        event.preventDefault();
        openUrl(info.releaseUrl || RELEASES_PAGE_URL);
      });
  }
}

export function dismissUpdateBanner(state, el, deps = {}) {
  const { localStorageObj = typeof localStorage !== "undefined" ? localStorage : null } = deps;
  if (el.updateBanner) {
    el.updateBanner.classList.add("hidden");
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
