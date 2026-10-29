//! Backend service. This file holds the `BackendService` struct, its
//! constructor and the shared constants; each area of the service lives in
//! its own submodule.

pub(crate) mod analysis;
pub mod anlz;
mod anlz_seek;
pub(crate) mod bpm_key;
#[cfg(test)]
mod bpm_reference_eval;
pub mod cues;
mod diagnostics;
mod engine_export;
mod export;
pub mod export_helpers;
mod export_log;
pub(crate) mod format_compat;
pub(crate) mod key_notation;
mod migrations;
mod mixxx_import;
mod rekordbox_import;
mod repair;
pub mod update_check;
mod usb;
pub mod usb_backups;
pub(crate) mod usb_display_files;
pub(crate) mod usb_helpers;
pub(crate) mod usb_identity;
pub mod usb_staging;
pub(crate) mod usb_utils;
pub mod usb_vendor_compat;
mod wine_paths;

mod library;
mod playback;
mod playlists;
mod rekordbox_resources;
mod settings;
mod source_roots;
#[cfg(test)]
mod tests;
mod track_list;
mod track_matching;
mod track_metadata;

use crate::audio_format::same_format_extension;
pub(crate) use library::*;
#[cfg(test)]
use playback::*;
pub(crate) use playlists::*;
use rekordbox_resources::*;
pub use settings::*;
pub(crate) use source_roots::*;
use track_list::{TrackQuery, TrackSort, duration_totals, list_totals, selection_duration};
pub use track_matching::build_track_match_fingerprint;
use track_matching::{
    library_metadata_candidate_compatible, matching_file_name, matching_file_stem,
    normalize_hash_part,
};

// Re-export functions used by commands.rs via crate::service::*
pub use engine_export::{EngineLibrarySummary, rebuild_engine_library};
pub use usb_utils::{detect_external_rekordbox_db, initialize_usb};
use usb_utils::{
    detect_external_rekordbox_db as detect_external_rekordbox_db_util,
    initialize_usb as initialize_usb_util, load_waveform_preview_from_analysis_path,
    read_pwv4_from_anlz,
};

use chrono::Utc;
use rusqlite::{OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use uuid::Uuid;

use base64::Engine;

use crate::db::Db;
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{
    AddLibrarySelectionToPlaylistRequest, AddLibrarySelectionToPlaylistsRequest, AddTrackCandidate,
    AddTrackCandidateResolution, AddTrackCandidatesToPlaylistData,
    AddTrackCandidatesToPlaylistRequest, AddTrackCandidatesToPlaylistsRequest,
    AddTracksToPlaylistData, AddTracksToPlaylistRequest, AddTracksToPlaylistsData,
    BrowseSourceFilesData, BrowseSourceFilesRequest, CheckSourceRootsData, CheckSourceRootsRequest,
    CreatePlaylistData, CreatePlaylistRequest, DeletePlaylistData, DeletePlaylistRequest,
    DetectExternalRekordboxDbData, ExternalPlaylistKind, ExternalPlaylistSummary,
    GetFrontendSettingsData, GetPlaylistMembershipData, GetPlaylistMembershipRequest,
    GetPlaylistTracksData, GetPlaylistTracksRequest, GetSelectionDurationData,
    GetSelectionDurationRequest, GetSourceRootAnalysisData, GetSourceRootAnalysisRequest,
    GetTracksByIdsData, GetTracksByIdsRequest, ImportedPlaylistRef, InitializeUsbData,
    InitializeUsbRequest, ListMatchingTrackIdsData, ListMatchingTrackIdsRequest, ListPlaylistsData,
    MaterializeSourceTrackData, MaterializeSourceTrackRequest, PlayResolvedTrackData,
    PlayResolvedTrackRequest, PlaybackMetronomeData, PlaybackSource, PlaybackStatusData, Playlist,
    PlaylistMembership, PlaylistMembershipState, RelocateSourceRootData, RelocateSourceRootRequest,
    RemoveTracksBySourceRootsData, RemoveTracksBySourceRootsRequest, RemoveTracksFromPlaylistData,
    RemoveTracksFromPlaylistRequest, RenamePlaylistData, RenamePlaylistRequest,
    ReorderPlaylistTracksData, ReorderPlaylistTracksRequest, ReorderPlaylistsData,
    ReorderPlaylistsRequest, ResolvePlaybackSourceData, ResolvePlaybackSourceRequest,
    ResolveTrackIdentityData, ResolveTrackIdentityRequest, ResolvedBy, ScanLibraryData,
    ScanLibraryRequest, SetFrontendSettingData, SetFrontendSettingRequest,
    SetPlaybackMetronomeRequest, SourceRootAnalysisStatus, SourceRootStatus, StopPlaybackData,
    Track, TrackMatch, WarningEntry,
};
use crate::player::PlaybackController;
use crate::scanner::{ScanCache, scan_audio_files, unique_paths};
use crate::wav_format::WavFormatIssue;

const TRACK_QUERY_LIMIT_MAX: usize = 5000;
const SETTING_EXPORT_OWNED_FILES_PREFIX: &str = "export_owned_files_v1";
pub(crate) const SETTING_EXPORT_MASTER_DB_ID: &str = "export_master_db_id_v1";

pub(crate) const SETTING_UI_THEME: &str = "ui_theme_v1";
pub(crate) const SETTING_UI_ACCENT_HUE: &str = "ui_accent_hue_v1";
pub(crate) const SETTING_UI_SOURCE_ROOTS: &str = "ui_source_roots_v1";
pub(crate) const SETTING_UI_SOURCE_ROOT_ENABLED: &str = "ui_source_root_enabled_v1";
pub(crate) const SETTING_UI_USB_ROOT: &str = "ui_usb_root_v1";
pub(crate) const SETTING_UI_EXPORT_PRUNE_STALE: &str = "ui_export_prune_stale_v1";
pub(crate) const SETTING_UI_EXPORT_BACKUP: &str = "ui_export_backup_v1";
pub(crate) const SETTING_UI_EXPORT_ENGINE_LIBRARY: &str = "ui_export_engine_library_v1";
pub(crate) const SETTING_UI_BACKUP_RETENTION_COUNT: &str = "ui_backup_retention_count_v1";
pub(crate) const DEFAULT_BACKUP_RETENTION_COUNT: u32 = 10;
pub(crate) const SETTING_UI_ANALYSIS_BPM_RANGE: &str = "ui_analysis_bpm_range_v1";
pub(crate) const SETTING_UI_ANALYSIS_ENGINE: &str = "ui_analysis_engine_v1";
pub(crate) const SETTING_UI_SIDEBAR_COLLAPSED: &str = "ui_sidebar_collapsed_v1";
pub(crate) const SETTING_UI_SIDEBAR_WIDTH: &str = "ui_sidebar_width_v1";
pub(crate) const SETTING_UI_HELP_SEEN: &str = "ui_help_seen_v1";
pub(crate) const SETTING_UI_KEY_NOTATION: &str = "ui_key_notation_v1";
pub(crate) const SETTING_UI_REKORDBOX_DB_ENABLED: &str = "ui_rekordbox_db_enabled_v1";
pub(crate) const SETTING_UI_MIXXX_DB_ENABLED: &str = "ui_mixxx_db_enabled_v1";
pub(crate) const SETTING_UI_SOURCES_EVER_CONFIGURED: &str = "ui_sources_ever_configured_v1";
pub(crate) const SETTING_UI_CUE_START_ON_FIRST_BEAT: &str = "ui_cue_start_on_first_beat_v1";
pub(crate) const SETTING_UI_CUE_BEATGRID_LEVEL: &str = "ui_cue_beatgrid_level_v1";
pub(crate) const SETTING_UI_CUE_QUANTIZE: &str = "ui_cue_quantize_v1";
pub(crate) const SETTING_UI_CUE_METRONOME_MIX: &str = "ui_cue_metronome_mix_v1";
pub(crate) const SETTING_UI_CUE_FOLLOW_GRID: &str = "ui_cue_follow_grid_v1";
const WAVEFORM_PREVIEW_BINS: usize = 2400;

const TRACK_CURSOR_VERSION: &str = "track_cursor_v1";

pub(crate) const TRACK_COLS: &str = "id, title, artist, album, track_number, bpm, tonality, file_path, \
    file_size_bytes, format_ext, sample_rate_hz, bit_depth, bitrate_kbps, duration_ms, \
    artwork_path, waveform_peaks_path, bpm_analyzer, created_at, updated_at, \
    COALESCE(rekordbox_db_source, 0) AS rekordbox_db_source, wav_extensible_kind, tonality_source, \
    COALESCE(mixxx_db_source, 0) AS mixxx_db_source, release_year, genre";

type ExistingTrackSnapshot = (
    String,
    Option<i64>, // file_modified_at, Unix seconds
    String,
    String,
    Option<String>,
    Option<u32>,
    Option<String>,
    Option<String>,
    Option<u32>,
    Option<u8>,
    Option<u32>,
    Option<u32>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<u32>,
    Option<String>,
    Option<String>,
    Option<String>, // genre
    Option<String>, // wav_extensible_kind
    bool,           // genre edited in the app (`genre_source = 'user'`)
    Option<i64>,    // file_created_at, Unix seconds
    bool,           // release date edited in the app (`release_date_source = 'user'`)
);
type ExistingTracksByPath = std::collections::HashMap<String, ExistingTrackSnapshot>;

fn trimmed_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Which imported desktop libraries the library view includes alongside its
/// source folders (the rekordbox `master.db` and Mixxx chips).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ExternalLibraries {
    pub rekordbox_db: bool,
    pub mixxx_db: bool,
}

impl ExternalLibraries {
    pub(crate) fn from_flags(rekordbox_db: bool, mixxx_db: bool) -> Self {
        Self {
            rekordbox_db,
            mixxx_db,
        }
    }

    pub(crate) fn any(self) -> bool {
        self.rekordbox_db || self.mixxx_db
    }

    /// True when `track` was imported from one of the enabled libraries.
    pub(crate) fn includes(self, track: &Track) -> bool {
        (self.rekordbox_db && track.rekordbox_db_source) || (self.mixxx_db && track.mixxx_db_source)
    }
}

/// The name of the drive at `usb_root`: the on-drive marker when it's plugged
/// in, else the name cached for it in `usb_devices`. None for a drive whose
/// marker names another drive -- the cached name may well be that drive's.
pub(crate) fn usb_drive_name(conn: &rusqlite::Connection, usb_root: &Path) -> Option<String> {
    if usb_identity::has_name_conflict(usb_root) {
        return None;
    }
    if let Some(name) = usb_identity::read_drive_name(usb_root) {
        return Some(name);
    }
    conn.query_row(
        "SELECT label FROM usb_devices WHERE root_path_key = ?1",
        params![normalize_source_root_for_matching(
            &usb_root.to_string_lossy()
        )],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()
    .ok()
    .flatten()
    .flatten()
    .filter(|name| !name.trim().is_empty())
}

/// Whether any track carries the import flag `column` (`rekordbox_db_source` /
/// `mixxx_db_source`), i.e. that library has been imported.
pub(crate) fn has_tracks_flagged(conn: &rusqlite::Connection, column: &str) -> BackendResult<bool> {
    debug_assert!(matches!(column, "rekordbox_db_source" | "mixxx_db_source"));
    Ok(conn.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM tracks WHERE {column} = 1)"),
        [],
        |row| row.get(0),
    )?)
}

#[derive(Debug, Clone)]
pub struct BackendService {
    pub db: Db,
    pub analysis_paused: Arc<AtomicBool>,
    pub analysis_cancelled: Arc<AtomicBool>,
    /// One-entry cache of the last USB stick's resolved playlists / histories,
    /// keyed by the staged PDB + eDB signatures. Lets the paginated
    /// `fetch_usb_playlist_tracks` / `fetch_usb_history_tracks` skip a full
    /// re-parse on every page. Shared across `.clone()`s of the service.
    pub(crate) usb_playlists_cache: Arc<usb::UsbPlaylistsCache>,
    pub(crate) usb_histories_cache: Arc<usb::UsbHistoriesCache>,
    /// The staged PDB's track files and sizes, for the playlist heading's
    /// export estimate. Shared across `.clone()`s of the service.
    pub(crate) usb_track_sizes_cache: Arc<usb::UsbTrackSizesCache>,
    /// Serializes cue-editor saves to a USB: they run as background jobs, and
    /// each one read-modify-writes the same staged `export.pdb` / eDB.
    /// Shared across `.clone()`s (the Tauri state and the job threads).
    pub(crate) usb_write_lock: Arc<std::sync::Mutex<()>>,
    /// Bumped by every `fetch_usb_playlist_tracks` / `fetch_usb_history_tracks`
    /// call, so a page load can tell a newer one started and stop reading
    /// files for a page the frontend will discard.
    pub(crate) usb_playlist_page_generation: Arc<std::sync::atomic::AtomicU64>,
    pub(crate) usb_history_page_generation: Arc<std::sync::atomic::AtomicU64>,
    /// Set while background data migrations run (see `migrations.rs`), so
    /// commands that write the analysis cache wait for them.
    pub(crate) data_migration_running: Arc<AtomicBool>,
    /// The source folders' file listings and tag reads, reused by
    /// `browse_source_files` (which lists the folders on every page),
    /// select-all and analysis. Shared across `.clone()`s of the service.
    pub(crate) library_scan_cache: Arc<ScanCache>,
}

impl BackendService {
    pub fn new(data_dir: impl AsRef<std::path::Path>) -> BackendResult<Self> {
        let svc = Self {
            db: Db::new(data_dir)?,
            analysis_paused: Arc::new(AtomicBool::new(false)),
            analysis_cancelled: Arc::new(AtomicBool::new(false)),
            usb_playlists_cache: Arc::new(std::sync::Mutex::new(None)),
            usb_histories_cache: Arc::new(std::sync::Mutex::new(None)),
            usb_track_sizes_cache: Arc::new(std::sync::Mutex::new(None)),
            usb_write_lock: Arc::new(std::sync::Mutex::new(())),
            usb_playlist_page_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            usb_history_page_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            data_migration_running: Arc::new(AtomicBool::new(false)),
            library_scan_cache: Arc::new(ScanCache::default()),
        };
        // Deliberately NOT called here (see `usb_staging::init_cache_root`'s
        // doc comment): `BackendService::new`/`BackendCommands::new` are the
        // same constructor `cargo test --lib`'s unit tests, `backend/tests/*`
        // integration tests, AND the real desktop app all call, and `CACHE_ROOT`
        // is a process-wide static. `#[cfg(test)]` can't distinguish these
        // cases -- it's false once `backend` is compiled as a normal library
        // dependency for an external integration-test binary, not just for
        // the real app -- so gating on it here previously left staging
        // silently enabled (and racing across concurrently-constructed
        // instances) in `backend/tests/*.rs`. Only the real desktop app,
        // which is the sole context where exactly one `BackendService`
        // lives for the whole process, calls `usb_staging::init_cache_root`
        // -- from `desktop/src-tauri/src/main.rs`, right after constructing
        // its one `BackendCommands`.
        svc.reset_mounted_usb_devices()?;
        // One-time upgrades of data older versions wrote; see `migrations.rs`.
        svc.run_startup_data_migrations();
        Ok(svc)
    }

    /// Mount state from a previous OS/app session can't be trusted (crash,
    /// drive physically removed while the app was closed) -- reset it
    /// unconditionally on every launch, before anything else touches
    /// `usb_devices`.
    fn reset_mounted_usb_devices(&self) -> BackendResult<()> {
        let conn = self.db.connect()?;
        conn.execute(
            "UPDATE usb_devices SET mounted = 0, updated_at = ?1 WHERE mounted != 0",
            params![now()],
        )?;
        Ok(())
    }

    /// One-time migration (`migrations.rs`): seed `usb_devices` from the legacy
    /// `ui_usb_root_v1`/`ui_usb_recent_roots_v1` app_settings entries so
    /// pre-existing databases get retroactive USB-vs-local protection with
    /// no per-track migration needed. `ui_usb_recent_roots_v1` is deleted
    /// afterward since its data now lives in `usb_devices` -- on the next
    /// launch this becomes an immediate no-op. `ui_usb_root_v1` is left in
    /// place; it serves an unrelated purpose (pre-filling the USB picker
    /// with the last-chosen root), not device-list bookkeeping.
    fn backfill_usb_devices_from_legacy_settings(&self) -> BackendResult<()> {
        let conn = self.db.connect()?;
        let now_ts = now();

        let mut seen = std::collections::HashSet::<String>::new();
        let mut roots = Vec::<String>::new();
        let push_root =
            |raw: &str, roots: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    return;
                }
                let key = normalize_source_root_for_matching(trimmed);
                if seen.insert(key) {
                    roots.push(trimmed.to_string());
                }
            };

        if let Some(root_value) = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                params![SETTING_UI_USB_ROOT],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            push_root(&root_value, &mut roots, &mut seen);
        }

        // "ui_usb_recent_roots_v1": the legacy recent-roots list. Read here by
        // its literal key (not a shared constant) because this one-time
        // migration is now the only remaining reader anywhere in the
        // codebase -- the setting is no longer part of
        // frontend_ui_setting_keys(), so nothing else needs its name.
        if let Some(recent_value) = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                params!["ui_usb_recent_roots_v1"],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            // Malformed JSON is tolerated -- skip this contribution, don't fail startup.
            if let Ok(entries) = serde_json::from_str::<Vec<String>>(&recent_value) {
                for entry in entries {
                    push_root(&entry, &mut roots, &mut seen);
                }
            }

            conn.execute(
                "DELETE FROM app_settings WHERE key = ?1",
                params!["ui_usb_recent_roots_v1"],
            )?;
        }

        for root in roots {
            usb_utils::upsert_usb_device(&conn, Path::new(&root), false, &now_ts)?;
        }

        Ok(())
    }

    /// One-line CPU/RAM summary for the startup Event Log, so support
    /// requests about crashes/throttling on a given machine don't require
    /// asking the user for their specs separately.
    pub fn system_resource_summary() -> String {
        let cpu_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .max(1);
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        let to_gib = |bytes: u64| bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        format!(
            "system resources: {cpu_threads} CPU thread(s), {:.1} GiB total RAM, {:.1} GiB available RAM",
            to_gib(sys.total_memory()),
            to_gib(sys.available_memory())
        )
    }

    pub fn detect_external_rekordbox_db(&self) -> BackendResult<DetectExternalRekordboxDbData> {
        let conn = self.db.connect()?;
        Ok(DetectExternalRekordboxDbData {
            imported: has_tracks_flagged(&conn, "rekordbox_db_source")?,
            ..detect_external_rekordbox_db_util()
        })
    }

    pub fn initialize_usb(&self, req: InitializeUsbRequest) -> BackendResult<InitializeUsbData> {
        let data = initialize_usb_util(Path::new(&req.usb_root))?;
        self.invalidate_usb_parse_cache();
        Ok(data)
    }
}

pub(crate) fn now() -> String {
    Utc::now().to_rfc3339()
}
