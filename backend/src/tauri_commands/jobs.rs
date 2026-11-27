//! What every command shares: the running-jobs table and the actions it
//! blocks, running a command off the main thread, and the job events a long
//! command reports its progress with.

use crate::commands::BackendCommands;
use crate::error::{ErrorCode, ErrorPayload};
use crate::i18n::{Text, tr};
use crate::job_conflicts::{ActionGroup, Blocker, JobType, blocked_actions, refusal};
use crate::models::{ApiResponse, JobEventPayload};
use chrono::Utc;
use serde::Serialize;
use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Mutex, MutexGuard, PoisonError};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

pub(super) const JOB_EVENT_CHANNEL: &str = "job:event";

/// Jobs that have emitted `job.started` but not yet `job.completed` /
/// `job.failed` (id -> job type), kept up to date by `emit_job_payload`.
/// `install_update` waits for it to empty and holds it while installing: every
/// job emits `job.started` before doing any work, so a job started meanwhile
/// blocks there until the app has restarted instead of being cut short. What
/// the running jobs block comes from `job_conflicts`.
pub(super) static RUNNING_JOBS: Mutex<BTreeMap<String, JobType>> = Mutex::new(BTreeMap::new());

/// The event that carries the blocked action groups (`job_conflicts::blocked_actions`)
/// whenever a blocker comes or goes: a job starts or ends, the USB order edit opens or
/// closes.
pub(super) const JOBS_BLOCKED_EVENT: &str = "jobs:blocked";

pub(super) fn running_jobs() -> MutexGuard<'static, BTreeMap<String, JobType>> {
    RUNNING_JOBS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether the USB Playlists page's order edit is open. Asked before `RUNNING_JOBS` is
/// locked, never while it's held.
pub(super) fn usb_order_edit_open<R: tauri::Runtime, M: Manager<R>>(manager: &M) -> bool {
    manager
        .try_state::<BackendCommands>()
        .is_some_and(|commands| commands.usb_order_edit_open())
}

/// What blocks actions now: the running jobs, and the order edit when it's open.
pub(super) fn blockers(
    jobs: &BTreeMap<String, JobType>,
    usb_order_edit_open: bool,
) -> impl Iterator<Item = Blocker> + '_ {
    jobs.values()
        .map(|job_type| Blocker::Job(*job_type))
        .chain(usb_order_edit_open.then_some(Blocker::UsbOrderEdit))
}

pub(super) fn send_blocked_actions<R: tauri::Runtime>(
    app: &AppHandle<R>,
    blocked: BTreeMap<ActionGroup, String>,
) {
    let _ = app.emit(
        JOBS_BLOCKED_EVENT,
        serde_json::json!({ "blockedActions": blocked }),
    );
}

/// Sends what's blocked now, after the order edit opened or closed outside a job.
pub(super) fn emit_blocked_actions<R: tauri::Runtime>(app: &AppHandle<R>) {
    let edit_open = usb_order_edit_open(app);
    let blocked = blocked_actions(blockers(&running_jobs(), edit_open));
    send_blocked_actions(app, blocked);
}

/// The refusal of `command` while these blockers are on (`job_conflicts`), or `None`.
pub(super) fn blocked_refusal<T: Serialize>(
    command: &str,
    jobs: &BTreeMap<String, JobType>,
    usb_order_edit_open: bool,
) -> Option<ApiResponse<T>> {
    let reason = refusal(command, blockers(jobs, usb_order_edit_open))?;
    Some(ApiResponse::failure(ErrorPayload::new(
        ErrorCode::ValidationError,
        reason,
    )))
}

/// The refusal of a command that something running blocks, for the desktop app's invoke
/// handler to answer with before the command runs.
pub fn refuse_blocked_command<R: tauri::Runtime, M: Manager<R>>(
    manager: &M,
    command: &str,
) -> Option<ApiResponse<()>> {
    let edit_open = usb_order_edit_open(manager);
    blocked_refusal(command, &running_jobs(), edit_open)
}

pub(super) fn panic_message(err: Box<dyn std::any::Any + Send>) -> String {
    if let Some(msg) = err.downcast_ref::<&str>() {
        (*msg).to_string()
    } else if let Some(msg) = err.downcast_ref::<String>() {
        msg.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

pub(super) async fn run_blocking_command<T, F>(label: &str, task: F) -> ApiResponse<T>
where
    T: Send + Serialize + 'static,
    F: FnOnce() -> ApiResponse<T> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(move || panic::catch_unwind(AssertUnwindSafe(task)))
        .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => ApiResponse::failure(ErrorPayload::new(
            ErrorCode::InternalError,
            format!("{label} crashed: {}", panic_message(e)),
        )),
        Err(e) => ApiResponse::failure(ErrorPayload::new(
            ErrorCode::InternalError,
            format!("{label} task failed: {e}"),
        )),
    }
}

#[derive(Clone, Copy)]
pub(super) struct JobIdentity<'a> {
    pub(super) job_id: &'a str,
    pub(super) job_type: JobType,
    pub(super) stage: &'a str,
}

pub(super) fn emit_job_event<R: tauri::Runtime>(
    app: &AppHandle<R>,
    event_name: &str,
    identity: JobIdentity<'_>,
    current: usize,
    total: usize,
    percent: usize,
    message: impl Into<String>,
) {
    let (message, detail) = crate::progress::split_owned(message.into());
    let payload = JobEventPayload {
        event: event_name.to_string(),
        job_id: identity.job_id.to_string(),
        job_type: identity.job_type,
        stage: identity.stage.to_string(),
        current,
        total,
        percent: percent.min(100),
        message,
        detail,
        track_id: None,
        track_title: None,
        file_path: None,
        bpm: None,
        bpm_source_text: None,
        key: None,
        key_source_text: None,
        key_display: None,
        key_color: None,
        artwork_path: None,
        waveform_peaks_path: None,
        waveform_preview: None,
        duration_ms: None,
        track_ready: None,
        track_started: None,
        reanalysis: None,
        failed: None,
        analysis_ready: None,
        error_message: None,
        failed_text: None,
        library_duration_text: None,
        track: None,
        waiting_track_ids: None,
        waiting_tooltip: None,
        warning_text: None,
        timestamp: Utc::now().to_rfc3339(),
    };

    emit_job_payload(app, event_name, payload);
}

pub(super) fn emit_job_payload<R: tauri::Runtime>(
    app: &AppHandle<R>,
    event_name: &str,
    payload: JobEventPayload,
) {
    if matches!(event_name, "job.started" | "job.completed" | "job.failed") {
        let edit_open = usb_order_edit_open(app);
        let blocked = {
            let mut jobs = running_jobs();
            if event_name == "job.started" {
                jobs.insert(payload.job_id.clone(), payload.job_type);
            } else {
                jobs.remove(&payload.job_id);
            }
            blocked_actions(blockers(&jobs, edit_open))
        };
        // Before the job's own event, so its controls are off by the time it shows.
        send_blocked_actions(app, blocked);
    }
    let direct_event_name = event_name.replace('.', ":");
    let _ = app.emit(&direct_event_name, payload.clone());
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit(JOB_EVENT_CHANNEL, payload);
    } else {
        let _ = app.emit(JOB_EVENT_CHANNEL, payload);
    }
}

pub(super) fn emit_job_failed<R: tauri::Runtime>(
    app: &AppHandle<R>,
    identity: JobIdentity<'_>,
    response_message: Option<String>,
) {
    emit_job_event(
        app,
        "job.failed",
        identity,
        0,
        1,
        100,
        response_message.unwrap_or_else(|| tr::job_failed_unknown().render()),
    );
}

pub(super) async fn run_usb_job<T, F>(
    app: &AppHandle,
    job_type: JobType,
    stage: &str,
    started_message: Text,
    completed_message: Text,
    initial_progress: Option<(usize, Text)>,
    task: F,
) -> Result<ApiResponse<T>, String>
where
    T: Serialize + Send + 'static,
    F: FnOnce() -> ApiResponse<T> + Send + 'static,
{
    let job_id = Uuid::now_v7().to_string();
    let identity = JobIdentity {
        job_id: &job_id,
        job_type,
        stage,
    };
    emit_job_event(
        app,
        "job.started",
        identity,
        0,
        1,
        0,
        started_message.render(),
    );
    if let Some((percent, message)) = initial_progress {
        emit_job_event(
            app,
            "job.progress",
            identity,
            0,
            1,
            percent,
            message.render(),
        );
    }

    let response = match tauri::async_runtime::spawn_blocking(task).await {
        Ok(resp) => resp,
        Err(err) => ApiResponse::failure(
            crate::error::BackendError::Internal(format!("{stage} task failed: {err}")).into(),
        ),
    };

    if response.ok {
        emit_job_event(
            app,
            "job.completed",
            identity,
            1,
            1,
            100,
            completed_message.render(),
        );
    } else {
        emit_job_failed(
            app,
            identity,
            response.error.as_ref().map(|e| e.message.clone()),
        );
    }

    Ok(response)
}

pub(super) async fn run_usb_job_with_progress<T, F>(
    app: &AppHandle,
    job_type: JobType,
    stage: &str,
    started_message: Text,
    completed_message: Text,
    task: F,
) -> Result<ApiResponse<T>, String>
where
    T: Serialize + Send + 'static,
    F: FnOnce(Box<dyn crate::progress::ProgressFn + Send>) -> ApiResponse<T> + Send + 'static,
{
    let job_id = Uuid::now_v7().to_string();
    let identity = JobIdentity {
        job_id: &job_id,
        job_type,
        stage,
    };
    let fallback_message = started_message.render();
    emit_job_event(
        app,
        "job.started",
        identity,
        0,
        1,
        0,
        fallback_message.clone(),
    );

    let app_for_task = app.clone();
    let job_id_for_task = job_id.clone();

    let stage_for_task = stage.to_string();
    let response = match tauri::async_runtime::spawn_blocking(move || {
        let progress = Box::new(move |current: usize, total: usize, message: &str| {
            let denom = total.max(1);
            let percent = ((current * 100) / denom).min(100);
            let identity = JobIdentity {
                job_id: &job_id_for_task,
                job_type,
                stage: &stage_for_task,
            };
            // Every job:event carries a non-empty message so the frontend never
            // has to invent stage-specific fallback text.
            let message = if message.trim().is_empty() {
                fallback_message.as_str()
            } else {
                message
            };
            emit_job_event(
                &app_for_task,
                "job.progress",
                identity,
                current,
                denom,
                percent,
                message.to_string(),
            );
        });
        task(progress)
    })
    .await
    {
        Ok(resp) => resp,
        Err(err) => ApiResponse::failure(
            crate::error::BackendError::Internal(format!("{stage} task failed: {err}")).into(),
        ),
    };

    if response.ok {
        emit_job_event(
            app,
            "job.completed",
            identity,
            1,
            1,
            100,
            completed_message.render(),
        );
    } else {
        emit_job_failed(
            app,
            identity,
            response.error.as_ref().map(|e| e.message.clone()),
        );
    }

    Ok(response)
}

/// What's blocked now, once the window has started listening to `jobs:blocked`: a job
/// or an order edit can outlive a reload of the window. Sync: two in-memory reads.
#[tauri::command]
pub fn get_blocked_actions(
    state: State<'_, BackendCommands>,
) -> ApiResponse<crate::models::GetBlockedActionsData> {
    let edit_open = state.usb_order_edit_open();
    let blocked_actions = blocked_actions(blockers(&running_jobs(), edit_open));
    ApiResponse::success(crate::models::GetBlockedActionsData { blocked_actions })
}

/// `RUNNING_JOBS`, locked, once no job but `own_job_id` is running.
pub(super) fn lock_jobs_when_idle(
    own_job_id: &str,
) -> Option<MutexGuard<'static, BTreeMap<String, JobType>>> {
    let jobs = running_jobs();
    jobs.keys().all(|id| id == own_job_id).then_some(jobs)
}
