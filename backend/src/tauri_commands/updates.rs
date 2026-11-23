//! Checking for and installing app updates.

use super::jobs::{JobIdentity, emit_job_event, emit_job_failed, lock_jobs_when_idle};
use crate::error::{ErrorCode, ErrorPayload};
use crate::i18n::tr;
use crate::job_conflicts::JobType;
use crate::models::ApiResponse;
use std::thread;
use tauri::AppHandle;
use uuid::Uuid;

/// HTTP client for the backend's own downloads. reqwest is built without a
/// bundled TLS crypto provider (`rustls-no-provider`, as tauri-plugin-updater
/// uses) to keep the aws-lc C build out of the tree, so install rustls's ring
/// provider first; whichever of us or the updater runs first wins.
pub(super) fn http_client() -> Result<reqwest::Client, String> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    reqwest::Client::builder()
        .user_agent(concat!("dj-usb-tkit/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

pub(super) const RELEASES_API_URL: &str =
    "https://api.github.com/repos/haivala/dj-usb-tkit/releases?per_page=10";

/// Which release asset the running build came from. The bundler patches a
/// marker into each packaged binary (`tauri::utils::platform::bundle_type`);
/// debug builds (`cargo run`) are always `Unknown` -- on macOS `bundle_type`
/// reports `App` even outside a bundle, and a dev build must never self-update.
pub(super) fn current_install_kind() -> crate::service::update_check::InstallKind {
    use crate::service::update_check::InstallKind;
    use tauri::utils::{config::BundleType, platform::bundle_type};

    if cfg!(debug_assertions) {
        return InstallKind::Unknown;
    }
    // The AUR package repackages the deb, so its binary says Deb; pacman's
    // own database is what tells the two apart.
    if cfg!(target_os = "linux")
        && std::env::current_exe().is_ok_and(|exe| {
            crate::service::update_check::pacman_owns(
                std::path::Path::new(crate::service::update_check::PACMAN_LOCAL_DB),
                &exe,
            )
        })
    {
        return InstallKind::Pacman;
    }
    match bundle_type() {
        Some(BundleType::AppImage) => InstallKind::AppImage,
        Some(BundleType::Deb) => InstallKind::Deb,
        Some(BundleType::Rpm) => InstallKind::Rpm,
        Some(BundleType::Nsis) => InstallKind::Nsis,
        Some(BundleType::Msi) => InstallKind::Msi,
        Some(BundleType::App | BundleType::Dmg) => InstallKind::Dmg,
        None => InstallKind::Unknown,
    }
}

/// Checks GitHub Releases for a newer stable build than the running one.
/// Version comparison and the severity rules live in
/// `service::update_check`; this only does the fetch and detects the install
/// kind. A failed check is background noise -- it logs quietly and reports
/// "no update" rather than surfacing an error.
#[tauri::command]
pub async fn check_for_update() -> ApiResponse<crate::service::update_check::UpdateInfo> {
    use crate::service::update_check::{self, GithubRelease, UpdateInfo};

    let current = env!("CARGO_PKG_VERSION");
    let install_kind = current_install_kind();

    let fetch = async {
        let response = http_client()?
            .get(RELEASES_API_URL)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "GitHub releases API returned {}",
                response.status()
            ));
        }
        let body = response.text().await.map_err(|e| e.to_string())?;
        serde_json::from_str::<Vec<GithubRelease>>(&body).map_err(|e| e.to_string())
    };

    match fetch.await {
        Ok(releases) => {
            ApiResponse::success(update_check::evaluate(current, install_kind, &releases))
        }
        Err(message) => {
            crate::logging::debug("update-check", format!("update check failed: {message}"));
            ApiResponse::success(UpdateInfo::failed(current, install_kind))
        }
    }
}

/// Downloads, verifies and installs the newest release with
/// `tauri-plugin-updater`, then restarts into it. Runs as an ordinary
/// `job:event` job (`job_type: "update"`) so the footer progress bar and the
/// event log show it like any other long operation. The frontend only offers
/// this when `check_for_update` said `canSelfUpdate`; the updater manifest
/// (`latest.json`) carries only installer-specific keys, so a deb/rpm/MSI
/// install can't pick up another format's payload even if it got here.
///
/// A job started while the update downloads (an export, a scan) is waited
/// for before installing, and `RUNNING_JOBS` stays locked from then on so no
/// new one can start before the restart cuts it short.
///
/// On Windows the NSIS installer exits the app itself once it starts, so the
/// restart below is only reached on Linux (AppImage) and macOS.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> ApiResponse<()> {
    use tauri_plugin_updater::UpdaterExt;

    let job_id = format!("job-update-{}", Uuid::now_v7());
    let identity = JobIdentity {
        job_id: &job_id,
        job_type: JobType::Update,
        stage: "install_update",
    };
    emit_job_event(
        &app,
        "job.started",
        identity,
        0,
        1,
        0,
        tr::job_install_update_started().render(),
    );

    // Only ever ends in an error: success restarts the app.
    let result: Result<std::convert::Infallible, String> = async {
        let update = app
            .updater()
            .map_err(|e| e.to_string())?
            .check()
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| {
                "no installable update is published for this platform yet; use the download link"
                    .to_string()
            })?;
        let version = update.version.clone();
        let mut downloaded: u64 = 0;
        let mut last_percent = None;
        let bytes = update
            .download(
                |chunk_len, content_len| {
                    downloaded += chunk_len as u64;
                    let percent = content_len
                        .filter(|total| *total > 0)
                        .map_or(0, |total| (downloaded.saturating_mul(100) / total) as usize);
                    // One event per whole percent, not per network chunk.
                    if last_percent != Some(percent) {
                        last_percent = Some(percent);
                        emit_job_event(
                            &app,
                            "job.progress",
                            identity,
                            downloaded as usize,
                            content_len.unwrap_or(0) as usize,
                            percent,
                            tr::job_install_update_downloading(&version).render(),
                        );
                    }
                },
                || {},
            )
            .await
            .map_err(|e| e.to_string())?;

        let mut waiting = false;
        let _no_new_jobs = loop {
            if let Some(lock) = lock_jobs_when_idle(&job_id) {
                break lock;
            }
            if !waiting {
                waiting = true;
                emit_job_event(
                    &app,
                    "job.progress",
                    identity,
                    1,
                    1,
                    100,
                    tr::job_install_update_waiting(&version).render(),
                );
            }
            let _ = tauri::async_runtime::spawn_blocking(|| {
                thread::sleep(std::time::Duration::from_millis(500))
            })
            .await;
        };

        // Progress events don't touch `RUNNING_JOBS`, so this can't deadlock
        // on the lock held above.
        emit_job_event(
            &app,
            "job.progress",
            identity,
            1,
            1,
            100,
            tr::job_install_update_installing(&version).render(),
        );
        update.install(bytes).map_err(|e| e.to_string())?;
        app.restart()
    }
    .await;

    let Err(message) = result;
    // ApiResponse::failure below puts the reason in the Event Log.
    // The reason is the updater's own text (English); the words around it are the catalog's.
    let text = tr::job_install_update_failed(&message);
    let payload = ErrorPayload {
        log_message: Some(text.render_english()),
        ..ErrorPayload::new(ErrorCode::InternalError, text.render())
    };
    emit_job_failed(&app, identity, Some(payload.message.clone()));
    ApiResponse::failure(payload)
}
