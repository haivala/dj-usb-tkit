//! USB repair: detects export-data issues on a stick and applies the fixes the
//! user selects. `run` drives one repair pass; the other submodules hold the
//! detectors and apply functions it calls.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::params;

use crate::edb::{
    ExportDbPlaylist, find_key_id_by_name, open_edb_from_usb_root, open_edb_rw,
    try_read_playlists_with_metadata_from_edb_with_conn,
};
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{
    DiagOverviewItem, DiagStatus, GetUsbPlayerMenuConfigData, GetUsbPlayerMenuConfigRequest,
    RepairFixProposal, RepairUnsupportedItem, RepairUsbDiagnosticsData,
    RepairUsbDiagnosticsRequest, RunUsbDiagnosticsRequest, RunUsbParityReportRequest,
    UpdateUsbPlayerMenuConfigData, UpdateUsbPlayerMenuConfigRequest, UsbParityPlaylistDetail,
    UsbPlayerMenuDivergence, UsbPlayerMenuEdit, UsbPlayerMenuItem, UsbPlayerMenuItemOrigin,
    WarningEntry,
};
use crate::pdb_page;
use crate::pdb_reader::{ParsedPdb, parse_pdb, pdb_page_size};

use super::BackendService;
use super::SETTING_EXPORT_MASTER_DB_ID;
use super::analysis::{build_waveform_preview_from_audio, detect_track_duration_ms};
use super::anlz::{
    AnlzAnalysisEdits, AnlzBundlePaths, GridUpgrade, WaveformData, analysis_bundle_paths_for_track,
    apply_analysis_edits_to_anlz, atomic_write_bytes, ensure_ppth_chunk, fill_mp3_pvbr,
    has_misplaced_pqtz_header, has_pvb2_chunk, pqt2_checksum_ok, pvbr_total_samples,
    read_beatgrid_tempo_from_anlz, read_first_beat_from_anlz, upgrade_bundle_beat_grid,
    with_seek_index, write_generated_anlz_bundle,
};
use super::anlz_seek::{SEEK_SKIP_REPORT_HINT, seek_index_for_audio};
use super::cues::collapse_anlz_cues;
use super::export::APP_CONTENT_LINK_ID;
use super::export_helpers::pdb_encoding::ARTIST_SHORT_ASCII_NAME_OFFSET;
use super::export_helpers::{
    EdbPlaylistWrite, ExportManifest, ExportManifestTrack, ExportPlaylistData, PdbLayoutProfile,
    PdbPlaylistWrite, PdbTrackRemoval, PdbTrackRowData, build_pdb_for_playlists, encode_album_row,
    encode_artist_row, load_table_columns, remove_tracks_from_pdb, t19_property_from_conn,
    table_exists, usb_relative_path, write_edb_playlists_in_tx,
};
use super::track_matching::{
    ContentFingerprint, content_fingerprint_key, matching_file_name, same_file_bytes,
};
use super::usb_staging;
use super::usb_utils::{
    canonicalize_playlist_name, resolve_usb_root, resolve_usb_side_path, sanitize_warning_path,
    scan_anlz,
};
#[cfg(test)]
use super::usb_vendor_compat::vendor_pdb_path;
use super::usb_vendor_compat::{USB_ANALYSIS_DIR, USB_VENDOR_ROOT_DIR};
use crate::audio_format::same_format_extension;
use crate::scanner::scan_audio_file;
use crate::utils::{counted, plural};

use super::diagnostics::{
    AnalysisOwnership, BpmKeyMismatch, audio_path_key, build_meta_key, collect_edb_audio_paths,
    detect_bpm_key_mismatches, normalize_analysis_path_for_identity,
    normalize_pdb_path_for_edb_lookup, scan_analysis_ownership, scan_audio_presence,
    track_identity_key,
};

mod audio_fixes;
mod history;
mod overview;
mod pdb_fixes;
mod player_menu;
mod run;
mod strict_parity;
#[cfg(test)]
mod tests;

use audio_fixes::*;
use history::*;
use overview::*;
pub(crate) use pdb_fixes::*;
pub(crate) use player_menu::*;
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
const PDB_SENTINEL_U5_FIX_ID: &str = "repair_pdb_sentinel_u5_on_data_pages";
const PDB_WRONG_PAGE_FLAGS_FIX_ID: &str = "repair_pdb_wrong_page_flags";
const PDB_ZERO_TRANRF_FIX_ID: &str = "repair_pdb_zero_tranrf_on_track_pages";
const PDB_WRONG_TRACK_U5_FIX_ID: &str = "repair_pdb_wrong_track_u5_num_rl";
const PDB_WRONG_HISTORY_SHAPE_FIX_ID: &str = "repair_pdb_wrong_history_page_shape";
const PDB_STALE_SENTINEL_BTREE_FIX_ID: &str = "repair_pdb_stale_sentinel_btree";
const PDB_WRONG_PLAYLIST_TREE_SHAPE_FIX_ID: &str = "repair_pdb_wrong_playlist_tree_shape";
const PDB_TOMBSTONED_PLAYLIST_TREE_ID_FIX_ID: &str = "repair_pdb_tombstoned_playlist_tree_ids";
const PDB_T00_MULTIPAGE_ACTIVE_FIX_ID: &str = "repair_pdb_t00_multipage_active_pages";
const PDB_EC_CONFLICT_FIX_ID: &str = "repair_pdb_ec_data_page_conflict";
const PDB_TORN_GROWTH_PAGES_FIX_ID: &str = "repair_pdb_torn_growth_pages";
const PDB_TRUNCATED_TABLE_CHAIN_FIX_ID: &str = "repair_pdb_truncated_table_chain";
const PDB_T19_TEMPLATE_STATE_FIX_ID: &str = "repair_pdb_t19_template_state";

/// Structural-prerequisite fixes the strict-parity upgrade needs to have run
/// first -- surfaced to the editor as `RepairFixProposal.always_applied` so it
/// can lock their checkbox. Kept in lockstep with `REPAIR_FIX_DISPLAY_ORDER`
/// and the apply block.
fn fix_always_applied(fix_id: &str) -> bool {
    matches!(
        fix_id,
        PDB_TRUNCATED_TABLE_CHAIN_FIX_ID | PDB_TORN_GROWTH_PAGES_FIX_ID
    )
}

/// Page shapes real rekordbox exports carry as transaction bookkeeping and
/// that players and rekordbox read fine, so they are never reported as
/// issues. Our writer never produces them, so their fixes still normalize
/// them whenever a repair runs. Surfaced as `RepairFixProposal.bookkeeping`.
fn fix_is_bookkeeping(fix_id: &str) -> bool {
    matches!(
        fix_id,
        PDB_ZERO_TRANRF_FIX_ID
            | PDB_WRONG_PAGE_FLAGS_FIX_ID
            | PDB_SENTINEL_U5_FIX_ID
            | PDB_WRONG_TRACK_U5_FIX_ID
            | PDB_T00_MULTIPAGE_ACTIVE_FIX_ID
            | PDB_TOMBSTONED_PLAYLIST_TREE_ID_FIX_ID
            | PDB_WRONG_PLAYLIST_TREE_SHAPE_FIX_ID
            | PDB_STALE_SENTINEL_BTREE_FIX_ID
    )
}
const PDB_TRACK_STRING_ALIGNMENT_FIX_ID: &str = "repair_pdb_track_string_alignment";
const RELINK_MOVED_AUDIO_FIX_ID: &str = "relink_moved_audio";
const UNINDEXED_AUDIO_PLAYLIST_FIX_ID: &str = "add_unindexed_audio_playlist";
const DUPLICATE_AUDIO_COPIES_FIX_ID: &str = "remove_duplicate_audio_copies";
const ENGINE_LIBRARY_FIX_ID: &str = "keep_engine_library_up_to_date";
/// USB playlist the unindexed-audio fix collects its files into.
const UNINDEXED_AUDIO_PLAYLIST_NAME: &str = "Unindexed";
const PDB_ALBUM_STRING_ALIGNMENT_FIX_ID: &str = "repair_pdb_album_string_alignment";
const PDB_ARTIST_NAME_LAYOUT_FIX_ID: &str = "repair_pdb_artist_name_layout";

/// The order proposed fixes are shown in (and, for the first four entries,
/// the order they're actually applied in — see the apply block below). Must
/// be kept in sync with that block by hand: truncated-table-chain and
/// torn-growth-pages are structural prerequisites that always run before
/// strict parity, so they must also always be displayed first.
///
/// If a future fix is likewise a structural prerequisite for strict parity
/// (must physically run before it, not just "happens to run early"), three
/// places need updating together: its position in the apply block below, its
/// position in this array, and `fix_always_applied` above — which sets
/// `RepairFixProposal.always_applied` so the editor locks that fix's checkbox
/// checked (a user can't deselect a prerequisite while leaving strict parity
/// selected).
const REPAIR_FIX_DISPLAY_ORDER: &[&str] = &[
    EMPTY_ANALYSIS_FILES_FIX_ID,
    PDB_TRUNCATED_TABLE_CHAIN_FIX_ID,
    PDB_TORN_GROWTH_PAGES_FIX_ID,
    PDB_T19_TEMPLATE_STATE_FIX_ID,
    STRICT_PARITY_UPGRADE_FIX_ID,
    PDB_DUPLICATE_PLAYLIST_ENTRIES_FIX_ID,
    PDB_ZERO_TRANRF_FIX_ID,
    PDB_WRONG_PAGE_FLAGS_FIX_ID,
    PDB_SENTINEL_U5_FIX_ID,
    PDB_WRONG_HISTORY_SHAPE_FIX_ID,
    PDB_WRONG_TRACK_U5_FIX_ID,
    PDB_T00_MULTIPAGE_ACTIVE_FIX_ID,
    PDB_TOMBSTONED_PLAYLIST_TREE_ID_FIX_ID,
    PDB_WRONG_PLAYLIST_TREE_SHAPE_FIX_ID,
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

/// Reuses a caller-supplied eDB connection if given, otherwise calls
/// `open_fallback` to open (and own) one of its own. Centralizes the
/// "reuse the shared connection, or open a local fallback" pattern that
/// would otherwise be duplicated across every repair apply-fix helper
/// below -- each of them previously re-opened the eDB independently even
/// when a repair run had already opened one, redoing SQLCipher key
/// negotiation for no reason.
enum EdbConnHandle<'a> {
    Shared(&'a mut rusqlite::Connection),
    Owned(rusqlite::Connection),
}

impl<'a> EdbConnHandle<'a> {
    fn acquire(
        shared: Option<&'a mut rusqlite::Connection>,
        open_fallback: impl FnOnce() -> Option<rusqlite::Connection>,
    ) -> Option<Self> {
        match shared {
            Some(conn) => Some(Self::Shared(conn)),
            None => open_fallback().map(Self::Owned),
        }
    }

    fn conn(&mut self) -> &mut rusqlite::Connection {
        match self {
            Self::Shared(conn) => conn,
            Self::Owned(conn) => conn,
        }
    }
}
