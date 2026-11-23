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
    for (idx, page) in pdb_page::data_pages(bytes) {
        if pdb_page::table_type(page) != Some(table_type) {
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
        if payload_end > len_page || !pdb_page::footer_clear_of_heap(page, num_row_offsets) {
            continue;
        }
        let idx = idx as usize;
        out.push(InplaceDataPage {
            idx,
            start: idx * len_page,
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

/// One playlist-tree (t07) row as it sits on its page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlaylistTreeSlot {
    pub(crate) page: usize,
    pub(crate) slot: usize,
    /// Whether the row is live (its `rowpf` bit is set).
    pub(crate) live: bool,
    /// Whether the page's last write touched the row (its `tranrf` bit).
    pub(crate) in_last_write: bool,
    pub(crate) id: u32,
    pub(crate) parent_id: u32,
    pub(crate) sort_order: u32,
    pub(crate) is_folder: bool,
    /// The name with every NUL kept.
    pub(crate) name: String,
}

/// Every playlist-tree row slot whose row can be read, live or deleted.
pub(crate) fn playlist_tree_slots(bytes: &[u8]) -> Vec<PlaylistTreeSlot> {
    let Some(len_page) = inplace_page_size(bytes) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for data_page in inplace_data_pages(bytes, len_page, 7) {
        let page = &bytes[data_page.start..data_page.start + len_page];
        for slot in pdb_page::footer_slots(page, data_page.num_row_offsets) {
            let row_start = HEAP_START + slot.heap_off;
            let Some(row) = page.get(row_start..data_page.payload_end) else {
                continue;
            };
            let (Some(parent_id), Some(sort_order), Some(id), Some(folder)) = (
                read_u32_le_at(row, 0),
                read_u32_le_at(row, 8),
                read_u32_le_at(row, 12),
                read_u32_le_at(row, 16),
            ) else {
                continue;
            };
            let Some((name, _)) = crate::pdb_row::raw_pdb_string(row, 20) else {
                continue;
            };
            out.push(PlaylistTreeSlot {
                page: data_page.idx,
                slot: slot.index,
                live: slot.present,
                in_last_write: slot.group.tranrf(page) & slot.bit != 0,
                id,
                parent_id,
                sort_order,
                is_folder: folder == 1,
                name,
            });
        }
    }
    out
}

/// Tombstone the live playlist-tree rows at `slots` (`(page, slot)`), as
/// [`remove_rows_inplace`] does by id. Returns the rows removed.
pub(crate) fn remove_playlist_tree_slots_inplace(
    bytes: &mut [u8],
    slots: &[(usize, usize)],
) -> usize {
    let Some(len_page) = inplace_page_size(bytes) else {
        return 0;
    };
    let mut removed = 0;
    for data_page in inplace_data_pages(bytes, len_page, 7) {
        let page = &mut bytes[data_page.start..data_page.start + len_page];
        let doomed: Vec<_> = pdb_page::footer_slots(page, data_page.num_row_offsets)
            .filter(|s| s.present && slots.contains(&(data_page.idx, s.index)))
            .collect();
        removed += doomed.len();
        tombstone_slots(page, &data_page, &doomed, false);
    }
    removed
}

/// Overwrite the name of the playlist-tree row at `(page, slot)` with
/// `name`, when its encoding is as long as the old one's. Returns whether it
/// was written.
pub(crate) fn rename_playlist_tree_slot_inplace(
    bytes: &mut [u8],
    (page_idx, slot_idx): (usize, usize),
    name: &str,
) -> bool {
    let Some(len_page) = inplace_page_size(bytes) else {
        return false;
    };
    let encoded = crate::service::export_helpers::encode_pdb_string(name);
    for data_page in inplace_data_pages(bytes, len_page, 7) {
        if data_page.idx != page_idx {
            continue;
        }
        let page = &mut bytes[data_page.start..data_page.start + len_page];
        let Some(slot) =
            pdb_page::footer_slots(page, data_page.num_row_offsets).find(|s| s.index == slot_idx)
        else {
            return false;
        };
        let name_start = HEAP_START + slot.heap_off + 20;
        let Some(old_len) = page
            .get(HEAP_START + slot.heap_off..data_page.payload_end)
            .and_then(|row| crate::pdb_row::raw_pdb_string(row, 20))
            .map(|(_, len)| len)
        else {
            return false;
        };
        if old_len != encoded.len() || name_start + old_len > data_page.payload_end {
            return false;
        }
        page[name_start..name_start + old_len].copy_from_slice(&encoded);
        return true;
    }
    false
}

/// Move the live playlist (tt=8) and history (tt=12, tt=18) entries of the
/// tracks in `twins` (track id → id of the track taking its place) onto the
/// twin, in place. A playlist entry whose playlist already holds the twin is
/// removed instead (tombstoned as in [`remove_rows_inplace`]); history
/// entries are always kept. Returns `(repointed, removed)`.
pub(crate) fn merge_track_entries_inplace(
    bytes: &mut [u8],
    twins: &std::collections::HashMap<u32, u32>,
) -> (usize, usize) {
    if twins.is_empty() {
        return (0, 0);
    }
    let Some(len_page) = inplace_page_size(bytes) else {
        return (0, 0);
    };
    // (table, offset of the track id in the row, offset of the playlist id).
    const ENTRY_TABLES: [(u32, usize, usize); 3] = [(8, 4, 8), (12, 0, 4), (18, 0, 4)];
    let row_u32 = |page: &[u8], row: usize, at: usize| read_u32_le_at(page, row + at).unwrap_or(0);

    let mut repointed = 0usize;
    let mut removed = 0usize;
    for (table_type, track_at, playlist_at) in ENTRY_TABLES {
        // (playlist, track) pairs already present, so a moved playlist entry
        // never duplicates one (history may list a track more than once).
        let mut present = std::collections::HashSet::<(u32, u32)>::new();
        let pages = inplace_data_pages(bytes, len_page, table_type);
        if table_type == 8 {
            for data_page in &pages {
                let page = &bytes[data_page.start..data_page.start + len_page];
                for slot in pdb_page::footer_slots(page, data_page.num_row_offsets) {
                    let row = HEAP_START + slot.heap_off;
                    if slot.present && row + 12 <= data_page.payload_end {
                        present.insert((
                            row_u32(page, row, playlist_at),
                            row_u32(page, row, track_at),
                        ));
                    }
                }
            }
        }
        for data_page in &pages {
            let page = &mut bytes[data_page.start..data_page.start + len_page];
            let mut doomed = Vec::new();
            let slots: Vec<_> = pdb_page::footer_slots(page, data_page.num_row_offsets).collect();
            for slot in slots {
                let row = HEAP_START + slot.heap_off;
                if !slot.present || row + 12 > data_page.payload_end {
                    continue;
                }
                let Some(&twin) = twins.get(&row_u32(page, row, track_at)) else {
                    continue;
                };
                if table_type == 8 && !present.insert((row_u32(page, row, playlist_at), twin)) {
                    doomed.push(slot);
                    continue;
                }
                page[row + track_at..row + track_at + 4].copy_from_slice(&twin.to_le_bytes());
                repointed += 1;
            }
            removed += doomed.len();
            tombstone_slots(
                page,
                data_page,
                &doomed,
                table_uses_tombstone_footer_convention(table_type),
            );
        }
    }
    (repointed, removed)
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

/// Give tt=8 (playlist_entry) pages whose footer doesn't follow the
/// transaction rule the `(u5=1, num_rl=trc-1)` shape, with `tranrf` on the
/// last slot to match (docs/PDB.md "Page Footer Conventions"). Pages that
/// already follow the rule, rekordbox's included, are left alone.
pub(crate) fn fix_tt8_num_rl_conventions_inplace(bytes: &mut [u8]) {
    let Some(len_page) = inplace_page_size(bytes) else {
        return;
    };
    for data_page in inplace_data_pages(bytes, len_page, 8) {
        let page = &mut bytes[data_page.start..data_page.start + len_page];
        let slots = data_page.num_row_offsets;
        if !pdb_page::footer_fits(len_page, slots) || pdb_page::transaction_consistent(page, slots)
        {
            continue;
        }
        pdb_page::write_transaction(page, slots, &[slots - 1]);
    }
}

/// Rebuild sentinel B-tree entries for all tables in-memory.
///
/// For tt=0 and tt=19: the B-tree indexes all pages with flags=0x34.
/// Single-page chains have one 0x34 page (the sole data page); multi-page
/// chains have no 0x34 pages (all pages are 0x24), so the sentinel is left
/// as-is (preserving the SENTINEL_EMPTY_ENTRY entry written by the original exporter).
pub(crate) fn rebuild_sentinel_btrees_inplace(bytes: &mut [u8]) {
    rebuild_sentinel_btrees_for_tables(bytes, &[0, 19]);
}

/// [`rebuild_sentinel_btrees_inplace`] for only the listed tables, which
/// must use 0x34 for active pages (tt=0, tt=19).
pub(crate) fn rebuild_sentinel_btrees_for_tables(bytes: &mut [u8], tables: &[u32]) {
    let Some(page_size) = pdb_page::file_page_size(bytes) else {
        return;
    };
    let sentinels: Vec<(u32, u32)> = pdb_page::index_pages(bytes)
        .filter_map(|(index, page)| Some((index, pdb_page::table_type(page)?)))
        .filter(|(_, tt)| tables.contains(tt))
        .collect();

    for (sent_idx, tt) in sentinels {
        let sent_off = sent_idx as usize * page_size;
        let active = pdb_page::active_data_pages(bytes, tt);
        if active.is_empty() {
            continue; // nothing to write — leave sentinel as-is
        }
        if !write_sentinel_btree(bytes, sent_off, page_size, &active) {
            continue;
        }

        // Update sentinel seq to current max data-page seq + 1.
        // All accepted reference exports have non-1 seq on updated sentinels
        // (e.g. tt=0 seq=31, tt=19 seq=15 vs seq=1 on un-updated sentinels).
        // A sentinel with seq=1 while data pages have higher seqs indicates the
        // B-tree was never refreshed, which detected as corruption.
        let max_seq = pdb_page::pages(bytes)
            .filter(|(_, page)| {
                !pdb_page::is_index_page(page) && read_u32_le_at(page, 4).unwrap_or(0) != 0
            })
            .filter_map(|(_, page)| read_u32_le_at(page, 0x10))
            .max()
            .unwrap_or(1);
        let sentinel_seq = max_seq.saturating_add(1);
        bytes[sent_off + 0x10..sent_off + 0x14].copy_from_slice(&sentinel_seq.to_le_bytes());
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

/// Point the sentinel page at `sent_off` at the `active` data pages (sorted
/// page indexes): entries `page * 8`, the rest empty. A B-tree that already
/// indexes exactly these pages stays as it is: rekordbox keeps its own entry
/// order and free slots, and rewriting them changes a valid page for nothing.
/// Returns whether anything was written.
pub(crate) fn write_sentinel_btree(
    bytes: &mut [u8],
    sent_off: usize,
    page_size: usize,
    active: &[u32],
) -> bool {
    let Some(entries) = sentinel_entry_area(bytes, sent_off, page_size) else {
        return false;
    };
    let mut indexed: Vec<u32> = bytes[entries]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&entry| u32::from_le_bytes(entry))
        .filter(|&entry| entry != SENTINEL_EMPTY_ENTRY)
        .map(|entry| entry / 8)
        .collect();
    indexed.sort_unstable();
    indexed != active && fill_sentinel_btree(bytes, sent_off, page_size, active)
}

/// The byte range of the sentinel page at `sent_off` that holds its B-tree
/// entries, when the page is in `bytes`.
fn sentinel_entry_area(
    bytes: &[u8],
    sent_off: usize,
    page_size: usize,
) -> Option<std::ops::Range<usize>> {
    page_size
        .checked_sub(SENTINEL_TAIL_ZERO_BYTES)
        .map(|end| sent_off + end)
        .filter(|&end| end <= bytes.len() && end >= sent_off + 0x3c)
        .map(|end| sent_off + 0x3c..end)
}

/// [`write_sentinel_btree`] whatever the B-tree holds now, so `ne`, the
/// free-list head and `u7` are rewritten too. The stale-B-tree repair uses
/// this: the entries may be right while those fields aren't.
pub(crate) fn fill_sentinel_btree(
    bytes: &mut [u8],
    sent_off: usize,
    page_size: usize,
    active: &[u32],
) -> bool {
    let Some(entries) = sentinel_entry_area(bytes, sent_off, page_size) else {
        return false;
    };
    let fill_end = entries.end;
    let mut cursor = entries.start;
    for &page_idx in active {
        if cursor + 4 > fill_end {
            break;
        }
        bytes[cursor..cursor + 4].copy_from_slice(&(page_idx * 8).to_le_bytes());
        cursor += 4;
    }
    while cursor + 4 <= fill_end {
        bytes[cursor..cursor + 4].copy_from_slice(&SENTINEL_EMPTY_ENTRY.to_le_bytes());
        cursor += 4;
    }
    let ne = active.len().min(0x1FFF) as u16;
    bytes[sent_off + 0x38..sent_off + 0x3a].copy_from_slice(&ne.to_le_bytes());
    bytes[sent_off + 0x3a..sent_off + 0x3c].copy_from_slice(&0x1fffu16.to_le_bytes());
    bytes[sent_off + 0x26..sent_off + 0x28].copy_from_slice(&ne.to_le_bytes());
    true
}
