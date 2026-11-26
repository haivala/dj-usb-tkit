//! USB commands: selecting and naming a stick, its playlists, histories and
//! tracks, its settings and backups, and initializing it.

use super::jobs::{run_blocking_command, run_usb_job, run_usb_job_with_progress};
use super::library::allow_asset_scope;
use crate::commands::BackendCommands;
use crate::i18n::tr;
use crate::job_conflicts::JobType;
use crate::models::{
    ApiResponse, BuildUsbHistoryTracklistData, BuildUsbHistoryTracklistRequest,
    DeleteUsbBackupData, DeleteUsbBackupRequest, FetchUsbHistoriesData, FetchUsbHistoriesRequest,
    FetchUsbPlaylistsData, FetchUsbPlaylistsRequest, FetchUsbTracksData, FetchUsbTracksRequest,
    GetUsbBackgroundColorRequest, GetUsbHistoryTracklistOptionsData,
    GetUsbHistoryTracklistOptionsRequest, GetUsbPlayerMenuConfigData,
    GetUsbPlayerMenuConfigRequest, GetUsbTrackDetailRequest, GetUsbTracksMetadataSummaryRequest,
    InitializeUsbData, InitializeUsbRequest, InspectUsbTrackData, InspectUsbTrackRequest,
    ListUsbBackupsData, ListUsbBackupsRequest, ListUsbDevicesData, PruneUsbDeviceData,
    PruneUsbDeviceRequest, RemoveUsbPlaylistData, RemoveUsbPlaylistRequest, RestoreUsbBackupData,
    RestoreUsbBackupRequest, SaveUsbTrackAnalysisEditsData, SaveUsbTrackAnalysisEditsRequest,
    SaveUsbTrackMetadataEditsData, SaveUsbTrackMetadataEditsRequest, SetUsbBackgroundColorRequest,
    SetUsbDeviceNameData, SetUsbDeviceNameRequest, TracksMetadataSummary,
    UpdateUsbPlayerMenuConfigData, UpdateUsbPlayerMenuConfigRequest, UsbBackgroundColorData,
    UsbTrackAnalysisDetail,
};
use tauri::{AppHandle, State};

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
        JobType::UsbWrite,
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
        JobType::UsbWrite,
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
pub async fn select_usb(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: crate::models::SelectUsbRequest,
) -> Result<ApiResponse<crate::models::WithPlaylists<crate::models::SelectUsbData>>, String> {
    let commands = state.inner().clone();
    let response = run_usb_job(
        &app,
        JobType::UsbRead,
        "select_usb",
        tr::job_validate_usb_root_started(),
        tr::job_validate_usb_root_completed(),
        Some((30, tr::job_validate_usb_root_progress())),
        move || commands.select_usb(request),
    )
    .await?;
    if let Some(root) = response
        .data
        .as_ref()
        .filter(|data| data.data.usb.valid)
        .and_then(|data| data.data.usb.normalized_root.as_deref())
    {
        allow_asset_scope(&app, [root]);
    }
    Ok(response)
}

#[tauri::command]
pub async fn list_usb_devices(
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<ListUsbDevicesData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("list_usb_devices", move || commands.list_usb_devices()).await)
}

#[tauri::command]
pub async fn preview_prune_usb_device(
    state: State<'_, BackendCommands>,
    request: PruneUsbDeviceRequest,
) -> Result<ApiResponse<crate::models::PreviewPruneUsbDeviceData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("preview_prune_usb_device", move || {
        commands.preview_prune_usb_device(request)
    })
    .await)
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
pub async fn set_export_sync_mode(
    state: State<'_, BackendCommands>,
    request: crate::models::SetExportSyncModeRequest,
) -> Result<ApiResponse<crate::models::WithPlaylists<crate::models::SetExportSyncModeData>>, String>
{
    let commands = state.inner().clone();
    Ok(run_blocking_command("set_export_sync_mode", move || {
        commands.set_export_sync_mode(request)
    })
    .await)
}

#[tauri::command]
pub async fn set_usb_device_name(
    state: State<'_, BackendCommands>,
    request: SetUsbDeviceNameRequest,
) -> Result<ApiResponse<crate::models::WithPlaylists<SetUsbDeviceNameData>>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("set_usb_device_name", move || {
        commands.set_usb_device_name(request)
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
) -> Result<ApiResponse<crate::models::WithPlaylists<RestoreUsbBackupData>>, String> {
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

#[tauri::command]
pub async fn fetch_usb_playlists(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: FetchUsbPlaylistsRequest,
) -> Result<ApiResponse<crate::models::WithPlaylists<FetchUsbPlaylistsData>>, String> {
    let commands = state.inner().clone();
    run_usb_job_with_progress(
        &app,
        JobType::UsbRead,
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
        JobType::UsbRead,
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
        JobType::UsbRead,
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
        JobType::UsbWrite,
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
        JobType::UsbRead,
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
        JobType::UsbWrite,
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
        JobType::UsbWrite,
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

#[tauri::command]
pub async fn initialize_usb(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: InitializeUsbRequest,
) -> Result<ApiResponse<InitializeUsbData>, String> {
    let commands = state.inner().clone();
    run_usb_job(
        &app,
        JobType::UsbWrite,
        "initialize_usb",
        tr::job_initialize_usb_started(),
        tr::job_initialize_usb_completed(),
        Some((40, tr::job_initialize_usb_progress())),
        move || commands.initialize_usb(request),
    )
    .await
}
