use std::path::Path;

use crate::error::BackendResult;
use crate::error::ErrorPayload;
use crate::models::{
    AddTracksToPlaylistData, AddTracksToPlaylistRequest, AnalyzeNewTracksData,
    AnalyzeNewTracksRequest, AnalyzeTrackPieceData, AnalyzeTrackPieceRequest, ApiResponse,
    BrowseSourceFilesData, BrowseSourceFilesRequest, CreatePlaylistData, CreatePlaylistRequest,
    DeletePlaylistData, DeletePlaylistRequest, DetectExternalMasterDbData, ExportToUsbData,
    ExportToUsbRequest, FetchUsbHistoriesData, FetchUsbHistoriesRequest, FetchUsbPlaylistsData,
    FetchUsbPlaylistsRequest, GetFrontendSettingsData, GetPlaylistTracksData,
    GetPlaylistTracksRequest, GetSystemParallelismData, GetTracksByIdsData, GetTracksByIdsRequest,
    GetUsbPlayerMenuConfigData, GetUsbPlayerMenuConfigRequest, InitializeUsbData,
    InitializeUsbRequest, InspectUsbTrackData, InspectUsbTrackRequest, ListPlaylistsData,
    ListTracksData, ListTracksRequest, MaterializeSourceTrackData, MaterializeSourceTrackRequest,
    PlayTrackData, PlayTrackRequest, PlaybackPreflightData, PlaybackPreflightRequest,
    PlaybackStatusData, RemoveTracksBySourceRootsData, RemoveTracksBySourceRootsRequest,
    RemoveTracksFromPlaylistData, RemoveTracksFromPlaylistRequest, RemoveUsbPlaylistData,
    RemoveUsbPlaylistRequest, RenamePlaylistData, RenamePlaylistRequest, RepairUsbDiagnosticsData,
    RepairUsbDiagnosticsRequest, ResolvePlaybackSourceData, ResolvePlaybackSourceRequest,
    RunUsbDiagnosticsData, RunUsbDiagnosticsRequest, RunUsbParityReportData,
    RunUsbParityReportRequest, ScanLibraryData, ScanLibraryRequest, ScanMasterDbRequest,
    SearchTracksData, SearchTracksRequest, SetFrontendSettingData, SetFrontendSettingRequest,
    StopPlaybackData, UpdateUsbPlayerMenuConfigData, UpdateUsbPlayerMenuConfigRequest,
    ValidateUsbRootData, ValidateUsbRootRequest,
};
use crate::player::PlaybackController;
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
}

impl BackendCommands {
    pub fn new(data_dir: impl AsRef<Path>) -> Result<Self, ErrorPayload> {
        let service = BackendService::new(data_dir).map_err(ErrorPayload::from)?;
        Ok(Self {
            service,
            playback: PlaybackController::new(),
        })
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

    pub fn materialize_source_track(
        &self,
        req: MaterializeSourceTrackRequest,
    ) -> ApiResponse<MaterializeSourceTrackData> {
        wrap(self.service.materialize_source_track(req))
    }

    pub fn remove_tracks_by_source_roots(
        &self,
        req: RemoveTracksBySourceRootsRequest,
    ) -> ApiResponse<RemoveTracksBySourceRootsData> {
        wrap(self.service.remove_tracks_by_source_roots(req))
    }

    pub fn get_tracks_by_ids_with_previews(
        &self,
        req: GetTracksByIdsRequest,
    ) -> ApiResponse<GetTracksByIdsData> {
        wrap(self.service.get_tracks_by_ids_with_previews(req))
    }

    pub fn get_system_parallelism(&self) -> ApiResponse<GetSystemParallelismData> {
        wrap(self.service.get_system_parallelism())
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

    pub fn remove_tracks_from_playlist(
        &self,
        req: RemoveTracksFromPlaylistRequest,
    ) -> ApiResponse<RemoveTracksFromPlaylistData> {
        wrap(self.service.remove_tracks_from_playlist(req))
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

    pub fn inspect_usb_track(
        &self,
        req: InspectUsbTrackRequest,
    ) -> ApiResponse<InspectUsbTrackData> {
        wrap(self.service.inspect_usb_track(req))
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

    pub fn analyze_track_piece(
        &self,
        req: AnalyzeTrackPieceRequest,
    ) -> ApiResponse<AnalyzeTrackPieceData> {
        wrap(self.service.analyze_track_piece(req))
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
