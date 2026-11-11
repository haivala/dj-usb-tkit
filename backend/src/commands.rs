use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

use crate::error::BackendResult;
use crate::error::ErrorPayload;
use crate::i18n::{Arg, Text, tr};
use crate::models::{
    AddLibrarySelectionToPlaylistRequest, AddLibrarySelectionToPlaylistsRequest,
    AddSourceFoldersData, AddSourceFoldersRequest, AddTrackCandidatesToPlaylistData,
    AddTrackCandidatesToPlaylistRequest, AddTrackCandidatesToPlaylistsRequest,
    AddTracksToPlaylistData, AddTracksToPlaylistsData, AnalyzeNewTracksData,
    AnalyzeNewTracksRequest, ApiResponse, BrowseSourceFilesData, BrowseSourceFilesRequest,
    BuildUsbHistoryTracklistData, BuildUsbHistoryTracklistRequest, CreatePlaylistData,
    CreatePlaylistRequest, DeletePlaylistData, DeletePlaylistRequest, DeleteUsbBackupData,
    DeleteUsbBackupRequest, DetectExternalMixxxDbData, DetectExternalRekordboxDbData,
    ExportToUsbData, ExportToUsbRequest, FetchUsbHistoriesData, FetchUsbHistoriesRequest,
    FetchUsbPlaylistsData, FetchUsbPlaylistsRequest, FetchUsbTracksData, FetchUsbTracksRequest,
    GetFrontendSettingsData, GetPlaylistExportSpaceData, GetPlaylistExportSpaceRequest,
    GetPlaylistMembershipData, GetPlaylistMembershipRequest, GetPlaylistTracksData,
    GetPlaylistTracksRequest, GetSelectionDurationData, GetSelectionDurationRequest,
    GetTrackDetailRequest, GetTracksMetadataSummaryRequest, GetUsbBackgroundColorRequest,
    GetUsbHistoryTracklistOptionsData, GetUsbHistoryTracklistOptionsRequest,
    GetUsbPlayerMenuConfigData, GetUsbPlayerMenuConfigRequest, GetUsbTrackDetailRequest,
    GetUsbTracksMetadataSummaryRequest, ImportExternalPlaylistsData,
    ImportExternalPlaylistsRequest, InitializeUsbData, InitializeUsbRequest, InspectUsbTrackData,
    InspectUsbTrackRequest, LibrarySourcesData, ListExternalPlaylistsData,
    ListExternalPlaylistsRequest, ListMatchingTrackIdsData, ListMatchingTrackIdsRequest,
    ListUsbBackupsData, ListUsbBackupsRequest, ListUsbDevicesData, MaterializeSourceTrackData,
    MaterializeSourceTrackRequest, PlayResolvedTrackData, PlayResolvedTrackRequest,
    PlaybackMetronomeData, PlaybackStatusData, PruneUsbDeviceData, PruneUsbDeviceRequest,
    RelocateSourceRootData, RelocateSourceRootRequest, RemoveSourceFolderData,
    RemoveSourceFolderRequest, RemoveTracksFromPlaylistData, RemoveTracksFromPlaylistRequest,
    RemoveUsbPlaylistData, RemoveUsbPlaylistRequest, RenamePlaylistData, RenamePlaylistRequest,
    ReorderPlaylistTracksData, ReorderPlaylistTracksRequest, ReorderPlaylistsData,
    ReorderPlaylistsRequest, RepairUsbDiagnosticsData, RepairUsbDiagnosticsRequest,
    ResolvePlaybackSourceData, ResolvePlaybackSourceRequest, ResolveTrackIdentityData,
    ResolveTrackIdentityRequest, RestoreUsbBackupData, RestoreUsbBackupRequest,
    RunDataMigrationsData, RunUsbDiagnosticsData, RunUsbDiagnosticsRequest, RunUsbParityReportData,
    RunUsbParityReportRequest, SaveTrackAnalysisEditsData, SaveTrackAnalysisEditsRequest,
    SaveTrackMetadataEditsData, SaveTrackMetadataEditsRequest, SaveUsbTrackAnalysisEditsData,
    SaveUsbTrackAnalysisEditsRequest, SaveUsbTrackMetadataEditsData,
    SaveUsbTrackMetadataEditsRequest, ScanLibraryData, ScanLibraryRequest, ScanMixxxDbRequest,
    ScanRekordboxDbRequest, SetAnalysisPausedData, SetExternalLibraryEnabledRequest,
    SetFrontendSettingData, SetFrontendSettingRequest, SetPlaybackMetronomeRequest,
    SetSourceFolderEnabledRequest, SetUsbBackgroundColorRequest, SetUsbDeviceNameData,
    SetUsbDeviceNameRequest, StopPlaybackData, TrackDetail, TracksMetadataSummary,
    UpdateUsbPlayerMenuConfigData, UpdateUsbPlayerMenuConfigRequest, UsbBackgroundColorData,
    UsbTrackAnalysisDetail, WithPlaylists,
};
use crate::player::{PlaybackController, PlaybackTransition};
use crate::service::BackendService;

fn wrap<T: serde::Serialize>(result: BackendResult<T>) -> ApiResponse<T> {
    result
        .map(ApiResponse::success)
        .unwrap_or_else(|err| ApiResponse::failure(err.into()))
}

/// [`wrap`], with a failure's message inside `context`, a text saying what failed
/// ("Rename failed: …"): the status line shows the message as it is.
fn wrap_failed<T: serde::Serialize>(
    result: BackendResult<T>,
    context: impl FnOnce(Arg) -> Text,
) -> ApiResponse<T> {
    result
        .map(ApiResponse::success)
        .unwrap_or_else(|err| ApiResponse::failure(ErrorPayload::within(err, context)))
}

#[derive(Debug, Clone)]
pub struct BackendCommands {
    service: BackendService,
    playback: PlaybackController,
    playback_transitions: Arc<Mutex<Option<mpsc::Receiver<PlaybackTransition>>>>,
}

impl BackendCommands {
    /// A playlist change's result with the playlists as they now are.
    fn with_playlists<T>(&self, result: BackendResult<T>) -> BackendResult<WithPlaylists<T>> {
        let data = result?;
        Ok(WithPlaylists {
            data,
            state: self.service.playlists_state()?,
        })
    }

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
        wrap_failed(self.service.list_rekordbox_playlists(req), |error| {
            tr::playlist_import_list_failed("rekordbox", error)
        })
    }

    pub fn scan_mixxx_db(&self, req: ScanMixxxDbRequest) -> ApiResponse<ScanLibraryData> {
        wrap(self.service.scan_mixxx_db(req))
    }

    pub fn list_mixxx_playlists(
        &self,
        req: ListExternalPlaylistsRequest,
    ) -> ApiResponse<ListExternalPlaylistsData> {
        wrap_failed(self.service.list_mixxx_playlists(req), |error| {
            tr::playlist_import_list_failed("Mixxx", error)
        })
    }

    pub fn import_external_playlists(
        &self,
        req: ImportExternalPlaylistsRequest,
    ) -> ApiResponse<WithPlaylists<ImportExternalPlaylistsData>> {
        wrap_failed(
            self.with_playlists(self.service.import_external_playlists(req)),
            tr::playlist_import_many_failed,
        )
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

    pub fn add_source_folders_with_progress(
        &self,
        req: AddSourceFoldersRequest,
        on_indexing: impl FnOnce(),
    ) -> ApiResponse<AddSourceFoldersData> {
        wrap(
            self.service
                .add_source_folders_with_progress(req, on_indexing),
        )
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
        wrap_failed(
            self.service.save_track_analysis_edits(req),
            tr::track_detail_save_cues_failed,
        )
    }

    pub fn save_track_metadata_edits(
        &self,
        req: SaveTrackMetadataEditsRequest,
    ) -> ApiResponse<SaveTrackMetadataEditsData> {
        wrap_failed(
            self.service.save_track_metadata_edits(req),
            tr::group_edit_save_failed,
        )
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
        wrap_failed(
            self.service
                .save_usb_track_metadata_edits_with_progress(req, on_progress),
            tr::group_edit_save_failed,
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
        wrap_failed(
            self.service.save_usb_track_analysis_edits(req),
            tr::track_detail_save_cues_failed,
        )
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

    pub fn relocate_source_root_with_progress(
        &self,
        req: RelocateSourceRootRequest,
        on_indexing: impl FnOnce(),
    ) -> ApiResponse<RelocateSourceRootData> {
        wrap(
            self.service
                .relocate_source_root_with_progress(req, on_indexing),
        )
    }

    pub fn resolve_playback_source(
        &self,
        req: ResolvePlaybackSourceRequest,
    ) -> ApiResponse<ResolvePlaybackSourceData> {
        wrap(self.service.resolve_playback_source(req))
    }

    pub fn create_playlist(
        &self,
        req: CreatePlaylistRequest,
    ) -> ApiResponse<WithPlaylists<CreatePlaylistData>> {
        wrap_failed(
            self.with_playlists(self.service.create_playlist(req)),
            tr::playlist_create_failed,
        )
    }

    pub fn rename_playlist(
        &self,
        req: RenamePlaylistRequest,
    ) -> ApiResponse<WithPlaylists<RenamePlaylistData>> {
        wrap_failed(
            self.with_playlists(self.service.rename_playlist(req)),
            tr::playlist_rename_failed,
        )
    }

    pub fn reorder_playlists(
        &self,
        req: ReorderPlaylistsRequest,
    ) -> ApiResponse<ReorderPlaylistsData> {
        wrap_failed(
            self.service.reorder_playlists(req),
            tr::playlist_order_failed,
        )
    }

    pub fn delete_playlist(
        &self,
        req: DeletePlaylistRequest,
    ) -> ApiResponse<WithPlaylists<DeletePlaylistData>> {
        wrap(self.with_playlists(self.service.delete_playlist(req)))
    }

    /// The playlists, with their export status against the selected USB.
    pub fn list_playlists(&self) -> ApiResponse<crate::models::PlaylistsState> {
        wrap(self.service.playlists_state())
    }

    /// No USB selected any more; answers with the playlists' export status without one.
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
    ) -> ApiResponse<WithPlaylists<AddTrackCandidatesToPlaylistData>> {
        wrap(self.with_playlists(self.service.add_track_candidates_to_playlist(req)))
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
    ) -> ApiResponse<WithPlaylists<AddTracksToPlaylistData>> {
        wrap(self.with_playlists(self.service.add_library_selection_to_playlist(req)))
    }

    pub fn add_library_selection_to_playlists(
        &self,
        req: AddLibrarySelectionToPlaylistsRequest,
    ) -> ApiResponse<WithPlaylists<AddTracksToPlaylistsData>> {
        wrap(self.with_playlists(self.service.add_library_selection_to_playlists(req)))
    }

    pub fn add_track_candidates_to_playlists(
        &self,
        req: AddTrackCandidatesToPlaylistsRequest,
    ) -> ApiResponse<WithPlaylists<AddTracksToPlaylistsData>> {
        wrap(self.with_playlists(self.service.add_track_candidates_to_playlists(req)))
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
    ) -> ApiResponse<WithPlaylists<RemoveTracksFromPlaylistData>> {
        wrap_failed(
            self.with_playlists(self.service.remove_tracks_from_playlist(req)),
            tr::playlist_remove_failed,
        )
    }

    pub fn reorder_playlist_tracks(
        &self,
        req: ReorderPlaylistTracksRequest,
    ) -> ApiResponse<WithPlaylists<ReorderPlaylistTracksData>> {
        wrap_failed(
            self.with_playlists(self.service.reorder_playlist_tracks(req)),
            tr::playlist_track_order_failed,
        )
    }

    /// The `ui-` texts in the current language (`i18n::ui_texts`).
    pub fn get_ui_texts(&self) -> ApiResponse<crate::models::GetUiTextsData> {
        ApiResponse::success(crate::i18n::ui_texts())
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

    /// Changes the language, refused while `job_running`: the change reloads the window,
    /// which loses a running job's progress bar, pause and cancel (no command reconnects to
    /// one). The desktop app passes whether its job registry is empty, and holds the registry
    /// meanwhile so no job starts mid-change.
    pub fn set_language(
        &self,
        req: crate::models::SetLanguageRequest,
        job_running: bool,
    ) -> ApiResponse<crate::models::SetLanguageData> {
        let result = if job_running {
            Err(crate::error::BackendError::Validation(
                tr::settings_language_job_running(),
            ))
        } else {
            self.service.set_language(req)
        };
        wrap_failed(result, tr::settings_language_failed)
    }

    /// See `BackendService::apply_saved_language`: the desktop app calls it once at startup.
    pub fn apply_saved_language(&self) -> ApiResponse<()> {
        wrap(self.service.apply_saved_language())
    }

    pub fn remove_essentia(&self) -> ApiResponse<()> {
        wrap_failed(
            self.service.remove_essentia(),
            tr::settings_essentia_remove_failed,
        )
    }

    pub fn data_dir(&self) -> std::path::PathBuf {
        self.service.db.data_dir()
    }

    pub fn select_usb(
        &self,
        req: crate::models::SelectUsbRequest,
    ) -> ApiResponse<WithPlaylists<crate::models::SelectUsbData>> {
        wrap(self.with_playlists(self.service.select_usb(req)))
    }

    pub fn list_usb_devices(&self) -> ApiResponse<ListUsbDevicesData> {
        wrap(self.service.list_usb_devices())
    }

    pub fn prune_usb_device(&self, req: PruneUsbDeviceRequest) -> ApiResponse<PruneUsbDeviceData> {
        wrap(self.service.prune_usb_device(req))
    }

    pub fn set_export_sync_mode(
        &self,
        req: crate::models::SetExportSyncModeRequest,
    ) -> ApiResponse<WithPlaylists<crate::models::SetExportSyncModeData>> {
        // The mode changes which playlists' order is locked.
        wrap(self.with_playlists(self.service.set_export_sync_mode(req)))
    }

    pub fn set_usb_device_name(
        &self,
        req: SetUsbDeviceNameRequest,
    ) -> ApiResponse<WithPlaylists<SetUsbDeviceNameData>> {
        wrap(self.with_playlists(self.service.set_usb_device_name(req)))
    }

    pub fn set_usb_audio_formats(
        &self,
        req: crate::models::SetUsbAudioFormatsRequest,
    ) -> ApiResponse<crate::models::UsbAudioFormatsData> {
        wrap_failed(
            self.service.set_usb_audio_formats(req),
            tr::usb_audio_formats_save_failed,
        )
    }

    pub fn list_usb_backups(&self, req: ListUsbBackupsRequest) -> ApiResponse<ListUsbBackupsData> {
        wrap(self.service.list_usb_backups(req))
    }

    pub fn restore_usb_backup(
        &self,
        req: RestoreUsbBackupRequest,
    ) -> ApiResponse<WithPlaylists<RestoreUsbBackupData>> {
        // The restored databases change what the playlists' USB marks show.
        wrap_failed(
            self.with_playlists(self.service.restore_usb_backup(req)),
            tr::backups_restore_failed,
        )
    }

    pub fn delete_usb_backup(
        &self,
        req: DeleteUsbBackupRequest,
    ) -> ApiResponse<DeleteUsbBackupData> {
        wrap_failed(
            self.service.delete_usb_backup(req),
            tr::backups_delete_failed,
        )
    }

    /// Ids of the background data migrations still to run.
    pub fn pending_data_migrations(&self) -> ApiResponse<Vec<String>> {
        wrap_failed(self.service.pending_data_migrations(), tr::migration_failed)
    }

    pub fn run_data_migrations_with_progress<F>(
        &self,
        on_progress: F,
    ) -> ApiResponse<RunDataMigrationsData>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap_failed(
            self.service.run_data_migrations(on_progress),
            tr::migration_failed,
        )
    }

    pub fn fetch_usb_playlists(
        &self,
        req: FetchUsbPlaylistsRequest,
    ) -> ApiResponse<WithPlaylists<FetchUsbPlaylistsData>> {
        wrap(self.with_playlists(self.service.fetch_usb_playlists(req)))
    }

    pub fn fetch_usb_playlists_with_progress<F>(
        &self,
        req: FetchUsbPlaylistsRequest,
        on_progress: F,
    ) -> ApiResponse<WithPlaylists<FetchUsbPlaylistsData>>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(
            self.with_playlists(
                self.service
                    .fetch_usb_playlists_with_progress(req, on_progress),
            ),
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

    pub fn get_usb_background_color(
        &self,
        req: GetUsbBackgroundColorRequest,
    ) -> ApiResponse<UsbBackgroundColorData> {
        wrap_failed(
            self.service.get_usb_background_color(req),
            tr::usb_background_color_read_failed,
        )
    }

    pub fn set_usb_background_color(
        &self,
        req: SetUsbBackgroundColorRequest,
    ) -> ApiResponse<UsbBackgroundColorData> {
        wrap(self.service.set_usb_background_color(req))
    }

    pub fn remove_usb_playlist(
        &self,
        req: RemoveUsbPlaylistRequest,
    ) -> ApiResponse<RemoveUsbPlaylistData> {
        wrap_failed(
            self.service.remove_usb_playlist(req),
            tr::usb_playlists_remove_failed,
        )
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

    pub fn begin_usb_order_edit(
        &self,
        req: crate::models::UsbOrderEditRequest,
    ) -> ApiResponse<crate::models::UsbOrderEditData> {
        wrap(self.service.begin_usb_order_edit(req))
    }

    pub fn move_usb_playlist(
        &self,
        req: crate::models::MoveUsbPlaylistRequest,
    ) -> ApiResponse<crate::models::UsbOrderEditData> {
        wrap(self.service.move_usb_playlist(req))
    }

    pub fn move_usb_playlist_track(
        &self,
        req: crate::models::MoveUsbPlaylistTrackRequest,
    ) -> ApiResponse<crate::models::UsbOrderEditData> {
        wrap(self.service.move_usb_playlist_track(req))
    }

    pub fn sort_usb_playlist_tracks(
        &self,
        req: crate::models::SortUsbPlaylistTracksRequest,
    ) -> ApiResponse<crate::models::UsbOrderEditData> {
        wrap(self.service.sort_usb_playlist_tracks(req))
    }

    pub fn cancel_usb_order_edit(
        &self,
        req: crate::models::UsbOrderEditRequest,
    ) -> ApiResponse<crate::models::UsbOrderEditData> {
        wrap(self.service.cancel_usb_order_edit(req))
    }

    pub fn commit_usb_order_edit(
        &self,
        req: crate::models::UsbOrderEditRequest,
    ) -> ApiResponse<crate::models::UsbOrderEditData> {
        self.commit_usb_order_edit_with_progress(req, |_, _, _| {})
    }

    pub fn commit_usb_order_edit_with_progress<F>(
        &self,
        req: crate::models::UsbOrderEditRequest,
        on_progress: F,
    ) -> ApiResponse<crate::models::UsbOrderEditData>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap_failed(
            self.service
                .commit_usb_order_edit_with_progress(req, on_progress),
            tr::usb_order_edit_save_failed,
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

    pub fn get_usb_history_tracklist_options(
        &self,
        req: GetUsbHistoryTracklistOptionsRequest,
    ) -> ApiResponse<GetUsbHistoryTracklistOptionsData> {
        wrap(self.service.get_usb_history_tracklist_options(req))
    }

    pub fn build_usb_history_tracklist(
        &self,
        req: BuildUsbHistoryTracklistRequest,
    ) -> ApiResponse<BuildUsbHistoryTracklistData> {
        wrap(self.service.build_usb_history_tracklist(req))
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

    pub fn submit_analysis(
        &self,
        req: AnalyzeNewTracksRequest,
    ) -> ApiResponse<crate::models::AnalysisSubmittedData> {
        wrap(self.service.submit_analysis(req))
    }

    pub fn set_analysis_observer(&self, observer: crate::service::AnalysisObserver) {
        self.service.set_analysis_observer(observer);
    }

    pub fn set_analysis_paused(&self, paused: bool) -> ApiResponse<SetAnalysisPausedData> {
        wrap(self.service.set_analysis_paused(paused))
    }

    pub fn cancel_analysis(&self) -> ApiResponse<()> {
        wrap(self.service.cancel_analysis())
    }

    pub fn transcode_tracks_with_progress<F>(
        &self,
        req: crate::models::TranscodeTracksRequest,
        on_progress: F,
    ) -> ApiResponse<crate::models::TranscodeTracksData>
    where
        F: FnMut(&crate::service::transcode::TranscodeProgress),
    {
        wrap(
            self.service
                .transcode_tracks_with_progress(req, on_progress),
        )
    }

    pub fn cancel_transcode(&self) -> ApiResponse<()> {
        wrap(self.service.cancel_transcode())
    }

    pub fn remove_track_transcodes(
        &self,
        req: crate::models::RemoveTrackTranscodesRequest,
    ) -> ApiResponse<crate::models::RemoveTrackTranscodesData> {
        wrap(self.service.remove_track_transcodes(req))
    }

    pub fn get_transcode_formats(&self) -> ApiResponse<crate::models::TranscodeFormatsData> {
        wrap(Ok(self.service.transcode_formats()))
    }

    pub fn get_transcode_folder(&self) -> ApiResponse<crate::models::TranscodeFolderData> {
        wrap_failed(
            self.service.transcode_folder(),
            tr::settings_transcode_folder_failed,
        )
    }

    pub fn set_transcode_folder(
        &self,
        req: crate::models::SetTranscodeFolderRequest,
    ) -> ApiResponse<crate::models::TranscodeFolderData> {
        wrap_failed(
            self.service.set_transcode_folder(req),
            tr::settings_transcode_folder_failed,
        )
    }

    pub fn export_to_usb(
        &self,
        req: ExportToUsbRequest,
    ) -> ApiResponse<WithPlaylists<ExportToUsbData>> {
        wrap(self.with_playlists(self.service.export_to_usb(req)))
    }

    pub fn export_to_usb_with_progress<F>(
        &self,
        req: ExportToUsbRequest,
        on_progress: F,
    ) -> ApiResponse<WithPlaylists<ExportToUsbData>>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(self.with_playlists(self.service.export_to_usb_with_progress(req, on_progress)))
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
    ) -> ApiResponse<WithPlaylists<RunUsbDiagnosticsData>> {
        wrap(self.with_playlists(self.service.run_usb_diagnostics(req)))
    }

    pub fn run_usb_diagnostics_with_progress<F>(
        &self,
        req: RunUsbDiagnosticsRequest,
        on_progress: F,
    ) -> ApiResponse<WithPlaylists<RunUsbDiagnosticsData>>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(
            self.with_playlists(
                self.service
                    .run_usb_diagnostics_with_progress(req, on_progress),
            ),
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
    ) -> ApiResponse<WithPlaylists<RepairUsbDiagnosticsData>> {
        wrap(self.with_playlists(self.service.repair_usb_diagnostics(req)))
    }

    pub fn repair_usb_diagnostics_with_progress<F>(
        &self,
        req: RepairUsbDiagnosticsRequest,
        on_progress: F,
    ) -> ApiResponse<WithPlaylists<RepairUsbDiagnosticsData>>
    where
        F: FnMut(usize, usize, &str),
    {
        wrap(
            self.with_playlists(
                self.service
                    .repair_usb_diagnostics_with_progress(req, on_progress),
            ),
        )
    }

    pub fn detect_external_rekordbox_db(&self) -> ApiResponse<DetectExternalRekordboxDbData> {
        wrap(self.service.detect_external_rekordbox_db())
    }

    pub fn detect_external_mixxx_db(&self) -> ApiResponse<DetectExternalMixxxDbData> {
        wrap(self.service.detect_external_mixxx_db())
    }

    pub fn initialize_usb(&self, req: InitializeUsbRequest) -> ApiResponse<InitializeUsbData> {
        wrap_failed(self.service.initialize_usb(req), tr::usb_initialize_failed)
    }
}
