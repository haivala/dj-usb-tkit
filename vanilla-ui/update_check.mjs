// In-app update check against GitHub Releases.
//
// Severity convention: a release is "critical" if its release notes body
// contains a line like `**Severity:** critical` (markdown bold stripped
// before matching). The release workflow copies the matching `## <version>`
// section of CHANGELOG.md verbatim into the GitHub Release body, so a
// maintainer flags a release as critical by adding that line under the
// version heading in CHANGELOG.md.

import { STORAGE_KEY_UPDATE_DISMISSED } from "./settings_keys.mjs";

export const RELEASES_API_URL =
  "https://api.github.com/repos/haivala/dj-usb-tkit/releases?per_page=10";
export const RELEASES_PAGE_URL = "https://github.com/haivala/dj-usb-tkit/releases";

export function parseSemver(tag) {
  if (typeof tag !== "string") return null;
  const match = tag.trim().replace(/^v/i, "").match(/^(\d+)\.(\d+)\.(\d+)/);
  if (!match) return null;
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

export function compareSemver(a, b) {
  for (let i = 0; i < 3; i++) {
    if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  }
  return 0;
}

export function releaseIsCritical(body) {
  if (typeof body !== "string") return false;
  const cleaned = body.replace(/\*/g, "").replace(/\s+/g, " ").toLowerCase();
  return cleaned.includes("severity: critical");
}

function noUpdateResult(currentVersion) {
  return {
    updateAvailable: false,
    severity: "none",
    currentVersion: currentVersion || "",
    latestVersion: currentVersion || "",
    releaseUrl: RELEASES_PAGE_URL
  };
}

export async function fetchUpdateInfo(currentVersion, deps = {}) {
  const { fetchFn = typeof fetch !== "undefined" ? fetch : null } = deps;
  const fallback = noUpdateResult(currentVersion);

  const currentParsed = parseSemver(currentVersion);
  if (!currentParsed || !fetchFn) return fallback;

  try {
    const response = await fetchFn(RELEASES_API_URL, {
      headers: { Accept: "application/vnd.github+json" }
    });
    if (!response || !response.ok) return fallback;

    const releases = await response.json();
    if (!Array.isArray(releases)) return fallback;

    const newer = releases
      .filter((release) => release && !release.draft && !release.prerelease)
      .map((release) => ({ version: parseSemver(release.tag_name), release }))
      .filter((entry) => entry.version && compareSemver(entry.version, currentParsed) > 0)
      .sort((a, b) => compareSemver(a.version, b.version));

    if (newer.length === 0) return fallback;

    const latest = newer[newer.length - 1];
    const severity = newer.some((entry) => releaseIsCritical(entry.release.body))
      ? "critical"
      : "normal";

    return {
      updateAvailable: true,
      severity,
      currentVersion,
      latestVersion: latest.version.join("."),
      releaseUrl: latest.release.html_url || RELEASES_PAGE_URL
    };
  } catch {
    return fallback;
  }
}

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
