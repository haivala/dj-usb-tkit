//! USB repair: detects export-data issues on a stick and applies the fixes the
//! user selects. `run` drives one repair pass; the other submodules hold the
//! detectors and apply functions it calls.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::params;

use crate::edb::{
    EdbRead, ExportDbPlaylist, find_key_id_by_name, open_edb_from_usb_root, read_edb,
};
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{
    DiagOverviewItem, DiagStatus, GetUsbPlayerMenuConfigData, GetUsbPlayerMenuConfigRequest,
    RepairFixProposal, RepairNameInput, RepairRow, RepairUnsupportedItem, RepairUsbDiagnosticsData,
    RepairUsbDiagnosticsRequest, RunUsbDiagnosticsRequest, RunUsbParityReportRequest,
    UpdateUsbPlayerMenuConfigData, UpdateUsbPlayerMenuConfigRequest, UsbParityPlaylistDetail,
    UsbPlayerMenuEdit, UsbPlayerMenuItem, WarningEntry,
};
use crate::pdb_page;
use crate::pdb_reader::{ParsedPdb, parse_pdb, pdb_page_size};

use super::BackendService;
use super::SETTING_EXPORT_MASTER_DB_ID;
use super::analysis::{build_waveform_preview_from_audio, detect_track_duration_ms};
use super::anlz::{
    AnlzAnalysisEdits, AnlzBundlePaths, GridUpgrade, WaveformData, analysis_bundle_paths_for_track,
    apply_analysis_edits_to_anlz, atomic_write_bytes, ensure_ppth_chunk, fill_mp3_pvbr,
    has_pvb2_chunk, pvbr_total_samples, read_anlz_chunks, read_beatgrid_tempo_from_anlz,
    read_first_beat_from_anlz, upgrade_bundle_beat_grid, with_seek_index,
    write_generated_anlz_bundle,
};
use super::anlz_seek::{SeekIndexSkip, seek_index_for_audio, seek_index_for_audio_reporting};
use super::cues::collapse_anlz_cues;
use super::export::APP_CONTENT_LINK_ID;
use super::export_helpers::pdb_encoding::ARTIST_SHORT_ASCII_NAME_OFFSET;
use super::export_helpers::{
    EdbPlaylistWrite, ExportManifest, ExportManifestTrack, ExportPlaylistData, PdbLayoutProfile,
    PdbPlaylistWrite, PdbTrackRemoval, PdbTrackRowData, PlaylistPlacement, build_pdb_for_playlists,
    encode_album_row, encode_artist_row, load_table_columns, remove_tracks_from_pdb,
    sync_property_content_count, t19_property_from_conn, table_exists, usb_relative_path,
    write_edb_playlists_in_tx,
};
use super::track_matching::{
    ContentFingerprint, content_fingerprint_key, matching_file_name, pdb_track_fingerprint,
    same_file_bytes,
};
use super::usb_display_files::SeekSkipMemory;
use super::usb_staging::{self, EdbRw};
use super::usb_utils::{
    canonicalize_playlist_name, resolve_usb_root, resolve_usb_side_path, sanitize_warning_path,
    scan_anlz,
};
#[cfg(test)]
use super::usb_vendor_compat::vendor_pdb_path;
use crate::audio_format::same_format_extension;
use crate::scanner::scan_audio_file;
use crate::utils::counted;

use super::diagnostics::{
    AnalysisOwnership, BpmKeyMismatch, audio_path_key, build_meta_key, collect_edb_audio_paths,
    detect_bpm_key_mismatches, normalize_analysis_path_for_identity,
    normalize_pdb_path_for_edb_lookup, scan_analysis_ownership, scan_audio_presence,
    track_identity_key,
};

mod audio_fixes;
mod background_color;
mod damaged_playlists;
mod damaged_tracks;
mod history;
mod overview;
mod pdb_fixes;
mod player_menu;
mod rebuild_tracks;
mod run;
mod strict_parity;
mod write_gate;
pub use write_gate::{pdb_shape_problems, pdb_write_problems};
#[cfg(test)]
mod tests;

use audio_fixes::*;
use damaged_playlists::*;
use damaged_tracks::*;
use history::*;
use overview::*;
pub(crate) use pdb_fixes::*;
use rebuild_tracks::*;
pub(crate) use strict_parity::*;

const EMPTY_ANALYSIS_FILES_FIX_ID: &str = "fix_empty_analysis_files";
const REMOVE_MISSING_AUDIO_REFERENCES_FIX_ID: &str = "remove_missing_audio_references";
const BPM_KEY_MISMATCH_FIX_ID: &str = "fix_bpm_key_mismatch";
const SHARED_ANALYSIS_FIX_ID: &str = "separate_shared_analysis";
const ANALYSIS_TRACK_PATH_FIX_ID: &str = "add_analysis_track_path";
const MISSING_MP3_SEEK_DATA_FIX_ID: &str = "add_missing_mp3_seek_data";
const MISSING_FLAC_SEEK_DATA_FIX_ID: &str = "add_missing_flac_seek_data";
const BEAT_GRID_HEADER_FIX_ID: &str = "fix_beat_grid_header";
const STRICT_PARITY_UPGRADE_FIX_ID: &str = "upgrade_export_data_to_strict_parity";
const PDB_DUPLICATE_PLAYLIST_ENTRIES_FIX_ID: &str = "repair_pdb_duplicate_playlist_entries";
const SYNC_EDB_HISTORY_FROM_PDB_FIX_ID: &str = "sync_edb_history_from_pdb";
const PDB_HEADER_COMPATIBILITY_FIX_ID: &str = "repair_pdb_header_compatibility_field";
const PDB_HEADER_COMPATIBILITY_FALLBACK_VALUE: u32 = 5;
const PDB_DAMAGED_PAGE_HEADERS_FIX_ID: &str = "repair_pdb_damaged_page_headers";
const PDB_STALE_SENTINEL_BTREE_FIX_ID: &str = "repair_pdb_stale_sentinel_btree";
const PDB_INCONSISTENT_FOOTERS_FIX_ID: &str = "repair_pdb_page_footer_transaction";
const PDB_EC_CONFLICT_FIX_ID: &str = "repair_pdb_ec_data_page_conflict";
const PDB_TORN_GROWTH_PAGES_FIX_ID: &str = "repair_pdb_torn_growth_pages";
const PDB_TRUNCATED_TABLE_CHAIN_FIX_ID: &str = "repair_pdb_truncated_table_chain";
const PDB_EMPTY_DUPLICATE_PLAYLISTS_FIX_ID: &str = "remove_pdb_empty_duplicate_playlists";
const PDB_T19_TEMPLATE_STATE_FIX_ID: &str = "repair_pdb_t19_template_state";
const DAMAGED_TRACK_ROWS_FIX_ID: &str = "merge_damaged_track_rows";
const REBUILD_DAMAGED_TRACK_ROWS_FIX_ID: &str = "rebuild_damaged_track_rows";
const DAMAGED_PLAYLISTS_FIX_ID: &str = "repair_damaged_playlist_rows";

/// Fixes for damage a USB can't be used with: broken page structure and rows
/// a crashed rekordbox write left behind. Not optional: every apply runs
/// them, before any optional fix, whatever was selected, and the editor
/// locks their checkbox (`RepairFixProposal.always_applied`). Strict parity,
/// including rebuilding a missing database, then works on repaired rows.
/// Kept in lockstep with `REPAIR_FIX_DISPLAY_ORDER` and the apply block.
fn fix_always_applied(fix_id: &str) -> bool {
    matches!(
        fix_id,
        PDB_TRUNCATED_TABLE_CHAIN_FIX_ID
            | PDB_TORN_GROWTH_PAGES_FIX_ID
            | PDB_DAMAGED_PAGE_HEADERS_FIX_ID
            | DAMAGED_TRACK_ROWS_FIX_ID
            | REBUILD_DAMAGED_TRACK_ROWS_FIX_ID
            | DAMAGED_PLAYLISTS_FIX_ID
            | PDB_EMPTY_DUPLICATE_PLAYLISTS_FIX_ID
    )
}

/// Page state real rekordbox exports carry and that players and rekordbox
/// read fine, so it is never reported as an issue; the fix normalizes it
/// whenever a repair runs. Surfaced as `RepairFixProposal.bookkeeping`.
/// rekordbox's own page footers and flags are not on this list: rewriting
/// them made rekordbox 6 crash (docs/PDB.md "Page Footer Conventions").
fn fix_is_bookkeeping(fix_id: &str) -> bool {
    matches!(fix_id, PDB_STALE_SENTINEL_BTREE_FIX_ID)
}
const PDB_TRACK_STRING_ALIGNMENT_FIX_ID: &str = "repair_pdb_track_string_alignment";
const RELINK_MOVED_AUDIO_FIX_ID: &str = "relink_moved_audio";
const UNINDEXED_AUDIO_PLAYLIST_FIX_ID: &str = "add_unindexed_audio_playlist";
const DUPLICATE_AUDIO_COPIES_FIX_ID: &str = "remove_duplicate_audio_copies";
const ENGINE_LIBRARY_FIX_ID: &str = "keep_engine_library_up_to_date";
const PDB_ALBUM_STRING_ALIGNMENT_FIX_ID: &str = "repair_pdb_album_string_alignment";
const PDB_ARTIST_NAME_LAYOUT_FIX_ID: &str = "repair_pdb_artist_name_layout";

/// The order proposed fixes are shown and applied in (see the apply block in
/// `run.rs`, kept in sync by hand). The `fix_always_applied` fixes come
/// first, in the order they must run: the page structure before anything
/// reads rows (torn growth before anything grows the file, since its
/// truncation boundary was worked out on the file as it was), then damaged
/// track rows, damaged playlist rows, then empty duplicate playlists. Every optional
/// fix comes after them.
const REPAIR_FIX_DISPLAY_ORDER: &[&str] = &[
    PDB_TRUNCATED_TABLE_CHAIN_FIX_ID,
    PDB_TORN_GROWTH_PAGES_FIX_ID,
    PDB_DAMAGED_PAGE_HEADERS_FIX_ID,
    DAMAGED_TRACK_ROWS_FIX_ID,
    REBUILD_DAMAGED_TRACK_ROWS_FIX_ID,
    DAMAGED_PLAYLISTS_FIX_ID,
    PDB_EMPTY_DUPLICATE_PLAYLISTS_FIX_ID,
    EMPTY_ANALYSIS_FILES_FIX_ID,
    PDB_T19_TEMPLATE_STATE_FIX_ID,
    STRICT_PARITY_UPGRADE_FIX_ID,
    PDB_DUPLICATE_PLAYLIST_ENTRIES_FIX_ID,
    PDB_INCONSISTENT_FOOTERS_FIX_ID,
    PDB_STALE_SENTINEL_BTREE_FIX_ID,
    PDB_EC_CONFLICT_FIX_ID,
    PDB_TRACK_STRING_ALIGNMENT_FIX_ID,
    PDB_ALBUM_STRING_ALIGNMENT_FIX_ID,
    PDB_ARTIST_NAME_LAYOUT_FIX_ID,
    PDB_HEADER_COMPATIBILITY_FIX_ID,
    // Before the BPM fix: it only aligns a bundle that is one track's own.
    SHARED_ANALYSIS_FIX_ID,
    ANALYSIS_TRACK_PATH_FIX_ID,
    BPM_KEY_MISMATCH_FIX_ID,
    BEAT_GRID_HEADER_FIX_ID,
    MISSING_MP3_SEEK_DATA_FIX_ID,
    MISSING_FLAC_SEEK_DATA_FIX_ID,
    RELINK_MOVED_AUDIO_FIX_ID,
    UNINDEXED_AUDIO_PLAYLIST_FIX_ID,
    DUPLICATE_AUDIO_COPIES_FIX_ID,
    REMOVE_MISSING_AUDIO_REFERENCES_FIX_ID,
    SYNC_EDB_HISTORY_FROM_PDB_FIX_ID,
    // Last: it rebuilds from the PDB the fixes above leave behind.
    ENGINE_LIBRARY_FIX_ID,
];

fn strict_repair_force_all_enabled() -> bool {
    std::env::var("STRICT_REPAIR_ALL")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}
