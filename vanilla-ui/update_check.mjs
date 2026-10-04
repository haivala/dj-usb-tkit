// In-app update UI: the version + re-check button in the settings header, the
// update line at the top of the settings body, the dot on the settings button,
// and the banner for feature/critical releases.
//
// The check itself -- fetching GitHub Releases, comparing versions, and
// deciding how urgently a release is flagged (its notes body carries a
// `**Severity:** critical` or `**Severity:** feature` line) -- is done in Rust
// (the `check_for_update` command / `backend/src/service/update_check.rs`).
// This module only renders the `state.updateCheck` verdict:
//   { updateAvailable, severity: "none"|"normal"|"feature"|"critical",
//     currentVersion, latestVersion, releaseUrl,
//     installKind: "appimage"|"deb"|"rpm"|"nsis"|"msi"|"dmg"|"unknown",
//     downloadUrl: string|null, canSelfUpdate,
//     action: "install"|"download"|"none", checkFailed }
// `action` (the one way to update this install) is decided by the backend;
// `downloadUrl` is also offered after a failed in-app install.

import { STORAGE_KEY_UPDATE_DISMISSED } from "./settings_keys.mjs";
import { openExternalUrl } from "./ui_utils.mjs";

export const RELEASES_PAGE_URL = "https://github.com/haivala/dj-usb-tkit/releases";

// The app stays open for days at a time, so the startup check is repeated.
export const UPDATE_RECHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

// The "Update & restart" button / download link, in the settings update line
// and in the banner.
function renderUpdateActions(ctx, container) {
  if (!container) return;
  const { state } = ctx;
  const info = state.updateCheck;
  const action = info?.updateAvailable ? info.action : "none";
  const download = container.querySelector(".update-download-link");
  const install = container.querySelector(".update-install-btn");

  const downloadUrl = action === "download" || (action === "install" && state.updateInstallFailed)
    ? info.downloadUrl
    : null;
  download.classList.toggle("hidden", !downloadUrl);
  download.onclick = (event) => {
    event.preventDefault();
    if (downloadUrl) openExternalUrl(ctx.window, downloadUrl);
  };

  install.classList.toggle("hidden", action !== "install");
  install.disabled = !!state.updateInstalling;
  install.onclick = () => {
    installUpdate(ctx);
  };
}

// The settings update line. Empty (line hidden) unless there is something to
// say: "Up to date" only answers a manual check, and stays until the drawer
// closes. A check in progress keeps the previous line (the header button spins).
function updateStatusText(state) {
  const info = state.updateCheck;
  if (!info) return "";
  if (info.updateAvailable) {
    return state.updateInstallFailed
      ? `Couldn't install version ${info.latestVersion}.`
      : `Version ${info.latestVersion} is available.`;
  }
  if (info.checkFailed) return "Couldn't check for updates.";
  return state.updateCheckManual ? "Up to date." : "";
}

// The settings header's re-check button, the update line, and the
// settings-button dot.
export function renderUpdateNotice(ctx) {
  const { state, el } = ctx;
  const info = state.updateCheck;
  const available = !!info?.updateAvailable;

  if (el.settingsUpdateDot) {
    el.settingsUpdateDot.classList.toggle("hidden", !available);
    el.settingsBtn?.setAttribute("aria-label", available ? "Settings (update available)" : "Settings");
  }

  // Only offered once a check has run (never in the browser/test build).
  const checkBtn = el.settingsUpdateCheckBtn;
  if (checkBtn) {
    checkBtn.classList.toggle("hidden", !info && !state.updateChecking);
    checkBtn.classList.toggle("is-checking", !!state.updateChecking);
    checkBtn.disabled = !!state.updateChecking || !!state.updateInstalling;
  }

  const section = el.settingsUpdateSection;
  if (!section) return;
  const text = updateStatusText(state);
  section.classList.toggle("hidden", !text);
  section.classList.toggle("is-available", available);
  el.settingsUpdateStatus.textContent = text;

  const link = el.settingsUpdateReleaseLink;
  link.classList.toggle("hidden", !available);
  link.onclick = (event) => {
    event.preventDefault();
    openExternalUrl(ctx.window, info?.releaseUrl || RELEASES_PAGE_URL);
  };

  renderUpdateActions(ctx, el.settingsUpdateActions);
}

// Closing the drawer drops a manual check's "Up to date": it answered that
// click, and isn't news the next time the drawer opens.
export function clearManualUpdateCheck(ctx) {
  if (!ctx.state?.updateCheckManual) return;
  ctx.state.updateCheckManual = false;
  renderUpdateNotice(ctx);
}

// Runs the backend check and re-renders. Startup, the periodic re-check and
// the settings header's re-check button (`manual`) all come through here.
export async function refreshUpdateCheck(ctx, { manual = false } = {}) {
  const { state } = ctx;
  if (!ctx.isTauriRuntime()) return;
  if (state.updateChecking || state.updateInstalling) return;
  if (manual) state.updateCheckManual = true;
  state.updateChecking = true;
  renderUpdateNotice(ctx);
  try {
    // Backend-owned: `check_for_update` knows the running version and does the
    // GitHub fetch + version compare itself (see backend/src/service/update_check.rs).
    const info = await ctx.command("check_for_update");
    if (info) {
      if (info.latestVersion !== state.updateCheck?.latestVersion) {
        state.updateInstallFailed = false;
      }
      state.updateCheck = info;
    }
  } catch {
    // An update check must never disrupt anything else.
  } finally {
    state.updateChecking = false;
    renderUpdateNotice(ctx);
    renderUpdateBanner(ctx);
  }
}

// The banner's wording per severity; any other severity gets no banner (just
// the quiet settings line).
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
    // Still running, so the install failed: put the banner back for a retry,
    // with the direct download as the fallback.
    state.updateInstalling = false;
    state.updateInstallFailed = true;
    renderUpdateNotice(ctx);
    renderUpdateBanner(ctx);
  }
}
