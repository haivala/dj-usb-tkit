//! Page/row primitives shared by the in-place paths: rewriting a
//! variable-row page, track-row string slot patches and reading a PDB's
//! playlist entries.

use super::*;

pub(crate) fn patch_track_row_slot_bytes_if_len_matches(
    row: &mut [u8],
    slot_index: usize,
    desired: &[u8],
) -> bool {
    if row.len() < 136 {
        return false;
    }
    let Some(slot_start) = pdb_row::track_string_offset(row, slot_index) else {
        return false;
    };
    let slot_end = if slot_index + 1 < pdb_row::TRACK_STRING_SLOTS {
        let Some(next) = pdb_row::track_string_offset(row, slot_index + 1) else {
            return false;
        };
        next
    } else {
        row.len()
    };
    if slot_start >= slot_end || slot_end > row.len() {
        return false;
    }
    let Some(dst) = row.get_mut(slot_start..slot_end) else {
        return false;
    };
    if dst.len() != desired.len() {
        return false;
    }
    dst.copy_from_slice(desired);
    true
}
/// The live playlist entries on `page`.
fn parse_t08_entries_from_page(page: &[u8]) -> Vec<PdbPlaylistEntryRow> {
    if !pdb_page::is_data_page(page) {
        return Vec::new();
    }
    pdb_page::present_rows(page)
        .iter()
        .filter_map(|row| pdb_row::playlist_entry(row))
        .collect()
}

pub(crate) fn collect_t08_entry_keys(bytes: &[u8], page_size: usize) -> Vec<PdbPlaylistEntryRow> {
    let Some((_ec, first, last)) = table_ptr_fields(bytes, 8) else {
        return Vec::new();
    };
    let chain = collect_chain_pages(bytes, page_size, first, last).unwrap_or_default();
    let mut out = Vec::new();
    for &page_idx in chain.iter().skip(1) {
        let Some(off) = page_offset(page_idx, page_size) else {
            continue;
        };
        let Some(page) = bytes.get(off..off + page_size) else {
            continue;
        };
        out.extend(parse_t08_entries_from_page(page));
    }
    out
}

/// Whether every heap offset, `used_s` and `free_s` on a page of `page_size`
/// bytes fits the u16 fields that hold them. The in-place writers take the
/// page size from the file header, so they check this before casting.
pub(super) fn page_size_fits_u16_fields(page_size: usize) -> bool {
    page_size <= usize::from(u16::MAX)
}

pub(crate) fn rewrite_variable_page_rows_in_place(
    bytes: &mut [u8],
    page_index: u32,
    rows: &[Vec<u8>],
    page_size: usize,
) -> bool {
    if !page_size_fits_u16_fields(page_size) {
        return false;
    }
    let Some(off) = page_offset(page_index, page_size) else {
        return false;
    };
    let Some(page) = bytes.get_mut(off..off + page_size) else {
        return false;
    };
    let mut payload = Vec::<u8>::new();
    let mut row_offsets = Vec::<u16>::with_capacity(rows.len());
    for row in rows {
        if row.is_empty() {
            continue;
        }
        if payload.len() > u16::MAX as usize {
            return false;
        }
        row_offsets.push(payload.len() as u16);
        payload.extend_from_slice(row);
        payload.resize(align4(payload.len()), 0);
    }
    let used_s = payload.len();
    let footer_size = footer_size(row_offsets.len());
    if HEAP_START + used_s + footer_size > page_size {
        return false;
    }

    pdb_page::write_slot_counts(page, row_offsets.len(), row_offsets.len());
    page[0x20..0x22].copy_from_slice(&(row_offsets.len() as u16).to_le_bytes());
    page[0x1e..0x20].copy_from_slice(&(used_s as u16).to_le_bytes());
    let free_s = page_size.saturating_sub(HEAP_START + used_s + footer_size);
    page[0x1c..0x1e].copy_from_slice(&(free_s as u16).to_le_bytes());
    // Every slot's tranrf bit is set below, so the transaction rule gives
    // `(trc, 0)`: the shape t06/t07/t16/t17/t18 keep. Callers writing a
    // `u5 = 1` table restamp it (`apply_page_footer_convention`).
    page[0x22..0x24].copy_from_slice(&0u16.to_le_bytes());

    if page_size > HEAP_START {
        page[HEAP_START..].fill(0u8);
    }
    if !payload.is_empty() {
        page[HEAP_START..HEAP_START + payload.len()].copy_from_slice(&payload);
    }
    pdb_page::write_footer_all_present(page, &row_offsets);
    true
}

pub(crate) fn max_seqpage_in_file(bytes: &[u8], page_size: usize) -> u32 {
    if bytes.len() < page_size || !bytes.len().is_multiple_of(page_size) {
        return 0;
    }
    let max_page = (bytes.len() / page_size).saturating_sub(1) as u32;
    let mut out = 0u32;
    for page_idx in 1..=max_page {
        let Some(off) = page_offset(page_idx, page_size) else {
            continue;
        };
        out = out.max(read_u32_le_at(bytes, off + 0x10).unwrap_or(0));
    }
    out
}
