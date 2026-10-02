// In-app update notice rendering.
//
// The check itself -- fetching GitHub Releases, comparing versions, and
// deciding how urgently a release is flagged (its notes body carries a
// `**Severity:** critical` or `**Severity:** feature` line) -- is done in Rust
// (the `check_for_update` command / `backend/src/service/update_check.rs`).
// This module only renders the `state.updateCheck` verdict:
//   { updateAvailable, severity: "none"|"normal"|"feature"|"critical",
//     currentVersion, latestVersion, releaseUrl,
//     installKind: "appimage"|"deb"|"rpm"|"nsis"|"msi"|"dmg"|"unknown",
//     downloadUrl: string|null, canSelfUpdate }
// `downloadUrl` (the latest release's asset for this install kind) and
// `canSelfUpdate` (the in-app updater may replace this install) are both
// decided by the backend.

import { STORAGE_KEY_UPDATE_DISMISSED } from "./settings_keys.mjs";
import { openExternalUrl } from "./ui_utils.mjs";

export const RELEASES_PAGE_URL = "https://github.com/haivala/dj-usb-tkit/releases";

// The download link and "Update & restart" button next to the settings note
// and in the banner.
function renderUpdateActions(ctx, container) {
  if (!container) return;
  const info = ctx.state.updateCheck;
  const available = !!info?.updateAvailable;
  const download = container.querySelector(".update-download-link");
  const install = container.querySelector(".update-install-btn");

  const downloadUrl = available ? info.downloadUrl : null;
  download.classList.toggle("hidden", !downloadUrl);
  download.onclick = (event) => {
    event.preventDefault();
    if (downloadUrl) openExternalUrl(ctx.window, downloadUrl);
  };

  install.classList.toggle("hidden", !(available && info.canSelfUpdate));
  install.disabled = !!ctx.state.updateInstalling;
  install.onclick = () => {
    installUpdate(ctx);
  };
}

export function renderUpdateNotice(ctx) {
  const { state, el } = ctx;
  if (!el.settingsUpdateNote) return;
  renderUpdateActions(ctx, el.settingsUpdateActions);

  const info = state.updateCheck;
  const link = el.settingsUpdateNote.querySelector(".update-note-link");
  if (!info || !info.updateAvailable) {
    el.settingsUpdateNote.classList.add("hidden");
    link.textContent = "";
    return;
  }

  el.settingsUpdateNote.classList.remove("hidden");
  link.textContent = `Update available: ${info.latestVersion}`;
  link.onclick = (event) => {
    event.preventDefault();
    openExternalUrl(ctx.window, info.releaseUrl || RELEASES_PAGE_URL);
  };
}

// The banner's wording per severity; any other severity gets no banner (just
// the quiet settings note).
const BANNER_TEXT = {
  critical: (version) => `Critical update available: ${version}`,
  feature: (version) => `New features available: ${version}`,
};

export function renderUpdateBanner(ctx) {
  const { state, el, localStorage } = ctx;
  if (!el.updateBanner) return;

  renderUpdateActions(ctx, el.updateBannerActions);
  const info = state.updateCheck;
  const bannerText = BANNER_TEXT[info?.severity];
  if (!info || !bannerText) {
    el.updateBanner.classList.add("hidden");
    return;
  }

  let dismissedVersion = null;
  try {
    dismissedVersion = localStorage?.getItem?.(STORAGE_KEY_UPDATE_DISMISSED) || null;
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
    el.updateBannerText.querySelector(".update-banner-message").textContent = bannerText(info.latestVersion);
    el.updateBannerText.querySelector(".update-banner-link").onclick = (event) => {
      event.preventDefault();
      openExternalUrl(ctx.window, info.releaseUrl || RELEASES_PAGE_URL);
    };
  }
}

export function dismissUpdateBanner(ctx) {
  const { state, el, localStorage } = ctx;
  if (el.updateBanner) {
    el.updateBanner.classList.add("hidden");
  }
  try {
    const latestVersion = state.updateCheck?.latestVersion;
    if (latestVersion) {
      localStorage?.setItem?.(STORAGE_KEY_UPDATE_DISMISSED, latestVersion);
    }
  } catch {
    // Best-effort persistence only.
  }
}

// Runs the backend's `install_update` job: progress, failure and the event-log
// entries come through `job:event` like any other job (footer progress bar).
// On success the app restarts, so the call never returns.
export async function installUpdate(ctx) {
  const { state, el } = ctx;
  if (state.updateInstalling) return;
  // A restart in the middle of an export or scan would cut it short.
  if (state.activeJobId) {
    ctx.emitMessage({
      level: "warn",
      source: "update",
      code: "update.busy",
      status: { text: "Finish the running job before updating." }
    });
    return;
  }

  state.updateInstalling = true;
  el.updateBanner?.classList.add("hidden");
  renderUpdateNotice(ctx);
  try {
    await ctx.command("install_update");
  } catch {
    // Already reported: the backend emitted `job.failed` with the reason.
  } finally {
    state.updateInstalling = false;
    renderUpdateNotice(ctx);
  }
}
