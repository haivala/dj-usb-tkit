//! In-app update check against the GitHub Releases list.
//!
//! Pure logic only -- version comparison and "what makes a release critical".
//! The HTTP fetch lives in `tauri_commands::check_for_update` (it needs the
//! `tauri` feature's `reqwest`); this module takes an already-parsed release
//! list so it stays unit-testable without a network.
//!
//! Severity convention: a release is "critical" when its notes body contains a
//! line like `**Severity:** critical` (markdown bold stripped before matching).
//! The release workflow copies the matching `## <version>` section of
//! CHANGELOG.md verbatim into the GitHub Release body, so a maintainer flags a
//! release by adding that line under the version heading.

use serde::{Deserialize, Serialize};

pub const RELEASES_PAGE_URL: &str = "https://github.com/haivala/dj-usb-tkit/releases";

/// One entry from the GitHub Releases API response (only the fields we use).
#[derive(Debug, Clone, Deserialize)]
pub struct GithubRelease {
    #[serde(default)]
    pub tag_name: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub html_url: Option<String>,
}

/// The verdict handed to the frontend. Mirrors the object the old
/// `vanilla-ui/update_check.mjs` `fetchUpdateInfo` produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub update_available: bool,
    /// `"none"` (up to date, or the check couldn't run), `"normal"`, or
    /// `"critical"`.
    pub severity: String,
    pub current_version: String,
    pub latest_version: String,
    pub release_url: String,
}

impl UpdateInfo {
    pub fn none(current_version: &str) -> Self {
        Self {
            update_available: false,
            severity: "none".to_string(),
            current_version: current_version.to_string(),
            latest_version: current_version.to_string(),
            release_url: RELEASES_PAGE_URL.to_string(),
        }
    }
}

/// `"v1.2.3"` / `"1.2.3-beta"` -> `[1, 2, 3]`; anything without three leading
/// numeric components is `None` (matches the old `/^(\d+)\.(\d+)\.(\d+)/`).
pub fn parse_semver(tag: &str) -> Option<[u32; 3]> {
    let mut parts = tag.trim().trim_start_matches(['v', 'V']).split(['.', '-', '+']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some([major, minor, patch])
}

pub fn release_is_critical(body: &str) -> bool {
    body.replace('*', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .contains("severity: critical")
}

/// Given the running version and the fetched releases, decide whether a newer
/// stable release exists and how urgent it is.
pub fn evaluate(current_version: &str, releases: &[GithubRelease]) -> UpdateInfo {
    let Some(current) = parse_semver(current_version) else {
        return UpdateInfo::none(current_version);
    };

    let mut newer: Vec<(&GithubRelease, [u32; 3])> = releases
        .iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| parse_semver(&r.tag_name).map(|v| (r, v)))
        .filter(|(_, v)| *v > current)
        .collect();
    newer.sort_by_key(|(_, v)| *v);

    let Some(&(latest, latest_version)) = newer.last() else {
        return UpdateInfo::none(current_version);
    };

    let critical = newer
        .iter()
        .any(|(r, _)| r.body.as_deref().is_some_and(release_is_critical));

    UpdateInfo {
        update_available: true,
        severity: if critical { "critical" } else { "normal" }.to_string(),
        current_version: current_version.to_string(),
        latest_version: format!(
            "{}.{}.{}",
            latest_version[0], latest_version[1], latest_version[2]
        ),
        release_url: latest
            .html_url
            .clone()
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| RELEASES_PAGE_URL.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str, body: Option<&str>) -> GithubRelease {
        GithubRelease {
            tag_name: tag.to_string(),
            body: body.map(str::to_string),
            draft: false,
            prerelease: false,
            html_url: Some(format!("https://example.test/{tag}")),
        }
    }

    #[test]
    fn parse_semver_handles_v_prefix_and_suffixes() {
        assert_eq!(parse_semver("v1.2.3"), Some([1, 2, 3]));
        assert_eq!(parse_semver("1.2.3"), Some([1, 2, 3]));
        assert_eq!(parse_semver("1.2.3-beta.1"), Some([1, 2, 3]));
        assert_eq!(parse_semver("1.2"), None);
        assert_eq!(parse_semver("nightly"), None);
    }

    #[test]
    fn no_newer_release_reports_none() {
        let info = evaluate("0.1.35", &[rel("v0.1.35", None), rel("v0.1.34", None)]);
        assert!(!info.update_available);
        assert_eq!(info.severity, "none");
    }

    #[test]
    fn picks_the_highest_newer_stable_release() {
        let releases = [
            rel("v0.1.36", None),
            rel("v0.2.0", None),
            rel("v0.1.35", None),
        ];
        let info = evaluate("0.1.35", &releases);
        assert!(info.update_available);
        assert_eq!(info.latest_version, "0.2.0");
        assert_eq!(info.severity, "normal");
        assert_eq!(info.release_url, "https://example.test/v0.2.0");
    }

    #[test]
    fn drafts_and_prereleases_are_ignored() {
        let mut draft = rel("v9.9.9", None);
        draft.draft = true;
        let mut pre = rel("v8.8.8", None);
        pre.prerelease = true;
        let info = evaluate("0.1.35", &[draft, pre, rel("v0.1.36", None)]);
        assert_eq!(info.latest_version, "0.1.36");
    }

    #[test]
    fn any_newer_release_flagged_critical_makes_the_whole_check_critical() {
        let releases = [
            rel("v0.1.36", Some("Routine fixes.")),
            rel("v0.1.37", Some("Heads up.\n\n**Severity:** critical\n\nUpgrade now.")),
        ];
        let info = evaluate("0.1.35", &releases);
        assert_eq!(info.severity, "critical");
        // ...but the link still points at the newest release.
        assert_eq!(info.latest_version, "0.1.37");
    }

    #[test]
    fn release_is_critical_strips_markdown_and_is_case_insensitive() {
        assert!(release_is_critical("**Severity:**  CRITICAL"));
        assert!(release_is_critical("intro\n*Severity:* critical\noutro"));
        assert!(!release_is_critical("Severity: normal"));
        assert!(!release_is_critical("nothing here"));
    }

    #[test]
    fn unparseable_current_version_reports_none() {
        let info = evaluate("dev", &[rel("v2.0.0", None)]);
        assert!(!info.update_available);
    }
}
