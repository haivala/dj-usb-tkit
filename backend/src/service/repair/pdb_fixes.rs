//! PDB page-level repairs: one detector and one apply function per fix.

use super::*;

#[derive(Debug, Clone)]
pub(super) struct PdbHeaderCompatibilityRepair {
    pub(super) current_value: u32,
    pub(super) target_value: u32,
}

pub(super) fn pdb_header_compatibility_value_from_bytes(bytes: &[u8]) -> Option<u32> {
    bytes
        .get(0x10..0x14)
        .and_then(|raw| raw.try_into().ok())
        .map(u32::from_le_bytes)
}

pub(super) fn read_pdb_header_compatibility_value(path: &Path) -> Option<u32> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| pdb_header_compatibility_value_from_bytes(&bytes))
}

pub(super) fn is_known_pdb_header_compatibility_value(value: u32) -> bool {
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
pub(super) fn detect_pdb_header_compatibility_repair(
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

pub(super) fn apply_pdb_header_compatibility_repair(
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

/// True when the PDB holds tracks but its tt=19 page does not match them
/// and the eDB's `property` row: still the untouched init template (older
/// players count 0 songs), or a ring whose newest id is not the track count
/// or whose name, color or version differs (see
/// [`crate::pdb_writer::t19_sync_target`]).
pub(in crate::service) fn detect_pdb_t19_template_state(
    pdb_path: &Path,
    track_count: usize,
    property: Option<&crate::pdb_writer::T19Property>,
) -> bool {
    let Ok(bytes) = std::fs::read(pdb_path) else {
        return false;
    };
    let page_size = crate::utils::read_u32_le_at(&bytes, 4).unwrap_or(0) as usize;
    crate::pdb_writer::t19_sync_target(&bytes, page_size, track_count, property).is_some()
}

/// Rewrite tt=19 as the ring rekordbox keeps for the PDB's track count and
/// the eDB's `property` row. Re-checks the current file, so it is a no-op
/// (`Ok(false)`) once the page matches. Touches only that page, its
/// sentinel and the header's seqdb.
pub(in crate::service) fn apply_pdb_t19_template_state_repair(
    usb_root: &Path,
) -> BackendResult<bool> {
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let track_count = crate::pdb_reader::parse_pdb_bytes(&bytes)?.tracks.len();
    let page_size = crate::utils::read_u32_le_at(&bytes, 4).unwrap_or(0) as usize;
    let property = crate::service::export_helpers::edb_t19_property(usb_root);
    let Some((track_count, property)) =
        crate::pdb_writer::t19_sync_target(&bytes, page_size, track_count, property.as_ref())
    else {
        return Ok(false);
    };
    crate::pdb_writer::sync_t19_ring_page(&mut bytes, page_size, track_count, &property)?;
    std::fs::write(&pdb_path, &bytes)?;
    Ok(true)
}

/// A data page whose `u5` field at offset 0x20 carries the sentinel value
/// 0x1FFF.  This makes player firmware treat the page as an empty fence
/// page and report the database as corrupted.
#[derive(Debug, Clone)]
pub(in crate::service) struct SentinelU5Page {
    pub(super) page_index: usize,
    current_num_rl: u16,
    correct_u5: u16,
    correct_num_rl: u16,
}

pub(in crate::service) fn detect_pdb_sentinel_u5_on_data_pages(
    pdb_path: &Path,
) -> Vec<SentinelU5Page> {
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

pub(super) fn apply_pdb_sentinel_u5_repair(
    usb_root: &Path,
    pages: &[SentinelU5Page],
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

/// Standalone counterpart to the duplicate-entry sweep bundled into
/// `apply_strict_parity_upgrade` (see its "Phase 3b"). Safe to run on its
/// own: `remove_duplicate_playlist_entries_inplace` is a pure function over
/// the raw PDB byte image with no dependency on eDB or merged-playlist
/// state, and it's idempotent (a second call on an already-clean PDB
/// returns 0 and leaves the bytes untouched).
pub(super) fn apply_pdb_duplicate_playlist_entries_repair(usb_root: &Path) -> BackendResult<usize> {
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let removed = crate::pdb_writer::remove_duplicate_playlist_entries_inplace(&mut bytes);
    if removed > 0 {
        std::fs::write(&pdb_path, &bytes)?;
    }
    Ok(removed)
}

pub(super) fn expected_page_flags(table_type: u32) -> u8 {
    // Both 0x24 (sealed) and 0x34 (active) are valid for tt=0/19.
    // This function is kept for backward-compat callers that only need
    // the "default/active" value; detection logic uses is_valid_page_flags.
    if matches!(table_type, 0 | 19) {
        0x34
    } else {
        0x24
    }
}

pub(super) fn is_valid_page_flags(table_type: u32, flags: u8) -> bool {
    match table_type {
        0 | 7 | 19 => flags == 0x24 || flags == 0x34, // sealed or active both valid
        _ => flags == 0x24,
    }
}

#[derive(Debug, Clone)]
pub(in crate::service) struct WrongFlagsPage {
    pub(in crate::service) page_index: usize,
    pub(in crate::service) correct_flags: u8,
}

pub(in crate::service) fn detect_pdb_wrong_page_flags(pdb_path: &Path) -> Vec<WrongFlagsPage> {
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

pub(super) fn apply_pdb_wrong_page_flags_repair(
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
pub(in crate::service) struct ZeroTranrfPage {
    pub(in crate::service) page_index: usize,
    pub(in crate::service) u5: u16,
    pub(in crate::service) num_rl: usize,
    /// rowpf value for each group (to copy into tranrf for groups that need fixing)
    pub(in crate::service) rowpf_groups: Vec<u16>,
    /// existing tranrf value for each group (0x0000 in groups that need fixing)
    pub(in crate::service) tranrf_groups: Vec<u16>,
    /// byte offset within the page of each group's 4-byte header
    pub(in crate::service) footer_group_offsets: Vec<usize>,
}

/// A tt=16/17/18 data page whose u5/num_rl footer fields carry the old wrong
/// `(1, nrs-1)` pattern instead of the correct `(nrs, 0)` convention.  DJ software
/// and player firmware may reject or misparse these pages.
#[derive(Debug, Clone)]
pub(in crate::service) struct WrongShapeHistoryPage {
    pub(in crate::service) page_index: usize,
    #[allow(dead_code)]
    pub(in crate::service) table_type: u32,
    pub(in crate::service) nrs: u16,
}

/// Detect tt=16/17/18 data pages carrying the wrong `(1, nrs-1)` footer shape.
/// The correct convention for these tables is `(nrs, 0)`.
pub(in crate::service) fn detect_pdb_wrong_history_page_shape(
    pdb_path: &Path,
) -> Vec<WrongShapeHistoryPage> {
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

pub(super) fn apply_pdb_wrong_history_page_shape_repair(
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
pub(in crate::service) struct WrongTrackU5Page {
    pub(in crate::service) page_index: usize,
}

pub(in crate::service) fn detect_pdb_wrong_track_u5(pdb_path: &Path) -> Vec<WrongTrackU5Page> {
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

/// A track/album/artist row whose long-form (UTF-16 `0x90` or long ASCII `0x40`) string slot
/// starts at a row-relative offset that isn't 4-byte aligned.
#[derive(Debug, Clone)]
pub(in crate::service) struct MisalignedStringSlot {
    /// 0 = tracks (t00), 2 = artists (t02), 3 = albums (t03).
    pub(in crate::service) table_type: u32,
    pub(in crate::service) id: u32,
    pub(in crate::service) slot_label: &'static str,
}

/// Mirrors `pdb_reader::track_string_slot_label` (private to that module — duplicated here as a
/// small const lookup rather than widening its visibility just for a label string).
pub(super) const TRACK_STRING_SLOT_LABELS: [&str; 21] = [
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
/// Artist near-variant rows are also flagged when a short ASCII name isn't at offset 10, where
/// rekordbox puts it: Engine DJ's `export.pdb` import reads it only there and leaves the artist
/// empty otherwise. This app's writer used to put every artist name at 12.
pub(in crate::service) fn detect_pdb_string_misalignment(
    pdb_path: &Path,
) -> Vec<MisalignedStringSlot> {
    std::fs::read(pdb_path)
        .map(|bytes| detect_string_misalignment_in_bytes(&bytes))
        .unwrap_or_default()
}

/// [`detect_pdb_string_misalignment`] on PDB bytes in memory.
pub(in crate::service) fn detect_string_misalignment_in_bytes(
    bytes: &[u8],
) -> Vec<MisalignedStringSlot> {
    use crate::pdb_writer::parse_page_row_slots;
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};
    let Some(page_size) = read_u32_le_at(bytes, 4).map(|v| v as usize) else {
        return Vec::new();
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return Vec::new();
    }
    let total = bytes.len() / page_size;
    let mut out = Vec::new();
    for i in 1..total {
        let off = i * page_size;
        let Some(idx) = read_u32_le_at(bytes, off + 4) else {
            continue;
        };
        if idx == 0 {
            continue;
        }
        let table_type = read_u32_le_at(bytes, off + 8).unwrap_or(9999);
        if !matches!(table_type, 0 | 2 | 3) {
            continue;
        }
        let used_s = read_u16_le_at(bytes, off + 30).unwrap_or(0) as usize;
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
                        if matches!(row.get(slot_offset), Some(0x40 | 0x90)) && slot_offset % 4 != 0
                        {
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
                    if matches!(row.get(name_offset), Some(0x40 | 0x90))
                        && !name_offset.is_multiple_of(4)
                    {
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
                    if matches!(row.get(name_offset), Some(0x40 | 0x90)) && name_offset % 4 != 0 {
                        out.push(MisalignedStringSlot {
                            table_type,
                            id,
                            slot_label: "name",
                        });
                    } else if typ == 0x60
                        && row.get(name_offset).is_some_and(|b| b & 1 == 1)
                        && name_offset != ARTIST_SHORT_ASCII_NAME_OFFSET
                    {
                        out.push(MisalignedStringSlot {
                            table_type,
                            id,
                            slot_label: "ascii_name_offset",
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
pub(in crate::service) struct T00MultipageActivePage {
    pub(in crate::service) page_index: usize,
    pub(in crate::service) nrs: u8,
}

/// Detect ACTV (0x34) tt=0 pages that must be SEAL (0x24) in a multi-page chain.
/// The last (active write) page in the chain legitimately stays ACTV; only
/// predecessor pages that are still marked ACTV are flagged.
pub(in crate::service) fn detect_pdb_t00_multipage_active_pages(
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

pub(super) fn apply_pdb_t00_multipage_active_repair(
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
pub(in crate::service) struct EcDataPageConflict {
    /// The table whose empty_candidate is wrong.
    pub(in crate::service) table_type: u32,
    /// Last data page of `table_type` (its `next_page` also needs fixing).
    pub(in crate::service) last_page: u32,
}

/// Detect tables whose `empty_candidate` points to a page already used as
/// data by a different table.
pub(in crate::service) fn detect_pdb_ec_data_page_conflicts(
    pdb_path: &Path,
) -> Vec<EcDataPageConflict> {
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

pub(super) fn apply_pdb_ec_data_page_conflict_repair(
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
/// growth), a table's `last` page must always physically exist: it is not a
/// future candidate, it is the table's real current tail.
#[derive(Debug, Clone)]
pub(in crate::service) struct TruncatedTableChain {
    pub(in crate::service) table_type: u32,
    /// The real last page, found by walking the chain from `first` and
    /// stopping at the last page that physically exists and holds live data.
    pub(in crate::service) corrected_last: u32,
    /// The corrected `empty_candidate`, taken from the real last page's own
    /// `next` field (the documented `empty_candidate == last_page.next`
    /// invariant, verified against reference exports) when that value looks
    /// like a sane forward pointer, else the physical end of the file.
    pub(in crate::service) corrected_ec: u32,
}

/// Detect tables whose declared `last` page index does not physically exist
/// in the file, and compute the metadata correction that restores
/// `empty_candidate == last_page.next` without touching any page content.
pub(in crate::service) fn detect_pdb_truncated_table_chains(
    pdb_path: &Path,
) -> Vec<TruncatedTableChain> {
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
        // Walk from `first` following `next` pointers, anchoring `real_last`
        // on the last page that both physically exists AND holds live rows
        // (`used_size > 0`). An interrupted additive-growth write can link an
        // all-zero reserve page into the chain tail before advancing the
        // header's `last`/`empty_candidate` pointers even further ahead;
        // anchoring only on in-bounds pages would land on that blank page and
        // defeat the repair. Bounded by total_pages so a cycle can't spin
        // forever; the traversal itself never skips a page, so a blank page
        // followed by more data still resolves to the last data page.
        let mut seen = HashSet::<u32>::new();
        let mut real_last = None;
        let mut cur = first;
        for _ in 0..=total_pages {
            if cur >= total_pages || !seen.insert(cur) {
                break;
            }
            let Some(off) = page_offset(cur, page_size) else {
                break;
            };
            let has_live_rows = bytes
                .get(off..off + page_size)
                .and_then(|page| read_u16_le_at(page, 0x1e))
                .is_some_and(|used_s| used_s > 0);
            if has_live_rows {
                real_last = Some(cur);
            }
            if cur == last {
                break;
            }
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
            // Unreachable now that the walk only anchors on live pages; kept
            // as a defensive guard against ever anchoring on a blank page.
            continue;
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
pub(super) fn apply_pdb_truncated_table_chain_repair(
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
pub(in crate::service) struct PdbTornGrowthReport {
    /// Table `empty_candidate` pages that sit within a page no other table
    /// owns, but aren't the all-zero blank page the additive writer expects
    /// before it will reuse them (`pdb_writer`'s `append_rows_to_chain_in_place`).
    pub(in crate::service) garbage_ec_pages: Vec<u32>,
    /// When set, the file should be truncated to this many pages: the header's
    /// own `next_unused_page` claims nothing beyond it is allocated, yet the
    /// physical file extends further with non-zero content.
    pub(in crate::service) truncate_to_pages: Option<u32>,
}

impl PdbTornGrowthReport {
    pub(in crate::service) fn is_empty(&self) -> bool {
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
pub(in crate::service) fn detect_pdb_torn_growth_pages(pdb_path: &Path) -> PdbTornGrowthReport {
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
pub(super) fn apply_pdb_torn_growth_pages_repair(
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
pub(crate) fn track_row_data_from_reader_row(
    row: &crate::pdb_reader::PdbTrackRow,
) -> PdbTrackRowData {
    PdbTrackRowData {
        composer_id: row.composer_id,
        label_id: row.label_id,
        remixer_id: row.remixer_id,
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
        color_id: Some(row.color_id),
        rating: Some(row.rating),
        title: row.title.clone(),
        anlz_path: row.anlz_path.clone(),
        file_path: row.track_file_path.clone(),
    }
}

/// Fix track rows whose title/filename/other UTF-16 string slot isn't 4-byte aligned (see
/// `detect_pdb_string_misalignment`). Re-encodes each row's content unchanged through the
/// current (already alignment-fixed) `encode_track_row_with_profile` and relocates it via
/// `mutate_tracks_in_place`, which now refuses to same-size-patch an already-misaligned row (see
/// `track_row_has_misaligned_utf16_slot` in pdb_writer) and instead always takes the
/// relocate/append path.
pub(super) fn apply_pdb_track_string_alignment_repair(
    usb_root: &Path,
    ids: &[u32],
) -> BackendResult<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let patched = realign_track_rows_in_bytes(&mut bytes, ids)?;
    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

/// [`apply_pdb_track_string_alignment_repair`] on PDB bytes in memory.
fn realign_track_rows_in_bytes(bytes: &mut Vec<u8>, ids: &[u32]) -> BackendResult<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let Some(page_size) = bytes
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
    else {
        return Err(BackendError::Validation(
            "PDB too small to read page size".into(),
        ));
    };
    let parsed = crate::pdb_reader::parse_pdb_bytes(bytes)?;
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
        bytes,
        &mutations,
        PdbLayoutProfile::DEFAULT,
        page_size,
    )?;
    let mismatches = crate::pdb_reader::validate_pdb_page_conventions(bytes);
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
    Ok(patched)
}

/// Fix album rows whose UTF-16 name slot isn't 4-byte aligned (see
/// `detect_pdb_string_misalignment`). Reads each affected row's existing `artist_id`/name
/// directly off the raw bytes (nothing semantic changes, only the encoding) and re-encodes it via
/// `encode_album_row` (already alignment-fixed).
pub(super) fn apply_pdb_album_string_alignment_repair(
    usb_root: &Path,
    ids: &[u32],
) -> BackendResult<usize> {
    reencode_pdb_dictionary_rows(
        usb_root,
        ids,
        3,
        12,
        "Album string alignment",
        reencode_album_row,
    )
}

/// Album row `id` re-encoded by the current writer, artist and name unchanged.
fn reencode_album_row(row: &[u8], id: u32) -> Option<Vec<u8>> {
    use crate::utils::{read_u8_at, read_u32_le_at};
    if row.len() < 16 {
        return None;
    }
    let artist_id = read_u32_le_at(row, 8)?;
    let name_offset = read_u8_at(row, 21).unwrap_or(22) as usize;
    let name = crate::pdb_reader::get_string_from_pdb(row, name_offset).unwrap_or_default();
    Some(encode_album_row(id, &name, artist_id))
}

/// Fix artist rows whose name isn't where rekordbox puts it (see
/// `detect_pdb_string_misalignment`): re-encodes each via `encode_artist_row`, name unchanged.
pub(super) fn apply_pdb_artist_name_layout_repair(
    usb_root: &Path,
    ids: &[u32],
) -> BackendResult<usize> {
    reencode_pdb_dictionary_rows(
        usb_root,
        ids,
        2,
        4,
        "Artist name layout",
        reencode_artist_row,
    )
}

/// Artist row `id` re-encoded by the current writer, name unchanged.
fn reencode_artist_row(row: &[u8], id: u32) -> Option<Vec<u8>> {
    use crate::utils::{read_u8_at, read_u16_le_at};
    if row.len() < 10 {
        return None;
    }
    let name_offset = if read_u16_le_at(row, 0)? == 0x64 {
        read_u16_le_at(row, 10)? as usize
    } else {
        read_u8_at(row, 9)? as usize
    };
    let name = crate::pdb_reader::get_string_from_pdb(row, name_offset)?;
    Some(encode_artist_row(id, &name))
}

/// Re-encode the dictionary rows (`table_type` 2 or 3, id at `id_offset`) named by `ids` and
/// relocate them via `realign_dictionary_rows_in_place`. `encode` builds the replacement from the
/// existing row's bytes.
pub(super) fn reencode_pdb_dictionary_rows(
    usb_root: &Path,
    ids: &[u32],
    table_type: u32,
    id_offset: usize,
    label: &str,
    encode: impl Fn(&[u8], u32) -> Option<Vec<u8>>,
) -> BackendResult<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let pdb_path = usb_staging::stage_pdb(usb_root)?;
    let mut bytes = std::fs::read(&pdb_path)?;
    let patched =
        reencode_dictionary_rows_in_bytes(&mut bytes, ids, table_type, id_offset, label, encode)?;
    std::fs::write(&pdb_path, &bytes)?;
    Ok(patched)
}

/// [`reencode_pdb_dictionary_rows`] on PDB bytes in memory.
fn reencode_dictionary_rows_in_bytes(
    bytes: &mut Vec<u8>,
    ids: &[u32],
    table_type: u32,
    id_offset: usize,
    label: &str,
    encode: impl Fn(&[u8], u32) -> Option<Vec<u8>>,
) -> BackendResult<usize> {
    use crate::utils::{page_offset, read_u32_le_at};
    if ids.is_empty() {
        return Ok(0);
    }
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
    let (_ec, first, last) =
        crate::utils::table_ptr_fields(bytes, table_type).ok_or_else(|| {
            BackendError::Validation(format!(
                "{label} repair blocked: t{table_type:02} pointer missing"
            ))
        })?;
    crate::utils::collect_chain(bytes, page_size, first, last).ok_or_else(|| {
        BackendError::Validation(format!(
            "{label} repair blocked: t{table_type:02} chain invalid"
        ))
    })?;

    // Scan every physical page stamped with this table type, not just pages reachable by walking
    // the first/last chain pointers -- matching `detect_pdb_string_misalignment`'s scope exactly.
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
        if read_u32_le_at(page, 8) != Some(table_type) {
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
            let Some(id) = read_u32_le_at(row, id_offset) else {
                continue;
            };
            if !target_ids.contains(&id) {
                continue;
            }
            if let Some(replacement) = encode(row, id) {
                rows.push((id, replacement));
            }
        }
    }
    if rows.is_empty() {
        return Ok(0);
    }
    let patched = crate::pdb_writer::realign_dictionary_rows_in_place(
        bytes, table_type, id_offset, &rows, page_size,
    )?;
    let mismatches = crate::pdb_reader::validate_pdb_page_conventions(bytes);
    if !mismatches.is_empty() {
        let detail = mismatches
            .iter()
            .take(8)
            .map(|m| m.to_string())
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(BackendError::Validation(format!(
            "{label} repair blocked: page-header convention mismatches ({detail})"
        )));
    }
    Ok(patched)
}

/// Normalizes flagged pages to the current writer's own convention
/// (`data_page_footer_fields(0, trc)`, i.e. `(1, nrs-1)`), not the older
/// `(2, 0)` shape `detect_pdb_wrong_track_u5` still tolerates as valid on
/// pages that already have it.
pub(super) fn apply_pdb_wrong_track_u5_repair(
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
        let trc =
            crate::utils::packed_page_row_slot_count(&bytes[off..off + page_size]).unwrap_or(0);
        let (u5, num_rl) = crate::pdb_writer::data_page_footer_fields(0, trc as u16);
        bytes[off + 0x20..off + 0x22].copy_from_slice(&u5.to_le_bytes());
        bytes[off + 0x22..off + 0x24].copy_from_slice(&num_rl.to_le_bytes());
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
pub(in crate::service) struct TombstonedRowId {
    pub(in crate::service) page_index: usize,
    /// byte offset within the page of the 4-byte id field in the tombstoned row
    pub(in crate::service) id_field_offset: usize,
}

// (table_type, id_field_offset_in_row) pairs for tables where the DJ software
// validates uniqueness including tombstoned slots.
// tt=0 (tracks): id at row byte 72
// tt=7 (playlist_tree): id at row byte 12
pub(super) const TOMBSTONE_ID_TABLES: &[(u32, usize)] = &[(0, 72), (7, 12)];

/// Detect tombstoned rows in tt=0 (tracks) and tt=7 (playlist_tree) whose id
/// field duplicates an active row — indicating a stale slot left by a repair
/// operation that wrote the row to a new position without zeroing the old one.
///
/// Non-zero IDs in tombstones that have no corresponding active row are normal
/// deletions (the DJ software leaves IDs in place when deleting rows) and are
/// not flagged.
pub(in crate::service) fn detect_pdb_tombstoned_playlist_tree_ids(
    pdb_path: &Path,
) -> Vec<TombstonedRowId> {
    use crate::utils::{read_u16_le_at, read_u32_le_at};
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

        let page = &bytes[off..off + page_size];
        let num_row_offsets = crate::utils::packed_page_row_slot_count(page).unwrap_or(0);
        if num_row_offsets == 0 || !pdb_page::footer_fits(page_size, num_row_offsets) {
            continue;
        }

        for slot in pdb_page::footer_slots(page, num_row_offsets) {
            let is_active = slot.present;
            let row_byte_off = off + pdb_page::HEAP_START + slot.heap_off;
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
pub(in crate::service) struct WrongPlaylistTreeShapePage {
    pub(in crate::service) page_index: usize,
    pub(in crate::service) nrs: u8,
}

pub(in crate::service) fn detect_pdb_wrong_playlist_tree_shape(
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

pub(super) fn apply_pdb_wrong_playlist_tree_shape_repair(
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
pub(in crate::service) struct StaleSentinelBtree {
    pub(in crate::service) page_index: usize,
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
pub(in crate::service) fn detect_pdb_stale_sentinel_btree(
    pdb_path: &Path,
) -> Vec<StaleSentinelBtree> {
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

pub(super) fn apply_pdb_stale_sentinel_btree_repair(
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

pub(super) fn apply_pdb_tombstoned_playlist_tree_id_repair(
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
pub(in crate::service) fn detect_pdb_zero_tranrf_all_tables(
    pdb_path: &Path,
) -> Vec<ZeroTranrfPage> {
    detect_pdb_zero_tranrf_pages_for_tables(pdb_path, Some(&[]))
}

// table_filter: None = only tt=0; Some(&[]) = all tables; Some(list) = specific tables
pub(super) fn detect_pdb_zero_tranrf_pages_for_tables(
    pdb_path: &Path,
    table_filter: Option<&[u32]>,
) -> Vec<ZeroTranrfPage> {
    use crate::utils::{read_u16_le_at, read_u32_le_at};
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
        let u5 = read_u16_le_at(&bytes, off + 0x20).unwrap_or(0);
        let num_rl = read_u16_le_at(&bytes, off + 0x22).unwrap_or(0) as usize;
        let page = &bytes[off..off + page_size];
        let row_slots = crate::utils::packed_page_row_slot_count(page).unwrap_or(0);
        if row_slots == 0 || !pdb_page::footer_fits(page_size, row_slots) {
            continue;
        }

        let groups: Vec<_> = pdb_page::footer_groups(page_size, row_slots).collect();
        let rowpf_groups: Vec<u16> = groups.iter().map(|group| group.rowpf(page)).collect();
        let tranrf_groups: Vec<u16> = groups.iter().map(|group| group.tranrf(page)).collect();
        let footer_group_offsets: Vec<usize> = groups.iter().map(|group| group.rowpf_off).collect();

        // For u5=1 tables (dict tables, playlist entries) only the last group carries
        // tranrf; all preceding groups are correctly zero per the documented formula.
        // For other tables (u5=trc, u5=2) tranrf must mirror rowpf in every group.
        let needs_fix = if u5 == 1 {
            let last_group = num_rl / 16;
            last_group < groups.len()
                && rowpf_groups[last_group] != 0
                && tranrf_groups[last_group] == 0
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

pub(super) fn apply_pdb_zero_tranrf_repair(
    usb_root: &Path,
    pages: &[ZeroTranrfPage],
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
