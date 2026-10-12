//! In-place row deletion and repair passes over an existing PDB: tombstoning
//! rows, duplicate cleanup, t08 entry-index patches and footer/sentinel fixes.

use super::*;

// ── In-place row deletion ───────────────────────────────────────────────────

/// Tables whose freshly written page footer uses the `(1, trc - 1)`
/// tombstone-style convention (see docs/PDB.md "Page Footer Conventions").
/// `t06`, `t07`, `t16`, `t17`, `t18` use `(trc, 0)` instead, and `trc` (the
/// row-slot count) never changes when a row is merely tombstoned, so those
/// tables must keep their existing `u5`/`num_rl`/`tranrf` footer values as-is.
fn table_uses_tombstone_footer_convention(table_type: u32) -> bool {
    matches!(table_type, 0 | 1 | 2 | 3 | 4 | 5 | 8 | 13)
}

/// A data page that the in-place row passes below may edit: of the wanted
/// table, not an index page, with at least one row slot and a payload that
/// ends inside the page.
struct InplaceDataPage {
    idx: usize,
    start: usize,
    num_row_offsets: usize,
    num_rows: usize,
    /// End of the row payload, relative to the page start.
    payload_end: usize,
}

/// Page size of a PDB the in-place passes can work on: the header's page
/// size must be [`PAGE_SIZE`] and the file must hold at least two pages.
fn inplace_page_size(bytes: &[u8]) -> Option<usize> {
    let len_page = read_u32_le_at(bytes, 4)? as usize;
    (len_page == PAGE_SIZE && bytes.len() >= len_page * 2).then_some(len_page)
}

fn inplace_data_pages(bytes: &[u8], len_page: usize, table_type: u32) -> Vec<InplaceDataPage> {
    let mut out = Vec::new();
    for idx in 1..bytes.len() / len_page {
        let start = idx * len_page;
        let page = &bytes[start..start + len_page];
        if read_u32_le_at(page, 0x08) != Some(table_type) {
            continue;
        }
        // Skip index/sentinel pages (bit 6 set in page_flags)
        if page[0x1b] & 0x40 != 0 {
            continue;
        }
        let Some((num_row_offsets, num_rows)) = pdb_page::read_slot_counts(page) else {
            continue;
        };
        if num_row_offsets == 0 {
            continue;
        }
        let Some(used_size) = read_u16_le_at(page, 0x1e) else {
            continue;
        };
        let payload_end = HEAP_START + used_size as usize;
        if payload_end > len_page {
            continue;
        }
        out.push(InplaceDataPage {
            idx,
            start,
            num_row_offsets,
            num_rows,
            payload_end,
        });
    }
    out
}

/// Tombstone `slots` (all live, all on `page`): clear their `rowpf` bits and
/// drop them from `num_rows`, keeping `num_row_offsets`.
///
/// Tombstone state is tracked only in the row footer bitmasks (rowpf/tranrf).
/// Page_flags must not be modified — setting 0x10 on a sealed (0x24)
/// overflow page would produce 0x34 which DJ players reject as corrupted on
/// non-tail pages.
///
/// With `restamp_convention`, also rewrite `u5=1`, `num_rl=trc-1` and the
/// matching single-bit `tranrf`. Only tables in the `(1, trc - 1)`
/// convention group use this shape; `(trc, 0)`-convention tables keep their
/// existing u5/num_rl/tranrf untouched (see
/// `table_uses_tombstone_footer_convention`).
///
/// num_rl must be trc-1 unconditionally, NOT the tombstoned row's own slot
/// index -- num_row_offsets never shrinks on a tombstone, so the page's last
/// slot stays num_row_offsets-1 regardless of which row got removed. Using
/// the removed slot here only looked correct in exports where the removed
/// row happened to be the page's last one; a mid-page removal left num_rl
/// short of trc-1, which strict player validation (and
/// `validate_pdb_page_conventions`) rejects.
fn tombstone_slots(
    page: &mut [u8],
    data_page: &InplaceDataPage,
    slots: &[pdb_page::FooterSlot],
    restamp_convention: bool,
) {
    if slots.is_empty() {
        return;
    }
    for slot in slots {
        let rowpf = slot.group.rowpf(page);
        slot.group.set_rowpf(page, rowpf & !slot.bit);
    }
    pdb_page::write_slot_counts(
        page,
        data_page.num_row_offsets,
        data_page.num_rows.saturating_sub(slots.len()),
    );
    if restamp_convention {
        let last_slot = data_page.num_row_offsets.saturating_sub(1);
        page[0x20..0x22].copy_from_slice(&1u16.to_le_bytes());
        page[0x22..0x24].copy_from_slice(&(last_slot as u16).to_le_bytes());
        pdb_page::write_single_slot_tranrf(page, data_page.num_row_offsets, last_slot);
    }
}

/// Remove rows from a PDB file in-place, preserving page layout.
///
/// For each page of the given `table_type`, rows whose ID (extracted by
/// `extract_row_id`) is in `ids_to_remove` are tombstoned (see
/// [`tombstone_slots`]). The heap data and page count are left unchanged.
///
/// Returns the number of rows actually removed.
pub(crate) fn remove_rows_inplace(
    bytes: &mut [u8],
    table_type: u32,
    ids_to_remove: &std::collections::HashSet<u32>,
    extract_row_id: fn(&[u8]) -> Option<u32>,
) -> usize {
    if ids_to_remove.is_empty() {
        return 0;
    }
    let Some(len_page) = inplace_page_size(bytes) else {
        return 0;
    };

    let mut removed = 0usize;
    for data_page in inplace_data_pages(bytes, len_page, table_type) {
        let page = &mut bytes[data_page.start..data_page.start + len_page];
        let doomed: Vec<_> = pdb_page::footer_slots(page, data_page.num_row_offsets)
            .filter(|slot| {
                let row_start = HEAP_START + slot.heap_off;
                slot.present
                    && row_start < data_page.payload_end
                    && extract_row_id(&page[row_start..data_page.payload_end])
                        .is_some_and(|id| ids_to_remove.contains(&id))
            })
            .collect();
        removed += doomed.len();
        tombstone_slots(
            page,
            &data_page,
            &doomed,
            table_uses_tombstone_footer_convention(table_type),
        );
    }
    removed
}

/// Location of one active `tt=8` (playlist_entries) row.
struct PlaylistEntryLocation {
    page_idx: usize,
    slot: usize,
    entry_index: u32,
}

/// Remove stale duplicate `tt=8` (playlist_entries) rows in-place.
///
/// Years of desktop-software edits on a real USB can leave a playlist's old
/// membership rows behind on a shared page: when a playlist grows and its
/// tail relocates to a fresh page, the old copy of the relocated rows is not
/// always cleared from the original page. The result is the same
/// `(playlist_id, track_id)` pair appearing on more than one active row —
/// invisible to hardware (which reads the current page's live count) but
/// visible to this app's own strict-parity diagnostics, which walks every
/// active row via the page's real row-presence bitmap
/// (`num_row_offsets`/`num_rows`, not the separate and occasionally stale
/// `num_rl` footer field — see docs/PDB.md).
///
/// For each `(playlist_id, track_id)` pair with more than one active row,
/// keeps the row with the highest `entry_index` and tombstones the rest the
/// same way as `remove_rows_inplace`.
///
/// Returns the number of rows tombstoned.
pub(crate) fn remove_duplicate_playlist_entries_inplace(bytes: &mut [u8]) -> usize {
    let Some(len_page) = inplace_page_size(bytes) else {
        return 0;
    };
    let data_pages = inplace_data_pages(bytes, len_page, 8);

    // ── Pass 1: locate every active row, grouped by (playlist_id, track_id) ──
    let mut locations: std::collections::HashMap<(u32, u32), Vec<PlaylistEntryLocation>> =
        std::collections::HashMap::new();
    for data_page in &data_pages {
        let page = &bytes[data_page.start..data_page.start + len_page];
        for slot in pdb_page::footer_slots(page, data_page.num_row_offsets) {
            let row_start = HEAP_START + slot.heap_off;
            if !slot.present || row_start >= data_page.payload_end {
                continue;
            }
            let row_data = &page[row_start..data_page.payload_end];
            let (Some(entry_index), Some(track_id), Some(playlist_id)) = (
                extract_u32_at(row_data, 0x00),
                extract_playlist_entry_track_id(row_data),
                extract_playlist_entry_playlist_id(row_data),
            ) else {
                continue;
            };
            locations
                .entry((playlist_id, track_id))
                .or_default()
                .push(PlaylistEntryLocation {
                    page_idx: data_page.idx,
                    slot: slot.index,
                    entry_index,
                });
        }
    }

    // ── Decide removals: keep the highest entry_index per (playlist_id, track_id) ──
    // The additive writer now updates a track's entry_index in place when its
    // position changes (see `patch_t08_entry_indices_in_place`), so this sweep
    // only exists to clean up duplicates left behind by exports predating that
    // fix. In that failure mode the *highest* entry_index is the most recently
    // written (correct, current) position; the lower one(s) are the stale
    // pre-reposition copies that were never cleaned up. Keeping the lowest (the
    // prior behavior) would silently keep the stale position instead.
    let mut to_remove: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for locs in locations.values() {
        if locs.len() < 2 {
            continue;
        }
        let mut sorted: Vec<&PlaylistEntryLocation> = locs.iter().collect();
        sorted.sort_by_key(|l| (l.entry_index, l.page_idx, l.slot));
        for loc in sorted.into_iter().rev().skip(1) {
            to_remove.insert((loc.page_idx, loc.slot));
        }
    }
    if to_remove.is_empty() {
        return 0;
    }

    // ── Pass 2: tombstone the selected rows, page by page ───────────────────
    let mut removed = 0usize;
    for data_page in &data_pages {
        let page = &mut bytes[data_page.start..data_page.start + len_page];
        let doomed: Vec<_> = pdb_page::footer_slots(page, data_page.num_row_offsets)
            .filter(|slot| slot.present && to_remove.contains(&(data_page.idx, slot.index)))
            .collect();
        removed += doomed.len();
        tombstone_slots(page, data_page, &doomed, true);
    }
    removed
}

/// Update a t08 (playlist_entry) row's `entry_index` in place, without
/// touching its `track_id`/`playlist_id` identity or moving the row.
///
/// `patches` is `(playlist_id, track_id, new_entry_index)`. An existing row
/// is matched by `(playlist_id, track_id)` only — `entry_index` is treated
/// as a mutable position, never part of identity (see `T08EntryKey`'s doc
/// comment). This is what makes a track's position change in an already-
/// exported playlist a true in-place update instead of a remove+add pair:
/// there is no delete/add pair here for a partial failure (e.g. an
/// interrupted export) to leave duplicated behind. Rows are fixed-length
/// (12 bytes), so this is a pure 4-byte overwrite — no page relayout.
///
/// Returns the number of rows actually patched.
pub(crate) fn patch_t08_entry_indices_in_place(
    bytes: &mut [u8],
    patches: &[(u32, u32, u32)],
    page_size: usize,
) -> usize {
    if patches.is_empty() {
        return 0;
    }
    if page_size != PAGE_SIZE || bytes.len() < page_size || !bytes.len().is_multiple_of(page_size) {
        return 0;
    }
    let desired_by_identity: std::collections::HashMap<(u32, u32), u32> = patches
        .iter()
        .map(|&(playlist_id, track_id, new_entry_index)| ((playlist_id, track_id), new_entry_index))
        .collect();

    let mut touched_pages = std::collections::HashSet::<usize>::new();
    let mut patched = 0usize;

    for data_page in inplace_data_pages(bytes, page_size, 8) {
        let page = &mut bytes[data_page.start..data_page.start + page_size];
        let row_starts: Vec<usize> = pdb_page::footer_slots(page, data_page.num_row_offsets)
            .filter(|slot| slot.present)
            .map(|slot| HEAP_START + slot.heap_off)
            .filter(|&row_start| row_start + 12 <= data_page.payload_end)
            .collect();
        for row_start in row_starts {
            let row_data = &page[row_start..row_start + 12];
            let (Some(track_id), Some(playlist_id)) = (
                extract_playlist_entry_track_id(row_data),
                extract_playlist_entry_playlist_id(row_data),
            ) else {
                continue;
            };
            let Some(&new_entry_index) = desired_by_identity.get(&(playlist_id, track_id)) else {
                continue;
            };
            let current_entry_index = extract_u32_at(row_data, 0x00).unwrap_or(0);
            if current_entry_index == new_entry_index {
                continue;
            }
            page[row_start..row_start + 4].copy_from_slice(&new_entry_index.to_le_bytes());
            touched_pages.insert(data_page.idx);
            patched += 1;
        }
    }

    if !touched_pages.is_empty() {
        let mut next_seq = max_seqpage_in_file(bytes, page_size)
            .saturating_add(1)
            .max(2);
        let mut pages: Vec<usize> = touched_pages.into_iter().collect();
        pages.sort_unstable();
        for page_idx in pages {
            let off = page_idx * page_size;
            let _ = write_u32_le_at(bytes, off + 0x10, next_seq);
            next_seq = next_seq.saturating_add(1);
        }
        let seqdb = read_u32_le_at(bytes, 0x14)
            .unwrap_or(0)
            .max(next_seq)
            .max(max_seqpage_in_file(bytes, page_size).saturating_add(1));
        let _ = write_u32_le_at(bytes, 0x14, seqdb);
    }

    patched
}

/// Normalise tt=8 (playlist_entry) page footer fields to `(u5=1, num_rl=trc-1)`.
///
/// Exports produced by other software occasionally carry interior pages whose
/// `num_rl` does not follow the `trc-1` convention required by strict player
/// validation. This pass corrects those pages before the post-write convention
/// check so the writer's output is always conformant.
pub(crate) fn fix_tt8_num_rl_conventions_inplace(bytes: &mut [u8]) {
    let len_page = match read_u32_le_at(bytes, 4) {
        Some(v) if v > 0 => v as usize,
        _ => return,
    };
    if bytes.len() < len_page {
        return;
    }
    let total_pages = bytes.len() / len_page;
    for page_idx in 1..total_pages {
        let off = page_idx * len_page;
        if off + 0x28 > bytes.len() {
            break;
        }
        let table_type = match read_u32_le_at(&bytes[off..], 0x08) {
            Some(t) => t,
            None => continue,
        };
        if table_type != 8 {
            continue;
        }
        let pf = match read_u8_at(&bytes[off..], 0x1b) {
            Some(v) => v,
            None => continue,
        };
        if pf == 0x64 {
            continue;
        }
        let nrs = match read_u8_at(&bytes[off..], 0x18) {
            Some(v) if v > 0 => v,
            _ => continue,
        };
        let observed_num_rl = match read_u16_le_at(&bytes[off..], 0x22) {
            Some(v) => v,
            None => continue,
        };
        if observed_num_rl == 8191 {
            continue;
        }
        let trc: u16 = if observed_num_rl >= nrs as u16 {
            observed_num_rl.saturating_add(1)
        } else {
            nrs as u16
        };
        let exp_num_rl = trc.saturating_sub(1);
        let exp_u5: u16 = 1;
        let observed_u5 = read_u16_le_at(&bytes[off..], 0x20).unwrap_or(0);
        if observed_u5 != exp_u5 {
            bytes[off + 0x20] = exp_u5 as u8;
            bytes[off + 0x21] = (exp_u5 >> 8) as u8;
        }
        if observed_num_rl != exp_num_rl {
            bytes[off + 0x22] = exp_num_rl as u8;
            bytes[off + 0x23] = (exp_num_rl >> 8) as u8;
        }
    }
}

/// Rebuild sentinel B-tree entries for all tables in-memory.
///
/// For tt=0 and tt=19: the B-tree indexes all pages with flags=0x34.
/// Single-page chains have one 0x34 page (the sole data page); multi-page
/// chains have no 0x34 pages (all pages are 0x24), so the sentinel is left
/// as-is (preserving the SENTINEL_EMPTY_ENTRY entry written by the original exporter).
pub(crate) fn rebuild_sentinel_btrees_inplace(bytes: &mut [u8]) {
    let Some(page_size) = bytes
        .get(4..8)
        .and_then(|b| b.try_into().ok())
        .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
    else {
        return;
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return;
    }
    let total = bytes.len() / page_size;

    for sent_idx in 1..total {
        let sent_off = sent_idx * page_size;
        if sent_off + page_size > bytes.len() {
            break;
        }
        // Must be a sentinel (0x64)
        if bytes.get(sent_off + 0x1b).copied() != Some(0x64) {
            continue;
        }
        let tt = bytes
            .get(sent_off + 8..sent_off + 12)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
            .unwrap_or(9999);
        // Only update tables that use 0x34 for active pages
        if !matches!(tt, 0 | 19) {
            continue;
        }

        // Collect all data pages for this table with flags=0x34
        let mut active: Vec<u32> = Vec::new();
        for pi in 1..total {
            if pi == sent_idx {
                continue;
            }
            let po = pi * page_size;
            if po + page_size > bytes.len() {
                break;
            }
            let ptt = bytes
                .get(po + 8..po + 12)
                .and_then(|b| b.try_into().ok())
                .map(u32::from_le_bytes)
                .unwrap_or(9999);
            if ptt != tt {
                continue;
            }
            let pf = bytes.get(po + 0x1b).copied().unwrap_or(0);
            if pf != 0x34 {
                continue; // 0x64 sentinel, 0x24 sealed, or unknown
            }
            let used = bytes
                .get(po + 0x1e..po + 0x20)
                .and_then(|b| b.try_into().ok())
                .map(|b: [u8; 2]| u16::from_le_bytes(b))
                .unwrap_or(0);
            if used == 0 {
                continue; // blank page
            }
            active.push(pi as u32);
        }
        active.sort_unstable();

        let ne = active.len().min(0x1FFF) as u16;
        if ne == 0 {
            continue; // nothing to write — leave sentinel as-is
        }

        // Update sentinel seq to current max data-page seq + 1.
        // All accepted reference exports have non-1 seq on updated sentinels
        // (e.g. tt=0 seq=31, tt=19 seq=15 vs seq=1 on un-updated sentinels).
        // A sentinel with seq=1 while data pages have higher seqs indicates the
        // B-tree was never refreshed, which detected as corruption.
        let max_seq = (1..total)
            .filter(|&j| j != sent_idx)
            .filter_map(|j| {
                let o = j * page_size;
                bytes.get(o..o + page_size).and_then(|p| {
                    if p[0x1b] != 0x64 && u32::from_le_bytes(p[4..8].try_into().ok()?) != 0 {
                        Some(u32::from_le_bytes(p[0x10..0x14].try_into().ok()?))
                    } else {
                        None
                    }
                })
            })
            .max()
            .unwrap_or(1);
        let sentinel_seq = max_seq.saturating_add(1);
        bytes[sent_off + 0x10..sent_off + 0x14].copy_from_slice(&sentinel_seq.to_le_bytes());

        let fill_end = sent_off + page_size.saturating_sub(SENTINEL_TAIL_ZERO_BYTES);
        let mut cursor = sent_off + 0x3c;
        for &pi in &active {
            if cursor + 4 > fill_end {
                break;
            }
            bytes[cursor..cursor + 4].copy_from_slice(&(pi * 8).to_le_bytes());
            cursor += 4;
        }
        while cursor + 4 <= fill_end {
            bytes[cursor..cursor + 4].copy_from_slice(&SENTINEL_EMPTY_ENTRY.to_le_bytes());
            cursor += 4;
        }
        bytes[sent_off + 0x38..sent_off + 0x3a].copy_from_slice(&ne.to_le_bytes());
        bytes[sent_off + 0x3a..sent_off + 0x3c].copy_from_slice(&0x1fffu16.to_le_bytes());
        bytes[sent_off + 0x26..sent_off + 0x28].copy_from_slice(&ne.to_le_bytes());
    }
}

fn extract_u32_at(row: &[u8], offset: usize) -> Option<u32> {
    read_u32_le_at(row, offset)
}

/// Extract track ID from a raw track row (at offset 0x48 LE).
pub(crate) fn extract_track_id(row: &[u8]) -> Option<u32> {
    extract_u32_at(row, 0x48)
}

/// Extract playlist tree row ID (at offset 0x0c LE).
pub(crate) fn extract_playlist_tree_id(row: &[u8]) -> Option<u32> {
    extract_u32_at(row, 0x0c)
}

/// Extract playlist entry's playlist_id (at offset 0x08 LE).
pub(crate) fn extract_playlist_entry_playlist_id(row: &[u8]) -> Option<u32> {
    extract_u32_at(row, 0x08)
}

/// Extract playlist entry's track_id (at offset 0x04 LE).
pub(crate) fn extract_playlist_entry_track_id(row: &[u8]) -> Option<u32> {
    extract_u32_at(row, 0x04)
}

/// Extract artwork row ID (at offset 0x00 LE).
pub(crate) fn extract_artwork_id(row: &[u8]) -> Option<u32> {
    extract_u32_at(row, 0)
}
