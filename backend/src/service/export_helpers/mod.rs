//! Export helpers: the shared export types plus submodules for PDB writing,
//! eDB writing, PDB row encoding, player menu tables, paths and playlist ops.

pub mod export_paths;
pub mod pdb_encoding;
pub mod pdb_menu;
pub mod playlist_ops;

mod edb_export;
mod pdb_export;
#[cfg(test)]
mod tests;

pub use edb_export::*;
pub use pdb_export::*;

// Re-export submodule items at this level for backward compatibility.
pub use crate::edb::{
    ExportManifestTrack, ExportPlaylistData, ExportTrackData, content_file_name,
    duration_ms_to_seconds, dynamic_insert, find_content_id_by_path, find_key_id_by_name,
    find_or_insert_album, find_or_insert_artist, find_or_insert_image,
    insert_content_from_template, link_playlist_content, load_table_columns,
    load_table_row_template, map_values_for_insert, next_numeric_id, normalize_export_date,
    open_edb_rw, populate_content_values_map, preferred_export_playlist_row_id,
    replace_export_playlist_row_with_identity, resolve_track_created_date_for_export,
    resolve_track_release_date_for_export, table_exists, upsert_export_playlist_row,
};
pub(crate) use crate::metadata::sanitize_metadata;
pub use crate::pdb_writer::{T08EntryKey, validate_no_empty_data_pages};
#[cfg(test)]
pub use export_paths::canonical_artwork_target_path;
pub use export_paths::{
    CONTENT_FILENAME_MAX_LEN, PruneResult, analysis_bundle_path_variants,
    canonical_analysis_bundle_paths, canonical_artwork_target_paths, collect_manifest_owned_paths,
    copy_if_different, copy_wav_normalized_if_needed, ensure_analysis_bundle_ppth,
    export_analysis_bundle_for_track, export_artwork_for_player, export_owned_files_setting_key,
    exported_media_target_path, exported_media_target_path_with_ordinal,
    filter_prunable_stale_paths_for_playlist, is_safe_export_owned_path, limit_contents_file_name,
    limit_contents_file_name_with_suffix, normalize_owned_export_path,
    prune_stale_export_owned_files, sanitize_contents_component, sanitize_contents_file_name,
    sanitize_filename_component, stable_u32_hash, to_usb_relative_path, truncate_component,
    usb_relative_path,
};
pub use pdb_encoding::{
    PdbLayoutProfile, encode_album_row, encode_artist_row, encode_artwork_row, encode_key_row,
    encode_pdb_string, encode_pdb_track_isrc_slot, encode_playlist_entry_row,
    encode_playlist_tree_row, encode_track_row_with_profile,
};
pub use pdb_menu::{
    PdbT16Row, encode_pdb_t16_row, encode_pdb_t17_cat_row, inspect_pdb_columns_playlist_order,
    load_pdb_t16_decoded, load_pdb_t16_raw, load_pdb_t16_rows_from_bytes,
    patch_pdb_columns_menu_set_by_kind, patch_pdb_t17_category_snapshot,
};
pub use playlist_ops::{
    MirrorOrphanCleanup, remove_orphaned_dropped_tracks_from_dbs,
    remove_playlist_and_tracks_from_pdb, remove_playlist_from_edb,
    remove_track_ids_from_pdb_playlist_entries, verify_edb_content, verify_edb_content_with_conn,
    verify_pdb_content,
};

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::error::{BackendError, BackendResult};
use crate::models::{ExportToUsbOptions, TrackCue, WarningEntry};
use crate::pdb_reader::parse_pdb;
use crate::service::cues::split_cues;
use crate::utils::{collect_chain as collect_chain_pages, page_offset, table_ptr_fields};

use super::usb_utils::{canonicalize_playlist_name, repair_utf8_mojibake};
use super::usb_vendor_compat::{USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR};

pub(crate) fn canonicalize_track_path_identity(value: &str) -> String {
    let normalized = repair_utf8_mojibake(value.trim()).replace('\\', "/");
    if normalized.is_empty() {
        return String::new();
    }

    // Trim trailing whitespace from the stem so alias filenames (e.g. "Track .mp3"
    // written by some DJ software) resolve to the same key as their originals.
    let normalized = if let Some(dot) = normalized.rfind('.') {
        let stem = normalized[..dot].trim_end_matches(' ');
        format!("{stem}{}", &normalized[dot..])
    } else {
        normalized
    };

    let lower = normalized.to_ascii_lowercase();
    if let Some(idx) = lower.rfind("/contents/") {
        lower[idx..].to_string()
    } else if lower.starts_with("contents/") {
        format!("/{lower}")
    } else {
        lower
    }
}

fn canonicalize_artwork_path_lookup(value: &str) -> String {
    repair_utf8_mojibake(value.trim())
        .replace('\\', "/")
        .to_ascii_lowercase()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportManifest {
    pub version: u32,
    pub generated_at: String,
    pub playlist_id: String,
    pub playlist_name: String,
    pub usb_root: String,
    pub options: ExportToUsbOptions,
    pub exported_tracks: usize,
    pub skipped_tracks: usize,
    pub warnings: Vec<WarningEntry>,
    pub tracks: Vec<ExportManifestTrack>,
}

#[derive(Debug, Clone)]
pub struct LocalTrackForAnalysis {
    pub id: String,
    pub title: String,
    pub file_path: String,
    /// Values the analysis keeps instead of replacing them with its own
    /// detection (see `analysis::kept_analysis_values`).
    pub kept: KeptAnalysis,
    /// Existing anchor used when audio cannot establish a new one.
    pub existing_first_beat_ms: Option<u32>,
    /// A first beat the first analysis keeps: one set in the cue editor, or
    /// one imported with the kept BPM. A reanalysis keeps none (see
    /// `analysis::row_to_track_for_analysis`).
    pub kept_first_beat_ms: Option<u32>,
    /// The track already has its waveform, so this run is a reanalysis that
    /// replaces its BPM / key (see `analysis::kept_analysis_values`).
    pub reanalysis: bool,
}

/// A BPM / key that an analysis run keeps: set by the user or imported from
/// rekordbox / Mixxx, on a track being analyzed for the first time.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeptAnalysis {
    pub bpm: Option<f64>,
    pub key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LocalAnalysisResult {
    pub bpm: Option<f64>,
    pub bpm_analyzer: Option<String>,
    pub key: Option<String>,
    pub first_beat_ms: Option<u32>,
    /// Whether this analysis placed `first_beat_ms`, rather than keeping one.
    pub first_beat_estimated: bool,
    pub duration_ms: Option<u64>,
    pub artwork_path: Option<String>,
    pub waveform_peaks_path: Option<String>,
    pub waveform_preview: Option<Vec<u8>>,
}

// Re-export from anlz module
pub use super::anlz::WaveformData;

/// Result of removing a playlist from PDB with track/artwork cleanup.
#[derive(Debug, Clone)]
pub struct PlaylistRemovalPdbResult {
    pub removed_playlist_count: usize,
    pub exclusive_tracks: Vec<ExclusiveTrackInfo>,
    pub shared_track_count: usize,
}

/// Info about a track that is exclusive to the removed playlist(s).
#[derive(Debug, Clone)]
pub struct ExclusiveTrackInfo {
    pub track_file_path: String,
    pub anlz_path: String,
    pub artwork_id: u32,
}

#[derive(Debug, Clone)]
pub struct WriteExportLibraryDbResult {
    pub inserted_content: usize,
    pub linked_playlist_entries: usize,
    pub playlist_id: i64,
    pub sort_order: i64,
}

#[derive(Debug, Clone)]
pub struct WriteExportPdbResult {
    pub inserted_tracks: usize,
    pub inserted_playlists: usize,
    pub topology_issues: Vec<String>,
    /// Non-fatal warnings produced by the PDB writer (e.g. tracks dropped
    /// because their encoded row exceeded a single PDB page). Surfaced into
    /// the export job response so the UI Event Log shows them.
    #[allow(clippy::vec_init_then_push)]
    pub writer_warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PdbTrackRowData {
    pub header_flags_u32: Option<u32>,
    pub content_link: Option<u32>,
    pub sample_rate_hz: Option<u32>,
    pub file_size_bytes: Option<u32>,
    pub master_content_id: Option<u32>,
    pub master_db_id: Option<u32>,
    pub id: u32,
    pub artist_id: u32,
    pub album_id: u32,
    pub artwork_id: u32,
    pub key_id: u32,
    pub genre_id: u32,
    pub bitrate_kbps: Option<u32>,
    pub track_number: Option<u32>,
    pub bpm: Option<f64>,
    pub release_year: Option<u16>,
    pub bit_depth: Option<u16>,
    pub duration_seconds: Option<u32>,
    pub file_type: Option<u16>,
    pub isrc: Option<String>,
    pub date_added: Option<String>,
    pub release_date: Option<String>,
    pub dj_comment: Option<String>,
    pub file_name: Option<String>,
    pub publish_track_info_on: Option<bool>,
    pub autoload_hotcues_on: Option<bool>,
    /// Track colour (row byte 88; 1–8, the `colors` table ids). None/0 = no colour.
    pub color_id: Option<u8>,
    /// Star rating (row byte 89; 0–5).
    pub rating: Option<u8>,
    pub title: String,
    pub anlz_path: String,
    pub file_path: String,
}
