//! Backend service: core library and playlist management.

pub(crate) mod analysis;
pub mod anlz;
pub(crate) mod bpm_key;
mod diagnostics;
mod export;
pub mod export_helpers;
mod export_log;
mod repair;
mod usb;
pub mod usb_backups;
pub(crate) mod usb_helpers;
pub(crate) mod usb_identity;
pub mod usb_staging;
pub(crate) mod usb_utils;
pub mod usb_vendor_compat;

// Re-export functions used by commands.rs via crate::service::*
pub use usb_utils::{detect_external_master_db, initialize_usb};
use usb_utils::{
    detect_external_master_db as detect_external_master_db_util,
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
    AddTrackCandidate, AddTrackCandidateResolution, AddTrackCandidatesToPlaylistData,
    AddTrackCandidatesToPlaylistRequest, AddTracksToPlaylistData, AddTracksToPlaylistRequest,
    BrowseSourceFilesData, BrowseSourceFilesRequest, CheckSourceRootsData, CheckSourceRootsRequest,
    CreatePlaylistData, CreatePlaylistRequest, DedupeMode, DeletePlaylistData,
    DeletePlaylistRequest, DetectExternalMasterDbData, GetFrontendSettingsData,
    GetPlaylistTracksData, GetPlaylistTracksRequest, GetTracksByIdsData, GetTracksByIdsRequest,
    InitializeUsbData, InitializeUsbRequest, ListPlaylistsData, ListTracksData, ListTracksRequest,
    MaterializeSourceTrackData, MaterializeSourceTrackRequest, PlayResolvedTrackData,
    PlayResolvedTrackRequest, PlayTrackData, PlayTrackRequest, PlaybackPreflightData,
    PlaybackPreflightRequest, PlaybackStatusData, Playlist, RelocateSourceRootData,
    RelocateSourceRootRequest, RemoveTracksBySourceRootsData, RemoveTracksBySourceRootsRequest,
    RemoveTracksFromPlaylistData, RemoveTracksFromPlaylistRequest, RenamePlaylistData,
    RenamePlaylistRequest, ReorderPlaylistTracksData, ReorderPlaylistTracksRequest,
    ResolvePlaybackSourceData, ResolvePlaybackSourceRequest, ResolveTrackIdentityData,
    ResolveTrackIdentityRequest, ScanLibraryData, ScanLibraryRequest, ScanMasterDbRequest,
    SearchTracksData, SearchTracksRequest, SetFrontendSettingData, SetFrontendSettingRequest,
    SourceRootAnalysisStatus, SourceRootStatus, StopPlaybackData, Track, WarningEntry,
};
use crate::player::{PlaybackController, run_playback_preflight};
use crate::scanner::{scan_audio_files, unique_paths};
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
pub(crate) const SETTING_UI_BACKUP_RETENTION_COUNT: &str = "ui_backup_retention_count_v1";
pub(crate) const DEFAULT_BACKUP_RETENTION_COUNT: u32 = 10;
pub(crate) const SETTING_UI_ANALYSIS_BPM_RANGE: &str = "ui_analysis_bpm_range_v1";
pub(crate) const SETTING_UI_ANALYSIS_ENGINE: &str = "ui_analysis_engine_v1";
pub(crate) const SETTING_UI_SIDEBAR_COLLAPSED: &str = "ui_sidebar_collapsed_v1";
pub(crate) const SETTING_UI_HELP_SEEN: &str = "ui_help_seen_v1";
const WAVEFORM_PREVIEW_BINS: usize = 2400;

const TRACK_CURSOR_VERSION: &str = "track_cursor_v1";

const TRACK_COLS: &str = "id, title, artist, album, track_number, bpm, tonality, file_path, \
    file_size_bytes, format_ext, sample_rate_hz, bit_depth, bitrate_kbps, duration_ms, \
    artwork_path, waveform_peaks_path, bpm_analyzer, created_at, updated_at, \
    COALESCE(master_db_source, 0) AS master_db_source, wav_extensible_kind";

type ExistingTrackSnapshot = (
    String,
    Option<String>,
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
);
type ExistingTracksByPath = std::collections::HashMap<String, ExistingTrackSnapshot>;

fn browse_path_key(path: &str) -> String {
    path.trim().replace('\\', "/").to_ascii_lowercase()
}

pub(crate) fn browse_path_matches_root(file_path: &str, root: &str) -> bool {
    let file_key = browse_path_key(file_path);
    let root_key = browse_path_key(root).trim_end_matches('/').to_string();
    !root_key.is_empty() && (file_key == root_key || file_key.starts_with(&format!("{root_key}/")))
}

fn playback_source_label(
    origin: Option<&str>,
    library_resolved: bool,
    has_usb_context: bool,
) -> String {
    let origin = origin.unwrap_or("").trim().to_ascii_lowercase();
    let external_origin = origin == "usb" || origin == "history";
    if library_resolved {
        if external_origin {
            "Library (matched)"
        } else {
            "Library"
        }
    } else if external_origin && has_usb_context {
        "USB"
    } else {
        "Local file"
    }
    .to_string()
}

fn is_recoverable_playback_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "busy", "already", "in use", "device", "stream", "sink", "playing",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn play_native_with_recovery(
    playback: &PlaybackController,
    path: &str,
    start_offset_ms: Option<u64>,
    start_ratio: Option<f64>,
) -> BackendResult<PlaybackStatusData> {
    match playback.play_path(path, start_offset_ms, start_ratio) {
        Ok(status) => Ok(status),
        Err(err) => {
            if !is_recoverable_playback_error(&err.to_string()) {
                return Err(err);
            }
            let _ = playback.stop();
            playback.play_path(path, start_offset_ms, start_ratio)
        }
    }
}

// Plain data assembly for the three playback-resolution outcomes below
// (library, USB, USB-after-library-failure).
fn play_resolved_track_data(
    status: PlaybackStatusData,
    requested_path: &str,
    track_id: Option<String>,
    matched_by: &str,
    source: &str,
    source_label: String,
    has_usb_context: bool,
) -> PlayResolvedTrackData {
    let library_resolved = source == "library";
    PlayResolvedTrackData {
        path: status.path.unwrap_or_else(|| requested_path.to_string()),
        playing: status.playing,
        position_ms: status.position_ms,
        duration_ms: status.duration_ms,
        track_id,
        matched_by: matched_by.to_string(),
        source: source.to_string(),
        source_label,
        library_resolved,
        has_usb_context,
    }
}

/// `query` must already be trimmed/lowercased (see `compute_visible_library_tracks`).
fn track_matches_query(track: &Track, query: &str) -> bool {
    let haystack = format!(
        "{} {} {}",
        track.title,
        track.artist,
        track.album.clone().unwrap_or_default()
    )
    .to_lowercase();
    haystack.contains(query)
}

fn sanitize_source_roots(source_roots: Vec<String>) -> Vec<String> {
    let mut roots = Vec::<String>::new();
    for root in source_roots {
        let trimmed = root.trim().to_string();
        if trimmed.is_empty() || roots.iter().any(|existing| existing == &trimmed) {
            continue;
        }
        roots.push(trimmed);
    }
    roots
}

fn source_root_is_usable(root: &str) -> bool {
    let path = Path::new(root);
    path.exists() && path.is_dir()
}

fn source_root_status(root: &str) -> SourceRootStatus {
    let path = Path::new(root);
    SourceRootStatus {
        source_root: root.to_string(),
        exists: path.exists(),
        is_dir: path.is_dir(),
    }
}

fn trimmed_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn track_id_exists(conn: &rusqlite::Connection, track_id: &str) -> BackendResult<bool> {
    let found = conn
        .query_row(
            "SELECT id FROM tracks WHERE id = ?1 LIMIT 1",
            params![track_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn add_candidate_is_usb_origin(
    candidate: &AddTrackCandidate,
    request_usb_root: Option<&str>,
) -> bool {
    let usb_analysis_path = candidate.usb_analysis_path.as_deref().unwrap_or("").trim();
    if !usb_analysis_path.is_empty() {
        return true;
    }

    let file_path = candidate.file_path.as_deref().unwrap_or("").trim();
    let usb_root = candidate
        .usb_root
        .as_deref()
        .or(request_usb_root)
        .unwrap_or("")
        .trim();
    !file_path.is_empty() && !usb_root.is_empty() && browse_path_matches_root(file_path, usb_root)
}

pub(crate) fn normalize_source_root_for_matching(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn relative_path_under_source_root(file_path: &str, source_root: &str) -> Option<String> {
    let normalized_path = file_path.trim().replace('\\', "/");
    let normalized_root = source_root.trim().replace('\\', "/");
    let root = normalized_root.trim_end_matches('/');
    if root.is_empty() {
        return None;
    }

    let path_key = normalized_path.to_ascii_lowercase();
    let root_key = root.to_ascii_lowercase();
    if path_key == root_key {
        return Some(String::new());
    }
    let prefix = format!("{root_key}/");
    if path_key.starts_with(&prefix) {
        return Some(normalized_path[root.len() + 1..].to_string());
    }
    None
}

fn relocated_source_path(new_root: &Path, relative_path: &str) -> PathBuf {
    let trimmed = relative_path.trim_matches('/');
    if trimmed.is_empty() {
        return new_root.to_path_buf();
    }
    let mut out = new_root.to_path_buf();
    for segment in trimmed.split('/') {
        if !segment.is_empty() {
            out.push(segment);
        }
    }
    out
}

/// The single source of truth for "this track has its core analysis": a
/// non-empty waveform-peaks path, a positive BPM, and a positive duration.
/// DB fields only -- deliberately no filesystem check (the export gate's
/// DAT/EXT/2EX bundle verification is separate, see
/// `service::export::has_required_analysis`). Every wrapper below and the
/// `Track::analysis_ready` flag route through this.
pub(crate) fn has_core_analysis_fields(
    waveform_peaks_path: Option<&str>,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
) -> bool {
    let has_waveform_path = waveform_peaks_path
        .map(|path| !path.trim().is_empty())
        .unwrap_or(false);
    let has_bpm = bpm.map(|bpm| bpm > 0.0).unwrap_or(false);
    let has_duration = duration_ms.map(|duration| duration > 0).unwrap_or(false);
    has_waveform_path && has_bpm && has_duration
}

fn track_has_core_analysis_for_source_status(track: &Track) -> bool {
    has_core_analysis_fields(track.waveform_peaks_path.as_deref(), track.bpm, track.duration_ms)
}

fn non_empty_db_value(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn looks_like_windows_absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == b'\\')
}

fn is_pioneer_virtual_path(value: &str) -> bool {
    value
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
        .starts_with("pioneer/")
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|p| p == &path) {
        paths.push(path);
    }
}

fn master_db_resource_candidates(master_path: &Path, db_path: &str) -> Vec<PathBuf> {
    let Some(raw) = non_empty_db_value(db_path) else {
        return Vec::new();
    };

    let master_parent = master_path.parent().unwrap_or(Path::new("."));
    let normalized = raw.replace('\\', "/");
    let relative = normalized.trim_start_matches('/');
    let raw_path = Path::new(raw);
    let mut candidates = Vec::<PathBuf>::new();

    if is_pioneer_virtual_path(raw) {
        // Desktop library stores /PIONEER/... values under <share>/PIONEER/...
        // on Windows. Older layouts may place PIONEER directly beside master.db.
        push_unique_path(&mut candidates, master_parent.join("share").join(relative));
        push_unique_path(&mut candidates, master_parent.join(relative));
    } else {
        if raw_path.is_absolute()
            || looks_like_windows_absolute_path(raw)
            || raw.starts_with("\\\\")
            || raw.starts_with("//")
        {
            push_unique_path(&mut candidates, PathBuf::from(raw));
        }
        push_unique_path(&mut candidates, master_parent.join(relative));
        push_unique_path(&mut candidates, master_parent.join("share").join(relative));
    }

    candidates
}

fn resolve_master_db_resource_path<F>(
    master_path: &Path,
    db_path: &str,
    exists: F,
) -> Option<PathBuf>
where
    F: Fn(&Path) -> bool,
{
    master_db_resource_candidates(master_path, db_path)
        .into_iter()
        .find(|p| exists(p.as_path()))
}

fn master_db_analysis_file_candidates(master_path: &Path, db_path: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::<PathBuf>::new();
    for base in master_db_resource_candidates(master_path, db_path) {
        push_unique_path(&mut candidates, base.with_extension("EXT"));
        push_unique_path(&mut candidates, base.with_extension("2EX"));
        push_unique_path(&mut candidates, base.with_extension("DAT"));
        push_unique_path(&mut candidates, base);
    }
    candidates
}

#[derive(Debug, Clone)]
pub struct BackendService {
    pub db: Db,
    pub analysis_paused: Arc<AtomicBool>,
    pub analysis_cancelled: Arc<AtomicBool>,
}

impl BackendService {
    pub fn new(data_dir: impl AsRef<std::path::Path>) -> BackendResult<Self> {
        let svc = Self {
            db: Db::new(data_dir)?,
            analysis_paused: Arc::new(AtomicBool::new(false)),
            analysis_cancelled: Arc::new(AtomicBool::new(false)),
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
        svc.backfill_usb_devices_from_legacy_settings()?;
        svc.backfill_track_fingerprints()?;
        svc.merge_orphaned_usb_placeholder_tracks()?;
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

    /// One-time migration: seed `usb_devices` from the legacy
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

    pub fn play_track_native(
        &self,
        playback: &PlaybackController,
        req: PlayTrackRequest,
    ) -> BackendResult<PlayTrackData> {
        let status = playback.play_path(&req.path, req.start_offset_ms, req.start_ratio)?;
        Ok(PlayTrackData {
            path: status.path.unwrap_or(req.path),
            playing: status.playing,
            position_ms: status.position_ms,
            duration_ms: status.duration_ms,
        })
    }

    pub fn play_resolved_track(
        &self,
        playback: &PlaybackController,
        req: PlayResolvedTrackRequest,
    ) -> BackendResult<PlayResolvedTrackData> {
        let resolved = self.resolve_playback_source(ResolvePlaybackSourceRequest {
            title: req.title.clone(),
            artist: req.artist.clone(),
            album: req.album.clone(),
            bpm: req.bpm,
            file_path: req.file_path.clone(),
            file_size_bytes: req.file_size_bytes,
            track_id: req.track_id.clone(),
        })?;
        let matched_by = resolved.matched_by.clone();
        let library_path = resolved
            .resolved_path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .filter(|_| matches!(matched_by.as_str(), "self" | "hash" | "metadata"))
            .map(|path| path.trim().to_string());
        let has_usb_context = req.usb_root_valid
            && req
                .usb_root
                .as_deref()
                .map(|root| !root.trim().is_empty())
                .unwrap_or(false);
        let usb_path = if has_usb_context {
            let track_path = req.file_path.as_deref().unwrap_or("").trim();
            let root = req.usb_root.as_deref().unwrap_or("").trim();
            if !track_path.is_empty() && browse_path_matches_root(track_path, root) {
                Some(track_path.to_string())
            } else {
                None
            }
        } else {
            None
        };

        if library_path.is_none() && usb_path.is_none() {
            return Err(BackendError::NotFound(
                "track not found in Library or selected USB".to_string(),
            ));
        }

        if let Some(path) = library_path.as_deref() {
            match play_native_with_recovery(playback, path, req.start_offset_ms, req.start_ratio) {
                Ok(status) => {
                    return Ok(play_resolved_track_data(
                        status,
                        path,
                        resolved.track_id.or(req.track_id),
                        &matched_by,
                        "library",
                        playback_source_label(req.origin.as_deref(), true, has_usb_context),
                        has_usb_context,
                    ));
                }
                Err(library_err) => {
                    if let Some(path) = usb_path
                        .as_deref()
                        .filter(|usb_path| browse_path_key(usb_path) != browse_path_key(path))
                    {
                        let status = play_native_with_recovery(
                            playback,
                            path,
                            req.start_offset_ms,
                            req.start_ratio,
                        )?;
                        return Ok(play_resolved_track_data(
                            status,
                            path,
                            req.track_id,
                            &matched_by,
                            "usb",
                            "USB (library unavailable)".to_string(),
                            has_usb_context,
                        ));
                    }
                    return Err(library_err);
                }
            }
        }

        if let Some(path) = usb_path.as_deref() {
            let status =
                play_native_with_recovery(playback, path, req.start_offset_ms, req.start_ratio)?;
            return Ok(play_resolved_track_data(
                status,
                path,
                req.track_id,
                &matched_by,
                "usb",
                playback_source_label(req.origin.as_deref(), false, has_usb_context),
                has_usb_context,
            ));
        }

        Err(BackendError::NotFound(
            "track not found in Library or selected USB".to_string(),
        ))
    }

    pub fn stop_playback_native(
        &self,
        playback: &PlaybackController,
    ) -> BackendResult<StopPlaybackData> {
        let status = playback.stop()?;
        Ok(StopPlaybackData {
            stopped: true,
            previous_path: status.path,
        })
    }

    pub fn get_playback_status_native(
        &self,
        playback: &PlaybackController,
    ) -> BackendResult<PlaybackStatusData> {
        playback.status()
    }

    pub fn playback_preflight_native(
        &self,
        req: PlaybackPreflightRequest,
    ) -> BackendResult<PlaybackPreflightData> {
        run_playback_preflight(&req.path)
    }

    pub fn detect_external_master_db(&self) -> BackendResult<DetectExternalMasterDbData> {
        Ok(detect_external_master_db_util())
    }

    pub fn initialize_usb(&self, req: InitializeUsbRequest) -> BackendResult<InitializeUsbData> {
        initialize_usb_util(&req.usb_root)
    }

    pub fn scan_library(&self, req: ScanLibraryRequest) -> BackendResult<ScanLibraryData> {
        let source_roots = sanitize_source_roots(req.source_roots);
        if source_roots.is_empty() {
            return Err(BackendError::Validation(
                "sourceRoots must contain at least one path".to_string(),
            ));
        }

        let mut existing_roots = Vec::<String>::new();
        let mut not_found = Vec::<String>::new();
        for root in source_roots {
            if source_root_is_usable(&root) {
                existing_roots.push(root);
            } else {
                not_found.push(root);
            }
        }
        let mut warnings = Vec::<WarningEntry>::new();
        if !not_found.is_empty() {
            warnings.push(logging::log(
                Level::Warn,
                "library-scan",
                "scan.source-folders-missing",
                format!(
                    "{} source folder(s) missing or not directories: {}",
                    not_found.len(),
                    not_found.join(", ")
                ),
            ));
        }

        if existing_roots.is_empty() {
            return Ok(ScanLibraryData {
                job_id: Uuid::now_v7().to_string(),
                indexed: 0,
                updated: 0,
                removed: 0,
                not_found,
                warnings,
            });
        }

        let scanned = scan_audio_files(&existing_roots)?;
        let now = now();
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;

        // Pre-load all existing tracks into a HashMap to avoid N+1 queries
        let mut existing_tracks = ExistingTracksByPath::new();
        {
            let mut stmt = tx.prepare(
                "SELECT id, file_modified_at, title, artist, album, track_number, tonality, format_ext, sample_rate_hz, bit_depth, bitrate_kbps, disc_number, subtitle, comment, isrc, release_year, release_date, recorded_date, genre, file_path, wav_extensible_kind FROM tracks",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(19)?, // file_path as key
                    (
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<u32>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<u32>>(8)?,
                        row.get::<_, Option<u8>>(9)?,
                        row.get::<_, Option<u32>>(10)?,
                        row.get::<_, Option<u32>>(11)?,
                        row.get::<_, Option<String>>(12)?,
                        row.get::<_, Option<String>>(13)?,
                        row.get::<_, Option<String>>(14)?,
                        row.get::<_, Option<u32>>(15)?,
                        row.get::<_, Option<String>>(16)?,
                        row.get::<_, Option<String>>(17)?,
                        row.get::<_, Option<String>>(18)?, // genre
                        row.get::<_, Option<String>>(20)?, // wav_extensible_kind
                    ),
                ))
            })?;
            for row in rows {
                let (path, data) = row?;
                existing_tracks.insert(path, data);
            }
        }

        let mut indexed = 0usize;
        let mut updated = 0usize;

        for item in &scanned {
            let existing = existing_tracks.get(&item.path).cloned();

            match existing {
                None => {
                    let id = Uuid::now_v7().to_string();
                    let fingerprint = build_track_match_fingerprint(
                        &item.title,
                        &item.artist,
                        item.album.as_deref(),
                    );
                    let wav_extensible_kind =
                        item.wav_extensible_kind.map(WavFormatIssue::as_db_str);
                    tx.execute(
                        r#"
                        INSERT INTO tracks (
                          id, title, artist, album, track_number, bpm, tonality, file_path,
                          file_size_bytes, file_modified_at, format_ext, sample_rate_hz, bit_depth, bitrate_kbps,
                          disc_number, subtitle, comment, isrc, release_year, release_date, recorded_date,
                          genre, duration_ms, artwork_path, waveform_peaks_path, match_fingerprint,
                          created_at, updated_at, wav_extensible_kind
                        ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, NULL, NULL, NULL, ?22, ?23, ?23, ?24)
                        "#,
                        params![
                            id,
                            item.title,
                            item.artist,
                            item.album,
                            item.track_number,
                            item.tonality,
                            item.path,
                            item.file_size_bytes,
                            item.file_modified_at,
                            item.format_ext,
                            item.sample_rate_hz,
                            item.bit_depth,
                            item.bitrate_kbps,
                            item.disc_number,
                            item.subtitle,
                            item.comment,
                            item.isrc,
                            item.release_year,
                            item.release_date,
                            item.recorded_date,
                            item.genre,
                            fingerprint,
                            now,
                            wav_extensible_kind
                        ],
                    )?;
                    indexed += 1;
                }
                Some((
                    id,
                    old_modified,
                    old_title,
                    old_artist,
                    old_album,
                    old_track_number,
                    old_tonality,
                    old_format_ext,
                    old_sample_rate_hz,
                    old_bit_depth,
                    old_bitrate_kbps,
                    old_disc_number,
                    old_subtitle,
                    old_comment,
                    old_isrc,
                    old_release_year,
                    old_release_date,
                    old_recorded_date,
                    old_genre,
                    old_wav_extensible_kind,
                )) => {
                    let wav_extensible_kind =
                        item.wav_extensible_kind.map(WavFormatIssue::as_db_str);
                    let tonality_changed = item.tonality.is_some() && old_tonality != item.tonality;
                    let metadata_changed = old_title != item.title
                        || old_artist != item.artist
                        || old_album != item.album
                        || old_track_number != item.track_number
                        || tonality_changed
                        || old_format_ext != item.format_ext
                        || old_sample_rate_hz != item.sample_rate_hz
                        || old_bit_depth != item.bit_depth
                        || old_bitrate_kbps != item.bitrate_kbps
                        || old_disc_number != item.disc_number
                        || old_subtitle != item.subtitle
                        || old_comment != item.comment
                        || old_isrc != item.isrc
                        || old_release_year != item.release_year
                        || old_release_date != item.release_date
                        || old_recorded_date != item.recorded_date
                        || old_genre != item.genre
                        || old_wav_extensible_kind.as_deref() != wav_extensible_kind;
                    if old_modified != item.file_modified_at || metadata_changed {
                        let fingerprint = build_track_match_fingerprint(
                            &item.title,
                            &item.artist,
                            item.album.as_deref(),
                        );
                        tx.execute(
                            r#"
                            UPDATE tracks
                            SET title = ?1,
                                artist = ?2,
                                album = ?3,
                                track_number = ?4,
                                tonality = COALESCE(?5, tonality),
                                file_size_bytes = ?6,
                                file_modified_at = ?7,
                                format_ext = ?8,
                                sample_rate_hz = ?9,
                                bit_depth = ?10,
                                bitrate_kbps = ?11,
                                disc_number = ?12,
                                subtitle = ?13,
                                comment = ?14,
                                isrc = ?15,
                                release_year = ?16,
                                release_date = ?17,
                                recorded_date = ?18,
                                genre = ?19,
                                match_fingerprint = ?20,
                                updated_at = ?21,
                                wav_extensible_kind = ?23
                            WHERE id = ?22
                            "#,
                            params![
                                item.title,
                                item.artist,
                                item.album,
                                item.track_number,
                                item.tonality,
                                item.file_size_bytes,
                                item.file_modified_at,
                                item.format_ext,
                                item.sample_rate_hz,
                                item.bit_depth,
                                item.bitrate_kbps,
                                item.disc_number,
                                item.subtitle,
                                item.comment,
                                item.isrc,
                                item.release_year,
                                item.release_date,
                                item.recorded_date,
                                item.genre,
                                fingerprint,
                                now,
                                id,
                                wav_extensible_kind
                            ],
                        )?;
                        updated += 1;
                    }
                }
            }
        }

        let scanned_paths = unique_paths(&scanned);
        let mut removed = 0usize;
        let analysis_dir = self.db.data_dir().join("analysis");
        let waveform_dir = analysis_dir.join("waveforms");
        let artwork_dir = analysis_dir.join("artwork");
        for root in &existing_roots {
            let escaped_root = root
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let slash_like = format!("{escaped_root}/%");
            let backslash_like = format!("{escaped_root}\\\\%");
            let mut stmt = tx.prepare(
                "SELECT id, file_path FROM tracks WHERE file_path = ?1 OR file_path LIKE ?2 ESCAPE '\\' OR file_path LIKE ?3 ESCAPE '\\'",
            )?;
            let rows = stmt.query_map(params![root, slash_like, backslash_like], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;

            for row in rows {
                let (id, path) = row?;
                if !scanned_paths.contains(path.as_str()) {
                    tx.execute("DELETE FROM tracks WHERE id = ?1", params![id])?;
                    // Clean up ANLZ cache and artwork files
                    for ext in ["DAT", "EXT", "2EX"] {
                        let _ = std::fs::remove_file(waveform_dir.join(format!("{id}.{ext}")));
                    }
                    let _ = std::fs::remove_file(artwork_dir.join(format!("{id}.jpg")));
                    removed += 1;
                }
            }
        }

        // Detect stale ANLZ cache (missing PWV6 in .2EX) and mark for re-analysis
        {
            let mut stmt =
                tx.prepare("SELECT id FROM tracks WHERE waveform_peaks_path IS NOT NULL")?;
            let ids: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            for id in &ids {
                let twoex = waveform_dir.join(format!("{id}.2EX"));
                if twoex.is_file()
                    && let Ok(bytes) = std::fs::read(&twoex)
                    && !bytes.windows(4).any(|w| w == b"PWV6")
                {
                    tx.execute(
                        "UPDATE tracks SET waveform_peaks_path = NULL WHERE id = ?1",
                        params![id],
                    )?;
                    for ext in ["DAT", "EXT", "2EX"] {
                        let _ = std::fs::remove_file(waveform_dir.join(format!("{id}.{ext}")));
                    }
                }
            }
        }

        tx.commit()?;

        Ok(ScanLibraryData {
            job_id: Uuid::now_v7().to_string(),
            indexed,
            updated,
            removed,
            not_found,
            warnings,
        })
    }

    pub fn scan_master_db(&self, req: ScanMasterDbRequest) -> BackendResult<ScanLibraryData> {
        use self::usb_utils::external_master_db_candidates;
        use self::usb_vendor_compat::DEFAULT_MASTER_DB_KEY;

        // Resolve path: explicit request > auto-detect
        let master_path = if let Some(p) = req.path.as_deref().filter(|s| !s.trim().is_empty()) {
            std::path::PathBuf::from(p.trim())
        } else {
            external_master_db_candidates()
                .into_iter()
                .find(|c| c.is_file())
                .ok_or_else(|| BackendError::Validation("master.db not found".to_string()))?
        };

        if !master_path.is_file() {
            return Err(BackendError::Validation(format!(
                "master.db not found: {}",
                master_path.display()
            )));
        }

        // Open read-only with SQLCipher key
        let conn = rusqlite::Connection::open_with_flags(
            &master_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| BackendError::Validation(format!("could not open master.db: {e}")))?;
        conn.execute_batch(&format!("PRAGMA key='{}';", DEFAULT_MASTER_DB_KEY))
            .map_err(|e| BackendError::Validation(format!("master.db key failed: {e}")))?;

        // Verify we can read the schema
        let ok: bool = conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master WHERE type='table' AND name='djmdContent'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !ok {
            return Err(BackendError::Validation(
                "master.db opened but djmdContent table not found (wrong key or version)"
                    .to_string(),
            ));
        }

        // Query all non-deleted tracks with available metadata.
        // FolderPath is the full file path (despite the name).
        // BPM is stored as centiBPM integer (12600 = 126.00 BPM).
        // Key is a FK into djmdKey; ScaleName holds the human-readable name.
        // AnalysisDataPath and ImagePath are desktop library virtual paths on Windows
        // (/PIONEER/...) and resolve under the share directory.
        let mut stmt = conn
            .prepare(
                r#"
            SELECT
              c.FolderPath,
              c.Title,
              COALESCE(ar.Name, c.SrcArtistName, '') AS Artist,
              COALESCE(al.Name, '')                   AS Album,
              c.BPM,
              k.ScaleName,
              c.Length,
              c.AnalysisDataPath,
              c.ImagePath
            FROM djmdContent c
            LEFT JOIN djmdArtist  ar ON ar.ID = c.ArtistID
            LEFT JOIN djmdAlbum   al ON al.ID = c.AlbumID
            LEFT JOIN djmdKey     k  ON k.ID  = c.KeyID
            WHERE IFNULL(c.rb_local_deleted, 0) = 0
              AND c.FolderPath IS NOT NULL
            "#,
            )
            .map_err(|e| BackendError::Validation(format!("master.db query failed: {e}")))?;

        struct RbTrack {
            file_path: String,
            title: String,
            artist: String,
            album: String,
            bpm: Option<f64>,
            tonality: Option<String>,
            duration_ms: Option<i64>,
            anlz_path: Option<String>,
            image_path: Option<String>,
        }

        let tracks: Vec<RbTrack> = stmt
            .query_map([], |row| {
                Ok(RbTrack {
                    file_path: row.get::<_, String>(0)?,
                    title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    artist: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    album: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    bpm: row.get::<_, Option<i64>>(4)?.map(|b| b as f64 / 100.0),
                    tonality: row.get::<_, Option<String>>(5)?,
                    duration_ms: row.get::<_, Option<i64>>(6)?.map(|s| s * 1000),
                    anlz_path: row.get::<_, Option<String>>(7)?,
                    image_path: row.get::<_, Option<String>>(8)?,
                })
            })
            .map_err(|e| BackendError::Validation(format!("master.db row error: {e}")))?
            .filter_map(|r| r.ok())
            .filter(|t| !t.file_path.trim().is_empty())
            .collect();

        let now = now();
        let mut db_conn = self.db.connect()?;
        let tx = db_conn.transaction()?;

        // Load existing tracks by file_path for upsert logic
        let mut existing: std::collections::HashMap<String, String> = {
            let mut stmt = tx.prepare("SELECT file_path, id FROM tracks")?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect()
        };

        let analysis_dir = self.db.data_dir().join("analysis");
        let artwork_dir = analysis_dir.join("artwork");
        let _ = std::fs::create_dir_all(&artwork_dir);

        let mut indexed = 0usize;
        let mut updated = 0usize;
        let mut removed = 0usize;
        let mut not_found: Vec<String> = Vec::new();
        let mut anlz_null = 0usize;
        let mut anlz_miss = 0usize;
        let mut anlz_ok = 0usize;
        let mut artwork_null = 0usize;
        let mut artwork_miss = 0usize;
        let mut artwork_ok = 0usize;
        let mut sample_anlz: Option<String> = None;
        let mut sample_img: Option<String> = None;
        let mut warnings = Vec::<WarningEntry>::new();

        for t in &tracks {
            if !std::path::Path::new(&t.file_path).exists() {
                // Remove from local DB if previously imported; skip upsert
                if existing.remove(&t.file_path).is_some() {
                    tx.execute(
                        "DELETE FROM tracks WHERE file_path = ?1",
                        params![t.file_path],
                    )?;
                    removed += 1;
                }
                not_found.push(t.file_path.clone());
                continue;
            }

            let fingerprint = build_track_match_fingerprint(
                &t.title,
                &t.artist,
                Some(t.album.as_str()).filter(|s| !s.is_empty()),
            );

            // Resolve (or generate) the track ID before any file writes
            let is_update = existing.contains_key(&t.file_path);
            let track_id = if is_update {
                existing[&t.file_path].clone()
            } else {
                Uuid::now_v7().to_string()
            };

            // Waveform: store the original ANLZ path in place - no copy, no conversion.
            // PWV4 bytes are extracted later when the track is loaded for display.
            let waveform_path = match t.anlz_path.as_deref().and_then(non_empty_db_value) {
                None => {
                    anlz_null += 1;
                    None
                }
                Some(anlz_rel) => {
                    if sample_anlz.is_none() {
                        sample_anlz = Some(anlz_rel.to_string());
                    }
                    let resolved = master_db_analysis_file_candidates(&master_path, anlz_rel)
                        .into_iter()
                        .find(|p| p.is_file());
                    if let Some(anlz_abs) = resolved {
                        anlz_ok += 1;
                        anlz_abs.to_str().map(str::to_owned)
                    } else {
                        anlz_miss += 1;
                        warnings.push(logging::log(
                            Level::Warn,
                            "scan-master-db",
                            "scan.master-db.anlz-not-found",
                            format!("ANLZ not found (AnalysisDataPath={anlz_rel:?})"),
                        ));
                        None
                    }
                }
            };

            // Artwork from djmdContent.ImagePath.
            let artwork_path = match t.image_path.as_deref().and_then(non_empty_db_value) {
                None => {
                    artwork_null += 1;
                    None
                }
                Some(img_rel) => {
                    if sample_img.is_none() {
                        sample_img = Some(img_rel.to_string());
                    }
                    match resolve_master_db_resource_path(&master_path, img_rel, |p| p.is_file()) {
                        None => {
                            artwork_miss += 1;
                            warnings.push(logging::log(
                                Level::Warn,
                                "scan-master-db",
                                "scan.master-db.artwork-not-found",
                                format!("artwork not found (ImagePath={img_rel:?})"),
                            ));
                            None
                        }
                        Some(src) => {
                            let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
                            let dest = artwork_dir.join(format!("{track_id}.{ext}"));
                            match std::fs::copy(&src, &dest) {
                                Ok(_) => {
                                    artwork_ok += 1;
                                    Some(dest.to_string_lossy().to_string())
                                }
                                Err(e) => {
                                    artwork_miss += 1;
                                    warnings.push(logging::log(
                                        Level::Error,
                                        "scan-master-db",
                                        "scan.master-db.artwork-copy-failed",
                                        format!("artwork copy failed {src:?} -> {dest:?}: {e}"),
                                    ));
                                    None
                                }
                            }
                        }
                    }
                }
            };

            if is_update {
                tx.execute(
                    r#"UPDATE tracks SET
                        title = ?1, artist = ?2, album = ?3,
                        bpm = COALESCE(bpm, ?4),
                        tonality = COALESCE(tonality, ?5),
                        duration_ms = COALESCE(duration_ms, ?6),
                        waveform_peaks_path = COALESCE(?7, waveform_peaks_path),
                        artwork_path = COALESCE(?8, artwork_path),
                        format_ext = COALESCE(format_ext, ?12),
                        match_fingerprint = ?9,
                        master_db_source = 1,
                        updated_at = ?10
                       WHERE id = ?11"#,
                    params![
                        t.title,
                        t.artist,
                        if t.album.is_empty() {
                            None
                        } else {
                            Some(&t.album)
                        },
                        t.bpm,
                        t.tonality,
                        t.duration_ms,
                        waveform_path,
                        artwork_path,
                        fingerprint,
                        now,
                        track_id,
                        crate::utils::format_ext_from_path(&t.file_path)
                    ],
                )?;
                updated += 1;
            } else {
                tx.execute(
                    r#"INSERT INTO tracks (
                        id, title, artist, album, bpm, tonality, file_path, format_ext,
                        duration_ms, waveform_peaks_path, artwork_path, match_fingerprint,
                        master_db_source, created_at, updated_at
                       ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,1,?13,?13)"#,
                    params![
                        track_id,
                        t.title,
                        t.artist,
                        if t.album.is_empty() {
                            None
                        } else {
                            Some(&t.album)
                        },
                        t.bpm,
                        t.tonality,
                        t.file_path,
                        crate::utils::format_ext_from_path(&t.file_path),
                        t.duration_ms,
                        waveform_path,
                        artwork_path,
                        fingerprint,
                        now
                    ],
                )?;
                existing.insert(t.file_path.clone(), track_id);
                indexed += 1;
            }
        }

        tx.commit()?;

        if let Some(p) = sample_anlz {
            warnings.push(logging::log(
                Level::Info,
                "scan-master-db",
                "scan.master-db.anlz-sample",
                format!("AnalysisDataPath sample: {p}"),
            ));
        }
        if let Some(p) = sample_img {
            warnings.push(logging::log(
                Level::Info,
                "scan-master-db",
                "scan.master-db.image-sample",
                format!("ImagePath sample: {p}"),
            ));
        }
        if anlz_null > 0 {
            warnings.push(logging::log(
                Level::Info,
                "scan-master-db",
                "scan.master-db.anlz-null",
                format!("{anlz_null} track(s) have no AnalysisDataPath"),
            ));
        }
        if anlz_miss > 0 {
            warnings.push(logging::log(
                Level::Warn,
                "scan-master-db",
                "scan.master-db.anlz-miss-summary",
                format!("{anlz_miss} ANLZ path(s) not found on disk"),
            ));
        }
        if anlz_ok > 0 {
            warnings.push(logging::log(
                Level::Info,
                "scan-master-db",
                "scan.master-db.anlz-ok",
                format!("{anlz_ok} ANLZ path(s) resolved OK"),
            ));
        }
        if artwork_null > 0 {
            warnings.push(logging::log(
                Level::Info,
                "scan-master-db",
                "scan.master-db.artwork-null",
                format!("{artwork_null} track(s) have no ImagePath"),
            ));
        }
        if artwork_miss > 0 {
            warnings.push(logging::log(
                Level::Warn,
                "scan-master-db",
                "scan.master-db.artwork-miss-summary",
                format!("{artwork_miss} artwork source file(s) not found or copy failed"),
            ));
        }
        if artwork_ok > 0 {
            warnings.push(logging::log(
                Level::Info,
                "scan-master-db",
                "scan.master-db.artwork-ok",
                format!("{artwork_ok} artwork file(s) copied OK"),
            ));
        }

        Ok(ScanLibraryData {
            job_id: Uuid::now_v7().to_string(),
            indexed,
            updated,
            removed,
            not_found,
            warnings,
        })
    }

    pub fn search_tracks(&self, req: SearchTracksRequest) -> BackendResult<SearchTracksData> {
        let limit = req.limit.clamp(1, TRACK_QUERY_LIMIT_MAX);
        let query = req.query.trim();
        let signature =
            build_track_cursor_signature(&[TRACK_CURSOR_VERSION, "search_tracks", query]);
        let cursor = decode_track_page_cursor(req.cursor.as_deref(), &signature)?;
        let fetch_limit = limit + 1;

        let conn = self.db.connect()?;

        let (total, mut items) = if query.is_empty() {
            let total: i64 = conn.query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))?;
            let items = if let Some(cursor) = cursor.as_ref() {
                let mut stmt = conn.prepare(
                    &format!("SELECT {TRACK_COLS} FROM tracks WHERE file_path COLLATE NOCASE > ?1 OR (file_path COLLATE NOCASE = ?1 AND id > ?2) ORDER BY file_path COLLATE NOCASE ASC, id ASC LIMIT ?3"),
                )?;
                let rows = stmt.query_map(
                    params![cursor.file_path, cursor.id, fetch_limit as i64],
                    |row| row_to_track(row, false),
                )?;
                rows.collect::<Result<Vec<_>, _>>()?
            } else {
                let mut stmt = conn.prepare(
                    &format!("SELECT {TRACK_COLS} FROM tracks ORDER BY file_path COLLATE NOCASE ASC, id ASC LIMIT ?1"),
                )?;
                let rows =
                    stmt.query_map(params![fetch_limit as i64], |row| row_to_track(row, false))?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            (total as usize, items)
        } else {
            let like = format!("%{query}%");
            let total: i64 = conn.query_row(
                "SELECT COUNT(*) FROM tracks WHERE title LIKE ?1 OR artist LIKE ?1 OR IFNULL(album,'') LIKE ?1",
                params![like],
                |row| row.get(0),
            )?;

            let items = if let Some(cursor) = cursor.as_ref() {
                let mut stmt = conn.prepare(
                    &format!("SELECT {TRACK_COLS} FROM tracks WHERE (title LIKE ?1 OR artist LIKE ?1 OR IFNULL(album,'') LIKE ?1) AND (file_path COLLATE NOCASE > ?2 OR (file_path COLLATE NOCASE = ?2 AND id > ?3)) ORDER BY file_path COLLATE NOCASE ASC, id ASC LIMIT ?4"),
                )?;
                let rows = stmt.query_map(
                    params![like, cursor.file_path, cursor.id, fetch_limit as i64],
                    |row| row_to_track(row, false),
                )?;
                rows.collect::<Result<Vec<_>, _>>()?
            } else {
                let mut stmt = conn.prepare(
                    &format!("SELECT {TRACK_COLS} FROM tracks WHERE title LIKE ?1 OR artist LIKE ?1 OR IFNULL(album,'') LIKE ?1 ORDER BY file_path COLLATE NOCASE ASC, id ASC LIMIT ?2"),
                )?;
                let rows = stmt.query_map(params![like, fetch_limit as i64], |row| {
                    row_to_track(row, false)
                })?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            (total as usize, items)
        };

        let (has_more, next_cursor) = paginate_tracks(&mut items, limit, &signature);
        apply_is_usb_path(&conn, &mut items)?;
        Ok(SearchTracksData {
            total,
            items,
            next_cursor,
            has_more,
        })
    }

    pub fn list_tracks(&self, req: ListTracksRequest) -> BackendResult<ListTracksData> {
        let limit = req.limit.clamp(1, TRACK_QUERY_LIMIT_MAX);
        let signature = build_track_cursor_signature(&[TRACK_CURSOR_VERSION, "list_tracks"]);
        let cursor = decode_track_page_cursor(req.cursor.as_deref(), &signature)?;
        let fetch_limit = limit + 1;
        let conn = self.db.connect()?;
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))?;
        let mut items = if let Some(cursor) = cursor.as_ref() {
            let mut stmt = conn.prepare(
                &format!("SELECT {TRACK_COLS} FROM tracks WHERE file_path COLLATE NOCASE > ?1 OR (file_path COLLATE NOCASE = ?1 AND id > ?2) ORDER BY file_path COLLATE NOCASE ASC, id ASC LIMIT ?3"),
            )?;
            let rows = stmt.query_map(
                params![cursor.file_path, cursor.id, fetch_limit as i64],
                |row| row_to_track(row, false),
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = conn.prepare(
                &format!("SELECT {TRACK_COLS} FROM tracks ORDER BY file_path COLLATE NOCASE ASC, id ASC LIMIT ?1"),
            )?;
            let rows =
                stmt.query_map(params![fetch_limit as i64], |row| row_to_track(row, false))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let (has_more, next_cursor) = paginate_tracks(&mut items, limit, &signature);
        apply_is_usb_path(&conn, &mut items)?;
        Ok(ListTracksData {
            total: total as usize,
            items,
            next_cursor,
            has_more,
        })
    }

    /// Builds the complete library track list for a set of source roots
    /// (plus optional master.db tracks) -- NOT filtered by search query.
    /// This is the expensive, I/O-bound part (filesystem scan + DB read),
    /// shared by `compute_visible_library_tracks` below and by
    /// `browse_source_files` directly (which needs the *unfiltered* set to
    /// compute `source_root_analysis` -- a folder's analyzed status must
    /// not depend on the current search query -- without scanning twice).
    /// Sanitizes/sorts/dedups `source_roots` itself, so callers may pass
    /// raw values.
    fn compute_all_library_tracks(
        &self,
        conn: &rusqlite::Connection,
        source_roots: &[String],
        include_master_db: bool,
    ) -> BackendResult<Vec<Track>> {
        let mut source_roots = source_roots
            .iter()
            .map(|root| root.trim().to_string())
            .filter(|root| !root.is_empty())
            .collect::<Vec<_>>();
        source_roots.sort();
        source_roots.dedup();
        if source_roots.is_empty() && !include_master_db {
            return Ok(Vec::new());
        }

        let scanned = if source_roots.is_empty() {
            Vec::new()
        } else {
            scan_audio_files(&source_roots)?
        };
        let mut stmt = conn.prepare(&format!("SELECT {TRACK_COLS} FROM tracks"))?;
        let indexed_rows = stmt.query_map([], |row| row_to_track(row, false))?;
        let indexed_tracks = indexed_rows.collect::<Result<Vec<_>, _>>()?;
        let indexed_by_path = indexed_tracks
            .iter()
            .cloned()
            .map(|track| (browse_path_key(&track.file_path), track))
            .collect::<std::collections::HashMap<_, _>>();

        let mut seen_paths = std::collections::HashSet::<String>::new();
        let mut items = scanned
            .into_iter()
            .map(|scanned| {
                let scanned_key = browse_path_key(&scanned.path);
                seen_paths.insert(scanned_key.clone());
                if let Some(existing) = indexed_by_path.get(&scanned_key) {
                    existing.clone()
                } else {
                    let now = now();
                    Track {
                        id: scanned.path.clone(),
                        title: scanned.title,
                        artist: scanned.artist,
                        album: scanned.album,
                        track_number: scanned.track_number,
                        bpm: None,
                        bpm_analyzer: None,
                        key: scanned.tonality,
                        file_path: scanned.path,
                        file_size_bytes: scanned.file_size_bytes,
                        format_ext: scanned.format_ext,
                        sample_rate_hz: scanned.sample_rate_hz,
                        bit_depth: scanned.bit_depth,
                        bitrate_kbps: scanned.bitrate_kbps,
                        wav_extensible_kind: scanned
                            .wav_extensible_kind
                            .map(|kind| kind.as_db_str().to_string()),
                        duration_ms: None,
                        artwork_path: None,
                        artwork_data_url: None,
                        waveform_peaks_path: None,
                        waveform_preview: None,
                        waveform_color_data: None,
                        created_at: now.clone(),
                        updated_at: now,
                        master_db_source: false,
                        is_usb_path: false,
                        // Freshly scanned, not yet indexed -- no analysis.
                        analysis_ready: false,
                    }
                }
            })
            .collect::<Vec<_>>();

        if include_master_db {
            for track in indexed_tracks
                .into_iter()
                .filter(|track| track.master_db_source)
            {
                let path_key = browse_path_key(&track.file_path);
                if seen_paths.insert(path_key) {
                    items.push(track);
                }
            }
        }

        items.sort_by(|a, b| {
            browse_path_key(&a.file_path)
                .cmp(&browse_path_key(&b.file_path))
                .then_with(|| a.id.cmp(&b.id))
        });

        Ok(items)
    }

    /// Builds the complete filtered track list for a set of source roots
    /// (plus optional master.db tracks) matching a search query -- the same
    /// filtering `browse_source_files` returns to the frontend before it's
    /// paginated. Shared by `browse_source_files` and the analysis pipeline
    /// (which uses it to compute the live library-duration-total baseline),
    /// so both stay consistent by construction. Sanitizes/sorts/dedups
    /// `source_roots` itself, so callers may pass raw values.
    fn compute_visible_library_tracks(
        &self,
        conn: &rusqlite::Connection,
        source_roots: &[String],
        include_master_db: bool,
        query: &str,
    ) -> BackendResult<Vec<Track>> {
        let mut items = self.compute_all_library_tracks(conn, source_roots, include_master_db)?;
        let query = query.trim().to_lowercase();
        if !query.is_empty() {
            items.retain(|track| track_matches_query(track, &query));
        }
        Ok(items)
    }

    pub fn browse_source_files(
        &self,
        req: BrowseSourceFilesRequest,
    ) -> BackendResult<BrowseSourceFilesData> {
        let mut source_roots = req
            .source_roots
            .into_iter()
            .map(|root| root.trim().to_string())
            .filter(|root| !root.is_empty())
            .collect::<Vec<_>>();
        source_roots.sort();
        source_roots.dedup();
        let include_master_db = req.include_master_db;
        if source_roots.is_empty() && !include_master_db {
            return Ok(BrowseSourceFilesData {
                total: 0,
                items: Vec::new(),
                next_cursor: None,
                has_more: false,
                source_root_analysis: Vec::new(),
                total_duration_ms: 0,
                duration_known_count: 0,
            });
        }

        let limit = req.limit.clamp(1, 5000);
        let query = req.query.trim().to_lowercase();
        let roots_signature = source_roots.join("\u{1F}");
        let signature = build_track_cursor_signature(&[
            TRACK_CURSOR_VERSION,
            "browse_source_files",
            &query,
            &roots_signature,
            if include_master_db {
                "master_db"
            } else {
                "no_master_db"
            },
        ]);
        let cursor = decode_track_page_cursor(req.cursor.as_deref(), &signature)?;

        let conn = self.db.connect()?;
        let all_items = self.compute_all_library_tracks(&conn, &source_roots, include_master_db)?;

        // source_root_analysis describes each folder's own analyzed state
        // and must not depend on the current search query -- otherwise a
        // query that doesn't match (all of) a folder's tracks makes an
        // otherwise fully-analyzed folder's chip appear unanalyzed (in the
        // worst case, a query matching zero tracks in a folder collapses
        // `total` to 0, and `fully_analyzed` requires `total > 0`). Computed
        // from the full, unfiltered set -- before the query filter below.
        let source_root_analysis = source_roots
            .iter()
            .map(|root| {
                let mut total = 0usize;
                let mut analyzed = 0usize;
                for track in &all_items {
                    if browse_path_matches_root(&track.file_path, root) {
                        total += 1;
                        if track_has_core_analysis_for_source_status(track) {
                            analyzed += 1;
                        }
                    }
                }
                SourceRootAnalysisStatus {
                    source_root: root.clone(),
                    total,
                    analyzed,
                    fully_analyzed: total > 0 && analyzed == total,
                }
            })
            .collect::<Vec<_>>();

        let mut items = all_items;
        if !query.is_empty() {
            items.retain(|track| track_matches_query(track, &query));
        }

        let mut total_duration_ms: u64 = 0;
        let mut duration_known_count: usize = 0;
        for track in &items {
            let has_duration = track.duration_ms.map(|d| d > 0).unwrap_or(false);
            let countable = track_has_core_analysis_for_source_status(track)
                || (track.master_db_source && has_duration);
            if countable && let Some(d) = track.duration_ms {
                total_duration_ms += d;
                duration_known_count += 1;
            }
        }

        let total = items.len();
        let start_idx = if let Some(cursor) = cursor.as_ref() {
            items
                .iter()
                .position(|track| {
                    browse_path_key(&track.file_path) == browse_path_key(&cursor.file_path)
                        && track.id == cursor.id
                })
                .map(|idx| idx + 1)
                .unwrap_or(0)
        } else {
            0
        };
        let mut page_items = items
            .into_iter()
            .skip(start_idx)
            .take(limit + 1)
            .collect::<Vec<_>>();
        let (has_more, next_cursor) = paginate_tracks(&mut page_items, limit, &signature);
        apply_is_usb_path(&conn, &mut page_items)?;
        Ok(BrowseSourceFilesData {
            total,
            items: page_items,
            next_cursor,
            has_more,
            source_root_analysis,
            total_duration_ms,
            duration_known_count,
        })
    }

    pub fn check_source_roots(
        &self,
        req: CheckSourceRootsRequest,
    ) -> BackendResult<CheckSourceRootsData> {
        let items = sanitize_source_roots(req.source_roots)
            .into_iter()
            .map(|root| source_root_status(&root))
            .collect::<Vec<_>>();
        let missing = items
            .iter()
            .filter(|item| !item.exists || !item.is_dir)
            .map(|item| item.source_root.clone())
            .collect::<Vec<_>>();

        Ok(CheckSourceRootsData { items, missing })
    }

    pub fn relocate_source_root(
        &self,
        req: RelocateSourceRootRequest,
    ) -> BackendResult<RelocateSourceRootData> {
        let old_root = req.old_root.trim().to_string();
        let new_root = req.new_root.trim().to_string();
        if old_root.is_empty() || new_root.is_empty() {
            return Err(BackendError::Validation(
                "oldRoot and newRoot must not be empty".to_string(),
            ));
        }
        if normalize_source_root_for_matching(&old_root)
            == normalize_source_root_for_matching(&new_root)
        {
            return Err(BackendError::Validation(
                "newRoot must be different from oldRoot".to_string(),
            ));
        }
        let new_root_path = PathBuf::from(&new_root);
        if !new_root_path.exists() || !new_root_path.is_dir() {
            return Err(BackendError::Validation(format!(
                "newRoot does not exist or is not a directory: {new_root}"
            )));
        }

        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        let tracks = {
            let mut stmt = tx.prepare("SELECT id, file_path FROM tracks")?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };

        let mut matched = 0usize;
        let mut updated = 0usize;
        let mut unchanged = 0usize;
        let mut missing_at_new_root = 0usize;
        let mut conflicts = 0usize;
        let timestamp = now();

        for (track_id, old_file_path) in tracks {
            let Some(relative_path) = relative_path_under_source_root(&old_file_path, &old_root)
            else {
                continue;
            };
            matched += 1;

            let new_file_path = relocated_source_path(&new_root_path, &relative_path);
            let new_file_path_string = new_file_path.to_string_lossy().to_string();
            if new_file_path_string == old_file_path {
                unchanged += 1;
                continue;
            }
            if !new_file_path.is_file() {
                missing_at_new_root += 1;
                continue;
            }

            let existing_id = tx
                .query_row(
                    "SELECT id FROM tracks WHERE file_path = ?1 LIMIT 1",
                    params![&new_file_path_string],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if existing_id
                .as_deref()
                .is_some_and(|existing| existing != track_id)
            {
                conflicts += 1;
                continue;
            }

            tx.execute(
                "UPDATE tracks SET file_path = ?1, updated_at = ?2 WHERE id = ?3",
                params![&new_file_path_string, &timestamp, &track_id],
            )?;
            updated += 1;
        }

        replace_relocated_source_root_settings(&tx, &old_root, &new_root)?;
        tx.commit()?;

        Ok(RelocateSourceRootData {
            old_root,
            new_root,
            matched,
            updated,
            unchanged,
            missing_at_new_root,
            conflicts,
        })
    }

    pub fn materialize_source_track(
        &self,
        req: MaterializeSourceTrackRequest,
    ) -> BackendResult<MaterializeSourceTrackData> {
        let file_path = req.file_path.trim();
        if file_path.is_empty() {
            return Err(BackendError::Validation(
                "filePath must not be empty".to_string(),
            ));
        }
        let path = std::path::Path::new(file_path);
        if !path.is_file() {
            return Err(BackendError::NotFound(format!(
                "source file does not exist: {file_path}"
            )));
        }

        let metadata = std::fs::metadata(path)?;
        let file_modified_at = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|dur| dur.as_secs().to_string());
        let title = req.title.trim();
        let artist = req.artist.trim();
        let album = req.album.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
        let format_ext = req
            .format_ext
            .and_then(|value| {
                let trimmed = value.trim().to_string();
                (!trimmed.is_empty()).then_some(trimmed)
            })
            .or_else(|| crate::utils::format_ext_from_path(file_path));
        let key = req.key.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
        let fingerprint = build_track_match_fingerprint(title, artist, album.as_deref());
        let file_size_bytes = req
            .file_size_bytes
            .or_else(|| i64::try_from(metadata.len()).ok());
        let now = now();

        let conn = self.db.connect()?;
        let mut stmt = conn.prepare("SELECT id FROM tracks WHERE file_path = ?1 LIMIT 1")?;
        let existing_id = stmt
            .query_row(params![file_path], |row| row.get::<_, String>(0))
            .optional()?;

        let track_id = if let Some(id) = existing_id {
            conn.execute(
                r#"
                UPDATE tracks
                SET title = ?1,
                    artist = ?2,
                    album = ?3,
                    track_number = ?4,
                    tonality = COALESCE(?5, tonality),
                    file_size_bytes = ?6,
                    file_modified_at = ?7,
                    format_ext = ?8,
                    sample_rate_hz = ?9,
                    bit_depth = ?10,
                    bitrate_kbps = ?11,
                    match_fingerprint = ?12,
                    updated_at = ?13
                WHERE id = ?14
                "#,
                params![
                    title,
                    artist,
                    album,
                    req.track_number,
                    key,
                    file_size_bytes,
                    file_modified_at,
                    format_ext,
                    req.sample_rate_hz,
                    req.bit_depth,
                    req.bitrate_kbps,
                    fingerprint,
                    now,
                    id
                ],
            )?;
            id
        } else {
            let id = Uuid::now_v7().to_string();
            conn.execute(
                r#"
                INSERT INTO tracks (
                  id, title, artist, album, track_number, bpm, tonality, file_path,
                  file_size_bytes, file_modified_at, format_ext, sample_rate_hz, bit_depth, bitrate_kbps,
                  disc_number, subtitle, comment, isrc, release_year, release_date, recorded_date,
                  duration_ms, artwork_path, waveform_peaks_path, match_fingerprint,
                  created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, ?14, ?15, ?15)
                "#,
                params![
                    id,
                    title,
                    artist,
                    album,
                    req.track_number,
                    key,
                    file_path,
                    file_size_bytes,
                    file_modified_at,
                    format_ext,
                    req.sample_rate_hz,
                    req.bit_depth,
                    req.bitrate_kbps,
                    fingerprint,
                    now
                ],
            )?;
            id
        };

        Ok(MaterializeSourceTrackData { track_id })
    }

    pub fn resolve_track_identity(
        &self,
        req: ResolveTrackIdentityRequest,
    ) -> BackendResult<ResolveTrackIdentityData> {
        let file_path = req.file_path.as_deref().unwrap_or("").trim();
        let usb_root = req.usb_root.as_deref().unwrap_or("").trim();
        let usb_analysis_path = req.usb_analysis_path.as_deref().unwrap_or("").trim();
        let path_is_selected_usb = !file_path.is_empty()
            && !usb_root.is_empty()
            && browse_path_matches_root(file_path, usb_root);
        let path_has_usb_marker = !usb_analysis_path.is_empty() || path_is_selected_usb;

        let resolve_request = || ResolvePlaybackSourceRequest {
            title: req.title.clone(),
            artist: req.artist.clone(),
            album: req.album.clone(),
            bpm: req.bpm,
            file_path: req.file_path.clone(),
            file_size_bytes: req.file_size_bytes,
            track_id: req.track_id.clone(),
        };

        let resolved = self.resolve_playback_source(resolve_request())?;
        if resolved.matched_by == "self" {
            return Ok(ResolveTrackIdentityData {
                track_id: resolved.track_id,
                resolved_by: resolved.matched_by,
                materialized: false,
            });
        }

        if !file_path.is_empty()
            && !path_has_usb_marker
            && let Ok(data) = self.materialize_source_track(MaterializeSourceTrackRequest {
                file_path: file_path.to_string(),
                title: req.title.clone(),
                artist: req.artist.clone(),
                album: req.album.clone(),
                track_number: req.track_number,
                key: req.key.clone(),
                file_size_bytes: req.file_size_bytes,
                format_ext: req.format_ext.clone(),
                sample_rate_hz: req.sample_rate_hz,
                bit_depth: req.bit_depth,
                bitrate_kbps: req.bitrate_kbps,
            })
        {
            return Ok(ResolveTrackIdentityData {
                track_id: Some(data.track_id),
                resolved_by: "materialized".to_string(),
                materialized: true,
            });
        }

        Ok(ResolveTrackIdentityData {
            track_id: resolved.track_id,
            resolved_by: resolved.matched_by,
            materialized: false,
        })
    }

    pub fn remove_tracks_by_source_roots(
        &self,
        req: RemoveTracksBySourceRootsRequest,
    ) -> BackendResult<RemoveTracksBySourceRootsData> {
        let mut roots = req
            .source_roots
            .into_iter()
            .map(|root| root.trim().to_string())
            .filter(|root| !root.is_empty())
            .collect::<Vec<_>>();
        roots.sort();
        roots.dedup();
        if roots.is_empty() {
            return Ok(RemoveTracksBySourceRootsData { removed: 0 });
        }

        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        let mut removed = 0usize;
        for root in roots {
            let escaped = root
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let slash_like = format!("{escaped}/%");
            let backslash_like = format!("{escaped}\\\\%");
            let deleted = tx.execute(
                r#"
                DELETE FROM tracks
                WHERE file_path = ?1
                   OR file_path LIKE ?2 ESCAPE '\'
                   OR file_path LIKE ?3 ESCAPE '\'
                "#,
                params![root, slash_like, backslash_like],
            )?;
            removed += deleted;
        }
        tx.commit()?;
        Ok(RemoveTracksBySourceRootsData { removed })
    }

    pub fn get_tracks_by_ids_with_previews(
        &self,
        req: GetTracksByIdsRequest,
    ) -> BackendResult<GetTracksByIdsData> {
        let mut ids = req
            .track_ids
            .into_iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return Ok(GetTracksByIdsData { items: Vec::new() });
        }

        let conn = self.db.connect()?;
        let placeholders = vec!["?"; ids.len()].join(", ");
        let sql = format!("SELECT {TRACK_COLS} FROM tracks WHERE id IN ({placeholders})");

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(ids.iter()), |row| row_to_track(row, true))?;
        let mut found = rows.collect::<Result<Vec<_>, _>>()?;
        found.sort_by(|a, b| a.id.cmp(&b.id));
        apply_is_usb_path(&conn, &mut found)?;

        Ok(GetTracksByIdsData { items: found })
    }

    pub fn resolve_playback_source(
        &self,
        req: ResolvePlaybackSourceRequest,
    ) -> BackendResult<ResolvePlaybackSourceData> {
        let conn = self.db.connect()?;
        let usb_root_paths = untainted_usb_root_paths(&conn)?;
        let is_usb_rooted = |path: &str| {
            usb_root_paths
                .iter()
                .any(|root| browse_path_matches_root(path, root))
        };

        // Fast path: any origin (library, playlist, USB, history) may carry
        // the id of the row it was dispatched from. If that row is a
        // genuine local track, resolve to it directly with a single indexed
        // lookup -- no fingerprint scan needed. If it turns out to be
        // USB-rooted (e.g. a playlist entry created before this fix, still
        // pointing at a stale placeholder), fall through to the
        // fingerprint/title search below so it can self-heal.
        if let Some(id) = req.track_id.as_deref().filter(|id| !id.trim().is_empty()) {
            let mut stmt =
                conn.prepare(&format!("SELECT {TRACK_COLS} FROM tracks WHERE id = ?1"))?;
            let track = stmt
                .query_row(params![id], |row| row_to_track(row, false))
                .optional()?;
            if let Some(track) = track
                && !is_usb_rooted(&track.file_path)
            {
                return Ok(ResolvePlaybackSourceData {
                    resolved_path: Some(track.file_path.clone()),
                    matched_by: "self".to_string(),
                    track_id: Some(track.id),
                });
            }
        }

        let title = req.title.trim();
        let artist = req.artist.trim();
        if title.is_empty() || artist.is_empty() {
            return Ok(ResolvePlaybackSourceData {
                resolved_path: None,
                matched_by: "none".to_string(),
                track_id: None,
            });
        }

        let fingerprint = build_track_match_fingerprint(title, artist, req.album.as_deref());
        let mut stmt = conn.prepare(
            &format!("SELECT {TRACK_COLS} FROM tracks WHERE match_fingerprint = ?1 ORDER BY updated_at DESC LIMIT 64"),
        )?;
        let rows = stmt.query_map(params![fingerprint], |row| row_to_track(row, false))?;
        let candidates = rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|t| !is_usb_rooted(&t.file_path))
            .collect::<Vec<_>>();

        if let Some(track) = best_candidate(candidates, &req) {
            return Ok(ResolvePlaybackSourceData {
                resolved_path: Some(track.file_path.clone()),
                matched_by: "hash".to_string(),
                track_id: Some(track.id),
            });
        }

        let like = format!("%{}%", title);
        let mut stmt = conn.prepare(&format!(
            "SELECT {TRACK_COLS} FROM tracks WHERE title LIKE ?1 ORDER BY updated_at DESC LIMIT 200"
        ))?;
        let rows = stmt.query_map(params![like], |row| row_to_track(row, false))?;
        let candidates = rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|t| !is_usb_rooted(&t.file_path))
            .collect::<Vec<_>>();
        if let Some(track) = best_candidate(candidates, &req) {
            return Ok(ResolvePlaybackSourceData {
                resolved_path: Some(track.file_path.clone()),
                matched_by: "metadata".to_string(),
                track_id: Some(track.id),
            });
        }

        Ok(ResolvePlaybackSourceData {
            resolved_path: None,
            matched_by: "none".to_string(),
            track_id: None,
        })
    }

    pub fn create_playlist(&self, req: CreatePlaylistRequest) -> BackendResult<CreatePlaylistData> {
        let name = req.name.trim();
        if name.is_empty() {
            return Err(BackendError::Validation(
                "playlist name must not be empty".to_string(),
            ));
        }

        let id = Uuid::now_v7().to_string();
        let now = now();
        let conn = self.db.connect()?;
        conn.execute(
            "INSERT INTO playlists (id, name, source, last_exported_at, last_exported_usb_root, last_exported_track_count, created_at, updated_at) VALUES (?1, ?2, 'local', NULL, NULL, NULL, ?3, ?3)",
            params![id, name, now],
        )?;

        Ok(CreatePlaylistData {
            playlist_id: id,
            name: name.to_string(),
        })
    }

    pub fn rename_playlist(&self, req: RenamePlaylistRequest) -> BackendResult<RenamePlaylistData> {
        let name = req.name.trim();
        if name.is_empty() {
            return Err(BackendError::Validation(
                "playlist name must not be empty".to_string(),
            ));
        }

        ensure_playlist_exists(&self.db, &req.playlist_id)?;

        let now = now();
        let conn = self.db.connect()?;
        conn.execute(
            "UPDATE playlists SET name = ?1, last_exported_at = NULL, last_exported_usb_root = NULL, last_exported_track_count = NULL, updated_at = ?2 WHERE id = ?3",
            params![name, now, req.playlist_id],
        )?;

        Ok(RenamePlaylistData {
            playlist_id: req.playlist_id,
            name: name.to_string(),
        })
    }

    pub fn delete_playlist(&self, req: DeletePlaylistRequest) -> BackendResult<DeletePlaylistData> {
        ensure_playlist_exists(&self.db, &req.playlist_id)?;

        let conn = self.db.connect()?;
        let deleted = conn.execute(
            "DELETE FROM playlists WHERE id = ?1",
            params![req.playlist_id.clone()],
        )?;

        Ok(DeletePlaylistData {
            playlist_id: req.playlist_id,
            deleted: deleted > 0,
        })
    }

    pub fn list_playlists(&self) -> BackendResult<ListPlaylistsData> {
        let conn = self.db.connect()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, name, source, last_exported_at, last_exported_usb_root, last_exported_track_count, created_at, updated_at
            FROM playlists
            ORDER BY created_at ASC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(Playlist {
                id: row.get(0)?,
                name: row.get(1)?,
                source: row.get(2)?,
                last_exported_at: row.get(3)?,
                last_exported_usb_root: row.get(4)?,
                last_exported_track_count: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?;

        let items = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(ListPlaylistsData { items })
    }

    pub fn get_playlist_tracks(
        &self,
        req: GetPlaylistTracksRequest,
    ) -> BackendResult<GetPlaylistTracksData> {
        ensure_playlist_exists(&self.db, &req.playlist_id)?;

        let conn = self.db.connect()?;
        // Column order MUST stay in lockstep with `TRACK_COLS` / `row_to_track`
        // (which reads `master_db_source` positionally at index 19) -- this JOIN
        // needs `t.`-prefixed names so it can't reuse `TRACK_COLS` verbatim.
        let mut stmt = conn.prepare(
            r#"
            SELECT t.id, t.title, t.artist, t.album, t.track_number, t.bpm, t.tonality, t.file_path,
                   t.file_size_bytes, t.format_ext, t.sample_rate_hz, t.bit_depth, t.bitrate_kbps, t.duration_ms,
                   t.artwork_path, t.waveform_peaks_path, t.bpm_analyzer, t.created_at, t.updated_at,
                   COALESCE(t.master_db_source, 0) AS master_db_source, t.wav_extensible_kind
            FROM playlist_tracks pt
            JOIN tracks t ON t.id = pt.track_id
            WHERE pt.playlist_id = ?1
            ORDER BY pt.position ASC
            "#,
        )?;

        let rows = stmt.query_map(params![req.playlist_id], |row| row_to_track(row, true))?;
        let mut items = rows.collect::<Result<Vec<_>, _>>()?;
        apply_is_usb_path(&conn, &mut items)?;

        // Computed once server-side (here) instead of by the frontend summing
        // whatever tracks it happens to have loaded -- see
        // `GetPlaylistTracksData::total_duration_ms`.
        let mut total_duration_ms: u64 = 0;
        let mut duration_known_count: usize = 0;
        for track in &items {
            if let Some(d) = track.duration_ms
                && d > 0
            {
                total_duration_ms += d;
                duration_known_count += 1;
            }
        }

        Ok(GetPlaylistTracksData {
            playlist_id: req.playlist_id,
            items,
            total_duration_ms,
            duration_known_count,
        })
    }

    pub fn add_tracks_to_playlist(
        &self,
        req: AddTracksToPlaylistRequest,
    ) -> BackendResult<AddTracksToPlaylistData> {
        if req.track_ids.is_empty() {
            return Err(BackendError::Validation(
                "trackIds must contain at least one id".to_string(),
            ));
        }

        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        ensure_playlist_exists_conn(&tx, &req.playlist_id)?;

        let mut next_position: i64 = tx.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM playlist_tracks WHERE playlist_id = ?1",
            params![req.playlist_id],
            |row| row.get(0),
        )?;

        let mut added = 0usize;
        let mut skipped = 0usize;
        let now = now();

        // Pre-load existing track IDs to avoid N+1 queries
        let mut existing_track_ids = std::collections::HashSet::new();
        {
            let mut stmt = tx.prepare("SELECT id FROM tracks")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                existing_track_ids.insert(row?);
            }
        }

        // Pre-load playlist track IDs for dedupe check
        let mut playlist_track_ids = std::collections::HashSet::new();
        if matches!(req.dedupe, DedupeMode::Skip) {
            let mut stmt =
                tx.prepare("SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1")?;
            let rows = stmt.query_map(params![req.playlist_id], |row| row.get::<_, String>(0))?;
            for row in rows {
                playlist_track_ids.insert(row?);
            }
        }

        for track_id in &req.track_ids {
            if !existing_track_ids.contains(track_id.as_str()) {
                return Err(BackendError::NotFound(format!(
                    "track not found: {track_id}"
                )));
            }

            if matches!(req.dedupe, DedupeMode::Skip)
                && playlist_track_ids.contains(track_id.as_str())
            {
                skipped += 1;
                continue;
            }

            tx.execute(
                r#"
                INSERT INTO playlist_tracks (id, playlist_id, track_id, position, added_at)
                VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                params![
                    Uuid::now_v7().to_string(),
                    req.playlist_id,
                    track_id,
                    next_position,
                    now
                ],
            )?;

            if matches!(req.dedupe, DedupeMode::Skip) {
                playlist_track_ids.insert(track_id.clone());
            }

            added += 1;
            next_position += 1;
        }

        tx.execute(
            "UPDATE playlists SET updated_at = ?1, last_exported_at = NULL, last_exported_usb_root = NULL, last_exported_track_count = NULL WHERE id = ?2",
            params![now, req.playlist_id],
        )?;

        tx.commit()?;

        Ok(AddTracksToPlaylistData {
            playlist_id: req.playlist_id,
            added,
            skipped,
        })
    }

    pub fn add_track_candidates_to_playlist(
        &self,
        req: AddTrackCandidatesToPlaylistRequest,
    ) -> BackendResult<AddTrackCandidatesToPlaylistData> {
        let conn = self.db.connect()?;
        let requested = req.tracks.len();
        let mut track_ids = Vec::<String>::new();
        let mut resolutions = Vec::<AddTrackCandidateResolution>::with_capacity(requested);

        for candidate in &req.tracks {
            let resolution = self.resolve_add_track_candidate(
                &conn,
                candidate,
                req.usb_root.as_deref(),
                req.usb_root_valid,
            )?;
            if let Some(track_id) = resolution.track_id.as_ref() {
                track_ids.push(track_id.clone());
            }
            resolutions.push(resolution);
        }
        drop(conn);

        let resolved = track_ids.len();
        let unresolved = requested.saturating_sub(resolved);
        if track_ids.is_empty() {
            return Ok(AddTrackCandidatesToPlaylistData {
                playlist_id: req.playlist_id,
                requested,
                resolved,
                unresolved,
                added: 0,
                skipped: 0,
                resolutions,
            });
        }

        let added = self.add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: req.playlist_id,
            track_ids,
            dedupe: req.dedupe,
        })?;

        Ok(AddTrackCandidatesToPlaylistData {
            playlist_id: added.playlist_id,
            requested,
            resolved,
            unresolved,
            added: added.added,
            skipped: added.skipped,
            resolutions,
        })
    }

    fn resolve_add_track_candidate(
        &self,
        conn: &rusqlite::Connection,
        candidate: &AddTrackCandidate,
        request_usb_root: Option<&str>,
        request_usb_root_valid: bool,
    ) -> BackendResult<AddTrackCandidateResolution> {
        let previous_id = trimmed_string(candidate.track_id.as_deref());
        if let Some(local_track_id) = trimmed_string(candidate.local_track_id.as_deref()) {
            return Ok(AddTrackCandidateResolution {
                previous_id,
                track_id: Some(local_track_id),
                resolved_by: "localTrackId".to_string(),
                materialized: false,
            });
        }

        if let Some(track_id) = previous_id.as_deref()
            && track_id_exists(conn, track_id)?
        {
            return Ok(AddTrackCandidateResolution {
                previous_id: previous_id.clone(),
                track_id: Some(track_id.to_string()),
                resolved_by: "self".to_string(),
                materialized: false,
            });
        }

        if add_candidate_is_usb_origin(candidate, request_usb_root) {
            return Ok(AddTrackCandidateResolution {
                previous_id,
                track_id: None,
                resolved_by: "usbOrigin".to_string(),
                materialized: false,
            });
        }

        let identity = self.resolve_track_identity(ResolveTrackIdentityRequest {
            track_id: previous_id.clone(),
            title: candidate.title.clone(),
            artist: candidate.artist.clone(),
            album: candidate.album.clone(),
            bpm: candidate.bpm,
            file_path: candidate.file_path.clone(),
            file_size_bytes: candidate.file_size_bytes,
            track_number: candidate.track_number,
            key: candidate.key.clone(),
            format_ext: candidate.format_ext.clone(),
            sample_rate_hz: candidate.sample_rate_hz,
            bit_depth: candidate.bit_depth,
            bitrate_kbps: candidate.bitrate_kbps,
            usb_root: candidate
                .usb_root
                .clone()
                .or_else(|| trimmed_string(request_usb_root)),
            usb_root_valid: candidate.usb_root_valid || request_usb_root_valid,
            usb_analysis_path: candidate.usb_analysis_path.clone(),
        })?;

        Ok(AddTrackCandidateResolution {
            previous_id,
            track_id: identity.track_id,
            resolved_by: identity.resolved_by,
            materialized: identity.materialized,
        })
    }

    pub fn remove_tracks_from_playlist(
        &self,
        req: RemoveTracksFromPlaylistRequest,
    ) -> BackendResult<RemoveTracksFromPlaylistData> {
        if req.track_ids.is_empty() {
            return Err(BackendError::Validation(
                "trackIds must contain at least one id".to_string(),
            ));
        }

        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        ensure_playlist_exists_conn(&tx, &req.playlist_id)?;

        let mut removed = 0usize;
        let mut uniq_track_ids = req.track_ids.clone();
        uniq_track_ids.sort();
        uniq_track_ids.dedup();

        for track_id in &uniq_track_ids {
            removed += tx.execute(
                "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
                params![req.playlist_id, track_id],
            )?;
        }

        let mut row_ids = Vec::<String>::new();
        {
            let mut stmt = tx.prepare(
                "SELECT id FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position ASC, id ASC",
            )?;
            let rows = stmt.query_map(params![req.playlist_id], |row| row.get::<_, String>(0))?;
            for row in rows {
                row_ids.push(row?);
            }
        }
        for (idx, row_id) in row_ids.iter().enumerate() {
            tx.execute(
                "UPDATE playlist_tracks SET position = ?1 WHERE id = ?2",
                params![(idx as i64) + 1, row_id],
            )?;
        }

        tx.execute(
            "UPDATE playlists SET updated_at = ?1, last_exported_at = NULL, last_exported_usb_root = NULL, last_exported_track_count = NULL WHERE id = ?2",
            params![now(), req.playlist_id],
        )?;

        tx.commit()?;
        Ok(RemoveTracksFromPlaylistData {
            playlist_id: req.playlist_id,
            removed,
        })
    }

    pub fn reorder_playlist_tracks(
        &self,
        req: ReorderPlaylistTracksRequest,
    ) -> BackendResult<ReorderPlaylistTracksData> {
        if req.ordered_track_ids.is_empty() {
            return Err(BackendError::Validation(
                "orderedTrackIds must contain at least one id".to_string(),
            ));
        }

        let mut uniq_ordered_ids = req.ordered_track_ids.clone();
        uniq_ordered_ids.sort();
        uniq_ordered_ids.dedup();
        if uniq_ordered_ids.len() != req.ordered_track_ids.len() {
            return Err(BackendError::Validation(
                "orderedTrackIds must not contain duplicates".to_string(),
            ));
        }

        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        ensure_playlist_exists_conn(&tx, &req.playlist_id)?;

        let mut row_by_track_id = std::collections::HashMap::<String, String>::new();
        {
            let mut stmt =
                tx.prepare("SELECT id, track_id FROM playlist_tracks WHERE playlist_id = ?1")?;
            let rows = stmt.query_map(params![req.playlist_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (id, track_id) = row?;
                row_by_track_id.insert(track_id, id);
            }
        }

        let existing_track_ids: std::collections::HashSet<&String> =
            row_by_track_id.keys().collect();
        let requested_track_ids: std::collections::HashSet<&String> =
            req.ordered_track_ids.iter().collect();
        if existing_track_ids != requested_track_ids {
            return Err(BackendError::Validation(
                "orderedTrackIds must match the playlist's current track set".to_string(),
            ));
        }

        // Two-phase update: `playlist_tracks` has UNIQUE(playlist_id, position),
        // so writing final positions directly can collide mid-transaction with
        // whatever a row currently holds. Shift every row to a disjoint negative
        // range first, then assign the real 1..N positions in a second pass.
        for (idx, track_id) in req.ordered_track_ids.iter().enumerate() {
            let row_id = &row_by_track_id[track_id];
            tx.execute(
                "UPDATE playlist_tracks SET position = ?1 WHERE id = ?2",
                params![-((idx as i64) + 1), row_id],
            )?;
        }
        for (idx, track_id) in req.ordered_track_ids.iter().enumerate() {
            let row_id = &row_by_track_id[track_id];
            tx.execute(
                "UPDATE playlist_tracks SET position = ?1 WHERE id = ?2",
                params![(idx as i64) + 1, row_id],
            )?;
        }

        tx.execute(
            "UPDATE playlists SET updated_at = ?1, last_exported_at = NULL, last_exported_usb_root = NULL, last_exported_track_count = NULL WHERE id = ?2",
            params![now(), req.playlist_id],
        )?;

        tx.commit()?;
        Ok(ReorderPlaylistTracksData {
            playlist_id: req.playlist_id,
            reordered: req.ordered_track_ids.len(),
        })
    }

    pub fn get_frontend_settings(&self) -> BackendResult<GetFrontendSettingsData> {
        let conn = self.db.connect()?;
        let mut values = std::collections::HashMap::<String, String>::new();
        let keys = frontend_ui_setting_keys();
        for key in keys {
            if let Some(value) = conn
                .query_row(
                    "SELECT value FROM app_settings WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
            {
                values.insert(key.to_string(), value);
            }
        }
        let node_available = check_node_available();
        let essentia_installed = check_essentia_installed(&self.db.data_dir());
        Ok(GetFrontendSettingsData {
            values,
            node_available,
            essentia_installed,
        })
    }

    pub fn remove_essentia(&self) -> BackendResult<()> {
        let essentia_dir = self.db.data_dir().join("essentia");
        if essentia_dir.exists() {
            std::fs::remove_dir_all(&essentia_dir).map_err(|e| {
                BackendError::Internal(format!("failed to remove essentia dir: {e}"))
            })?;
        }
        let conn = self.db.connect()?;
        conn.execute(
            "DELETE FROM app_settings WHERE key = ?1",
            params![SETTING_UI_ANALYSIS_ENGINE],
        )?;
        Ok(())
    }

    pub fn set_frontend_setting(
        &self,
        req: SetFrontendSettingRequest,
    ) -> BackendResult<SetFrontendSettingData> {
        let key = req.key.trim();
        if !frontend_ui_setting_keys().contains(&key) {
            return Err(BackendError::Validation(format!(
                "unsupported frontend setting key: {key}"
            )));
        }
        let conn = self.db.connect()?;
        if let Some(raw_value) = req.value {
            let value = raw_value.trim().to_string();
            if value.len() > 262_144 {
                return Err(BackendError::Validation(
                    "frontend setting value too large".to_string(),
                ));
            }
            conn.execute(
                r#"
                INSERT INTO app_settings (key, value, updated_at)
                VALUES (?1, ?2, ?3)
                ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
                "#,
                params![key, value, now()],
            )?;
        } else {
            conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])?;
        }
        Ok(SetFrontendSettingData { saved: true })
    }
}

fn frontend_ui_setting_keys() -> &'static [&'static str] {
    &[
        SETTING_UI_THEME,
        SETTING_UI_ACCENT_HUE,
        SETTING_UI_SOURCE_ROOTS,
        SETTING_UI_SOURCE_ROOT_ENABLED,
        SETTING_UI_USB_ROOT,
        SETTING_UI_EXPORT_PRUNE_STALE,
        SETTING_UI_EXPORT_BACKUP,
        SETTING_UI_BACKUP_RETENTION_COUNT,
        SETTING_UI_ANALYSIS_BPM_RANGE,
        SETTING_UI_ANALYSIS_ENGINE,
        SETTING_UI_SIDEBAR_COLLAPSED,
        SETTING_UI_HELP_SEEN,
    ]
}

/// The user's currently-configured local library folders, read directly from
/// `app_settings` (the same JSON array `SETTING_UI_SOURCE_ROOTS` the frontend
/// persists). An explicit "this is my library folder" assertion from the user
/// outranks stale USB-device history for that same path (see
/// `usb_utils::all_usb_device_root_paths`). Tolerates a missing key (fresh
/// install) or malformed JSON by returning an empty list rather than failing.
pub(crate) fn configured_source_roots(conn: &rusqlite::Connection) -> BackendResult<Vec<String>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![SETTING_UI_SOURCE_ROOTS],
            |row| row.get(0),
        )
        .optional()?;
    Ok(raw
        .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default())
}

/// The user's currently-configured export sync mode, read directly from
/// `app_settings` (the same `"1"`/`"0"` string the frontend mirrors into
/// `localStorage` under `SETTING_UI_EXPORT_PRUNE_STALE`). Defaults to `true`
/// (mirror/prune-stale) when unset, matching the frontend's own default.
pub(crate) fn export_prune_stale_setting(conn: &rusqlite::Connection) -> BackendResult<bool> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![SETTING_UI_EXPORT_PRUNE_STALE],
            |row| row.get(0),
        )
        .optional()?;
    Ok(raw.map(|value| value == "1").unwrap_or(true))
}

/// All known USB device root paths, minus any that are also currently
/// configured as a local `sourceRoots` folder. `all_usb_device_root_paths`
/// intentionally never forgets a path once it's been seen as a USB root
/// (see its doc comment) -- an explicit "this is my library folder"
/// assertion from the user is what overrides that stale taint for reused
/// mount paths (e.g. `/mnt/data` remounted as a permanent local folder),
/// not the passage of time or a prune action.
pub(crate) fn untainted_usb_root_paths(conn: &rusqlite::Connection) -> BackendResult<Vec<String>> {
    let source_roots = configured_source_roots(conn)?;
    let all_roots = usb_utils::all_usb_device_root_paths(conn)?;
    Ok(all_roots
        .into_iter()
        .filter(|root| {
            !source_roots.iter().any(|sr| {
                normalize_source_root_for_matching(sr) == normalize_source_root_for_matching(root)
            })
        })
        .collect())
}

/// Sets `is_usb_path` on every track, one query for the whole batch (not
/// per-row). Mirrors `resolve_playback_source`'s `is_usb_rooted` check
/// exactly: matched against `file_path` only, against every known USB
/// device root (including pruned ones -- see `untainted_usb_root_paths`).
/// Call this from any method that returns `Track` rows to the frontend.
pub(crate) fn apply_is_usb_path(
    conn: &rusqlite::Connection,
    tracks: &mut [Track],
) -> BackendResult<()> {
    let usb_root_paths = untainted_usb_root_paths(conn)?;
    for track in tracks.iter_mut() {
        track.is_usb_path = usb_root_paths
            .iter()
            .any(|root| browse_path_matches_root(&track.file_path, root));
    }
    Ok(())
}

const FINGERPRINT_MATCH_DURATION_TOLERANCE_MS: i64 = 2000;

/// Finds a single high-confidence local-track match for a USB-sourced
/// fingerprint. Duration (tolerance-based) and file size (exact) must both
/// agree whenever both sides have the data -- fingerprinting alone is
/// text-only (lowercased title/artist/album) and can't distinguish
/// same-length variants like a Clean vs. Explicit edit, so both gates are
/// required together, each only when its data is actually available.
/// Returns `None` (no match) whenever zero or more than one candidate
/// survives -- a spurious extra placeholder row is a strictly safer failure
/// mode than silently linking the wrong audio file. Candidates whose own
/// `file_path` falls under a known (untainted) USB root are skipped
/// entirely: a placeholder row matching another placeholder is never a
/// genuine match.
pub(crate) fn find_confident_fingerprint_match(
    conn: &rusqlite::Connection,
    fingerprint: &str,
    incoming_duration_ms: Option<i64>,
    incoming_file_size_bytes: Option<i64>,
    usb_root_paths: &[String],
) -> BackendResult<Option<String>> {
    if fingerprint.trim().is_empty() {
        return Ok(None);
    }
    let mut stmt = conn.prepare(
        "SELECT id, file_path, duration_ms, file_size_bytes FROM tracks WHERE match_fingerprint = ?1",
    )?;
    let rows: Vec<(String, String, Option<i64>, Option<i64>)> = stmt
        .query_map(params![fingerprint], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<_, _>>()?;

    let mut passing = Vec::new();
    for (id, path, cand_duration, cand_size) in rows {
        if usb_root_paths
            .iter()
            .any(|root| browse_path_matches_root(&path, root))
        {
            continue;
        }
        let duration_ok = match (incoming_duration_ms, cand_duration) {
            (Some(a), Some(b)) => (a - b).abs() <= FINGERPRINT_MATCH_DURATION_TOLERANCE_MS,
            _ => true,
        };
        let size_ok = match (incoming_file_size_bytes, cand_size) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        if duration_ok && size_ok {
            passing.push(id);
        }
    }

    if passing.len() == 1 {
        Ok(passing.into_iter().next())
    } else {
        Ok(None)
    }
}

fn source_root_keys_equal(left: &str, right: &str) -> bool {
    normalize_source_root_for_matching(left) == normalize_source_root_for_matching(right)
}

fn write_app_setting_tx(
    tx: &rusqlite::Transaction<'_>,
    key: &str,
    value: String,
) -> BackendResult<()> {
    tx.execute(
        r#"
        INSERT INTO app_settings (key, value, updated_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
        "#,
        params![key, value, now()],
    )?;
    Ok(())
}

fn replace_relocated_source_root_settings(
    tx: &rusqlite::Transaction<'_>,
    old_root: &str,
    new_root: &str,
) -> BackendResult<()> {
    if let Some(raw_roots) = tx
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![SETTING_UI_SOURCE_ROOTS],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        && let Ok(roots) = serde_json::from_str::<Vec<String>>(&raw_roots)
    {
        let mut changed = false;
        let mut next_roots = Vec::<String>::new();
        for root in roots {
            let replacement = if source_root_keys_equal(&root, old_root) {
                changed = true;
                new_root.to_string()
            } else {
                root
            };
            if next_roots
                .iter()
                .any(|existing| source_root_keys_equal(existing, &replacement))
            {
                changed = true;
            } else {
                next_roots.push(replacement);
            }
        }
        if changed {
            let encoded = serde_json::to_string(&next_roots).map_err(|err| {
                BackendError::Internal(format!("failed to encode source roots setting: {err}"))
            })?;
            write_app_setting_tx(tx, SETTING_UI_SOURCE_ROOTS, encoded)?;
        }
    }

    if let Some(raw_enabled) = tx
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![SETTING_UI_SOURCE_ROOT_ENABLED],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        && let Ok(mut enabled) =
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&raw_enabled)
    {
        let old_keys = enabled
            .keys()
            .filter(|key| source_root_keys_equal(key, old_root))
            .cloned()
            .collect::<Vec<_>>();
        if !old_keys.is_empty() {
            let mut moved_value = None;
            for key in old_keys {
                if moved_value.is_none() {
                    moved_value = enabled.remove(&key);
                } else {
                    enabled.remove(&key);
                }
            }
            if let Some(value) = moved_value {
                let has_new_key = enabled
                    .keys()
                    .any(|key| source_root_keys_equal(key, new_root));
                if !has_new_key {
                    enabled.insert(new_root.to_string(), value);
                }
            }
            let encoded = serde_json::to_string(&enabled).map_err(|err| {
                BackendError::Internal(format!(
                    "failed to encode source root enabled setting: {err}"
                ))
            })?;
            write_app_setting_tx(tx, SETTING_UI_SOURCE_ROOT_ENABLED, encoded)?;
        }
    }

    Ok(())
}

pub fn check_essentia_installed(data_dir: &std::path::Path) -> bool {
    let node_modules = data_dir.join("essentia/node_modules");
    node_modules.join("essentia.js/package.json").is_file()
        && node_modules
            .join("essentia.js/dist/essentia-wasm.umd.js")
            .is_file()
        && node_modules.join("node-wav/package.json").is_file()
}

fn check_node_available() -> bool {
    let node_bin = std::env::var("DJTKIT_ESSENTIA_NODE").unwrap_or_else(|_| "node".to_string());
    std::process::Command::new(&node_bin)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn row_to_track(row: &rusqlite::Row<'_>, include_previews: bool) -> rusqlite::Result<Track> {
    let file_path: String = row.get(7)?;
    let artwork_path: Option<String> = row.get(14)?;
    let waveform_peaks_path: Option<String> = row.get(15)?;
    let bpm_analyzer: Option<String> = row.get(16)?;
    // Load waveform preview (small, ~400 bytes) when requested.
    // Skip artwork base64 data URL for local tracks - the frontend uses
    // artworkPath via Tauri's asset protocol (convertFileSrc) instead.
    // Embedding full images as data URLs in JSON causes IPC/memory crashes
    // when hydrating many tracks at once.
    let is_master_db = row.get::<_, i64>(19).unwrap_or(0) != 0;
    // master.db tracks: try PWV4 color data from .EXT first; fall back to greyscale
    // PWAV/PWV2 from .DAT if extended analysis hasn't been run.
    let (waveform_preview, waveform_color_data) = if include_previews {
        if is_master_db {
            let color = waveform_peaks_path.as_deref().and_then(read_pwv4_from_anlz);
            if color.is_some() {
                (None, color)
            } else {
                let preview = waveform_peaks_path
                    .as_deref()
                    .and_then(load_waveform_preview_from_analysis_path);
                (preview, None)
            }
        } else {
            let preview = waveform_peaks_path
                .as_deref()
                .and_then(load_waveform_preview_from_analysis_path);
            (preview, None)
        }
    } else {
        (None, None)
    };
    let artwork_data_url: Option<String> = None;

    let bpm: Option<f64> = row.get(5)?;
    let duration_ms: Option<u64> = row.get(13)?;
    // Pure DB-column math, correct for every caller (no separate pass).
    let analysis_ready =
        has_core_analysis_fields(waveform_peaks_path.as_deref(), bpm, duration_ms);

    Ok(Track {
        id: row.get(0)?,
        title: row.get(1)?,
        artist: row.get(2)?,
        album: row.get(3)?,
        track_number: row.get(4)?,
        bpm,
        bpm_analyzer,
        key: row.get(6)?,
        file_size_bytes: row.get(8)?,
        // Backend-owned: every track-returning command guarantees a format on
        // the wire. Fall back to the file-path extension for legacy NULL rows
        // and import paths that don't set the column (master.db, USB merge).
        format_ext: row
            .get::<_, Option<String>>(9)?
            .or_else(|| crate::utils::format_ext_from_path(&file_path)),
        file_path,
        sample_rate_hz: row.get(10)?,
        bit_depth: row.get(11)?,
        bitrate_kbps: row.get(12)?,
        // Looked up by name (not position) since callers select the tracks
        // columns with varying shapes; some queries omit this column.
        wav_extensible_kind: row.get("wav_extensible_kind").unwrap_or(None),
        duration_ms,
        artwork_path,
        artwork_data_url,
        waveform_peaks_path,
        waveform_preview,
        waveform_color_data,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
        master_db_source: is_master_db,
        // Filled in by callers that expose Track to the frontend (see
        // apply_is_usb_path); internal-only callers leave this false.
        is_usb_path: false,
        analysis_ready,
    })
}

pub fn build_track_match_fingerprint(title: &str, artist: &str, album: Option<&str>) -> String {
    let normalized = format!(
        "{}|{}|{}",
        normalize_hash_part(title),
        normalize_hash_part(artist),
        normalize_hash_part(album.unwrap_or_default()),
    );
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalized.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn normalize_hash_part(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_whitespace() {
                ' '
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn path_file_name_lower(value: &str) -> String {
    let p = value.replace('\\', "/");
    p.rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn path_stem_lower(value: &str) -> String {
    let file = path_file_name_lower(value);
    if let Some((stem, _)) = file.rsplit_once('.') {
        stem.to_string()
    } else {
        file
    }
}

fn best_candidate(candidates: Vec<Track>, req: &ResolvePlaybackSourceRequest) -> Option<Track> {
    let mut best: Option<Track> = None;
    let mut best_score = -1i32;
    for candidate in candidates {
        // A known file-size mismatch is stronger evidence of "different
        // file" than any positive score contribution elsewhere can
        // outweigh -- veto outright rather than merely downweighting.
        if let (Some(req_size), Some(candidate_size)) =
            (req.file_size_bytes, candidate.file_size_bytes)
            && req_size != candidate_size
        {
            continue;
        }
        let score = score_playback_candidate(&candidate, req);
        if score > best_score {
            best_score = score;
            best = Some(candidate);
        }
    }
    best.filter(|_| best_score >= 24)
}

fn paginate_tracks(
    items: &mut Vec<Track>,
    limit: usize,
    signature: &str,
) -> (bool, Option<String>) {
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next_cursor = items
        .last()
        .and_then(|last| encode_track_page_cursor(signature, &last.file_path, &last.id));
    (has_more, next_cursor)
}

fn score_playback_candidate(track: &Track, req: &ResolvePlaybackSourceRequest) -> i32 {
    let mut score = 0;
    if normalize_hash_part(&track.title) == normalize_hash_part(&req.title) {
        score += 12;
    }
    if normalize_hash_part(&track.artist) == normalize_hash_part(&req.artist) {
        score += 12;
    }
    if normalize_hash_part(track.album.as_deref().unwrap_or_default())
        == normalize_hash_part(req.album.as_deref().unwrap_or_default())
    {
        score += 8;
    }
    if let Some(src_path) = req.file_path.as_deref() {
        if path_file_name_lower(src_path) == path_file_name_lower(&track.file_path) {
            score += 16;
        }
        if path_stem_lower(src_path) == path_stem_lower(&track.file_path) {
            score += 8;
        }
    }
    if let (Some(a), Some(b)) = (req.bpm, track.bpm)
        && (a - b).abs() <= 0.15
    {
        score += 4;
    }
    score
}

fn ensure_playlist_exists_conn(
    conn: &rusqlite::Connection,
    playlist_id: &str,
) -> BackendResult<()> {
    let exists: Option<String> = conn
        .query_row(
            "SELECT id FROM playlists WHERE id = ?1",
            params![playlist_id],
            |row| row.get(0),
        )
        .optional()?;

    if exists.is_none() {
        return Err(BackendError::NotFound(format!(
            "playlist not found: {playlist_id}"
        )));
    }

    Ok(())
}

fn ensure_playlist_exists(db: &Db, playlist_id: &str) -> BackendResult<()> {
    let conn = db.connect()?;
    ensure_playlist_exists_conn(&conn, playlist_id)
}

pub(crate) fn now() -> String {
    Utc::now().to_rfc3339()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrackPageCursor {
    version: String,
    signature: String,
    file_path: String,
    id: String,
}

fn build_track_cursor_signature(parts: &[&str]) -> String {
    let joined = parts.join("|");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    joined.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn encode_track_page_cursor(signature: &str, file_path: &str, id: &str) -> Option<String> {
    if file_path.trim().is_empty() || id.trim().is_empty() {
        return None;
    }
    let payload = TrackPageCursor {
        version: TRACK_CURSOR_VERSION.to_string(),
        signature: signature.to_string(),
        file_path: file_path.to_string(),
        id: id.to_string(),
    };
    let json = serde_json::to_vec(&payload).ok()?;
    Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
}

fn decode_track_page_cursor(
    raw_cursor: Option<&str>,
    expected_signature: &str,
) -> BackendResult<Option<TrackPageCursor>> {
    let raw = match raw_cursor {
        Some(value) if !value.trim().is_empty() => value.trim(),
        _ => return Ok(None),
    };
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| BackendError::Validation("invalid cursor token".to_string()))?;
    let cursor: TrackPageCursor = serde_json::from_slice(&decoded)
        .map_err(|_| BackendError::Validation("invalid cursor payload".to_string()))?;
    if cursor.version != TRACK_CURSOR_VERSION {
        return Err(BackendError::Validation(
            "unsupported cursor version".to_string(),
        ));
    }
    if cursor.signature != expected_signature {
        return Err(BackendError::Validation(
            "cursor does not match current query".to_string(),
        ));
    }
    if cursor.file_path.trim().is_empty() || cursor.id.trim().is_empty() {
        return Err(BackendError::Validation(
            "invalid cursor payload".to_string(),
        ));
    }
    Ok(Some(cursor))
}

impl BackendService {
    fn backfill_track_fingerprints(&self) -> BackendResult<()> {
        let conn = self.db.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, artist, album FROM tracks WHERE match_fingerprint IS NULL OR match_fingerprint = ''",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        let pending = rows.collect::<Result<Vec<_>, _>>()?;
        if pending.is_empty() {
            return Ok(());
        }
        for (id, title, artist, album) in pending {
            let fp = build_track_match_fingerprint(&title, &artist, album.as_deref());
            conn.execute(
                "UPDATE tracks SET match_fingerprint = ?1 WHERE id = ?2",
                params![fp, id],
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- build_track_match_fingerprint ---

    #[test]
    fn fingerprint_deterministic() {
        let fp1 = build_track_match_fingerprint("Title", "Artist", Some("Album"));
        let fp2 = build_track_match_fingerprint("Title", "Artist", Some("Album"));
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn fingerprint_case_insensitive() {
        let lower = build_track_match_fingerprint("midnight", "dj shadow", Some("entroducing"));
        let upper = build_track_match_fingerprint("MIDNIGHT", "DJ SHADOW", Some("ENTRODUCING"));
        let mixed = build_track_match_fingerprint("Midnight", "DJ Shadow", Some("Entroducing"));
        assert_eq!(lower, upper);
        assert_eq!(lower, mixed);
    }

    #[test]
    fn fingerprint_whitespace_normalized() {
        let single = build_track_match_fingerprint("My Track", "An Artist", None);
        let multi = build_track_match_fingerprint("My   Track", "An  Artist", None);
        let tabs = build_track_match_fingerprint("My\tTrack", "An\tArtist", None);
        assert_eq!(single, multi);
        assert_eq!(single, tabs);
    }

    #[test]
    fn fingerprint_leading_trailing_whitespace() {
        let clean = build_track_match_fingerprint("Title", "Artist", None);
        let padded = build_track_match_fingerprint("  Title  ", "  Artist  ", None);
        assert_eq!(clean, padded);
    }

    #[test]
    fn fingerprint_different_tracks_differ() {
        let a = build_track_match_fingerprint("Track A", "Artist X", None);
        let b = build_track_match_fingerprint("Track B", "Artist X", None);
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_album_matters() {
        let with_album = build_track_match_fingerprint("Title", "Artist", Some("Album"));
        let without_album = build_track_match_fingerprint("Title", "Artist", None);
        let diff_album = build_track_match_fingerprint("Title", "Artist", Some("Other"));
        assert_ne!(with_album, without_album);
        assert_ne!(with_album, diff_album);
    }

    #[test]
    fn fingerprint_empty_fields() {
        // Should not panic on empty inputs
        let fp = build_track_match_fingerprint("", "", None);
        assert!(!fp.is_empty(), "fingerprint should still produce output");
        assert_eq!(fp.len(), 16, "fingerprint should be 16 hex chars");
    }

    #[test]
    fn fingerprint_is_hex_string() {
        let fp = build_track_match_fingerprint("Test", "Artist", Some("Album"));
        assert_eq!(fp.len(), 16);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn fingerprint_unicode_preserved() {
        // Unicode chars are lowercased but not stripped
        let a = build_track_match_fingerprint("Café", "Müsik", None);
        let b = build_track_match_fingerprint("café", "müsik", None);
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_very_long_strings() {
        let long = "a".repeat(10_000);
        let fp = build_track_match_fingerprint(&long, &long, Some(&long));
        assert_eq!(fp.len(), 16);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // --- normalize_hash_part ---

    #[test]
    fn normalize_hash_part_collapses_whitespace() {
        assert_eq!(normalize_hash_part("a  b   c"), "a b c");
    }

    #[test]
    fn normalize_hash_part_lowercases() {
        assert_eq!(normalize_hash_part("HELLO"), "hello");
    }

    #[test]
    fn normalize_hash_part_trims() {
        assert_eq!(normalize_hash_part("  hello  "), "hello");
    }

    #[test]
    fn normalize_hash_part_tabs_to_spaces() {
        assert_eq!(normalize_hash_part("a\tb"), "a b");
    }

    #[test]
    fn normalize_hash_part_empty() {
        assert_eq!(normalize_hash_part(""), "");
    }

    #[test]
    fn master_db_resource_candidates_prefer_windows_share_for_pioneer_paths() {
        let master_path = Path::new("/tmp/AppData/Roaming/Pioneer/rekordbox/master.db");
        let candidates = master_db_resource_candidates(
            master_path,
            "/PIONEER/USBANLZ/b13/f6121-04a7-4e91-9d08-0b039109a193/ANLZ0000.DAT",
        );

        assert_eq!(
            candidates[0],
            PathBuf::from("/tmp/AppData/Roaming/Pioneer/rekordbox")
                .join("share")
                .join("PIONEER/USBANLZ/b13/f6121-04a7-4e91-9d08-0b039109a193/ANLZ0000.DAT")
        );
        assert_eq!(
            candidates[1],
            PathBuf::from("/tmp/AppData/Roaming/Pioneer/rekordbox")
                .join("PIONEER/USBANLZ/b13/f6121-04a7-4e91-9d08-0b039109a193/ANLZ0000.DAT")
        );
    }

    #[test]
    fn master_db_resource_candidates_ignore_blank_values() {
        let candidates = master_db_resource_candidates(Path::new("/tmp/rekordbox/master.db"), " ");
        assert!(candidates.is_empty());
    }

    #[test]
    fn master_db_analysis_file_candidates_prefer_ext_for_pwv4() {
        let master_path = Path::new("/tmp/AppData/Roaming/Pioneer/rekordbox/master.db");
        let candidates = master_db_analysis_file_candidates(
            master_path,
            "/PIONEER/USBANLZ/b13/f6121-04a7-4e91-9d08-0b039109a193/ANLZ0000.DAT",
        );

        assert_eq!(
            candidates[0],
            PathBuf::from("/tmp/AppData/Roaming/Pioneer/rekordbox")
                .join("share")
                .join("PIONEER/USBANLZ/b13/f6121-04a7-4e91-9d08-0b039109a193/ANLZ0000.EXT")
        );
        assert_eq!(
            candidates[1],
            PathBuf::from("/tmp/AppData/Roaming/Pioneer/rekordbox")
                .join("share")
                .join("PIONEER/USBANLZ/b13/f6121-04a7-4e91-9d08-0b039109a193/ANLZ0000.2EX")
        );
    }

    #[test]
    fn decode_track_page_cursor_rejects_invalid_token() {
        let signature = build_track_cursor_signature(&[TRACK_CURSOR_VERSION, "list_tracks"]);
        let result = decode_track_page_cursor(Some("not-base64"), &signature);
        assert!(result.is_err());
    }

    #[test]
    fn decode_track_page_cursor_rejects_signature_mismatch() {
        let sig_a = build_track_cursor_signature(&[TRACK_CURSOR_VERSION, "list_tracks"]);
        let sig_b = build_track_cursor_signature(&[TRACK_CURSOR_VERSION, "search_tracks", "abc"]);
        let token = encode_track_page_cursor(&sig_a, "/music/a.mp3", "track-a").expect("token");
        let result = decode_track_page_cursor(Some(&token), &sig_b);
        assert!(result.is_err());
    }

    #[test]
    fn decode_track_page_cursor_accepts_matching_signature() {
        let sig = build_track_cursor_signature(&[TRACK_CURSOR_VERSION, "search_tracks", "q"]);
        let token = encode_track_page_cursor(&sig, "/music/a.mp3", "track-a").expect("token");
        let decoded = decode_track_page_cursor(Some(&token), &sig)
            .expect("decode ok")
            .expect("cursor present");
        assert_eq!(decoded.file_path, "/music/a.mp3");
        assert_eq!(decoded.id, "track-a");
        assert_eq!(decoded.signature, sig);
    }

    // --- find_confident_fingerprint_match / untainted_usb_root_paths ---

    fn test_db() -> (tempfile::TempDir, crate::db::Db) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = crate::db::Db::new(dir.path()).expect("db init");
        (dir, db)
    }

    fn insert_track(
        conn: &rusqlite::Connection,
        id: &str,
        file_path: &str,
        fingerprint: &str,
        duration_ms: Option<i64>,
        file_size_bytes: Option<i64>,
    ) {
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, duration_ms, file_size_bytes, match_fingerprint, created_at, updated_at)
             VALUES (?1, 'T', 'A', ?2, ?3, ?4, ?5, datetime('now'), datetime('now'))",
            params![id, file_path, duration_ms, file_size_bytes, fingerprint],
        )
        .expect("insert track");
    }

    #[test]
    fn find_confident_fingerprint_match_requires_duration_and_size_agreement() {
        let (_dir, db) = test_db();
        let conn = db.connect().expect("connect");
        insert_track(
            &conn,
            "local-1",
            "/music/a.mp3",
            "fp1",
            Some(200_000),
            Some(1_000),
        );

        // Both agree -> match.
        let matched =
            find_confident_fingerprint_match(&conn, "fp1", Some(201_000), Some(1_000), &[])
                .expect("query");
        assert_eq!(matched, Some("local-1".to_string()));

        // Duration diverges beyond tolerance -> no match.
        let no_match =
            find_confident_fingerprint_match(&conn, "fp1", Some(210_000), Some(1_000), &[])
                .expect("query");
        assert_eq!(no_match, None);

        // Duration matches but size diverges -> no match (Clean/Explicit collision case).
        let no_match_size =
            find_confident_fingerprint_match(&conn, "fp1", Some(200_500), Some(999), &[])
                .expect("query");
        assert_eq!(no_match_size, None);
    }

    #[test]
    fn find_confident_fingerprint_match_relaxes_missing_gate_data() {
        let (_dir, db) = test_db();
        let conn = db.connect().expect("connect");
        insert_track(&conn, "local-1", "/music/a.mp3", "fp1", Some(200_000), None);

        // Local candidate has no file_size_bytes recorded -- duration alone,
        // when it matches, is still enough (missing data doesn't force a
        // non-match, it just isn't checked).
        let matched =
            find_confident_fingerprint_match(&conn, "fp1", Some(200_100), Some(9_999), &[])
                .expect("query");
        assert_eq!(matched, Some("local-1".to_string()));
    }

    #[test]
    fn find_confident_fingerprint_match_refuses_to_pick_between_two_candidates() {
        let (_dir, db) = test_db();
        let conn = db.connect().expect("connect");
        insert_track(
            &conn,
            "local-1",
            "/music/a.mp3",
            "fp1",
            Some(200_000),
            Some(1_000),
        );
        insert_track(
            &conn,
            "local-2",
            "/music/b.mp3",
            "fp1",
            Some(200_000),
            Some(1_000),
        );

        let result =
            find_confident_fingerprint_match(&conn, "fp1", Some(200_000), Some(1_000), &[])
                .expect("query");
        assert_eq!(result, None, "ambiguous ties must not auto-merge");
    }

    #[test]
    fn find_confident_fingerprint_match_skips_candidates_under_a_known_usb_root() {
        let (_dir, db) = test_db();
        let conn = db.connect().expect("connect");
        insert_track(
            &conn,
            "placeholder-1",
            "/mnt/usb1/Contents/a.mp3",
            "fp1",
            Some(200_000),
            Some(1_000),
        );

        let result = find_confident_fingerprint_match(
            &conn,
            "fp1",
            Some(200_000),
            Some(1_000),
            &["/mnt/usb1".to_string()],
        )
        .expect("query");
        assert_eq!(
            result, None,
            "placeholder-vs-placeholder must never be treated as a match"
        );
    }

    #[test]
    fn untainted_usb_root_paths_excludes_configured_source_roots() {
        let (_dir, db) = test_db();
        let conn = db.connect().expect("connect");
        usb_utils::upsert_usb_device(&conn, Path::new("/mnt/data"), false, &now())
            .expect("seed usb device");
        conn.execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
            params![SETTING_UI_SOURCE_ROOTS, r#"["/mnt/data"]"#],
        )
        .expect("seed source roots setting");

        let paths = untainted_usb_root_paths(&conn).expect("untainted paths");
        assert!(
            !paths.iter().any(|p| p == "/mnt/data"),
            "a path reused as a configured source root should no longer be treated as USB-tainted"
        );
    }

    // --- small pure helpers ---

    #[test]
    fn sanitize_source_roots_trims_dedupes_and_drops_empty() {
        let roots = sanitize_source_roots(vec![
            "  /music  ".to_string(),
            "/music".to_string(),
            "   ".to_string(),
            "/other".to_string(),
        ]);
        assert_eq!(roots, vec!["/music".to_string(), "/other".to_string()]);
    }

    #[test]
    fn source_root_is_usable_true_for_dir_false_for_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(source_root_is_usable(dir.path().to_str().unwrap()));
        assert!(!source_root_is_usable(
            "/definitely/does/not/exist/anywhere"
        ));
    }

    #[test]
    fn source_root_status_reports_exists_and_is_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let status = source_root_status(dir.path().to_str().unwrap());
        assert!(status.exists);
        assert!(status.is_dir);

        let missing = source_root_status("/definitely/does/not/exist/anywhere");
        assert!(!missing.exists);
        assert!(!missing.is_dir);
    }

    #[test]
    fn relative_path_under_source_root_matches_case_insensitively() {
        assert_eq!(
            relative_path_under_source_root("/Music/Artist/song.mp3", "/music"),
            Some("Artist/song.mp3".to_string())
        );
        assert_eq!(
            relative_path_under_source_root("/music", "/music"),
            Some(String::new())
        );
        assert_eq!(
            relative_path_under_source_root("/other/song.mp3", "/music"),
            None
        );
        assert_eq!(relative_path_under_source_root("/music/song.mp3", ""), None);
    }

    #[test]
    fn relocated_source_path_joins_segments_and_handles_root_only() {
        let root = Path::new("/new/root");
        assert_eq!(
            relocated_source_path(root, "Artist/song.mp3"),
            root.join("Artist").join("song.mp3")
        );
        assert_eq!(relocated_source_path(root, ""), root.to_path_buf());
        assert_eq!(relocated_source_path(root, "///"), root.to_path_buf());
    }

    #[test]
    fn track_has_core_analysis_for_source_status_requires_all_three_fields() {
        let mut track = sample_track("t1", "/music/a.mp3");
        assert!(!track_has_core_analysis_for_source_status(&track));

        track.waveform_peaks_path = Some("/data/a.dat".to_string());
        track.bpm = Some(120.0);
        track.duration_ms = Some(200_000);
        assert!(track_has_core_analysis_for_source_status(&track));

        track.bpm = Some(0.0);
        assert!(!track_has_core_analysis_for_source_status(&track));
    }

    #[test]
    fn has_core_analysis_fields_requires_waveform_bpm_and_positive_duration() {
        assert!(has_core_analysis_fields(Some("/data/a.dat"), Some(120.0), Some(200_000)));
        // missing / blank waveform path
        assert!(!has_core_analysis_fields(None, Some(120.0), Some(200_000)));
        assert!(!has_core_analysis_fields(Some("   "), Some(120.0), Some(200_000)));
        // non-positive bpm
        assert!(!has_core_analysis_fields(Some("/data/a.dat"), Some(0.0), Some(200_000)));
        assert!(!has_core_analysis_fields(Some("/data/a.dat"), None, Some(200_000)));
        // non-positive / missing duration
        assert!(!has_core_analysis_fields(Some("/data/a.dat"), Some(120.0), Some(0)));
        assert!(!has_core_analysis_fields(Some("/data/a.dat"), Some(120.0), None));
    }

    #[test]
    fn non_empty_db_value_filters_blank_and_trims() {
        assert_eq!(non_empty_db_value("  hello  "), Some("hello"));
        assert_eq!(non_empty_db_value("   "), None);
        assert_eq!(non_empty_db_value(""), None);
    }

    #[test]
    fn looks_like_windows_absolute_path_detects_drive_letter() {
        assert!(looks_like_windows_absolute_path("C:/Users/dj"));
        assert!(looks_like_windows_absolute_path("D:\\Music"));
        assert!(!looks_like_windows_absolute_path("/mnt/data"));
        assert!(!looks_like_windows_absolute_path("relative/path"));
    }

    #[test]
    fn is_pioneer_virtual_path_case_insensitive() {
        assert!(is_pioneer_virtual_path("/PIONEER/USBANLZ/x"));
        assert!(is_pioneer_virtual_path("pioneer/usbanlz/x"));
        assert!(!is_pioneer_virtual_path("/Contents/x"));
    }

    #[test]
    fn push_unique_path_dedupes_equal_paths() {
        let mut paths = Vec::new();
        push_unique_path(&mut paths, PathBuf::from("/a"));
        push_unique_path(&mut paths, PathBuf::from("/a"));
        push_unique_path(&mut paths, PathBuf::from("/b"));
        assert_eq!(paths, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn resolve_master_db_resource_path_returns_first_existing_candidate() {
        let master_path = Path::new("/tmp/rekordbox/master.db");
        let db_path = "/PIONEER/USBANLZ/x/ANLZ0000.DAT";
        let expected = master_db_resource_candidates(master_path, db_path)
            .into_iter()
            .nth(1)
            .expect("at least two candidates");
        let expected_for_closure = expected.clone();
        let resolved =
            resolve_master_db_resource_path(master_path, db_path, |p| p == expected_for_closure);
        assert_eq!(resolved, Some(expected));
    }

    #[test]
    fn resolve_master_db_resource_path_none_when_nothing_exists() {
        let master_path = Path::new("/tmp/rekordbox/master.db");
        let resolved =
            resolve_master_db_resource_path(master_path, "/PIONEER/x/ANLZ0000.DAT", |_| false);
        assert!(resolved.is_none());
    }

    #[test]
    fn frontend_ui_setting_keys_contains_theme_and_help_seen() {
        let keys = frontend_ui_setting_keys();
        assert!(keys.contains(&SETTING_UI_THEME));
        assert!(keys.contains(&SETTING_UI_HELP_SEEN));
    }

    #[test]
    fn source_root_keys_equal_normalizes_case_and_trailing_slash() {
        assert!(source_root_keys_equal("/Music/", "/music"));
        assert!(!source_root_keys_equal("/music", "/other"));
    }

    #[test]
    fn path_file_name_and_stem_lower_handle_backslashes() {
        assert_eq!(path_file_name_lower(r"C:\Music\Song.MP3"), "song.mp3");
        assert_eq!(path_stem_lower(r"C:\Music\Song.MP3"), "song");
        assert_eq!(path_stem_lower("no-extension"), "no-extension");
    }

    #[test]
    fn build_track_cursor_signature_differs_by_input() {
        let sig_a = build_track_cursor_signature(&["v1", "list_tracks"]);
        let sig_b = build_track_cursor_signature(&["v1", "search_tracks"]);
        assert_ne!(sig_a, sig_b);
        assert_eq!(sig_a, build_track_cursor_signature(&["v1", "list_tracks"]));
    }

    #[test]
    fn check_essentia_installed_false_when_files_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!check_essentia_installed(dir.path()));
    }

    // --- BackendService CRUD/service-level tests ---

    fn test_service() -> (tempfile::TempDir, BackendService) {
        let dir = tempfile::tempdir().expect("service data dir");
        let service = BackendService::new(dir.path()).expect("backend service");
        (dir, service)
    }

    fn sample_track(id: &str, file_path: &str) -> Track {
        Track {
            id: id.to_string(),
            title: "Title".to_string(),
            artist: "Artist".to_string(),
            album: None,
            track_number: None,
            bpm: None,
            bpm_analyzer: None,
            key: None,
            file_path: file_path.to_string(),
            file_size_bytes: None,
            format_ext: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            wav_extensible_kind: None,
            duration_ms: None,
            artwork_path: None,
            artwork_data_url: None,
            waveform_peaks_path: None,
            waveform_preview: None,
            waveform_color_data: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            master_db_source: false,
            is_usb_path: false,
            analysis_ready: false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_full_track(
        conn: &rusqlite::Connection,
        id: &str,
        title: &str,
        artist: &str,
        file_path: &str,
        duration_ms: Option<i64>,
        file_size_bytes: Option<i64>,
        master_db_source: bool,
    ) {
        let fp = build_track_match_fingerprint(title, artist, None);
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, duration_ms, file_size_bytes, match_fingerprint, master_db_source, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'), datetime('now'))",
            params![id, title, artist, file_path, duration_ms, file_size_bytes, fp, master_db_source as i64],
        )
        .expect("insert track");
    }

    #[test]
    fn create_playlist_rejects_empty_name() {
        let (_dir, service) = test_service();
        let err = service
            .create_playlist(CreatePlaylistRequest {
                name: "   ".to_string(),
            })
            .unwrap_err();
        assert!(matches!(err, BackendError::Validation(_)));
    }

    #[test]
    fn playlist_create_rename_delete_roundtrip() {
        let (_dir, service) = test_service();
        let created = service
            .create_playlist(CreatePlaylistRequest {
                name: "  My Playlist  ".to_string(),
            })
            .expect("create playlist");
        assert_eq!(created.name, "My Playlist");

        let listed = service.list_playlists().expect("list playlists");
        assert_eq!(listed.items.len(), 1);
        assert_eq!(listed.items[0].id, created.playlist_id);

        let renamed = service
            .rename_playlist(RenamePlaylistRequest {
                playlist_id: created.playlist_id.clone(),
                name: "Renamed".to_string(),
            })
            .expect("rename playlist");
        assert_eq!(renamed.name, "Renamed");

        let deleted = service
            .delete_playlist(DeletePlaylistRequest {
                playlist_id: created.playlist_id.clone(),
            })
            .expect("delete playlist");
        assert!(deleted.deleted);

        let err = service
            .rename_playlist(RenamePlaylistRequest {
                playlist_id: created.playlist_id,
                name: "Nope".to_string(),
            })
            .unwrap_err();
        assert!(matches!(err, BackendError::NotFound(_)));
    }

    #[test]
    fn add_tracks_to_playlist_rejects_empty_ids() {
        let (_dir, service) = test_service();
        let playlist = service
            .create_playlist(CreatePlaylistRequest {
                name: "PL".to_string(),
            })
            .expect("create playlist");
        let err = service
            .add_tracks_to_playlist(AddTracksToPlaylistRequest {
                playlist_id: playlist.playlist_id,
                track_ids: Vec::new(),
                dedupe: DedupeMode::Allow,
            })
            .unwrap_err();
        assert!(matches!(err, BackendError::Validation(_)));
    }

    #[test]
    fn add_tracks_to_playlist_rejects_unknown_track() {
        let (_dir, service) = test_service();
        let playlist = service
            .create_playlist(CreatePlaylistRequest {
                name: "PL".to_string(),
            })
            .expect("create playlist");
        let err = service
            .add_tracks_to_playlist(AddTracksToPlaylistRequest {
                playlist_id: playlist.playlist_id,
                track_ids: vec!["missing-track".to_string()],
                dedupe: DedupeMode::Allow,
            })
            .unwrap_err();
        assert!(matches!(err, BackendError::NotFound(_)));
    }

    #[test]
    fn add_track_candidates_to_playlist_materializes_local_source_rows() {
        let (_dir, service) = test_service();
        let source_dir = tempfile::tempdir().expect("source dir");
        let file_path = source_dir.path().join("track.mp3");
        std::fs::write(&file_path, b"audio").expect("write source file");
        let playlist = service
            .create_playlist(CreatePlaylistRequest {
                name: "PL".to_string(),
            })
            .expect("create playlist");

        let result = service
            .add_track_candidates_to_playlist(AddTrackCandidatesToPlaylistRequest {
                playlist_id: playlist.playlist_id.clone(),
                tracks: vec![AddTrackCandidate {
                    track_id: Some(file_path.to_string_lossy().to_string()),
                    title: "Track".to_string(),
                    artist: "Artist".to_string(),
                    file_path: Some(file_path.to_string_lossy().to_string()),
                    ..Default::default()
                }],
                dedupe: DedupeMode::Skip,
                usb_root: None,
                usb_root_valid: false,
            })
            .expect("add candidate");

        assert_eq!(result.requested, 1);
        assert_eq!(result.resolved, 1);
        assert_eq!(result.unresolved, 0);
        assert_eq!(result.added, 1);
        assert_eq!(result.skipped, 0);
        assert!(result.resolutions[0].materialized);
        assert_eq!(result.resolutions[0].resolved_by, "materialized");

        let tracks = service
            .get_playlist_tracks(GetPlaylistTracksRequest {
                playlist_id: playlist.playlist_id,
            })
            .expect("get playlist tracks");
        assert_eq!(tracks.items.len(), 1);
        assert_eq!(tracks.items[0].title, "Track");
    }

    #[test]
    fn add_track_candidates_to_playlist_does_not_fuzzy_resolve_usb_origin_rows() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        insert_full_track(
            &conn,
            "local-1",
            "Same Song",
            "Same Artist",
            "/music/same.mp3",
            None,
            Some(1000),
            false,
        );
        drop(conn);
        let playlist = service
            .create_playlist(CreatePlaylistRequest {
                name: "PL".to_string(),
            })
            .expect("create playlist");

        let result = service
            .add_track_candidates_to_playlist(AddTrackCandidatesToPlaylistRequest {
                playlist_id: playlist.playlist_id.clone(),
                tracks: vec![AddTrackCandidate {
                    track_id: Some("usb-1".to_string()),
                    title: "Same Song".to_string(),
                    artist: "Same Artist".to_string(),
                    file_path: Some("/usb/Contents/same.mp3".to_string()),
                    file_size_bytes: Some(1000),
                    usb_root: Some("/usb".to_string()),
                    usb_analysis_path: Some("/usb/PIONEER/USBANLZ/P001/A/ANLZ0000.DAT".to_string()),
                    ..Default::default()
                }],
                dedupe: DedupeMode::Skip,
                usb_root: None,
                usb_root_valid: false,
            })
            .expect("add candidate");

        assert_eq!(result.requested, 1);
        assert_eq!(result.resolved, 0);
        assert_eq!(result.unresolved, 1);
        assert_eq!(result.added, 0);
        assert_eq!(result.skipped, 0);
        assert_eq!(result.resolutions[0].resolved_by, "usbOrigin");

        let tracks = service
            .get_playlist_tracks(GetPlaylistTracksRequest {
                playlist_id: playlist.playlist_id,
            })
            .expect("get playlist tracks");
        assert!(tracks.items.is_empty());
    }

    #[test]
    fn add_tracks_to_playlist_dedupe_skip_counts_skipped_and_get_playlist_tracks_orders_by_position()
     {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        insert_full_track(
            &conn,
            "t1",
            "Song A",
            "Artist",
            "/music/a.mp3",
            None,
            None,
            false,
        );
        insert_full_track(
            &conn,
            "t2",
            "Song B",
            "Artist",
            "/music/b.mp3",
            None,
            None,
            false,
        );
        drop(conn);

        let playlist = service
            .create_playlist(CreatePlaylistRequest {
                name: "PL".to_string(),
            })
            .expect("create playlist");

        let added = service
            .add_tracks_to_playlist(AddTracksToPlaylistRequest {
                playlist_id: playlist.playlist_id.clone(),
                track_ids: vec!["t1".to_string(), "t2".to_string()],
                dedupe: DedupeMode::Allow,
            })
            .expect("add tracks");
        assert_eq!(added.added, 2);
        assert_eq!(added.skipped, 0);

        let skip_result = service
            .add_tracks_to_playlist(AddTracksToPlaylistRequest {
                playlist_id: playlist.playlist_id.clone(),
                track_ids: vec!["t1".to_string()],
                dedupe: DedupeMode::Skip,
            })
            .expect("add duplicate with skip mode");
        assert_eq!(skip_result.added, 0);
        assert_eq!(skip_result.skipped, 1);

        let tracks = service
            .get_playlist_tracks(GetPlaylistTracksRequest {
                playlist_id: playlist.playlist_id,
            })
            .expect("get playlist tracks");
        assert_eq!(
            tracks
                .items
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>(),
            vec!["t1".to_string(), "t2".to_string()]
        );
    }

    #[test]
    fn get_playlist_tracks_computes_total_duration_server_side() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        insert_full_track(
            &conn,
            "t1",
            "Song A",
            "Artist",
            "/music/a.mp3",
            Some(200_000),
            None,
            false,
        );
        insert_full_track(
            &conn,
            "t2",
            "Song B",
            "Artist",
            "/music/b.mp3",
            None,
            None,
            false,
        );
        // master.db-sourced -- must survive get_playlist_tracks' hand-written
        // SELECT (regression: its column list was misaligned vs `row_to_track`,
        // which reads master_db_source positionally, so every playlist track
        // came back master_db_source=false).
        insert_full_track(
            &conn,
            "t3",
            "Song C",
            "Artist",
            "/music/c.mp3",
            Some(100_000),
            None,
            true,
        );
        drop(conn);

        let playlist = service
            .create_playlist(CreatePlaylistRequest {
                name: "PL".to_string(),
            })
            .expect("create playlist");
        service
            .add_tracks_to_playlist(AddTracksToPlaylistRequest {
                playlist_id: playlist.playlist_id.clone(),
                track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
                dedupe: DedupeMode::Allow,
            })
            .expect("add tracks");

        let tracks = service
            .get_playlist_tracks(GetPlaylistTracksRequest {
                playlist_id: playlist.playlist_id,
            })
            .expect("get playlist tracks");
        assert_eq!(tracks.total_duration_ms, 300_000);
        assert_eq!(tracks.duration_known_count, 2);

        let by_id = |id: &str| {
            tracks
                .items
                .iter()
                .find(|t| t.id == id)
                .unwrap_or_else(|| panic!("missing track {id}"))
        };
        assert!(
            by_id("t3").master_db_source,
            "master_db_source must survive get_playlist_tracks"
        );
        assert!(!by_id("t1").master_db_source);
    }

    #[test]
    fn remove_tracks_from_playlist_recompacts_positions() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        insert_full_track(
            &conn,
            "t1",
            "Song A",
            "Artist",
            "/music/a.mp3",
            None,
            None,
            false,
        );
        insert_full_track(
            &conn,
            "t2",
            "Song B",
            "Artist",
            "/music/b.mp3",
            None,
            None,
            false,
        );
        insert_full_track(
            &conn,
            "t3",
            "Song C",
            "Artist",
            "/music/c.mp3",
            None,
            None,
            false,
        );
        drop(conn);

        let playlist = service
            .create_playlist(CreatePlaylistRequest {
                name: "PL".to_string(),
            })
            .expect("create playlist");
        service
            .add_tracks_to_playlist(AddTracksToPlaylistRequest {
                playlist_id: playlist.playlist_id.clone(),
                track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
                dedupe: DedupeMode::Allow,
            })
            .expect("add tracks");

        let removed = service
            .remove_tracks_from_playlist(RemoveTracksFromPlaylistRequest {
                playlist_id: playlist.playlist_id.clone(),
                track_ids: vec!["t2".to_string()],
            })
            .expect("remove track");
        assert_eq!(removed.removed, 1);

        let tracks = service
            .get_playlist_tracks(GetPlaylistTracksRequest {
                playlist_id: playlist.playlist_id,
            })
            .expect("get playlist tracks");
        assert_eq!(
            tracks
                .items
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>(),
            vec!["t1".to_string(), "t3".to_string()]
        );
    }

    #[test]
    fn reorder_playlist_tracks_applies_arbitrary_new_order() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        insert_full_track(
            &conn,
            "t1",
            "Song A",
            "Artist",
            "/music/a.mp3",
            None,
            None,
            false,
        );
        insert_full_track(
            &conn,
            "t2",
            "Song B",
            "Artist",
            "/music/b.mp3",
            None,
            None,
            false,
        );
        insert_full_track(
            &conn,
            "t3",
            "Song C",
            "Artist",
            "/music/c.mp3",
            None,
            None,
            false,
        );
        drop(conn);

        let playlist = service
            .create_playlist(CreatePlaylistRequest {
                name: "PL".to_string(),
            })
            .expect("create playlist");
        service
            .add_tracks_to_playlist(AddTracksToPlaylistRequest {
                playlist_id: playlist.playlist_id.clone(),
                track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
                dedupe: DedupeMode::Allow,
            })
            .expect("add tracks");

        let reordered = service
            .reorder_playlist_tracks(ReorderPlaylistTracksRequest {
                playlist_id: playlist.playlist_id.clone(),
                ordered_track_ids: vec!["t3".to_string(), "t1".to_string(), "t2".to_string()],
            })
            .expect("reorder tracks");
        assert_eq!(reordered.reordered, 3);

        let tracks = service
            .get_playlist_tracks(GetPlaylistTracksRequest {
                playlist_id: playlist.playlist_id,
            })
            .expect("get playlist tracks");
        assert_eq!(
            tracks
                .items
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>(),
            vec!["t3".to_string(), "t1".to_string(), "t2".to_string()]
        );
    }

    #[test]
    fn reorder_playlist_tracks_rejects_mismatched_track_set() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        insert_full_track(
            &conn,
            "t1",
            "Song A",
            "Artist",
            "/music/a.mp3",
            None,
            None,
            false,
        );
        insert_full_track(
            &conn,
            "t2",
            "Song B",
            "Artist",
            "/music/b.mp3",
            None,
            None,
            false,
        );
        drop(conn);

        let playlist = service
            .create_playlist(CreatePlaylistRequest {
                name: "PL".to_string(),
            })
            .expect("create playlist");
        service
            .add_tracks_to_playlist(AddTracksToPlaylistRequest {
                playlist_id: playlist.playlist_id.clone(),
                track_ids: vec!["t1".to_string(), "t2".to_string()],
                dedupe: DedupeMode::Allow,
            })
            .expect("add tracks");

        let missing_track = service.reorder_playlist_tracks(ReorderPlaylistTracksRequest {
            playlist_id: playlist.playlist_id.clone(),
            ordered_track_ids: vec!["t1".to_string()],
        });
        assert!(missing_track.is_err());

        let unknown_track = service.reorder_playlist_tracks(ReorderPlaylistTracksRequest {
            playlist_id: playlist.playlist_id.clone(),
            ordered_track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        });
        assert!(unknown_track.is_err());

        let duplicate_track = service.reorder_playlist_tracks(ReorderPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ordered_track_ids: vec!["t1".to_string(), "t1".to_string()],
        });
        assert!(duplicate_track.is_err());
    }

    #[test]
    fn frontend_settings_roundtrip_and_reject_unsupported_key() {
        let (_dir, service) = test_service();
        let defaults = service.get_frontend_settings().expect("get settings");
        assert!(defaults.values.is_empty());

        service
            .set_frontend_setting(SetFrontendSettingRequest {
                key: SETTING_UI_THEME.to_string(),
                value: Some("dark".to_string()),
            })
            .expect("set theme");
        let after_set = service.get_frontend_settings().expect("get settings");
        assert_eq!(
            after_set.values.get(SETTING_UI_THEME).map(String::as_str),
            Some("dark")
        );

        service
            .set_frontend_setting(SetFrontendSettingRequest {
                key: SETTING_UI_THEME.to_string(),
                value: None,
            })
            .expect("clear theme");
        let after_clear = service.get_frontend_settings().expect("get settings");
        assert!(!after_clear.values.contains_key(SETTING_UI_THEME));

        let err = service
            .set_frontend_setting(SetFrontendSettingRequest {
                key: "not_a_real_setting".to_string(),
                value: Some("x".to_string()),
            })
            .unwrap_err();
        assert!(matches!(err, BackendError::Validation(_)));
    }

    #[test]
    fn remove_essentia_removes_dir_and_clears_setting() {
        let (_dir, service) = test_service();
        let essentia_dir = service.db.data_dir().join("essentia");
        std::fs::create_dir_all(&essentia_dir).expect("create essentia dir");
        std::fs::write(essentia_dir.join("marker.txt"), b"x").expect("write marker");
        service
            .set_frontend_setting(SetFrontendSettingRequest {
                key: SETTING_UI_ANALYSIS_ENGINE.to_string(),
                value: Some("essentia".to_string()),
            })
            .expect("set analysis engine");

        service.remove_essentia().expect("remove essentia");

        assert!(!essentia_dir.exists());
        let settings = service.get_frontend_settings().expect("get settings");
        assert!(!settings.values.contains_key(SETTING_UI_ANALYSIS_ENGINE));
    }

    #[test]
    fn list_tracks_paginates_with_cursor() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        for i in 0..3 {
            insert_full_track(
                &conn,
                &format!("t{i}"),
                &format!("Song {i}"),
                "Artist",
                &format!("/music/{i}.mp3"),
                None,
                None,
                false,
            );
        }
        drop(conn);

        let page1 = service
            .list_tracks(ListTracksRequest {
                limit: 2,
                cursor: None,
            })
            .expect("list page 1");
        assert_eq!(page1.total, 3);
        assert_eq!(page1.items.len(), 2);
        assert!(page1.has_more);
        let cursor = page1.next_cursor.expect("cursor present");

        let page2 = service
            .list_tracks(ListTracksRequest {
                limit: 2,
                cursor: Some(cursor),
            })
            .expect("list page 2");
        assert_eq!(page2.items.len(), 1);
        assert!(!page2.has_more);
    }

    #[test]
    fn search_tracks_filters_by_query() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        insert_full_track(
            &conn,
            "t1",
            "Aurora",
            "DJ One",
            "/music/a.mp3",
            None,
            None,
            false,
        );
        insert_full_track(
            &conn,
            "t2",
            "Nebula",
            "DJ Two",
            "/music/b.mp3",
            None,
            None,
            false,
        );
        drop(conn);

        let result = service
            .search_tracks(SearchTracksRequest {
                query: "aurora".to_string(),
                limit: 10,
                cursor: None,
            })
            .expect("search tracks");
        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].id, "t1");

        let empty = service
            .search_tracks(SearchTracksRequest {
                query: "nonexistent".to_string(),
                limit: 10,
                cursor: None,
            })
            .expect("search tracks empty");
        assert_eq!(empty.total, 0);
    }

    #[test]
    fn browse_source_files_empty_without_roots_or_master_db() {
        let (_dir, service) = test_service();
        let result = service
            .browse_source_files(BrowseSourceFilesRequest {
                source_roots: Vec::new(),
                include_master_db: false,
                query: String::new(),
                limit: 100,
                cursor: None,
            })
            .expect("browse source files");
        assert_eq!(result.total, 0);
        assert!(result.items.is_empty());
    }

    #[test]
    fn browse_source_files_includes_master_db_tracks_when_requested() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        insert_full_track(
            &conn,
            "t1",
            "Master Song",
            "Artist",
            "/master/a.mp3",
            None,
            None,
            true,
        );
        drop(conn);

        let result = service
            .browse_source_files(BrowseSourceFilesRequest {
                source_roots: Vec::new(),
                include_master_db: true,
                query: String::new(),
                limit: 100,
                cursor: None,
            })
            .expect("browse source files");
        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].id, "t1");
        assert!(result.items[0].master_db_source);
    }

    #[test]
    fn browse_source_files_computes_duration_total_over_countable_tracks_only() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        // Fully core-analyzed (bpm + waveform + duration): countable via the
        // primary rule.
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, waveform_peaks_path, match_fingerprint, master_db_source, created_at, updated_at)
             VALUES ('t1', 'Analyzed', 'Artist', '/master/a.mp3', 200000, 120.0, '/data/a.dat', ?1, 1, datetime('now'), datetime('now'))",
            params![build_track_match_fingerprint("Analyzed", "Artist", None)],
        )
        .expect("insert t1");
        // master.db track with only a duration: countable via the
        // master-db-specific OR-branch, not the primary rule.
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, duration_ms, match_fingerprint, master_db_source, created_at, updated_at)
             VALUES ('t2', 'Master Only', 'Artist', '/master/b.mp3', 100000, ?1, 1, datetime('now'), datetime('now'))",
            params![build_track_match_fingerprint("Master Only", "Artist", None)],
        )
        .expect("insert t2");
        // No analysis at all: not countable, excluded from both the sum and
        // the known count, but still present in `total`.
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, match_fingerprint, master_db_source, created_at, updated_at)
             VALUES ('t3', 'Unanalyzed', 'Artist', '/master/c.mp3', ?1, 1, datetime('now'), datetime('now'))",
            params![build_track_match_fingerprint("Unanalyzed", "Artist", None)],
        )
        .expect("insert t3");
        drop(conn);

        let result = service
            .browse_source_files(BrowseSourceFilesRequest {
                source_roots: Vec::new(),
                include_master_db: true,
                query: String::new(),
                limit: 100,
                cursor: None,
            })
            .expect("browse source files");
        assert_eq!(result.total, 3);
        assert_eq!(result.duration_known_count, 2);
        assert_eq!(result.total_duration_ms, 300_000);
    }

    #[test]
    fn track_rows_carry_analysis_ready_computed_from_db_fields() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        // Fully core-analyzed: bpm + waveform path + duration.
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, waveform_peaks_path, match_fingerprint, master_db_source, created_at, updated_at)
             VALUES ('ready', 'Ready', 'Artist', '/master/ready.mp3', 200000, 120.0, '/data/ready.dat', ?1, 1, datetime('now'), datetime('now'))",
            params![build_track_match_fingerprint("Ready", "Artist", None)],
        )
        .expect("insert ready");
        // Has duration + bpm but no waveform path -> still needs analysis.
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, match_fingerprint, master_db_source, created_at, updated_at)
             VALUES ('missing', 'Missing', 'Artist', '/master/missing.mp3', 200000, 120.0, ?1, 1, datetime('now'), datetime('now'))",
            params![build_track_match_fingerprint("Missing", "Artist", None)],
        )
        .expect("insert missing");
        drop(conn);

        let result = service
            .browse_source_files(BrowseSourceFilesRequest {
                source_roots: Vec::new(),
                include_master_db: true,
                query: String::new(),
                limit: 100,
                cursor: None,
            })
            .expect("browse source files");
        let by_id = |id: &str| result.items.iter().find(|t| t.id == id).unwrap().analysis_ready;
        assert!(by_id("ready"));
        assert!(!by_id("missing"));
    }

    #[test]
    fn track_rows_derive_format_ext_from_path_when_the_column_is_null() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        // master.db import path leaves format_ext NULL.
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, match_fingerprint, master_db_source, created_at, updated_at)
             VALUES ('m1', 'Song', 'Artist', '/master/Artist/Song.FLAC', ?1, 1, datetime('now'), datetime('now'))",
            params![build_track_match_fingerprint("Song", "Artist", None)],
        )
        .expect("insert null-format row");
        drop(conn);

        let browsed = service
            .browse_source_files(BrowseSourceFilesRequest {
                source_roots: Vec::new(),
                include_master_db: true,
                query: String::new(),
                limit: 100,
                cursor: None,
            })
            .expect("browse source files");
        assert_eq!(
            browsed.items.iter().find(|t| t.id == "m1").unwrap().format_ext.as_deref(),
            Some("flac"),
            "row_to_track must fall back to the file-path extension"
        );

        let playlist = service
            .create_playlist(CreatePlaylistRequest {
                name: "PL".to_string(),
            })
            .expect("create playlist");
        service
            .add_tracks_to_playlist(AddTracksToPlaylistRequest {
                playlist_id: playlist.playlist_id.clone(),
                track_ids: vec!["m1".to_string()],
                dedupe: DedupeMode::Allow,
            })
            .expect("add track");
        let tracks = service
            .get_playlist_tracks(GetPlaylistTracksRequest {
                playlist_id: playlist.playlist_id,
            })
            .expect("get playlist tracks");
        assert_eq!(tracks.items[0].format_ext.as_deref(), Some("flac"));
    }

    #[test]
    fn browse_source_files_source_root_analysis_ignores_the_search_query() {
        let (_dir, service) = test_service();
        let root_a = tempfile::tempdir().expect("root a");
        let root_b = tempfile::tempdir().expect("root b");
        let path_a = root_a.path().join("alpha.mp3");
        let path_b = root_b.path().join("bravo.mp3");
        std::fs::write(&path_a, b"data").expect("write alpha");
        std::fs::write(&path_b, b"data").expect("write bravo");
        let path_a_str = path_a.to_string_lossy().to_string();
        let path_b_str = path_b.to_string_lossy().to_string();

        let conn = service.db.connect().expect("connect");
        // Both tracks are fully analyzed (bpm + waveform + duration). Their
        // titles deliberately share no substring, so a query matching one
        // matches zero tracks in the other's source root.
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, waveform_peaks_path, match_fingerprint, master_db_source, created_at, updated_at)
             VALUES ('t1', 'Findable Alpha', 'Artist', ?1, 200000, 120.0, '/data/a.dat', ?2, 0, datetime('now'), datetime('now'))",
            params![path_a_str, build_track_match_fingerprint("Findable Alpha", "Artist", None)],
        )
        .expect("insert t1");
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, waveform_peaks_path, match_fingerprint, master_db_source, created_at, updated_at)
             VALUES ('t2', 'Unrelated Bravo', 'Artist', ?1, 200000, 120.0, '/data/b.dat', ?2, 0, datetime('now'), datetime('now'))",
            params![path_b_str, build_track_match_fingerprint("Unrelated Bravo", "Artist", None)],
        )
        .expect("insert t2");
        drop(conn);

        let result = service
            .browse_source_files(BrowseSourceFilesRequest {
                source_roots: vec![
                    root_a.path().to_string_lossy().to_string(),
                    root_b.path().to_string_lossy().to_string(),
                ],
                include_master_db: false,
                // Matches only the track in root_a -- root_b's track matches
                // nothing, which used to collapse its `total` to 0 and flip
                // `fully_analyzed` to false even though root_b is 100%
                // analyzed on its own.
                query: "Findable".to_string(),
                limit: 100,
                cursor: None,
            })
            .expect("browse source files");

        // The search still narrows what's returned/shown...
        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].id, "t1");

        // ...but both source roots' analyzed status reflects their full,
        // query-independent contents.
        assert_eq!(result.source_root_analysis.len(), 2);
        for status in &result.source_root_analysis {
            assert_eq!(status.total, 1, "source root {}", status.source_root);
            assert_eq!(status.analyzed, 1, "source root {}", status.source_root);
            assert!(status.fully_analyzed, "source root {}", status.source_root);
        }
    }

    #[test]
    fn check_source_roots_reports_missing_and_existing() {
        let (_dir, service) = test_service();
        let existing = tempfile::tempdir().expect("tempdir");
        let result = service
            .check_source_roots(CheckSourceRootsRequest {
                source_roots: vec![
                    existing.path().to_string_lossy().to_string(),
                    "/definitely/does/not/exist/anywhere".to_string(),
                ],
            })
            .expect("check source roots");
        assert_eq!(result.items.len(), 2);
        assert_eq!(
            result.missing,
            vec!["/definitely/does/not/exist/anywhere".to_string()]
        );
    }

    #[test]
    fn relocate_source_root_updates_matching_track_paths() {
        let (_dir, service) = test_service();
        let old_root = tempfile::tempdir().expect("old root");
        let new_root = tempfile::tempdir().expect("new root");
        std::fs::write(new_root.path().join("song.mp3"), b"data").expect("write new file");

        let old_path = old_root.path().join("song.mp3");
        let conn = service.db.connect().expect("connect");
        insert_full_track(
            &conn,
            "t1",
            "Song",
            "Artist",
            old_path.to_str().unwrap(),
            None,
            None,
            false,
        );
        drop(conn);

        let result = service
            .relocate_source_root(RelocateSourceRootRequest {
                old_root: old_root.path().to_string_lossy().to_string(),
                new_root: new_root.path().to_string_lossy().to_string(),
            })
            .expect("relocate source root");
        assert_eq!(result.matched, 1);
        assert_eq!(result.updated, 1);

        let conn = service.db.connect().expect("connect");
        let new_path: String = conn
            .query_row("SELECT file_path FROM tracks WHERE id = 't1'", [], |r| {
                r.get(0)
            })
            .expect("read updated path");
        assert_eq!(new_path, new_root.path().join("song.mp3").to_string_lossy());
    }

    #[test]
    fn relocate_source_root_rejects_same_root() {
        let (_dir, service) = test_service();
        let err = service
            .relocate_source_root(RelocateSourceRootRequest {
                old_root: "/music".to_string(),
                new_root: "/music".to_string(),
            })
            .unwrap_err();
        assert!(matches!(err, BackendError::Validation(_)));
    }

    #[test]
    fn materialize_source_track_inserts_new_then_updates_existing() {
        let (_dir, service) = test_service();
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("song.mp3");
        std::fs::write(&file_path, b"data").expect("write file");

        let inserted = service
            .materialize_source_track(MaterializeSourceTrackRequest {
                file_path: file_path.to_string_lossy().to_string(),
                title: "Song".to_string(),
                artist: "Artist".to_string(),
                album: None,
                track_number: None,
                key: None,
                file_size_bytes: None,
                format_ext: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
            })
            .expect("materialize new track");

        let updated = service
            .materialize_source_track(MaterializeSourceTrackRequest {
                file_path: file_path.to_string_lossy().to_string(),
                title: "Song Updated".to_string(),
                artist: "Artist".to_string(),
                album: None,
                track_number: None,
                key: None,
                file_size_bytes: None,
                format_ext: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
            })
            .expect("materialize existing track");
        assert_eq!(inserted.track_id, updated.track_id);

        let conn = service.db.connect().expect("connect");
        let title: String = conn
            .query_row(
                "SELECT title FROM tracks WHERE id = ?1",
                params![updated.track_id],
                |r| r.get(0),
            )
            .expect("read title");
        assert_eq!(title, "Song Updated");
    }

    #[test]
    fn materialize_source_track_rejects_missing_file() {
        let (_dir, service) = test_service();
        let err = service
            .materialize_source_track(MaterializeSourceTrackRequest {
                file_path: "/definitely/does/not/exist.mp3".to_string(),
                title: "Song".to_string(),
                artist: "Artist".to_string(),
                album: None,
                track_number: None,
                key: None,
                file_size_bytes: None,
                format_ext: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
            })
            .unwrap_err();
        assert!(matches!(err, BackendError::NotFound(_)));
    }

    #[test]
    fn resolve_track_identity_materializes_safe_local_source_path() {
        let (_dir, service) = test_service();
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("song.mp3");
        std::fs::write(&file_path, b"data").expect("write file");

        let result = service
            .resolve_track_identity(ResolveTrackIdentityRequest {
                track_id: Some(file_path.to_string_lossy().to_string()),
                title: "Song".to_string(),
                artist: "Artist".to_string(),
                album: None,
                bpm: None,
                file_path: Some(file_path.to_string_lossy().to_string()),
                file_size_bytes: None,
                track_number: None,
                key: Some("8A".to_string()),
                format_ext: Some("mp3".to_string()),
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                usb_root: Some("/usb".to_string()),
                usb_root_valid: true,
                usb_analysis_path: None,
            })
            .expect("resolve identity");

        assert!(result.materialized);
        assert_eq!(result.resolved_by, "materialized");
        assert!(result.track_id.is_some());
    }

    #[test]
    fn resolve_track_identity_skips_usb_materialization_and_resolves_local_match() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        insert_full_track(
            &conn,
            "local-1",
            "Song",
            "Artist",
            "/music/song.mp3",
            None,
            Some(1234),
            false,
        );
        drop(conn);

        let result = service
            .resolve_track_identity(ResolveTrackIdentityRequest {
                track_id: Some("usb-placeholder".to_string()),
                title: "Song".to_string(),
                artist: "Artist".to_string(),
                album: None,
                bpm: None,
                file_path: Some("/usb/Contents/song.mp3".to_string()),
                file_size_bytes: Some(1234),
                track_number: None,
                key: None,
                format_ext: Some("mp3".to_string()),
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                usb_root: Some("/usb".to_string()),
                usb_root_valid: true,
                usb_analysis_path: Some("/usb/PIONEER/USBANLZ/ANLZ0000.DAT".to_string()),
            })
            .expect("resolve identity");

        assert!(!result.materialized);
        assert_eq!(result.resolved_by, "hash");
        assert_eq!(result.track_id.as_deref(), Some("local-1"));
    }

    #[test]
    fn remove_tracks_by_source_roots_deletes_matching_prefix() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        insert_full_track(
            &conn,
            "t1",
            "A",
            "Artist",
            "/music/keep/a.mp3",
            None,
            None,
            false,
        );
        insert_full_track(
            &conn,
            "t2",
            "B",
            "Artist",
            "/music/drop/b.mp3",
            None,
            None,
            false,
        );
        drop(conn);

        let result = service
            .remove_tracks_by_source_roots(RemoveTracksBySourceRootsRequest {
                source_roots: vec!["/music/drop".to_string()],
            })
            .expect("remove tracks by source roots");
        assert_eq!(result.removed, 1);

        let remaining = service
            .list_tracks(ListTracksRequest {
                limit: 10,
                cursor: None,
            })
            .expect("list tracks");
        assert_eq!(remaining.total, 1);
        assert_eq!(remaining.items[0].id, "t1");
    }

    #[test]
    fn get_tracks_by_ids_with_previews_dedupes_and_sorts() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        insert_full_track(
            &conn,
            "t2",
            "B",
            "Artist",
            "/music/b.mp3",
            None,
            None,
            false,
        );
        insert_full_track(
            &conn,
            "t1",
            "A",
            "Artist",
            "/music/a.mp3",
            None,
            None,
            false,
        );
        drop(conn);

        let result = service
            .get_tracks_by_ids_with_previews(GetTracksByIdsRequest {
                track_ids: vec!["t2".to_string(), "t1".to_string(), "t1".to_string()],
            })
            .expect("get tracks by ids");
        assert_eq!(
            result
                .items
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>(),
            vec!["t1".to_string(), "t2".to_string()]
        );
    }

    #[test]
    fn get_tracks_by_ids_with_previews_empty_ids_returns_empty() {
        let (_dir, service) = test_service();
        let result = service
            .get_tracks_by_ids_with_previews(GetTracksByIdsRequest {
                track_ids: Vec::new(),
            })
            .expect("get tracks by ids");
        assert!(result.items.is_empty());
    }

    #[test]
    fn playback_source_label_matches_origin_and_resolution_context() {
        assert_eq!(playback_source_label(Some("local"), true, false), "Library");
        assert_eq!(
            playback_source_label(Some("playlist"), true, false),
            "Library"
        );
        assert_eq!(
            playback_source_label(Some("usb"), true, true),
            "Library (matched)"
        );
        assert_eq!(playback_source_label(Some("history"), false, true), "USB");
        assert_eq!(
            playback_source_label(Some("history"), false, false),
            "Local file"
        );
    }

    #[test]
    fn recoverable_playback_error_detection_matches_backend_retry_policy() {
        assert!(is_recoverable_playback_error("Output device is busy"));
        assert!(is_recoverable_playback_error("stream is already playing"));
        assert!(is_recoverable_playback_error("sink in use"));
        assert!(!is_recoverable_playback_error("decoder error: bad frame"));
    }

    #[test]
    fn resolve_playback_source_fast_path_by_track_id() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        insert_full_track(
            &conn,
            "t1",
            "Song",
            "Artist",
            "/music/a.mp3",
            None,
            None,
            false,
        );
        drop(conn);

        let result = service
            .resolve_playback_source(ResolvePlaybackSourceRequest {
                title: String::new(),
                artist: String::new(),
                album: None,
                bpm: None,
                file_path: None,
                file_size_bytes: None,
                track_id: Some("t1".to_string()),
            })
            .expect("resolve playback source");
        assert_eq!(result.matched_by, "self");
        assert_eq!(result.resolved_path.as_deref(), Some("/music/a.mp3"));
        assert_eq!(result.track_id.as_deref(), Some("t1"));
    }

    #[test]
    fn resolve_playback_source_falls_back_to_fingerprint_hash_match() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        insert_full_track(
            &conn,
            "t1",
            "Song",
            "Artist",
            "/music/a.mp3",
            None,
            None,
            false,
        );
        drop(conn);

        let result = service
            .resolve_playback_source(ResolvePlaybackSourceRequest {
                title: "Song".to_string(),
                artist: "Artist".to_string(),
                album: None,
                bpm: None,
                file_path: None,
                file_size_bytes: None,
                track_id: None,
            })
            .expect("resolve playback source");
        assert_eq!(result.matched_by, "hash");
        assert_eq!(result.track_id.as_deref(), Some("t1"));
    }

    #[test]
    fn resolve_playback_source_returns_none_for_blank_title_or_artist() {
        let (_dir, service) = test_service();
        let result = service
            .resolve_playback_source(ResolvePlaybackSourceRequest {
                title: "   ".to_string(),
                artist: "Artist".to_string(),
                album: None,
                bpm: None,
                file_path: None,
                file_size_bytes: None,
                track_id: None,
            })
            .expect("resolve playback source");
        assert_eq!(result.matched_by, "none");
        assert!(result.resolved_path.is_none());
    }
}
