//! Export commands: writing playlists to the USB.

use super::jobs::run_usb_job_with_progress;
use crate::commands::BackendCommands;
use crate::i18n::tr;
use crate::job_conflicts::JobType;
use crate::models::{ApiResponse, ExportToUsbData, ExportToUsbRequest};
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn export_to_usb(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: ExportToUsbRequest,
) -> Result<ApiResponse<crate::models::WithPlaylists<ExportToUsbData>>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        JobType::Export,
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
pub async fn export_playlists_to_usb(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: crate::models::ExportPlaylistsToUsbRequest,
) -> Result<
    ApiResponse<crate::models::WithPlaylists<crate::models::ExportPlaylistsToUsbData>>,
    String,
> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        JobType::Export,
        "export_playlists_to_usb",
        tr::job_export_playlists_to_usb_started(),
        tr::job_export_to_usb_completed(),
        move |mut progress| {
            commands.export_playlists_to_usb_with_progress(request, move |c, t, m| {
                progress(c, t, m);
            })
        },
    )
    .await
}
