//! Analysis commands: analyzing new tracks, pausing and cancelling, and the
//! analysis events relayed to the UI.

use super::jobs::{emit_job_payload, run_blocking_command};
use crate::commands::BackendCommands;
use crate::models::{
    AnalysisSubmittedData, AnalyzeNewTracksRequest, ApiResponse, SetAnalysisPausedData,
    SetAnalysisPausedRequest,
};
use crate::service::{AnalysisEvent, analysis_event_payload};
use tauri::{AppHandle, State};

pub(super) fn emit_analysis_event<R: tauri::Runtime>(app: &AppHandle<R>, event: &AnalysisEvent) {
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
