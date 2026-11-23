//! Transcoding commands: making and removing a track's files in other
//! formats, and where they go.

use super::jobs::{JobIdentity, emit_job_event, emit_job_failed, run_blocking_command};
use crate::commands::BackendCommands;
use crate::i18n::tr;
use crate::job_conflicts::JobType;
use crate::models::ApiResponse;
use tauri::{AppHandle, State};
use uuid::Uuid;

/// Makes transcodes of the selected tracks as a cancellable job
/// (`jobType: "transcode"`), one progress event per finished track.
#[tauri::command]
pub async fn transcode_tracks(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: crate::models::TranscodeTracksRequest,
) -> Result<ApiResponse<crate::models::TranscodeTracksData>, String> {
    let job_id = Uuid::now_v7().to_string();
    let identity = JobIdentity {
        job_id: &job_id,
        job_type: JobType::Transcode,
        stage: "transcode_tracks",
    };
    emit_job_event(
        &app,
        "job.started",
        identity,
        0,
        1,
        0,
        tr::job_transcode_tracks_started().render(),
    );
    let commands = state.inner().clone();
    let app_for_task = app.clone();
    let job_id_for_task = job_id.clone();
    let mut response = match tauri::async_runtime::spawn_blocking(move || {
        commands.transcode_tracks_with_progress(request, |progress| {
            let identity = JobIdentity {
                job_id: &job_id_for_task,
                job_type: JobType::Transcode,
                stage: "transcode_tracks",
            };
            let file_name = std::path::Path::new(&progress.file_path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&progress.file_path);
            let total = progress.total.max(1);
            emit_job_event(
                &app_for_task,
                "job.progress",
                identity,
                progress.current,
                total,
                progress.current * 100 / total,
                crate::progress::count(
                    tr::job_transcode_tracks_progress(file_name).render(),
                    progress.current,
                    progress.total,
                ),
            );
        })
    })
    .await
    {
        Ok(response) => response,
        Err(err) => ApiResponse::failure(
            crate::error::BackendError::Internal(format!("transcode_tracks task failed: {err}"))
                .into(),
        ),
    };
    if let Some(data) = response.data.as_mut() {
        data.job_id = job_id.clone();
        let message = if data.cancelled {
            tr::job_transcode_tracks_cancelled(data.transcoded)
        } else {
            tr::job_transcode_tracks_completed(data.transcoded, data.skipped, data.failed)
        };
        emit_job_event(&app, "job.completed", identity, 1, 1, 100, message.render());
    } else {
        emit_job_failed(
            &app,
            identity,
            response.error.as_ref().map(|e| e.message.clone()),
        );
    }
    Ok(response)
}

#[tauri::command]
pub fn cancel_transcode(state: State<'_, BackendCommands>) -> ApiResponse<()> {
    state.cancel_transcode()
}

#[tauri::command]
pub async fn remove_track_transcodes(
    state: State<'_, BackendCommands>,
    request: crate::models::RemoveTrackTranscodesRequest,
) -> Result<ApiResponse<crate::models::RemoveTrackTranscodesData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("remove_track_transcodes", move || {
        commands.remove_track_transcodes(request)
    })
    .await)
}

#[tauri::command]
pub fn get_transcode_formats(
    state: State<'_, BackendCommands>,
) -> ApiResponse<crate::models::TranscodeFormatsData> {
    state.get_transcode_formats()
}

#[tauri::command]
pub async fn get_transcode_folder(
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<crate::models::TranscodeFolderData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("get_transcode_folder", move || {
        commands.get_transcode_folder()
    })
    .await)
}

#[tauri::command]
pub async fn set_transcode_folder(
    state: State<'_, BackendCommands>,
    request: crate::models::SetTranscodeFolderRequest,
) -> Result<ApiResponse<crate::models::TranscodeFolderData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("set_transcode_folder", move || {
        commands.set_transcode_folder(request)
    })
    .await)
}
