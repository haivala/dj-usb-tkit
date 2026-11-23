//! USB diagnostics, the parity report, and repairs.

use super::jobs::run_usb_job_with_progress;
use crate::commands::BackendCommands;
use crate::i18n::tr;
use crate::job_conflicts::JobType;
use crate::models::{
    ApiResponse, RepairUsbDiagnosticsData, RepairUsbDiagnosticsRequest, RunUsbDiagnosticsData,
    RunUsbDiagnosticsRequest, RunUsbParityReportData, RunUsbParityReportRequest,
};
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn run_usb_diagnostics(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: RunUsbDiagnosticsRequest,
) -> Result<ApiResponse<crate::models::WithPlaylists<RunUsbDiagnosticsData>>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        JobType::Diagnostics,
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
        JobType::Diagnostics,
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
) -> Result<ApiResponse<crate::models::WithPlaylists<RepairUsbDiagnosticsData>>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        JobType::Diagnostics,
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
