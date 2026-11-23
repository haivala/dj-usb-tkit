//! Library commands: source folders and scanning them, a track's details
//! and edits, and the asset scope the webview may read files from.

use super::jobs::{JobIdentity, emit_job_event, emit_job_failed, run_blocking_command};
use crate::commands::BackendCommands;
use crate::i18n::tr;
use crate::job_conflicts::JobType;
use crate::models::{
    AddSourceFoldersData, AddSourceFoldersRequest, ApiResponse, BrowseSourceFilesData,
    BrowseSourceFilesRequest, GetTrackDetailRequest, GetTracksMetadataSummaryRequest,
    LibrarySourcesData, ListMatchingTrackIdsData, ListMatchingTrackIdsRequest,
    RelocateSourceRootData, RelocateSourceRootRequest, RemoveSourceFolderData,
    RemoveSourceFolderRequest, ResolveTrackIdentityData, ResolveTrackIdentityRequest,
    SaveTrackAnalysisEditsData, SaveTrackAnalysisEditsRequest, SaveTrackMetadataEditsData,
    SaveTrackMetadataEditsRequest, ScanLibraryData, ScanLibraryRequest,
    SetExternalLibraryEnabledRequest, SetSourceFolderEnabledRequest, TrackDetail,
    TracksMetadataSummary,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

#[tauri::command]
pub async fn scan_library(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: ScanLibraryRequest,
) -> Result<ApiResponse<ScanLibraryData>, String> {
    let job_id = Uuid::now_v7().to_string();
    let identity = JobIdentity {
        job_id: &job_id,
        job_type: JobType::Scan,
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

/// The indexing a source-folder command runs (adding a folder, relocating
/// one), reported as a scan job like `scan_library`'s: started when the scan
/// starts, finished only if it did.
pub(super) struct SourceFolderScanJob {
    app: AppHandle,
    job_id: String,
    running: AtomicBool,
}

impl SourceFolderScanJob {
    fn new(app: AppHandle) -> Self {
        Self {
            app,
            job_id: Uuid::now_v7().to_string(),
            running: AtomicBool::new(false),
        }
    }

    fn identity(&self) -> JobIdentity<'_> {
        JobIdentity {
            job_id: &self.job_id,
            job_type: JobType::Scan,
            stage: "scan_library",
        }
    }

    fn started(&self) {
        self.running.store(true, Ordering::SeqCst);
        emit_job_event(
            &self.app,
            "job.started",
            self.identity(),
            0,
            1,
            0,
            tr::job_scan_library_started().render(),
        );
        emit_job_event(
            &self.app,
            "job.progress",
            self.identity(),
            0,
            1,
            30,
            tr::job_scan_library_progress().render(),
        );
    }

    /// `scan` is the finished scan, `None` when it failed with `failure`.
    fn finished(&self, scan: Option<&ScanLibraryData>, failure: Option<String>) {
        if !self.running.load(Ordering::SeqCst) {
            return;
        }
        match scan {
            Some(scan) => emit_job_event(
                &self.app,
                "job.completed",
                self.identity(),
                1,
                1,
                100,
                tr::job_scan_library_completed_counts(scan.indexed, scan.updated, scan.removed)
                    .render(),
            ),
            None => emit_job_failed(&self.app, self.identity(), failure),
        }
    }
}

#[tauri::command]
pub async fn add_source_folders(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: AddSourceFoldersRequest,
) -> Result<ApiResponse<AddSourceFoldersData>, String> {
    let job = Arc::new(SourceFolderScanJob::new(app.clone()));
    let commands = state.inner().clone();
    let scan_job = Arc::clone(&job);
    let response = run_blocking_command("add_source_folders", move || {
        commands.add_source_folders_with_progress(request, || scan_job.started())
    })
    .await;
    if let Some(data) = &response.data {
        allow_library_sources_asset_scope(&app, &data.sources);
    }
    job.finished(
        response.data.as_ref().and_then(|data| data.scan.as_ref()),
        response
            .data
            .as_ref()
            .map(|data| data.status_text.clone())
            .or_else(|| response.error.as_ref().map(|e| e.message.clone())),
    );
    Ok(response)
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

#[tauri::command]
pub async fn relocate_source_root(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: RelocateSourceRootRequest,
) -> Result<ApiResponse<RelocateSourceRootData>, String> {
    let job = Arc::new(SourceFolderScanJob::new(app.clone()));
    let commands = state.inner().clone();
    let scan_job = Arc::clone(&job);
    let response = run_blocking_command("relocate_source_root", move || {
        commands.relocate_source_root_with_progress(request, || scan_job.started())
    })
    .await;
    if let Some(data) = &response.data {
        allow_library_sources_asset_scope(&app, &data.sources);
    }
    job.finished(
        response.data.as_ref().and_then(|data| data.scan.as_ref()),
        response
            .data
            .as_ref()
            .map(|data| data.status_text.clone())
            .or_else(|| response.error.as_ref().map(|e| e.message.clone())),
    );
    Ok(response)
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

/// Lets the window load files (covers, artwork) from each of `paths` and below: the asset
/// protocol's scope. Only folders the backend itself holds go in, the library's source
/// folders and a validated USB, never a path the page names. `tauri.conf.json` covers the
/// usual places; this adds the ones elsewhere, such as another drive.
pub fn allow_asset_scope<P: AsRef<std::path::Path>>(
    app: &AppHandle,
    paths: impl IntoIterator<Item = P>,
) {
    let scope = app.asset_protocol_scope();
    for path in paths {
        let path = path.as_ref();
        let candidate = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Err(err) = scope.allow_directory(&candidate, true) {
            crate::logging::emit(
                crate::logging::Level::Warn,
                "asset-scope",
                &format!(
                    "allow asset scope failed for {}: {err}",
                    candidate.display()
                ),
            );
        }
    }
}

/// The library's source folders into the asset scope: at startup, and after a command that
/// can add one.
pub fn allow_library_sources_asset_scope(
    app: &AppHandle,
    sources: &crate::models::LibrarySourcesData,
) {
    allow_asset_scope(app, sources.folders.iter().map(|folder| &folder.path));
}
