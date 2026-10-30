use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

use crate::error::BackendResult;
use crate::error::ErrorPayload;
use crate::models::{
    AddLibrarySelectionToPlaylistRequest, AddLibrarySelectionToPlaylistsRequest,
    AddSourceFoldersData, AddSourceFoldersRequest, AddTrackCandidatesToPlaylistData,
    AddTrackCandidatesToPlaylistRequest, AddTrackCandidatesToPlaylistsRequest,
    AddTracksToPlaylistData, AddTracksToPlaylistsData, AnalyzeNewTracksData,
    AnalyzeNewTracksRequest, ApiResponse, BrowseSourceFilesData, BrowseSourceFilesRequest,
    CreatePlaylistData, CreatePlaylistRequest, DeletePlaylistData, DeletePlaylistRequest,
    DeleteUsbBackupData, DeleteUsbBackupRequest, DetectExternalMixxxDbData,
    DetectExternalRekordboxDbData, ExportToUsbData, ExportToUsbRequest, FetchUsbHistoriesData,
    FetchUsbHistoriesRequest, FetchUsbPlaylistsData, FetchUsbPlaylistsRequest, FetchUsbTracksData,
    FetchUsbTracksRequest, GetFrontendSettingsData, GetPlaylistExportSpaceData,
    GetPlaylistExportSpaceRequest, GetPlaylistMembershipData, GetPlaylistMembershipRequest,
    GetPlaylistTracksData, GetPlaylistTracksRequest, GetSelectionDurationData,
    GetSelectionDurationRequest, GetTrackDetailRequest, GetTracksMetadataSummaryRequest,
    GetUsbDeviceNameData, GetUsbDeviceNameRequest, GetUsbPlayerMenuConfigData,
    GetUsbPlayerMenuConfigRequest, GetUsbTrackDetailRequest, GetUsbTracksMetadataSummaryRequest,
    ImportExternalPlaylistsData, ImportExternalPlaylistsRequest, InitializeUsbData,
    InitializeUsbRequest, InspectUsbTrackData, InspectUsbTrackRequest, LibrarySourcesData,
    ListExternalPlaylistsData, ListExternalPlaylistsRequest, ListMatchingTrackIdsData,
    ListMatchingTrackIdsRequest, ListPlaylistsData, ListUsbBackupsData, ListUsbBackupsRequest,
    ListUsbDevicesData, MaterializeSourceTrackData, MaterializeSourceTrackRequest,
    PlayResolvedTrackData, PlayResolvedTrackRequest, PlaybackMetronomeData, PlaybackStatusData,
    PruneUsbDeviceData, PruneUsbDeviceRequest, RelocateSourceRootData, RelocateSourceRootRequest,
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
    SetExternalLibraryEnabledRequest, SetFrontendSettingData, SetFrontendSettingRequest,
    SetPlaybackMetronomeRequest, SetSourceFolderEnabledRequest, SetUsbDeviceNameData,
    SetUsbDeviceNameRequest, StopPlaybackData, TrackDetail, TracksMetadataSummary,
    UpdateUsbPlayerMenuConfigData, UpdateUsbPlayerMenuConfigRequest, UsbTrackAnalysisDetail,
    ValidateUsbRootData, ValidateUsbRootRequest,
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

    pub fn scan_rekordbox_db(&self, req: ScanRekordboxDbRequest) -> ApiResponse<ScanLibraryData> {
        wrap(self.service.scan_rekordbox_db(req))
    }

    pub fn list_rekordbox_playlists(
        &self,
        req: ListExternalPlaylistsRequest,
    ) -> ApiResponse<ListExternalPlaylistsData> {
        wrap(self.service.list_rekordbox_playlists(req))
    }

    pub fn scan_mixxx_db(&self, req: ScanMixxxDbRequest) -> ApiResponse<ScanLibraryData> {
        wrap(self.service.scan_mixxx_db(req))
    }

    pub fn list_mixxx_playlists(
        &self,
        req: ListExternalPlaylistsRequest,
    ) -> ApiResponse<ListExternalPlaylistsData> {
        wrap(self.service.list_mixxx_playlists(req))
    }

    pub fn import_external_playlists(
        &self,
        req: ImportExternalPlaylistsRequest,
    ) -> ApiResponse<ImportExternalPlaylistsData> {
        wrap(self.service.import_external_playlists(req))
    }

    pub fn browse_source_files(
        &self,
        req: BrowseSourceFilesRequest,
    ) -> ApiResponse<BrowseSourceFilesData> {
        wrap(self.service.browse_source_files(req))
    }

    pub fn get_library_sources(&self) -> ApiResponse<LibrarySourcesData> {
        wrap(self.service.get_library_sources())
    }

    pub fn add_source_folders(
        &self,
        req: AddSourceFoldersRequest,
    ) -> ApiResponse<AddSourceFoldersData> {
        wrap(self.service.add_source_folders(req))
    }

    pub fn remove_source_folder(
        &self,
        req: RemoveSourceFolderRequest,
    ) -> ApiResponse<RemoveSourceFolderData> {
        wrap(self.service.remove_source_folder(req))
    }

    pub fn set_source_folder_enabled(
        &self,
        req: SetSourceFolderEnabledRequest,
    ) -> ApiResponse<LibrarySourcesData> {
        wrap(self.service.set_source_folder_enabled(req))
    }

    pub fn set_external_library_enabled(
        &self,
        req: SetExternalLibraryEnabledRequest,
    ) -> ApiResponse<LibrarySourcesData> {
        wrap(self.service.set_external_library_enabled(req))
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

    pub fn get_track_detail(&self, req: GetTrackDetailRequest) -> ApiResponse<TrackDetail> {
        wrap(self.service.get_track_detail(req))
    }

    pub fn save_track_analysis_edits(
        &self,
        req: SaveTrackAnalysisEditsRequest,
    ) -> ApiResponse<SaveTrackAnalysisEditsData> {
        wrap(self.service.save_track_analysis_edits(req))
    }

    pub fn save_track_metadata_edits(
        &self,
        req: SaveTrackMetadataEditsRequest,
    ) -> ApiResponse<SaveTrackMetadataEditsData> {
        wrap(self.service.save_track_metadata_edits(req))
    }

    pub fn get_tracks_metadata_summary(
        &self,
        req: GetTracksMetadataSummaryRequest,
    ) -> ApiResponse<TracksMetadataSummary> {
        wrap(self.service.get_tracks_metadata_summary(req))
    }

    pub fn get_usb_tracks_metadata_summary(
        &self,
        req: GetUsbTracksMetadataSummaryRequest,
    ) -> ApiResponse<TracksMetadataSummary> {
        wrap(self.service.get_usb_tracks_metadata_summary(req))
    }

    pub fn save_usb_track_metadata_edits_with_progress<F>(
        &self,
        req: SaveUsbTrackMetadataEditsRequest,
        on_progress: F,
    ) -> ApiResponse<SaveUsbTrackMetadataEditsData>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(
            self.service
                .save_usb_track_metadata_edits_with_progress(req, on_progress),
        )
    }

    pub fn get_usb_track_detail(
        &self,
        req: GetUsbTrackDetailRequest,
    ) -> ApiResponse<UsbTrackAnalysisDetail> {
        wrap(self.service.get_usb_track_detail(req))
    }

    pub fn save_usb_track_analysis_edits(
        &self,
        req: SaveUsbTrackAnalysisEditsRequest,
    ) -> ApiResponse<SaveUsbTrackAnalysisEditsData> {
        wrap(self.service.save_usb_track_analysis_edits(req))
    }

    pub fn save_usb_track_analysis_edits_with_progress<F>(
        &self,
        req: SaveUsbTrackAnalysisEditsRequest,
        on_progress: F,
    ) -> ApiResponse<SaveUsbTrackAnalysisEditsData>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(
            self.service
                .save_usb_track_analysis_edits_with_progress(req, on_progress),
        )
    }

    pub fn relocate_source_root(
        &self,
        req: RelocateSourceRootRequest,
    ) -> ApiResponse<RelocateSourceRootData> {
        wrap(self.service.relocate_source_root(req))
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

    pub fn reorder_playlists(
        &self,
        req: ReorderPlaylistsRequest,
    ) -> ApiResponse<ReorderPlaylistsData> {
        wrap(self.service.reorder_playlists(req))
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

    pub fn get_playlist_export_space(
        &self,
        req: GetPlaylistExportSpaceRequest,
    ) -> ApiResponse<GetPlaylistExportSpaceData> {
        wrap(self.service.get_playlist_export_space(req))
    }

    pub fn add_track_candidates_to_playlist(
        &self,
        req: AddTrackCandidatesToPlaylistRequest,
    ) -> ApiResponse<AddTrackCandidatesToPlaylistData> {
        wrap(self.service.add_track_candidates_to_playlist(req))
    }

    pub fn forget_source_folder_listings(&self) {
        self.service.forget_source_folder_listings();
    }

    pub fn list_matching_track_ids(
        &self,
        req: ListMatchingTrackIdsRequest,
    ) -> ApiResponse<ListMatchingTrackIdsData> {
        wrap(self.service.list_matching_track_ids(req))
    }

    pub fn get_selection_duration(
        &self,
        req: GetSelectionDurationRequest,
    ) -> ApiResponse<GetSelectionDurationData> {
        wrap(self.service.get_selection_duration(req))
    }

    pub fn add_library_selection_to_playlist(
        &self,
        req: AddLibrarySelectionToPlaylistRequest,
    ) -> ApiResponse<AddTracksToPlaylistData> {
        wrap(self.service.add_library_selection_to_playlist(req))
    }

    pub fn add_library_selection_to_playlists(
        &self,
        req: AddLibrarySelectionToPlaylistsRequest,
    ) -> ApiResponse<AddTracksToPlaylistsData> {
        wrap(self.service.add_library_selection_to_playlists(req))
    }

    pub fn add_track_candidates_to_playlists(
        &self,
        req: AddTrackCandidatesToPlaylistsRequest,
    ) -> ApiResponse<AddTracksToPlaylistsData> {
        wrap(self.service.add_track_candidates_to_playlists(req))
    }

    pub fn get_playlist_membership(
        &self,
        req: GetPlaylistMembershipRequest,
    ) -> ApiResponse<GetPlaylistMembershipData> {
        wrap(self.service.get_playlist_membership(req))
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

    /// Ids of the background data migrations still to run.
    pub fn pending_data_migrations(&self) -> ApiResponse<Vec<String>> {
        wrap(self.service.pending_data_migrations())
    }

    pub fn run_data_migrations_with_progress<F>(
        &self,
        on_progress: F,
    ) -> ApiResponse<RunDataMigrationsData>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(self.service.run_data_migrations(on_progress))
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

    pub fn fetch_usb_playlist_tracks(
        &self,
        req: FetchUsbTracksRequest,
    ) -> ApiResponse<FetchUsbTracksData> {
        wrap(self.service.fetch_usb_playlist_tracks(req))
    }

    pub fn fetch_usb_history_tracks(
        &self,
        req: FetchUsbTracksRequest,
    ) -> ApiResponse<FetchUsbTracksData> {
        wrap(self.service.fetch_usb_history_tracks(req))
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

    pub fn refresh_playlist_export_status(
        &self,
        req: crate::models::RefreshPlaylistExportStatusRequest,
    ) -> ApiResponse<crate::models::RefreshPlaylistExportStatusData> {
        wrap(self.service.refresh_playlist_export_status(req))
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

    pub fn play_resolved_track(
        &self,
        req: PlayResolvedTrackRequest,
    ) -> ApiResponse<PlayResolvedTrackData> {
        wrap(self.service.play_resolved_track(&self.playback, req))
    }

    pub fn stop_playback_native(&self) -> ApiResponse<StopPlaybackData> {
        wrap(self.service.stop_playback_native(&self.playback))
    }

    pub fn pause_playback_native(&self) -> ApiResponse<PlaybackStatusData> {
        wrap(self.service.pause_playback_native(&self.playback))
    }

    pub fn resume_playback_native(&self) -> ApiResponse<PlaybackStatusData> {
        wrap(self.service.resume_playback_native(&self.playback))
    }

    pub fn get_playback_status_native(&self) -> ApiResponse<PlaybackStatusData> {
        wrap(self.service.get_playback_status_native(&self.playback))
    }

    pub fn set_playback_metronome(
        &self,
        req: SetPlaybackMetronomeRequest,
    ) -> ApiResponse<PlaybackMetronomeData> {
        wrap(self.service.set_playback_metronome(&self.playback, req))
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

    pub fn detect_external_rekordbox_db(&self) -> ApiResponse<DetectExternalRekordboxDbData> {
        wrap(self.service.detect_external_rekordbox_db())
    }

    pub fn detect_external_mixxx_db(&self) -> ApiResponse<DetectExternalMixxxDbData> {
        wrap(self.service.detect_external_mixxx_db())
    }

    pub fn initialize_usb(&self, req: InitializeUsbRequest) -> ApiResponse<InitializeUsbData> {
        wrap(self.service.initialize_usb(req))
    }
}
