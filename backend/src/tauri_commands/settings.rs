//! App settings commands: UI texts and language, frontend settings, track
//! columns, and data migrations.

use super::jobs::{blocked_refusal, run_blocking_command, run_usb_job_with_progress, running_jobs};
use crate::commands::BackendCommands;
use crate::i18n::tr;
use crate::job_conflicts::JobType;
use crate::models::{
    ApiResponse, GetFrontendSettingsData, RunDataMigrationsData, SetFrontendSettingData,
    SetFrontendSettingRequest,
};
use tauri::{AppHandle, State};

/// The UI's texts, once at startup, before anything renders. Sync: it reads the parsed
/// catalog, no I/O.
#[tauri::command]
pub fn get_ui_texts(
    state: State<'_, BackendCommands>,
) -> ApiResponse<crate::models::GetUiTextsData> {
    state.get_ui_texts()
}

#[tauri::command]
pub async fn get_frontend_settings(
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<GetFrontendSettingsData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("get_frontend_settings", move || {
        commands.get_frontend_settings()
    })
    .await)
}

/// Stays sync (main thread) on purpose: sync commands run in call order, and
/// the UI fires these without awaiting (the accent-hue slider sends one per
/// step), so the last value sent must be the last one saved. It is one small
/// row write.
#[tauri::command]
pub fn set_frontend_setting(
    state: State<'_, BackendCommands>,
    request: SetFrontendSettingRequest,
) -> ApiResponse<SetFrontendSettingData> {
    state.set_frontend_setting(request)
}

#[tauri::command]
pub fn get_track_columns(
    state: State<'_, BackendCommands>,
) -> ApiResponse<crate::models::GetTrackColumnsData> {
    state.get_track_columns()
}

/// Sync (main thread), like `set_frontend_setting`: sync commands run in call order, so of
/// quick changes in a row the last one sent is the last one saved.
#[tauri::command]
pub fn set_track_columns(
    state: State<'_, BackendCommands>,
    request: crate::models::SetTrackColumnsRequest,
) -> ApiResponse<crate::models::SetTrackColumnsData> {
    state.set_track_columns(request)
}

/// Sync, so the job registry can be held until the language is saved: no job starts during
/// the change (it reloads the window, which would lose a running job's progress bar, pause
/// and cancel). A job that started after the invoke handler's check is caught here.
#[tauri::command]
pub fn set_language(
    state: State<'_, BackendCommands>,
    request: crate::models::SetLanguageRequest,
) -> ApiResponse<crate::models::SetLanguageData> {
    let edit_open = state.usb_order_edit_open();
    let jobs = running_jobs();
    if let Some(refused) = blocked_refusal("set_language", &jobs, edit_open) {
        return refused;
    }
    state.set_language(request)
}

/// Runs the pending one-time background data migrations (`migrations.rs`)
/// as a progress-bar job. Called by the frontend once after startup; with
/// nothing pending it returns at once and emits no job events, so normal
/// launches show nothing.
#[tauri::command]
pub async fn run_data_migrations(
    app: AppHandle,
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<RunDataMigrationsData>, String> {
    let commands = state.inner().clone();
    let pending = commands.pending_data_migrations();
    match pending.data.as_ref() {
        Some(ids) if !ids.is_empty() => {}
        Some(_) => return Ok(ApiResponse::success(RunDataMigrationsData::default())),
        None => {
            return Ok(ApiResponse::failure(pending.error.unwrap_or_else(|| {
                crate::error::BackendError::Internal("data migrations unavailable".to_string())
                    .into()
            })));
        }
    }
    run_usb_job_with_progress(
        &app,
        JobType::Migration,
        "migrate",
        tr::job_migrate_started(),
        tr::job_migrate_completed(),
        move |mut progress| {
            commands.run_data_migrations_with_progress(move |c, t, m| progress(c, t, m))
        },
    )
    .await
}
