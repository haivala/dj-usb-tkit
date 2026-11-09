#![cfg(feature = "tauri")]

use std::collections::BTreeSet;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;

use chrono::Utc;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

use crate::commands::BackendCommands;
use crate::error::{ErrorCode, ErrorPayload};
use crate::i18n::{Text, tr};
use crate::models::{
    AddLibrarySelectionToPlaylistRequest, AddLibrarySelectionToPlaylistsRequest,
    AddSourceFoldersData, AddSourceFoldersRequest, AddTrackCandidatesToPlaylistData,
    AddTrackCandidatesToPlaylistRequest, AddTrackCandidatesToPlaylistsRequest,
    AddTracksToPlaylistData, AddTracksToPlaylistsData, AnalysisSubmittedData,
    AnalyzeNewTracksRequest, ApiResponse, BrowseSourceFilesData, BrowseSourceFilesRequest,
    BuildUsbHistoryTracklistData, BuildUsbHistoryTracklistRequest, CreatePlaylistData,
    CreatePlaylistRequest, DeletePlaylistData, DeletePlaylistRequest, DeleteUsbBackupData,
    DeleteUsbBackupRequest, DetectExternalMixxxDbData, DetectExternalRekordboxDbData,
    ExportToUsbData, ExportToUsbRequest, FetchUsbHistoriesData, FetchUsbHistoriesRequest,
    FetchUsbPlaylistsData, FetchUsbPlaylistsRequest, FetchUsbTracksData, FetchUsbTracksRequest,
    GetFrontendSettingsData, GetPlaylistExportSpaceData, GetPlaylistExportSpaceRequest,
    GetPlaylistMembershipData, GetPlaylistMembershipRequest, GetPlaylistTracksData,
    GetPlaylistTracksRequest, GetSelectionDurationData, GetSelectionDurationRequest,
    GetTrackDetailRequest, GetTracksMetadataSummaryRequest, GetUsbBackgroundColorRequest,
    GetUsbDeviceNameData, GetUsbDeviceNameRequest, GetUsbHistoryTracklistOptionsData,
    GetUsbHistoryTracklistOptionsRequest, GetUsbPlayerMenuConfigData,
    GetUsbPlayerMenuConfigRequest, GetUsbTrackDetailRequest, GetUsbTracksMetadataSummaryRequest,
    ImportExternalPlaylistsData, ImportExternalPlaylistsRequest, InitializeUsbData,
    InitializeUsbRequest, InspectUsbTrackData, InspectUsbTrackRequest, JobEventPayload,
    LibrarySourcesData, ListExternalPlaylistsData, ListExternalPlaylistsRequest,
    ListMatchingTrackIdsData, ListMatchingTrackIdsRequest, ListPlaylistsData, ListUsbBackupsData,
    ListUsbBackupsRequest, ListUsbDevicesData, PlayResolvedTrackData, PlayResolvedTrackRequest,
    PlaybackEventPayload, PlaybackMetronomeData, PlaybackStatusData, PruneUsbDeviceData,
    PruneUsbDeviceRequest, RelocateSourceRootData, RelocateSourceRootRequest,
    RemoveSourceFolderData, RemoveSourceFolderRequest, RemoveTracksFromPlaylistData,
    RemoveTracksFromPlaylistRequest, RemoveUsbPlaylistData, RemoveUsbPlaylistRequest,
    RenamePlaylistData, RenamePlaylistRequest, ReorderPlaylistTracksData,
    ReorderPlaylistTracksRequest, ReorderPlaylistsData, ReorderPlaylistsRequest,
    ReorderUsbPlaylistsData, ReorderUsbPlaylistsRequest, RepairUsbDiagnosticsData,
    RepairUsbDiagnosticsRequest, ResolvePlaybackSourceData, ResolvePlaybackSourceRequest,
    ResolveTrackIdentityData, ResolveTrackIdentityRequest, RestoreUsbBackupData,
    RestoreUsbBackupRequest, RunDataMigrationsData, RunUsbDiagnosticsData,
    RunUsbDiagnosticsRequest, RunUsbParityReportData, RunUsbParityReportRequest,
    SaveTrackAnalysisEditsData, SaveTrackAnalysisEditsRequest, SaveTrackMetadataEditsData,
    SaveTrackMetadataEditsRequest, SaveUsbTrackAnalysisEditsData, SaveUsbTrackAnalysisEditsRequest,
    SaveUsbTrackMetadataEditsData, SaveUsbTrackMetadataEditsRequest, ScanLibraryData,
    ScanLibraryRequest, ScanMixxxDbRequest, ScanRekordboxDbRequest, SetAnalysisPausedData,
    SetAnalysisPausedRequest, SetExternalLibraryEnabledRequest, SetFrontendSettingData,
    SetFrontendSettingRequest, SetPlaybackMetronomeRequest, SetSourceFolderEnabledRequest,
    SetUsbBackgroundColorRequest, SetUsbDeviceNameData, SetUsbDeviceNameRequest, StopPlaybackData,
    TrackDetail, TracksMetadataSummary, UpdateUsbPlayerMenuConfigData,
    UpdateUsbPlayerMenuConfigRequest, UsbBackgroundColorData, UsbTrackAnalysisDetail,
    ValidateUsbRootData, ValidateUsbRootRequest,
};
use crate::service::{AnalysisEvent, analysis_event_payload};

const JOB_EVENT_CHANNEL: &str = "job:event";
const PLAYBACK_EVENT_CHANNEL: &str = "playback:event";
const ESSENTIA_DOWNLOAD_EVENT: &str = "essentia_download_progress";

/// Ids of jobs that have emitted `job.started` but not yet `job.completed` /
/// `job.failed`, kept up to date by `emit_job_payload`. `install_update` waits
/// for it to empty and holds it while installing: every job emits
/// `job.started` before doing any work, so a job started meanwhile blocks
/// there until the app has restarted instead of being cut short.
static RUNNING_JOBS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn running_jobs() -> MutexGuard<'static, BTreeSet<String>> {
    RUNNING_JOBS.lock().unwrap_or_else(PoisonError::into_inner)
}

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
    ApiResponse::failure(ErrorPayload::new(ErrorCode::InternalError, message))
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
            Err(ApiResponse::failure(ErrorPayload::new(
                ErrorCode::InternalError,
                message,
            )))
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
        bpm_source_text: None,
        key: None,
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

fn emit_job_payload<R: tauri::Runtime>(
    app: &AppHandle<R>,
    event_name: &str,
    payload: JobEventPayload,
) {
    match event_name {
        "job.started" => {
            running_jobs().insert(payload.job_id.clone());
        }
        "job.completed" | "job.failed" => {
            running_jobs().remove(&payload.job_id);
        }
        _ => {}
    }
    let direct_event_name = event_name.replace('.', ":");
    let _ = app.emit(&direct_event_name, payload.clone());
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit(JOB_EVENT_CHANNEL, payload);
    } else {
        let _ = app.emit(JOB_EVENT_CHANNEL, payload);
    }
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
        response_message.unwrap_or_else(|| tr::job_failed_unknown().render()),
    );
}

async fn run_usb_job<T, F>(
    app: &AppHandle,
    job_type: &str,
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

async fn run_usb_job_with_progress<T, F>(
    app: &AppHandle,
    job_type: &str,
    stage: &str,
    started_message: Text,
    completed_message: Text,
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
    let job_type_for_task = job_type.to_string();
    let stage_for_task = stage.to_string();
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
    /// `playback.paused` / `playback.resumed`: the worker's status right after
    /// the pause/resume was applied.
    PauseChanged(PlaybackStatusData),
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
            event: if is_seek {
                "playback.seeked"
            } else {
                "playback.started"
            }
            .to_string(),
            path: Some(path),
            playing,
            paused: false,
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
            paused: false,
            position_ms: 0,
            duration_ms,
            message: None,
            track_id: None,
            timestamp: Utc::now().to_rfc3339(),
        },
        PlaybackEvent::PauseChanged(status) => PlaybackEventPayload {
            event: if status.paused {
                "playback.paused"
            } else {
                "playback.resumed"
            }
            .to_string(),
            path: status.path,
            playing: status.playing,
            paused: status.paused,
            position_ms: status.position_ms,
            duration_ms: status.duration_ms,
            message: None,
            track_id: None,
            timestamp: Utc::now().to_rfc3339(),
        },
        PlaybackEvent::Error(message) => PlaybackEventPayload {
            event: "playback.error".to_string(),
            path: None,
            playing: false,
            paused: false,
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
        tr::job_scan_library_started().render(),
    );
    emit_job_event(
        &app,
        "job.progress",
        identity,
        0,
        1,
        30,
        tr::job_scan_library_progress().render(),
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
                tr::job_scan_library_completed_counts(d.indexed, d.updated, d.removed).render()
            })
            .unwrap_or_else(|| tr::job_scan_library_completed().render());
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
pub async fn scan_rekordbox_db(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: ScanRekordboxDbRequest,
) -> Result<ApiResponse<ScanLibraryData>, String> {
    let job_id = Uuid::now_v7().to_string();
    let identity = JobIdentity {
        job_id: &job_id,
        job_type: "scan",
        stage: "scan_rekordbox_db",
    };
    emit_job_event(
        &app,
        "job.started",
        identity,
        0,
        1,
        0,
        tr::job_scan_rekordbox_db_started().render(),
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
        job_type: "scan",
        stage: "scan_mixxx_db",
    };
    emit_job_event(
        &app,
        "job.started",
        identity,
        0,
        1,
        0,
        tr::job_scan_mixxx_db_started().render(),
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

// Off the main thread: it rebuilds, filters and sorts the whole library per
// page, and a sync command would freeze the window meanwhile.
#[tauri::command]
pub async fn browse_source_files(
    state: State<'_, BackendCommands>,
    request: BrowseSourceFilesRequest,
) -> Result<ApiResponse<BrowseSourceFilesData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("browse_source_files", move || {
        commands.browse_source_files(request)
    })
    .await)
}

#[tauri::command]
pub async fn get_library_sources(
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<LibrarySourcesData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("get_library_sources", move || {
        commands.get_library_sources()
    })
    .await)
}

#[tauri::command]
pub async fn add_source_folders(
    state: State<'_, BackendCommands>,
    request: AddSourceFoldersRequest,
) -> Result<ApiResponse<AddSourceFoldersData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("add_source_folders", move || {
        commands.add_source_folders(request)
    })
    .await)
}

#[tauri::command]
pub async fn remove_source_folder(
    state: State<'_, BackendCommands>,
    request: RemoveSourceFolderRequest,
) -> Result<ApiResponse<RemoveSourceFolderData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("remove_source_folder", move || {
        commands.remove_source_folder(request)
    })
    .await)
}

#[tauri::command]
pub async fn set_source_folder_enabled(
    state: State<'_, BackendCommands>,
    request: SetSourceFolderEnabledRequest,
) -> Result<ApiResponse<LibrarySourcesData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("set_source_folder_enabled", move || {
        commands.set_source_folder_enabled(request)
    })
    .await)
}

#[tauri::command]
pub async fn set_external_library_enabled(
    state: State<'_, BackendCommands>,
    request: SetExternalLibraryEnabledRequest,
) -> Result<ApiResponse<LibrarySourcesData>, String> {
    let commands = state.inner().clone();
    Ok(
        run_blocking_command("set_external_library_enabled", move || {
            commands.set_external_library_enabled(request)
        })
        .await,
    )
}

#[tauri::command]
pub async fn resolve_track_identity(
    state: State<'_, BackendCommands>,
    request: ResolveTrackIdentityRequest,
) -> Result<ApiResponse<ResolveTrackIdentityData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("resolve_track_identity", move || {
        commands.resolve_track_identity(request)
    })
    .await)
}

#[tauri::command]
pub async fn get_track_detail(
    state: State<'_, BackendCommands>,
    request: GetTrackDetailRequest,
) -> Result<ApiResponse<TrackDetail>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("get_track_detail", move || {
        commands.get_track_detail(request)
    })
    .await)
}

#[tauri::command]
pub async fn save_track_analysis_edits(
    state: State<'_, BackendCommands>,
    request: SaveTrackAnalysisEditsRequest,
) -> Result<ApiResponse<SaveTrackAnalysisEditsData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("save_track_analysis_edits", move || {
        commands.save_track_analysis_edits(request)
    })
    .await)
}

#[tauri::command]
pub async fn save_track_metadata_edits(
    state: State<'_, BackendCommands>,
    request: SaveTrackMetadataEditsRequest,
) -> Result<ApiResponse<SaveTrackMetadataEditsData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("save_track_metadata_edits", move || {
        commands.save_track_metadata_edits(request)
    })
    .await)
}

/// What the selected library tracks share, for the selection bar's Edit.
#[tauri::command]
pub async fn get_tracks_metadata_summary(
    state: State<'_, BackendCommands>,
    request: GetTracksMetadataSummaryRequest,
) -> Result<ApiResponse<TracksMetadataSummary>, String> {
    let commands = state.inner().clone();
    Ok(
        run_blocking_command("get_tracks_metadata_summary", move || {
            commands.get_tracks_metadata_summary(request)
        })
        .await,
    )
}

/// What the selected USB tracks share, as the USB's PDB has them.
#[tauri::command]
pub async fn get_usb_tracks_metadata_summary(
    state: State<'_, BackendCommands>,
    request: GetUsbTracksMetadataSummaryRequest,
) -> Result<ApiResponse<TracksMetadataSummary>, String> {
    let commands = state.inner().clone();
    Ok(
        run_blocking_command("get_usb_tracks_metadata_summary", move || {
            commands.get_usb_tracks_metadata_summary(request)
        })
        .await,
    )
}

/// A background `usb_write` job, like `save_usb_track_analysis_edits`: it
/// writes `export.pdb`, the library and `exportLibrary.db`.
#[tauri::command]
pub async fn save_usb_track_metadata_edits(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: SaveUsbTrackMetadataEditsRequest,
) -> Result<ApiResponse<SaveUsbTrackMetadataEditsData>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        "usb_write",
        "save_usb_track_metadata_edits",
        tr::job_save_usb_track_metadata_edits_started(),
        tr::job_save_usb_track_metadata_edits_completed(),
        move |mut progress| {
            commands.save_usb_track_metadata_edits_with_progress(request, move |c, t, m| {
                progress(c, t, m);
            })
        },
    )
    .await
}

#[tauri::command]
pub async fn get_usb_track_detail(
    state: State<'_, BackendCommands>,
    request: GetUsbTrackDetailRequest,
) -> Result<ApiResponse<UsbTrackAnalysisDetail>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("get_usb_track_detail", move || {
        commands.get_usb_track_detail(request)
    })
    .await)
}

/// Runs as a background `usb_write` job (not on the main thread): the save
/// writes the ANLZ bundle, `export.pdb`, the library and `exportLibrary.db`,
/// and the window must not freeze meanwhile. Progress drives the footer bar.
#[tauri::command]
pub async fn save_usb_track_analysis_edits(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: SaveUsbTrackAnalysisEditsRequest,
) -> Result<ApiResponse<SaveUsbTrackAnalysisEditsData>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        "usb_write",
        "save_usb_track_analysis_edits",
        tr::job_save_usb_track_analysis_edits_started(),
        tr::job_save_usb_track_analysis_edits_completed(),
        move |mut progress| {
            commands.save_usb_track_analysis_edits_with_progress(request, move |c, t, m| {
                progress(c, t, m);
            })
        },
    )
    .await
}

#[tauri::command]
pub async fn relocate_source_root(
    state: State<'_, BackendCommands>,
    request: RelocateSourceRootRequest,
) -> Result<ApiResponse<RelocateSourceRootData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("relocate_source_root", move || {
        commands.relocate_source_root(request)
    })
    .await)
}

#[tauri::command]
pub async fn resolve_playback_source(
    state: State<'_, BackendCommands>,
    request: ResolvePlaybackSourceRequest,
) -> Result<ApiResponse<ResolvePlaybackSourceData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("resolve_playback_source", move || {
        commands.resolve_playback_source(request)
    })
    .await)
}

#[tauri::command]
pub async fn create_playlist(
    state: State<'_, BackendCommands>,
    request: CreatePlaylistRequest,
) -> Result<ApiResponse<CreatePlaylistData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("create_playlist", move || commands.create_playlist(request)).await)
}

#[tauri::command]
pub async fn rename_playlist(
    state: State<'_, BackendCommands>,
    request: RenamePlaylistRequest,
) -> Result<ApiResponse<RenamePlaylistData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("rename_playlist", move || commands.rename_playlist(request)).await)
}

#[tauri::command]
pub async fn reorder_playlists(
    state: State<'_, BackendCommands>,
    request: ReorderPlaylistsRequest,
) -> Result<ApiResponse<ReorderPlaylistsData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("reorder_playlists", move || {
        commands.reorder_playlists(request)
    })
    .await)
}

#[tauri::command]
pub async fn delete_playlist(
    state: State<'_, BackendCommands>,
    request: DeletePlaylistRequest,
) -> Result<ApiResponse<DeletePlaylistData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("delete_playlist", move || commands.delete_playlist(request)).await)
}

#[tauri::command]
pub async fn list_playlists(
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<ListPlaylistsData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("list_playlists", move || commands.list_playlists()).await)
}

#[tauri::command]
pub async fn get_playlist_tracks(
    state: State<'_, BackendCommands>,
    request: GetPlaylistTracksRequest,
) -> Result<ApiResponse<GetPlaylistTracksData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("get_playlist_tracks", move || {
        commands.get_playlist_tracks(request)
    })
    .await)
}

#[tauri::command]
pub async fn get_playlist_export_space(
    state: State<'_, BackendCommands>,
    request: GetPlaylistExportSpaceRequest,
) -> Result<ApiResponse<GetPlaylistExportSpaceData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("get_playlist_export_space", move || {
        commands.get_playlist_export_space(request)
    })
    .await)
}

#[tauri::command]
pub async fn add_track_candidates_to_playlist(
    state: State<'_, BackendCommands>,
    request: AddTrackCandidatesToPlaylistRequest,
) -> Result<ApiResponse<AddTrackCandidatesToPlaylistData>, String> {
    let commands = state.inner().clone();
    Ok(
        run_blocking_command("add_track_candidates_to_playlist", move || {
            commands.add_track_candidates_to_playlist(request)
        })
        .await,
    )
}

#[tauri::command]
pub async fn list_matching_track_ids(
    state: State<'_, BackendCommands>,
    request: ListMatchingTrackIdsRequest,
) -> Result<ApiResponse<ListMatchingTrackIdsData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("list_matching_track_ids", move || {
        commands.list_matching_track_ids(request)
    })
    .await)
}

#[tauri::command]
pub async fn get_selection_duration(
    state: State<'_, BackendCommands>,
    request: GetSelectionDurationRequest,
) -> Result<ApiResponse<GetSelectionDurationData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("get_selection_duration", move || {
        commands.get_selection_duration(request)
    })
    .await)
}

#[tauri::command]
pub async fn add_library_selection_to_playlist(
    state: State<'_, BackendCommands>,
    request: AddLibrarySelectionToPlaylistRequest,
) -> Result<ApiResponse<AddTracksToPlaylistData>, String> {
    let commands = state.inner().clone();
    Ok(
        run_blocking_command("add_library_selection_to_playlist", move || {
            commands.add_library_selection_to_playlist(request)
        })
        .await,
    )
}

#[tauri::command]
pub async fn add_library_selection_to_playlists(
    state: State<'_, BackendCommands>,
    request: AddLibrarySelectionToPlaylistsRequest,
) -> Result<ApiResponse<AddTracksToPlaylistsData>, String> {
    let commands = state.inner().clone();
    Ok(
        run_blocking_command("add_library_selection_to_playlists", move || {
            commands.add_library_selection_to_playlists(request)
        })
        .await,
    )
}

#[tauri::command]
pub async fn add_track_candidates_to_playlists(
    state: State<'_, BackendCommands>,
    request: AddTrackCandidatesToPlaylistsRequest,
) -> Result<ApiResponse<AddTracksToPlaylistsData>, String> {
    let commands = state.inner().clone();
    Ok(
        run_blocking_command("add_track_candidates_to_playlists", move || {
            commands.add_track_candidates_to_playlists(request)
        })
        .await,
    )
}

#[tauri::command]
pub async fn get_playlist_membership(
    state: State<'_, BackendCommands>,
    request: GetPlaylistMembershipRequest,
) -> Result<ApiResponse<GetPlaylistMembershipData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("get_playlist_membership", move || {
        commands.get_playlist_membership(request)
    })
    .await)
}

#[tauri::command]
pub async fn remove_tracks_from_playlist(
    state: State<'_, BackendCommands>,
    request: RemoveTracksFromPlaylistRequest,
) -> Result<ApiResponse<RemoveTracksFromPlaylistData>, String> {
    let commands = state.inner().clone();
    Ok(
        run_blocking_command("remove_tracks_from_playlist", move || {
            commands.remove_tracks_from_playlist(request)
        })
        .await,
    )
}

#[tauri::command]
pub async fn refresh_playlist_export_status(
    state: State<'_, BackendCommands>,
    request: crate::models::RefreshPlaylistExportStatusRequest,
) -> Result<ApiResponse<crate::models::RefreshPlaylistExportStatusData>, String> {
    let commands = state.inner().clone();
    Ok(
        run_blocking_command("refresh_playlist_export_status", move || {
            commands.refresh_playlist_export_status(request)
        })
        .await,
    )
}

#[tauri::command]
pub async fn reorder_playlist_tracks(
    state: State<'_, BackendCommands>,
    request: ReorderPlaylistTracksRequest,
) -> Result<ApiResponse<ReorderPlaylistTracksData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("reorder_playlist_tracks", move || {
        commands.reorder_playlist_tracks(request)
    })
    .await)
}

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
    // Held until the setting is saved, so no job starts during a language change.
    let jobs = running_jobs();
    state.set_frontend_setting_unless_job_running(request, !jobs.is_empty())
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
        tr::job_validate_usb_root_started(),
        tr::job_validate_usb_root_completed(),
        Some((30, tr::job_validate_usb_root_progress())),
        move || commands.validate_usb_root(request),
    )
    .await
}

#[tauri::command]
pub async fn list_usb_devices(
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<ListUsbDevicesData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("list_usb_devices", move || commands.list_usb_devices()).await)
}

#[tauri::command]
pub async fn prune_usb_device(
    state: State<'_, BackendCommands>,
    request: PruneUsbDeviceRequest,
) -> Result<ApiResponse<PruneUsbDeviceData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("prune_usb_device", move || {
        commands.prune_usb_device(request)
    })
    .await)
}

#[tauri::command]
pub async fn get_usb_device_name(
    state: State<'_, BackendCommands>,
    request: GetUsbDeviceNameRequest,
) -> Result<ApiResponse<GetUsbDeviceNameData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("get_usb_device_name", move || {
        commands.get_usb_device_name(request)
    })
    .await)
}

#[tauri::command]
pub async fn set_usb_device_name(
    state: State<'_, BackendCommands>,
    request: SetUsbDeviceNameRequest,
) -> Result<ApiResponse<SetUsbDeviceNameData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("set_usb_device_name", move || {
        commands.set_usb_device_name(request)
    })
    .await)
}

#[tauri::command]
pub async fn get_usb_audio_formats(
    state: State<'_, BackendCommands>,
    request: crate::models::GetUsbAudioFormatsRequest,
) -> Result<ApiResponse<crate::models::UsbAudioFormatsData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("get_usb_audio_formats", move || {
        commands.get_usb_audio_formats(request)
    })
    .await)
}

#[tauri::command]
pub async fn set_usb_audio_formats(
    state: State<'_, BackendCommands>,
    request: crate::models::SetUsbAudioFormatsRequest,
) -> Result<ApiResponse<crate::models::UsbAudioFormatsData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("set_usb_audio_formats", move || {
        commands.set_usb_audio_formats(request)
    })
    .await)
}

#[tauri::command]
pub async fn list_usb_backups(
    state: State<'_, BackendCommands>,
    request: ListUsbBackupsRequest,
) -> Result<ApiResponse<ListUsbBackupsData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("list_usb_backups", move || {
        commands.list_usb_backups(request)
    })
    .await)
}

#[tauri::command]
pub async fn restore_usb_backup(
    state: State<'_, BackendCommands>,
    request: RestoreUsbBackupRequest,
) -> Result<ApiResponse<RestoreUsbBackupData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("restore_usb_backup", move || {
        commands.restore_usb_backup(request)
    })
    .await)
}

#[tauri::command]
pub async fn delete_usb_backup(
    state: State<'_, BackendCommands>,
    request: DeleteUsbBackupRequest,
) -> Result<ApiResponse<DeleteUsbBackupData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("delete_usb_backup", move || {
        commands.delete_usb_backup(request)
    })
    .await)
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
        "migration",
        "migrate",
        tr::job_migrate_started(),
        tr::job_migrate_completed(),
        move |mut progress| {
            commands.run_data_migrations_with_progress(move |c, t, m| progress(c, t, m))
        },
    )
    .await
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
        tr::job_fetch_usb_playlists_started(),
        tr::job_fetch_usb_playlists_completed(),
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
        tr::job_fetch_usb_histories_started(),
        tr::job_fetch_usb_histories_completed(),
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
        tr::job_get_usb_player_menu_config_started(),
        tr::job_get_usb_player_menu_config_completed(),
        Some((40, tr::job_get_usb_player_menu_config_progress())),
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
        tr::job_update_usb_player_menu_config_started(),
        tr::job_update_usb_player_menu_config_completed(),
        Some((45, tr::job_update_usb_player_menu_config_progress())),
        move || commands.update_usb_player_menu_config(request),
    )
    .await
}

#[tauri::command]
pub async fn get_usb_background_color(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: GetUsbBackgroundColorRequest,
) -> Result<ApiResponse<UsbBackgroundColorData>, String> {
    let commands = state.inner().clone();
    run_usb_job(
        &app,
        "usb_read",
        "get_usb_background_color",
        tr::job_get_usb_background_color_started(),
        tr::job_get_usb_background_color_completed(),
        Some((40, tr::job_get_usb_background_color_progress())),
        move || commands.get_usb_background_color(request),
    )
    .await
}

#[tauri::command]
pub async fn set_usb_background_color(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: SetUsbBackgroundColorRequest,
) -> Result<ApiResponse<UsbBackgroundColorData>, String> {
    let commands = state.inner().clone();
    run_usb_job(
        &app,
        "usb_write",
        "set_usb_background_color",
        tr::job_set_usb_background_color_started(),
        tr::job_set_usb_background_color_completed(),
        Some((45, tr::job_set_usb_background_color_progress())),
        move || commands.set_usb_background_color(request),
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
        tr::job_remove_usb_playlist_started(),
        tr::job_remove_usb_playlist_completed(),
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
        tr::job_reorder_usb_playlists_started(),
        tr::job_reorder_usb_playlists_completed(),
        move |mut progress| {
            commands.reorder_usb_playlists_with_progress(request, move |c, t, m| {
                progress(c, t, m);
            })
        },
    )
    .await
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
        // A `start_paused` load/seek is a pause change, not a start: a
        // `playback.started` with `playing: false` would read as stopped.
        let event = if data.paused {
            PlaybackEvent::PauseChanged(PlaybackStatusData {
                path: Some(data.path.clone()),
                playing: false,
                paused: true,
                position_ms: data.position_ms,
                duration_ms: data.duration_ms,
            })
        } else {
            PlaybackEvent::Started {
                is_seek,
                path: data.path.clone(),
                playing: data.playing,
                position_ms: data.position_ms,
                duration_ms: data.duration_ms,
                track_id: data.track_id.clone(),
            }
        };
        emit_playback_event(&app, event);
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

/// Shared body of pause/resume: run on the playback thread, then broadcast the
/// resulting status so every listener sees the same backend-owned state.
async fn run_pause_change(
    app: AppHandle,
    commands: BackendCommands,
    label: &'static str,
    op: fn(&BackendCommands) -> ApiResponse<PlaybackStatusData>,
) -> Result<ApiResponse<PlaybackStatusData>, String> {
    let response = match run_playback_blocking(&app, label, move || op(&commands)).await {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    match response.data.as_ref() {
        // Only a loaded track has a pause state worth announcing; pausing when
        // idle is a quiet no-op.
        Some(data) if data.path.is_some() && (data.playing || data.paused) => {
            emit_playback_event(&app, PlaybackEvent::PauseChanged(data.clone()));
        }
        Some(_) => {}
        None => emit_playback_event(
            &app,
            PlaybackEvent::Error(response.error.as_ref().map(|e| e.message.clone())),
        ),
    }
    Ok(response)
}

#[tauri::command]
pub async fn pause_playback_native(
    app: AppHandle,
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<PlaybackStatusData>, String> {
    let commands = state.inner().clone();
    run_pause_change(
        app,
        commands,
        "pause",
        BackendCommands::pause_playback_native,
    )
    .await
}

#[tauri::command]
pub async fn resume_playback_native(
    app: AppHandle,
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<PlaybackStatusData>, String> {
    let commands = state.inner().clone();
    run_pause_change(
        app,
        commands,
        "resume",
        BackendCommands::resume_playback_native,
    )
    .await
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
pub async fn set_playback_metronome(
    state: State<'_, BackendCommands>,
    request: SetPlaybackMetronomeRequest,
) -> Result<ApiResponse<PlaybackMetronomeData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("playback metronome", move || {
        commands.set_playback_metronome(request)
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
pub async fn get_usb_history_tracklist_options(
    state: State<'_, BackendCommands>,
    request: GetUsbHistoryTracklistOptionsRequest,
) -> Result<ApiResponse<GetUsbHistoryTracklistOptionsData>, String> {
    let commands = state.inner().clone();
    Ok(
        run_blocking_command("get_usb_history_tracklist_options", move || {
            commands.get_usb_history_tracklist_options(request)
        })
        .await,
    )
}

#[tauri::command]
pub async fn build_usb_history_tracklist(
    state: State<'_, BackendCommands>,
    request: BuildUsbHistoryTracklistRequest,
) -> Result<ApiResponse<BuildUsbHistoryTracklistData>, String> {
    let commands = state.inner().clone();
    Ok(
        run_blocking_command("build_usb_history_tracklist", move || {
            commands.build_usb_history_tracklist(request)
        })
        .await,
    )
}

fn emit_analysis_event<R: tauri::Runtime>(app: &AppHandle<R>, event: &AnalysisEvent) {
    let payload = analysis_event_payload(event);
    let name = payload.event.clone();
    emit_job_payload(app, &name, payload);
}

/// Adds the request to the analysis job and returns at once; the job reports
/// the rest through its events.
#[tauri::command]
pub async fn analyze_new_tracks(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: AnalyzeNewTracksRequest,
) -> Result<ApiResponse<AnalysisSubmittedData>, String> {
    let commands = state.inner().clone();
    commands.set_analysis_observer(std::sync::Arc::new(move |event| {
        emit_analysis_event(&app, event);
    }));
    Ok(run_blocking_command("analyze_new_tracks", move || {
        commands.submit_analysis(request)
    })
    .await)
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
        job_type: "transcode",
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
                job_type: "transcode",
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
                tr::job_transcode_tracks_progress(progress.current, progress.total, file_name)
                    .render(),
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
        tr::job_export_to_usb_started(),
        tr::job_export_to_usb_completed(),
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
        tr::job_run_usb_diagnostics_started(),
        tr::job_run_usb_diagnostics_completed(),
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
        tr::job_run_usb_parity_report_started(),
        tr::job_run_usb_parity_report_completed(),
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
        tr::job_repair_usb_diagnostics_started(),
        tr::job_repair_usb_diagnostics_completed(),
        move |mut progress| {
            commands.repair_usb_diagnostics_with_progress(request, move |c, t, m| {
                progress(c, t, m);
            })
        },
    )
    .await
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
) -> Result<ApiResponse<ImportExternalPlaylistsData>, String> {
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
pub async fn detect_external_mixxx_db(
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<DetectExternalMixxxDbData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("detect_external_mixxx_db", move || {
        commands.detect_external_mixxx_db()
    })
    .await)
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
        tr::job_initialize_usb_started(),
        tr::job_initialize_usb_completed(),
        Some((40, tr::job_initialize_usb_progress())),
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

/// HTTP client for the backend's own downloads. reqwest is built without a
/// bundled TLS crypto provider (`rustls-no-provider`, as tauri-plugin-updater
/// uses) to keep the aws-lc C build out of the tree, so install rustls's ring
/// provider first; whichever of us or the updater runs first wins.
fn http_client() -> Result<reqwest::Client, String> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    reqwest::Client::builder()
        .user_agent(concat!("dj-usb-tkit/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
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
        let response = http_client()?
            .get(url)
            .send()
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
        let dep_bytes = http_client()?
            .get(dep_url)
            .send()
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
        crate::essentia_runtime::set_node_modules(node_modules_dir.clone());

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
                error: Some(crate::error::ErrorPayload::new(
                    crate::error::ErrorCode::InternalError,
                    msg,
                )),
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
pub async fn remove_essentia(state: State<'_, BackendCommands>) -> Result<ApiResponse<()>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("remove_essentia", move || commands.remove_essentia()).await)
}

const RELEASES_API_URL: &str =
    "https://api.github.com/repos/haivala/dj-usb-tkit/releases?per_page=10";

/// Which release asset the running build came from. The bundler patches a
/// marker into each packaged binary (`tauri::utils::platform::bundle_type`);
/// debug builds (`cargo run`) are always `Unknown` -- on macOS `bundle_type`
/// reports `App` even outside a bundle, and a dev build must never self-update.
fn current_install_kind() -> crate::service::update_check::InstallKind {
    use crate::service::update_check::InstallKind;
    use tauri::utils::{config::BundleType, platform::bundle_type};

    if cfg!(debug_assertions) {
        return InstallKind::Unknown;
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
        job_type: "update",
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

/// `RUNNING_JOBS`, locked, once no job but `own_job_id` is running.
fn lock_jobs_when_idle(own_job_id: &str) -> Option<MutexGuard<'static, BTreeSet<String>>> {
    let jobs = running_jobs();
    jobs.iter().all(|id| id == own_job_id).then_some(jobs)
}
