#![cfg(feature = "tauri")]

use std::panic::{self, AssertUnwindSafe};
use std::thread;

use chrono::Utc;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

use crate::commands::BackendCommands;
use crate::error::{ErrorCode, ErrorPayload};
use crate::models::{
    AddLibrarySelectionToPlaylistRequest, AddTrackCandidatesToPlaylistData,
    AddTrackCandidatesToPlaylistRequest, AddTracksToPlaylistData, AddTracksToPlaylistRequest,
    AnalyzeNewTracksData, AnalyzeNewTracksRequest, ApiResponse, ListMatchingTrackIdsData,
    ListMatchingTrackIdsRequest,
    BrowseSourceFilesData, BrowseSourceFilesRequest, CheckSourceRootsData, CheckSourceRootsRequest,
    CreatePlaylistData, CreatePlaylistRequest, DeletePlaylistData, DeletePlaylistRequest,
    DeleteUsbBackupData, DeleteUsbBackupRequest, DetectExternalMasterDbData, ExportToUsbData,
    ExportToUsbRequest, FetchUsbHistoriesData, FetchUsbHistoriesRequest, FetchUsbPlaylistsData,
    FetchUsbPlaylistsRequest, FetchUsbTracksData, FetchUsbTracksRequest, GetFrontendSettingsData,
    GetPlaylistTracksData,
    GetPlaylistTracksRequest, GetTrackDetailRequest, GetTracksByIdsData, GetTracksByIdsRequest,
    GetUsbDeviceNameData,
    GetUsbDeviceNameRequest, GetUsbPlayerMenuConfigData, GetUsbPlayerMenuConfigRequest,
    InitializeUsbData, InitializeUsbRequest, InspectUsbTrackData, InspectUsbTrackRequest,
    InspectUsbTracksData, InspectUsbTracksRequest, JobEventPayload, ListPlaylistsData,
    ListTracksData, ListTracksRequest, ListUsbBackupsData, ListUsbBackupsRequest,
    ListUsbDevicesData, MaterializeSourceTrackData, MaterializeSourceTrackRequest,
    MergeUsbPlaceholderTracksData, PlayResolvedTrackData, PlayResolvedTrackRequest, PlayTrackData,
    PlayTrackRequest, PlaybackEventPayload, PlaybackPreflightData, PlaybackPreflightRequest,
    PlaybackStatusData, PruneUsbDeviceData, PruneUsbDeviceRequest, RelocateSourceRootData,
    RelocateSourceRootRequest, RemoveTracksBySourceRootsData, RemoveTracksBySourceRootsRequest,
    RemoveTracksFromPlaylistData, RemoveTracksFromPlaylistRequest, RemoveUsbPlaylistData,
    RemoveUsbPlaylistRequest, RenamePlaylistData, RenamePlaylistRequest, ReorderPlaylistTracksData,
    ReorderPlaylistTracksRequest, ReorderUsbPlaylistsData, ReorderUsbPlaylistsRequest,
    RepairUsbDiagnosticsData, RepairUsbDiagnosticsRequest, ResolvePlaybackSourceData,
    ResolvePlaybackSourceRequest, ResolveTrackIdentityData, ResolveTrackIdentityRequest,
    RestoreUsbBackupData, RestoreUsbBackupRequest, RunUsbDiagnosticsData, RunUsbDiagnosticsRequest,
    RunUsbParityReportData, RunUsbParityReportRequest, ScanLibraryData, ScanLibraryRequest,
    SaveTrackAnalysisEditsData, SaveTrackAnalysisEditsRequest, ScanMasterDbRequest, SearchTracksData,
    SearchTracksRequest, SetAnalysisPausedData,
    SetAnalysisPausedRequest, SetFrontendSettingData, SetFrontendSettingRequest,
    SetUsbDeviceNameData, SetUsbDeviceNameRequest, StopPlaybackData, TrackDetail,
    UpdateUsbPlayerMenuConfigData,
    UpdateUsbPlayerMenuConfigRequest, ValidateUsbRootData, ValidateUsbRootRequest,
};

const JOB_EVENT_CHANNEL: &str = "job:event";
const PLAYBACK_EVENT_CHANNEL: &str = "playback:event";
const ESSENTIA_DOWNLOAD_EVENT: &str = "essentia_download_progress";

/// Cancellation flag for in-progress Essentia download. Managed Tauri state.
pub struct EssentiaDownloadCancel(pub std::sync::Arc<std::sync::atomic::AtomicBool>);

fn panic_message(err: Box<dyn std::any::Any + Send>) -> String {
    if let Some(msg) = err.downcast_ref::<&str>() {
        (*msg).to_string()
    } else if let Some(msg) = err.downcast_ref::<String>() {
        msg.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

fn playback_panic_response<T: serde::Serialize>(
    app: &AppHandle,
    action: &str,
    panic_text: String,
) -> ApiResponse<T> {
    let message = format!("native playback {action} crashed: {panic_text}");
    let _ = app.emit(
        "backend:log",
        serde_json::json!({
            "level": "error",
            "source": "playback",
            "code": "playback.crash",
            "message": message,
            "timestamp": Utc::now().to_rfc3339(),
        }),
    );
    ApiResponse::failure(ErrorPayload {
        code: ErrorCode::InternalError,
        message,
        details: None,
    })
}

async fn run_playback_blocking<T, F>(
    app: &AppHandle,
    action: &str,
    task: F,
) -> Result<ApiResponse<T>, ApiResponse<T>>
where
    T: Send + Serialize + 'static,
    F: FnOnce() -> ApiResponse<T> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(move || panic::catch_unwind(AssertUnwindSafe(task)))
        .await
    {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => {
            let panic_text = panic_message(e);
            emit_playback_event(
                app,
                PlaybackEvent::Error(Some(format!(
                    "Native playback {action} crashed: {panic_text}"
                ))),
            );
            Err(playback_panic_response(app, action, panic_text))
        }
        Err(e) => {
            let message = format!("native playback {action} task failed: {e}");
            emit_playback_event(app, PlaybackEvent::Error(Some(message.clone())));
            Err(ApiResponse::failure(ErrorPayload {
                code: ErrorCode::InternalError,
                message,
                details: None,
            }))
        }
    }
}

async fn run_blocking_command<T, F>(label: &str, task: F) -> ApiResponse<T>
where
    T: Send + Serialize + 'static,
    F: FnOnce() -> ApiResponse<T> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(move || panic::catch_unwind(AssertUnwindSafe(task)))
        .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => ApiResponse::failure(ErrorPayload {
            code: ErrorCode::InternalError,
            message: format!("{label} crashed: {}", panic_message(e)),
            details: None,
        }),
        Err(e) => ApiResponse::failure(ErrorPayload {
            code: ErrorCode::InternalError,
            message: format!("{label} task failed: {e}"),
            details: None,
        }),
    }
}

#[derive(Clone, Copy)]
struct JobIdentity<'a> {
    job_id: &'a str,
    job_type: &'a str,
    stage: &'a str,
}

fn emit_job_event<R: tauri::Runtime>(
    app: &AppHandle<R>,
    event_name: &str,
    identity: JobIdentity<'_>,
    current: usize,
    total: usize,
    percent: usize,
    message: impl Into<String>,
) {
    let payload = JobEventPayload {
        event: event_name.to_string(),
        job_id: identity.job_id.to_string(),
        job_type: identity.job_type.to_string(),
        stage: identity.stage.to_string(),
        current,
        total,
        percent: percent.min(100),
        message: message.into(),
        track_id: None,
        track_title: None,
        file_path: None,
        bpm: None,
        bpm_analyzer: None,
        key: None,
        artwork_path: None,
        waveform_peaks_path: None,
        waveform_preview: None,
        duration_ms: None,
        track_ready: None,
        failed: None,
        analysis_ready: None,
        error_message: None,
        library_total_duration_ms: None,
        library_duration_unknown_count: None,
        timestamp: Utc::now().to_rfc3339(),
    };

    emit_job_payload(app, event_name, payload);
}

fn emit_job_payload<R: tauri::Runtime>(
    app: &AppHandle<R>,
    event_name: &str,
    payload: JobEventPayload,
) {
    let direct_event_name = event_name.replace('.', ":");
    let _ = app.emit(&direct_event_name, payload.clone());
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit(JOB_EVENT_CHANNEL, payload);
    } else {
        let _ = app.emit(JOB_EVENT_CHANNEL, payload);
    }
}

fn emit_job_event_with_track<R: tauri::Runtime>(
    app: &AppHandle<R>,
    event_name: &str,
    identity: JobIdentity<'_>,
    message: impl Into<String>,
    progress: &crate::service::analysis::AnalyzeTrackProgress,
) {
    let total = progress.total.max(1);
    let percent = (progress.current * 100)
        .checked_div(total)
        .unwrap_or(100)
        .min(100);
    let payload = JobEventPayload {
        event: event_name.to_string(),
        job_id: identity.job_id.to_string(),
        job_type: identity.job_type.to_string(),
        stage: identity.stage.to_string(),
        current: progress.current,
        total,
        percent,
        message: message.into(),
        track_id: Some(progress.track_id.clone()),
        track_title: Some(progress.track_title.clone()),
        file_path: Some(progress.file_path.clone()),
        bpm: progress.bpm,
        bpm_analyzer: progress.bpm_analyzer.clone(),
        key: progress.key.clone(),
        artwork_path: progress.artwork_path.clone(),
        waveform_peaks_path: progress.waveform_peaks_path.clone(),
        waveform_preview: progress.waveform_preview.clone(),
        duration_ms: progress.duration_ms,
        track_ready: Some(progress.track_ready),
        failed: Some(progress.failed),
        analysis_ready: Some(progress.analysis_ready),
        error_message: progress.error_message.clone(),
        library_total_duration_ms: progress.library_total_duration_ms,
        library_duration_unknown_count: progress.library_duration_unknown_count,
        timestamp: Utc::now().to_rfc3339(),
    };

    emit_job_payload(app, event_name, payload);
}

fn emit_job_failed<R: tauri::Runtime>(
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
        response_message.unwrap_or_else(|| "Operation failed".to_string()),
    );
}

async fn run_usb_job<T, F>(
    app: &AppHandle,
    job_type: &str,
    stage: &str,
    started_message: &str,
    completed_message: &str,
    initial_progress: Option<(usize, String)>,
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
    emit_job_event(app, "job.started", identity, 0, 1, 0, started_message);
    if let Some((percent, message)) = initial_progress {
        emit_job_event(app, "job.progress", identity, 0, 1, percent, message);
    }

    let response = match tauri::async_runtime::spawn_blocking(task).await {
        Ok(resp) => resp,
        Err(err) => ApiResponse::failure(
            crate::error::BackendError::Internal(format!("{stage} task failed: {err}")).into(),
        ),
    };

    if response.ok {
        emit_job_event(app, "job.completed", identity, 1, 1, 100, completed_message);
    } else {
        emit_job_failed(
            app,
            identity,
            response.error.as_ref().map(|e| e.message.clone()),
        );
    }

    Ok(response)
}

async fn run_usb_job_with_progress<T, F>(
    app: &AppHandle,
    job_type: &str,
    stage: &str,
    started_message: &str,
    completed_message: &str,
    task: F,
) -> Result<ApiResponse<T>, String>
where
    T: Serialize + Send + 'static,
    F: FnOnce(Box<dyn FnMut(usize, usize, &str) + Send>) -> ApiResponse<T> + Send + 'static,
{
    let job_id = Uuid::now_v7().to_string();
    let identity = JobIdentity {
        job_id: &job_id,
        job_type,
        stage,
    };
    emit_job_event(app, "job.started", identity, 0, 1, 0, started_message);

    let app_for_task = app.clone();
    let job_id_for_task = job_id.clone();
    let job_type_for_task = job_type.to_string();
    let stage_for_task = stage.to_string();
    let fallback_message = started_message.to_string();
    let response = match tauri::async_runtime::spawn_blocking(move || {
        let progress = Box::new(move |current: usize, total: usize, message: &str| {
            let denom = total.max(1);
            let percent = ((current * 100) / denom).min(100);
            let identity = JobIdentity {
                job_id: &job_id_for_task,
                job_type: &job_type_for_task,
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
        emit_job_event(app, "job.completed", identity, 1, 1, 100, completed_message);
    } else {
        emit_job_failed(
            app,
            identity,
            response.error.as_ref().map(|e| e.message.clone()),
        );
    }

    Ok(response)
}

/// The three playback notifications the frontend listens for. Each carries only
/// the fields that are meaningful for it -- the flat `PlaybackEventPayload` wire
/// shape (with its inert `None`/`false`/`0` columns) is assembled here, once.
enum PlaybackEvent {
    /// `playback.started`, or `playback.seeked` when `is_seek`.
    Started {
        is_seek: bool,
        path: String,
        playing: bool,
        position_ms: u64,
        duration_ms: Option<u64>,
        track_id: Option<String>,
    },
    Stopped {
        path: Option<String>,
        duration_ms: Option<u64>,
    },
    Error(Option<String>),
}

fn emit_playback_event<R: tauri::Runtime>(app: &AppHandle<R>, event: PlaybackEvent) {
    let payload = match event {
        PlaybackEvent::Started {
            is_seek,
            path,
            playing,
            position_ms,
            duration_ms,
            track_id,
        } => PlaybackEventPayload {
            event: if is_seek { "playback.seeked" } else { "playback.started" }.to_string(),
            path: Some(path),
            playing,
            position_ms,
            duration_ms,
            message: None,
            track_id,
            timestamp: Utc::now().to_rfc3339(),
        },
        PlaybackEvent::Stopped { path, duration_ms } => PlaybackEventPayload {
            event: "playback.stopped".to_string(),
            path,
            playing: false,
            position_ms: 0,
            duration_ms,
            message: None,
            track_id: None,
            timestamp: Utc::now().to_rfc3339(),
        },
        PlaybackEvent::Error(message) => PlaybackEventPayload {
            event: "playback.error".to_string(),
            path: None,
            playing: false,
            position_ms: 0,
            duration_ms: None,
            message,
            track_id: None,
            timestamp: Utc::now().to_rfc3339(),
        },
    };

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit(PLAYBACK_EVENT_CHANNEL, payload.clone());
    }
    let _ = app.emit(PLAYBACK_EVENT_CHANNEL, payload);
}

/// Relays natural end-of-track notifications from the playback worker thread to the
/// frontend. Purely reactive — blocks on `recv()`, no polling loop — the worker thread
/// itself (`player.rs`) is what detects a sink emptying on its own, using the same
/// serialized loop that processes explicit Play/Stop commands, so this relay can never
/// race an explicit stop the way a separate status-polling thread could.
pub fn start_playback_transition_relay(
    app: AppHandle,
    transitions: std::sync::mpsc::Receiver<crate::player::PlaybackTransition>,
) {
    thread::spawn(move || {
        while let Ok(transition) = transitions.recv() {
            emit_playback_event(
                &app,
                PlaybackEvent::Stopped {
                    path: transition.path,
                    duration_ms: transition.duration_ms,
                },
            );
        }
        // The channel only disconnects if the playback worker thread exited (panicked or
        // the process is shutting down) — natural end-of-track detection is silently gone
        // for the rest of the session, so this needs to be visible, not just dropped.
        let _ = app.emit(
            "backend:log",
            serde_json::json!({
                "level": "warn",
                "source": "playback",
                "code": "playback.transition_relay_stopped",
                "message": "playback transition relay stopped: worker channel disconnected; natural end-of-track detection is no longer active",
                "timestamp": Utc::now().to_rfc3339(),
            }),
        );
    });
}

#[tauri::command]
pub async fn scan_library(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: ScanLibraryRequest,
) -> Result<ApiResponse<ScanLibraryData>, String> {
    let job_id = Uuid::now_v7().to_string();
    let identity = JobIdentity {
        job_id: &job_id,
        job_type: "scan",
        stage: "scan_library",
    };
    emit_job_event(
        &app,
        "job.started",
        identity,
        0,
        1,
        0,
        "Scanning media library",
    );
    emit_job_event(
        &app,
        "job.progress",
        identity,
        0,
        1,
        30,
        "Discovering audio files",
    );

    let commands = state.inner().clone();
    let response =
        match tauri::async_runtime::spawn_blocking(move || commands.scan_library(request)).await {
            Ok(resp) => resp,
            Err(err) => ApiResponse::failure(
                crate::error::BackendError::Internal(format!("scan_library task failed: {err}"))
                    .into(),
            ),
        };
    if response.ok {
        let summary = response
            .data
            .as_ref()
            .map(|d| {
                format!(
                    "Library scan completed: indexed {}, updated {}, removed {}",
                    d.indexed, d.updated, d.removed
                )
            })
            .unwrap_or_else(|| "Library scan completed".to_string());
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
pub async fn scan_master_db(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: ScanMasterDbRequest,
) -> Result<ApiResponse<ScanLibraryData>, String> {
    let job_id = Uuid::now_v7().to_string();
    let identity = JobIdentity {
        job_id: &job_id,
        job_type: "scan",
        stage: "scan_master_db",
    };
    emit_job_event(
        &app,
        "job.started",
        identity,
        0,
        1,
        0,
        "Importing from desktop library",
    );
    emit_job_event(
        &app,
        "job.progress",
        identity,
        0,
        1,
        30,
        "Reading desktop library database",
    );

    let commands = state.inner().clone();
    let response = match tauri::async_runtime::spawn_blocking(move || {
        commands.scan_master_db(request)
    })
    .await
    {
        Ok(resp) => resp,
        Err(err) => ApiResponse::failure(
            crate::error::BackendError::Internal(format!("scan_master_db task failed: {err}"))
                .into(),
        ),
    };
    if response.ok {
        let summary = response
            .data
            .as_ref()
            .map(|d| {
                format!(
                    "Desktop library import completed: indexed {}, updated {}",
                    d.indexed, d.updated
                )
            })
            .unwrap_or_else(|| "Desktop library import completed".to_string());
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
pub fn search_tracks(
    state: State<'_, BackendCommands>,
    request: SearchTracksRequest,
) -> ApiResponse<SearchTracksData> {
    state.search_tracks(request)
}

#[tauri::command]
pub fn list_tracks(
    state: State<'_, BackendCommands>,
    request: ListTracksRequest,
) -> ApiResponse<ListTracksData> {
    state.list_tracks(request)
}

#[tauri::command]
pub fn browse_source_files(
    state: State<'_, BackendCommands>,
    request: BrowseSourceFilesRequest,
) -> ApiResponse<BrowseSourceFilesData> {
    state.browse_source_files(request)
}

#[tauri::command]
pub fn check_source_roots(
    state: State<'_, BackendCommands>,
    request: CheckSourceRootsRequest,
) -> ApiResponse<CheckSourceRootsData> {
    state.check_source_roots(request)
}

#[tauri::command]
pub fn materialize_source_track(
    state: State<'_, BackendCommands>,
    request: MaterializeSourceTrackRequest,
) -> ApiResponse<MaterializeSourceTrackData> {
    state.materialize_source_track(request)
}

#[tauri::command]
pub fn resolve_track_identity(
    state: State<'_, BackendCommands>,
    request: ResolveTrackIdentityRequest,
) -> ApiResponse<ResolveTrackIdentityData> {
    state.resolve_track_identity(request)
}

#[tauri::command]
pub fn remove_tracks_by_source_roots(
    state: State<'_, BackendCommands>,
    request: RemoveTracksBySourceRootsRequest,
) -> ApiResponse<RemoveTracksBySourceRootsData> {
    state.remove_tracks_by_source_roots(request)
}

#[tauri::command]
pub fn get_track_detail(
    state: State<'_, BackendCommands>,
    request: GetTrackDetailRequest,
) -> ApiResponse<TrackDetail> {
    state.get_track_detail(request)
}

#[tauri::command]
pub fn save_track_analysis_edits(
    state: State<'_, BackendCommands>,
    request: SaveTrackAnalysisEditsRequest,
) -> ApiResponse<SaveTrackAnalysisEditsData> {
    state.save_track_analysis_edits(request)
}

#[tauri::command]
pub fn relocate_source_root(
    state: State<'_, BackendCommands>,
    request: RelocateSourceRootRequest,
) -> ApiResponse<RelocateSourceRootData> {
    state.relocate_source_root(request)
}

#[tauri::command]
pub fn get_tracks_by_ids_with_previews(
    state: State<'_, BackendCommands>,
    request: GetTracksByIdsRequest,
) -> ApiResponse<GetTracksByIdsData> {
    state.get_tracks_by_ids_with_previews(request)
}

#[tauri::command]
pub fn resolve_playback_source(
    state: State<'_, BackendCommands>,
    request: ResolvePlaybackSourceRequest,
) -> ApiResponse<ResolvePlaybackSourceData> {
    state.resolve_playback_source(request)
}

#[tauri::command]
pub fn create_playlist(
    state: State<'_, BackendCommands>,
    request: CreatePlaylistRequest,
) -> ApiResponse<CreatePlaylistData> {
    state.create_playlist(request)
}

#[tauri::command]
pub fn rename_playlist(
    state: State<'_, BackendCommands>,
    request: RenamePlaylistRequest,
) -> ApiResponse<RenamePlaylistData> {
    state.rename_playlist(request)
}

#[tauri::command]
pub fn delete_playlist(
    state: State<'_, BackendCommands>,
    request: DeletePlaylistRequest,
) -> ApiResponse<DeletePlaylistData> {
    state.delete_playlist(request)
}

#[tauri::command]
pub fn list_playlists(state: State<'_, BackendCommands>) -> ApiResponse<ListPlaylistsData> {
    state.list_playlists()
}

#[tauri::command]
pub fn get_playlist_tracks(
    state: State<'_, BackendCommands>,
    request: GetPlaylistTracksRequest,
) -> ApiResponse<GetPlaylistTracksData> {
    state.get_playlist_tracks(request)
}

#[tauri::command]
pub fn add_tracks_to_playlist(
    state: State<'_, BackendCommands>,
    request: AddTracksToPlaylistRequest,
) -> ApiResponse<AddTracksToPlaylistData> {
    state.add_tracks_to_playlist(request)
}

#[tauri::command]
pub fn add_track_candidates_to_playlist(
    state: State<'_, BackendCommands>,
    request: AddTrackCandidatesToPlaylistRequest,
) -> ApiResponse<AddTrackCandidatesToPlaylistData> {
    state.add_track_candidates_to_playlist(request)
}

#[tauri::command]
pub fn list_matching_track_ids(
    state: State<'_, BackendCommands>,
    request: ListMatchingTrackIdsRequest,
) -> ApiResponse<ListMatchingTrackIdsData> {
    state.list_matching_track_ids(request)
}

#[tauri::command]
pub fn add_library_selection_to_playlist(
    state: State<'_, BackendCommands>,
    request: AddLibrarySelectionToPlaylistRequest,
) -> ApiResponse<AddTracksToPlaylistData> {
    state.add_library_selection_to_playlist(request)
}

#[tauri::command]
pub fn remove_tracks_from_playlist(
    state: State<'_, BackendCommands>,
    request: RemoveTracksFromPlaylistRequest,
) -> ApiResponse<RemoveTracksFromPlaylistData> {
    state.remove_tracks_from_playlist(request)
}

#[tauri::command]
pub fn refresh_playlist_export_status(
    state: State<'_, BackendCommands>,
    request: crate::models::RefreshPlaylistExportStatusRequest,
) -> ApiResponse<crate::models::RefreshPlaylistExportStatusData> {
    state.refresh_playlist_export_status(request)
}

#[tauri::command]
pub fn reorder_playlist_tracks(
    state: State<'_, BackendCommands>,
    request: ReorderPlaylistTracksRequest,
) -> ApiResponse<ReorderPlaylistTracksData> {
    state.reorder_playlist_tracks(request)
}

#[tauri::command]
pub fn get_frontend_settings(
    state: State<'_, BackendCommands>,
) -> ApiResponse<GetFrontendSettingsData> {
    state.get_frontend_settings()
}

#[tauri::command]
pub fn set_frontend_setting(
    state: State<'_, BackendCommands>,
    request: SetFrontendSettingRequest,
) -> ApiResponse<SetFrontendSettingData> {
    state.set_frontend_setting(request)
}

#[tauri::command]
pub async fn validate_usb_root(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: ValidateUsbRootRequest,
) -> Result<ApiResponse<ValidateUsbRootData>, String> {
    let commands = state.inner().clone();
    run_usb_job(
        &app,
        "usb_read",
        "validate_usb_root",
        "USB: Validating root",
        "USB: Root validation complete",
        Some((30, "USB: Checking folders and permissions".to_string())),
        move || commands.validate_usb_root(request),
    )
    .await
}

#[tauri::command]
pub fn list_usb_devices(state: State<'_, BackendCommands>) -> ApiResponse<ListUsbDevicesData> {
    state.list_usb_devices()
}

#[tauri::command]
pub fn prune_usb_device(
    state: State<'_, BackendCommands>,
    request: PruneUsbDeviceRequest,
) -> ApiResponse<PruneUsbDeviceData> {
    state.prune_usb_device(request)
}

#[tauri::command]
pub fn get_usb_device_name(
    state: State<'_, BackendCommands>,
    request: GetUsbDeviceNameRequest,
) -> ApiResponse<GetUsbDeviceNameData> {
    state.get_usb_device_name(request)
}

#[tauri::command]
pub fn set_usb_device_name(
    state: State<'_, BackendCommands>,
    request: SetUsbDeviceNameRequest,
) -> ApiResponse<SetUsbDeviceNameData> {
    state.set_usb_device_name(request)
}

#[tauri::command]
pub fn list_usb_backups(
    state: State<'_, BackendCommands>,
    request: ListUsbBackupsRequest,
) -> ApiResponse<ListUsbBackupsData> {
    state.list_usb_backups(request)
}

#[tauri::command]
pub fn restore_usb_backup(
    state: State<'_, BackendCommands>,
    request: RestoreUsbBackupRequest,
) -> ApiResponse<RestoreUsbBackupData> {
    state.restore_usb_backup(request)
}

#[tauri::command]
pub fn delete_usb_backup(
    state: State<'_, BackendCommands>,
    request: DeleteUsbBackupRequest,
) -> ApiResponse<DeleteUsbBackupData> {
    state.delete_usb_backup(request)
}

#[tauri::command]
pub fn merge_orphaned_usb_placeholder_tracks(
    state: State<'_, BackendCommands>,
) -> ApiResponse<MergeUsbPlaceholderTracksData> {
    state.merge_orphaned_usb_placeholder_tracks()
}

#[tauri::command]
pub async fn fetch_usb_playlists(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: FetchUsbPlaylistsRequest,
) -> Result<ApiResponse<FetchUsbPlaylistsData>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        "usb_read",
        "fetch_usb_playlists",
        "USB: Reading playlists",
        "USB: Playlist read complete",
        move |mut progress| {
            commands.fetch_usb_playlists_with_progress(request, move |c, t, m| {
                progress(c, t, m);
            })
        },
    )
    .await
}

#[tauri::command]
pub async fn fetch_usb_histories(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: FetchUsbHistoriesRequest,
) -> Result<ApiResponse<FetchUsbHistoriesData>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        "usb_read",
        "fetch_usb_histories",
        "USB: Reading history",
        "USB: History read complete",
        move |mut progress| {
            commands.fetch_usb_histories_with_progress(request, move |c, t, m| {
                progress(c, t, m);
            })
        },
    )
    .await
}

#[tauri::command]
pub async fn get_usb_player_menu_config(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: GetUsbPlayerMenuConfigRequest,
) -> Result<ApiResponse<GetUsbPlayerMenuConfigData>, String> {
    let commands = state.inner().clone();
    run_usb_job(
        &app,
        "usb_read",
        "get_usb_player_menu_config",
        "USB: Loading player menu",
        "USB: Player menu loaded",
        Some((40, "USB: Reading menu configuration".to_string())),
        move || commands.get_usb_player_menu_config(request),
    )
    .await
}

#[tauri::command]
pub async fn update_usb_player_menu_config(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: UpdateUsbPlayerMenuConfigRequest,
) -> Result<ApiResponse<UpdateUsbPlayerMenuConfigData>, String> {
    let commands = state.inner().clone();
    run_usb_job(
        &app,
        "usb_write",
        "update_usb_player_menu_config",
        "USB: Updating player menu",
        "USB: Player menu updated",
        Some((45, "USB: Writing menu configuration".to_string())),
        move || commands.update_usb_player_menu_config(request),
    )
    .await
}

#[tauri::command]
pub async fn sync_usb_player_menu_edb_to_pdb(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: GetUsbPlayerMenuConfigRequest,
) -> Result<ApiResponse<UpdateUsbPlayerMenuConfigData>, String> {
    let commands = state.inner().clone();
    run_usb_job(
        &app,
        "usb_write",
        "sync_usb_player_menu_edb_to_pdb",
        "USB: Fixing PDB sync",
        "USB: PDB synced to active menu",
        Some((45, "USB: Writing PDB from active menu".to_string())),
        move || commands.sync_usb_player_menu_edb_to_pdb(request),
    )
    .await
}

#[tauri::command]
pub async fn remove_usb_playlist(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: RemoveUsbPlaylistRequest,
) -> Result<ApiResponse<RemoveUsbPlaylistData>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        "usb_write",
        "remove_usb_playlist",
        "USB: Removing playlist",
        "USB: Playlist removal complete",
        move |mut progress| {
            commands.remove_usb_playlist_with_progress(request, move |c, t, m| {
                progress(c, t, m);
            })
        },
    )
    .await
}

#[tauri::command]
pub async fn reorder_usb_playlists(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: ReorderUsbPlaylistsRequest,
) -> Result<ApiResponse<ReorderUsbPlaylistsData>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        "usb_write",
        "reorder_usb_playlists",
        "USB: Saving playlist order",
        "USB: Playlist order saved",
        move |mut progress| {
            commands.reorder_usb_playlists_with_progress(request, move |c, t, m| {
                progress(c, t, m);
            })
        },
    )
    .await
}

#[tauri::command]
pub async fn play_track_native(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: PlayTrackRequest,
) -> Result<ApiResponse<PlayTrackData>, String> {
    let commands = state.inner().clone();
    let is_seek = request.start_offset_ms.unwrap_or(0) > 0
        || request
            .start_ratio
            .map(|ratio| ratio > 0.0)
            .unwrap_or(false);
    let request_for_call = request.clone();
    let response = match run_playback_blocking(&app, "start", move || {
        commands.play_track_native(request_for_call)
    })
    .await
    {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    if let Some(data) = response.data.as_ref() {
        emit_playback_event(
            &app,
            PlaybackEvent::Started {
                is_seek,
                path: data.path.clone(),
                playing: data.playing,
                position_ms: data.position_ms,
                duration_ms: data.duration_ms,
                // legacy path -- `PlayTrackData` carries no resolved track id
                track_id: None,
            },
        );
    } else {
        emit_playback_event(
            &app,
            PlaybackEvent::Error(response.error.as_ref().map(|e| e.message.clone())),
        );
    }
    Ok(response)
}

#[tauri::command]
pub async fn play_resolved_track(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: PlayResolvedTrackRequest,
) -> Result<ApiResponse<PlayResolvedTrackData>, String> {
    let commands = state.inner().clone();
    let is_seek = request.start_offset_ms.unwrap_or(0) > 0
        || request
            .start_ratio
            .map(|ratio| ratio > 0.0)
            .unwrap_or(false);
    let request_for_call = request.clone();
    let response = match run_playback_blocking(&app, "start", move || {
        commands.play_resolved_track(request_for_call)
    })
    .await
    {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    if let Some(data) = response.data.as_ref() {
        emit_playback_event(
            &app,
            PlaybackEvent::Started {
                is_seek,
                path: data.path.clone(),
                playing: data.playing,
                position_ms: data.position_ms,
                duration_ms: data.duration_ms,
                track_id: data.track_id.clone(),
            },
        );
    } else {
        emit_playback_event(
            &app,
            PlaybackEvent::Error(response.error.as_ref().map(|e| e.message.clone())),
        );
    }
    Ok(response)
}

#[tauri::command]
pub async fn stop_playback_native(
    app: AppHandle,
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<StopPlaybackData>, String> {
    let commands = state.inner().clone();
    let response =
        match run_playback_blocking(&app, "stop", move || commands.stop_playback_native()).await {
            Ok(r) => r,
            Err(r) => return Ok(r),
        };
    if let Some(data) = response.data.as_ref() {
        emit_playback_event(
            &app,
            PlaybackEvent::Stopped {
                path: data.previous_path.clone(),
                duration_ms: None,
            },
        );
    } else {
        emit_playback_event(
            &app,
            PlaybackEvent::Error(response.error.as_ref().map(|e| e.message.clone())),
        );
    }
    Ok(response)
}

#[tauri::command]
pub async fn get_playback_status_native(
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<PlaybackStatusData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("native playback status", move || {
        commands.get_playback_status_native()
    })
    .await)
}

#[tauri::command]
pub async fn playback_preflight_native(
    state: State<'_, BackendCommands>,
    request: PlaybackPreflightRequest,
) -> Result<ApiResponse<PlaybackPreflightData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("native playback preflight", move || {
        commands.playback_preflight_native(request)
    })
    .await)
}

#[tauri::command]
pub async fn inspect_usb_track(
    state: State<'_, BackendCommands>,
    request: InspectUsbTrackRequest,
) -> Result<ApiResponse<InspectUsbTrackData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("inspect_usb_track", move || {
        commands.inspect_usb_track(request)
    })
    .await)
}

#[tauri::command]
pub async fn inspect_usb_tracks(
    state: State<'_, BackendCommands>,
    request: InspectUsbTracksRequest,
) -> Result<ApiResponse<InspectUsbTracksData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("inspect_usb_tracks", move || {
        commands.inspect_usb_tracks(request)
    })
    .await)
}

#[tauri::command]
pub async fn fetch_usb_playlist_tracks(
    state: State<'_, BackendCommands>,
    request: FetchUsbTracksRequest,
) -> Result<ApiResponse<FetchUsbTracksData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("fetch_usb_playlist_tracks", move || {
        commands.fetch_usb_playlist_tracks(request)
    })
    .await)
}

#[tauri::command]
pub async fn fetch_usb_history_tracks(
    state: State<'_, BackendCommands>,
    request: FetchUsbTracksRequest,
) -> Result<ApiResponse<FetchUsbTracksData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("fetch_usb_history_tracks", move || {
        commands.fetch_usb_history_tracks(request)
    })
    .await)
}

#[tauri::command]
pub async fn analyze_new_tracks(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: AnalyzeNewTracksRequest,
) -> Result<ApiResponse<AnalyzeNewTracksData>, String> {
    let job_id = Uuid::now_v7().to_string();
    let identity = JobIdentity {
        job_id: &job_id,
        job_type: "analysis",
        stage: "analyze_new_tracks",
    };
    emit_job_event(
        &app,
        "job.started",
        identity,
        0,
        1,
        0,
        "Analyzing selected tracks",
    );
    emit_job_event(
        &app,
        "job.progress",
        identity,
        0,
        1,
        40,
        "Decoding audio and extracting metadata",
    );
    let commands = state.inner().clone();
    let app_for_task = app.clone();
    let job_id_for_task = job_id.clone();
    let mut response = match tauri::async_runtime::spawn_blocking(move || {
        commands.analyze_new_tracks_with_progress(request, |progress| {
            let current = progress.current;
            let total = progress.total;
            let file_name = std::path::Path::new(&progress.file_path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&progress.file_path);
            let identity = JobIdentity {
                job_id: &job_id_for_task,
                job_type: "analysis",
                stage: "analyze_new_tracks",
            };
            emit_job_event_with_track(
                &app_for_task,
                "job.progress",
                identity,
                format!("Analyzing {current}/{total}: {file_name}"),
                progress,
            );
        })
    })
    .await
    {
        Ok(response) => response,
        Err(err) => ApiResponse::failure(
            crate::error::BackendError::Internal(format!("analyze_new_tracks task failed: {err}"))
                .into(),
        ),
    };
    if let Some(data) = response.data.as_mut() {
        data.job_id = job_id.clone();
        emit_job_event(
            &app,
            "job.completed",
            identity,
            1,
            1,
            100,
            format!(
                "Analysis finished: {} analyzed, {} failed",
                data.analyzed, data.failed
            ),
        );
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
pub fn set_analysis_paused(
    state: State<'_, BackendCommands>,
    request: SetAnalysisPausedRequest,
) -> ApiResponse<SetAnalysisPausedData> {
    state.set_analysis_paused(request.paused)
}

#[tauri::command]
pub fn cancel_analysis(state: State<'_, BackendCommands>) -> ApiResponse<()> {
    state.cancel_analysis()
}

#[tauri::command]
pub async fn export_to_usb(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: ExportToUsbRequest,
) -> Result<ApiResponse<ExportToUsbData>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        "export",
        "export_to_usb",
        "USB: Exporting playlist",
        "USB: Export complete",
        move |mut progress| {
            commands.export_to_usb_with_progress(request, move |c, t, m| {
                progress(c, t, m);
            })
        },
    )
    .await
}

#[tauri::command]
pub async fn run_usb_diagnostics(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: RunUsbDiagnosticsRequest,
) -> Result<ApiResponse<RunUsbDiagnosticsData>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        "diagnostics",
        "run_usb_diagnostics",
        "USB: Running diagnostics",
        "USB: Diagnostics complete",
        move |mut progress| {
            commands.run_usb_diagnostics_with_progress(request, move |c, t, m| {
                progress(c, t, m);
            })
        },
    )
    .await
}

#[tauri::command]
pub async fn run_usb_parity_report(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: RunUsbParityReportRequest,
) -> Result<ApiResponse<RunUsbParityReportData>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        "diagnostics",
        "run_usb_parity_report",
        "USB: Running parity report",
        "USB: Parity report complete",
        move |mut progress| {
            commands.run_usb_parity_report_with_progress(request, move |c, t, m| {
                progress(c, t, m);
            })
        },
    )
    .await
}

#[tauri::command]
pub async fn repair_usb_diagnostics(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: RepairUsbDiagnosticsRequest,
) -> Result<ApiResponse<RepairUsbDiagnosticsData>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        "diagnostics",
        "repair_usb_diagnostics",
        "USB: Planning repair fixes",
        "USB: Repair planning complete",
        move |mut progress| {
            commands.repair_usb_diagnostics_with_progress(request, move |c, t, m| {
                progress(c, t, m);
            })
        },
    )
    .await
}

#[tauri::command]
pub fn detect_external_master_db(
    state: State<'_, BackendCommands>,
) -> ApiResponse<DetectExternalMasterDbData> {
    state.detect_external_master_db()
}

#[tauri::command]
pub async fn initialize_usb(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: InitializeUsbRequest,
) -> Result<ApiResponse<InitializeUsbData>, String> {
    let commands = state.inner().clone();
    run_usb_job(
        &app,
        "usb_read",
        "initialize_usb",
        "USB: Initializing structure",
        "USB: Initialization complete",
        Some((40, "USB: Creating External library directories".to_string())),
        move || commands.initialize_usb(request),
    )
    .await
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EssentiaDownloadProgress {
    bytes_received: u64,
    total_bytes: Option<u64>,
    percent: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[tauri::command]
pub async fn download_essentia(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    cancel: State<'_, EssentiaDownloadCancel>,
) -> Result<ApiResponse<()>, String> {
    use futures_util::StreamExt;
    use std::sync::atomic::Ordering;

    fn extract_npm_package_from_tgz(
        tgz_path: &std::path::Path,
        node_modules_dir: &std::path::Path,
        package_name: &str,
    ) -> Result<(), String> {
        let extract_dir = node_modules_dir
            .parent()
            .unwrap_or(node_modules_dir)
            .join(format!("extract_tmp_{package_name}"));
        if extract_dir.exists() {
            std::fs::remove_dir_all(&extract_dir)
                .map_err(|e| format!("cleanup extract dir ({package_name}): {e}"))?;
        }
        std::fs::create_dir_all(&extract_dir)
            .map_err(|e| format!("create extract dir ({package_name}): {e}"))?;

        let tgz_file =
            std::fs::File::open(tgz_path).map_err(|e| format!("open tgz ({package_name}): {e}"))?;
        let gz = flate2::read::GzDecoder::new(tgz_file);
        let mut archive = tar::Archive::new(gz);
        archive
            .unpack(&extract_dir)
            .map_err(|e| format!("extract failed ({package_name}): {e}"))?;

        let package_dir = extract_dir.join("package");
        let dest = node_modules_dir.join(package_name);
        if dest.exists() {
            std::fs::remove_dir_all(&dest)
                .map_err(|e| format!("remove old package ({package_name}): {e}"))?;
        }
        std::fs::rename(&package_dir, &dest)
            .map_err(|e| format!("install package ({package_name}): {e}"))?;

        let _ = std::fs::remove_dir_all(&extract_dir);
        Ok(())
    }

    cancel.0.store(false, Ordering::SeqCst);

    let data_dir = state.data_dir().to_path_buf();
    let cancel_flag = cancel.0.clone();

    let result: Result<(), String> = async {
        let url = "https://registry.npmjs.org/essentia.js/-/essentia.js-0.1.3.tgz";
        let response = reqwest::get(url)
            .await
            .map_err(|e| format!("download failed: {e}"))?;
        let total_bytes = response.content_length();
        let mut stream = response.bytes_stream();

        let tmp_tgz = data_dir.join("essentia_download.tmp.tgz");
        let mut file = std::fs::File::create(&tmp_tgz)
            .map_err(|e| format!("failed to create temp file: {e}"))?;

        let mut bytes_received: u64 = 0;
        while let Some(chunk) = stream.next().await {
            if cancel_flag.load(Ordering::SeqCst) {
                drop(file);
                let _ = std::fs::remove_file(&tmp_tgz);
                return Err("cancelled".to_string());
            }
            let chunk = chunk.map_err(|e| format!("download error: {e}"))?;
            std::io::Write::write_all(&mut file, &chunk)
                .map_err(|e| format!("write error: {e}"))?;
            bytes_received += chunk.len() as u64;
            let percent = total_bytes
                .map(|t| (bytes_received as f32 / t as f32 * 100.0).min(99.0))
                .unwrap_or(0.0);
            let _ = app.emit(
                ESSENTIA_DOWNLOAD_EVENT,
                EssentiaDownloadProgress {
                    bytes_received,
                    total_bytes,
                    percent,
                    done: None,
                    error: None,
                },
            );
        }
        drop(file);

        // Install package tarball to app-data node_modules.
        let node_modules_dir = data_dir.join("essentia").join("node_modules");
        std::fs::create_dir_all(&node_modules_dir)
            .map_err(|e| format!("create node_modules dir: {e}"))?;
        extract_npm_package_from_tgz(&tmp_tgz, &node_modules_dir, "essentia.js")?;

        // Install known dependency required by essentia.js package.
        let node_wav_tgz = data_dir.join("node_wav_download.tmp.tgz");
        let dep_url = "https://registry.npmjs.org/node-wav/-/node-wav-0.0.2.tgz";
        let dep_bytes = reqwest::get(dep_url)
            .await
            .map_err(|e| format!("download node-wav failed: {e}"))?
            .bytes()
            .await
            .map_err(|e| format!("read node-wav payload failed: {e}"))?;
        std::fs::write(&node_wav_tgz, &dep_bytes)
            .map_err(|e| format!("write node-wav temp file failed: {e}"))?;
        extract_npm_package_from_tgz(&node_wav_tgz, &node_modules_dir, "node-wav")?;
        let _ = std::fs::remove_file(&node_wav_tgz);

        if !node_modules_dir
            .join("essentia.js")
            .join("dist")
            .join("essentia-wasm.umd.js")
            .is_file()
        {
            return Err(
                "essentia install incomplete: missing dist/essentia-wasm.umd.js".to_string(),
            );
        }
        if !node_modules_dir
            .join("node-wav")
            .join("package.json")
            .is_file()
        {
            return Err("essentia install incomplete: missing node-wav dependency".to_string());
        }

        // Clean up temp files
        let _ = std::fs::remove_file(&tmp_tgz);

        // Make the new install visible to analysis in this session without restart.
        // SAFETY: single write after download completes; analysis threads read this lazily per-job.
        unsafe {
            std::env::set_var(
                "DJTKIT_ESSENTIA_NODE_MODULES",
                node_modules_dir.to_string_lossy().to_string(),
            );
        }

        Ok(())
    }
    .await;

    match result {
        Ok(()) => {
            let _ = app.emit(
                ESSENTIA_DOWNLOAD_EVENT,
                EssentiaDownloadProgress {
                    bytes_received: 0,
                    total_bytes: None,
                    percent: 100.0,
                    done: Some(true),
                    error: None,
                },
            );
            Ok(ApiResponse {
                ok: true,
                data: Some(()),
                error: None,
            })
        }
        Err(msg) => {
            let _ = app.emit(
                ESSENTIA_DOWNLOAD_EVENT,
                EssentiaDownloadProgress {
                    bytes_received: 0,
                    total_bytes: None,
                    percent: 0.0,
                    done: None,
                    error: Some(msg.clone()),
                },
            );
            Ok(ApiResponse {
                ok: false,
                data: None,
                error: Some(crate::error::ErrorPayload {
                    code: crate::error::ErrorCode::InternalError,
                    message: msg,
                    details: None,
                }),
            })
        }
    }
}

#[tauri::command]
pub fn cancel_essentia_download(cancel: State<'_, EssentiaDownloadCancel>) -> ApiResponse<()> {
    use std::sync::atomic::Ordering;
    cancel.0.store(true, Ordering::SeqCst);
    ApiResponse {
        ok: true,
        data: Some(()),
        error: None,
    }
}

#[tauri::command]
pub fn remove_essentia(state: State<'_, BackendCommands>) -> ApiResponse<()> {
    state.remove_essentia()
}

const RELEASES_API_URL: &str =
    "https://api.github.com/repos/haivala/dj-usb-tkit/releases?per_page=10";

/// Checks GitHub Releases for a newer stable build than the running one.
/// Version comparison and the "critical" rule live in
/// `service::update_check`; this only does the fetch. A failed check is
/// background noise -- it logs quietly and reports "no update" rather than
/// surfacing an error.
#[tauri::command]
pub async fn check_for_update() -> ApiResponse<crate::service::update_check::UpdateInfo> {
    use crate::service::update_check::{self, GithubRelease, UpdateInfo};

    let current = env!("CARGO_PKG_VERSION");

    let fetch = async {
        let client = reqwest::Client::builder()
            .user_agent(concat!("dj-usb-tkit/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        let response = client
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
        Ok(releases) => ApiResponse::success(update_check::evaluate(current, &releases)),
        Err(message) => {
            crate::backend_log!(Warn, "update-check", "update check failed: {message}");
            ApiResponse::success(UpdateInfo::none(current))
        }
    }
}
