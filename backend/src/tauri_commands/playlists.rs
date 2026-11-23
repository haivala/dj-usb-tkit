//! Playlist commands: creating and ordering playlists, their tracks and
//! membership, and the export space and duration of a selection.

use super::jobs::run_blocking_command;
use crate::commands::BackendCommands;
use crate::models::{
    AddLibrarySelectionToPlaylistRequest, AddLibrarySelectionToPlaylistsRequest,
    AddTrackCandidatesToPlaylistData, AddTrackCandidatesToPlaylistRequest,
    AddTrackCandidatesToPlaylistsRequest, AddTracksToPlaylistData, AddTracksToPlaylistsData,
    ApiResponse, CreatePlaylistData, CreatePlaylistRequest, DeletePlaylistData,
    DeletePlaylistRequest, GetPlaylistExportSpaceData, GetPlaylistExportSpaceRequest,
    GetPlaylistMembershipData, GetPlaylistMembershipRequest, GetPlaylistTracksData,
    GetPlaylistTracksRequest, GetSelectionDurationData, GetSelectionDurationRequest,
    RemoveTracksFromPlaylistData, RemoveTracksFromPlaylistRequest, RenamePlaylistData,
    RenamePlaylistRequest, ReorderPlaylistTracksData, ReorderPlaylistTracksRequest,
    ReorderPlaylistsData, ReorderPlaylistsRequest,
};
use tauri::State;

#[tauri::command]
pub async fn create_playlist(
    state: State<'_, BackendCommands>,
    request: CreatePlaylistRequest,
) -> Result<ApiResponse<crate::models::WithPlaylists<CreatePlaylistData>>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("create_playlist", move || commands.create_playlist(request)).await)
}

#[tauri::command]
pub async fn rename_playlist(
    state: State<'_, BackendCommands>,
    request: RenamePlaylistRequest,
) -> Result<ApiResponse<crate::models::WithPlaylists<RenamePlaylistData>>, String> {
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
) -> Result<ApiResponse<crate::models::WithPlaylists<DeletePlaylistData>>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("delete_playlist", move || commands.delete_playlist(request)).await)
}

#[tauri::command]
pub async fn list_playlists(
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<crate::models::PlaylistsState>, String> {
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
pub async fn get_playlists_export_space(
    state: State<'_, BackendCommands>,
    request: crate::models::GetPlaylistsExportSpaceRequest,
) -> Result<ApiResponse<GetPlaylistExportSpaceData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("get_playlists_export_space", move || {
        commands.get_playlists_export_space(request)
    })
    .await)
}

#[tauri::command]
pub async fn get_batch_export_overview(
    state: State<'_, BackendCommands>,
    request: crate::models::GetBatchExportOverviewRequest,
) -> Result<ApiResponse<crate::models::BatchExportOverviewData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("get_batch_export_overview", move || {
        commands.get_batch_export_overview(request)
    })
    .await)
}

#[tauri::command]
pub async fn add_track_candidates_to_playlist(
    state: State<'_, BackendCommands>,
    request: AddTrackCandidatesToPlaylistRequest,
) -> Result<ApiResponse<crate::models::WithPlaylists<AddTrackCandidatesToPlaylistData>>, String> {
    let commands = state.inner().clone();
    Ok(
        run_blocking_command("add_track_candidates_to_playlist", move || {
            commands.add_track_candidates_to_playlist(request)
        })
        .await,
    )
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
) -> Result<ApiResponse<crate::models::WithPlaylists<AddTracksToPlaylistData>>, String> {
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
) -> Result<ApiResponse<crate::models::WithPlaylists<AddTracksToPlaylistsData>>, String> {
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
) -> Result<ApiResponse<crate::models::WithPlaylists<AddTracksToPlaylistsData>>, String> {
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
) -> Result<ApiResponse<crate::models::WithPlaylists<RemoveTracksFromPlaylistData>>, String> {
    let commands = state.inner().clone();
    Ok(
        run_blocking_command("remove_tracks_from_playlist", move || {
            commands.remove_tracks_from_playlist(request)
        })
        .await,
    )
}

#[tauri::command]
pub async fn reorder_playlist_tracks(
    state: State<'_, BackendCommands>,
    request: ReorderPlaylistTracksRequest,
) -> Result<ApiResponse<crate::models::WithPlaylists<ReorderPlaylistTracksData>>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("reorder_playlist_tracks", move || {
        commands.reorder_playlist_tracks(request)
    })
    .await)
}
