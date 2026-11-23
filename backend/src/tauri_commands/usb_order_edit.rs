//! The USB view's Edit order commands.

use super::jobs::{emit_blocked_actions, run_blocking_command, run_usb_job_with_progress};
use crate::commands::BackendCommands;
use crate::i18n::tr;
use crate::job_conflicts::JobType;
use crate::models::ApiResponse;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn begin_usb_order_edit(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: crate::models::UsbOrderEditRequest,
) -> Result<ApiResponse<crate::models::UsbOrderEditData>, String> {
    let commands = state.inner().clone();
    let response = run_blocking_command("begin_usb_order_edit", move || {
        commands.begin_usb_order_edit(request)
    })
    .await;
    emit_blocked_actions(&app);
    Ok(response)
}

#[tauri::command]
pub async fn move_usb_playlist(
    state: State<'_, BackendCommands>,
    request: crate::models::MoveUsbPlaylistRequest,
) -> Result<ApiResponse<crate::models::UsbOrderEditData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("move_usb_playlist", move || {
        commands.move_usb_playlist(request)
    })
    .await)
}

#[tauri::command]
pub async fn move_usb_playlist_track(
    state: State<'_, BackendCommands>,
    request: crate::models::MoveUsbPlaylistTrackRequest,
) -> Result<ApiResponse<crate::models::UsbOrderEditData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("move_usb_playlist_track", move || {
        commands.move_usb_playlist_track(request)
    })
    .await)
}

#[tauri::command]
pub async fn sort_usb_playlist_tracks(
    state: State<'_, BackendCommands>,
    request: crate::models::SortUsbPlaylistTracksRequest,
) -> Result<ApiResponse<crate::models::UsbOrderEditData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("sort_usb_playlist_tracks", move || {
        commands.sort_usb_playlist_tracks(request)
    })
    .await)
}

#[tauri::command]
pub async fn cancel_usb_order_edit(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: crate::models::UsbOrderEditRequest,
) -> Result<ApiResponse<crate::models::UsbOrderEditData>, String> {
    let commands = state.inner().clone();
    let response = run_blocking_command("cancel_usb_order_edit", move || {
        commands.cancel_usb_order_edit(request)
    })
    .await;
    emit_blocked_actions(&app);
    Ok(response)
}

#[tauri::command]
pub async fn commit_usb_order_edit(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: crate::models::UsbOrderEditRequest,
) -> Result<ApiResponse<crate::models::UsbOrderEditData>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        JobType::UsbWrite,
        "commit_usb_order_edit",
        tr::job_commit_usb_order_edit_started(),
        tr::job_commit_usb_order_edit_completed(),
        move |mut progress| {
            commands.commit_usb_order_edit_with_progress(request, move |c, t, m| {
                progress(c, t, m);
            })
        },
    )
    .await
}
