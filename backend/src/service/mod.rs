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
pub(crate) mod usb_helpers;
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
use uuid::Uuid;

use base64::Engine;

use crate::db::Db;
use crate::error::{BackendError, BackendResult};
use crate::models::{
    AddTracksToPlaylistData, AddTracksToPlaylistRequest, BrowseSourceFilesData,
    BrowseSourceFilesRequest, CreatePlaylistData, CreatePlaylistRequest, DedupeMode,
    DeletePlaylistData, DeletePlaylistRequest, DetectExternalMasterDbData, GetFrontendSettingsData,
    GetPlaylistTracksData, GetPlaylistTracksRequest, GetSystemParallelismData, GetTracksByIdsData,
    GetTracksByIdsRequest, InitializeUsbData, InitializeUsbRequest, ListPlaylistsData,
    ListTracksData, ListTracksRequest, MaterializeSourceTrackData, MaterializeSourceTrackRequest,
    PlayTrackData, PlayTrackRequest, PlaybackPreflightData, PlaybackPreflightRequest,
    PlaybackStatusData, Playlist, RemoveTracksBySourceRootsData, RemoveTracksBySourceRootsRequest,
    RemoveTracksFromPlaylistData, RemoveTracksFromPlaylistRequest, RenamePlaylistData,
    RenamePlaylistRequest, ResolvePlaybackSourceData, ResolvePlaybackSourceRequest,
    ScanLibraryData, ScanLibraryRequest, ScanMasterDbRequest, SearchTracksData,
    SearchTracksRequest, SetFrontendSettingData, SetFrontendSettingRequest,
    SourceRootAnalysisStatus, StopPlaybackData, Track,
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
pub(crate) const SETTING_UI_ANALYSIS_BPM_RANGE: &str = "ui_analysis_bpm_range_v1";
pub(crate) const SETTING_UI_ANALYSIS_ENGINE: &str = "ui_analysis_engine_v1";
pub(crate) const SETTING_UI_SIDEBAR_COLLAPSED: &str = "ui_sidebar_collapsed_v1";
pub(crate) const SETTING_UI_HELP_SEEN: &str = "ui_help_seen_v1";
pub(crate) const SETTING_UI_USB_RECENT_ROOTS: &str = "ui_usb_recent_roots_v1";
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

fn browse_path_matches_root(file_path: &str, root: &str) -> bool {
    let file_key = browse_path_key(file_path);
    let root_key = browse_path_key(root).trim_end_matches('/').to_string();
    !root_key.is_empty() && (file_key == root_key || file_key.starts_with(&format!("{root_key}/")))
}

fn track_has_core_analysis_for_source_status(track: &Track) -> bool {
    let has_waveform_path = track
        .waveform_peaks_path
        .as_deref()
        .map(|path| !path.trim().is_empty())
        .unwrap_or(false);
    let has_bpm = track.bpm.map(|bpm| bpm > 0.0).unwrap_or(false);
    let has_duration = track
        .duration_ms
        .map(|duration| duration > 0)
        .unwrap_or(false);
    has_waveform_path && has_bpm && has_duration
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
}

impl BackendService {
    pub fn new(data_dir: impl AsRef<std::path::Path>) -> BackendResult<Self> {
        let svc = Self {
            db: Db::new(data_dir)?,
        };
        svc.backfill_track_fingerprints()?;
        Ok(svc)
    }

    pub fn get_system_parallelism(&self) -> BackendResult<GetSystemParallelismData> {
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .max(1);
        Ok(GetSystemParallelismData { workers })
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
        if req.source_roots.is_empty() {
            return Err(BackendError::Validation(
                "sourceRoots must contain at least one path".to_string(),
            ));
        }

        let scanned = scan_audio_files(&req.source_roots)?;
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
        for root in &req.source_roots {
            let escaped_root = root.replace('%', "\\%").replace('_', "\\_");
            let like = format!("{escaped_root}%");
            let mut stmt =
                tx.prepare("SELECT id, file_path FROM tracks WHERE file_path LIKE ?1 ESCAPE '\\'")?;
            let rows = stmt.query_map(params![like], |row| {
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
                if twoex.is_file() {
                    if let Ok(bytes) = std::fs::read(&twoex) {
                        if !bytes.windows(4).any(|w| w == b"PWV6") {
                            tx.execute(
                                "UPDATE tracks SET waveform_peaks_path = NULL WHERE id = ?1",
                                params![id],
                            )?;
                            for ext in ["DAT", "EXT", "2EX"] {
                                let _ =
                                    std::fs::remove_file(waveform_dir.join(format!("{id}.{ext}")));
                            }
                        }
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
            not_found: vec![],
            warnings: vec![],
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
                        eprintln!(
                            "[scan_master_db] ANLZ not found (AnalysisDataPath={anlz_rel:?})"
                        );
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
                            eprintln!("[scan_master_db] artwork not found (ImagePath={img_rel:?})");
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
                                    eprintln!(
                                        "[scan_master_db] artwork copy failed {src:?} -> {dest:?}: {e}"
                                    );
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
                        track_id
                    ],
                )?;
                updated += 1;
            } else {
                tx.execute(
                    r#"INSERT INTO tracks (
                        id, title, artist, album, bpm, tonality, file_path,
                        duration_ms, waveform_peaks_path, artwork_path, match_fingerprint,
                        master_db_source, created_at, updated_at
                       ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,1,?12,?12)"#,
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

        let mut warnings = Vec::<String>::new();
        if let Some(p) = sample_anlz {
            warnings.push(format!("AnalysisDataPath sample: {p}"));
        }
        if let Some(p) = sample_img {
            warnings.push(format!("ImagePath sample: {p}"));
        }
        if anlz_null > 0 {
            warnings.push(format!("{anlz_null} track(s) have no AnalysisDataPath"));
        }
        if anlz_miss > 0 {
            warnings.push(format!("{anlz_miss} ANLZ path(s) not found on disk"));
        }
        if anlz_ok > 0 {
            warnings.push(format!("{anlz_ok} ANLZ path(s) resolved OK"));
        }
        if artwork_null > 0 {
            warnings.push(format!("{artwork_null} track(s) have no ImagePath"));
        }
        if artwork_miss > 0 {
            warnings.push(format!(
                "{artwork_miss} artwork source file(s) not found or copy failed"
            ));
        }
        if artwork_ok > 0 {
            warnings.push(format!("{artwork_ok} artwork file(s) copied OK"));
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
        let limit = req.limit.max(1).min(TRACK_QUERY_LIMIT_MAX);
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
        Ok(SearchTracksData {
            total,
            items,
            next_cursor,
            has_more,
        })
    }

    pub fn list_tracks(&self, req: ListTracksRequest) -> BackendResult<ListTracksData> {
        let limit = req.limit.max(1).min(TRACK_QUERY_LIMIT_MAX);
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
        Ok(ListTracksData {
            total: total as usize,
            items,
            next_cursor,
            has_more,
        })
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
            });
        }

        let limit = req.limit.max(1).min(5000);
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

        let scanned = if source_roots.is_empty() {
            Vec::new()
        } else {
            scan_audio_files(&source_roots)?
        };
        let conn = self.db.connect()?;
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
                    }
                }
            })
            .collect::<Vec<_>>();

        let source_root_analysis = source_roots
            .iter()
            .map(|root| {
                let mut total = 0usize;
                let mut analyzed = 0usize;
                for track in &items {
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

        if !query.is_empty() {
            items.retain(|track| {
                let haystack = format!(
                    "{} {} {}",
                    track.title,
                    track.artist,
                    track.album.clone().unwrap_or_default()
                )
                .to_lowercase();
                haystack.contains(&query)
            });
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
        Ok(BrowseSourceFilesData {
            total,
            items: page_items,
            next_cursor,
            has_more,
            source_root_analysis,
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
        let format_ext = req.format_ext.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
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
            let like = format!("{escaped}/%");
            let deleted = tx.execute(
                r#"
                DELETE FROM tracks
                WHERE file_path = ?1
                   OR file_path LIKE ?2 ESCAPE '\'
                "#,
                params![root, like],
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

        Ok(GetTracksByIdsData { items: found })
    }

    pub fn resolve_playback_source(
        &self,
        req: ResolvePlaybackSourceRequest,
    ) -> BackendResult<ResolvePlaybackSourceData> {
        let title = req.title.trim();
        let artist = req.artist.trim();
        if title.is_empty() || artist.is_empty() {
            return Ok(ResolvePlaybackSourceData {
                resolved_path: None,
                matched_by: "none".to_string(),
                track_id: None,
            });
        }

        let conn = self.db.connect()?;
        let fingerprint = build_track_match_fingerprint(title, artist, req.album.as_deref());
        let mut stmt = conn.prepare(
            &format!("SELECT {TRACK_COLS} FROM tracks WHERE match_fingerprint = ?1 ORDER BY updated_at DESC LIMIT 64"),
        )?;
        let rows = stmt.query_map(params![fingerprint], |row| row_to_track(row, false))?;
        let candidates = rows.collect::<Result<Vec<_>, _>>()?;

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
        let candidates = rows.collect::<Result<Vec<_>, _>>()?;
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
        let mut stmt = conn.prepare(
            r#"
            SELECT t.id, t.title, t.artist, t.album, t.track_number, t.bpm, t.tonality, t.file_path,
                   t.file_size_bytes, t.format_ext, t.sample_rate_hz, t.bit_depth, t.bitrate_kbps, t.duration_ms,
                   t.artwork_path, t.waveform_peaks_path, t.bpm_analyzer, t.created_at, t.updated_at,
                   t.wav_extensible_kind
            FROM playlist_tracks pt
            JOIN tracks t ON t.id = pt.track_id
            WHERE pt.playlist_id = ?1
            ORDER BY pt.position ASC
            "#,
        )?;

        let rows = stmt.query_map(params![req.playlist_id], |row| row_to_track(row, true))?;
        let items = rows.collect::<Result<Vec<_>, _>>()?;

        Ok(GetPlaylistTracksData {
            playlist_id: req.playlist_id,
            items,
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
        SETTING_UI_ANALYSIS_BPM_RANGE,
        SETTING_UI_ANALYSIS_ENGINE,
        SETTING_UI_SIDEBAR_COLLAPSED,
        SETTING_UI_HELP_SEEN,
        SETTING_UI_USB_RECENT_ROOTS,
    ]
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

    Ok(Track {
        id: row.get(0)?,
        title: row.get(1)?,
        artist: row.get(2)?,
        album: row.get(3)?,
        track_number: row.get(4)?,
        bpm: row.get(5)?,
        bpm_analyzer,
        key: row.get(6)?,
        file_path: row.get(7)?,
        file_size_bytes: row.get(8)?,
        format_ext: row.get(9)?,
        sample_rate_hz: row.get(10)?,
        bit_depth: row.get(11)?,
        bitrate_kbps: row.get(12)?,
        // Looked up by name (not position) since callers select the tracks
        // columns with varying shapes; some queries omit this column.
        wav_extensible_kind: row.get("wav_extensible_kind").unwrap_or(None),
        duration_ms: row.get(13)?,
        artwork_path,
        artwork_data_url,
        waveform_peaks_path,
        waveform_preview,
        waveform_color_data,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
        master_db_source: is_master_db,
    })
}

pub(crate) fn build_track_match_fingerprint(
    title: &str,
    artist: &str,
    album: Option<&str>,
) -> String {
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
    if let (Some(a), Some(b)) = (req.bpm, track.bpm) {
        if (a - b).abs() <= 0.15 {
            score += 4;
        }
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
