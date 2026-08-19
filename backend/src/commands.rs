use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

use crate::error::BackendResult;
use crate::error::ErrorPayload;
use crate::models::{
    AddTrackCandidatesToPlaylistData, AddTrackCandidatesToPlaylistRequest, AddTracksToPlaylistData,
    AddTracksToPlaylistRequest, AnalyzeNewTracksData, AnalyzeNewTracksRequest, ApiResponse,
    BrowseSourceFilesData, BrowseSourceFilesRequest, CheckSourceRootsData, CheckSourceRootsRequest,
    CreatePlaylistData, CreatePlaylistRequest, DeletePlaylistData, DeletePlaylistRequest,
    DeleteUsbBackupData, DeleteUsbBackupRequest, DetectExternalMasterDbData, ExportToUsbData,
    ExportToUsbRequest, FetchUsbHistoriesData, FetchUsbHistoriesRequest, FetchUsbPlaylistsData,
    FetchUsbPlaylistsRequest, GetFrontendSettingsData, GetPlaylistTracksData,
    GetPlaylistTracksRequest, GetTracksByIdsData, GetTracksByIdsRequest, GetUsbDeviceNameData,
    GetUsbDeviceNameRequest, GetUsbPlayerMenuConfigData, GetUsbPlayerMenuConfigRequest,
    InitializeUsbData, InitializeUsbRequest, InspectUsbTrackData, InspectUsbTrackRequest,
    InspectUsbTracksData, InspectUsbTracksRequest, ListPlaylistsData, ListTracksData,
    ListTracksRequest, ListUsbBackupsData, ListUsbBackupsRequest, ListUsbDevicesData,
    MaterializeSourceTrackData, MaterializeSourceTrackRequest, MergeUsbPlaceholderTracksData,
    PlayResolvedTrackData, PlayResolvedTrackRequest, PlayTrackData, PlayTrackRequest,
    PlaybackPreflightData, PlaybackPreflightRequest, PlaybackStatusData, PruneUsbDeviceData,
    PruneUsbDeviceRequest, RelocateSourceRootData, RelocateSourceRootRequest,
    RemoveTracksBySourceRootsData, RemoveTracksBySourceRootsRequest, RemoveTracksFromPlaylistData,
    RemoveTracksFromPlaylistRequest, RemoveUsbPlaylistData, RemoveUsbPlaylistRequest,
    RenamePlaylistData, RenamePlaylistRequest, ReorderPlaylistTracksData,
    ReorderPlaylistTracksRequest, ReorderUsbPlaylistsData, ReorderUsbPlaylistsRequest,
    RepairUsbDiagnosticsData, RepairUsbDiagnosticsRequest, ResolvePlaybackSourceData,
    ResolvePlaybackSourceRequest, ResolveTrackIdentityData, ResolveTrackIdentityRequest,
    RestoreUsbBackupData, RestoreUsbBackupRequest, RunUsbDiagnosticsData, RunUsbDiagnosticsRequest,
    RunUsbParityReportData, RunUsbParityReportRequest, ScanLibraryData, ScanLibraryRequest,
    ScanMasterDbRequest, SearchTracksData, SearchTracksRequest, SetAnalysisPausedData,
    SetFrontendSettingData, SetFrontendSettingRequest, SetUsbDeviceNameData,
    SetUsbDeviceNameRequest, StopPlaybackData, UpdateUsbPlayerMenuConfigData,
    UpdateUsbPlayerMenuConfigRequest, ValidateUsbRootData, ValidateUsbRootRequest,
};
use crate::player::{PlaybackController, PlaybackTransition};
use crate::service::BackendService;

fn wrap<T: serde::Serialize>(result: BackendResult<T>) -> ApiResponse<T> {
    result
        .map(ApiResponse::success)
        .unwrap_or_else(|err| ApiResponse::failure(err.into()))
}

#[derive(Debug, Clone)]
pub struct BackendCommands {
    service: BackendService,
    playback: PlaybackController,
    playback_transitions: Arc<Mutex<Option<mpsc::Receiver<PlaybackTransition>>>>,
}

impl BackendCommands {
    pub fn new(data_dir: impl AsRef<Path>) -> Result<Self, ErrorPayload> {
        let service = BackendService::new(data_dir).map_err(ErrorPayload::from)?;
        let (playback, transition_rx) = PlaybackController::new();
        Ok(Self {
            service,
            playback,
            playback_transitions: Arc::new(Mutex::new(Some(transition_rx))),
        })
    }

    /// Hands off the natural-stop notification channel to whichever caller asks first
    /// (the Tauri transition relay, at startup). Returns `None` on every subsequent call —
    /// there is exactly one consumer for the process's lifetime.
    pub fn take_playback_transitions(&self) -> Option<mpsc::Receiver<PlaybackTransition>> {
        self.playback_transitions.lock().ok()?.take()
    }

    pub fn scan_library(&self, req: ScanLibraryRequest) -> ApiResponse<ScanLibraryData> {
        wrap(self.service.scan_library(req))
    }

    pub fn scan_master_db(&self, req: ScanMasterDbRequest) -> ApiResponse<ScanLibraryData> {
        wrap(self.service.scan_master_db(req))
    }

    pub fn search_tracks(&self, req: SearchTracksRequest) -> ApiResponse<SearchTracksData> {
        wrap(self.service.search_tracks(req))
    }

    pub fn list_tracks(&self, req: ListTracksRequest) -> ApiResponse<ListTracksData> {
        wrap(self.service.list_tracks(req))
    }

    pub fn browse_source_files(
        &self,
        req: BrowseSourceFilesRequest,
    ) -> ApiResponse<BrowseSourceFilesData> {
        wrap(self.service.browse_source_files(req))
    }

    pub fn check_source_roots(
        &self,
        req: CheckSourceRootsRequest,
    ) -> ApiResponse<CheckSourceRootsData> {
        wrap(self.service.check_source_roots(req))
    }

    pub fn materialize_source_track(
        &self,
        req: MaterializeSourceTrackRequest,
    ) -> ApiResponse<MaterializeSourceTrackData> {
        wrap(self.service.materialize_source_track(req))
    }

    pub fn resolve_track_identity(
        &self,
        req: ResolveTrackIdentityRequest,
    ) -> ApiResponse<ResolveTrackIdentityData> {
        wrap(self.service.resolve_track_identity(req))
    }

    pub fn remove_tracks_by_source_roots(
        &self,
        req: RemoveTracksBySourceRootsRequest,
    ) -> ApiResponse<RemoveTracksBySourceRootsData> {
        wrap(self.service.remove_tracks_by_source_roots(req))
    }

    pub fn relocate_source_root(
        &self,
        req: RelocateSourceRootRequest,
    ) -> ApiResponse<RelocateSourceRootData> {
        wrap(self.service.relocate_source_root(req))
    }

    pub fn get_tracks_by_ids_with_previews(
        &self,
        req: GetTracksByIdsRequest,
    ) -> ApiResponse<GetTracksByIdsData> {
        wrap(self.service.get_tracks_by_ids_with_previews(req))
    }

    pub fn resolve_playback_source(
        &self,
        req: ResolvePlaybackSourceRequest,
    ) -> ApiResponse<ResolvePlaybackSourceData> {
        wrap(self.service.resolve_playback_source(req))
    }

    pub fn create_playlist(&self, req: CreatePlaylistRequest) -> ApiResponse<CreatePlaylistData> {
        wrap(self.service.create_playlist(req))
    }

    pub fn rename_playlist(&self, req: RenamePlaylistRequest) -> ApiResponse<RenamePlaylistData> {
        wrap(self.service.rename_playlist(req))
    }

    pub fn delete_playlist(&self, req: DeletePlaylistRequest) -> ApiResponse<DeletePlaylistData> {
        wrap(self.service.delete_playlist(req))
    }

    pub fn list_playlists(&self) -> ApiResponse<ListPlaylistsData> {
        wrap(self.service.list_playlists())
    }

    pub fn get_playlist_tracks(
        &self,
        req: GetPlaylistTracksRequest,
    ) -> ApiResponse<GetPlaylistTracksData> {
        wrap(self.service.get_playlist_tracks(req))
    }

    pub fn add_tracks_to_playlist(
        &self,
        req: AddTracksToPlaylistRequest,
    ) -> ApiResponse<AddTracksToPlaylistData> {
        wrap(self.service.add_tracks_to_playlist(req))
    }

    pub fn add_track_candidates_to_playlist(
        &self,
        req: AddTrackCandidatesToPlaylistRequest,
    ) -> ApiResponse<AddTrackCandidatesToPlaylistData> {
        wrap(self.service.add_track_candidates_to_playlist(req))
    }

    pub fn remove_tracks_from_playlist(
        &self,
        req: RemoveTracksFromPlaylistRequest,
    ) -> ApiResponse<RemoveTracksFromPlaylistData> {
        wrap(self.service.remove_tracks_from_playlist(req))
    }

    pub fn reorder_playlist_tracks(
        &self,
        req: ReorderPlaylistTracksRequest,
    ) -> ApiResponse<ReorderPlaylistTracksData> {
        wrap(self.service.reorder_playlist_tracks(req))
    }

    pub fn get_frontend_settings(&self) -> ApiResponse<GetFrontendSettingsData> {
        wrap(self.service.get_frontend_settings())
    }

    pub fn set_frontend_setting(
        &self,
        req: SetFrontendSettingRequest,
    ) -> ApiResponse<SetFrontendSettingData> {
        wrap(self.service.set_frontend_setting(req))
    }

    pub fn remove_essentia(&self) -> ApiResponse<()> {
        wrap(self.service.remove_essentia())
    }

    pub fn data_dir(&self) -> std::path::PathBuf {
        self.service.db.data_dir()
    }

    pub fn validate_usb_root(
        &self,
        req: ValidateUsbRootRequest,
    ) -> ApiResponse<ValidateUsbRootData> {
        wrap(self.service.validate_usb_root(req))
    }

    pub fn list_usb_devices(&self) -> ApiResponse<ListUsbDevicesData> {
        wrap(self.service.list_usb_devices())
    }

    pub fn prune_usb_device(&self, req: PruneUsbDeviceRequest) -> ApiResponse<PruneUsbDeviceData> {
        wrap(self.service.prune_usb_device(req))
    }

    pub fn get_usb_device_name(
        &self,
        req: GetUsbDeviceNameRequest,
    ) -> ApiResponse<GetUsbDeviceNameData> {
        wrap(self.service.get_usb_device_name(req))
    }

    pub fn set_usb_device_name(
        &self,
        req: SetUsbDeviceNameRequest,
    ) -> ApiResponse<SetUsbDeviceNameData> {
        wrap(self.service.set_usb_device_name(req))
    }

    pub fn list_usb_backups(&self, req: ListUsbBackupsRequest) -> ApiResponse<ListUsbBackupsData> {
        wrap(self.service.list_usb_backups(req))
    }

    pub fn restore_usb_backup(
        &self,
        req: RestoreUsbBackupRequest,
    ) -> ApiResponse<RestoreUsbBackupData> {
        wrap(self.service.restore_usb_backup(req))
    }

    pub fn delete_usb_backup(
        &self,
        req: DeleteUsbBackupRequest,
    ) -> ApiResponse<DeleteUsbBackupData> {
        wrap(self.service.delete_usb_backup(req))
    }

    pub fn merge_orphaned_usb_placeholder_tracks(
        &self,
    ) -> ApiResponse<MergeUsbPlaceholderTracksData> {
        wrap(self.service.merge_orphaned_usb_placeholder_tracks())
    }

    pub fn fetch_usb_playlists(
        &self,
        req: FetchUsbPlaylistsRequest,
    ) -> ApiResponse<FetchUsbPlaylistsData> {
        wrap(self.service.fetch_usb_playlists(req))
    }

    pub fn fetch_usb_playlists_with_progress<F>(
        &self,
        req: FetchUsbPlaylistsRequest,
        on_progress: F,
    ) -> ApiResponse<FetchUsbPlaylistsData>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(
            self.service
                .fetch_usb_playlists_with_progress(req, on_progress),
        )
    }

    pub fn fetch_usb_histories(
        &self,
        req: FetchUsbHistoriesRequest,
    ) -> ApiResponse<FetchUsbHistoriesData> {
        wrap(self.service.fetch_usb_histories(req))
    }

    pub fn fetch_usb_histories_with_progress<F>(
        &self,
        req: FetchUsbHistoriesRequest,
        on_progress: F,
    ) -> ApiResponse<FetchUsbHistoriesData>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(
            self.service
                .fetch_usb_histories_with_progress(req, on_progress),
        )
    }

    pub fn get_usb_player_menu_config(
        &self,
        req: GetUsbPlayerMenuConfigRequest,
    ) -> ApiResponse<GetUsbPlayerMenuConfigData> {
        wrap(self.service.get_usb_player_menu_config(req))
    }

    pub fn update_usb_player_menu_config(
        &self,
        req: UpdateUsbPlayerMenuConfigRequest,
    ) -> ApiResponse<UpdateUsbPlayerMenuConfigData> {
        wrap(self.service.update_usb_player_menu_config(req))
    }

    pub fn sync_usb_player_menu_edb_to_pdb(
        &self,
        req: GetUsbPlayerMenuConfigRequest,
    ) -> ApiResponse<UpdateUsbPlayerMenuConfigData> {
        wrap(self.service.sync_usb_player_menu_edb_to_pdb(req))
    }

    pub fn remove_usb_playlist(
        &self,
        req: RemoveUsbPlaylistRequest,
    ) -> ApiResponse<RemoveUsbPlaylistData> {
        wrap(self.service.remove_usb_playlist(req))
    }

    pub fn remove_usb_playlist_with_progress<F>(
        &self,
        req: RemoveUsbPlaylistRequest,
        on_progress: F,
    ) -> ApiResponse<RemoveUsbPlaylistData>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(
            self.service
                .remove_usb_playlist_with_progress(req, on_progress),
        )
    }

    pub fn reorder_usb_playlists(
        &self,
        req: ReorderUsbPlaylistsRequest,
    ) -> ApiResponse<ReorderUsbPlaylistsData> {
        wrap(self.service.reorder_usb_playlists(req))
    }

    pub fn reorder_usb_playlists_with_progress<F>(
        &self,
        req: ReorderUsbPlaylistsRequest,
        on_progress: F,
    ) -> ApiResponse<ReorderUsbPlaylistsData>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(
            self.service
                .reorder_usb_playlists_with_progress(req, on_progress),
        )
    }

    pub fn inspect_usb_track(
        &self,
        req: InspectUsbTrackRequest,
    ) -> ApiResponse<InspectUsbTrackData> {
        wrap(self.service.inspect_usb_track(req))
    }

    pub fn inspect_usb_tracks(
        &self,
        req: InspectUsbTracksRequest,
    ) -> ApiResponse<InspectUsbTracksData> {
        wrap(self.service.inspect_usb_tracks(req))
    }

    pub fn analyze_new_tracks(
        &self,
        req: AnalyzeNewTracksRequest,
    ) -> ApiResponse<AnalyzeNewTracksData> {
        wrap(self.service.analyze_new_tracks(req))
    }

    pub fn analyze_new_tracks_with_progress<F>(
        &self,
        req: AnalyzeNewTracksRequest,
        on_progress: F,
    ) -> ApiResponse<AnalyzeNewTracksData>
    where
        F: FnMut(&crate::service::analysis::AnalyzeTrackProgress),
    {
        wrap(
            self.service
                .analyze_new_tracks_with_progress(req, on_progress),
        )
    }

    pub fn set_analysis_paused(&self, paused: bool) -> ApiResponse<SetAnalysisPausedData> {
        wrap(self.service.set_analysis_paused(paused))
    }

    pub fn cancel_analysis(&self) -> ApiResponse<()> {
        wrap(self.service.cancel_analysis())
    }

    pub fn export_to_usb(&self, req: ExportToUsbRequest) -> ApiResponse<ExportToUsbData> {
        wrap(self.service.export_to_usb(req))
    }

    pub fn export_to_usb_with_progress<F>(
        &self,
        req: ExportToUsbRequest,
        on_progress: F,
    ) -> ApiResponse<ExportToUsbData>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(self.service.export_to_usb_with_progress(req, on_progress))
    }

    pub fn play_track_native(&self, req: PlayTrackRequest) -> ApiResponse<PlayTrackData> {
        wrap(self.service.play_track_native(&self.playback, req))
    }

    pub fn play_resolved_track(
        &self,
        req: PlayResolvedTrackRequest,
    ) -> ApiResponse<PlayResolvedTrackData> {
        wrap(self.service.play_resolved_track(&self.playback, req))
    }

    pub fn stop_playback_native(&self) -> ApiResponse<StopPlaybackData> {
        wrap(self.service.stop_playback_native(&self.playback))
    }

    pub fn get_playback_status_native(&self) -> ApiResponse<PlaybackStatusData> {
        wrap(self.service.get_playback_status_native(&self.playback))
    }

    pub fn playback_preflight_native(
        &self,
        req: PlaybackPreflightRequest,
    ) -> ApiResponse<PlaybackPreflightData> {
        wrap(self.service.playback_preflight_native(req))
    }

    pub fn run_usb_diagnostics(
        &self,
        req: RunUsbDiagnosticsRequest,
    ) -> ApiResponse<RunUsbDiagnosticsData> {
        wrap(self.service.run_usb_diagnostics(req))
    }

    pub fn run_usb_diagnostics_with_progress<F>(
        &self,
        req: RunUsbDiagnosticsRequest,
        on_progress: F,
    ) -> ApiResponse<RunUsbDiagnosticsData>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(
            self.service
                .run_usb_diagnostics_with_progress(req, on_progress),
        )
    }

    pub fn run_usb_parity_report(
        &self,
        req: RunUsbParityReportRequest,
    ) -> ApiResponse<RunUsbParityReportData> {
        wrap(self.service.run_usb_parity_report(req))
    }

    pub fn run_usb_parity_report_with_progress<F>(
        &self,
        req: RunUsbParityReportRequest,
        on_progress: F,
    ) -> ApiResponse<RunUsbParityReportData>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(
            self.service
                .run_usb_parity_report_with_progress(req, on_progress),
        )
    }

    pub fn repair_usb_diagnostics(
        &self,
        req: RepairUsbDiagnosticsRequest,
    ) -> ApiResponse<RepairUsbDiagnosticsData> {
        wrap(self.service.repair_usb_diagnostics(req))
    }

    pub fn repair_usb_diagnostics_with_progress<F>(
        &self,
        req: RepairUsbDiagnosticsRequest,
        on_progress: F,
    ) -> ApiResponse<RepairUsbDiagnosticsData>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(
            self.service
                .repair_usb_diagnostics_with_progress(req, on_progress),
        )
    }

    pub fn detect_external_master_db(&self) -> ApiResponse<DetectExternalMasterDbData> {
        wrap(self.service.detect_external_master_db())
    }

    pub fn initialize_usb(&self, req: InitializeUsbRequest) -> ApiResponse<InitializeUsbData> {
        wrap(self.service.initialize_usb(req))
    }
}
