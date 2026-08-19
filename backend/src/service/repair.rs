//! USB repair functions: strict parity upgrade, analysis fix, missing audio cleanup.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use rusqlite::params;

use crate::edb::{
    ExportDbPlaylist, open_edb_from_usb_root, open_edb_rw,
    try_read_playlists_with_metadata_from_edb, try_read_playlists_with_metadata_from_edb_with_conn,
};
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{
    DiagCheck, DiagStatus, GetUsbPlayerMenuConfigData, GetUsbPlayerMenuConfigRequest,
    RepairFixProposal, RepairUnsupportedItem, RepairUsbDiagnosticsData,
    RepairUsbDiagnosticsRequest, RunUsbDiagnosticsRequest, RunUsbParityReportRequest,
    UpdateUsbPlayerMenuConfigData, UpdateUsbPlayerMenuConfigRequest, UsbParityPlaylistDetail,
    UsbPlayerMenuDivergence, UsbPlayerMenuItem, UsbPlayerMenuItemOrigin, WarningEntry,
};
use crate::pdb_reader::parse_pdb;

use super::BackendService;
use super::analysis::build_waveform_preview_from_audio;
use super::anlz::{AnlzBundlePaths, WaveformData, write_generated_anlz_bundle};
use super::export_helpers::{
    ExportManifest, ExportManifestTrack, ExportPlaylistData, PdbLayoutProfile, PdbTrackRowData,
    encode_album_row, load_table_columns, remove_track_ids_from_pdb_playlist_entries,
    replace_export_playlist_row_with_identity, table_exists, write_edb_playlist_with_conn,
    write_pdb,
};
use super::usb_staging;
use super::usb_utils::{
    canonicalize_playlist_name, collect_contents_audio_files, resolve_usb_root,
    resolve_usb_side_path, scan_anlz_warnings,
};
#[cfg(test)]
use super::usb_vendor_compat::vendor_pdb_path;

/// Player menu kinds that cannot be removed once present in the current menu.
/// TRACK=131, PLAYLIST=132, FOLDER=144, SEARCH=145, HISTORY=149.
const REQUIRED_PLAYER_MENU_KINDS: &[u32] = &[131, 132, 144, 145, 149];

use super::diagnostics::{
    build_meta_key, collect_edb_indexed_paths, contents_path_match_key,
    normalize_analysis_path_for_identity, normalize_path_for_contents_match,
    normalize_pdb_path_for_edb_lookup, track_identity_key,
};

const STRICT_PARITY_UPGRADE_FIX_ID: &str = "upgrade_export_data_to_strict_parity";
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
const PDB_TRACK_STRING_ALIGNMENT_FIX_ID: &str = "repair_pdb_track_string_alignment";
const PDB_ALBUM_STRING_ALIGNMENT_FIX_ID: &str = "repair_pdb_album_string_alignment";

/// The order proposed fixes are shown in (and, for the first four entries,
/// the order they're actually applied in — see the apply block below). Must
/// be kept in sync with that block by hand: truncated-table-chain and
/// torn-growth-pages are structural prerequisites that always run before
/// strict parity, so they must also always be displayed first.
///
/// If a future fix is likewise a structural prerequisite for strict parity
/// (must physically run before it, not just "happens to run early"), three
/// places need updating together: its position in the apply block below,
/// its position in this array, and `ALWAYS_APPLIED_FIX_IDS` in
/// `vanilla-ui/components/usb/actions.mjs` — the frontend locks that fix's
/// checkbox checked so a user can't deselect a prerequisite while leaving
/// strict parity selected.
const REPAIR_FIX_DISPLAY_ORDER: &[&str] = &[
    "fix_empty_analysis_files",
    PDB_TRUNCATED_TABLE_CHAIN_FIX_ID,
    PDB_TORN_GROWTH_PAGES_FIX_ID,
    STRICT_PARITY_UPGRADE_FIX_ID,
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
    PDB_HEADER_COMPATIBILITY_FIX_ID,
    "manual_reimport_unindexed_audio",
    "remove_missing_audio_references",
    SYNC_EDB_HISTORY_FROM_PDB_FIX_ID,
];

#[derive(Debug, Default, Clone)]
struct StrictParityUpgradeApplyResult {
    merged_playlists: usize,
    edb_playlists_written: usize,
    failed_playlists: usize,
    artwork_patch_incomplete: bool,
    duplicate_entries_removed: usize,
}

#[derive(Debug, Clone)]
struct PdbHeaderCompatibilityRepair {
    current_value: u32,
    target_value: u32,
}

fn pdb_header_compatibility_value_from_bytes(bytes: &[u8]) -> Option<u32> {
    bytes
        .get(0x10..0x14)
        .and_then(|raw| raw.try_into().ok())
        .map(u32::from_le_bytes)
}

fn read_pdb_header_compatibility_value(path: &Path) -> Option<u32> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| pdb_header_compatibility_value_from_bytes(&bytes))
}

fn is_known_pdb_header_compatibility_value(value: u32) -> bool {
    matches!(value, 1 | PDB_HEADER_COMPATIBILITY_FALLBACK_VALUE)
}

/// `docs/PDB.md` "File header `0x10` compatibility field" confirms both `1`
/// and `5` are accepted by every tested validator. Comparing against the
/// most recent backup snapshot's value here would cause this repair to
/// oscillate forever: the writer always emits `5` on a fresh export, so any
/// export following a prior application of this repair (which patches to
/// `1`) immediately re-flags "drift" back toward `1`, and applying it again
/// just sets up the next export to flag drift back toward `5`. Only an
/// actually-unknown value is worth repairing.
fn detect_pdb_header_compatibility_repair(
    staged_pdb_path: &Path,
) -> Option<PdbHeaderCompatibilityRepair> {
    let current_value = read_pdb_header_compatibility_value(staged_pdb_path)?;
    if is_known_pdb_header_compatibility_value(current_value) {
        return None;
    }
    Some(PdbHeaderCompatibilityRepair {
        current_value,
        target_value: PDB_HEADER_COMPATIBILITY_FALLBACK_VALUE,
    })
}

fn apply_pdb_header_compatibility_repair(
    usb_root: &Path,
    repair: &PdbHeaderCompatibilityRepair,
) -> BackendResult<bool> {
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let bytes = std::fs::read(&pdb_path)?;
    let Some(current_value) = pdb_header_compatibility_value_from_bytes(&bytes) else {
        return Err(BackendError::Validation(format!(
            "PDB is too small to patch header compatibility field: {}",
            pdb_path.display()
        )));
    };
    if current_value != repair.current_value {
        return Ok(false);
    }

    let mut file = OpenOptions::new().write(true).open(&pdb_path)?;
    file.seek(SeekFrom::Start(0x10))?;
    file.write_all(&repair.target_value.to_le_bytes())?;
    file.sync_data()?;
    Ok(true)
}

fn strict_repair_force_all_enabled() -> bool {
    std::env::var("STRICT_REPAIR_ALL")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// A data page whose `u5` field at offset 0x20 carries the sentinel value
/// 0x1FFF.  This makes player firmware treat the page as an empty fence
/// page and report the database as corrupted.
#[derive(Debug, Clone)]
pub(super) struct SentinelU5Page {
    page_index: usize,
    current_num_rl: u16,
    correct_u5: u16,
    correct_num_rl: u16,
}

pub(super) fn detect_pdb_sentinel_u5_on_data_pages(pdb_path: &Path) -> Vec<SentinelU5Page> {
    use crate::pdb_writer::data_page_footer_fields;
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};
    let Ok(bytes) = std::fs::read(pdb_path) else {
        return Vec::new();
    };
    let Some(page_size) = read_u32_le_at(&bytes, 4).map(|v| v as usize) else {
        return Vec::new();
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return Vec::new();
    }
    let total = bytes.len() / page_size;
    let mut out = Vec::new();
    for i in 1..total {
        let off = i * page_size;
        let Some(idx) = read_u32_le_at(&bytes, off + 4) else {
            continue;
        };
        if idx == 0 {
            continue; // blank/zeroed page
        }
        let Some(used_s) = read_u16_le_at(&bytes, off + 0x1e) else {
            continue;
        };
        if used_s == 0 {
            continue; // sentinel/fence page — u5=8191 is expected here
        }
        let Some(u5) = read_u16_le_at(&bytes, off + 0x20) else {
            continue;
        };
        if u5 != 0x1FFF {
            continue;
        }
        let table_type = read_u32_le_at(&bytes, off + 8).unwrap_or(9999);
        let nrs = read_u8_at(&bytes, off + 24).unwrap_or(0);
        let current_num_rl = read_u16_le_at(&bytes, off + 0x22).unwrap_or(0);
        let (correct_u5, correct_num_rl) = data_page_footer_fields(table_type, nrs as u16);
        out.push(SentinelU5Page {
            page_index: i,
            current_num_rl,
            correct_u5,
            correct_num_rl,
        });
    }
    out
}

fn apply_pdb_sentinel_u5_repair(usb_root: &Path, pages: &[SentinelU5Page]) -> BackendResult<usize> {
    if pages.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let Some(page_size) = bytes
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
    else {
        return Err(BackendError::Validation(
            "PDB too small to read page size".into(),
        ));
    };
    let mut patched = 0usize;
    for page in pages {
        let off = page.page_index * page_size;
        if off + 0x24 > bytes.len() {
            continue;
        }
        bytes[off + 0x20..off + 0x22].copy_from_slice(&page.correct_u5.to_le_bytes());
        // Only patch num_rl if it also carries the sentinel value; leave a
        // correct existing num_rl alone.
        if page.current_num_rl == 0x1FFF {
            bytes[off + 0x22..off + 0x24].copy_from_slice(&page.correct_num_rl.to_le_bytes());
        }
        patched += 1;
    }
    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

fn expected_page_flags(table_type: u32) -> u8 {
    // Both 0x24 (sealed) and 0x34 (active) are valid for tt=0/19.
    // This function is kept for backward-compat callers that only need
    // the "default/active" value; detection logic uses is_valid_page_flags.
    if matches!(table_type, 0 | 19) {
        0x34
    } else {
        0x24
    }
}

fn is_valid_page_flags(table_type: u32, flags: u8) -> bool {
    match table_type {
        0 | 7 | 19 => flags == 0x24 || flags == 0x34, // sealed or active both valid
        _ => flags == 0x24,
    }
}

#[derive(Debug, Clone)]
pub(super) struct WrongFlagsPage {
    pub(super) page_index: usize,
    pub(super) correct_flags: u8,
}

pub(super) fn detect_pdb_wrong_page_flags(pdb_path: &Path) -> Vec<WrongFlagsPage> {
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};
    let Ok(bytes) = std::fs::read(pdb_path) else {
        return Vec::new();
    };
    let Some(page_size) = read_u32_le_at(&bytes, 4).map(|v| v as usize) else {
        return Vec::new();
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return Vec::new();
    }
    let total = bytes.len() / page_size;
    let mut out = Vec::new();
    for i in 1..total {
        let off = i * page_size;
        let Some(idx) = read_u32_le_at(&bytes, off + 4) else {
            continue;
        };
        if idx == 0 {
            continue;
        }
        let used_s = read_u16_le_at(&bytes, off + 0x1e).unwrap_or(0);
        if used_s == 0 {
            continue; // sentinel/fence page — skip
        }
        let table_type = read_u32_le_at(&bytes, off + 8).unwrap_or(9999);
        let flags = read_u8_at(&bytes, off + 0x1b).unwrap_or(0);
        if !is_valid_page_flags(table_type, flags) {
            out.push(WrongFlagsPage {
                page_index: i,
                correct_flags: expected_page_flags(table_type),
            });
        }
    }
    out
}

fn apply_pdb_wrong_page_flags_repair(
    usb_root: &Path,
    pages: &[WrongFlagsPage],
) -> BackendResult<usize> {
    if pages.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let Some(page_size) = bytes
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
    else {
        return Err(BackendError::Validation(
            "PDB too small to read page size".into(),
        ));
    };
    let mut patched = 0usize;
    for page in pages {
        let off = page.page_index * page_size + 0x1b;
        if off >= bytes.len() {
            continue;
        }
        bytes[off] = page.correct_flags;
        patched += 1;
    }
    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

/// A data page whose footer has tranrf=0 despite having active rows (rowpf≠0).
/// Some DJ software versions reject databases where tranrf is zero on a page
/// that contains live rows.  Fix: set tranrf = rowpf so all active rows are
/// considered part of the last transaction.
#[derive(Debug, Clone)]
pub(super) struct ZeroTranrfPage {
    pub(super) page_index: usize,
    pub(super) u5: u16,
    pub(super) num_rl: usize,
    /// rowpf value for each group (to copy into tranrf for groups that need fixing)
    pub(super) rowpf_groups: Vec<u16>,
    /// existing tranrf value for each group (0x0000 in groups that need fixing)
    pub(super) tranrf_groups: Vec<u16>,
    /// byte offset within the page of each group's 4-byte header
    pub(super) footer_group_offsets: Vec<usize>,
}

/// A tt=16/17/18 data page whose u5/num_rl footer fields carry the old wrong
/// `(1, nrs-1)` pattern instead of the correct `(nrs, 0)` convention.  DJ software
/// and player firmware may reject or misparse these pages.
#[derive(Debug, Clone)]
pub(super) struct WrongShapeHistoryPage {
    pub(super) page_index: usize,
    #[allow(dead_code)]
    pub(super) table_type: u32,
    pub(super) nrs: u16,
}

/// Detect tt=16/17/18 data pages carrying the wrong `(1, nrs-1)` footer shape.
/// The correct convention for these tables is `(nrs, 0)`.
pub(super) fn detect_pdb_wrong_history_page_shape(pdb_path: &Path) -> Vec<WrongShapeHistoryPage> {
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};
    let Ok(bytes) = std::fs::read(pdb_path) else {
        return Vec::new();
    };
    let Some(page_size) = read_u32_le_at(&bytes, 4).map(|v| v as usize) else {
        return Vec::new();
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return Vec::new();
    }
    let total = bytes.len() / page_size;
    let mut out = Vec::new();
    for i in 1..total {
        let off = i * page_size;
        let Some(idx) = read_u32_le_at(&bytes, off + 4) else {
            continue;
        };
        if idx == 0 {
            continue;
        }
        let used_s = read_u16_le_at(&bytes, off + 0x1e).unwrap_or(0);
        if used_s == 0 {
            continue;
        }
        let table_type = read_u32_le_at(&bytes, off + 8).unwrap_or(9999);
        if !matches!(table_type, 16..=18) {
            continue;
        }
        let nrs = read_u8_at(&bytes, off + 0x18).unwrap_or(0) as u16;
        if nrs == 0 {
            continue;
        }
        let u5 = read_u16_le_at(&bytes, off + 0x20).unwrap_or(0);
        let num_rl = read_u16_le_at(&bytes, off + 0x22).unwrap_or(0);
        // Flag the old-bug pattern: u5=1 and num_rl=nrs-1. When nrs=1 this
        // pattern is bit-identical to the correct (u5=nrs, num_rl=0) shape —
        // a single-row page can never be distinguished as wrong, so skip it
        // rather than flag a false positive that no repair can actually fix.
        if nrs > 1 && u5 == 1 && num_rl == nrs.saturating_sub(1) {
            out.push(WrongShapeHistoryPage {
                page_index: i,
                table_type,
                nrs,
            });
        }
    }
    out
}

fn apply_pdb_wrong_history_page_shape_repair(
    usb_root: &Path,
    pages: &[WrongShapeHistoryPage],
) -> BackendResult<usize> {
    if pages.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let Some(page_size) = bytes
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
    else {
        return Err(BackendError::Validation(
            "PDB too small to read page size".into(),
        ));
    };
    let mut patched = 0usize;
    for page in pages {
        let off = page.page_index * page_size;
        if off + 0x24 > bytes.len() {
            continue;
        }
        // Set u5 = nrs, num_rl = 0.
        bytes[off + 0x20..off + 0x22].copy_from_slice(&page.nrs.to_le_bytes());
        bytes[off + 0x22..off + 0x24].copy_from_slice(&0u16.to_le_bytes());
        patched += 1;
    }
    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

/// A tt=0 track data page whose u5/num_rl footer fields are inconsistent with its flags.
/// Valid combinations:
///   - flags=0x34 (active page): `(u5=2, num_rl=0)`
///   - flags=0x24 (sealed page): `(u5=1, num_rl=nrs-1)` OR `(u5=2, num_rl=0)` (player compat)
///
/// Only active (0x34) pages with wrong footer are flagged — they affect player loading.
#[derive(Debug, Clone)]
pub(super) struct WrongTrackU5Page {
    pub(super) page_index: usize,
}

pub(super) fn detect_pdb_wrong_track_u5(pdb_path: &Path) -> Vec<WrongTrackU5Page> {
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};
    let Ok(bytes) = std::fs::read(pdb_path) else {
        return Vec::new();
    };
    let Some(page_size) = read_u32_le_at(&bytes, 4).map(|v| v as usize) else {
        return Vec::new();
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return Vec::new();
    }
    let total = bytes.len() / page_size;
    let mut out = Vec::new();
    for i in 1..total {
        let off = i * page_size;
        let Some(idx) = read_u32_le_at(&bytes, off + 4) else {
            continue;
        };
        if idx == 0 {
            continue;
        }
        let table_type = read_u32_le_at(&bytes, off + 8).unwrap_or(9999);
        if table_type != 0 {
            continue;
        }
        let nrs = read_u8_at(&bytes, off + 0x18).unwrap_or(0) as u16;
        if nrs == 0 {
            continue;
        }
        let u5 = read_u16_le_at(&bytes, off + 0x20).unwrap_or(0);
        if u5 == 0x1FFF {
            continue; // sentinel — handled by separate repair
        }
        let flags = read_u8_at(&bytes, off + 0x1b).unwrap_or(0);
        if flags == 0x64 {
            continue; // sentinel page
        }
        let num_rl = read_u16_le_at(&bytes, off + 0x22).unwrap_or(0);
        // Active (0x34) pages: accept both (u5=1, num_rl=nrs-1) (current writer)
        // and (u5=2, num_rl=0) (old writer / early reference export observation). Flag anything else.
        if flags == 0x34 {
            let is_new_format = u5 == 1 && num_rl == nrs.saturating_sub(1);
            let is_old_format = u5 == 2 && num_rl == 0;
            if !is_new_format && !is_old_format {
                out.push(WrongTrackU5Page { page_index: i });
            }
        }
    }
    out
}

/// A track/album/artist row whose UTF-16 (`0x90`-marker) string slot starts at a row-relative
/// offset that isn't 4-byte aligned.
#[derive(Debug, Clone)]
pub(super) struct MisalignedStringSlot {
    /// 0 = tracks (t00), 2 = artists (t02), 3 = albums (t03).
    pub(super) table_type: u32,
    pub(super) id: u32,
    pub(super) slot_label: &'static str,
}

/// Mirrors `pdb_reader::track_string_slot_label` (private to that module — duplicated here as a
/// small const lookup rather than widening its visibility just for a label string).
const TRACK_STRING_SLOT_LABELS: [&str; 21] = [
    "isrc",
    "lyricist",
    "unknown_string_2",
    "unknown_string_3",
    "unknown_string_4",
    "message",
    "publish_track_info",
    "autoload_hotcues",
    "unknown_string_5",
    "unknown_string_6",
    "date_added",
    "release_date",
    "mix_name",
    "unknown_string_7",
    "analysis_path_start",
    "analysis_path_end",
    "comment",
    "title_start",
    "title_end",
    "filename",
    "track_file_path",
];

/// Detect rows whose UTF-16 string slot isn't 4-byte aligned. MIPS-based player hardware
/// requires 4-byte aligned reads for the UTF-16 header (`0x90 [len:u16] 0x00`); a misaligned slot
/// freezes/comm-errors CDJ hardware while browsing. Any exporter that doesn't guarantee this
/// alignment can produce it. `dump_pdb_anomalies`-style checks only look at page/table shape,
/// not per-row string alignment.
///
/// Artist near-variant rows (`encode_artist_row`) hardcode a 4-byte-aligned name offset (12), so
/// the current writer can never produce a misaligned artist row — table 2 is still checked
/// here for diagnostic completeness (e.g. a foreign-origin PDB), but `repair_usb_diagnostics`
/// offers no automated fix for it, only tracks (t00) and albums (t03).
pub(super) fn detect_pdb_string_misalignment(pdb_path: &Path) -> Vec<MisalignedStringSlot> {
    use crate::pdb_writer::parse_page_row_slots;
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};
    let Ok(bytes) = std::fs::read(pdb_path) else {
        return Vec::new();
    };
    let Some(page_size) = read_u32_le_at(&bytes, 4).map(|v| v as usize) else {
        return Vec::new();
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return Vec::new();
    }
    let total = bytes.len() / page_size;
    let mut out = Vec::new();
    for i in 1..total {
        let off = i * page_size;
        let Some(idx) = read_u32_le_at(&bytes, off + 4) else {
            continue;
        };
        if idx == 0 {
            continue;
        }
        let table_type = read_u32_le_at(&bytes, off + 8).unwrap_or(9999);
        if !matches!(table_type, 0 | 2 | 3) {
            continue;
        }
        let used_s = read_u16_le_at(&bytes, off + 30).unwrap_or(0) as usize;
        if used_s == 0 {
            continue;
        }
        let Some(page) = bytes.get(off..off + page_size) else {
            continue;
        };
        let payload_len = used_s.min(page_size.saturating_sub(40));
        for slot in parse_page_row_slots(page, page_size) {
            if !slot.present || slot.end > payload_len || slot.start >= slot.end {
                continue;
            }
            let row_abs = off + 40 + slot.start;
            let row_len = slot.end - slot.start;
            let Some(row) = bytes.get(row_abs..row_abs + row_len) else {
                continue;
            };
            match table_type {
                0 => {
                    if row_len < 136 {
                        continue;
                    }
                    let Some(id) = read_u32_le_at(row, 72) else {
                        continue;
                    };
                    for (index, label) in TRACK_STRING_SLOT_LABELS.iter().enumerate() {
                        let Some(slot_offset) =
                            read_u16_le_at(row, 94 + 2 * index).map(|v| v as usize)
                        else {
                            continue;
                        };
                        if row.get(slot_offset) == Some(&0x90) && slot_offset % 4 != 0 {
                            out.push(MisalignedStringSlot {
                                table_type,
                                id,
                                slot_label: label,
                            });
                        }
                    }
                }
                3 => {
                    if row_len < 16 {
                        continue;
                    }
                    let Some(id) = read_u32_le_at(row, 12) else {
                        continue;
                    };
                    let name_offset = read_u8_at(row, 21).unwrap_or(22) as usize;
                    if row.get(name_offset) == Some(&0x90) && !name_offset.is_multiple_of(4) {
                        out.push(MisalignedStringSlot {
                            table_type,
                            id,
                            slot_label: "name",
                        });
                    }
                }
                2 => {
                    if row_len < 10 {
                        continue;
                    }
                    let Some(id) = read_u32_le_at(row, 4) else {
                        continue;
                    };
                    let typ = read_u16_le_at(row, 0).unwrap_or(0);
                    let Some(name_offset) = (if typ == 100 {
                        read_u16_le_at(row, 10).map(|v| v as usize)
                    } else {
                        read_u8_at(row, 9).map(|v| v as usize)
                    }) else {
                        continue;
                    };
                    if row.get(name_offset) == Some(&0x90) && name_offset % 4 != 0 {
                        out.push(MisalignedStringSlot {
                            table_type,
                            id,
                            slot_label: "name",
                        });
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// A tt=0 (tracks) data page with flags=0x34 (ACTV) in a table that has more
/// than one data page. Reference exports confirm: single-page tt=0 chains use
/// ACTV (0x34); multi-page chains must use ALL SEAL (0x24). DJ software rejects
/// a multi-page chain that contains any ACTV page.
#[derive(Debug, Clone)]
pub(super) struct T00MultipageActivePage {
    pub(super) page_index: usize,
    pub(super) nrs: u8,
}

/// Detect ACTV (0x34) tt=0 pages that must be SEAL (0x24) in a multi-page chain.
/// The last (active write) page in the chain legitimately stays ACTV; only
/// predecessor pages that are still marked ACTV are flagged.
pub(super) fn detect_pdb_t00_multipage_active_pages(
    pdb_path: &Path,
) -> Vec<T00MultipageActivePage> {
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};
    let Ok(bytes) = std::fs::read(pdb_path) else {
        return Vec::new();
    };
    let Some(page_size) = read_u32_le_at(&bytes, 4).map(|v| v as usize) else {
        return Vec::new();
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return Vec::new();
    }
    let total = bytes.len() / page_size;

    // Collect all non-sentinel, non-empty tt=0 data pages: (page_index, flags, nrs, next_page).
    let mut t00_pages: Vec<(usize, u8, u8, u32)> = Vec::new();
    for i in 1..total {
        let off = i * page_size;
        let Some(idx) = read_u32_le_at(&bytes, off + 4) else {
            continue;
        };
        if idx == 0 {
            continue;
        }
        let tt = read_u32_le_at(&bytes, off + 8).unwrap_or(9999);
        if tt != 0 {
            continue;
        }
        let flags = read_u8_at(&bytes, off + 0x1b).unwrap_or(0);
        if flags == 0x64 {
            continue; // sentinel page
        }
        let used_s = read_u16_le_at(&bytes, off + 0x1e).unwrap_or(0);
        if used_s == 0 {
            continue;
        }
        let nrs = read_u8_at(&bytes, off + 0x18).unwrap_or(0);
        let next_page = read_u32_le_at(&bytes, off + 0x0c).unwrap_or(0);
        t00_pages.push((i, flags, nrs, next_page));
    }

    // Only flag when there are multiple data pages (multi-page chain rule).
    if t00_pages.len() <= 1 {
        return Vec::new();
    }

    // The last data page has next_page pointing outside the set of tt=0 data
    // pages (back to the sentinel or zero). Only predecessor pages must be SEAL.
    let data_page_indices: std::collections::HashSet<usize> =
        t00_pages.iter().map(|(i, _, _, _)| *i).collect();

    t00_pages
        .into_iter()
        .filter(|&(_page_index, flags, _, next_page)| {
            // Only flag ACTV pages whose next_page leads to another data page —
            // i.e. they are predecessors, not the terminal active page.
            flags == 0x34 && data_page_indices.contains(&(next_page as usize))
        })
        .map(|(page_index, _, nrs, _)| T00MultipageActivePage { page_index, nrs })
        .collect()
}

fn apply_pdb_t00_multipage_active_repair(
    usb_root: &Path,
    pages: &[T00MultipageActivePage],
) -> BackendResult<usize> {
    if pages.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let Some(page_size) = bytes
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
    else {
        return Err(BackendError::Validation(
            "PDB too small to read page size".into(),
        ));
    };
    let mut patched = 0usize;
    for page in pages {
        let off = page.page_index * page_size;
        if off + 0x24 > bytes.len() {
            continue;
        }
        // Flags: 0x34 → 0x24 (sealed convention for multi-page tt=0 chain)
        bytes[off + 0x1b] = 0x24;
        // Footer shape: sealed pages use (u5=1, num_rl=nrs-1) like dict tables.
        let nrs = page.nrs as u16;
        bytes[off + 0x20..off + 0x22].copy_from_slice(&1u16.to_le_bytes());
        bytes[off + 0x22..off + 0x24].copy_from_slice(&nrs.saturating_sub(1).to_le_bytes());
        patched += 1;
    }
    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

/// A table whose `empty_candidate` pointer targets a physical page that is
/// already a data page owned by a different table.  This creates an alias:
/// when either table grows, it will overwrite the other's data.  DJ software
/// validates this invariant on load and reports "database corrupted".
///
/// Cause: the old additive writer allocated overflow pages at
/// `bytes.len()/page_size` without accounting for virtual ec pages already
/// reserved in `next_unused_page` by other tables.
#[derive(Debug, Clone)]
pub(super) struct EcDataPageConflict {
    /// The table whose empty_candidate is wrong.
    pub(super) table_type: u32,
    /// Last data page of `table_type` (its `next_page` also needs fixing).
    pub(super) last_page: u32,
}

/// Detect tables whose `empty_candidate` points to a page already used as
/// data by a different table.
pub(super) fn detect_pdb_ec_data_page_conflicts(pdb_path: &Path) -> Vec<EcDataPageConflict> {
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};
    let Ok(bytes) = std::fs::read(pdb_path) else {
        return Vec::new();
    };
    let Some(page_size) = read_u32_le_at(&bytes, 4).map(|v| v as usize) else {
        return Vec::new();
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return Vec::new();
    }
    let total_pages = bytes.len() / page_size;
    let num_tables = read_u32_le_at(&bytes, 8).unwrap_or(0) as usize;

    // Build: page_index → table_type for all non-sentinel, non-empty data pages.
    let mut data_page_owner: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for p in 1..total_pages {
        let off = p * page_size;
        let stored_idx = read_u32_le_at(&bytes, off + 4).unwrap_or(0);
        if stored_idx == 0 {
            continue;
        }
        let pf = read_u8_at(&bytes, off + 0x1b).unwrap_or(0);
        if pf == 0x64 {
            continue; // sentinel page
        }
        let tt = read_u32_le_at(&bytes, off + 8).unwrap_or(0);
        let used_s = read_u16_le_at(&bytes, off + 0x1e).unwrap_or(0);
        if used_s == 0 {
            continue;
        }
        data_page_owner.insert(stored_idx, tt);
    }

    let mut conflicts = Vec::new();
    for i in 0..num_tables {
        let toff = 0x1c + i * 16;
        if toff + 16 > bytes.len() {
            break;
        }
        let tt = read_u32_le_at(&bytes, toff).unwrap_or(0);
        let ec = read_u32_le_at(&bytes, toff + 4).unwrap_or(0);
        let fp = read_u32_le_at(&bytes, toff + 8).unwrap_or(0);
        let lp = read_u32_le_at(&bytes, toff + 12).unwrap_or(0);
        if fp == lp {
            continue; // empty table — ec is just the pre-allocated blank slot
        }
        if let Some(&owner_tt) = data_page_owner.get(&ec)
            && owner_tt != tt
        {
            conflicts.push(EcDataPageConflict {
                table_type: tt,
                last_page: lp,
            });
        }
    }
    conflicts
}

fn apply_pdb_ec_data_page_conflict_repair(
    usb_root: &Path,
    conflicts: &[EcDataPageConflict],
) -> BackendResult<usize> {
    if conflicts.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let Some(page_size) = bytes
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
    else {
        return Err(BackendError::Validation(
            "PDB too small to read page size".into(),
        ));
    };
    let num_tables = bytes
        .get(8..12)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
        .unwrap_or(0);

    let physical_end = (bytes.len() / page_size) as u32;
    let current_next_unused = bytes
        .get(0x0c..0x10)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b))
        .unwrap_or(physical_end);
    let mut next_free = physical_end.max(current_next_unused);

    let mut patched = 0usize;
    for conflict in conflicts {
        let new_ec = next_free;
        next_free += 1;

        // Find the table pointer entry for this table type.
        let mut found_toff = None;
        for i in 0..num_tables {
            let off = 0x1c + i * 16;
            if off + 16 > bytes.len() {
                break;
            }
            if bytes
                .get(off..off + 4)
                .and_then(|b| b.try_into().ok())
                .map(|b: [u8; 4]| u32::from_le_bytes(b))
                .unwrap_or(9999)
                == conflict.table_type
            {
                found_toff = Some(off);
                break;
            }
        }
        let Some(toff) = found_toff else {
            continue;
        };

        // Patch empty_candidate in the table pointer.
        if toff + 8 <= bytes.len() {
            bytes[toff + 4..toff + 8].copy_from_slice(&new_ec.to_le_bytes());
        }

        // Patch last data page's next_page to match the new ec.
        let lp_off = (conflict.last_page as usize) * page_size;
        if lp_off + 0x10 <= bytes.len() {
            bytes[lp_off + 0x0c..lp_off + 0x10].copy_from_slice(&new_ec.to_le_bytes());
        }

        patched += 1;
    }

    // Update next_unused_page so future writers don't collide with the new ECs.
    bytes[0x0c..0x10].copy_from_slice(&next_free.to_le_bytes());

    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

/// A table whose declared `last` page is a physically nonexistent page index
/// (beyond the end of the file). This happens when an additive-growth write
/// is interrupted after the table pointer was advanced to claim new pages
/// but before those pages were actually flushed to disk: the real last
/// written page still has a valid, live row, but the header believes growth
/// continued further than it did.
///
/// Unlike `empty_candidate`, which healthy rekordbox exports routinely set
/// beyond the file's current physical length (reserved headroom for future
/// growth — confirmed against `USB_REKORDBOX6` and `USB_LOTS_OF_TRACKS_RB`),
/// a table's `last` page must always physically exist: it is not a future
/// candidate, it is the table's real current tail.
#[derive(Debug, Clone)]
pub(super) struct TruncatedTableChain {
    pub(super) table_type: u32,
    /// The real last page, found by walking the chain from `first` and
    /// stopping at the last page that physically exists and holds live data.
    pub(super) corrected_last: u32,
    /// The corrected `empty_candidate`, taken from the real last page's own
    /// `next` field (the documented `empty_candidate == last_page.next`
    /// invariant, verified against reference exports) when that value looks
    /// like a sane forward pointer, else the physical end of the file.
    pub(super) corrected_ec: u32,
}

/// Detect tables whose declared `last` page index does not physically exist
/// in the file, and compute the metadata correction that restores
/// `empty_candidate == last_page.next` without touching any page content.
pub(super) fn detect_pdb_truncated_table_chains(pdb_path: &Path) -> Vec<TruncatedTableChain> {
    use crate::utils::{collect_chain, page_offset, read_u16_le_at, read_u32_le_at};
    let Ok(bytes) = std::fs::read(pdb_path) else {
        return Vec::new();
    };
    let Some(page_size) = read_u32_le_at(&bytes, 4).map(|v| v as usize) else {
        return Vec::new();
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return Vec::new();
    }
    let total_pages = (bytes.len() / page_size) as u32;
    let num_tables = read_u32_le_at(&bytes, 8).unwrap_or(0) as usize;

    let mut fixes = Vec::new();
    for i in 0..num_tables {
        let toff = 0x1c + i * 16;
        if toff + 16 > bytes.len() {
            break;
        }
        let tt = read_u32_le_at(&bytes, toff).unwrap_or(0);
        let first = read_u32_le_at(&bytes, toff + 8).unwrap_or(0);
        let last = read_u32_le_at(&bytes, toff + 12).unwrap_or(0);
        if first == 0 || last < total_pages {
            continue; // empty table, or declared last already physically exists
        }
        if collect_chain(&bytes, page_size, first, last).is_some() {
            continue; // shouldn't happen (last >= total_pages), but stay safe
        }
        // Walk from `first` following `next` pointers, stopping at the last
        // page that is still within the physical file. Bounded by
        // total_pages so a cycle can't spin forever.
        let mut seen = HashSet::<u32>::new();
        let mut real_last = None;
        let mut cur = first;
        for _ in 0..=total_pages {
            if cur >= total_pages || !seen.insert(cur) {
                break;
            }
            real_last = Some(cur);
            if cur == last {
                break;
            }
            let Some(off) = page_offset(cur, page_size) else {
                break;
            };
            let Some(next) = read_u32_le_at(&bytes, off + 0x0c) else {
                break;
            };
            if next == 0 {
                break;
            }
            cur = next;
        }
        let Some(real_last) = real_last else { continue };
        if real_last == last {
            continue;
        }
        let Some(off) = page_offset(real_last, page_size) else {
            continue;
        };
        let Some(page) = bytes.get(off..off + page_size) else {
            continue;
        };
        let used_s = read_u16_le_at(page, 0x1e).unwrap_or(0);
        if used_s == 0 {
            continue; // don't anchor a repair on a blank/garbage page
        }
        let real_next = read_u32_le_at(&bytes, off + 0x0c).unwrap_or(total_pages);
        let corrected_ec = if real_next > real_last {
            real_next
        } else {
            total_pages
        };
        fixes.push(TruncatedTableChain {
            table_type: tt,
            corrected_last: real_last,
            corrected_ec,
        });
    }
    fixes
}

/// Patch each affected table's `last`/`empty_candidate` pointer fields to
/// match physical reality. No page content is modified — the real last page
/// found by [`detect_pdb_truncated_table_chains`] is already valid, live
/// data, and its own `next` field already anticipated the corrected `ec`.
fn apply_pdb_truncated_table_chain_repair(
    usb_root: &Path,
    fixes: &[TruncatedTableChain],
) -> BackendResult<usize> {
    if fixes.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let num_tables = bytes
        .get(8..12)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
        .unwrap_or(0);

    let mut patched = 0usize;
    for fix in fixes {
        let mut found_toff = None;
        for i in 0..num_tables {
            let off = 0x1c + i * 16;
            if off + 16 > bytes.len() {
                break;
            }
            if bytes
                .get(off..off + 4)
                .and_then(|b| b.try_into().ok())
                .map(|b: [u8; 4]| u32::from_le_bytes(b))
                .unwrap_or(9999)
                == fix.table_type
            {
                found_toff = Some(off);
                break;
            }
        }
        let Some(toff) = found_toff else { continue };
        if toff + 16 > bytes.len() {
            continue;
        }
        bytes[toff + 4..toff + 8].copy_from_slice(&fix.corrected_ec.to_le_bytes());
        bytes[toff + 12..toff + 16].copy_from_slice(&fix.corrected_last.to_le_bytes());
        patched += 1;
    }

    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

#[derive(Debug, Clone, Default)]
pub(super) struct PdbTornGrowthReport {
    /// Table `empty_candidate` pages that sit within a page no other table
    /// owns, but aren't the all-zero blank page the additive writer expects
    /// before it will reuse them (`pdb_writer.rs`'s `append_rows_to_chain_in_place`).
    pub(super) garbage_ec_pages: Vec<u32>,
    /// When set, the file should be truncated to this many pages: the header's
    /// own `next_unused_page` claims nothing beyond it is allocated, yet the
    /// physical file extends further with non-zero content.
    pub(super) truncate_to_pages: Option<u32>,
}

impl PdbTornGrowthReport {
    pub(super) fn is_empty(&self) -> bool {
        self.garbage_ec_pages.is_empty() && self.truncate_to_pages.is_none()
    }
}

/// Detect a torn additive-growth write left behind by a USB disconnect mid-export.
///
/// A rekordbox additive-growth write first extends `export.pdb` (and/or
/// earmarks existing `empty_candidate` pages as the next slot for one or
/// more tables), then writes the new page contents. If the drive is
/// disconnected between those two steps, the pages that were about to be
/// (re)written are left as whatever bytes previously occupied that disk
/// region — never zeroed, never structured — while every page actually
/// reachable through a table's `first_page`/`next_page` chain remains
/// untouched and valid.
///
/// Two independent signals of this:
/// - a table's `empty_candidate` points at a page that is not live data for
///   any table (so overwriting it can't corrupt another table's rows) but is
///   not all-zero either, meaning it failed to become the blank page the
///   additive writer requires before reuse;
/// - the physical file is longer than the header's own `next_unused_page`
///   claims, and that unreferenced tail is non-zero.
///
/// Only pages outside every table's live-data set are ever proposed here;
/// this never touches a page that is part of a real chain.
pub(super) fn detect_pdb_torn_growth_pages(pdb_path: &Path) -> PdbTornGrowthReport {
    use crate::utils::{page_offset, read_u8_at, read_u16_le_at, read_u32_le_at};
    let Ok(bytes) = std::fs::read(pdb_path) else {
        return PdbTornGrowthReport::default();
    };
    let Some(page_size) = read_u32_le_at(&bytes, 4).map(|v| v as usize) else {
        return PdbTornGrowthReport::default();
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return PdbTornGrowthReport::default();
    }
    let total_pages = (bytes.len() / page_size) as u32;
    let num_tables = read_u32_le_at(&bytes, 8).unwrap_or(0) as usize;
    let next_unused = read_u32_le_at(&bytes, 0x0c).unwrap_or(total_pages);

    // Same page-ownership scan detect_pdb_ec_data_page_conflicts uses: a page
    // is "live" only if it is a non-sentinel, non-empty data page actually
    // claimed by some table. Anything else is fair game to be a torn blank.
    let mut live_data_pages: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for p in 1..total_pages as usize {
        let off = p * page_size;
        let stored_idx = read_u32_le_at(&bytes, off + 4).unwrap_or(0);
        if stored_idx == 0 {
            continue;
        }
        let pf = read_u8_at(&bytes, off + 0x1b).unwrap_or(0);
        if pf == 0x64 {
            continue; // sentinel page
        }
        let used_s = read_u16_le_at(&bytes, off + 0x1e).unwrap_or(0);
        if used_s == 0 {
            continue;
        }
        live_data_pages.insert(stored_idx);
    }

    let mut garbage_ec_pages = Vec::new();
    for i in 0..num_tables {
        let toff = 0x1c + i * 16;
        if toff + 16 > bytes.len() {
            break;
        }
        let ec = read_u32_le_at(&bytes, toff + 4).unwrap_or(0);
        if ec == 0 || live_data_pages.contains(&ec) {
            continue;
        }
        let Some(off) = page_offset(ec, page_size) else {
            continue;
        };
        let Some(page) = bytes.get(off..off + page_size) else {
            continue;
        };
        if !page.iter().all(|b| *b == 0) {
            garbage_ec_pages.push(ec);
        }
    }
    garbage_ec_pages.sort_unstable();
    garbage_ec_pages.dedup();

    let truncate_to_pages = if total_pages > next_unused && next_unused > 0 {
        let tail_off = next_unused as usize * page_size;
        let tail_is_dirty = bytes
            .get(tail_off..)
            .map(|tail| tail.iter().any(|b| *b != 0))
            .unwrap_or(false);
        tail_is_dirty.then_some(next_unused)
    } else {
        None
    };

    PdbTornGrowthReport {
        garbage_ec_pages,
        truncate_to_pages,
    }
}

/// Zero the garbage `empty_candidate` pages and drop the never-populated
/// tail identified by [`detect_pdb_torn_growth_pages`]. Page indices used by
/// any table are never touched or renumbered — this only restores pages the
/// header itself says are blank/unallocated back to that state.
fn apply_pdb_torn_growth_pages_repair(
    usb_root: &Path,
    report: &PdbTornGrowthReport,
) -> BackendResult<usize> {
    use crate::utils::{page_offset, read_u32_le_at};
    if report.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let Some(page_size) = read_u32_le_at(&bytes, 4).map(|v| v as usize) else {
        return Err(BackendError::Validation(
            "PDB too small to read page size".into(),
        ));
    };
    if page_size == 0 {
        return Err(BackendError::Validation("PDB has zero page size".into()));
    }

    let mut patched = 0usize;
    for &ec in &report.garbage_ec_pages {
        let Some(off) = page_offset(ec, page_size) else {
            continue;
        };
        if off + page_size > bytes.len() {
            continue;
        }
        bytes[off..off + page_size].fill(0);
        patched += 1;
    }

    if let Some(truncate_to) = report.truncate_to_pages {
        let new_len = truncate_to as usize * page_size;
        if new_len < bytes.len() {
            bytes.truncate(new_len);
        }
    }

    // seqdb must stay strictly greater than every remaining seqpage. The
    // garbage pages could have carried bogus huge seqpage values; recompute
    // now that they're zeroed/truncated away.
    let max_seq = crate::pdb_writer::max_seqpage_in_file(&bytes, page_size);
    let cur = read_u32_le_at(&bytes, 0x14).unwrap_or(0);
    let seqdb = cur.max(max_seq.saturating_add(1));
    bytes[0x14..0x18].copy_from_slice(&seqdb.to_le_bytes());

    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

/// Round-trip an existing decoded track row into the writer's input struct, unchanged. Used
/// only to force a re-encode via the current (alignment-fixed) `encode_track_row_with_profile` —
/// no dictionary lookups needed since nothing semantic changes, unlike `build_merged_pdb_track_row`.
fn track_row_data_from_reader_row(row: &crate::pdb_reader::PdbTrackRow) -> PdbTrackRowData {
    PdbTrackRowData {
        header_flags_u32: None,
        content_link: row.content_link,
        sample_rate_hz: row.sample_rate_hz,
        file_size_bytes: row.file_size_bytes,
        master_content_id: row.master_content_id,
        master_db_id: row.master_db_id,
        id: row.id,
        artist_id: row.artist_id,
        album_id: row.album_id,
        artwork_id: row.artwork_id,
        key_id: row.key_id,
        genre_id: row.genre_id,
        bitrate_kbps: row.bitrate_kbps,
        track_number: (row.track_number != 0).then_some(row.track_number),
        bpm: (row.tempo_x100 != 0).then_some(row.tempo_x100 as f64 / 100.0),
        release_year: row.release_year,
        bit_depth: row.bit_depth,
        duration_seconds: row.duration_seconds,
        file_type: row.file_type,
        isrc: row.isrc.clone(),
        date_added: row.date_added.clone(),
        release_date: row.release_date.clone(),
        dj_comment: row.dj_comment.clone(),
        file_name: row.file_name.clone(),
        publish_track_info_on: Some(row.publish_track_info.as_deref() == Some("ON")),
        autoload_hotcues_on: Some(row.autoload_hotcues.as_deref() == Some("ON")),
        title: row.title.clone(),
        anlz_path: row.anlz_path.clone(),
        file_path: row.track_file_path.clone(),
    }
}

/// Fix track rows whose title/filename/other UTF-16 string slot isn't 4-byte aligned (see
/// `detect_pdb_string_misalignment`). Re-encodes each row's content unchanged through the
/// current (already alignment-fixed) `encode_track_row_with_profile` and relocates it via
/// `mutate_tracks_in_place`, which now refuses to same-size-patch an already-misaligned row (see
/// `track_row_has_misaligned_utf16_slot` in pdb_writer.rs) and instead always takes the
/// relocate/append path.
fn apply_pdb_track_string_alignment_repair(usb_root: &Path, ids: &[u32]) -> BackendResult<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let Some(page_size) = bytes
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
    else {
        return Err(BackendError::Validation(
            "PDB too small to read page size".into(),
        ));
    };
    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes)?;
    let by_id: HashMap<u32, &crate::pdb_reader::PdbTrackRow> =
        parsed.tracks.iter().map(|t| (t.id, t)).collect();
    let mutations: Vec<crate::pdb_writer::PdbTrackRowMutation> = ids
        .iter()
        .filter_map(|id| by_id.get(id))
        .map(|row| crate::pdb_writer::PdbTrackRowMutation {
            row: track_row_data_from_reader_row(row),
            changed_fields: vec!["title", "file_name", "dj_comment", "isrc"],
        })
        .collect();
    if mutations.is_empty() {
        return Ok(0);
    }
    let patched = crate::pdb_writer::mutate_tracks_in_place(
        &mut bytes,
        &mutations,
        PdbLayoutProfile::DEFAULT,
        page_size,
    )?;
    let mismatches = crate::pdb_reader::validate_pdb_page_conventions(&bytes);
    if !mismatches.is_empty() {
        let detail = mismatches
            .iter()
            .take(8)
            .map(|m| m.to_string())
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(BackendError::Validation(format!(
            "Track string alignment repair blocked: page-header convention mismatches ({detail})"
        )));
    }
    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

/// Fix album rows whose UTF-16 name slot isn't 4-byte aligned (see
/// `detect_pdb_string_misalignment`). Reads each affected row's existing `artist_id`/name
/// directly off the raw bytes (nothing semantic changes, only the encoding), re-encodes via
/// `encode_album_row` (already alignment-fixed), and relocates via
/// `realign_dictionary_rows_in_place`.
fn apply_pdb_album_string_alignment_repair(usb_root: &Path, ids: &[u32]) -> BackendResult<usize> {
    use crate::utils::{page_offset, read_u8_at, read_u32_le_at};
    if ids.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let Some(page_size) = bytes
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
    else {
        return Err(BackendError::Validation(
            "PDB too small to read page size".into(),
        ));
    };
    let target_ids: HashSet<u32> = ids.iter().copied().collect();
    let (_ec, first, last) = crate::utils::table_ptr_fields(&bytes, 3).ok_or_else(|| {
        BackendError::Validation(
            "Album string alignment repair blocked: t03 pointer missing".into(),
        )
    })?;
    crate::utils::collect_chain(&bytes, page_size, first, last).ok_or_else(|| {
        BackendError::Validation("Album string alignment repair blocked: t03 chain invalid".into())
    })?;

    // Scan every physical page stamped table_type=3, not just pages reachable by walking the
    // first/last chain pointers -- matching `detect_pdb_string_misalignment`'s scope exactly.
    // `ids` came from that same raw scan, so a chain-only walk here could silently fail to find
    // (and thus never fix) a row `ids` named: e.g. a stale row left present on a page that an
    // earlier rewrite orphaned from the live chain without touching its content.
    let mut rows: Vec<(u32, Vec<u8>)> = Vec::new();
    let total_pages = bytes.len() / page_size;
    for page_idx in 1..total_pages as u32 {
        let Some(off) = page_offset(page_idx, page_size) else {
            continue;
        };
        let Some(page) = bytes.get(off..off + page_size) else {
            continue;
        };
        if read_u32_le_at(page, 4).unwrap_or(0) == 0 {
            continue;
        }
        if read_u32_le_at(page, 8) != Some(3) {
            continue;
        }
        for slot in crate::pdb_writer::parse_page_row_slots(page, page_size) {
            if !slot.present || slot.start >= slot.end {
                continue;
            }
            let row_abs = off + 40 + slot.start;
            let row_len = slot.end - slot.start;
            let Some(row) = bytes.get(row_abs..row_abs + row_len) else {
                continue;
            };
            if row_len < 16 {
                continue;
            }
            let Some(id) = read_u32_le_at(row, 12) else {
                continue;
            };
            if !target_ids.contains(&id) {
                continue;
            }
            let Some(artist_id) = read_u32_le_at(row, 8) else {
                continue;
            };
            let name_offset = read_u8_at(row, 21).unwrap_or(22) as usize;
            let name = crate::pdb_reader::get_string_from_pdb(row, name_offset).unwrap_or_default();
            rows.push((id, encode_album_row(id, &name, artist_id)));
        }
    }
    if rows.is_empty() {
        return Ok(0);
    }
    let patched =
        crate::pdb_writer::realign_dictionary_rows_in_place(&mut bytes, 3, 12, &rows, page_size)?;
    let mismatches = crate::pdb_reader::validate_pdb_page_conventions(&bytes);
    if !mismatches.is_empty() {
        let detail = mismatches
            .iter()
            .take(8)
            .map(|m| m.to_string())
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(BackendError::Validation(format!(
            "Album string alignment repair blocked: page-header convention mismatches ({detail})"
        )));
    }
    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

fn apply_pdb_wrong_track_u5_repair(
    usb_root: &Path,
    pages: &[WrongTrackU5Page],
) -> BackendResult<usize> {
    if pages.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let Some(page_size) = bytes
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
    else {
        return Err(BackendError::Validation(
            "PDB too small to read page size".into(),
        ));
    };
    let mut patched = 0usize;
    for page in pages {
        let off = page.page_index * page_size;
        if off + 0x24 > bytes.len() {
            continue;
        }
        bytes[off + 0x20..off + 0x22].copy_from_slice(&2u16.to_le_bytes());
        bytes[off + 0x22..off + 0x24].copy_from_slice(&0u16.to_le_bytes());
        patched += 1;
    }
    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

/// A tombstoned row whose id field is non-zero.  When a repair re-writes a row
/// into a new slot it may leave the old (tombstoned) slot with its original id
/// intact.  DJ software validates row ids across all slots including tombstoned
/// ones and rejects the PDB if duplicates are found.
/// Fix: zero out the id field in each affected tombstoned slot.
#[derive(Debug, Clone)]
pub(super) struct TombstonedRowId {
    pub(super) page_index: usize,
    /// byte offset within the page of the 4-byte id field in the tombstoned row
    pub(super) id_field_offset: usize,
}

// (table_type, id_field_offset_in_row) pairs for tables where the DJ software
// validates uniqueness including tombstoned slots.
// tt=0 (tracks): id at row byte 72
// tt=7 (playlist_tree): id at row byte 12
const TOMBSTONE_ID_TABLES: &[(u32, usize)] = &[(0, 72), (7, 12)];

/// Detect tombstoned rows in tt=0 (tracks) and tt=7 (playlist_tree) whose id
/// field duplicates an active row — indicating a stale slot left by a repair
/// operation that wrote the row to a new position without zeroing the old one.
///
/// Non-zero IDs in tombstones that have no corresponding active row are normal
/// deletions (the DJ software leaves IDs in place when deleting rows) and are
/// not flagged.
pub(super) fn detect_pdb_tombstoned_playlist_tree_ids(pdb_path: &Path) -> Vec<TombstonedRowId> {
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};
    let Ok(bytes) = std::fs::read(pdb_path) else {
        return Vec::new();
    };
    let Some(page_size) = read_u32_le_at(&bytes, 4).map(|v| v as usize) else {
        return Vec::new();
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return Vec::new();
    }
    let total = bytes.len() / page_size;

    // Collect active IDs and tombstone candidates in one pass.
    // active_ids: table_type → set of IDs present in live rows.
    // candidates: tombstoned rows with non-zero IDs that might be duplicates.
    let mut active_ids: std::collections::HashMap<u32, std::collections::HashSet<u32>> =
        std::collections::HashMap::new();
    let mut candidates: Vec<(u32, u32, TombstonedRowId)> = Vec::new(); // (tt, id, row)

    for i in 1..total {
        let off = i * page_size;
        let Some(idx) = read_u32_le_at(&bytes, off + 4) else {
            continue;
        };
        if idx == 0 {
            continue;
        }
        let used_s = read_u16_le_at(&bytes, off + 0x1e).unwrap_or(0);
        if used_s == 0 {
            continue;
        }
        let tt = read_u32_le_at(&bytes, off + 8).unwrap_or(9999);
        let id_field_offset = match TOMBSTONE_ID_TABLES.iter().find(|(t, _)| *t == tt) {
            Some((_, ofs)) => *ofs,
            None => continue,
        };

        // Read row count from packed header (standard packed format for all supported tables).
        let b18 = read_u8_at(&bytes, off + 0x18).unwrap_or(0) as u32;
        let b19 = read_u8_at(&bytes, off + 0x19).unwrap_or(0) as u32;
        let b1a = read_u8_at(&bytes, off + 0x1a).unwrap_or(0) as u32;
        let packed = b18 | (b19 << 8) | (b1a << 16);
        let num_row_offsets = (packed & 0x1FFF) as usize;
        if num_row_offsets == 0 {
            continue;
        }

        // Read rowpf from the footer (grows backward from end of page).
        let footer_groups = num_row_offsets.div_ceil(16);
        let mut cursor = page_size;
        let mut rowpf_groups: Vec<u16> = Vec::with_capacity(footer_groups);
        let mut row_offsets_per_group: Vec<Vec<u16>> = Vec::with_capacity(footer_groups);
        let mut ok = true;
        for g in 0..footer_groups {
            if cursor < 4 {
                ok = false;
                break;
            }
            cursor -= 4;
            let rowpf = read_u16_le_at(&bytes, off + cursor).unwrap_or(0);
            rowpf_groups.push(rowpf);
            let glen = num_row_offsets.saturating_sub(g * 16).min(16);
            if cursor < glen * 2 {
                ok = false;
                break;
            }
            cursor -= glen * 2;
            let mut offsets = Vec::with_capacity(glen);
            for r in 0..glen {
                let roff = read_u16_le_at(&bytes, off + cursor + r * 2).unwrap_or(0);
                offsets.push(roff);
            }
            row_offsets_per_group.push(offsets);
        }
        if !ok {
            continue;
        }

        // The footer offset table is stored in REVERSED row order within each
        // group: position 0 holds the offset for the LAST row in the group,
        // position glen-1 holds the offset for the FIRST row. Rowpf bits are
        // in forward (row-index) order, so for position r within a group of
        // glen rows the correct rowpf bit is g*16 + (glen-1-r).
        for (g, rowpf) in rowpf_groups.iter().enumerate() {
            let group_offsets = &row_offsets_per_group[g];
            let glen = group_offsets.len();
            for (r, &heap_off) in group_offsets.iter().enumerate() {
                let bit = g * 16 + (glen - 1 - r);
                let is_active = (rowpf >> (bit % 16)) & 1 == 1;
                let heap_start = 0x28usize;
                let row_byte_off = off + heap_start + heap_off as usize;
                let id_byte_off = row_byte_off + id_field_offset;
                if id_byte_off + 4 > bytes.len() {
                    continue;
                }
                let id = read_u32_le_at(&bytes, id_byte_off).unwrap_or(0);
                if id == 0 {
                    continue;
                }
                if is_active {
                    active_ids.entry(tt).or_default().insert(id);
                } else {
                    candidates.push((
                        tt,
                        id,
                        TombstonedRowId {
                            page_index: i,
                            id_field_offset: id_byte_off - off,
                        },
                    ));
                }
            }
        }
    }

    // Only flag tombstones whose ID is still present in a live row — true duplicates.
    candidates
        .into_iter()
        .filter(|(tt, id, _)| {
            active_ids
                .get(tt)
                .map(|set| set.contains(id))
                .unwrap_or(false)
        })
        .map(|(_, _, row)| row)
        .collect()
}

/// A tt=7 (playlist_tree) data page with wrong (u5, num_rl) footer shape.
/// The correct convention is (nrs, 0). The original broken export wrote num_rl=1
/// (and u5=1) and the additive exporter silently preserved old_num_rl, leaving
/// num_rl ≠ 0 — which DJ software detects as corrupted.
#[derive(Debug, Clone)]
pub(super) struct WrongPlaylistTreeShapePage {
    pub(super) page_index: usize,
    pub(super) nrs: u8,
}

pub(super) fn detect_pdb_wrong_playlist_tree_shape(
    pdb_path: &Path,
) -> Vec<WrongPlaylistTreeShapePage> {
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};
    let Ok(bytes) = std::fs::read(pdb_path) else {
        return Vec::new();
    };
    let Some(page_size) = read_u32_le_at(&bytes, 4).map(|v| v as usize) else {
        return Vec::new();
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return Vec::new();
    }
    let total = bytes.len() / page_size;
    let mut out = Vec::new();
    for i in 1..total {
        let off = i * page_size;
        let Some(idx) = read_u32_le_at(&bytes, off + 4) else {
            continue;
        };
        if idx == 0 {
            continue;
        }
        let tt = read_u32_le_at(&bytes, off + 8).unwrap_or(9999);
        if tt != 7 {
            continue;
        }
        let flags = read_u8_at(&bytes, off + 0x1b).unwrap_or(0);
        if flags == 0x64 {
            continue; // sentinel page, skip
        }
        let nrs = read_u8_at(&bytes, off + 0x18).unwrap_or(0);
        if nrs == 0 {
            continue;
        }
        let u5 = read_u16_le_at(&bytes, off + 0x20).unwrap_or(0);
        if u5 == 0x1FFF {
            continue; // sentinel value — handled separately
        }
        let num_rl = read_u16_le_at(&bytes, off + 0x22).unwrap_or(0);
        // Convention: (nrs, 0). Flag if num_rl ≠ 0 OR u5 ≠ nrs.
        if num_rl != 0 || u5 != nrs as u16 {
            out.push(WrongPlaylistTreeShapePage { page_index: i, nrs });
        }
    }
    out
}

fn apply_pdb_wrong_playlist_tree_shape_repair(
    usb_root: &Path,
    pages: &[WrongPlaylistTreeShapePage],
) -> BackendResult<usize> {
    if pages.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let Some(page_size) = bytes
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
    else {
        return Err(BackendError::Validation(
            "PDB too small to read page size".into(),
        ));
    };
    let mut patched = 0usize;
    for page in pages {
        let off = page.page_index * page_size;
        if off + 0x24 > bytes.len() {
            continue;
        }
        // Set u5 = nrs (total row slot count), num_rl = 0.
        let nrs_u16 = page.nrs as u16;
        bytes[off + 0x20..off + 0x22].copy_from_slice(&nrs_u16.to_le_bytes());
        bytes[off + 0x22..off + 0x24].copy_from_slice(&0u16.to_le_bytes());
        patched += 1;
    }
    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

/// A sentinel (index) page whose B-tree entry area is stale — it has num_entries > 0
/// with entries that no longer match the actual data page layout after additive exports
/// grew the table beyond what the original export recorded.
///
/// DJ desktop software uses the B-tree index to navigate and validate the table chain.
/// If the entries point to wrong pages the validation fails and the DJ software reports "corrupted".
/// Fresh exports write num_entries=0 and player firmware walks the page chain directly,
/// so clearing the stale entries is safe for all validators.
#[derive(Debug, Clone)]
pub(super) struct StaleSentinelBtree {
    pub(super) page_index: usize,
}

/// Detect sentinel pages with a stale B-tree index.
///
/// The B-tree entry format is `page_index * 8` (sector-addressed, 512-byte sectors).
/// A sentinel is stale when its B-tree entries don't cover all data pages of its table
/// type — which happens when an additive export appended new pages without updating the
/// B-tree that originally written by DJ software.
///
/// A sentinel whose entries all decode to valid data pages of the correct table type,
/// and whose entry count equals the actual data page count, is considered consistent and
/// is NOT flagged. This means a freshly DJ-software-authored PDB with correct B-tree
/// entries passes without error.
pub(super) fn detect_pdb_stale_sentinel_btree(pdb_path: &Path) -> Vec<StaleSentinelBtree> {
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};
    use std::collections::HashMap;
    let Ok(bytes) = std::fs::read(pdb_path) else {
        return Vec::new();
    };
    let Some(page_size) = read_u32_le_at(&bytes, 4).map(|v| v as usize) else {
        return Vec::new();
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return Vec::new();
    }
    let total = bytes.len() / page_size;

    // Count data pages per table type: total and active (0x34 only).
    // B-tree entries must match active pages only; sealed (0x24) pages are
    // navigated via next_page chain and are not indexed in the B-tree.
    let mut data_pages_per_tt: HashMap<u32, usize> = HashMap::new();
    let mut active_pages_per_tt: HashMap<u32, usize> = HashMap::new();
    for i in 1..total {
        let off = i * page_size;
        let Some(idx) = read_u32_le_at(&bytes, off + 4) else {
            continue;
        };
        if idx == 0 {
            continue;
        }
        let flags = read_u8_at(&bytes, off + 0x1b).unwrap_or(0);
        if flags == 0x64 {
            continue;
        } // sentinel — skip
        let used_s = read_u16_le_at(&bytes, off + 0x1e).unwrap_or(0);
        if used_s == 0 {
            continue;
        } // empty page — skip
        let tt = read_u32_le_at(&bytes, off + 8).unwrap_or(9999);
        *data_pages_per_tt.entry(tt).or_insert(0) += 1;
        if flags == 0x34 {
            *active_pages_per_tt.entry(tt).or_insert(0) += 1;
        }
    }

    let mut out = Vec::new();
    for i in 1..total {
        let off = i * page_size;
        let Some(idx) = read_u32_le_at(&bytes, off + 4) else {
            continue;
        };
        if idx == 0 {
            continue;
        }
        let flags = read_u8_at(&bytes, off + 0x1b).unwrap_or(0);
        if flags != 0x64 {
            continue;
        } // only sentinel/index pages
        let ne = read_u16_le_at(&bytes, off + 0x38).unwrap_or(0);
        let u7 = read_u16_le_at(&bytes, off + 0x26).unwrap_or(0);
        let tt = read_u32_le_at(&bytes, off + 8).unwrap_or(9999);
        let active = *active_pages_per_tt.get(&tt).unwrap_or(&0);
        // Quick-exit: clean sentinel — no active pages and no orphaned pointer.
        if ne == 0 && u7 == 0 && active == 0 {
            continue;
        }

        // Check each B-tree entry: must be in-bounds and point to a data page
        // (not a sentinel) of the correct table type. Entries may point to SEAL
        // (0x24) pages — reference exports use 0x24-page B-trees for dict tables.
        // Only sentinel (0x64) targets or wrong-table targets are invalid.
        // ne count is not required to equal the active (0x34) page count; reference
        // exports often have ne that differs from the live 0x34 page count.
        let mut entries_valid = true;
        for slot in 0..ne as usize {
            let entry_off = off + 0x3c + slot * 4;
            let Some(entry) = read_u32_le_at(&bytes, entry_off) else {
                entries_valid = false;
                break;
            };
            if entry == 0x1fff_fff8 {
                entries_valid = false;
                break;
            }
            let page_idx = (entry / 8) as usize;
            let page_off = page_idx * page_size;
            if page_off + page_size > bytes.len() {
                entries_valid = false;
                break;
            }
            let page_tt = read_u32_le_at(&bytes, page_off + 8).unwrap_or(9999);
            let page_flags = read_u8_at(&bytes, page_off + 0x1b).unwrap_or(0);
            if page_tt != tt || page_flags == 0x64 {
                entries_valid = false;
                break;
            }
        }

        // u7 orphaned: write-pointer non-zero but no entries.
        let u7_orphaned = ne == 0 && u7 != 0;
        // Missing: has 0x34 pages but B-tree is empty.
        let missing_entries = ne == 0 && active > 0;
        let stale = !entries_valid || u7_orphaned || missing_entries;
        if stale {
            out.push(StaleSentinelBtree { page_index: i });
        }
    }
    out
}

fn apply_pdb_stale_sentinel_btree_repair(
    usb_root: &Path,
    pages: &[StaleSentinelBtree],
) -> BackendResult<usize> {
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};
    if pages.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let Some(page_size) = bytes
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
    else {
        return Err(BackendError::Validation(
            "PDB too small to read page size".into(),
        ));
    };
    const SENTINEL_EMPTY_ENTRY: u32 = 0x1fff_fff8;
    const SENTINEL_TAIL_ZERO_BYTES: usize = 20;
    let total_physical = bytes.len() / page_size;
    let mut patched = 0usize;
    for page in pages {
        let off = page.page_index * page_size;
        if off + page_size > bytes.len() {
            continue;
        }
        // Collect all active data pages for this table by scanning the whole file.
        // Following next_page chain pointers is unreliable — old pages may have
        // stale pointers to pages that have been repurposed for a different table.
        // Scanning by tt gives the definitive set of live pages.
        let tt = read_u32_le_at(&bytes, off + 8).unwrap_or(9999);
        let mut data_pages: Vec<u32> = Vec::new();
        for j in 1..total_physical {
            if j == page.page_index {
                continue;
            }
            let poff = j * page_size;
            let page_tt = read_u32_le_at(&bytes, poff + 8).unwrap_or(9999);
            if page_tt != tt {
                continue;
            }
            let page_flags = read_u8_at(&bytes, poff + 0x1b).unwrap_or(0);
            // B-tree entries must point to active (0x34) pages only.
            // Sealed (0x24) pages are navigated via next_page chain, not indexed.
            if page_flags != 0x34 {
                continue;
            }
            let used_s = read_u16_le_at(&bytes, poff + 0x1e).unwrap_or(0);
            if used_s == 0 {
                continue;
            } // skip empty pages
            data_pages.push(j as u32);
        }
        data_pages.sort_unstable();
        let n = data_pages.len();
        let fill_end = off + page_size.saturating_sub(SENTINEL_TAIL_ZERO_BYTES);
        // Write correct entries, then fill remaining slots with SENTINEL_EMPTY_ENTRY.
        let mut cursor = off + 0x3c;
        for (i, &page_idx) in data_pages.iter().enumerate() {
            if cursor + 4 > fill_end {
                break;
            }
            let entry_val = page_idx * 8;
            bytes[cursor..cursor + 4].copy_from_slice(&entry_val.to_le_bytes());
            cursor += 4;
            let _ = i;
        }
        while cursor + 4 <= fill_end {
            bytes[cursor..cursor + 4].copy_from_slice(&SENTINEL_EMPTY_ENTRY.to_le_bytes());
            cursor += 4;
        }
        let ne = n.min(0x1FFF) as u16;
        // num_entries = number of data pages
        bytes[off + 0x38..off + 0x3a].copy_from_slice(&ne.to_le_bytes());
        // first_empty = 0x1FFF (no free slots)
        bytes[off + 0x3a..off + 0x3c].copy_from_slice(&0x1fffu16.to_le_bytes());
        // u7 (write-pointer) = n
        bytes[off + 0x26..off + 0x28].copy_from_slice(&ne.to_le_bytes());
        patched += 1;
    }
    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

fn apply_pdb_tombstoned_playlist_tree_id_repair(
    usb_root: &Path,
    items: &[TombstonedRowId],
) -> BackendResult<usize> {
    if items.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let Some(page_size) = bytes
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
    else {
        return Err(BackendError::Validation(
            "PDB too small to read page size".into(),
        ));
    };
    let mut patched = 0usize;
    for item in items {
        let abs = item.page_index * page_size + item.id_field_offset;
        if abs + 4 > bytes.len() {
            continue;
        }
        bytes[abs..abs + 4].copy_from_slice(&0u32.to_le_bytes());
        patched += 1;
    }
    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

/// Detect zero-tranrf on all non-empty data pages (any table type).
pub(super) fn detect_pdb_zero_tranrf_all_tables(pdb_path: &Path) -> Vec<ZeroTranrfPage> {
    detect_pdb_zero_tranrf_pages_for_tables(pdb_path, Some(&[]))
}

// table_filter: None = only tt=0; Some(&[]) = all tables; Some(list) = specific tables
fn detect_pdb_zero_tranrf_pages_for_tables(
    pdb_path: &Path,
    table_filter: Option<&[u32]>,
) -> Vec<ZeroTranrfPage> {
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};
    let Ok(bytes) = std::fs::read(pdb_path) else {
        return Vec::new();
    };
    let Some(page_size) = read_u32_le_at(&bytes, 4).map(|v| v as usize) else {
        return Vec::new();
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return Vec::new();
    }
    let total = bytes.len() / page_size;
    let mut out = Vec::new();

    for i in 1..total {
        let off = i * page_size;
        let Some(idx) = read_u32_le_at(&bytes, off + 4) else {
            continue;
        };
        if idx == 0 {
            continue;
        }
        let tt = read_u32_le_at(&bytes, off + 8).unwrap_or(9999);
        let pass = match table_filter {
            None => tt == 0,   // track pages only
            Some(&[]) => true, // all tables
            Some(list) => list.contains(&tt),
        };
        if !pass {
            continue;
        }
        let used_s = read_u16_le_at(&bytes, off + 0x1e).unwrap_or(0);
        if used_s == 0 {
            continue;
        }
        let nrs = read_u8_at(&bytes, off + 0x18).unwrap_or(0) as usize;
        let u5 = read_u16_le_at(&bytes, off + 0x20).unwrap_or(0);
        let num_rl = read_u16_le_at(&bytes, off + 0x22).unwrap_or(0) as usize;
        let row_slots = if num_rl == 8191 { nrs } else { nrs.max(num_rl) };
        if row_slots == 0 {
            continue;
        }

        let page = &bytes[off..off + page_size];
        let groups = row_slots.div_ceil(16);
        let mut cursor = page_size;
        let mut rowpf_groups = Vec::with_capacity(groups);
        let mut tranrf_groups = Vec::with_capacity(groups);
        let mut footer_group_offsets = Vec::with_capacity(groups);
        let mut parse_ok = true;

        for g in 0..groups {
            if cursor < 4 {
                parse_ok = false;
                break;
            }
            cursor -= 4;
            rowpf_groups.push(read_u16_le_at(page, cursor).unwrap_or(0));
            tranrf_groups.push(read_u16_le_at(page, cursor + 2).unwrap_or(0));
            footer_group_offsets.push(cursor);
            let glen = (row_slots - g * 16).min(16);
            if cursor < glen * 2 {
                parse_ok = false;
                break;
            }
            cursor -= glen * 2;
        }

        if !parse_ok {
            continue;
        }

        // For u5=1 tables (dict tables, playlist entries) only the last group carries
        // tranrf; all preceding groups are correctly zero per the documented formula.
        // For other tables (u5=trc, u5=2) tranrf must mirror rowpf in every group.
        let needs_fix = if u5 == 1 {
            let last_group = num_rl / 16;
            last_group < groups && rowpf_groups[last_group] != 0 && tranrf_groups[last_group] == 0
        } else {
            rowpf_groups
                .iter()
                .zip(tranrf_groups.iter())
                .any(|(&rf, &tf)| rf != 0 && tf == 0)
        };

        if needs_fix {
            out.push(ZeroTranrfPage {
                page_index: i,
                u5,
                num_rl,
                rowpf_groups,
                tranrf_groups,
                footer_group_offsets,
            });
        }
    }
    out
}

fn apply_pdb_zero_tranrf_repair(usb_root: &Path, pages: &[ZeroTranrfPage]) -> BackendResult<usize> {
    if pages.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let Some(page_size) = bytes
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
    else {
        return Err(BackendError::Validation(
            "PDB too small to read page size".into(),
        ));
    };
    let mut patched = 0usize;
    for page in pages {
        let page_base = page.page_index * page_size;
        let mut fixed_any = false;
        for (group_idx, ((&rowpf, &existing_tranrf), &footer_off)) in page
            .rowpf_groups
            .iter()
            .zip(page.tranrf_groups.iter())
            .zip(page.footer_group_offsets.iter())
            .enumerate()
        {
            if rowpf == 0 || existing_tranrf != 0 {
                continue;
            }
            let tranrf_val: u16 = if page.u5 == 1 {
                // u5=1: only the last group gets 1<<(num_rl%16); non-last groups are correct at 0.
                if group_idx != page.num_rl / 16 {
                    continue;
                }
                1u16 << (page.num_rl % 16)
            } else {
                rowpf
            };
            let tranrf_off = page_base + footer_off + 2;
            if tranrf_off + 2 > bytes.len() {
                continue;
            }
            bytes[tranrf_off..tranrf_off + 2].copy_from_slice(&tranrf_val.to_le_bytes());
            fixed_any = true;
        }
        if fixed_any {
            patched += 1;
        }
    }
    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

fn edb_track_analysis_key(track: &crate::models::UsbTrack) -> String {
    normalize_analysis_path_for_identity(
        track
            .usb_analysis_path_raw
            .as_deref()
            .or(track.usb_analysis_path.as_deref())
            .unwrap_or_default(),
    )
}

pub(crate) fn playlist_requires_strict_upgrade(detail: &UsbParityPlaylistDetail) -> bool {
    !matches!(detail.status, DiagStatus::Pass)
        || detail.only_in_pdb > 0
        || detail.only_in_edb > 0
        || detail.order_mismatch
        || detail.path_mismatch_tracks > 0
        || detail.dictionary_id_issue_tracks > 0
        || detail.pdb_duplicate_entries > 0
        || detail.pdb_missing_core_metadata > 0
        || !detail.playlist_id_match
        || !detail.sort_order_match
}

fn strict_raw_coverage_issue_from_parity_checks(checks: &[DiagCheck]) -> Option<String> {
    let raw_check = checks
        .iter()
        .find(|check| check.label == "USB audio directory raw coverage parity (strict)")?;
    if matches!(raw_check.status, DiagStatus::Pass) {
        return None;
    }
    Some(format!(
        "strict raw-count parity drift: {}",
        raw_check.detail
    ))
}

/// (history_id, sequence_no, name, attribute, history_id_parent)
type HistoryRow = (i64, i64, String, i64, i64);
/// (history_id, content_id, sequence_no)
type HistoryContentRow = (i64, i64, i64);

fn derive_history_sync_payload(
    parsed: &crate::pdb_reader::ParsedPdb,
) -> (Vec<HistoryRow>, Vec<HistoryContentRow>) {
    let history_rows = parsed
        .history_playlists
        .iter()
        .filter_map(|h| {
            if !h.name.starts_with("HISTORY ") {
                return None;
            }
            let id = i64::from(h.id);
            if id <= 0 {
                return None;
            }
            Some((id, 0_i64, h.name.clone(), 0_i64, 0_i64))
        })
        .collect::<Vec<_>>();

    let valid_history_ids = history_rows
        .iter()
        .map(|(id, _, _, _, _)| *id)
        .collect::<HashSet<_>>();

    let history_content_rows = parsed
        .history_entries
        .iter()
        .filter_map(|e| {
            let track_id = e.track_id?;
            if track_id == 0 {
                return None;
            }
            let history_id = i64::from(e.playlist_id);
            if !valid_history_ids.contains(&history_id) {
                return None;
            }
            Some((history_id, i64::from(track_id), i64::from(e.entry_index)))
        })
        .collect::<Vec<_>>();

    (history_rows, history_content_rows)
}

fn normalize_player_menu_name(raw: &str) -> String {
    raw.replace(['\u{fffa}', '\u{fffb}'], "").trim().to_string()
}

#[derive(Debug, Clone)]
struct EdbMenuRow {
    menu_item_id: u32,
    kind: u32,
    name: String,
    sequence_no: Option<u32>,
    is_visible: bool,
}

fn load_edb_menu_rows(
    usb_root: &std::path::Path,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<Vec<EdbMenuRow>> {
    let Some(conn) = open_edb_rw(usb_root, warnings) else {
        return Err(crate::error::BackendError::Validation(
            "unable to open eDB for player menu config".to_string(),
        ));
    };
    if !table_exists(&conn, "menuItem") || !table_exists(&conn, "category") {
        return Err(crate::error::BackendError::Validation(
            "eDB missing menuItem/category tables".to_string(),
        ));
    }
    let mut stmt = conn.prepare(
        r#"
        SELECT m.menuItem_id,
               COALESCE(m.kind, 0),
               COALESCE(m.name, ''),
               c.sequenceNo,
               COALESCE(c.isVisible, 0)
        FROM menuItem m
        LEFT JOIN category c ON c.menuItem_id = m.menuItem_id
        ORDER BY COALESCE(c.sequenceNo, 2147483647), m.menuItem_id
        "#,
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(EdbMenuRow {
                menu_item_id: u32::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
                kind: u32::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                name: normalize_player_menu_name(&row.get::<_, String>(2)?),
                sequence_no: row
                    .get::<_, Option<i64>>(3)?
                    .and_then(|v| u32::try_from(v).ok()),
                is_visible: row.get::<_, i64>(4).unwrap_or(0) == 1,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Load the player menu state from a USB, treating eDB.category as the master source.
///
/// eDB is what the player software reads; PDB t16 is a secondary index that
/// must mirror eDB. `current` is the eDB-visible set in sequenceNo order;
/// `available` is the full eDB menuItem catalog minus the visible set.
///
/// `divergence` summarises where PDB t16 diverges from the eDB visible set.
/// Non-empty divergence means the two indexes disagree; Fix PDB sync resolves it.
pub(crate) fn load_usb_player_menu_config_public(
    usb_root: &std::path::Path,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<(
    Vec<UsbPlayerMenuItem>,
    Vec<UsbPlayerMenuItem>,
    UsbPlayerMenuDivergence,
)> {
    load_usb_player_menu_config(usb_root, warnings)
}

/// Build PDB t17 8-byte rows from the current eDB.category state.
/// Used to keep PDB t17 in sync with eDB.category after a menu config save.
fn load_t17_encoded_rows(
    usb_root: &std::path::Path,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<Vec<[u8; 8]>> {
    let Some(conn) = open_edb_from_usb_root(usb_root, warnings) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(
        "SELECT c.category_id, c.menuItem_id, c.sequenceNo, c.isVisible, m.kind
         FROM category c
         JOIN menuItem m ON m.menuItem_id = c.menuItem_id
         ORDER BY c.category_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (cat_id, menu_item_id, seq_no, is_visible, kind) = r?;
        out.push(super::export_helpers::encode_pdb_t17_cat_row(
            u16::try_from(menu_item_id).unwrap_or(0),
            u16::try_from(cat_id).unwrap_or(0),
            is_visible != 0,
            u32::try_from(kind).unwrap_or(0),
            u16::try_from(seq_no).unwrap_or(0),
        ));
    }
    Ok(out)
}

fn load_usb_player_menu_config(
    usb_root: &std::path::Path,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<(
    Vec<UsbPlayerMenuItem>,
    Vec<UsbPlayerMenuItem>,
    UsbPlayerMenuDivergence,
)> {
    let edb_rows = load_edb_menu_rows(usb_root, warnings)?;
    let pdb_rows = super::export_helpers::load_pdb_t16_decoded(usb_root)?;

    let pdb_kinds: HashSet<u32> = pdb_rows.iter().map(|r| u32::from(r.kind)).collect();

    // current = eDB visible items in eDB sequenceNo order.
    let mut edb_visible: Vec<&EdbMenuRow> = edb_rows.iter().filter(|r| r.is_visible).collect();
    edb_visible.sort_by_key(|r| (r.sequence_no.unwrap_or(u32::MAX), r.menu_item_id));
    let edb_visible_kinds: HashSet<u32> = edb_visible.iter().map(|r| r.kind).collect();

    let mut current = Vec::<UsbPlayerMenuItem>::with_capacity(edb_visible.len());
    for (seq, row) in edb_visible.iter().enumerate() {
        let origin = if pdb_kinds.contains(&row.kind) {
            UsbPlayerMenuItemOrigin::Both
        } else {
            UsbPlayerMenuItemOrigin::EdbOnly
        };
        current.push(UsbPlayerMenuItem {
            menu_item_id: row.menu_item_id,
            kind: row.kind,
            name: row.name.clone(),
            is_visible: true,
            sequence_no: Some(u32::try_from(seq).unwrap_or(u32::MAX)),
            origin,
        });
    }

    // available = all eDB menuItems not currently visible.
    let mut available = Vec::<UsbPlayerMenuItem>::new();
    for row in &edb_rows {
        if row.is_visible {
            continue;
        }
        let origin = if pdb_kinds.contains(&row.kind) {
            UsbPlayerMenuItemOrigin::PdbOnly
        } else {
            UsbPlayerMenuItemOrigin::EdbOnly
        };
        available.push(UsbPlayerMenuItem {
            menu_item_id: row.menu_item_id,
            kind: row.kind,
            name: row.name.clone(),
            is_visible: false,
            sequence_no: None,
            origin,
        });
    }
    available.sort_by_key(|item| item.menu_item_id);

    // divergence = where PDB t16 diverges from the eDB visible set.
    let mut in_pdb_only: Vec<u32> = pdb_kinds
        .iter()
        .copied()
        .filter(|k| !edb_visible_kinds.contains(k))
        .collect();
    let mut in_edb_visible_only: Vec<u32> = edb_visible_kinds
        .iter()
        .copied()
        .filter(|k| !pdb_kinds.contains(k))
        .collect();
    in_pdb_only.sort_unstable();
    in_pdb_only.dedup();
    in_edb_visible_only.sort_unstable();
    in_edb_visible_only.dedup();

    let pdb_order: Vec<u32> = pdb_rows.iter().map(|r| u32::from(r.kind)).collect();
    let common_from_pdb: Vec<u32> = pdb_order
        .iter()
        .copied()
        .filter(|k| edb_visible_kinds.contains(k))
        .collect();
    let common_from_edb: Vec<u32> = edb_visible
        .iter()
        .map(|r| r.kind)
        .filter(|k| pdb_kinds.contains(k))
        .collect();
    let order_mismatch = common_from_pdb != common_from_edb;

    let edb_all_kinds: HashSet<u32> = edb_rows.iter().map(|r| r.kind).collect();
    let mut pdb_missing_kinds: Vec<u32> = edb_all_kinds
        .iter()
        .copied()
        .filter(|k| !pdb_kinds.contains(k))
        .collect();
    pdb_missing_kinds.sort_unstable();

    Ok((
        current,
        available,
        UsbPlayerMenuDivergence {
            in_edb_visible_only,
            in_pdb_only,
            order_mismatch,
            pdb_missing_kinds,
        },
    ))
}

/// Mirror eDB `category` rows from an ordered list of PDB t16 kinds.
///
/// PDB is master; eDB must reflect the same kind set in the same order or
/// Newer players may refuse the USB. For each kind in `pdb_kind_order`, the matching
/// eDB `category` row is set `isVisible = 1` with `sequenceNo = i`; if no
/// category row exists for that kind's menuItem, one is inserted. All other
/// eDB `category` rows are set `isVisible = 0`. Kinds in PDB that have no
/// matching eDB `menuItem` row are left unmirrored (eDB cannot show them);
/// this is surfaced via a warning so callers can notice.
fn mirror_edb_category_from_pdb_kinds(
    usb_root: &std::path::Path,
    pdb_kind_order: &[u32],
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<bool> {
    let Some(mut conn) = open_edb_rw(usb_root, warnings) else {
        return Err(crate::error::BackendError::Validation(
            "unable to open eDB for player menu update".to_string(),
        ));
    };
    if !table_exists(&conn, "menuItem") || !table_exists(&conn, "category") {
        return Err(crate::error::BackendError::Validation(
            "eDB missing menuItem/category tables".to_string(),
        ));
    }

    let mut desired_kinds = Vec::<u32>::new();
    let mut seen = HashSet::<u32>::new();
    for kind in pdb_kind_order {
        if seen.insert(*kind) {
            desired_kinds.push(*kind);
        }
    }

    let menu_item_id_by_kind: HashMap<u32, i64> = {
        let mut stmt = conn.prepare("SELECT menuItem_id, kind FROM menuItem")?;
        stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?
            .filter_map(|result| result.ok())
            .filter_map(|(id, kind)| u32::try_from(kind).ok().map(|k| (k, id)))
            .collect()
    };

    {
        let tx = conn.transaction()?;

        let category_rows = {
            let mut stmt = tx.prepare("SELECT category_id, menuItem_id FROM category")?;
            stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut category_by_menu = HashMap::<i64, i64>::new();
        let mut max_category_id = 0i64;
        for (category_id, menu_item_id) in category_rows {
            if category_id > max_category_id {
                max_category_id = category_id;
            }
            category_by_menu.entry(menu_item_id).or_insert(category_id);
        }

        let mut mirrored_menu_ids = HashSet::<i64>::new();
        for (seq, kind) in desired_kinds.iter().enumerate() {
            let Some(menu_id) = menu_item_id_by_kind.get(kind).copied() else {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-player-menu",
                    "usb.player-menu.pdb-kind-unmapped",
                    format!("PDB kind {kind} has no matching eDB menuItem; eDB cannot mirror it"),
                ));
                continue;
            };
            mirrored_menu_ids.insert(menu_id);
            let seq_i64 = i64::try_from(seq + 1).unwrap_or(1);
            if let Some(category_id) = category_by_menu.get(&menu_id).copied() {
                tx.execute(
                    "UPDATE category SET isVisible = 1, sequenceNo = ?2 WHERE category_id = ?1",
                    params![category_id, seq_i64],
                )?;
            } else {
                max_category_id += 1;
                tx.execute(
                    "INSERT INTO category (category_id, menuItem_id, sequenceNo, isVisible) VALUES (?1, ?2, ?3, 1)",
                    params![max_category_id, menu_id, seq_i64],
                )?;
                category_by_menu.insert(menu_id, max_category_id);
            }
        }

        for (menu_id, category_id) in category_by_menu.iter() {
            if mirrored_menu_ids.contains(menu_id) {
                continue;
            }
            tx.execute(
                "UPDATE category SET isVisible = 0, sequenceNo = 0 WHERE category_id = ?1",
                params![category_id],
            )?;
        }

        tx.commit()?;
    }
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
    Ok(true)
}

/// Represents a merged track produced during collect-merge-write.
/// Holds PDB track row data + eDB metadata for eDB write-back.
#[derive(Debug, Clone)]
struct MergedTrack {
    pdb_track_id: u32,
    pdb_row: PdbTrackRowData,
    /// Canonical media path for eDB (e.g. "/Contents/Artist/Album/file.mp3").
    edb_media_path: String,
    /// eDB fields for write-back.
    title: String,
    artist: String,
    album: Option<String>,
    key: Option<String>,
    track_number: Option<u32>,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
    artwork_path: Option<String>,
    analysis_path: String,
    /// PDB technical fields preserved for eDB write-back.
    sample_rate_hz: Option<u32>,
    bitrate_kbps: Option<u32>,
    bit_depth: Option<u32>,
    file_size_bytes: Option<u32>,
    file_type: Option<u32>,
    isrc: Option<String>,
    release_year: Option<u32>,
    release_date: Option<String>,
}

/// Represents a merged playlist with its identity and ordered track list.
#[derive(Debug, Clone)]
struct MergedPlaylist {
    name: String,
    playlist_id: u32,
    sort_order: u32,
    tracks: Vec<MergedTrack>,
    /// True if eDB needs to be updated for this playlist.
    edb_needs_write: bool,
}

fn sorted_edb_playlist_names(
    edb_playlists: Option<&HashMap<String, ExportDbPlaylist>>,
) -> Vec<String> {
    let Some(edb_playlists) = edb_playlists else {
        return Vec::new();
    };
    let mut names = edb_playlists
        .iter()
        .map(|(name, playlist)| {
            (
                name.clone(),
                playlist.sort_order,
                playlist.playlist_id,
                canonicalize_playlist_name(name),
            )
        })
        .collect::<Vec<_>>();
    names.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
            .then_with(|| a.0.cmp(&b.0))
    });
    names.into_iter().map(|(name, _, _, _)| name).collect()
}

fn strict_repair_ordered_playlist_names(
    parsed: &crate::pdb_reader::ParsedPdb,
    edb_playlists: Option<&HashMap<String, ExportDbPlaylist>>,
) -> Vec<String> {
    let sorted_edb_names = sorted_edb_playlist_names(edb_playlists);
    let edb_name_set = sorted_edb_names.iter().cloned().collect::<HashSet<_>>();
    let mut edb_names_by_canon = HashMap::<String, Vec<String>>::new();
    for name in &sorted_edb_names {
        edb_names_by_canon
            .entry(canonicalize_playlist_name(name))
            .or_default()
            .push(name.clone());
    }

    let mut pdb_leaves = parsed
        .playlist_tree
        .iter()
        .filter(|row| !row.row_is_folder)
        .collect::<Vec<_>>();
    pdb_leaves.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.name.cmp(&b.name))
    });

    let mut out = Vec::<String>::new();
    let mut seen_canon = HashSet::<String>::new();
    let mut consumed_edb_names = HashSet::<String>::new();
    for leaf in pdb_leaves {
        let canon = canonicalize_playlist_name(&leaf.name);
        if !seen_canon.insert(canon.clone()) {
            continue;
        }
        let chosen = if edb_name_set.contains(&leaf.name) {
            leaf.name.clone()
        } else {
            edb_names_by_canon
                .get(&canon)
                .and_then(|names| {
                    names
                        .iter()
                        .find(|name| !consumed_edb_names.contains(*name))
                        .cloned()
                })
                .unwrap_or_else(|| leaf.name.clone())
        };
        if edb_name_set.contains(&chosen) {
            consumed_edb_names.insert(chosen.clone());
        }
        out.push(chosen);
    }

    for name in sorted_edb_names {
        let canon = canonicalize_playlist_name(&name);
        if seen_canon.insert(canon) {
            out.push(name);
        }
    }

    out
}

fn pdb_page_size_from_bytes(bytes: &[u8]) -> Option<usize> {
    bytes
        .get(4..8)
        .and_then(|raw| raw.try_into().ok())
        .map(u32::from_le_bytes)
        .and_then(|value| usize::try_from(value).ok())
}

pub(crate) fn restore_pdb_playlist_sort_orders(
    usb_root: &std::path::Path,
    desired_sort_by_id: &HashMap<u32, u32>,
) -> BackendResult<usize> {
    if desired_sort_by_id.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    if !pdb_path.is_file() {
        return Ok(0);
    }
    let mut bytes = std::fs::read(&pdb_path)?;
    let page_size = pdb_page_size_from_bytes(&bytes).ok_or_else(|| {
        BackendError::Validation(format!(
            "PDB playlist order restore failed: cannot read page size from {}",
            pdb_path.display()
        ))
    })?;
    let patches = desired_sort_by_id
        .iter()
        .map(
            |(id, sort_order)| crate::pdb_writer::PdbPlaylistTreeSortOrderPatch {
                id: *id,
                sort_order: *sort_order,
            },
        )
        .collect::<Vec<_>>();
    let patched = crate::pdb_writer::patch_playlist_tree_sort_orders_in_place(
        &mut bytes, &patches, page_size,
    )?;
    if patched > 0 {
        std::fs::write(&pdb_path, &bytes)?;
    }
    Ok(patched)
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

pub(crate) fn sync_edb_playlist_sort_orders_from_pdb(
    usb_root: &std::path::Path,
    edb_conn: Option<&mut rusqlite::Connection>,
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<usize> {
    let parsed = parse_pdb(&usb_staging::stage_pdb(usb_root)?)?;
    let Some(mut handle) = EdbConnHandle::acquire(edb_conn, || open_edb_rw(usb_root, warnings))
    else {
        warnings.push(logging::log(
            Level::Error,
            "usb-repair",
            "usb.repair.strict-parity.order-sync-edb-open-failed",
            "strict parity upgrade: unable to open eDB for playlist order sync",
        ));
        return Ok(0);
    };
    let conn = handle.conn();
    if !table_exists(conn, "playlist") {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    let mut updated = 0usize;
    for leaf in parsed.playlist_tree.iter().filter(|row| !row.row_is_folder) {
        updated += tx.execute(
            "UPDATE playlist SET sequenceNo = ?1 WHERE playlist_id = ?2 AND attribute = 0",
            params![i64::from(leaf.sort_order), i64::from(leaf.id)],
        )?;
    }
    tx.commit()?;
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
    Ok(updated)
}

fn build_manifest_for_merged_playlist(
    usb_root: &std::path::Path,
    playlist: &MergedPlaylist,
) -> (ExportPlaylistData, ExportManifest) {
    let playlist_data = ExportPlaylistData {
        id: format!("usb-pl-{}", playlist.playlist_id),
        name: playlist.name.clone(),
        tracks: Vec::new(),
    };
    let manifest_tracks: Vec<ExportManifestTrack> = playlist
        .tracks
        .iter()
        .enumerate()
        .map(|(idx, mt)| ExportManifestTrack {
            id: format!("merged-{}", mt.pdb_track_id),
            master_db_id: mt.pdb_row.master_db_id.map(i64::from),
            master_content_id: mt.pdb_row.master_content_id.map(i64::from),
            content_link: mt.pdb_row.content_link.map(i64::from),
            position: idx + 1,
            track_number: mt.track_number,
            title: mt.title.clone(),
            artist: mt.artist.clone(),
            album: mt.album.clone(),
            bpm: mt.bpm,
            key: mt.key.clone(),
            source_path: mt.edb_media_path.clone(),
            exported_path: mt.edb_media_path.clone(),
            file_modified_at: None,
            file_size_bytes: mt.file_size_bytes.map(i64::from),
            sample_rate_hz: mt.sample_rate_hz,
            bit_depth: mt.bit_depth.map(|v| v as u8),
            bitrate_kbps: mt.bitrate_kbps,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: None,
            color_id: None,
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: mt.isrc.clone(),
            release_year: mt.release_year,
            release_date: mt.release_date.clone(),
            recorded_date: None,
            file_type: mt.file_type.map(i64::from),
            owns_exported_media: false,
            owns_artwork: false,
            owns_waveform: false,
            artwork_path: mt.artwork_path.clone(),
            waveform_path: (!mt.analysis_path.is_empty()).then(|| mt.analysis_path.clone()),
            duration_ms: mt.duration_ms,
        })
        .collect();
    let manifest = ExportManifest {
        version: 1,
        generated_at: "1970-01-01T00:00:00Z".to_string(),
        playlist_id: playlist_data.id.clone(),
        playlist_name: playlist.name.clone(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: true,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: manifest_tracks.len(),
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: manifest_tracks,
    };
    (playlist_data, manifest)
}

/// Merged track metadata fields shared by [`build_merged_pdb_track_row`] call sites.
struct MergedTrackFields<'a> {
    title: &'a str,
    artist: &'a str,
    album: &'a Option<String>,
    key: &'a Option<String>,
    track_number: Option<u32>,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
    media_path: &'a str,
    analysis_path: &'a str,
}

/// Build a PDB track row from merged metadata.
/// When an existing PDB track is available, preserve non-core linkage fields.
fn build_merged_pdb_track_row(
    track_id: u32,
    existing_pdb: Option<&crate::pdb_reader::PdbTrackRow>,
    fields: MergedTrackFields,
    parsed: &crate::pdb_reader::ParsedPdb,
) -> PdbTrackRowData {
    let MergedTrackFields {
        title,
        artist,
        album,
        key,
        track_number,
        bpm,
        duration_ms,
        media_path,
        analysis_path,
    } = fields;
    // Find or create dictionary IDs for artist/album/key
    let artist_id = if !artist.is_empty() {
        let canon = canonicalize_playlist_name(artist);
        parsed
            .artists
            .iter()
            .find(|(_, v)| canonicalize_playlist_name(v) == canon)
            .map(|(id, _)| *id)
            .or_else(|| existing_pdb.map(|t| t.artist_id))
            .unwrap_or(0)
    } else {
        existing_pdb.map(|t| t.artist_id).unwrap_or(0)
    };
    let album_id = if let Some(alb) = album {
        let canon = canonicalize_playlist_name(alb);
        parsed
            .albums
            .iter()
            .find(|(_, v)| canonicalize_playlist_name(v) == canon)
            .map(|(id, _)| *id)
            .or_else(|| existing_pdb.map(|t| t.album_id))
            .unwrap_or(0)
    } else {
        existing_pdb.map(|t| t.album_id).unwrap_or(0)
    };
    let key_id = if let Some(k) = key {
        parsed
            .keys
            .iter()
            .find(|(_, v)| v.trim().eq_ignore_ascii_case(k.trim()))
            .map(|(id, _)| *id)
            .or_else(|| existing_pdb.map(|t| t.key_id))
            .unwrap_or(0)
    } else {
        existing_pdb.map(|t| t.key_id).unwrap_or(0)
    };

    PdbTrackRowData {
        header_flags_u32: None,
        id: track_id,
        title: title.to_string(),
        file_path: media_path.to_string(),
        anlz_path: analysis_path.to_string(),
        artist_id,
        album_id,
        key_id,
        genre_id: existing_pdb.map(|t| t.genre_id).unwrap_or(0),
        artwork_id: existing_pdb.map(|t| t.artwork_id).unwrap_or(0),
        track_number,
        bpm,
        duration_seconds: duration_ms.map(|ms| (ms / 1000) as u32),
        content_link: existing_pdb.and_then(|t| t.content_link),
        sample_rate_hz: existing_pdb.and_then(|t| t.sample_rate_hz),
        file_size_bytes: existing_pdb.and_then(|t| t.file_size_bytes),
        master_content_id: existing_pdb.and_then(|t| t.master_content_id),
        master_db_id: existing_pdb.and_then(|t| t.master_db_id),
        bitrate_kbps: existing_pdb.and_then(|t| t.bitrate_kbps),
        release_year: existing_pdb.and_then(|t| t.release_year),
        bit_depth: existing_pdb.and_then(|t| t.bit_depth),
        file_type: existing_pdb.and_then(|t| t.file_type),
        isrc: existing_pdb.and_then(|t| t.isrc.clone()),
        date_added: existing_pdb.and_then(|t| t.date_added.clone()),
        release_date: existing_pdb.and_then(|t| t.release_date.clone()),
        dj_comment: existing_pdb.and_then(|t| t.dj_comment.clone()),
        file_name: existing_pdb.and_then(|t| t.file_name.clone()),
        publish_track_info_on: existing_pdb.and_then(|t| {
            t.publish_track_info
                .as_deref()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("on"))
        }),
        autoload_hotcues_on: existing_pdb.and_then(|t| {
            t.autoload_hotcues
                .as_deref()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("on"))
        }),
    }
}

fn resolved_strict_artwork_path(
    usb_root: &std::path::Path,
    pdb_track: Option<&crate::pdb_reader::PdbTrackRow>,
    parsed: &crate::pdb_reader::ParsedPdb,
    edb_artwork_path: Option<&str>,
) -> Option<String> {
    edb_artwork_path
        .filter(|path| !path.trim().is_empty())
        .map(|path| {
            super::export_helpers::to_usb_relative_path(usb_root, path)
                .unwrap_or_else(|| path.to_string())
        })
        .or_else(|| {
            pdb_track
                .and_then(|track| parsed.artworks.get(&track.artwork_id))
                .filter(|path| !path.trim().is_empty())
                .cloned()
        })
}

impl BackendService {
    pub fn get_usb_player_menu_config(
        &self,
        req: GetUsbPlayerMenuConfigRequest,
    ) -> BackendResult<GetUsbPlayerMenuConfigData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut warnings = Vec::<WarningEntry>::new();
        let (current_items, available_items, divergence) =
            load_usb_player_menu_config(&usb_root, &mut warnings)?;
        Ok(GetUsbPlayerMenuConfigData {
            current_items,
            available_items,
            divergence,
            warnings,
        })
    }

    /// Restore PDB t16 from the full eDB menuItem catalog.
    ///
    /// Reads ALL eDB menuItem rows (visible and hidden) and writes them to PDB
    /// t16 in menuItem_id order. This repairs USBs where PDB was trimmed by
    /// old code, restoring the full browse category set.
    pub fn sync_usb_player_menu_edb_to_pdb(
        &self,
        req: GetUsbPlayerMenuConfigRequest,
    ) -> BackendResult<UpdateUsbPlayerMenuConfigData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut warnings = Vec::<WarningEntry>::new();
        warnings.extend(
            self.backup_usb_databases_with_retention(&usb_root, "Before repair", None)
                .into_iter()
                .map(|message| {
                    logging::log(
                        Level::Info,
                        "usb-player-menu",
                        "usb.player-menu.backup",
                        message,
                    )
                }),
        );

        let mut edb_rows = load_edb_menu_rows(&usb_root, &mut warnings)?;
        edb_rows.sort_by_key(|r| r.menu_item_id);

        let all_rows: Vec<(u16, String)> = edb_rows
            .iter()
            .filter_map(|r| u16::try_from(r.kind).ok().map(|k| (k, r.name.clone())))
            .collect();

        let pdb_updated =
            super::export_helpers::patch_pdb_columns_menu_set_by_kind(&usb_root, &all_rows)?;
        if pdb_updated {
            warnings.push(logging::log(
                Level::Info,
                "usb-player-menu",
                "usb.player-menu.t16-restored",
                "PDB t16 columns restored from eDB menuItem catalog",
            ));
        }

        match load_t17_encoded_rows(&usb_root, &mut warnings) {
            Ok(t17_rows) if !t17_rows.is_empty() => {
                match super::export_helpers::patch_pdb_t17_category_snapshot(&usb_root, &t17_rows) {
                    Ok(true) => {
                        warnings.push(logging::log(
                            Level::Info,
                            "usb-player-menu",
                            "usb.player-menu.t17-snapshot-updated",
                            "PDB t17 category snapshot updated",
                        ));
                    }
                    Ok(false) => {}
                    Err(e) => {
                        warnings.push(logging::log(
                            Level::Warn,
                            "usb-player-menu",
                            "usb.player-menu.t17-update-skipped",
                            format!("PDB t17 update skipped: {e}"),
                        ));
                    }
                }
            }
            _ => {}
        }

        if let Err(err) = usb_staging::write_back_if_changed(&usb_root, usb_staging::DbKind::Pdb) {
            warnings.push(logging::log(
                Level::Error,
                "usb-player-menu",
                "usb.player-menu.pdb-write-back-failed",
                format!("PDB write-back to USB failed: {err}"),
            ));
        }

        let (current_items, available_items, divergence) =
            load_usb_player_menu_config(&usb_root, &mut warnings)?;

        Ok(UpdateUsbPlayerMenuConfigData {
            updated: pdb_updated,
            current_items,
            available_items,
            divergence,
            warnings,
        })
    }

    pub fn update_usb_player_menu_config(
        &self,
        req: UpdateUsbPlayerMenuConfigRequest,
    ) -> BackendResult<UpdateUsbPlayerMenuConfigData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut warnings = Vec::<WarningEntry>::new();
        warnings.extend(
            self.backup_usb_databases_with_retention(&usb_root, "Before menu update", None)
                .into_iter()
                .map(|message| {
                    logging::log(
                        Level::Info,
                        "usb-player-menu",
                        "usb.player-menu.backup",
                        message,
                    )
                }),
        );

        let (before_current, before_available, _before_divergence) =
            load_usb_player_menu_config(&usb_root, &mut warnings)?;

        // Build lookup: (kind -> display name), preferring PDB-sourced names
        // (they came from the stick itself) and falling back to eDB names for
        // kinds only available there.
        let mut name_by_kind = HashMap::<u32, String>::new();
        for item in &before_available {
            name_by_kind.insert(item.kind, item.name.clone());
        }
        for item in &before_current {
            name_by_kind.insert(item.kind, item.name.clone());
        }
        let kind_by_menu: HashMap<u32, u32> = before_current
            .iter()
            .chain(before_available.iter())
            .filter(|item| item.menu_item_id != 0)
            .map(|item| (item.menu_item_id, item.kind))
            .collect();

        // Resolve the submitted selection into an ordered, deduped list of
        // kinds. `current_kinds` wins when present; otherwise we fall back to
        // `current_menu_item_ids` (which can only address eDB-backed items).
        let mut desired_kinds = Vec::<u32>::new();
        let mut seen = HashSet::<u32>::new();
        if !req.current_kinds.is_empty() {
            for kind in &req.current_kinds {
                if seen.insert(*kind) {
                    desired_kinds.push(*kind);
                }
            }
        } else {
            for menu_id in &req.current_menu_item_ids {
                if let Some(kind) = kind_by_menu.get(menu_id).copied()
                    && seen.insert(kind)
                {
                    desired_kinds.push(kind);
                }
            }
        }

        // Reject if any currently-present protected kind is absent from the request.
        let desired_set: HashSet<u32> = desired_kinds.iter().copied().collect();
        for &kind in REQUIRED_PLAYER_MENU_KINDS {
            if before_current.iter().any(|item| item.kind == kind) && !desired_set.contains(&kind) {
                let name = name_by_kind.get(&kind).map(|s| s.as_str()).unwrap_or("?");
                return Err(crate::error::BackendError::Validation(format!(
                    "Player menu item \"{name}\" (kind {kind}) cannot be removed"
                )));
            }
        }

        // Validate desired kinds: drop any with no known display name.
        let mut final_kinds = Vec::<u32>::with_capacity(desired_kinds.len());
        for kind in &desired_kinds {
            if !name_by_kind.contains_key(kind) {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-player-menu",
                    "usb.player-menu.dropped-unknown-kind",
                    format!("dropped unknown player menu kind {kind}: no display name available"),
                ));
                continue;
            }
            final_kinds.push(*kind);
        }

        // Snapshot eDB for rollback if the write fails. Staged path: this must
        // match whatever path `mirror_edb_category_from_pdb_kinds`'s
        // `open_edb_rw` actually mutates (the local staged copy when staging
        // is active), or the snapshot/restore would operate on a file the
        // write never touched.
        let edb_path = usb_staging::stage_edb(&usb_root)?;
        let edb_snapshot = if edb_path.is_file() {
            std::fs::read(&edb_path).ok()
        } else {
            None
        };

        if let Err(err) = mirror_edb_category_from_pdb_kinds(&usb_root, &final_kinds, &mut warnings)
        {
            warnings.push(logging::log(
                Level::Error,
                "usb-player-menu",
                "usb.player-menu.edb-write-failed",
                format!("eDB write failed ({err}); restoring eDB from snapshot"),
            ));
            if let Some(snapshot) = edb_snapshot.as_ref() {
                let _ = std::fs::write(&edb_path, snapshot);
            }
            // Deliberately no write_back_if_changed here: the local copy was
            // just restored to its pre-attempt state, so the real USB drive
            // must never see this failed attempt.
            return Err(err);
        }
        if let Err(err) = usb_staging::write_back_if_changed(&usb_root, usb_staging::DbKind::Edb) {
            warnings.push(logging::log(
                Level::Error,
                "usb-player-menu",
                "usb.player-menu.edb-write-back-failed",
                format!("eDB write-back to USB failed: {err}"),
            ));
        }

        let (current_items, available_items, divergence) =
            load_usb_player_menu_config(&usb_root, &mut warnings)?;
        let before_kinds = before_current
            .iter()
            .map(|item| item.kind)
            .collect::<Vec<_>>();
        let after_kinds = current_items
            .iter()
            .map(|item| item.kind)
            .collect::<Vec<_>>();
        let updated = before_kinds != after_kinds;

        if updated {
            match load_t17_encoded_rows(&usb_root, &mut warnings) {
                Ok(t17_rows) if !t17_rows.is_empty() => {
                    match super::export_helpers::patch_pdb_t17_category_snapshot(
                        &usb_root, &t17_rows,
                    ) {
                        Ok(true) => {
                            warnings.push(logging::log(
                                Level::Info,
                                "usb-player-menu",
                                "usb.player-menu.t17-snapshot-updated",
                                "PDB t17 category snapshot updated",
                            ));
                        }
                        Ok(false) => {}
                        Err(e) => {
                            warnings.push(logging::log(
                                Level::Warn,
                                "usb-player-menu",
                                "usb.player-menu.t17-update-skipped",
                                format!("PDB t17 update skipped: {e}"),
                            ));
                        }
                    }
                }
                _ => {}
            }
            if let Err(err) =
                usb_staging::write_back_if_changed(&usb_root, usb_staging::DbKind::Pdb)
            {
                warnings.push(logging::log(
                    Level::Error,
                    "usb-player-menu",
                    "usb.player-menu.pdb-write-back-failed",
                    format!("PDB write-back to USB failed: {err}"),
                ));
            }
        }

        Ok(UpdateUsbPlayerMenuConfigData {
            updated,
            current_items,
            available_items,
            divergence,
            warnings,
        })
    }

    pub fn repair_usb_diagnostics(
        &self,
        req: RepairUsbDiagnosticsRequest,
    ) -> BackendResult<RepairUsbDiagnosticsData> {
        self.repair_usb_diagnostics_with_progress(req, |_, _, _| {})
    }

    pub fn repair_usb_diagnostics_with_progress<F>(
        &self,
        req: RepairUsbDiagnosticsRequest,
        mut on_progress: F,
    ) -> BackendResult<RepairUsbDiagnosticsData>
    where
        F: FnMut(usize, usize, &str),
    {
        let start = std::time::Instant::now();
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;

        // Open the eDB and stage the PDB exactly once for this whole repair run,
        // then hand the same connection/path to the diagnostics and parity
        // baselines below (and to this function's own eDB reads further down)
        // instead of each independently re-staging/re-opening from the USB
        // device -- see `run_usb_diagnostics_with_progress_impl`.
        let mut edb_open_warnings = Vec::<WarningEntry>::new();
        let edb_conn = open_edb_from_usb_root(&usb_root, &mut edb_open_warnings);
        let staged_pdb_path = usb_staging::stage_pdb(&usb_root)?;

        on_progress(5, 100, "USB: Running diagnostics baseline");
        let (diagnostics, parsed_pdb_from_diagnostics) = self
            .run_usb_diagnostics_with_progress_impl(
                RunUsbDiagnosticsRequest {
                    usb_root: Some(usb_root.to_string_lossy().to_string()),
                },
                edb_conn.as_ref(),
                Some(&staged_pdb_path),
                |c, t, m| {
                    let pct = 5 + ((c * 50) / t.max(1)).min(50);
                    on_progress(pct, 100, m);
                },
            )?;

        on_progress(60, 100, "USB: Building parity guidance");
        let parity = self.run_usb_parity_report_with_progress_impl(
            RunUsbParityReportRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            },
            edb_conn.as_ref(),
            Some(&staged_pdb_path),
            parsed_pdb_from_diagnostics.as_ref(),
            |c, t, m| {
                let pct = 60 + ((c * 20) / t.max(1)).min(20);
                on_progress(pct, 100, m);
            },
        );

        on_progress(85, 100, "USB: Collecting repair opportunities");
        let mut detected_issues = Vec::<String>::new();
        let mut proposed_fixes = Vec::<RepairFixProposal>::new();
        let mut unsupported_items = Vec::<RepairUnsupportedItem>::new();
        let mut applied_fixes = Vec::<String>::new();
        let mut skipped_fixes = Vec::<String>::new();
        let mut failed_fixes = Vec::<String>::new();
        let mut warnings = diagnostics.warnings.clone();
        warnings.append(&mut edb_open_warnings);
        let mut estimated_file_writes = 0usize;
        let estimated_file_deletes = 0usize;
        let mut missing_audio_track_ids = HashSet::<u32>::new();
        let mut missing_audio_paths = Vec::<String>::new();
        let mut unindexed_audio_paths = Vec::<String>::new();
        let mut remove_missing_audio_supported = false;
        let mut sync_edb_history_supported = false;
        let mut sync_edb_history_needed = false;

        let anlz_scan_warnings = scan_anlz_warnings(&usb_root);
        warnings.extend(anlz_scan_warnings.iter().cloned());
        let mut empty_analysis_paths = diagnostics
            .warnings
            .iter()
            .map(|w| w.message.as_str())
            .filter_map(|line| line.strip_prefix("analysis file appears empty: "))
            .map(|s| s.trim().to_string())
            .collect::<Vec<_>>();
        let mut malformed_analysis_entries = diagnostics
            .warnings
            .iter()
            .map(|w| w.message.as_str())
            .filter_map(|line| line.strip_prefix("analysis malformed entry is empty: "))
            .map(|s| s.trim().to_string())
            .collect::<Vec<_>>();
        empty_analysis_paths.extend(anlz_scan_warnings.iter().filter_map(|w| {
            w.message
                .strip_prefix("analysis file appears empty: ")
                .map(str::to_string)
        }));
        malformed_analysis_entries.extend(anlz_scan_warnings.iter().filter_map(|w| {
            w.message
                .strip_prefix("analysis malformed entry is empty: ")
                .map(str::to_string)
        }));
        empty_analysis_paths.sort();
        empty_analysis_paths.dedup();
        malformed_analysis_entries.sort();
        malformed_analysis_entries.dedup();
        if !empty_analysis_paths.is_empty() {
            detected_issues.push(format!(
                "{} empty USB analysis file(s) detected",
                empty_analysis_paths.len()
            ));
            let writes = empty_analysis_paths.len() * 3;
            estimated_file_writes += writes;
            proposed_fixes.push(RepairFixProposal {
                id: "fix_empty_analysis_files".to_string(),
                title: "Fix Empty Analysis Files".to_string(),
                description:
                    "Regenerate missing/empty DAT/EXT/2EX bundles when source audio is resolvable."
                        .to_string(),
                supported: true,
                destructive: false,
                estimated_writes: writes,
                estimated_deletes: 0,
            });
        }
        if !malformed_analysis_entries.is_empty() {
            detected_issues.push(format!(
                "{} malformed entry/entries detected under USBANLZ",
                malformed_analysis_entries.len()
            ));
            unsupported_items.push(RepairUnsupportedItem {
                issue: format!(
                    "{} malformed USBANLZ entry/entries",
                    malformed_analysis_entries.len()
                ),
                reason: "These indicate USB analysis directory corruption or stray files. Inspect Event Log entries tagged 'analysis entry malformed' and re-export affected tracks (or rebuild USB analysis data).".to_string(),
            });
        }

        // Staged once and reused below: these ~15 detectors otherwise each
        // re-read the same PDB file fresh from the USB mount.
        let staged_pdb_path = usb_staging::stage_pdb(&usb_root)?;
        // Reuse the PDB the diagnostics baseline above already parsed instead
        // of parsing it a third time (once for diagnostics, once for parity,
        // and this one) -- only falls back to a fresh parse if diagnostics'
        // own parse failed.
        let parsed_pdb = parsed_pdb_from_diagnostics.or_else(|| parse_pdb(&staged_pdb_path).ok());
        let pdb_sentinel_u5_pages = detect_pdb_sentinel_u5_on_data_pages(&staged_pdb_path);
        let pdb_wrong_flags_pages = detect_pdb_wrong_page_flags(&staged_pdb_path);
        let pdb_zero_tranrf_pages = detect_pdb_zero_tranrf_all_tables(&staged_pdb_path);
        let pdb_wrong_history_shape_pages = detect_pdb_wrong_history_page_shape(&staged_pdb_path);
        let pdb_tombstoned_playlist_ids = detect_pdb_tombstoned_playlist_tree_ids(&staged_pdb_path);
        let pdb_wrong_track_u5_pages = detect_pdb_wrong_track_u5(&staged_pdb_path);
        let pdb_t00_multipage_active_pages =
            detect_pdb_t00_multipage_active_pages(&staged_pdb_path);
        let pdb_stale_sentinel_btree_pages = detect_pdb_stale_sentinel_btree(&staged_pdb_path);
        let pdb_wrong_playlist_tree_shape_pages =
            detect_pdb_wrong_playlist_tree_shape(&staged_pdb_path);
        let pdb_ec_conflicts = detect_pdb_ec_data_page_conflicts(&staged_pdb_path);
        let pdb_truncated_chains = detect_pdb_truncated_table_chains(&staged_pdb_path);
        let pdb_torn_growth = detect_pdb_torn_growth_pages(&staged_pdb_path);
        let pdb_misaligned_slots = detect_pdb_string_misalignment(&staged_pdb_path);
        let pdb_misaligned_track_ids: Vec<u32> = {
            let mut ids: Vec<u32> = pdb_misaligned_slots
                .iter()
                .filter(|m| m.table_type == 0)
                .map(|m| m.id)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            ids.sort_unstable();
            ids
        };
        let pdb_misaligned_album_ids: Vec<u32> = {
            let mut ids: Vec<u32> = pdb_misaligned_slots
                .iter()
                .filter(|m| m.table_type == 3)
                .map(|m| m.id)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            ids.sort_unstable();
            ids
        };
        let pdb_misaligned_artist_ids: Vec<u32> = {
            let mut ids: Vec<u32> = pdb_misaligned_slots
                .iter()
                .filter(|m| m.table_type == 2)
                .map(|m| m.id)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            ids.sort_unstable();
            ids
        };
        if !pdb_zero_tranrf_pages.is_empty() {
            detected_issues.push(format!(
                "{} data page(s) with tranrf=0 and active rows (may be rejected by some DJ software versions)",
                pdb_zero_tranrf_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_ZERO_TRANRF_FIX_ID.to_string(),
                title: "Repair Data Pages With Zero tranrf".to_string(),
                description: format!(
                    "Set tranrf = rowpf on {} data page(s) where tranrf is zero but active rows \
                     exist. Some DJ software versions reject databases where tranrf is cleared on \
                     a live data page.",
                    pdb_zero_tranrf_pages.len()
                ),
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        if !pdb_wrong_flags_pages.is_empty() {
            detected_issues.push(format!(
                "{} PDB data page(s) with wrong page_flags byte (firmware may reject as corrupted)",
                pdb_wrong_flags_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_WRONG_PAGE_FLAGS_FIX_ID.to_string(),
                title: "Repair PDB Data Page Flags".to_string(),
                description: format!(
                    "Patch page_flags byte (0x1b) on {} data page(s) to the correct per-table value \
                     (tracks/history=0x34, all others=0x24).",
                    pdb_wrong_flags_pages.len()
                ),
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        if !pdb_sentinel_u5_pages.is_empty() {
            detected_issues.push(format!(
                "{} PDB data page(s) with sentinel u5=0x1FFF (may be rejected by some DJ software and player firmware)",
                pdb_sentinel_u5_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_SENTINEL_U5_FIX_ID.to_string(),
                title: "Repair PDB Data Pages With Sentinel u5".to_string(),
                description: format!(
                    "Patch u5/num_rl header fields on {} data page(s) from sentinel 0x1FFF to the correct per-table value. \
                     Some DJ software and player firmware versions may reject data pages with u5=0x1FFF.",
                    pdb_sentinel_u5_pages.len()
                ),
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        if !pdb_wrong_history_shape_pages.is_empty() {
            detected_issues.push(format!(
                "{} PDB history/columns page(s) with wrong u5/num_rl shape (may be rejected as corrupted)",
                pdb_wrong_history_shape_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_WRONG_HISTORY_SHAPE_FIX_ID.to_string(),
                title: "Repair PDB History/Columns Page Footer Shape".to_string(),
                description: format!(
                    "Patch u5/num_rl footer fields on {} tt=16/17/18 data page(s) from the wrong \
                     (1, nrs-1) shape to the correct (nrs, 0) convention.",
                    pdb_wrong_history_shape_pages.len()
                ),
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        if !pdb_wrong_track_u5_pages.is_empty() {
            detected_issues.push(format!(
                "{} PDB track page(s) with u5/num_rl shape outside expected values",
                pdb_wrong_track_u5_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_WRONG_TRACK_U5_FIX_ID.to_string(),
                title: "Repair PDB Track Page Footer Shape".to_string(),
                description: format!(
                    "Normalise u5/num_rl footer fields on {} tt=0 track page(s) to (2, 0). \
                     Both (1, nrs-1) and (2, 0) are accepted by player firmware; this repair \
                     aligns the export to the format written by the current writer.",
                    pdb_wrong_track_u5_pages.len()
                ),
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        if !pdb_t00_multipage_active_pages.is_empty() {
            detected_issues.push(format!(
                "{} tt=0 track page(s) with ACTV (0x34) flag in a multi-page chain \
                 (expected SEAL 0x24; may be flagged as corrupted by some DJ software versions)",
                pdb_t00_multipage_active_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_T00_MULTIPAGE_ACTIVE_FIX_ID.to_string(),
                title: "Repair Track Page Flag (Multi-Page Chain)".to_string(),
                description: format!(
                    "Set flags=0x24 (SEAL) and u5/num_rl=(1, nrs-1) on {} tt=0 track page(s). \
                     Single-page track chains use ACTV (0x34); multi-page chains are expected \
                     to use SEAL (0x24). Some DJ software versions may flag ACTV pages in a \
                     multi-page chain as corrupted.",
                    pdb_t00_multipage_active_pages.len()
                ),
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        if !pdb_tombstoned_playlist_ids.is_empty() {
            detected_issues.push(format!(
                "{} tombstoned row(s) with non-zero id in tracks/playlist_tree (may cause duplicate-id rejection)",
                pdb_tombstoned_playlist_ids.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_TOMBSTONED_PLAYLIST_TREE_ID_FIX_ID.to_string(),
                title: "Repair Tombstoned Row IDs".to_string(),
                description: format!(
                    "Zero out the id field in {} tombstoned slot(s) (tracks + playlist_tree) to \
                     prevent duplicate-id rejection when active and dead rows share the same id.",
                    pdb_tombstoned_playlist_ids.len()
                ),
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        if !pdb_stale_sentinel_btree_pages.is_empty() {
            detected_issues.push(format!(
                "{} PDB sentinel page(s) with stale B-tree index (rejected as corrupted — \
                 B-tree entries reference wrong pages after additive export grew the table)",
                pdb_stale_sentinel_btree_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_STALE_SENTINEL_BTREE_FIX_ID.to_string(),
                title: "Repair Stale Sentinel B-Tree Index".to_string(),
                description: format!(
                    "Reset the B-tree index area in {} sentinel page(s) to the empty state \
                     (num_entries=0, first_empty=0x1FFF). The DJ software uses the B-tree to validate \
                     the table chain; stale entries referencing wrong pages cause a 'corrupted' \
                     error. player firmware walks the page chain directly and is unaffected. \
                     Fresh exports already write empty B-trees.",
                    pdb_stale_sentinel_btree_pages.len()
                ),
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        if !pdb_wrong_playlist_tree_shape_pages.is_empty() {
            detected_issues.push(format!(
                "{} PDB playlist_tree page(s) with (u5, num_rl) shape outside expected (nrs, 0)",
                pdb_wrong_playlist_tree_shape_pages.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_WRONG_PLAYLIST_TREE_SHAPE_FIX_ID.to_string(),
                title: "Repair PDB Playlist-Tree Page Footer Shape".to_string(),
                description: format!(
                    "Set u5=nrs and num_rl=0 on {} tt=7 (playlist_tree) data page(s). \
                     The export wrote num_rl=1 instead of 0; the expected shape for this \
                     table is (nrs, 0).",
                    pdb_wrong_playlist_tree_shape_pages.len()
                ),
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        if !pdb_ec_conflicts.is_empty() {
            detected_issues.push(format!(
                "{} table(s) with empty_candidate pointing to another table's data page \
                 (rejected as corrupted — aliased write pointer)",
                pdb_ec_conflicts.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_EC_CONFLICT_FIX_ID.to_string(),
                title: "Repair Table Write-Pointer Conflict".to_string(),
                description: format!(
                    "{} table(s) have an empty_candidate pointer targeting a page already \
                     used as data by a different table. The DJ software validates this invariant \
                     and reports 'database corrupted' when it finds a conflict. The repair \
                     reassigns each conflicting empty_candidate to a free page beyond the \
                     physical file and updates next_unused_page in the header.",
                    pdb_ec_conflicts.len()
                ),
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        if !pdb_truncated_chains.is_empty() {
            detected_issues.push(format!(
                "{} table(s) with a declared last page beyond the physical end of the file \
                 (interrupted export left growth pointers ahead of the actual written data)",
                pdb_truncated_chains.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_TRUNCATED_TABLE_CHAIN_FIX_ID.to_string(),
                title: "Repair Truncated Table Chain".to_string(),
                description: format!(
                    "{} table(s) declare a last page that does not physically exist in the file \
                     (an additive-growth write was interrupted after the table pointer was \
                     advanced but before the new pages were flushed to disk). The repair points \
                     the table's last/empty_candidate fields back at the real last written page; \
                     no page content is changed. This is a structural prerequisite and always \
                     runs before other repairs, including the strict-parity upgrade.",
                    pdb_truncated_chains.len()
                ),
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        if !pdb_torn_growth.is_empty() {
            detected_issues.push(format!(
                "PDB shows signs of a torn additive-growth write (mid-export USB disconnect): \
                 {} empty_candidate page(s) hold non-zero garbage instead of a blank reusable page{}",
                pdb_torn_growth.garbage_ec_pages.len(),
                if pdb_torn_growth.truncate_to_pages.is_some() {
                    ", plus a never-populated file tail beyond next_unused_page"
                } else {
                    ""
                }
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_TORN_GROWTH_PAGES_FIX_ID.to_string(),
                title: "Repair Torn Growth Pages (Interrupted Export)".to_string(),
                description: format!(
                    "The export was interrupted (e.g. USB disconnected) while rekordbox was \
                     growing the PDB: {} table empty_candidate page(s) were earmarked as the \
                     next slot to write but never received data, leaving non-zero garbage \
                     instead of the blank page the writer requires before reuse{}. This zeroes \
                     those page(s){} and recomputes seqdb. No table chain, page index, or \
                     transaction history is touched.",
                    pdb_torn_growth.garbage_ec_pages.len(),
                    if pdb_torn_growth.truncate_to_pages.is_some() {
                        " and left the file extended past next_unused_page with unpopulated content"
                    } else {
                        ""
                    },
                    if pdb_torn_growth.truncate_to_pages.is_some() {
                        " and truncates the never-populated tail"
                    } else {
                        ""
                    },
                ) + " This is a structural prerequisite and always runs before other repairs, \
                     including the strict-parity upgrade: parity's additive writes can grow the \
                     file, and this repair's tail-truncation boundary would otherwise go stale \
                     and cut off tracks/playlists parity had just written.",
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        if !pdb_misaligned_track_ids.is_empty() {
            detected_issues.push(format!(
                "{} track row(s) with a misaligned UTF-16 string slot (freezes/comm-errors CDJ hardware while browsing)",
                pdb_misaligned_track_ids.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_TRACK_STRING_ALIGNMENT_FIX_ID.to_string(),
                title: "Repair Track String Alignment".to_string(),
                description: format!(
                    "Re-encode {} track row(s) whose title/filename/other UTF-16 string slot \
                     doesn't start at a 4-byte aligned offset. MIPS-based CDJ hardware requires \
                     4-byte aligned reads for the UTF-16 string header; a misaligned slot \
                     freezes/comm-errors the player while browsing.",
                    pdb_misaligned_track_ids.len()
                ),
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        if !pdb_misaligned_album_ids.is_empty() {
            detected_issues.push(format!(
                "{} album row(s) with a misaligned UTF-16 name slot (freezes/comm-errors CDJ hardware while browsing)",
                pdb_misaligned_album_ids.len()
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_ALBUM_STRING_ALIGNMENT_FIX_ID.to_string(),
                title: "Repair Album String Alignment".to_string(),
                description: format!(
                    "Re-encode {} album row(s) whose UTF-16 name slot doesn't start at a 4-byte \
                     aligned offset. Same class of issue as the track string alignment repair.",
                    pdb_misaligned_album_ids.len()
                ),
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        if !pdb_misaligned_artist_ids.is_empty() {
            detected_issues.push(format!(
                "{} artist row(s) with a misaligned UTF-16 name slot",
                pdb_misaligned_artist_ids.len()
            ));
            unsupported_items.push(RepairUnsupportedItem {
                issue: format!(
                    "{} artist row(s) with a misaligned UTF-16 name slot",
                    pdb_misaligned_artist_ids.len()
                ),
                reason: "This app's own writer always places the artist name at a 4-byte aligned \
                         offset, so this indicates a foreign-origin PDB. No automated repair is \
                         offered; re-export the affected playlists to regenerate clean artist rows."
                    .to_string(),
            });
        }
        let pdb_header_compatibility_repair =
            detect_pdb_header_compatibility_repair(&staged_pdb_path);
        if let Some(repair) = pdb_header_compatibility_repair.as_ref() {
            detected_issues.push(format!(
                "PDB header compatibility field 0x10..0x14 is {}",
                repair.current_value
            ));
            estimated_file_writes += 1;
            proposed_fixes.push(RepairFixProposal {
                id: PDB_HEADER_COMPATIBILITY_FIX_ID.to_string(),
                title: "Repair PDB Header Compatibility Field".to_string(),
                description: format!(
                    "Patch only bytes 0x10..0x14 from {} to the built-in compatibility value {}.",
                    repair.current_value, repair.target_value,
                ),
                supported: true,
                destructive: false,
                estimated_writes: 1,
                estimated_deletes: 0,
            });
        }
        let contents_root = usb_root.join("Contents");
        let contents_present = contents_root.is_dir();
        let all_contents_audio = if contents_present {
            collect_contents_audio_files(&usb_root)
        } else {
            vec![]
        };
        let any_audio_on_usb = !all_contents_audio.is_empty();

        if let Some(parsed) = parsed_pdb.as_ref() {
            if any_audio_on_usb {
                let referenced_track_ids = parsed
                    .playlist_entries
                    .iter()
                    .map(|e| e.track_id)
                    .collect::<HashSet<_>>();
                for track in &parsed.tracks {
                    if !referenced_track_ids.contains(&track.id) {
                        continue;
                    }
                    let exists = resolve_usb_side_path(&usb_root, &track.track_file_path)
                        .as_deref()
                        .map(std::path::Path::new)
                        .map(std::path::Path::is_file)
                        .unwrap_or(false);
                    if exists {
                        continue;
                    }
                    missing_audio_track_ids.insert(track.id);
                    missing_audio_paths
                        .push(normalize_pdb_path_for_edb_lookup(&track.track_file_path));
                }
            } else {
                warnings.push(logging::log(
                    Level::Info,
                    "usb-diagnostics",
                    "usb.diagnostics.missing-audio-scan-skipped",
                    "missing-audio scan skipped: Contents directory is absent or empty (DB-only snapshot)",
                ));
            }
        }
        missing_audio_paths.sort();
        missing_audio_paths.dedup();
        if !missing_audio_track_ids.is_empty() {
            detected_issues.push(format!(
                "{} track reference(s) point to missing audio files under Contents",
                missing_audio_track_ids.len()
            ));
        }

        // Detect audio files present on USB that are not indexed in either PDB or eDB content rows.
        if let Some(parsed) = parsed_pdb.as_ref()
            && any_audio_on_usb
        {
            let mut indexed_paths = parsed
                .tracks
                .iter()
                .map(|t| normalize_path_for_contents_match(&t.track_file_path))
                .filter(|p| !p.is_empty())
                .collect::<HashSet<_>>();
            let edb_indexed_paths = edb_conn
                .as_ref()
                .map(|conn| collect_edb_indexed_paths(conn, &mut warnings))
                .unwrap_or_default();
            indexed_paths.extend(edb_indexed_paths);
            // Case-insensitive containment: FAT32/exFAT resolves paths case-insensitively, so
            // a folder's on-disk casing can drift from the currently-recorded artist/album
            // casing without the file actually being missing or unindexed.
            let indexed_paths_ci: HashSet<String> = indexed_paths
                .iter()
                .map(|p| contents_path_match_key(p))
                .collect();
            unindexed_audio_paths = all_contents_audio
                .iter()
                .map(|p| normalize_path_for_contents_match(p))
                .filter(|p| !p.is_empty())
                .filter(|p| !indexed_paths_ci.contains(&contents_path_match_key(p)))
                .collect::<Vec<_>>();
            unindexed_audio_paths.sort();
            unindexed_audio_paths.dedup();

            if !unindexed_audio_paths.is_empty() {
                detected_issues.push(format!(
                        "{} audio file(s) exist under Contents but are missing from the canonical-path indexed set (PDB/eDB)",
                        unindexed_audio_paths.len()
                    ));
                proposed_fixes.push(RepairFixProposal {
                        id: "manual_reimport_unindexed_audio".to_string(),
                        title: "Manual Re-import Unindexed Audio".to_string(),
                        description: "Non-destructive guidance: copy unindexed files to safety/media library and import/export again."
                            .to_string(),
                        supported: false,
                        destructive: false,
                        estimated_writes: 0,
                        estimated_deletes: 0,
                    });
                unsupported_items.push(RepairUnsupportedItem {
                        issue: format!(
                            "{} canonical-path unindexed audio file(s) under Contents",
                            unindexed_audio_paths.len()
                        ),
                        reason: "Automatic deletion is intentionally disabled for canonical-path index drift. Recommended flow: copy files to safety/media library, import into playlists, export again. (Strict raw-count drift is reported separately in parity checks.)".to_string(),
                    });
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.unindexed-audio-summary",
                    format!(
                        "canonical-path unindexed audio files detected: {}",
                        unindexed_audio_paths.len()
                    ),
                ));
                for path in &unindexed_audio_paths {
                    warnings.push(logging::log(
                        Level::Warn,
                        "usb-diagnostics",
                        "usb.diagnostics.unindexed-audio",
                        format!("unindexed audio file: {path}"),
                    ));
                }
            }
        }

        if !missing_audio_track_ids.is_empty() {
            if unindexed_audio_paths.is_empty() {
                remove_missing_audio_supported = true;
                proposed_fixes.push(RepairFixProposal {
                    id: "remove_missing_audio_references".to_string(),
                    title: "Remove Missing Audio References".to_string(),
                    description:
                        "Remove playlist/content references for tracks whose audio files no longer exist on USB."
                            .to_string(),
                    supported: true,
                    destructive: true,
                    estimated_writes: 0,
                    estimated_deletes: missing_audio_track_ids.len(),
                });
            } else {
                proposed_fixes.push(RepairFixProposal {
                    id: "remove_missing_audio_references".to_string(),
                    title: "Remove Missing Audio References".to_string(),
                    description:
                        "Manual-only in this state: index drift detected (unindexed files are present), so automatic deletion is disabled."
                            .to_string(),
                    supported: false,
                    destructive: false,
                    estimated_writes: 0,
                    estimated_deletes: 0,
                });
                unsupported_items.push(RepairUnsupportedItem {
                    issue: format!(
                        "{} missing-audio reference(s) require manual review",
                        missing_audio_track_ids.len()
                    ),
                    reason: format!(
                        "Automatic removal is disabled while {} canonical-path unindexed audio file(s) are present. Re-import/export first, then re-run diagnostics.",
                        unindexed_audio_paths.len(),
                    ),
                });
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.missing-audio-auto-repair-disabled",
                    "missing-audio auto-repair disabled: canonical-path unindexed audio files are present; manual re-import recommended first",
                ));
            }
            for path in &missing_audio_paths {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.missing-audio",
                    format!("missing-audio reference: {path}"),
                ));
            }
        }

        if let Some(parsed) = parsed_pdb.as_ref() {
            let (history_rows, history_content_rows) = derive_history_sync_payload(parsed);
            if !history_rows.is_empty()
                && let Some(conn) = edb_conn.as_ref()
                && table_exists(conn, "history")
                && table_exists(conn, "history_content")
            {
                let current_history_count = conn
                    .query_row("SELECT COUNT(*) FROM history", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .ok()
                    .unwrap_or(0)
                    .max(0) as usize;
                let current_history_content_count = conn
                    .query_row("SELECT COUNT(*) FROM history_content", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .ok()
                    .unwrap_or(0)
                    .max(0) as usize;
                let target_history_count = history_rows.len();
                let target_history_content_count = history_content_rows.len();
                if current_history_count != target_history_count
                    || current_history_content_count != target_history_content_count
                {
                    sync_edb_history_needed = true;
                    sync_edb_history_supported = true;
                    detected_issues.push(format!(
                                "eDB history differs from PDB-derived payload (history {current_history_count}->{target_history_count}, history_content {current_history_content_count}->{target_history_content_count})"
                            ));
                    proposed_fixes.push(RepairFixProposal {
                                id: SYNC_EDB_HISTORY_FROM_PDB_FIX_ID.to_string(),
                                title: "Sync eDB History Tables From PDB".to_string(),
                                description: "Optional fix: replace eDB history/history_content rows using current PDB history payload mapping.".to_string(),
                                supported: true,
                                destructive: current_history_count > 0
                                    || current_history_content_count > 0,
                                estimated_writes: target_history_count + target_history_content_count,
                                estimated_deletes: current_history_count + current_history_content_count,
                            });
                }
            }
        }

        let mut strict_upgrade_targets = HashSet::<String>::new();
        match parity {
            Ok(report) => {
                if let Some(raw_issue) =
                    strict_raw_coverage_issue_from_parity_checks(&report.checks)
                {
                    detected_issues.push(raw_issue.clone());
                    warnings.push(logging::log(
                        Level::Warn,
                        "usb-diagnostics",
                        "usb.diagnostics.strict-raw-coverage",
                        raw_issue,
                    ));
                }
                let strict_count = report
                    .playlist_details
                    .iter()
                    .filter(|d| playlist_requires_strict_upgrade(d))
                    .count();
                strict_upgrade_targets = report
                    .playlist_details
                    .iter()
                    .filter(|d| playlist_requires_strict_upgrade(d))
                    .map(|d| canonicalize_playlist_name(&d.name))
                    .collect();
                if strict_count > 0 {
                    detected_issues.push(format!("{strict_count} playlist(s) fail strict parity"));
                    proposed_fixes.push(RepairFixProposal {
                        id: STRICT_PARITY_UPGRADE_FIX_ID.to_string(),
                        title: "Upgrade Export Data To Strict Parity".to_string(),
                        description: "Collect all playlists from both eDB and PDB, merge metadata from both sides, and rewrite both databases once.".to_string(),
                        supported: true,
                        destructive: false,
                        estimated_writes: 0,
                        estimated_deletes: 0,
                    });
                }
                warnings.extend(report.warnings);
            }
            Err(err) => {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-diagnostics",
                    "usb.diagnostics.parity-preview-unavailable",
                    format!("parity preview unavailable: {err}"),
                ));
            }
        }

        // Display order should match the actual apply order below (see
        // REPAIR_FIX_DISPLAY_ORDER), not the incidental order each fix
        // happened to be detected in.
        proposed_fixes.sort_by_key(|f| {
            REPAIR_FIX_DISPLAY_ORDER
                .iter()
                .position(|id| *id == f.id)
                .unwrap_or(usize::MAX)
        });

        let selected = if req.selected_fix_ids.is_empty() {
            proposed_fixes
                .iter()
                .filter(|f| f.supported && f.id != SYNC_EDB_HISTORY_FROM_PDB_FIX_ID)
                .map(|f| f.id.clone())
                .collect::<std::collections::HashSet<_>>()
        } else {
            req.selected_fix_ids.iter().cloned().collect()
        };

        if req.apply {
            warnings.extend(
                self.backup_usb_databases_with_retention(&usb_root, "Before repair", None)
                    .into_iter()
                    .map(|message| {
                        logging::log(
                            Level::Info,
                            "usb-diagnostics",
                            "usb.diagnostics.backup",
                            message,
                        )
                    }),
            );

            // Open the eDB read-write once for every fix below instead of
            // each fix independently re-staging/re-opening it -- SQLCipher
            // key negotiation is the expensive part. Individual fixes fall
            // back to opening their own connection if this fails.
            let mut fix_edb_conn = open_edb_rw(&usb_root, &mut warnings);

            if selected.contains("fix_empty_analysis_files") {
                match self.apply_fix_empty_analysis_files(
                    &usb_root,
                    &empty_analysis_paths,
                    fix_edb_conn.as_mut(),
                    parsed_pdb.as_ref(),
                    &mut warnings,
                ) {
                    Ok((fixed, skipped, failed, write_count)) => {
                        estimated_file_writes = estimated_file_writes.max(write_count);
                        if failed > 0 {
                            failed_fixes.push(format!(
                                "Fix Empty Analysis Files: fixed {fixed}, skipped {skipped}, failed {failed}"
                            ));
                        } else if fixed > 0 {
                            applied_fixes.push(format!(
                                "Fix Empty Analysis Files: fixed {fixed}, skipped {skipped}"
                            ));
                        } else {
                            skipped_fixes
                                .push("Fix Empty Analysis Files: nothing to apply".to_string());
                        }
                    }
                    Err(err) => {
                        failed_fixes.push(format!("Fix Empty Analysis Files failed: {err}"))
                    }
                }
            } else {
                skipped_fixes.push("Fix Empty Analysis Files: not selected".to_string());
            }

            // Unlike the other structural repairs (which are safe to run after
            // strict parity, since write_pdb's own additive writes establish
            // correct page structure), a truncated table chain must be fixed
            // *before* strict parity: write_pdb's additive track-append path
            // hard-fails on a table whose declared last page doesn't physically
            // exist, so every strict-parity mutation that needs a new/changed
            // track row would otherwise fail (or only succeed once enough other
            // playlists happen to have organically grown the chain past it).
            if selected.contains(PDB_TRUNCATED_TABLE_CHAIN_FIX_ID) {
                if pdb_truncated_chains.is_empty() {
                    skipped_fixes
                        .push("Repair Truncated Table Chain: nothing to apply".to_string());
                } else {
                    match apply_pdb_truncated_table_chain_repair(&usb_root, &pdb_truncated_chains) {
                        Ok(n) => applied_fixes
                            .push(format!("Repair Truncated Table Chain: fixed {n} table(s)")),
                        Err(err) => {
                            failed_fixes.push(format!("Repair Truncated Table Chain failed: {err}"))
                        }
                    }
                }
            } else if !pdb_truncated_chains.is_empty() {
                skipped_fixes.push("Repair Truncated Table Chain: not selected".to_string());
            }

            // Also unlike the other structural repairs, torn-growth-pages must be
            // fixed *before* strict parity, for a different reason: its
            // `truncate_to_pages` boundary was computed once, up front, against
            // the pre-repair file. Strict parity's additive writer legitimately
            // grows the file past that boundary while merging playlists/tracks
            // (`append_rows_to_chain_in_place`); if this repair ran afterward
            // using the now-stale boundary, it would truncate the file back down
            // and silently drop everything strict parity had just written.
            // Neither `fix_empty_analysis_files` nor the truncated-table-chain
            // repair above grow the PDB file, so running this immediately after
            // them (and still before strict parity) keeps the boundary accurate.
            if selected.contains(PDB_TORN_GROWTH_PAGES_FIX_ID) {
                if pdb_torn_growth.is_empty() {
                    skipped_fixes.push("Repair Torn Growth Pages: nothing to apply".to_string());
                } else {
                    match apply_pdb_torn_growth_pages_repair(&usb_root, &pdb_torn_growth) {
                        Ok(n) => applied_fixes.push(format!(
                            "Repair Torn Growth Pages: zeroed {n} page(s){}",
                            if pdb_torn_growth.truncate_to_pages.is_some() {
                                ", truncated unpopulated tail"
                            } else {
                                ""
                            }
                        )),
                        Err(err) => {
                            failed_fixes.push(format!("Repair Torn Growth Pages failed: {err}"))
                        }
                    }
                }
            } else if !pdb_torn_growth.is_empty() {
                skipped_fixes.push("Repair Torn Growth Pages: not selected".to_string());
            }

            // Strict parity runs after the two structural prerequisites above so
            // that write_pdb establishes correct page structure before the
            // remaining structural repairs (especially stale B-tree) run over
            // the final result in one clean pass.
            if selected.contains(STRICT_PARITY_UPGRADE_FIX_ID) {
                let force_all = strict_repair_force_all_enabled();
                if strict_upgrade_targets.is_empty() && !force_all {
                    skipped_fixes
                        .push("Upgrade Export Data To Strict Parity: nothing to apply".to_string());
                } else {
                    let target_names = if force_all {
                        None
                    } else {
                        Some(&strict_upgrade_targets)
                    };
                    match self.apply_strict_parity_upgrade(
                        &usb_root,
                        target_names,
                        fix_edb_conn.as_mut(),
                        &mut warnings,
                    ) {
                        Ok(result) => {
                            if result.failed_playlists > 0 {
                                failed_fixes.push(format!(
                                "Upgrade Export Data To Strict Parity: merged {} playlist(s), wrote {} eDB playlist(s), removed {} duplicate PDB entry/entries, {} failed{}",
                                result.merged_playlists,
                                result.edb_playlists_written,
                                result.duplicate_entries_removed,
                                result.failed_playlists,
                                if result.artwork_patch_incomplete {
                                    " (artwork parity incomplete)"
                                } else {
                                    ""
                                }
                            ));
                            } else if result.merged_playlists > 0 {
                                applied_fixes.push(format!(
                                "Upgrade Export Data To Strict Parity: merged {} playlist(s), wrote {} eDB playlist(s), removed {} duplicate PDB entry/entries{}",
                                result.merged_playlists,
                                result.edb_playlists_written,
                                result.duplicate_entries_removed,
                                if result.artwork_patch_incomplete {
                                    " (artwork parity incomplete)"
                                } else {
                                    ""
                                }
                            ));
                            } else {
                                skipped_fixes.push(
                                    "Upgrade Export Data To Strict Parity: nothing to apply"
                                        .to_string(),
                                );
                            }
                        }
                        Err(err) => failed_fixes.push(format!(
                            "Upgrade Export Data To Strict Parity failed: {err}"
                        )),
                    }
                }
            } else {
                skipped_fixes
                    .push("Upgrade Export Data To Strict Parity: not selected".to_string());
            }

            if selected.contains(PDB_ZERO_TRANRF_FIX_ID) {
                if pdb_zero_tranrf_pages.is_empty() {
                    skipped_fixes
                        .push("Repair Data Pages With Zero tranrf: nothing to apply".to_string());
                } else {
                    match apply_pdb_zero_tranrf_repair(&usb_root, &pdb_zero_tranrf_pages) {
                        Ok(n) => applied_fixes.push(format!(
                            "Repair Data Pages With Zero tranrf: patched {n} page(s)"
                        )),
                        Err(err) => failed_fixes
                            .push(format!("Repair Data Pages With Zero tranrf failed: {err}")),
                    }
                }
            } else if !pdb_zero_tranrf_pages.is_empty() {
                skipped_fixes.push("Repair Data Pages With Zero tranrf: not selected".to_string());
            }

            if selected.contains(PDB_WRONG_PAGE_FLAGS_FIX_ID) {
                if pdb_wrong_flags_pages.is_empty() {
                    skipped_fixes.push("Repair PDB Data Page Flags: nothing to apply".to_string());
                } else {
                    match apply_pdb_wrong_page_flags_repair(&usb_root, &pdb_wrong_flags_pages) {
                        Ok(n) => applied_fixes
                            .push(format!("Repair PDB Data Page Flags: patched {n} page(s)")),
                        Err(err) => {
                            failed_fixes.push(format!("Repair PDB Data Page Flags failed: {err}"))
                        }
                    }
                }
            } else if !pdb_wrong_flags_pages.is_empty() {
                skipped_fixes.push("Repair PDB Data Page Flags: not selected".to_string());
            }

            if selected.contains(PDB_SENTINEL_U5_FIX_ID) {
                if pdb_sentinel_u5_pages.is_empty() {
                    skipped_fixes.push(
                        "Repair PDB Data Pages With Sentinel u5: nothing to apply".to_string(),
                    );
                } else {
                    match apply_pdb_sentinel_u5_repair(&usb_root, &pdb_sentinel_u5_pages) {
                        Ok(n) => applied_fixes.push(format!(
                            "Repair PDB Data Pages With Sentinel u5: patched {n} page(s)"
                        )),
                        Err(err) => failed_fixes.push(format!(
                            "Repair PDB Data Pages With Sentinel u5 failed: {err}"
                        )),
                    }
                }
            } else if !pdb_sentinel_u5_pages.is_empty() {
                skipped_fixes
                    .push("Repair PDB Data Pages With Sentinel u5: not selected".to_string());
            }

            if selected.contains(PDB_WRONG_HISTORY_SHAPE_FIX_ID) {
                if pdb_wrong_history_shape_pages.is_empty() {
                    skipped_fixes.push(
                        "Repair PDB History/Columns Page Footer Shape: nothing to apply"
                            .to_string(),
                    );
                } else {
                    match apply_pdb_wrong_history_page_shape_repair(
                        &usb_root,
                        &pdb_wrong_history_shape_pages,
                    ) {
                        Ok(n) => applied_fixes.push(format!(
                            "Repair PDB History/Columns Page Footer Shape: patched {n} page(s)"
                        )),
                        Err(err) => failed_fixes.push(format!(
                            "Repair PDB History/Columns Page Footer Shape failed: {err}"
                        )),
                    }
                }
            } else if !pdb_wrong_history_shape_pages.is_empty() {
                skipped_fixes
                    .push("Repair PDB History/Columns Page Footer Shape: not selected".to_string());
            }

            if selected.contains(PDB_WRONG_TRACK_U5_FIX_ID) {
                if pdb_wrong_track_u5_pages.is_empty() {
                    skipped_fixes
                        .push("Repair PDB Track Page Footer Shape: nothing to apply".to_string());
                } else {
                    match apply_pdb_wrong_track_u5_repair(&usb_root, &pdb_wrong_track_u5_pages) {
                        Ok(n) => applied_fixes.push(format!(
                            "Repair PDB Track Page Footer Shape: patched {n} page(s)"
                        )),
                        Err(err) => failed_fixes
                            .push(format!("Repair PDB Track Page Footer Shape failed: {err}")),
                    }
                }
            } else if !pdb_wrong_track_u5_pages.is_empty() {
                skipped_fixes.push("Repair PDB Track Page Footer Shape: not selected".to_string());
            }

            if selected.contains(PDB_T00_MULTIPAGE_ACTIVE_FIX_ID) {
                if pdb_t00_multipage_active_pages.is_empty() {
                    skipped_fixes.push(
                        "Repair Track Page Flag (Multi-Page Chain): nothing to apply".to_string(),
                    );
                } else {
                    match apply_pdb_t00_multipage_active_repair(
                        &usb_root,
                        &pdb_t00_multipage_active_pages,
                    ) {
                        Ok(n) => applied_fixes.push(format!(
                            "Repair Track Page Flag (Multi-Page Chain): patched {n} page(s)"
                        )),
                        Err(err) => failed_fixes.push(format!(
                            "Repair Track Page Flag (Multi-Page Chain) failed: {err}"
                        )),
                    }
                }
            } else if !pdb_t00_multipage_active_pages.is_empty() {
                skipped_fixes
                    .push("Repair Track Page Flag (Multi-Page Chain): not selected".to_string());
            }

            if selected.contains(PDB_TOMBSTONED_PLAYLIST_TREE_ID_FIX_ID) {
                if pdb_tombstoned_playlist_ids.is_empty() {
                    skipped_fixes
                        .push("Repair Tombstoned Playlist Tree IDs: nothing to apply".to_string());
                } else {
                    match apply_pdb_tombstoned_playlist_tree_id_repair(
                        &usb_root,
                        &pdb_tombstoned_playlist_ids,
                    ) {
                        Ok(n) => applied_fixes.push(format!(
                            "Repair Tombstoned Playlist Tree IDs: zeroed id in {n} slot(s)"
                        )),
                        Err(err) => failed_fixes
                            .push(format!("Repair Tombstoned Playlist Tree IDs failed: {err}")),
                    }
                }
            } else if !pdb_tombstoned_playlist_ids.is_empty() {
                skipped_fixes.push("Repair Tombstoned Playlist Tree IDs: not selected".to_string());
            }

            if selected.contains(PDB_WRONG_PLAYLIST_TREE_SHAPE_FIX_ID) {
                if pdb_wrong_playlist_tree_shape_pages.is_empty() {
                    skipped_fixes
                        .push("Repair PDB Playlist-Tree Page Shape: nothing to apply".to_string());
                } else {
                    match apply_pdb_wrong_playlist_tree_shape_repair(
                        &usb_root,
                        &pdb_wrong_playlist_tree_shape_pages,
                    ) {
                        Ok(n) => applied_fixes.push(format!(
                            "Repair PDB Playlist-Tree Page Shape: patched {n} page(s)"
                        )),
                        Err(err) => failed_fixes
                            .push(format!("Repair PDB Playlist-Tree Page Shape failed: {err}")),
                    }
                }
            } else if !pdb_wrong_playlist_tree_shape_pages.is_empty() {
                skipped_fixes.push("Repair PDB Playlist-Tree Page Shape: not selected".to_string());
            }

            if selected.contains(PDB_STALE_SENTINEL_BTREE_FIX_ID) {
                if pdb_stale_sentinel_btree_pages.is_empty() {
                    skipped_fixes
                        .push("Repair Stale Sentinel B-Tree: nothing to apply".to_string());
                } else {
                    match apply_pdb_stale_sentinel_btree_repair(
                        &usb_root,
                        &pdb_stale_sentinel_btree_pages,
                    ) {
                        Ok(n) => applied_fixes.push(format!(
                            "Repair Stale Sentinel B-Tree: reset {n} sentinel page(s)"
                        )),
                        Err(err) => {
                            failed_fixes.push(format!("Repair Stale Sentinel B-Tree failed: {err}"))
                        }
                    }
                }
            } else if !pdb_stale_sentinel_btree_pages.is_empty() {
                skipped_fixes.push("Repair Stale Sentinel B-Tree: not selected".to_string());
            }

            if selected.contains(PDB_EC_CONFLICT_FIX_ID) {
                if pdb_ec_conflicts.is_empty() {
                    skipped_fixes
                        .push("Repair Table Write-Pointer Conflict: nothing to apply".to_string());
                } else {
                    match apply_pdb_ec_data_page_conflict_repair(&usb_root, &pdb_ec_conflicts) {
                        Ok(n) => applied_fixes.push(format!(
                            "Repair Table Write-Pointer Conflict: fixed {n} table(s)"
                        )),
                        Err(err) => failed_fixes
                            .push(format!("Repair Table Write-Pointer Conflict failed: {err}")),
                    }
                }
            } else if !pdb_ec_conflicts.is_empty() {
                skipped_fixes.push("Repair Table Write-Pointer Conflict: not selected".to_string());
            }

            if selected.contains(PDB_TRACK_STRING_ALIGNMENT_FIX_ID) {
                if pdb_misaligned_track_ids.is_empty() {
                    skipped_fixes
                        .push("Repair Track String Alignment: nothing to apply".to_string());
                } else {
                    match apply_pdb_track_string_alignment_repair(
                        &usb_root,
                        &pdb_misaligned_track_ids,
                    ) {
                        Ok(n) => applied_fixes
                            .push(format!("Repair Track String Alignment: fixed {n} row(s)")),
                        Err(err) => failed_fixes
                            .push(format!("Repair Track String Alignment failed: {err}")),
                    }
                }
            } else if !pdb_misaligned_track_ids.is_empty() {
                skipped_fixes.push("Repair Track String Alignment: not selected".to_string());
            }

            if selected.contains(PDB_ALBUM_STRING_ALIGNMENT_FIX_ID) {
                if pdb_misaligned_album_ids.is_empty() {
                    skipped_fixes
                        .push("Repair Album String Alignment: nothing to apply".to_string());
                } else {
                    match apply_pdb_album_string_alignment_repair(
                        &usb_root,
                        &pdb_misaligned_album_ids,
                    ) {
                        Ok(n) => applied_fixes
                            .push(format!("Repair Album String Alignment: fixed {n} row(s)")),
                        Err(err) => failed_fixes
                            .push(format!("Repair Album String Alignment failed: {err}")),
                    }
                }
            } else if !pdb_misaligned_album_ids.is_empty() {
                skipped_fixes.push("Repair Album String Alignment: not selected".to_string());
            }

            if selected.contains(PDB_HEADER_COMPATIBILITY_FIX_ID) {
                if let Some(repair) = pdb_header_compatibility_repair.as_ref() {
                    match apply_pdb_header_compatibility_repair(&usb_root, repair) {
                        Ok(true) => applied_fixes.push(format!(
                            "Repair PDB Header Compatibility Field: wrote 0x10..0x14 {}->{}",
                            repair.current_value, repair.target_value,
                        )),
                        Ok(false) => skipped_fixes.push(
                            "Repair PDB Header Compatibility Field: nothing to apply".to_string(),
                        ),
                        Err(err) => failed_fixes.push(format!(
                            "Repair PDB Header Compatibility Field failed: {err}"
                        )),
                    }
                } else {
                    skipped_fixes.push(
                        "Repair PDB Header Compatibility Field: nothing to apply".to_string(),
                    );
                }
            } else if pdb_header_compatibility_repair.is_some() {
                skipped_fixes
                    .push("Repair PDB Header Compatibility Field: not selected".to_string());
            }

            if selected.contains("remove_missing_audio_references") {
                if !remove_missing_audio_supported {
                    skipped_fixes.push(
                        "Remove Missing Audio References: preview-only/manual in current USB state"
                            .to_string(),
                    );
                } else {
                    match self.apply_fix_remove_missing_audio_references(
                        &usb_root,
                        &missing_audio_track_ids,
                        &missing_audio_paths,
                        fix_edb_conn.as_mut(),
                        &mut warnings,
                    ) {
                        Ok((
                            removed_db_content,
                            removed_db_playlist_links,
                            removed_pdb_entries,
                        )) => {
                            if removed_db_content > 0
                                || removed_db_playlist_links > 0
                                || removed_pdb_entries > 0
                            {
                                applied_fixes.push(format!(
                                    "Remove Missing Audio References: removed content rows {removed_db_content}, playlist_content rows {removed_db_playlist_links}, PDB playlist entries {removed_pdb_entries}"
                                ));
                            } else {
                                skipped_fixes.push(
                                    "Remove Missing Audio References: nothing to apply".to_string(),
                                );
                            }
                        }
                        Err(err) => failed_fixes
                            .push(format!("Remove Missing Audio References failed: {err}")),
                    }
                }
            } else {
                skipped_fixes.push("Remove Missing Audio References: not selected".to_string());
            }

            if selected.contains(SYNC_EDB_HISTORY_FROM_PDB_FIX_ID) {
                if !sync_edb_history_supported || !sync_edb_history_needed {
                    skipped_fixes
                        .push("Sync eDB History Tables From PDB: nothing to apply".to_string());
                } else if let Some(parsed) = parsed_pdb.as_ref() {
                    match self.apply_fix_sync_edb_history_from_pdb(
                        &usb_root,
                        parsed,
                        fix_edb_conn.as_mut(),
                        &mut warnings,
                    ) {
                        Ok((history_written, history_content_written)) => {
                            applied_fixes.push(format!(
                                "Sync eDB History Tables From PDB: wrote history {history_written}, history_content {history_content_written}"
                            ));
                        }
                        Err(err) => failed_fixes
                            .push(format!("Sync eDB History Tables From PDB failed: {err}")),
                    }
                } else {
                    skipped_fixes
                        .push("Sync eDB History Tables From PDB: PDB unavailable".to_string());
                }
            } else if sync_edb_history_needed {
                skipped_fixes.push("Sync eDB History Tables From PDB: not selected".to_string());
            }

            // Checkpoint the shared eDB connection's WAL into the main db
            // file before write-back reads it off disk below -- write-back
            // copies raw file bytes, not through SQLite, so any change still
            // sitting in the WAL would otherwise be silently dropped.
            if let Some(conn) = fix_edb_conn.as_ref() {
                let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            }
            drop(fix_edb_conn);

            // All selected fixes above write to their local staged copies only
            // (see usb_staging.rs) -- flush the accumulated local changes back
            // to the USB drive here, once per database, instead of once per
            // fix.
            if let Err(err) =
                usb_staging::write_back_if_changed(&usb_root, usb_staging::DbKind::Pdb)
            {
                warnings.push(logging::log(
                    Level::Error,
                    "usb-repair",
                    "usb.repair.pdb-write-back-failed",
                    format!("PDB write-back to USB failed: {err}"),
                ));
            }
            if let Err(err) =
                usb_staging::write_back_if_changed(&usb_root, usb_staging::DbKind::Edb)
            {
                warnings.push(logging::log(
                    Level::Error,
                    "usb-repair",
                    "usb.repair.edb-write-back-failed",
                    format!("eDB write-back to USB failed: {err}"),
                ));
            }
        }

        detected_issues.sort();
        detected_issues.dedup();
        {
            let mut seen = HashSet::<String>::new();
            unsupported_items.retain(|item| seen.insert(format!("{}|{}", item.issue, item.reason)));
        }

        let diagnostics_after_apply = if req.apply {
            on_progress(95, 100, "USB: Re-running diagnostics after repair");
            match self.run_usb_diagnostics_with_progress(
                RunUsbDiagnosticsRequest {
                    usb_root: Some(usb_root.to_string_lossy().to_string()),
                },
                |c, t, m| {
                    let pct = 95 + ((c * 4) / t.max(1)).min(4);
                    on_progress(pct, 100, m);
                },
            ) {
                Ok(data) => Some(Box::new(data)),
                Err(err) => {
                    warnings.push(logging::log(
                        Level::Warn,
                        "usb-repair",
                        "usb.repair.post-diagnostics-failed",
                        format!("Post-repair diagnostics failed: {err}"),
                    ));
                    None
                }
            }
        } else {
            None
        };

        on_progress(
            100,
            100,
            if req.apply {
                "USB: Repair apply complete"
            } else {
                "USB: Repair preview ready"
            },
        );
        Ok(RepairUsbDiagnosticsData {
            detected_issues,
            proposed_fixes,
            unsupported_items,
            applied_fixes,
            skipped_fixes,
            failed_fixes,
            estimated_file_writes,
            estimated_file_deletes,
            warnings,
            duration_ms: start.elapsed().as_millis() as u64,
            diagnostics: diagnostics_after_apply,
        })
    }

    /// Collect-merge-write: parse both DBs once, merge all playlists, write once.
    fn apply_strict_parity_upgrade(
        &self,
        usb_root: &std::path::Path,
        target_playlist_names: Option<&HashSet<String>>,
        mut edb_conn: Option<&mut rusqlite::Connection>,
        warnings: &mut Vec<WarningEntry>,
    ) -> BackendResult<StrictParityUpgradeApplyResult> {
        let mut result = StrictParityUpgradeApplyResult::default();
        let pdb_path = usb_staging::stage_pdb(usb_root)?;

        // ── Phase 1: Collect ──────────────────────────────────────────
        let parsed = parse_pdb(&pdb_path)?;
        let edb_playlists = match edb_conn.as_deref() {
            Some(conn) => {
                try_read_playlists_with_metadata_from_edb_with_conn(conn, usb_root, warnings)
            }
            None => try_read_playlists_with_metadata_from_edb(usb_root, warnings),
        };

        // Build PDB track index: identity_key → PdbTrackRow
        let pdb_track_by_key: HashMap<String, &crate::pdb_reader::PdbTrackRow> = parsed
            .tracks
            .iter()
            .map(|t| {
                let artist = parsed
                    .artists
                    .get(&t.artist_id)
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let key = track_identity_key(
                    &t.track_file_path,
                    &t.title,
                    artist,
                    Some(&t.id.to_string()),
                );
                (key, t)
            })
            .collect();

        let pdb_track_by_analysis: HashMap<String, &crate::pdb_reader::PdbTrackRow> = parsed
            .tracks
            .iter()
            .filter_map(|t| {
                let key = normalize_analysis_path_for_identity(&t.anlz_path);
                if key.is_empty() { None } else { Some((key, t)) }
            })
            .collect();

        // Build eDB track index: identity_key → UsbTrack
        let mut edb_track_by_key: HashMap<String, &crate::models::UsbTrack> = HashMap::new();
        if let Some(ref edb_pls) = edb_playlists {
            for pl in edb_pls.values() {
                for track in &pl.tracks {
                    let key = track_identity_key(
                        track.identity_path(),
                        &track.title,
                        &track.artist,
                        Some(&track.id),
                    );
                    edb_track_by_key.entry(key).or_insert(track);
                }
            }
        }

        // Secondary PDB index: meta_key → PdbTrackRow for tracks with empty path.
        // Used for cross-matching when eDB has a path: key but PDB has a meta: key.
        let pdb_track_by_meta: HashMap<String, &crate::pdb_reader::PdbTrackRow> = parsed
            .tracks
            .iter()
            .filter(|t| t.track_file_path.trim().is_empty())
            .map(|t| {
                let artist = parsed
                    .artists
                    .get(&t.artist_id)
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let meta = build_meta_key(&t.title, artist);
                (meta, t)
            })
            .collect();

        // Build PDB playlist structure indexed by exact name + canonical fallback
        let pdb_track_by_id: HashMap<u32, &crate::pdb_reader::PdbTrackRow> =
            parsed.tracks.iter().map(|t| (t.id, t)).collect();
        struct PdbPlaylistInfo {
            playlist_id: u32,
            sort_order: u32,
            entries: Vec<crate::pdb_reader::PdbPlaylistEntryRow>,
        }
        let mut pdb_playlists_by_exact: HashMap<String, PdbPlaylistInfo> = HashMap::new();
        let mut pdb_canon_to_exact: HashMap<String, Vec<String>> = HashMap::new();
        for leaf in parsed.playlist_tree.iter().filter(|r| !r.row_is_folder) {
            let canon = canonicalize_playlist_name(&leaf.name);
            let mut entries: Vec<_> = parsed
                .playlist_entries
                .iter()
                .filter(|e| e.playlist_id == leaf.id)
                .cloned()
                .collect();
            entries.sort_by_key(|e| e.entry_index);
            // Defensive deterministic selection if multiple rows resolve to the
            // same name during parse: prefer better track coverage; on tie,
            // prefer lower sort order.
            let candidate = PdbPlaylistInfo {
                playlist_id: leaf.id,
                sort_order: leaf.sort_order,
                entries,
            };
            match pdb_playlists_by_exact.get(&leaf.name) {
                None => {
                    pdb_playlists_by_exact.insert(leaf.name.clone(), candidate);
                }
                Some(existing) => {
                    let replace = candidate.entries.len() > existing.entries.len()
                        || (candidate.entries.len() == existing.entries.len()
                            && candidate.sort_order < existing.sort_order);
                    if replace {
                        pdb_playlists_by_exact.insert(leaf.name.clone(), candidate);
                    }
                }
            }
            pdb_canon_to_exact
                .entry(canon)
                .or_default()
                .push(leaf.name.clone());
        }

        // ── Phase 2: Merge playlists ──────────────────────────────────
        let all_playlist_names =
            strict_repair_ordered_playlist_names(&parsed, edb_playlists.as_ref());

        let mut next_track_id = parsed.tracks.iter().map(|t| t.id).max().unwrap_or(0) + 1;
        let mut next_playlist_id = parsed.playlist_tree.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        let mut next_sort_order = parsed
            .playlist_tree
            .iter()
            .map(|r| r.sort_order)
            .max()
            .unwrap_or(0)
            + 1;

        let mut merged_playlists: Vec<MergedPlaylist> = Vec::new();
        let has_existing_pdb_order = parsed.playlist_tree.iter().any(|r| !r.row_is_folder);
        // Track all assigned PDB track IDs to avoid collisions
        let mut assigned_track_ids: HashSet<u32> = parsed.tracks.iter().map(|t| t.id).collect();
        // Track consumed PDB playlists to prevent double-matching on canonical collisions
        let mut consumed_pdb_ids: HashSet<u32> = HashSet::new();
        // Reserve folder IDs so merged leaf IDs never collide with folder rows.
        let folder_playlist_ids: HashSet<u32> = parsed
            .playlist_tree
            .iter()
            .filter(|r| r.row_is_folder)
            .map(|r| r.id)
            .collect();
        // Track assigned merged leaf playlist IDs.
        let mut assigned_playlist_ids: HashSet<u32> = HashSet::new();
        let allocate_unique_playlist_id = |candidate: u32,
                                           folder_playlist_ids: &HashSet<u32>,
                                           assigned_playlist_ids: &HashSet<u32>,
                                           next_playlist_id: &mut u32|
         -> u32 {
            if !folder_playlist_ids.contains(&candidate)
                && !assigned_playlist_ids.contains(&candidate)
            {
                return candidate;
            }
            while folder_playlist_ids.contains(next_playlist_id)
                || assigned_playlist_ids.contains(next_playlist_id)
            {
                *next_playlist_id += 1;
            }
            let id = *next_playlist_id;
            *next_playlist_id += 1;
            id
        };

        for playlist_name in &all_playlist_names {
            if let Some(targets) = target_playlist_names {
                let canon_name = canonicalize_playlist_name(playlist_name);
                if !targets.contains(&canon_name) {
                    continue;
                }
            }
            let canon = canonicalize_playlist_name(playlist_name);
            let edb_pl = edb_playlists
                .as_ref()
                .and_then(|pls| pls.get(playlist_name));
            // Two-tier PDB lookup: exact name first, canonical fallback (skip consumed)
            let pdb_pl = pdb_playlists_by_exact
                .get(playlist_name)
                .filter(|p| !consumed_pdb_ids.contains(&p.playlist_id))
                .or_else(|| {
                    pdb_canon_to_exact.get(&canon).and_then(|names| {
                        names
                            .iter()
                            .filter_map(|n| pdb_playlists_by_exact.get(n))
                            .find(|p| !consumed_pdb_ids.contains(&p.playlist_id))
                    })
                });
            if let Some(pdb) = pdb_pl {
                consumed_pdb_ids.insert(pdb.playlist_id);
            }

            // Determine playlist identity, avoiding ID collisions
            let (pl_id, pl_sort) = match (edb_pl, pdb_pl) {
                (Some(_edb), Some(pdb)) => {
                    // Preserve existing PDB playlist IDs and sort order for matched
                    // playlists. PDB sort_order controls player display order and must
                    // not be overwritten with eDB values — eDB and PDB sort orders can
                    // legitimately differ without affecting player acceptance.
                    let pdb_id = pdb.playlist_id;
                    (
                        allocate_unique_playlist_id(
                            pdb_id,
                            &folder_playlist_ids,
                            &assigned_playlist_ids,
                            &mut next_playlist_id,
                        ),
                        pdb.sort_order,
                    )
                }
                (Some(edb), None) => {
                    let candidate = u32::try_from(edb.playlist_id).unwrap_or_else(|_| {
                        let id = next_playlist_id;
                        next_playlist_id += 1;
                        id
                    });
                    let sort = if has_existing_pdb_order {
                        let s = next_sort_order;
                        next_sort_order += 1;
                        s
                    } else {
                        u32::try_from(edb.sort_order).unwrap_or_else(|_| {
                            let s = next_sort_order;
                            next_sort_order += 1;
                            s
                        })
                    };
                    (
                        allocate_unique_playlist_id(
                            candidate,
                            &folder_playlist_ids,
                            &assigned_playlist_ids,
                            &mut next_playlist_id,
                        ),
                        sort,
                    )
                }
                (None, Some(pdb)) => {
                    let id = allocate_unique_playlist_id(
                        pdb.playlist_id,
                        &folder_playlist_ids,
                        &assigned_playlist_ids,
                        &mut next_playlist_id,
                    );
                    (id, pdb.sort_order)
                }
                (None, None) => continue,
            };
            assigned_playlist_ids.insert(pl_id);

            // Build merged track list
            let mut merged_tracks: Vec<MergedTrack> = Vec::new();
            let mut seen_identity_keys = HashSet::<String>::new();

            // eDB tracks first (preferred order)
            if let Some(edb) = edb_pl {
                for edb_track in &edb.tracks {
                    let ident = track_identity_key(
                        edb_track.identity_path(),
                        &edb_track.title,
                        &edb_track.artist,
                        Some(&edb_track.id),
                    );
                    if !seen_identity_keys.insert(ident.clone()) {
                        continue; // skip duplicates
                    }

                    // Find matching PDB track (identity path first, then analysis path,
                    // then metadata fallback).
                    let pdb_track = pdb_track_by_key
                        .get(&ident)
                        .copied()
                        .or_else(|| {
                            let analysis = edb_track_analysis_key(edb_track);
                            if analysis.is_empty() {
                                None
                            } else {
                                pdb_track_by_analysis.get(&analysis).copied()
                            }
                        })
                        .or_else(|| {
                            let meta = build_meta_key(&edb_track.title, &edb_track.artist);
                            pdb_track_by_meta.get(&meta).copied()
                        });
                    // If matched via meta fallback, mark the PDB track's identity key
                    // as seen so it won't be re-added in the PDB-only pass.
                    if let Some(pt) = pdb_track
                        && !pdb_track_by_key.contains_key(&ident)
                    {
                        let pt_artist = parsed
                            .artists
                            .get(&pt.artist_id)
                            .map(|s| s.as_str())
                            .unwrap_or("");
                        let pt_ident = track_identity_key(
                            &pt.track_file_path,
                            &pt.title,
                            pt_artist,
                            Some(&pt.id.to_string()),
                        );
                        seen_identity_keys.insert(pt_ident);
                    }
                    let track_id = if let Some(pt) = pdb_track {
                        pt.id
                    } else {
                        // Assign new PDB track ID for eDB-only track
                        while assigned_track_ids.contains(&next_track_id) {
                            next_track_id += 1;
                        }
                        let id = next_track_id;
                        next_track_id += 1;
                        assigned_track_ids.insert(id);
                        id
                    };

                    let media_path = edb_track
                        .usb_media_path
                        .clone()
                        .unwrap_or_else(|| edb_track.file_path.clone());
                    let media_path = if !media_path.is_empty() {
                        media_path
                    } else {
                        pdb_track
                            .map(|t| normalize_pdb_path_for_edb_lookup(&t.track_file_path))
                            .unwrap_or_default()
                    };
                    let analysis_path = edb_track
                        .usb_analysis_path_raw
                        .clone()
                        .or_else(|| edb_track.usb_analysis_path.clone())
                        .filter(|s| !s.is_empty())
                        .or_else(|| {
                            pdb_track
                                .map(|t| t.anlz_path.clone())
                                .filter(|s| !s.is_empty())
                        })
                        .unwrap_or_default();

                    // Merge metadata: prefer eDB non-empty, fall back to PDB
                    let title = if !edb_track.title.is_empty() {
                        edb_track.title.clone()
                    } else {
                        pdb_track.map(|t| t.title.clone()).unwrap_or_default()
                    };
                    let artist = if !edb_track.artist.is_empty() {
                        edb_track.artist.clone()
                    } else {
                        pdb_track
                            .and_then(|t| parsed.artists.get(&t.artist_id))
                            .cloned()
                            .unwrap_or_default()
                    };
                    let album = edb_track.album.clone().or_else(|| {
                        pdb_track
                            .and_then(|t| parsed.albums.get(&t.album_id))
                            .cloned()
                    });
                    let key = edb_track
                        .key
                        .clone()
                        .or_else(|| pdb_track.and_then(|t| parsed.keys.get(&t.key_id)).cloned());
                    let track_number = edb_track.track_number.or_else(|| {
                        pdb_track.and_then(|t| (t.track_number > 0).then_some(t.track_number))
                    });
                    let bpm = edb_track.bpm.or_else(|| {
                        pdb_track
                            .and_then(|t| (t.tempo_x100 > 0).then_some(t.tempo_x100 as f64 / 100.0))
                    });
                    let duration_ms = edb_track.duration_ms.or_else(|| {
                        pdb_track.and_then(|t| t.duration_seconds.map(|d| u64::from(d) * 1000))
                    });

                    // Build PDB track row from merged data
                    let pdb_row = build_merged_pdb_track_row(
                        track_id,
                        pdb_track,
                        MergedTrackFields {
                            title: &title,
                            artist: &artist,
                            album: &album,
                            key: &key,
                            track_number,
                            bpm,
                            duration_ms,
                            media_path: &media_path,
                            analysis_path: &analysis_path,
                        },
                        &parsed,
                    );

                    let artwork_path = resolved_strict_artwork_path(
                        usb_root,
                        pdb_track,
                        &parsed,
                        edb_track.artwork_path.as_deref(),
                    );

                    merged_tracks.push(MergedTrack {
                        pdb_track_id: track_id,
                        pdb_row,
                        edb_media_path: media_path,
                        title,
                        artist,
                        album,
                        key,
                        track_number,
                        bpm,
                        duration_ms,
                        artwork_path,
                        analysis_path,
                        sample_rate_hz: pdb_track.and_then(|t| t.sample_rate_hz),
                        bitrate_kbps: pdb_track.and_then(|t| t.bitrate_kbps),
                        bit_depth: pdb_track.and_then(|t| t.bit_depth.map(u32::from)),
                        file_size_bytes: pdb_track.and_then(|t| t.file_size_bytes),
                        file_type: pdb_track.and_then(|t| t.file_type.map(u32::from)),
                        isrc: pdb_track.and_then(|t| t.isrc.clone()),
                        release_year: pdb_track.and_then(|t| t.release_year.map(u32::from)),
                        release_date: pdb_track.and_then(|t| t.release_date.clone()),
                    });
                }
            }

            // Preserve existing playlist membership from PDB in strict repair.
            // Device-side membership must not be pruned automatically just because
            // one DB side is sparser; strict repair should repair linkage/metadata
            // without silently removing members.
            let preserve_pdb_membership = true;
            if preserve_pdb_membership {
                if let Some(pdb) = pdb_pl {
                    for entry in &pdb.entries {
                        let Some(pdb_track) = pdb_track_by_id.get(&entry.track_id) else {
                            continue;
                        };
                        let artist_name = parsed
                            .artists
                            .get(&pdb_track.artist_id)
                            .map(|s| s.as_str())
                            .unwrap_or("");
                        let ident = track_identity_key(
                            &pdb_track.track_file_path,
                            &pdb_track.title,
                            artist_name,
                            Some(&pdb_track.id.to_string()),
                        );
                        if !seen_identity_keys.insert(ident) {
                            // Strict parity repair rebuilds a canonical playlist membership
                            // list, so repeated PDB identities must not be reintroduced
                            // just because they use a different legacy track id.
                            continue;
                        }

                        let album = parsed.albums.get(&pdb_track.album_id).cloned();
                        let key = parsed.keys.get(&pdb_track.key_id).cloned();
                        let media_path =
                            normalize_pdb_path_for_edb_lookup(&pdb_track.track_file_path);
                        let bpm_val = (pdb_track.tempo_x100 > 0)
                            .then_some(pdb_track.tempo_x100 as f64 / 100.0);
                        let track_number_val =
                            (pdb_track.track_number > 0).then_some(pdb_track.track_number);
                        let duration_ms_val =
                            pdb_track.duration_seconds.map(|d| u64::from(d) * 1000);

                        let pdb_row = build_merged_pdb_track_row(
                            pdb_track.id,
                            Some(pdb_track),
                            MergedTrackFields {
                                title: &pdb_track.title,
                                artist: artist_name,
                                album: &album,
                                key: &key,
                                track_number: track_number_val,
                                bpm: bpm_val,
                                duration_ms: duration_ms_val,
                                media_path: &pdb_track.track_file_path,
                                analysis_path: &pdb_track.anlz_path,
                            },
                            &parsed,
                        );

                        merged_tracks.push(MergedTrack {
                            pdb_track_id: pdb_track.id,
                            pdb_row,
                            edb_media_path: media_path,
                            title: pdb_track.title.clone(),
                            artist: artist_name.to_string(),
                            album,
                            key,
                            track_number: track_number_val,
                            bpm: bpm_val,
                            duration_ms: duration_ms_val,
                            artwork_path: parsed.artworks.get(&pdb_track.artwork_id).cloned(),
                            analysis_path: pdb_track.anlz_path.clone(),
                            sample_rate_hz: pdb_track.sample_rate_hz,
                            bitrate_kbps: pdb_track.bitrate_kbps,
                            bit_depth: pdb_track.bit_depth.map(u32::from),
                            file_size_bytes: pdb_track.file_size_bytes,
                            file_type: pdb_track.file_type.map(u32::from),
                            isrc: pdb_track.isrc.clone(),
                            release_year: pdb_track.release_year.map(u32::from),
                            release_date: pdb_track.release_date.clone(),
                        });
                    }
                }
            } else if pdb_playlists_by_exact.is_empty() && all_playlist_names.len() == 1 {
                // Fallback for synthetic/no-playlist-mapping snapshots: keep
                // unmatched PDB tracks so strict merge remains additive.
                for pdb_track in &parsed.tracks {
                    let artist_name = parsed
                        .artists
                        .get(&pdb_track.artist_id)
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    let ident = track_identity_key(
                        &pdb_track.track_file_path,
                        &pdb_track.title,
                        artist_name,
                        Some(&pdb_track.id.to_string()),
                    );
                    if !seen_identity_keys.insert(ident) {
                        continue;
                    }
                    let album = parsed.albums.get(&pdb_track.album_id).cloned();
                    let key = parsed.keys.get(&pdb_track.key_id).cloned();
                    let media_path = normalize_pdb_path_for_edb_lookup(&pdb_track.track_file_path);
                    let bpm_val =
                        (pdb_track.tempo_x100 > 0).then_some(pdb_track.tempo_x100 as f64 / 100.0);
                    let track_number_val =
                        (pdb_track.track_number > 0).then_some(pdb_track.track_number);
                    let duration_ms_val = pdb_track.duration_seconds.map(|d| u64::from(d) * 1000);
                    let pdb_row = build_merged_pdb_track_row(
                        pdb_track.id,
                        Some(pdb_track),
                        MergedTrackFields {
                            title: &pdb_track.title,
                            artist: artist_name,
                            album: &album,
                            key: &key,
                            track_number: track_number_val,
                            bpm: bpm_val,
                            duration_ms: duration_ms_val,
                            media_path: &pdb_track.track_file_path,
                            analysis_path: &pdb_track.anlz_path,
                        },
                        &parsed,
                    );
                    merged_tracks.push(MergedTrack {
                        pdb_track_id: pdb_track.id,
                        pdb_row,
                        edb_media_path: media_path,
                        title: pdb_track.title.clone(),
                        artist: artist_name.to_string(),
                        album,
                        key,
                        track_number: track_number_val,
                        bpm: bpm_val,
                        duration_ms: duration_ms_val,
                        artwork_path: parsed.artworks.get(&pdb_track.artwork_id).cloned(),
                        analysis_path: pdb_track.anlz_path.clone(),
                        sample_rate_hz: pdb_track.sample_rate_hz,
                        bitrate_kbps: pdb_track.bitrate_kbps,
                        bit_depth: pdb_track.bit_depth.map(u32::from),
                        file_size_bytes: pdb_track.file_size_bytes,
                        file_type: pdb_track.file_type.map(u32::from),
                        isrc: pdb_track.isrc.clone(),
                        release_year: pdb_track.release_year.map(u32::from),
                        release_date: pdb_track.release_date.clone(),
                    });
                }
            }

            // Always write eDB: metadata may differ even when track counts match.
            // write_edb_playlist is idempotent, so this is safe.
            let edb_needs_write = true;

            merged_playlists.push(MergedPlaylist {
                name: playlist_name.clone(),
                playlist_id: pl_id,
                sort_order: pl_sort,
                tracks: merged_tracks,
                edb_needs_write,
            });
        }

        result.merged_playlists = merged_playlists.len();

        // ── Phase 3: Rewrite PDB using the current export writer ──────
        let mut desired_pdb_sort_by_id: HashMap<u32, u32> = parsed
            .playlist_tree
            .iter()
            .map(|row| (row.id, row.sort_order))
            .collect();
        for mpl in &merged_playlists {
            desired_pdb_sort_by_id.insert(mpl.playlist_id, mpl.sort_order);
        }
        for mpl in &merged_playlists {
            let (playlist_data, manifest) = build_manifest_for_merged_playlist(usb_root, mpl);
            if let Err(err) = write_pdb(
                usb_root,
                &playlist_data,
                &manifest,
                true,
                Some(mpl.playlist_id),
                Some(mpl.sort_order),
            ) {
                warnings.push(logging::log(
                    Level::Error,
                    "usb-repair",
                    "usb.repair.strict-parity.pdb-rewrite-failed",
                    format!(
                        "strict parity upgrade: PDB rewrite failed for '{}': {err}",
                        mpl.name
                    ),
                ));
                result.failed_playlists += 1;
            }
        }
        if result.merged_playlists > 0
            && let Err(err) = restore_pdb_playlist_sort_orders(usb_root, &desired_pdb_sort_by_id)
        {
            warnings.push(logging::log(
                Level::Error,
                "usb-repair",
                "usb.repair.strict-parity.pdb-order-restore-failed",
                format!("strict parity upgrade: PDB playlist order restore failed: {err}"),
            ));
            result.failed_playlists += 1;
        }

        // ── Phase 3b: Remove stale duplicate playlist_entries rows ──────
        // A playlist's tail can be left behind on a shared tt=8 page from an
        // earlier point in the USB's history when the playlist later grew and
        // its continuation relocated to a fresh page (see docs/PDB.md). The
        // rewrite above only manages rows within each merged playlist's
        // current live boundary, so pre-existing stale copies elsewhere on
        // shared pages survive it. Sweep the whole PDB for any
        // (playlist_id, track_id) pair with more than one active row and
        // tombstone all but the earliest.
        if result.merged_playlists > 0 {
            let pdb_path = usb_staging::stage_pdb(usb_root)?;
            match std::fs::read(&pdb_path) {
                Ok(mut pdb_bytes) => {
                    let removed = crate::pdb_writer::remove_duplicate_playlist_entries_inplace(
                        &mut pdb_bytes,
                    );
                    if removed > 0 {
                        if let Err(err) = std::fs::write(&pdb_path, &pdb_bytes) {
                            warnings.push(logging::log(
                                Level::Error,
                                "usb-repair",
                                "usb.repair.strict-parity.dedup-write-failed",
                                format!(
                                    "strict parity upgrade: failed to write deduplicated PDB: {err}"
                                ),
                            ));
                        } else {
                            result.duplicate_entries_removed = removed;
                        }
                    }
                }
                Err(err) => {
                    warnings.push(logging::log(
                        Level::Error,
                        "usb-repair",
                        "usb.repair.strict-parity.dedup-read-failed",
                        format!(
                            "strict parity upgrade: unable to read PDB for duplicate cleanup: {err}"
                        ),
                    ));
                }
            }
        }

        // Re-read written PDB identities so eDB uses the exact playlist id/sort
        // that ended up in the device-facing PDB after rewrite.
        let pdb_identity_by_name: HashMap<String, (u32, u32)> = {
            let pdb_path = usb_staging::stage_pdb(usb_root)?;
            if pdb_path.is_file() {
                if let Ok(parsed_after_write) = parse_pdb(&pdb_path) {
                    let mut map = HashMap::<String, (u32, u32, usize)>::new();
                    for leaf in parsed_after_write
                        .playlist_tree
                        .iter()
                        .filter(|r| !r.row_is_folder)
                    {
                        let entry_count = parsed_after_write
                            .playlist_entries
                            .iter()
                            .filter(|e| e.playlist_id == leaf.id)
                            .count();
                        match map.get(&leaf.name) {
                            None => {
                                map.insert(
                                    leaf.name.clone(),
                                    (leaf.id, leaf.sort_order, entry_count),
                                );
                            }
                            Some((_, existing_sort, existing_count)) => {
                                let replace = entry_count > *existing_count
                                    || (entry_count == *existing_count
                                        && leaf.sort_order < *existing_sort);
                                if replace {
                                    map.insert(
                                        leaf.name.clone(),
                                        (leaf.id, leaf.sort_order, entry_count),
                                    );
                                }
                            }
                        }
                    }
                    map.into_iter()
                        .map(|(name, (id, sort, _))| (name, (id, sort)))
                        .collect()
                } else {
                    warnings.push(logging::log(
                        Level::Warn,
                        "usb-repair",
                        "usb.repair.strict-parity.identity-sync-parse-failed",
                        "strict parity upgrade: unable to parse rewritten PDB for playlist identity sync; falling back to merged ids",
                    ));
                    HashMap::new()
                }
            } else {
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-repair",
                    "usb.repair.strict-parity.identity-sync-pdb-missing",
                    "strict parity upgrade: rewritten PDB missing during playlist identity sync; falling back to merged ids",
                ));
                HashMap::new()
            }
        };

        // ── Phase 4: Write eDB ────────────────────────────────────────
        // Open (or reuse the caller-supplied) eDB connection once for the
        // whole write phase below, instead of once per playlist -- each
        // open re-runs SQLCipher key negotiation.
        let mut handle =
            EdbConnHandle::acquire(edb_conn.take(), || open_edb_rw(usb_root, warnings));
        for mpl in &merged_playlists {
            let (playlist_data, manifest) = build_manifest_for_merged_playlist(usb_root, mpl);
            if !mpl.edb_needs_write {
                continue;
            }
            let Some(conn) = handle.as_mut().map(EdbConnHandle::conn) else {
                warnings.push(logging::log(
                    Level::Error,
                    "usb-repair",
                    "usb.repair.strict-parity.edb-open-failed",
                    format!(
                        "strict parity upgrade: unable to open eDB for playlist '{}'",
                        mpl.name
                    ),
                ));
                result.failed_playlists += 1;
                continue;
            };
            // Use the re-read PDB id (handles collision remapping). The eDB
            // writer may move this playlist to the front, so all eDB sequenceNos
            // are synced from the final PDB sort order after the write loop.
            let target_playlist_id = pdb_identity_by_name
                .get(&mpl.name)
                .map(|(id, _)| *id)
                .unwrap_or(mpl.playlist_id);
            let tx = conn.transaction()?;
            replace_export_playlist_row_with_identity(
                &tx,
                &playlist_data,
                i64::from(target_playlist_id),
                i64::from(mpl.sort_order),
            )?;
            tx.commit()?;
            match write_edb_playlist_with_conn(conn, &playlist_data, &manifest, true) {
                Ok(_) => result.edb_playlists_written += 1,
                Err(err) => {
                    warnings.push(logging::log(
                        Level::Error,
                        "usb-repair",
                        "usb.repair.strict-parity.edb-write-failed",
                        format!(
                            "strict parity upgrade: eDB write failed for '{}': {err}",
                            mpl.name
                        ),
                    ));
                    result.failed_playlists += 1;
                }
            }
        }
        if result.merged_playlists > 0
            && let Err(err) = sync_edb_playlist_sort_orders_from_pdb(
                usb_root,
                handle.as_mut().map(EdbConnHandle::conn),
                warnings,
            )
        {
            warnings.push(logging::log(
                Level::Error,
                "usb-repair",
                "usb.repair.strict-parity.edb-order-sync-failed",
                format!("strict parity upgrade: eDB playlist order sync failed: {err}"),
            ));
            result.failed_playlists += 1;
        }

        Ok(result)
    }

    fn apply_fix_empty_analysis_files(
        &self,
        usb_root: &std::path::Path,
        empty_analysis_paths: &[String],
        edb_conn: Option<&mut rusqlite::Connection>,
        parsed_pdb: Option<&crate::pdb_reader::ParsedPdb>,
        warnings: &mut Vec<WarningEntry>,
    ) -> BackendResult<(usize, usize, usize, usize)> {
        if empty_analysis_paths.is_empty() {
            return Ok((0, 0, 0, 0));
        }

        #[derive(Clone)]
        struct AnalysisRepairTarget {
            source_audio: String,
            analysis_dir: std::path::PathBuf,
            track_path: String,
        }

        let mut map_by_file = std::collections::HashMap::<String, AnalysisRepairTarget>::new();
        let mut map_by_dir = std::collections::HashMap::<String, AnalysisRepairTarget>::new();
        // Reuse the caller's already-parsed PDB when given -- this fix always
        // runs before anything that mutates the PDB, so it's still current.
        // Only stage/parse our own copy if the caller didn't have one.
        let owned_parsed_pdb;
        let parsed_pdb_ref: Option<&crate::pdb_reader::ParsedPdb> = match parsed_pdb {
            Some(parsed) => Some(parsed),
            None => {
                owned_parsed_pdb = usb_staging::stage_pdb(usb_root)
                    .ok()
                    .and_then(|staged_pdb_path| parse_pdb(&staged_pdb_path).ok());
                owned_parsed_pdb.as_ref()
            }
        };
        if let Some(parsed) = parsed_pdb_ref {
            for t in &parsed.tracks {
                if let (Some(a), Some(s)) = (
                    resolve_usb_side_path(usb_root, &t.anlz_path),
                    resolve_usb_side_path(usb_root, &t.track_file_path),
                ) {
                    let analysis_path = std::path::PathBuf::from(&a);
                    let analysis_dir = analysis_path
                        .parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| usb_root.to_path_buf());
                    let target = AnalysisRepairTarget {
                        source_audio: s,
                        analysis_dir: analysis_dir.clone(),
                        track_path: t.track_file_path.clone(),
                    };
                    map_by_file.insert(canonicalize_playlist_name(&a), target.clone());
                    map_by_dir.insert(
                        canonicalize_playlist_name(&analysis_dir.to_string_lossy()),
                        target,
                    );
                }
            }
        }

        if let Some(mut handle) =
            EdbConnHandle::acquire(edb_conn, || open_edb_from_usb_root(usb_root, warnings))
        {
            let conn = handle.conn();
            let mut stmt = conn.prepare(
                "SELECT path, analysisDataFilePath FROM content WHERE analysisDataFilePath IS NOT NULL",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })?;
            for row in rows {
                let (path, analysis_path) = row?;
                if let (Some(p), Some(a)) = (path.as_deref(), analysis_path.as_deref())
                    && let (Some(ra), Some(rp)) = (
                        resolve_usb_side_path(usb_root, a),
                        resolve_usb_side_path(usb_root, p),
                    )
                {
                    let analysis_path = std::path::PathBuf::from(&ra);
                    let analysis_dir = analysis_path
                        .parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| usb_root.to_path_buf());
                    let target = AnalysisRepairTarget {
                        source_audio: rp,
                        analysis_dir: analysis_dir.clone(),
                        track_path: p.to_string(),
                    };
                    map_by_file
                        .entry(canonicalize_playlist_name(&ra))
                        .or_insert_with(|| target.clone());
                    map_by_dir
                        .entry(canonicalize_playlist_name(&analysis_dir.to_string_lossy()))
                        .or_insert(target);
                }
            }
        }

        let mut fixed = 0usize;
        let mut skipped = 0usize;
        let mut failed = 0usize;
        let mut writes = 0usize;

        for empty_path in empty_analysis_paths {
            let key_file = canonicalize_playlist_name(empty_path);
            let key_dir = std::path::Path::new(empty_path)
                .parent()
                .map(|p| canonicalize_playlist_name(&p.to_string_lossy()))
                .unwrap_or_default();
            let target = map_by_file
                .get(&key_file)
                .cloned()
                .or_else(|| map_by_dir.get(&key_dir).cloned());
            let Some(target) = target else {
                skipped += 1;
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-repair",
                    "usb.repair.empty-analysis.no-mapping",
                    format!("repair skipped (empty analysis): source audio mapping not found for {empty_path}"),
                ));
                continue;
            };
            let source_audio = target.source_audio;
            if !std::path::Path::new(&source_audio).is_file() {
                skipped += 1;
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-repair",
                    "usb.repair.empty-analysis.source-missing",
                    format!("repair skipped (empty analysis): source audio missing for {empty_path} -> {source_audio}"),
                ));
                continue;
            }
            let waveform = build_waveform_preview_from_audio(
                std::path::Path::new(&source_audio),
                super::WAVEFORM_PREVIEW_BINS,
                2_000_000,
            )
            .unwrap_or_else(|_| WaveformData::empty());
            if waveform.peaks.is_empty() {
                failed += 1;
                warnings.push(logging::log(
                    Level::Error,
                    "usb-repair",
                    "usb.repair.empty-analysis.analyze-failed",
                    format!("repair failed (empty analysis): unable to analyze source audio {source_audio}"),
                ));
                continue;
            }
            let base_dir = target.analysis_dir;
            let bundle_paths = AnlzBundlePaths {
                dat_path: base_dir.join("ANLZ0000.DAT"),
                ext_path: base_dir.join("ANLZ0000.EXT"),
                twoex_path: base_dir.join("ANLZ0000.2EX"),
            };
            if let Err(err) = write_generated_anlz_bundle(
                &waveform,
                &bundle_paths,
                &target.track_path,
                None,
                None,
            ) {
                failed += 1;
                warnings.push(logging::log(
                    Level::Error,
                    "usb-repair",
                    "usb.repair.empty-analysis.write-failed",
                    format!("repair failed (empty analysis): {empty_path}: {err}"),
                ));
                continue;
            }
            fixed += 1;
            writes += 3;
        }

        Ok((fixed, skipped, failed, writes))
    }

    fn apply_fix_sync_edb_history_from_pdb(
        &self,
        usb_root: &std::path::Path,
        parsed: &crate::pdb_reader::ParsedPdb,
        edb_conn: Option<&mut rusqlite::Connection>,
        warnings: &mut Vec<WarningEntry>,
    ) -> BackendResult<(usize, usize)> {
        let (history_rows, history_content_rows) = derive_history_sync_payload(parsed);
        let Some(mut handle) = EdbConnHandle::acquire(edb_conn, || open_edb_rw(usb_root, warnings))
        else {
            return Ok((0, 0));
        };
        let conn = handle.conn();
        let tx = conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS history(history_id integer primary key, sequenceNo integer, name varchar, attribute integer, history_id_parent integer); \
             CREATE TABLE IF NOT EXISTS history_content(history_id integer, content_id integer, sequenceNo integer);",
        )?;
        tx.execute("DELETE FROM history_content", [])?;
        tx.execute("DELETE FROM history", [])?;

        for (history_id, sequence_no, name, attribute, history_id_parent) in &history_rows {
            tx.execute(
                "INSERT INTO history(history_id, sequenceNo, name, attribute, history_id_parent) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![history_id, sequence_no, name, attribute, history_id_parent],
            )?;
        }
        for (history_id, content_id, sequence_no) in &history_content_rows {
            tx.execute(
                "INSERT INTO history_content(history_id, content_id, sequenceNo) VALUES (?1, ?2, ?3)",
                params![history_id, content_id, sequence_no],
            )?;
        }

        tx.commit()?;
        Ok((history_rows.len(), history_content_rows.len()))
    }

    fn apply_fix_remove_missing_audio_references(
        &self,
        usb_root: &std::path::Path,
        missing_track_ids: &HashSet<u32>,
        missing_paths: &[String],
        edb_conn: Option<&mut rusqlite::Connection>,
        warnings: &mut Vec<WarningEntry>,
    ) -> BackendResult<(usize, usize, usize)> {
        if missing_track_ids.is_empty() || missing_paths.is_empty() {
            return Ok((0, 0, 0));
        }

        let mut removed_db_content = 0usize;
        let mut removed_db_playlist_links = 0usize;
        let missing_path_set = missing_paths
            .iter()
            .map(|p| normalize_pdb_path_for_edb_lookup(p))
            .collect::<HashSet<_>>();

        if let Some(mut handle) =
            EdbConnHandle::acquire(edb_conn, || open_edb_rw(usb_root, warnings))
        {
            let conn = handle.conn();
            let tx = conn.transaction()?;
            if table_exists(&tx, "content") {
                let columns = load_table_columns(&tx, "content")?;
                let path_col = if columns.iter().any(|c| c == "path") {
                    Some("path")
                } else if columns.iter().any(|c| c == "filePath") {
                    Some("filePath")
                } else if columns.iter().any(|c| c == "file_path") {
                    Some("file_path")
                } else {
                    None
                };

                if let Some(path_col) = path_col {
                    let mut stmt = tx.prepare(&format!(
                        "SELECT content_id, {path_col} FROM content WHERE {path_col} IS NOT NULL"
                    ))?;
                    let rows = stmt.query_map([], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
                    })?;
                    let mut content_ids_to_remove = Vec::<i64>::new();
                    for row in rows {
                        let (content_id, path) = row?;
                        let Some(path) = path else { continue };
                        let normalized = normalize_pdb_path_for_edb_lookup(&path);
                        if missing_path_set.contains(&normalized) {
                            content_ids_to_remove.push(content_id);
                        }
                    }

                    if !content_ids_to_remove.is_empty() {
                        if table_exists(&tx, "playlist_content") {
                            for content_id in &content_ids_to_remove {
                                let changed = tx.execute(
                                    "DELETE FROM playlist_content WHERE content_id = ?1",
                                    rusqlite::params![content_id],
                                )?;
                                removed_db_playlist_links += changed;
                            }
                        }
                        for content_id in &content_ids_to_remove {
                            removed_db_content += tx.execute(
                                "DELETE FROM content WHERE content_id = ?1",
                                rusqlite::params![content_id],
                            )?;
                        }
                    }
                } else {
                    warnings.push(logging::log(
                        Level::Warn,
                        "usb-repair",
                        "usb.repair.missing-audio.no-path-column",
                        "repair skipped (missing audio): eDB content path column not found",
                    ));
                }
            }
            tx.commit()?;
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
        } else {
            warnings.push(logging::log(
                Level::Warn,
                "usb-repair",
                "usb.repair.missing-audio.edb-open-failed",
                "repair skipped (missing audio): unable to open eDB read-write",
            ));
        }

        let removed_pdb_entries = match remove_track_ids_from_pdb_playlist_entries(
            usb_root,
            missing_track_ids,
        ) {
            Ok(removed) => removed,
            Err(err) => {
                warnings.push(logging::log(
                        Level::Error,
                        "usb-repair",
                        "usb.repair.missing-audio.pdb-update-failed",
                        format!("repair skipped (missing audio): unable to update PDB playlist entries ({err})"),
                    ));
                0
            }
        };

        Ok((
            removed_db_content,
            removed_db_playlist_links,
            removed_pdb_entries,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edb::ExportDbPlaylist;
    use crate::models::{DiagCheck, DiagStatus, UsbParityPlaylistDetail};
    use crate::pdb_reader::{
        ParsedPdb, PdbHistoryEntryRow, PdbHistoryPlaylistRow, PdbPlaylistTreeRow,
    };
    use crate::service::export_helpers::inspect_pdb_columns_playlist_order;
    use tempfile::tempdir;

    fn make_strict_repair_detail(name: &str) -> UsbParityPlaylistDetail {
        UsbParityPlaylistDetail {
            name: name.to_string(),
            pdb_tracks: 1,
            edb_tracks: 1,
            matched_tracks: 1,
            only_in_pdb: 0,
            only_in_edb: 0,
            order_mismatch: false,
            path_mismatch_tracks: 0,
            dictionary_id_issue_tracks: 0,
            playlist_id_match: true,
            sort_order_match: true,
            parent_match: Some(true),
            pdb_playlist_id: Some(1),
            edb_playlist_id: Some(1),
            pdb_sort_order: Some(1),
            edb_sort_order: Some(1),
            pdb_duplicate_entries: 0,
            edb_missing_core_metadata: 0,
            pdb_missing_core_metadata: 0,
            artwork_mismatch_tracks: 0,
            sample_only_in_pdb: Vec::new(),
            sample_only_in_edb: Vec::new(),
            sample_metadata_mismatches: Vec::new(),
            status: DiagStatus::Pass,
        }
    }

    #[test]
    fn playlist_requires_strict_upgrade_detects_failure_cases() {
        let mut failing = make_strict_repair_detail("Failing");
        failing.status = DiagStatus::Fail;
        failing.pdb_missing_core_metadata = 1;
        assert!(playlist_requires_strict_upgrade(&failing));

        let mut pdb_only = make_strict_repair_detail("PDB Only");
        pdb_only.status = DiagStatus::Fail;
        pdb_only.only_in_pdb = 1;
        assert!(playlist_requires_strict_upgrade(&pdb_only));

        let clean = make_strict_repair_detail("Clean");
        assert!(!playlist_requires_strict_upgrade(&clean));
    }

    #[test]
    fn strict_repair_playlist_names_follow_pdb_order_with_edb_only_tail() {
        let mut parsed = ParsedPdb::default();
        parsed.playlist_tree.push(PdbPlaylistTreeRow {
            id: 10,
            parent_id: 0,
            sort_order: 1,
            row_is_folder: false,
            name: "Zeta".to_string(),
        });
        parsed.playlist_tree.push(PdbPlaylistTreeRow {
            id: 20,
            parent_id: 0,
            sort_order: 2,
            row_is_folder: false,
            name: "Alpha".to_string(),
        });

        let mut edb_playlists = HashMap::<String, ExportDbPlaylist>::new();
        edb_playlists.insert(
            "Alpha".to_string(),
            ExportDbPlaylist {
                playlist_id: 20,
                sort_order: 0,
                tracks: Vec::new(),
            },
        );
        edb_playlists.insert(
            "Zeta".to_string(),
            ExportDbPlaylist {
                playlist_id: 10,
                sort_order: 1,
                tracks: Vec::new(),
            },
        );
        edb_playlists.insert(
            "Beta".to_string(),
            ExportDbPlaylist {
                playlist_id: 30,
                sort_order: 0,
                tracks: Vec::new(),
            },
        );

        let names = strict_repair_ordered_playlist_names(&parsed, Some(&edb_playlists));

        assert_eq!(names, vec!["Zeta", "Alpha", "Beta"]);
    }

    #[test]
    fn derive_history_sync_payload_uses_named_histories_and_valid_entries() {
        let parsed = ParsedPdb {
            history_playlists: vec![
                PdbHistoryPlaylistRow {
                    id: 10,
                    name: "HISTORY 001".to_string(),
                    source_table: 11,
                },
                PdbHistoryPlaylistRow {
                    id: 11,
                    name: "Not History".to_string(),
                    source_table: 11,
                },
            ],
            history_entries: vec![
                PdbHistoryEntryRow {
                    track_id: Some(101),
                    playlist_id: 10,
                    entry_index: 1,
                    source_table: 12,
                },
                PdbHistoryEntryRow {
                    track_id: Some(0),
                    playlist_id: 10,
                    entry_index: 2,
                    source_table: 12,
                },
                PdbHistoryEntryRow {
                    track_id: Some(202),
                    playlist_id: 11,
                    entry_index: 1,
                    source_table: 12,
                },
            ],
            ..ParsedPdb::default()
        };

        let (history_rows, history_content_rows) = derive_history_sync_payload(&parsed);
        assert_eq!(history_rows, vec![(10, 0, "HISTORY 001".to_string(), 0, 0)]);
        assert_eq!(history_content_rows, vec![(10, 101, 1)]);
    }

    #[test]
    fn strict_raw_coverage_issue_wording_uses_raw_count_parity_terms() {
        let checks = vec![DiagCheck {
            label: "USB audio directory raw coverage parity (strict)".to_string(),
            status: DiagStatus::Warn,
            detail: "extra raw audio files in USB audio directory within warn threshold: +3 file(s) beyond strict indexed set (raw=103, indexed=100, warn<= 5)".to_string(),
            link: None,
        }];
        let issue = strict_raw_coverage_issue_from_parity_checks(&checks)
            .expect("expected strict raw-coverage issue");
        assert!(issue.contains("strict raw-count parity drift"));
        assert!(!issue.to_lowercase().contains("unreferenced"));
        assert!(!issue.to_lowercase().contains("not indexed"));
    }

    #[test]
    fn strict_raw_coverage_issue_absent_when_check_is_pass() {
        let checks = vec![DiagCheck {
            label: "USB audio directory raw coverage parity (strict)".to_string(),
            status: DiagStatus::Pass,
            detail: "no extra raw audio files in USB audio directory beyond strict indexed set (raw=100, indexed=100)"
                .to_string(),
            link: None,
        }];
        assert!(strict_raw_coverage_issue_from_parity_checks(&checks).is_none());
    }

    fn test_usb_root() -> (tempfile::TempDir, std::path::PathBuf) {
        let td = tempdir().expect("tempdir");
        let usb_root = td.path().join("USB_TEST");
        std::fs::create_dir_all(&usb_root).expect("create usb root");
        crate::service::initialize_usb(
            usb_root
                .to_str()
                .expect("usb root path should be valid utf-8"),
        )
        .expect("initialize usb");
        (td, usb_root)
    }

    #[test]
    fn player_menu_config_roundtrip_updates_edb_not_pdb() {
        let (_td, usb_root) = test_usb_root();
        let service_data_dir = tempdir().expect("service data dir");
        let service = BackendService::new(service_data_dir.path()).expect("backend service");

        let before = service
            .get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            })
            .expect("get cdj menu config");
        assert!(
            !before.current_items.is_empty(),
            "expected initialized USB to have visible player menu items"
        );
        // Playlist (menuItem_id=17) is at sequenceNo=5 by default — not first.
        assert_ne!(
            before.current_items.first().map(|i| i.menu_item_id),
            Some(17),
            "playlist should not be first in default seeded state"
        );
        let pdb_row_count_before = inspect_pdb_columns_playlist_order(&usb_root)
            .expect("inspect pdb before")
            .expect("pdb t16 rows available")
            .1;

        // Protected kinds (TRACK=4, HISTORY=19, SEARCH=20) must remain in the
        // list even though the intent of this test is to check ordering only.
        let updated = service
            .update_usb_player_menu_config(UpdateUsbPlayerMenuConfigRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                current_menu_item_ids: vec![17, 2, 3, 24, 4, 19, 20],
                current_kinds: vec![],
            })
            .expect("update cdj menu config");
        assert!(updated.updated, "expected player menu update to be applied");

        let ordered_ids = updated
            .current_items
            .iter()
            .map(|item| item.menu_item_id)
            .collect::<Vec<_>>();
        assert_eq!(&ordered_ids[0..4], &[17, 2, 3, 24]);

        // PDB t16 must NOT be modified — row count stays the same.
        let pdb_after = inspect_pdb_columns_playlist_order(&usb_root)
            .expect("inspect pdb after update")
            .expect("pdb t16 rows available");
        assert_eq!(
            pdb_after.1, pdb_row_count_before,
            "PDB t16 row count must not change: category edit is eDB-only"
        );

        // Re-read via get_usb_player_menu_config to confirm the change persisted in eDB.
        let after = service
            .get_usb_player_menu_config(GetUsbPlayerMenuConfigRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            })
            .expect("re-read cdj menu config after update");
        let after_ids = after
            .current_items
            .iter()
            .map(|i| i.menu_item_id)
            .collect::<Vec<_>>();
        assert_eq!(
            &after_ids[0..4],
            &[17, 2, 3, 24],
            "menu order must persist across a re-read"
        );
    }

    // ── Local HDD staging integration ─────────────────────────────────
    //
    // These follow the same pattern edb.rs's staging tests already
    // established: `set_cache_root_for_test` overrides `cache_root()` only
    // for the calling thread, and `cargo test` never runs two tests
    // concurrently on the same thread, so this needs no lock and can't
    // race with unrelated tests on other threads. The returned guard resets
    // the override on drop (including on panic), and the cache tempdir is
    // leaked via `.keep()` as harmless extra insurance.

    #[test]
    fn apply_pdb_sentinel_u5_repair_writes_to_staged_local_copy_when_enabled() {
        let cache_dir = tempdir().expect("cache dir").keep();
        let _guard = usb_staging::set_cache_root_for_test(Some(cache_dir));

        let mut p1 = data_page(1, 7);
        set_nrs(&mut p1, 3);
        set_u5_num_rl(&mut p1, 0x1FFF, 0x1FFF);
        let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
        let raw_pdb_path = vendor_pdb_path(&usb_root);
        let raw_bytes_before = std::fs::read(&raw_pdb_path).unwrap();

        let staged_path = usb_staging::stage_pdb(&usb_root).expect("stage pdb");
        assert_ne!(
            staged_path, raw_pdb_path,
            "staging must route through a local path distinct from the USB mount"
        );
        let pages = detect_pdb_sentinel_u5_on_data_pages(&staged_path);
        assert_eq!(pages.len(), 1, "expected one sentinel-u5 page detected");

        let patched = apply_pdb_sentinel_u5_repair(&usb_root, &pages).unwrap();
        assert_eq!(patched, 1);

        // The raw USB file must be completely untouched -- the write landed
        // in the local staged copy only, per the batching decision (no
        // individual writer self-commits).
        let raw_bytes_after = std::fs::read(&raw_pdb_path).unwrap();
        assert_eq!(
            raw_bytes_before, raw_bytes_after,
            "write must not reach the raw USB path directly"
        );

        // The local staged copy must reflect the patch.
        let staged_bytes = std::fs::read(&staged_path).unwrap();
        assert_ne!(
            raw_bytes_before, staged_bytes,
            "local staged copy should reflect the patch"
        );
        assert!(
            detect_pdb_sentinel_u5_on_data_pages(&staged_path).is_empty(),
            "repair should be idempotent against the staged copy"
        );
    }

    #[test]
    fn repair_apply_batches_local_writes_and_flushes_pdb_to_usb_once() {
        let cache_dir = tempdir().expect("cache dir").keep();
        let _guard = usb_staging::set_cache_root_for_test(Some(cache_dir));

        let (_td, usb_root) = test_usb_root();
        let service_data_dir = tempdir().expect("service data dir");
        let service = BackendService::new(service_data_dir.path()).expect("backend service");

        // Corrupt the header-compatibility field (offset 0x10) on the real,
        // initialized USB's export.pdb to an unknown value, so the repair
        // detects and proposes fixing it.
        let raw_pdb_path = vendor_pdb_path(&usb_root);
        {
            let mut bytes = std::fs::read(&raw_pdb_path).expect("read seed pdb");
            bytes[0x10..0x14].copy_from_slice(&99u32.to_le_bytes());
            std::fs::write(&raw_pdb_path, &bytes).expect("write corrupted header value");
        }
        let raw_bytes_before_apply = std::fs::read(&raw_pdb_path).unwrap();

        let result = service
            .repair_usb_diagnostics_with_progress(
                RepairUsbDiagnosticsRequest {
                    usb_root: Some(usb_root.to_string_lossy().to_string()),
                    apply: true,
                    selected_fix_ids: vec![PDB_HEADER_COMPATIBILITY_FIX_ID.to_string()],
                },
                |_, _, _| {},
            )
            .expect("repair apply");

        assert!(
            !result.applied_fixes.is_empty() && result.failed_fixes.is_empty(),
            "expected header-compatibility fix to apply cleanly: {result:?}"
        );

        // The batched end-of-apply write-back must have reached the *raw*
        // USB path, not just the local cache -- this is what the
        // `?`-early-return fix and the write-back call placement exist to
        // guarantee.
        let raw_bytes_after_apply = std::fs::read(&raw_pdb_path).unwrap();
        assert_ne!(
            raw_bytes_before_apply, raw_bytes_after_apply,
            "USB file must reflect the fix after the batched write-back"
        );
        let value_after = u32::from_le_bytes(raw_bytes_after_apply[0x10..0x14].try_into().unwrap());
        assert_eq!(value_after, PDB_HEADER_COMPATIBILITY_FALLBACK_VALUE);
    }

    #[test]
    fn apply_fix_sync_edb_history_from_pdb_flushes_to_usb_when_staging_enabled() {
        let cache_dir = tempdir().expect("cache dir").keep();
        let _guard = usb_staging::set_cache_root_for_test(Some(cache_dir));

        let (_td, usb_root) = test_usb_root();
        let service_data_dir = tempdir().expect("service data dir");
        let service = BackendService::new(service_data_dir.path()).expect("backend service");

        let parsed = ParsedPdb {
            history_playlists: vec![PdbHistoryPlaylistRow {
                id: 10,
                name: "HISTORY 001".to_string(),
                source_table: 17,
            }],
            history_entries: vec![PdbHistoryEntryRow {
                track_id: Some(101),
                playlist_id: 10,
                entry_index: 1,
                source_table: 18,
            }],
            ..ParsedPdb::default()
        };

        let mut warnings = Vec::new();
        let (history_written, _history_content_written) = service
            .apply_fix_sync_edb_history_from_pdb(&usb_root, &parsed, None, &mut warnings)
            .expect("sync edb history");
        assert_eq!(history_written, 1);

        // Before any write-back: the write went through `open_edb_rw`, which
        // is staging-transparent, so it must have landed in the local
        // staged copy only -- the raw USB eDB file must not have the new
        // row yet. This is the eDB flush gap this whole change exists to
        // close: proving it *doesn't* reach USB here is as important as
        // proving it *does* after the flush below.
        // The real initialize_usb-seeded eDB is SQLCipher-encrypted, so
        // `crate::edb::open_edb` (which tries the known keys) is used
        // instead of a plain `rusqlite::Connection::open` -- applied
        // directly to the raw path to bypass staging entirely.
        let raw_edb_path = super::super::usb_vendor_compat::vendor_edb_path(&usb_root);
        {
            let mut open_warnings = Vec::new();
            let conn =
                crate::edb::open_edb(&raw_edb_path, &mut open_warnings).expect("open raw usb edb");
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(1) FROM history WHERE name = 'HISTORY 001'",
                    [],
                    |r| r.get(0),
                )
                .expect("query raw usb edb before write-back");
            assert_eq!(
                count, 0,
                "raw USB eDB must not see the change before write-back"
            );
        }

        let committed =
            usb_staging::write_back_if_changed(&usb_root, usb_staging::DbKind::Edb).unwrap();
        assert!(committed, "expected the eDB write-back to actually commit");

        let mut open_warnings = Vec::new();
        let conn =
            crate::edb::open_edb(&raw_edb_path, &mut open_warnings).expect("re-open raw usb edb");
        let (hid, name): (i64, String) = conn
            .query_row("SELECT history_id, name FROM history", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .expect("query raw usb edb after write-back");
        assert_eq!(hid, 10);
        assert_eq!(name, "HISTORY 001");
    }

    // ── Hand-built minimal PDB fixtures for the byte-level repair pairs ──
    //
    // These construct a minimal but structurally valid PDB file (a header
    // page plus one or more data pages) directly in memory rather than
    // corrupting a real export, since these detect_pdb_*/apply_pdb_*_repair
    // functions are pure `&[u8]` byte-format functions. Field offsets below
    // mirror what each detect_pdb_* function reads: idx@+4, table_type@+8,
    // next_page@+0xc, nrs@+0x18 (single byte) or packed 3-byte row count
    // @0x18..0x1b, page_flags@+0x1b, used_s@+0x1e, u5@+0x20, num_rl@+0x22.

    const TEST_PAGE_SIZE: usize = 4096;

    fn empty_page() -> Vec<u8> {
        vec![0u8; TEST_PAGE_SIZE]
    }

    fn header_page() -> Vec<u8> {
        let mut p = empty_page();
        p[4..8].copy_from_slice(&(TEST_PAGE_SIZE as u32).to_le_bytes());
        p
    }

    /// A minimal non-empty, non-sentinel data page: idx set, used_s nonzero,
    /// flags/u5/num_rl/nrs zeroed (callers overwrite what they need).
    fn data_page(page_index: u32, table_type: u32) -> Vec<u8> {
        let mut p = empty_page();
        p[4..8].copy_from_slice(&page_index.to_le_bytes());
        p[8..12].copy_from_slice(&table_type.to_le_bytes());
        p[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes()); // used_s: nonzero placeholder
        p
    }

    fn set_flags(page: &mut [u8], flags: u8) {
        page[0x1b] = flags;
    }

    fn set_next_page(page: &mut [u8], next: u32) {
        page[0x0c..0x10].copy_from_slice(&next.to_le_bytes());
    }

    fn set_nrs(page: &mut [u8], nrs: u8) {
        page[0x18] = nrs;
    }

    fn set_u5_num_rl(page: &mut [u8], u5: u16, num_rl: u16) {
        page[0x20..0x22].copy_from_slice(&u5.to_le_bytes());
        page[0x22..0x24].copy_from_slice(&num_rl.to_le_bytes());
    }

    /// USB-root-shaped wrapper so `apply_*` functions (which build their own
    /// `vendor_pdb_path(usb_root)`) find the fixture PDB.
    fn write_pdb_pages_as_usb_root(pages: Vec<Vec<u8>>) -> (tempfile::TempDir, std::path::PathBuf) {
        let td = tempdir().expect("tempdir");
        let usb_root = td.path().to_path_buf();
        let pdb_path = vendor_pdb_path(&usb_root);
        std::fs::create_dir_all(pdb_path.parent().unwrap()).expect("create vendor db dir");
        let mut bytes = Vec::with_capacity(pages.len() * TEST_PAGE_SIZE);
        for p in &pages {
            assert_eq!(
                p.len(),
                TEST_PAGE_SIZE,
                "page must be exactly one page long"
            );
            bytes.extend_from_slice(p);
        }
        std::fs::write(&pdb_path, &bytes).expect("write test pdb");
        (td, usb_root)
    }

    #[test]
    fn sentinel_u5_repair_detects_and_fixes_sentinel_footer() {
        let mut p1 = data_page(1, 7); // tt=7 (playlist_tree): correct is (nrs, 0)
        set_nrs(&mut p1, 3);
        set_u5_num_rl(&mut p1, 0x1FFF, 0x1FFF);
        let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
        let pdb_path = vendor_pdb_path(&usb_root);

        let pages = detect_pdb_sentinel_u5_on_data_pages(&pdb_path);
        assert_eq!(pages.len(), 1, "expected one sentinel-u5 page detected");
        assert_eq!(pages[0].page_index, 1);

        // No-op on empty input.
        assert_eq!(apply_pdb_sentinel_u5_repair(&usb_root, &[]).unwrap(), 0);

        let patched = apply_pdb_sentinel_u5_repair(&usb_root, &pages).unwrap();
        assert_eq!(patched, 1);

        let bytes = std::fs::read(&pdb_path).unwrap();
        let off = TEST_PAGE_SIZE;
        assert_eq!(
            u16::from_le_bytes(bytes[off + 0x20..off + 0x22].try_into().unwrap()),
            3
        );
        assert_eq!(
            u16::from_le_bytes(bytes[off + 0x22..off + 0x24].try_into().unwrap()),
            0
        );

        assert!(
            detect_pdb_sentinel_u5_on_data_pages(&pdb_path).is_empty(),
            "repair should be idempotent"
        );
    }

    #[test]
    fn wrong_page_flags_repair_detects_and_fixes_invalid_flags() {
        let mut p1 = data_page(1, 7); // tt=7: only 0x24 is valid
        set_flags(&mut p1, 0x99);
        let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
        let pdb_path = vendor_pdb_path(&usb_root);

        let pages = detect_pdb_wrong_page_flags(&pdb_path);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].page_index, 1);
        assert_eq!(pages[0].correct_flags, 0x24);

        assert_eq!(
            apply_pdb_wrong_page_flags_repair(&usb_root, &[]).unwrap(),
            0
        );

        let patched = apply_pdb_wrong_page_flags_repair(&usb_root, &pages).unwrap();
        assert_eq!(patched, 1);

        let bytes = std::fs::read(&pdb_path).unwrap();
        assert_eq!(bytes[TEST_PAGE_SIZE + 0x1b], 0x24);
        assert!(detect_pdb_wrong_page_flags(&pdb_path).is_empty());
    }

    #[test]
    fn wrong_page_flags_repair_allows_active_flag_on_track_table() {
        // tt=0 (tracks) accepts both 0x24 and 0x34; only a value outside that
        // set should be flagged, and the correction target is 0x34 (active).
        let mut p1 = data_page(1, 0);
        set_flags(&mut p1, 0x11);
        let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
        let pdb_path = vendor_pdb_path(&usb_root);

        let pages = detect_pdb_wrong_page_flags(&pdb_path);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].correct_flags, 0x34);
    }

    #[test]
    fn zero_tranrf_repair_detects_and_fixes_missing_transaction_flags() {
        // tt=0, u5=2 (non-dict shape): tranrf must mirror rowpf in every group.
        let mut p1 = data_page(1, 0);
        set_nrs(&mut p1, 1);
        set_u5_num_rl(&mut p1, 2, 0);
        // Single footer group at the tail of the page: [row-offset(u16)] [rowpf(u16)] [tranrf(u16)]
        // laid out backwards from the end: cursor -= 4 (rowpf|tranrf), then -= glen*2 (offsets).
        let rowpf_off = TEST_PAGE_SIZE - 4;
        let tranrf_off = TEST_PAGE_SIZE - 2;
        p1[rowpf_off..rowpf_off + 2].copy_from_slice(&1u16.to_le_bytes()); // rowpf != 0
        p1[tranrf_off..tranrf_off + 2].copy_from_slice(&0u16.to_le_bytes()); // tranrf == 0 (bug)
        let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
        let pdb_path = vendor_pdb_path(&usb_root);

        let pages = detect_pdb_zero_tranrf_all_tables(&pdb_path);
        assert_eq!(pages.len(), 1, "expected one zero-tranrf page detected");
        assert_eq!(pages[0].page_index, 1);

        assert_eq!(apply_pdb_zero_tranrf_repair(&usb_root, &[]).unwrap(), 0);

        let patched = apply_pdb_zero_tranrf_repair(&usb_root, &pages).unwrap();
        assert_eq!(patched, 1);

        let bytes = std::fs::read(&pdb_path).unwrap();
        let off = TEST_PAGE_SIZE + tranrf_off;
        assert_eq!(
            u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap()),
            1,
            "tranrf should now mirror rowpf"
        );
        assert!(detect_pdb_zero_tranrf_all_tables(&pdb_path).is_empty());
    }

    #[test]
    fn wrong_history_page_shape_repair_detects_and_fixes_old_bug_pattern() {
        // tt=17 (columns/history-family): correct convention is (nrs, 0); the
        // old-bug pattern (u5=1, num_rl=nrs-1) must be flagged and corrected.
        let mut p1 = data_page(1, 17);
        set_nrs(&mut p1, 4);
        set_u5_num_rl(&mut p1, 1, 3); // nrs-1 == 3
        let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
        let pdb_path = vendor_pdb_path(&usb_root);

        let pages = detect_pdb_wrong_history_page_shape(&pdb_path);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].nrs, 4);

        assert_eq!(
            apply_pdb_wrong_history_page_shape_repair(&usb_root, &[]).unwrap(),
            0
        );
        let patched = apply_pdb_wrong_history_page_shape_repair(&usb_root, &pages).unwrap();
        assert_eq!(patched, 1);

        let bytes = std::fs::read(&pdb_path).unwrap();
        let off = TEST_PAGE_SIZE;
        assert_eq!(
            u16::from_le_bytes(bytes[off + 0x20..off + 0x22].try_into().unwrap()),
            4
        );
        assert_eq!(
            u16::from_le_bytes(bytes[off + 0x22..off + 0x24].try_into().unwrap()),
            0
        );
        assert!(detect_pdb_wrong_history_page_shape(&pdb_path).is_empty());
    }

    /// Build a small realistic PDB (via the real writer) with one track and one album, both
    /// carrying non-ASCII names, so their string slots are UTF-16 encoded.
    fn build_seed_pdb_bytes_for_alignment_test() -> Vec<u8> {
        let mut data = crate::pdb_writer::PdbData::empty();
        data.artists.push(crate::pdb_writer::PdbArtistRow {
            id: 1,
            name: "Existing Artist".into(),
        });
        data.albums.push(crate::pdb_writer::PdbAlbumRow {
            id: 1,
            name: "Café Album".into(),
            artist_id: 1,
        });
        data.genres.push(crate::pdb_writer::PdbDictRow {
            id: 1,
            name: "Existing Genre".into(),
        });
        data.keys.push(crate::pdb_writer::PdbKeyRow {
            id: 1,
            name: "Am".into(),
        });
        data.tracks.push(PdbTrackRowData {
            header_flags_u32: None,
            id: 1,
            artist_id: 1,
            album_id: 1,
            artwork_id: 0,
            key_id: 1,
            genre_id: 0,
            title: "Café Track".into(),
            anlz_path: "/PIONEER/USBANLZ/P000/00000001/ANLZ0000.DAT".into(),
            file_path: "/Contents/existing.flac".into(),
            content_link: None,
            sample_rate_hz: None,
            file_size_bytes: None,
            master_content_id: None,
            master_db_id: None,
            bitrate_kbps: None,
            track_number: None,
            bpm: None,
            release_year: None,
            bit_depth: None,
            duration_seconds: Some(180),
            file_type: None,
            isrc: None,
            date_added: None,
            release_date: None,
            dj_comment: None,
            file_name: None,
            publish_track_info_on: None,
            autoload_hotcues_on: None,
        });
        data.colors = crate::pdb_writer::standard_colors();
        data.columns_raw_rows = crate::pdb_writer::standard_columns_raw();
        crate::pdb_writer::write_pdb(&data).expect("write seed")
    }

    /// Corrupt the on-disk track row (id 1) and album row (id 1) to simulate a misaligned export:
    /// insert one byte before the UTF-16 string content and bump the recorded offset, breaking
    /// 4-byte alignment without changing the decoded content.
    fn corrupt_alignment_in_place(bytes: &mut [u8]) {
        use crate::pdb_writer::{read_present_page_rows, rewrite_variable_page_rows_in_place};
        use crate::utils::{
            collect_chain, page_offset, read_u16_le_at, read_u32_le_at, table_ptr_fields,
        };
        const PAGE_SIZE: usize = 4096;

        for (table_type, id_offset) in [(0u32, 72usize), (3u32, 12usize)] {
            let (_ec, first, last) =
                table_ptr_fields(bytes, table_type).expect("table pointer present");
            let chain = collect_chain(bytes, PAGE_SIZE, first, last).expect("chain present");
            for page_idx in chain.into_iter().skip(1) {
                let off = page_offset(page_idx, PAGE_SIZE).expect("page offset");
                let Some(page_view) = bytes.get(off..off + PAGE_SIZE) else {
                    continue;
                };
                let used_s = read_u16_le_at(page_view, 30).unwrap_or(0);
                if used_s == 0 {
                    continue;
                }
                let page = page_view.to_vec();
                let mut rows = read_present_page_rows(&page, PAGE_SIZE);
                let mut any_changed = false;
                for row in rows.iter_mut() {
                    if read_u32_le_at(row, id_offset) != Some(1) {
                        continue;
                    }
                    if table_type == 0 {
                        let off17 = read_u16_le_at(row, 94 + 2 * 17).unwrap() as usize;
                        assert_eq!(row[off17], 0x90);
                        row.insert(off17, 0u8);
                        for index in 17..21usize {
                            let off_pos = 94 + 2 * index;
                            let cur = read_u16_le_at(row, off_pos).unwrap();
                            row[off_pos..off_pos + 2].copy_from_slice(&(cur + 1).to_le_bytes());
                        }
                    } else {
                        let name_offset = row[21] as usize;
                        assert_eq!(row[name_offset], 0x90);
                        row.insert(name_offset, 0u8);
                        row[21] = (name_offset + 1) as u8;
                    }
                    any_changed = true;
                }
                if any_changed {
                    assert!(rewrite_variable_page_rows_in_place(
                        bytes, page_idx, &rows, PAGE_SIZE
                    ));
                }
            }
        }
    }

    #[test]
    fn detect_pdb_string_misalignment_finds_corrupted_track_and_album_rows() {
        let mut bytes = build_seed_pdb_bytes_for_alignment_test();
        assert!(
            detect_pdb_string_misalignment_bytes(&bytes).is_empty(),
            "current writer must produce aligned rows"
        );
        corrupt_alignment_in_place(&mut bytes);

        let td = tempdir().expect("tempdir");
        let usb_root = td.path().to_path_buf();
        let pdb_path = vendor_pdb_path(&usb_root);
        std::fs::create_dir_all(pdb_path.parent().unwrap()).expect("create vendor db dir");
        std::fs::write(&pdb_path, &bytes).expect("write corrupted pdb");

        let found = detect_pdb_string_misalignment(&pdb_path);
        assert!(
            found.iter().any(|m| m.table_type == 0 && m.id == 1),
            "must detect the misaligned track row"
        );
        assert!(
            found.iter().any(|m| m.table_type == 3 && m.id == 1),
            "must detect the misaligned album row"
        );
    }

    /// Test-only helper mirroring `detect_pdb_string_misalignment` but operating on an in-memory
    /// byte buffer, so the "current writer produces clean output" precondition can be asserted
    /// without a round-trip through a temp file.
    fn detect_pdb_string_misalignment_bytes(bytes: &[u8]) -> Vec<MisalignedStringSlot> {
        let td = tempdir().expect("tempdir");
        let usb_root = td.path().to_path_buf();
        let pdb_path = vendor_pdb_path(&usb_root);
        std::fs::create_dir_all(pdb_path.parent().unwrap()).expect("create vendor db dir");
        std::fs::write(&pdb_path, bytes).expect("write pdb");
        detect_pdb_string_misalignment(&pdb_path)
    }

    #[test]
    fn repair_usb_diagnostics_proposes_and_applies_string_alignment_fixes() {
        let mut bytes = build_seed_pdb_bytes_for_alignment_test();
        corrupt_alignment_in_place(&mut bytes);

        let td = tempdir().expect("tempdir");
        let usb_root = td.path().to_path_buf();
        let pdb_path = vendor_pdb_path(&usb_root);
        std::fs::create_dir_all(pdb_path.parent().unwrap()).expect("create vendor db dir");
        std::fs::write(&pdb_path, &bytes).expect("write corrupted pdb");

        let found_before = detect_pdb_string_misalignment(&pdb_path);
        assert!(!found_before.is_empty());

        let track_ids: Vec<u32> = found_before
            .iter()
            .filter(|m| m.table_type == 0)
            .map(|m| m.id)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let album_ids: Vec<u32> = found_before
            .iter()
            .filter(|m| m.table_type == 3)
            .map(|m| m.id)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        assert_eq!(track_ids, vec![1]);
        assert_eq!(album_ids, vec![1]);

        let track_patched = apply_pdb_track_string_alignment_repair(&usb_root, &track_ids)
            .expect("apply track repair");
        assert_eq!(track_patched, 1);
        let album_patched = apply_pdb_album_string_alignment_repair(&usb_root, &album_ids)
            .expect("apply album repair");
        assert_eq!(album_patched, 1);

        assert!(
            detect_pdb_string_misalignment(&pdb_path).is_empty(),
            "no misaligned rows should remain after both repairs"
        );

        let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse repaired pdb");
        assert_eq!(
            parsed
                .tracks
                .iter()
                .find(|t| t.id == 1)
                .map(|t| t.title.as_str()),
            Some("Café Track")
        );
        assert_eq!(
            parsed.albums.get(&1).map(String::as_str),
            Some("Café Album")
        );
    }

    /// Reproduces a real-world report: a track exported by an old (pre-alignment-fix) build was
    /// played on a CDJ, which recorded it into a PDB history playlist; the user then removed the
    /// track's regular playlist. A later library rewrite orphaned the album's original data page
    /// from the live t03 chain (the table's first/last pointers moved on, e.g. because a fresh
    /// growth page took over) without touching that page's still-`present` misaligned row.
    /// `detect_pdb_string_misalignment` still finds it (it scans every physical page, not just
    /// the live chain) and CDJ hardware can still choke on it, but the repair used to walk only
    /// the live chain and would silently report success without ever touching the orphaned row.
    #[test]
    fn album_string_alignment_repair_fixes_row_orphaned_from_live_t03_chain() {
        let mut bytes = build_seed_pdb_bytes_for_alignment_test();
        corrupt_alignment_in_place(&mut bytes);
        const PAGE_SIZE: usize = 4096;

        // Orphan the t03 data page (still holding the misaligned album id=1 row) from the live
        // chain by repointing the table's first/last pointers at a freshly-appended, empty page
        // instead -- simulating a later rewrite that grew the table onto a new page without
        // revisiting the old page's content.
        let (ec, first, last) =
            crate::utils::table_ptr_fields(&bytes, 3).expect("t03 pointer present");
        let new_page = (bytes.len() / PAGE_SIZE) as u32;
        bytes.extend(std::iter::repeat_n(0u8, PAGE_SIZE));
        assert!(
            crate::utils::set_table_ptr_fields(&mut bytes, 3, ec, new_page, new_page),
            "repoint t03 first/last at the fresh empty page"
        );
        // Sanity: the orphaned page is unreachable from the new chain, but the row is still on
        // disk and still flagged by the raw scan.
        assert!(crate::utils::collect_chain(&bytes, PAGE_SIZE, new_page, new_page).is_some());
        assert_ne!(first, new_page);
        assert_ne!(last, new_page);

        let td = tempdir().expect("tempdir");
        let usb_root = td.path().to_path_buf();
        let pdb_path = vendor_pdb_path(&usb_root);
        std::fs::create_dir_all(pdb_path.parent().unwrap()).expect("create vendor db dir");
        std::fs::write(&pdb_path, &bytes).expect("write corrupted pdb");

        let found_before = detect_pdb_string_misalignment(&pdb_path);
        assert!(
            found_before.iter().any(|m| m.table_type == 3 && m.id == 1),
            "raw scan must still find the orphaned misaligned album row"
        );

        let album_patched =
            apply_pdb_album_string_alignment_repair(&usb_root, &[1]).expect("apply album repair");
        assert_eq!(
            album_patched, 1,
            "repair must find and fix the orphaned row, not silently report 0"
        );

        assert!(
            !detect_pdb_string_misalignment(&pdb_path)
                .iter()
                .any(|m| m.table_type == 3),
            "no misaligned album rows should remain after repair, including orphaned ones \
             (the seed's track row was left deliberately uncorrected -- only album alignment \
             is under test here)"
        );
    }

    #[test]
    fn wrong_track_u5_repair_detects_and_fixes_active_page_footer() {
        // tt=0, flags=0x34 (active): only (u5=2, num_rl=0) or (u5=1, num_rl=nrs-1)
        // are valid; anything else must be flagged and normalised to (2, 0).
        let mut p1 = data_page(1, 0);
        set_flags(&mut p1, 0x34);
        set_nrs(&mut p1, 5);
        set_u5_num_rl(&mut p1, 99, 99);
        let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
        let pdb_path = vendor_pdb_path(&usb_root);

        let pages = detect_pdb_wrong_track_u5(&pdb_path);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].page_index, 1);

        assert_eq!(apply_pdb_wrong_track_u5_repair(&usb_root, &[]).unwrap(), 0);
        let patched = apply_pdb_wrong_track_u5_repair(&usb_root, &pages).unwrap();
        assert_eq!(patched, 1);

        let bytes = std::fs::read(&pdb_path).unwrap();
        let off = TEST_PAGE_SIZE;
        assert_eq!(
            u16::from_le_bytes(bytes[off + 0x20..off + 0x22].try_into().unwrap()),
            2
        );
        assert_eq!(
            u16::from_le_bytes(bytes[off + 0x22..off + 0x24].try_into().unwrap()),
            0
        );
        assert!(detect_pdb_wrong_track_u5(&pdb_path).is_empty());
    }

    #[test]
    fn t00_multipage_active_repair_flags_only_predecessor_pages() {
        // Two chained tt=0 pages, both ACTV (0x34). Page 1 -> Page 2 makes
        // page 1 a predecessor (must become SEAL); page 2 is the terminal
        // active page (next_page not in the data-page set) and stays ACTV.
        let mut p1 = data_page(1, 0);
        set_flags(&mut p1, 0x34);
        set_nrs(&mut p1, 4);
        set_next_page(&mut p1, 2);
        let mut p2 = data_page(2, 0);
        set_flags(&mut p2, 0x34);
        set_nrs(&mut p2, 4);
        set_next_page(&mut p2, 0); // terminal: doesn't point at another data page
        let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1, p2]);
        let pdb_path = vendor_pdb_path(&usb_root);

        let pages = detect_pdb_t00_multipage_active_pages(&pdb_path);
        assert_eq!(
            pages.len(),
            1,
            "only the predecessor page should be flagged"
        );
        assert_eq!(pages[0].page_index, 1);

        assert_eq!(
            apply_pdb_t00_multipage_active_repair(&usb_root, &[]).unwrap(),
            0
        );
        let patched = apply_pdb_t00_multipage_active_repair(&usb_root, &pages).unwrap();
        assert_eq!(patched, 1);

        let bytes = std::fs::read(&pdb_path).unwrap();
        let off = TEST_PAGE_SIZE; // page 1
        assert_eq!(bytes[off + 0x1b], 0x24, "predecessor page must become SEAL");
        assert_eq!(
            u16::from_le_bytes(bytes[off + 0x20..off + 0x22].try_into().unwrap()),
            1
        );
        assert_eq!(
            u16::from_le_bytes(bytes[off + 0x22..off + 0x24].try_into().unwrap()),
            3
        );
        // Terminal page 2 must be left untouched.
        let off2 = TEST_PAGE_SIZE * 2;
        assert_eq!(bytes[off2 + 0x1b], 0x34);
        assert!(detect_pdb_t00_multipage_active_pages(&pdb_path).is_empty());
    }

    #[test]
    fn wrong_playlist_tree_shape_repair_detects_and_fixes_footer() {
        // tt=7 (playlist_tree): correct convention is (nrs, 0).
        let mut p1 = data_page(1, 7);
        set_nrs(&mut p1, 6);
        set_u5_num_rl(&mut p1, 1, 1); // wrong: neither num_rl==0 nor u5==nrs
        let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
        let pdb_path = vendor_pdb_path(&usb_root);

        let pages = detect_pdb_wrong_playlist_tree_shape(&pdb_path);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].nrs, 6);

        assert_eq!(
            apply_pdb_wrong_playlist_tree_shape_repair(&usb_root, &[]).unwrap(),
            0
        );
        let patched = apply_pdb_wrong_playlist_tree_shape_repair(&usb_root, &pages).unwrap();
        assert_eq!(patched, 1);

        let bytes = std::fs::read(&pdb_path).unwrap();
        let off = TEST_PAGE_SIZE;
        assert_eq!(
            u16::from_le_bytes(bytes[off + 0x20..off + 0x22].try_into().unwrap()),
            6
        );
        assert_eq!(
            u16::from_le_bytes(bytes[off + 0x22..off + 0x24].try_into().unwrap()),
            0
        );
        assert!(detect_pdb_wrong_playlist_tree_shape(&pdb_path).is_empty());
    }

    #[test]
    fn tombstoned_playlist_tree_id_repair_zeroes_duplicate_dead_id() {
        // tt=7 (playlist_tree), id field at row byte 12 (TOMBSTONE_ID_TABLES).
        // Two row slots: row0 (active, id=42) and row1 (tombstoned, id=42 —
        // a stale duplicate left behind by an earlier repair).
        let mut p1 = data_page(1, 7);
        p1[0x18] = 2; // packed row count low byte: num_row_offsets = 2
        // rowpf group header at the tail: cursor -= 4 for (rowpf, tranrf).
        let rowpf_off = TEST_PAGE_SIZE - 4;
        // bit0 (row index 0, first row) = active; bit1 (row index 1, last row) = tombstoned.
        p1[rowpf_off..rowpf_off + 2].copy_from_slice(&0b01u16.to_le_bytes());
        // Row-offset table (2 entries, u16 each), stored in reversed row order:
        // position0 -> LAST row (index1), position1 -> FIRST row (index0).
        let offsets_off = rowpf_off - 4;
        p1[offsets_off..offsets_off + 2].copy_from_slice(&0x20u16.to_le_bytes()); // row1 heap offset
        p1[offsets_off + 2..offsets_off + 4].copy_from_slice(&0x00u16.to_le_bytes()); // row0 heap offset
        // Row bytes on the heap (heap_start = 0x28): id field at row_byte_off + 12.
        let heap_start = 0x28usize;
        p1[heap_start + 12..heap_start + 16].copy_from_slice(&42u32.to_le_bytes()); // row0 id (active)
        p1[heap_start + 0x20 + 12..heap_start + 0x20 + 16].copy_from_slice(&42u32.to_le_bytes()); // row1 id (tombstoned dup)
        let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), p1]);
        let pdb_path = vendor_pdb_path(&usb_root);

        let items = detect_pdb_tombstoned_playlist_tree_ids(&pdb_path);
        assert_eq!(
            items.len(),
            1,
            "expected the duplicate tombstoned id to be flagged"
        );
        assert_eq!(items[0].page_index, 1);
        assert_eq!(items[0].id_field_offset, heap_start + 0x20 + 12);

        assert_eq!(
            apply_pdb_tombstoned_playlist_tree_id_repair(&usb_root, &[]).unwrap(),
            0
        );
        let patched = apply_pdb_tombstoned_playlist_tree_id_repair(&usb_root, &items).unwrap();
        assert_eq!(patched, 1);

        let bytes = std::fs::read(&pdb_path).unwrap();
        let id_off = TEST_PAGE_SIZE + heap_start + 0x20 + 12;
        assert_eq!(
            u32::from_le_bytes(bytes[id_off..id_off + 4].try_into().unwrap()),
            0
        );
        assert!(detect_pdb_tombstoned_playlist_tree_ids(&pdb_path).is_empty());
    }

    #[test]
    fn stale_sentinel_btree_repair_rebuilds_missing_index() {
        // One active tt=9 data page plus a sentinel (flags=0x64) page for the
        // same table with an empty (ne=0) B-tree despite the active page
        // existing — the "missing_entries" stale condition.
        let mut data = data_page(1, 9);
        set_flags(&mut data, 0x34);
        let sentinel = data_page(2, 9); // flags default 0 below, override to 0x64
        let mut sentinel = sentinel;
        set_flags(&mut sentinel, 0x64);
        // ne (num_entries) @ +0x38 and u7 (write pointer) @ +0x26 both default 0.
        let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header_page(), data, sentinel]);
        let pdb_path = vendor_pdb_path(&usb_root);

        let pages = detect_pdb_stale_sentinel_btree(&pdb_path);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].page_index, 2);

        assert_eq!(
            apply_pdb_stale_sentinel_btree_repair(&usb_root, &[]).unwrap(),
            0
        );
        let patched = apply_pdb_stale_sentinel_btree_repair(&usb_root, &pages).unwrap();
        assert_eq!(patched, 1);

        let bytes = std::fs::read(&pdb_path).unwrap();
        let soff = TEST_PAGE_SIZE * 2;
        assert_eq!(
            u16::from_le_bytes(bytes[soff + 0x38..soff + 0x3a].try_into().unwrap()),
            1
        );
        assert_eq!(
            u32::from_le_bytes(bytes[soff + 0x3c..soff + 0x40].try_into().unwrap()),
            8, // page_index(1) * 8
        );
        assert!(detect_pdb_stale_sentinel_btree(&pdb_path).is_empty());
    }

    #[test]
    fn ec_data_page_conflict_repair_relocates_empty_candidate() {
        // Table B's empty_candidate (ec) points at page 1, which is actually
        // owned by table A — a stale pointer left by an old additive writer.
        let mut header = header_page();
        header[8..12].copy_from_slice(&1u32.to_le_bytes()); // num_tables = 1
        let toff = 0x1c;
        header[toff..toff + 4].copy_from_slice(&9u32.to_le_bytes()); // tt = 9 (table B)
        header[toff + 4..toff + 8].copy_from_slice(&1u32.to_le_bytes()); // ec = page 1 (conflict)
        header[toff + 8..toff + 12].copy_from_slice(&10u32.to_le_bytes()); // fp
        header[toff + 12..toff + 16].copy_from_slice(&1u32.to_le_bytes()); // lp = page 1 (fp != lp)
        let mut owned_by_a = data_page(1, 5); // table A's real data page
        set_flags(&mut owned_by_a, 0x34);
        let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header, owned_by_a]);
        let pdb_path = vendor_pdb_path(&usb_root);

        let conflicts = detect_pdb_ec_data_page_conflicts(&pdb_path);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].table_type, 9);
        assert_eq!(conflicts[0].last_page, 1);

        assert_eq!(
            apply_pdb_ec_data_page_conflict_repair(&usb_root, &[]).unwrap(),
            0
        );
        let patched = apply_pdb_ec_data_page_conflict_repair(&usb_root, &conflicts).unwrap();
        assert_eq!(patched, 1);

        let bytes = std::fs::read(&pdb_path).unwrap();
        let new_ec = u32::from_le_bytes(bytes[toff + 4..toff + 8].try_into().unwrap());
        assert_eq!(
            new_ec, 2,
            "ec should be relocated past the physical end of the file"
        );
        // The (former) conflicting page's next_page must be repointed too.
        let p1_next = u32::from_le_bytes(
            bytes[TEST_PAGE_SIZE + 0x0c..TEST_PAGE_SIZE + 0x10]
                .try_into()
                .unwrap(),
        );
        assert_eq!(p1_next, new_ec);
        assert!(detect_pdb_ec_data_page_conflicts(&pdb_path).is_empty());
    }

    #[test]
    fn truncated_table_chain_repair_restores_last_and_ec_to_physical_reality() {
        // Mirrors an additive-growth write interrupted after the table
        // pointer was advanced to claim page 3 as the new last page, but
        // before page 3 was ever flushed to disk: the real last page (2)
        // already has its `next` field pointing at 3, anticipating the
        // growth that never landed.
        let mut header = header_page();
        header[8..12].copy_from_slice(&1u32.to_le_bytes()); // num_tables = 1
        let toff = 0x1c;
        header[toff..toff + 4].copy_from_slice(&9u32.to_le_bytes()); // tt = 9
        header[toff + 4..toff + 8].copy_from_slice(&4u32.to_le_bytes()); // ec = 4 (nonexistent)
        header[toff + 8..toff + 12].copy_from_slice(&1u32.to_le_bytes()); // fp = 1
        header[toff + 12..toff + 16].copy_from_slice(&3u32.to_le_bytes()); // lp = 3 (nonexistent)

        let mut p1 = data_page(1, 9);
        set_flags(&mut p1, 0x34);
        set_next_page(&mut p1, 2);
        let mut p2 = data_page(2, 9); // real last page; already points at the growth that never landed
        set_flags(&mut p2, 0x34);
        set_next_page(&mut p2, 3);

        let (_td, usb_root) = write_pdb_pages_as_usb_root(vec![header, p1, p2]);
        let pdb_path = vendor_pdb_path(&usb_root);

        let fixes = detect_pdb_truncated_table_chains(&pdb_path);
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].table_type, 9);
        assert_eq!(fixes[0].corrected_last, 2);
        assert_eq!(fixes[0].corrected_ec, 3);

        assert_eq!(
            apply_pdb_truncated_table_chain_repair(&usb_root, &[]).unwrap(),
            0
        );
        let patched = apply_pdb_truncated_table_chain_repair(&usb_root, &fixes).unwrap();
        assert_eq!(patched, 1);

        let bytes = std::fs::read(&pdb_path).unwrap();
        let new_ec = u32::from_le_bytes(bytes[toff + 4..toff + 8].try_into().unwrap());
        let new_last = u32::from_le_bytes(bytes[toff + 12..toff + 16].try_into().unwrap());
        assert_eq!(new_ec, 3);
        assert_eq!(new_last, 2);
        // Page 2's own content is untouched — its `next` already matched.
        let p2_next = u32::from_le_bytes(
            bytes[2 * TEST_PAGE_SIZE + 0x0c..2 * TEST_PAGE_SIZE + 0x10]
                .try_into()
                .unwrap(),
        );
        assert_eq!(p2_next, 3);
        assert!(
            crate::utils::collect_chain(&bytes, TEST_PAGE_SIZE, 1, 2).is_some(),
            "chain must now walk cleanly to the corrected last page"
        );
        assert!(detect_pdb_truncated_table_chains(&pdb_path).is_empty());
    }

    #[test]
    fn torn_growth_repair_zeroes_garbage_ec_page_and_truncates_tail() {
        // Mirrors a USB disconnected mid-additive-growth-write: table tt=9's
        // empty_candidate (page 2) was earmarked as the next blank slot but
        // never got zeroed/written, and the file was already extended past
        // next_unused_page (3) with an unpopulated, non-zero tail (page 3).
        let mut header = header_page();
        header[8..12].copy_from_slice(&1u32.to_le_bytes()); // num_tables = 1
        header[0x0c..0x10].copy_from_slice(&3u32.to_le_bytes()); // next_unused_page = 3
        let toff = 0x1c;
        header[toff..toff + 4].copy_from_slice(&9u32.to_le_bytes()); // tt = 9
        header[toff + 4..toff + 8].copy_from_slice(&2u32.to_le_bytes()); // ec = page 2 (garbage)
        header[toff + 8..toff + 12].copy_from_slice(&1u32.to_le_bytes()); // fp = 1
        header[toff + 12..toff + 16].copy_from_slice(&1u32.to_le_bytes()); // lp = 1

        let mut p1 = data_page(1, 9); // table tt=9's only real data page
        set_flags(&mut p1, 0x34);
        let garbage_ec = vec![0xABu8; TEST_PAGE_SIZE]; // page 2: torn ec target
        let garbage_tail = vec![0xCDu8; TEST_PAGE_SIZE]; // page 3: never-populated tail

        let (_td, usb_root) =
            write_pdb_pages_as_usb_root(vec![header, p1, garbage_ec, garbage_tail]);
        let pdb_path = vendor_pdb_path(&usb_root);

        let report = detect_pdb_torn_growth_pages(&pdb_path);
        assert_eq!(report.garbage_ec_pages, vec![2]);
        assert_eq!(report.truncate_to_pages, Some(3));

        assert_eq!(
            apply_pdb_torn_growth_pages_repair(&usb_root, &PdbTornGrowthReport::default()).unwrap(),
            0
        );
        let patched = apply_pdb_torn_growth_pages_repair(&usb_root, &report).unwrap();
        assert_eq!(patched, 1, "one garbage ec page zeroed");

        let bytes = std::fs::read(&pdb_path).unwrap();
        assert_eq!(
            bytes.len(),
            3 * TEST_PAGE_SIZE,
            "never-populated tail must be truncated away"
        );
        let ec_off = 2 * TEST_PAGE_SIZE;
        assert!(
            bytes[ec_off..ec_off + TEST_PAGE_SIZE]
                .iter()
                .all(|b| *b == 0),
            "the reclaimed ec page must be all-zero"
        );
        let seqdb = u32::from_le_bytes(bytes[0x14..0x18].try_into().unwrap());
        let max_seq = crate::pdb_writer::max_seqpage_in_file(&bytes, TEST_PAGE_SIZE);
        assert!(
            seqdb > max_seq,
            "seqdb must stay strictly greater than max seqpage"
        );
        assert!(detect_pdb_torn_growth_pages(&pdb_path).is_empty());

        // The reclaimed ec page must now satisfy the additive writer's own
        // "all zero" reuse precondition (pdb_writer.rs's
        // append_rows_to_chain_in_place), proving a follow-up rekordbox
        // export would succeed instead of hitting the same rejection again.
        let page = &bytes[ec_off..ec_off + TEST_PAGE_SIZE];
        assert!(page.iter().all(|b| *b == 0));
    }

    #[test]
    fn apply_fix_sync_edb_history_from_pdb_overwrites_stale_edb_rows() {
        let (_td, usb_root) = test_usb_root();
        let service_data_dir = tempdir().expect("service data dir");
        let service = BackendService::new(service_data_dir.path()).expect("backend service");

        // A named history playlist with one valid (non-zero track_id) entry
        // and one invalid entry (track_id=0, must be dropped) — mirrors
        // derive_history_sync_payload's own filtering rules.
        let parsed = ParsedPdb {
            history_playlists: vec![PdbHistoryPlaylistRow {
                id: 10,
                name: "HISTORY 001".to_string(),
                source_table: 17,
            }],
            history_entries: vec![
                PdbHistoryEntryRow {
                    track_id: Some(101),
                    playlist_id: 10,
                    entry_index: 1,
                    source_table: 18,
                },
                PdbHistoryEntryRow {
                    track_id: Some(0),
                    playlist_id: 10,
                    entry_index: 2,
                    source_table: 18,
                },
            ],
            ..ParsedPdb::default()
        };

        // Seed the eDB with stale history rows that must be replaced.
        {
            let mut warnings = Vec::new();
            let mut conn = open_edb_rw(&usb_root, &mut warnings).expect("open eDB rw");
            let tx = conn.transaction().expect("begin tx");
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS history(history_id integer primary key, sequenceNo integer, name varchar, attribute integer, history_id_parent integer); \
                 CREATE TABLE IF NOT EXISTS history_content(history_id integer, content_id integer, sequenceNo integer); \
                 DELETE FROM history; DELETE FROM history_content; \
                 INSERT INTO history(history_id, sequenceNo, name, attribute, history_id_parent) VALUES (999, 5, 'Stale', 0, 0); \
                 INSERT INTO history_content(history_id, content_id, sequenceNo) VALUES (999, 555, 1), (999, 556, 2);",
            )
            .expect("seed stale history rows");
            tx.commit().expect("commit seed");
        }

        let mut warnings = Vec::new();
        let (history_written, history_content_written) = service
            .apply_fix_sync_edb_history_from_pdb(&usb_root, &parsed, None, &mut warnings)
            .expect("sync edb history");
        assert_eq!(history_written, 1);
        assert_eq!(
            history_content_written, 1,
            "the track_id=0 entry must be dropped"
        );

        let conn = open_edb_rw(&usb_root, &mut Vec::new()).expect("re-open eDB rw");
        let (hid, name): (i64, String) = conn
            .query_row("SELECT history_id, name FROM history", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .expect("history row");
        assert_eq!(hid, 10);
        assert_eq!(name, "HISTORY 001");
        let (content_hid, content_id): (i64, i64) = conn
            .query_row(
                "SELECT history_id, content_id FROM history_content",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("history_content row");
        assert_eq!(content_hid, 10);
        assert_eq!(content_id, 101);
    }
}
