//! Importing from rekordbox and Mixxx: finding their libraries, scanning
//! them, and importing their playlists and track data.

use super::jobs::{JobIdentity, emit_job_event, emit_job_failed, run_blocking_command};
use crate::commands::BackendCommands;
use crate::i18n::tr;
use crate::job_conflicts::JobType;
use crate::models::{
    ApiResponse, ApplyExternalTrackDataData, ApplyExternalTrackDataRequest,
    DetectExternalMixxxDbData, DetectExternalRekordboxDbData, ImportExternalPlaylistsData,
    ImportExternalPlaylistsRequest, ListExternalPlaylistsData, ListExternalPlaylistsRequest,
    ScanLibraryData, ScanMixxxDbRequest, ScanRekordboxDbRequest,
};
use tauri::{AppHandle, State};
use uuid::Uuid;

#[tauri::command]
pub async fn scan_rekordbox_db(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: ScanRekordboxDbRequest,
) -> Result<ApiResponse<ScanLibraryData>, String> {
    let job_id = Uuid::now_v7().to_string();
    let identity = JobIdentity {
        job_id: &job_id,
        job_type: JobType::Scan,
        stage: "scan_rekordbox_db",
    };
    emit_job_event(
        &app,
        "job.started",
        identity,
        0,
        1,
        0,
        tr::ui_library_import_rekordbox_starting().render(),
    );
    emit_job_event(
        &app,
        "job.progress",
        identity,
        0,
        1,
        30,
        tr::job_scan_rekordbox_db_progress().render(),
    );

    let commands = state.inner().clone();
    let response =
        match tauri::async_runtime::spawn_blocking(move || commands.scan_rekordbox_db(request))
            .await
        {
            Ok(resp) => resp,
            Err(err) => ApiResponse::failure(
                crate::error::BackendError::Internal(format!(
                    "scan_rekordbox_db task failed: {err}"
                ))
                .into(),
            ),
        };
    if response.ok {
        let summary = response
            .data
            .as_ref()
            .map(|d| tr::job_scan_rekordbox_db_completed_counts(d.indexed, d.updated).render())
            .unwrap_or_else(|| tr::job_scan_rekordbox_db_completed().render());
        emit_job_event(&app, "job.completed", identity, 1, 1, 100, summary);
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
pub async fn scan_mixxx_db(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: ScanMixxxDbRequest,
) -> Result<ApiResponse<ScanLibraryData>, String> {
    let job_id = Uuid::now_v7().to_string();
    let identity = JobIdentity {
        job_id: &job_id,
        job_type: JobType::Scan,
        stage: "scan_mixxx_db",
    };
    emit_job_event(
        &app,
        "job.started",
        identity,
        0,
        1,
        0,
        tr::ui_library_import_mixxx_starting().render(),
    );
    emit_job_event(
        &app,
        "job.progress",
        identity,
        0,
        1,
        30,
        tr::job_scan_mixxx_db_progress().render(),
    );

    let commands = state.inner().clone();
    let response =
        match tauri::async_runtime::spawn_blocking(move || commands.scan_mixxx_db(request)).await {
            Ok(resp) => resp,
            Err(err) => ApiResponse::failure(
                crate::error::BackendError::Internal(format!("scan_mixxx_db task failed: {err}"))
                    .into(),
            ),
        };
    if response.ok {
        let summary = response
            .data
            .as_ref()
            .map(|d| tr::job_scan_mixxx_db_completed_counts(d.indexed, d.updated).render())
            .unwrap_or_else(|| tr::job_scan_mixxx_db_completed().render());
        emit_job_event(&app, "job.completed", identity, 1, 1, 100, summary);
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
pub async fn detect_external_rekordbox_db(
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<DetectExternalRekordboxDbData>, String> {
    let commands = state.inner().clone();
    Ok(
        run_blocking_command("detect_external_rekordbox_db", move || {
            commands.detect_external_rekordbox_db()
        })
        .await,
    )
}

#[tauri::command]
pub async fn list_mixxx_playlists(
    state: State<'_, BackendCommands>,
    request: ListExternalPlaylistsRequest,
) -> Result<ApiResponse<ListExternalPlaylistsData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("list_mixxx_playlists", move || {
        commands.list_mixxx_playlists(request)
    })
    .await)
}

#[tauri::command]
pub async fn list_rekordbox_playlists(
    state: State<'_, BackendCommands>,
    request: ListExternalPlaylistsRequest,
) -> Result<ApiResponse<ListExternalPlaylistsData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("list_rekordbox_playlists", move || {
        commands.list_rekordbox_playlists(request)
    })
    .await)
}

#[tauri::command]
pub async fn import_external_playlists(
    state: State<'_, BackendCommands>,
    request: ImportExternalPlaylistsRequest,
) -> Result<ApiResponse<crate::models::WithPlaylists<ImportExternalPlaylistsData>>, String> {
    let commands = state.inner().clone();
    Ok(
        tauri::async_runtime::spawn_blocking(move || commands.import_external_playlists(request))
            .await
            .unwrap_or_else(|err| {
                ApiResponse::failure(
                    crate::error::BackendError::Internal(format!(
                        "import_external_playlists task failed: {err}"
                    ))
                    .into(),
                )
            }),
    )
}

#[tauri::command]
pub async fn apply_external_track_data(
    state: State<'_, BackendCommands>,
    request: ApplyExternalTrackDataRequest,
) -> Result<ApiResponse<ApplyExternalTrackDataData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("apply_external_track_data", move || {
        commands.apply_external_track_data(request)
    })
    .await)
}

#[tauri::command]
pub async fn detect_external_mixxx_db(
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<DetectExternalMixxxDbData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("detect_external_mixxx_db", move || {
        commands.detect_external_mixxx_db()
    })
    .await)
}
