//! Topology-locked additive writer: brings an existing `export.pdb` up to date
//! with a `PdbData` in place. Rows are appended to existing table chains (fresh
//! pages only when they run out), changed track rows are patched or relocated,
//! and pages the diff doesn't touch are left byte-for-byte as they were.

use super::*;

/// Maximum encoded row length that can fit on any single page after 4-byte heap
/// alignment, the 40-byte page header, and the minimum 6-byte single-row footer
/// (`[ofs0][rowpf][tranrf]`). Rows larger than this cannot be placed anywhere
/// (existing or fresh) and must be rejected up-front.
pub(crate) const MAX_ROW_LEN: usize = (PAGE_SIZE - HEAP_START - 6) & !3;

/// Outcome of appending a batch of rows to a single table's chain.
#[derive(Debug, Clone, Default)]
pub(crate) struct AppendOutcome {
    /// Number of existing data pages that absorbed at least one row.
    pub pages_reused: usize,
    /// Number of fresh data pages appended at the file tail.
    pub pages_appended: usize,
    /// `last_page` after the operation (== before, when no fresh pages
    /// were appended; advances by `pages_appended` otherwise).
    pub new_last_page: u32,
    /// `empty_candidate` after the operation. Preserved when rows fit into
    /// existing pages; advanced only when the table chain grows.
    pub new_empty_candidate: u32,
}

pub(super) fn page_has_transaction_tombstones(page: &[u8], page_size: usize) -> bool {
    if page.len() < page_size || page_size < HEAP_START {
        return false;
    }
    let used_s = read_u16_le_at(page, 0x1e).unwrap_or(0);
    if used_s == 0 {
        return false;
    }
    let row_slots = packed_page_row_slot_count(page).unwrap_or(0);
    pdb_page::footer_groups(page_size, row_slots)
        .any(|group| group.tranrf(page) & !group.rowpf(page) & group.mask() != 0)
}

pub(super) struct PageFooterState {
    pub(super) offsets: Vec<usize>,
    pub(super) rowpf_by_group: Vec<u16>,
    pub(super) tranrf_by_group: Vec<u16>,
}

pub(super) fn read_page_footer_state(page: &[u8], page_size: usize) -> Option<PageFooterState> {
    if page.len() < page_size || page_size < HEAP_START {
        return None;
    }
    let used_s = read_u16_le_at(page, 0x1e).unwrap_or(0) as usize;
    if used_s == 0 {
        return None;
    }
    let row_slots = packed_page_row_slot_count(page).unwrap_or(0);
    if row_slots == 0 || !pdb_page::footer_fits(page_size, row_slots) {
        return None;
    }

    let page = &page[..page_size];
    let mut offsets = Vec::<usize>::with_capacity(row_slots);
    let mut rowpf_by_group = Vec::<u16>::with_capacity(row_slots.div_ceil(pdb_page::GROUP_SLOTS));
    let mut tranrf_by_group = Vec::<u16>::with_capacity(row_slots.div_ceil(pdb_page::GROUP_SLOTS));
    for group in pdb_page::footer_groups(page_size, row_slots) {
        rowpf_by_group.push(group.rowpf(page));
        tranrf_by_group.push(group.tranrf(page));
        offsets.extend((0..group.len).map(|j| group.row_offset(page, j)));
    }

    if offsets.iter().any(|offset| *offset > used_s) {
        return None;
    }
    if offsets.windows(2).any(|pair| pair[0] > pair[1]) {
        return None;
    }

    Some(PageFooterState {
        offsets,
        rowpf_by_group,
        tranrf_by_group,
    })
}

fn append_rows_to_existing_page_preserving_footer_state(
    bytes: &mut [u8],
    table_type: u32,
    page_idx: u32,
    rows: &[Vec<u8>],
    page_size: usize,
) -> BackendResult<usize> {
    if rows.is_empty() {
        return Ok(0);
    }
    // tt=19 (history_runtime) carries a whole-page "runtime profile" (exactly
    // one active row) rather than per-row presence bits. OR-ing an appended
    // row's bit into an existing page's footer here would mark every history
    // row active — the exact shape the tt=19 footer-corruption fix removed.
    // Growth of tt=19 must only ever go through
    // `synthesize_t19_runtime_rows_in_place`, never this generic path.
    debug_assert_ne!(
        table_type, 19,
        "tt=19 must not grow through the generic append path"
    );
    if !page_size_fits_u16_fields(page_size) {
        return Err(BackendError::Validation(format!(
            "additive append: page size {page_size} too large for u16 page header fields"
        )));
    }
    let off = page_offset(page_idx, page_size).ok_or_else(|| {
        BackendError::Validation(format!(
            "additive append: page {page_idx} for table {table_type} out of bounds"
        ))
    })?;
    let page_snapshot = bytes
        .get(off..off + page_size)
        .ok_or_else(|| {
            BackendError::Validation(format!(
                "additive append: page {page_idx} for table {table_type} out of bounds"
            ))
        })?
        .to_vec();
    let Some(mut footer) = read_page_footer_state(&page_snapshot, page_size) else {
        return Ok(0);
    };
    let old_u5 = read_u16_le_at(&page_snapshot, 0x20).unwrap_or(0);
    let old_num_rl = read_u16_le_at(&page_snapshot, 0x22).unwrap_or(0);
    let mut used_s = read_u16_le_at(&page_snapshot, 0x1e).unwrap_or(0) as usize;
    let mut new_used_s = used_s;
    let mut absorbed = 0usize;

    while absorbed < rows.len() {
        let row = &rows[absorbed];
        let start = align4(new_used_s);
        let end = start.saturating_add(row.len());
        let aligned_end = align4(end);
        let new_row_count = footer
            .offsets
            .len()
            .saturating_add(absorbed)
            .saturating_add(1);
        let new_footer = footer_size(new_row_count);
        if HEAP_START + aligned_end + new_footer > page_size || new_row_count > MAX_ROWS_PER_PAGE {
            break;
        }
        new_used_s = aligned_end;
        absorbed += 1;
    }

    if absorbed == 0 {
        return Ok(0);
    }

    let page = bytes.get_mut(off..off + page_size).ok_or_else(|| {
        BackendError::Validation(format!(
            "additive append: page {page_idx} for table {table_type} disappeared"
        ))
    })?;
    for row in rows.iter().take(absorbed) {
        let start = align4(used_s);
        if start > used_s {
            page[HEAP_START + used_s..HEAP_START + start].fill(0);
        }
        let slot = footer.offsets.len();
        let mut row_bytes = row.clone();
        apply_page_local_index_shift(table_type, &mut row_bytes, slot);
        let end = start + row_bytes.len();
        page[HEAP_START + start..HEAP_START + end].copy_from_slice(&row_bytes);
        let aligned_end = align4(end);
        if aligned_end > end {
            page[HEAP_START + end..HEAP_START + aligned_end].fill(0);
        }

        footer.offsets.push(start);
        let group = slot / pdb_page::GROUP_SLOTS;
        let bit = slot % pdb_page::GROUP_SLOTS;
        while footer.rowpf_by_group.len() <= group {
            footer.rowpf_by_group.push(0);
            footer.tranrf_by_group.push(0);
        }
        let bit_mask = 1u16 << bit;
        footer.rowpf_by_group[group] |= bit_mask;
        footer.tranrf_by_group[group] |= bit_mask;
        used_s = aligned_end;
    }

    let row_count = footer.offsets.len();
    let active_count: usize = pdb_page::footer_groups(page_size, row_count)
        .map(|group| (footer.rowpf_by_group[group.index] & group.mask()).count_ones() as usize)
        .sum();
    pdb_page::write_slot_counts(page, row_count, active_count);
    page[0x1e..0x20].copy_from_slice(&(used_s as u16).to_le_bytes());

    let (u5, num_rl) = if table_type == 7 {
        // tt=7 (playlist_tree) convention: (nrs, 0). Use row_count (total slots including
        // tombstoned) as u5, always 0 for num_rl. Never preserve old_num_rl — the original
        // broken export wrote num_rl=1 and preserving it causes DJ software to reject.
        (row_count as u16, 0u16)
    } else {
        // Existing data pages carry transaction history in u5/num_rl/tranrf.
        // Preserve u5 unless the page is being populated from an empty state,
        // and avoid rewriting sentinel-style num_rl values observed on working
        // hardware-tested exports. For normal u5=1 pages, num_rl still tracks
        // the final row slot required by strict table validators such as tt=8.
        let (fresh_u5, fresh_num_rl) = data_page_footer_fields(table_type, row_count as u16);
        let u5 = if old_u5 == 0 { fresh_u5 } else { old_u5 };
        let num_rl = if old_num_rl == 0x1FFF {
            old_num_rl
        } else if u5 == 1 {
            row_count.saturating_sub(1) as u16
        } else {
            fresh_num_rl
        };
        (u5, num_rl)
    };
    page[0x20..0x22].copy_from_slice(&u5.to_le_bytes());
    page[0x22..0x24].copy_from_slice(&num_rl.to_le_bytes());

    // Keep existing page_flags. Pages must not be resealed or normalized
    // during additive export.

    let footer_size = footer_size(row_count);
    let footer_start = page_size - footer_size;
    if HEAP_START + used_s <= footer_start {
        page[HEAP_START + used_s..footer_start].fill(0);
    }
    let free_s = page_size.saturating_sub(HEAP_START + used_s + footer_size);
    page[0x1c..0x1e].copy_from_slice(&(free_s as u16).to_le_bytes());

    // Existing tranrf groups are validated transaction history on player hardware
    // family players. Preserve them and OR in only the appended row bits.
    let offsets: Vec<u16> = footer.offsets.iter().map(|&offset| offset as u16).collect();
    pdb_page::write_footer(page, &offsets, |group| {
        (
            footer.rowpf_by_group[group.index],
            footer.tranrf_by_group[group.index],
        )
    });

    Ok(absorbed)
}

fn append_rows_to_t07_tombstone_pages_in_place(
    bytes: &mut [u8],
    rows: &[Vec<u8>],
    page_size: usize,
) -> BackendResult<(usize, usize)> {
    if rows.is_empty() {
        return Ok((0, 0));
    }
    let (_ec, first, last) = table_ptr_fields(bytes, 7).ok_or_else(|| {
        BackendError::Validation("t07 tombstone append: table pointer missing".into())
    })?;
    let chain = collect_chain_pages(bytes, page_size, first, last).ok_or_else(|| {
        BackendError::Validation(format!(
            "t07 tombstone append: chain unreachable from first={first} last={last}"
        ))
    })?;

    let mut cursor = 0usize;
    let mut pages_reused = 0usize;
    for &page_idx in chain.iter().skip(1) {
        if cursor >= rows.len() {
            break;
        }
        let off = match page_offset(page_idx, page_size) {
            Some(off) => off,
            None => continue,
        };
        let page_snapshot = bytes
            .get(off..off + page_size)
            .ok_or_else(|| {
                BackendError::Validation(format!(
                    "t07 tombstone append: page {page_idx} out of bounds"
                ))
            })?
            .to_vec();
        if !page_has_transaction_tombstones(&page_snapshot, page_size) {
            continue;
        }
        let absorbed = append_rows_to_existing_page_preserving_footer_state(
            bytes,
            7,
            page_idx,
            &rows[cursor..],
            page_size,
        )?;
        if absorbed == 0 {
            continue;
        }
        pages_reused += 1;
        cursor += absorbed;
    }

    Ok((pages_reused, cursor))
}

/// Append `new_rows` to an existing table's chain in place, mutating
/// `bytes` directly.
///
/// Walks the chain from `first` to `last` (skipping the sentinel/index
/// page at `first` itself), absorbing rows into existing pages with free
/// heap space first. When existing capacity runs out, appends fresh pages
/// at the file tail and links them to the chain.
///
/// On success returns an `AppendOutcome` describing how the chain grew.
/// Returns `Err(BackendError::Validation)` only when a single row's
/// encoded length exceeds `MAX_ROW_LEN` (which would be unfittable on any
/// page); the caller should reject such rows before invoking this
/// function.
pub(crate) fn append_rows_to_chain_in_place(
    bytes: &mut Vec<u8>,
    table_type: u32,
    new_rows: &[Vec<u8>],
    page_size: usize,
) -> BackendResult<AppendOutcome> {
    if page_size != PAGE_SIZE {
        return Err(BackendError::Validation(format!(
            "additive append requires page_size {PAGE_SIZE}, got {page_size}"
        )));
    }
    if new_rows.is_empty() {
        let (ec, _first, last) = table_ptr_fields(bytes, table_type).ok_or_else(|| {
            BackendError::Validation(format!(
                "additive append: table {table_type} pointer missing from header"
            ))
        })?;
        return Ok(AppendOutcome {
            pages_reused: 0,
            pages_appended: 0,
            new_last_page: last,
            new_empty_candidate: ec,
        });
    }

    // Up-front capacity gate: a single row > MAX_ROW_LEN cannot go on
    // any page, fresh or existing. Rejecting here keeps the in-place
    // path total — once we start writing, we never have to abort.
    for (idx, row) in new_rows.iter().enumerate() {
        if row.len() > MAX_ROW_LEN {
            return Err(BackendError::Validation(format!(
                "additive append: row {idx} for table {table_type} exceeds page capacity \
                 ({} bytes; max {MAX_ROW_LEN})",
                row.len()
            )));
        }
    }

    let (old_ec, first, last) = table_ptr_fields(bytes, table_type).ok_or_else(|| {
        BackendError::Validation(format!(
            "additive append: table {table_type} pointer missing from header"
        ))
    })?;
    let chain = collect_chain_pages(bytes, page_size, first, last).ok_or_else(|| {
        BackendError::Validation(format!(
            "additive append: table {table_type} chain unreachable from first={first} last={last}"
        ))
    })?;

    let mut outcome = AppendOutcome {
        pages_reused: 0,
        pages_appended: 0,
        new_last_page: last,
        new_empty_candidate: 0, // set at the end
    };

    let mut cursor = 0usize; // index into new_rows

    // Phase 1: walk existing data pages (skip the sentinel at chain[0])
    // and absorb rows where heap capacity allows.
    for &page_idx in chain.iter().skip(1) {
        if cursor >= new_rows.len() {
            break;
        }
        let off = match page_offset(page_idx, page_size) {
            Some(o) => o,
            None => continue,
        };
        let page_view: Vec<u8> = bytes
            .get(off..off + page_size)
            .map(|s| s.to_vec())
            .ok_or_else(|| {
                BackendError::Validation(format!(
                    "additive append: page {page_idx} for table {table_type} \
                     out of bounds"
                ))
            })?;
        if page_has_transaction_tombstones(&page_view, page_size) {
            continue;
        }
        let absorbed = append_rows_to_existing_page_preserving_footer_state(
            bytes,
            table_type,
            page_idx,
            &new_rows[cursor..],
            page_size,
        )?;
        if absorbed > 0 {
            outcome.pages_reused += 1;
            cursor += absorbed;
        }
    }

    // Phase 2: when existing capacity is exhausted, append fresh pages
    // at the file tail. Each fresh page absorbs as many remaining rows
    // as fit. We link them on the fly: the previous tail's `next_page`
    // is patched to point at the first fresh page; subsequent fresh
    // pages link forward to the next fresh page.
    let total_pages_before = bytes.len() / page_size;
    let mut reusable_empty_candidate = if chain.len() == 1
        && old_ec != first
        && (old_ec as usize) < total_pages_before
        && page_offset(old_ec, page_size)
            .and_then(|off| bytes.get(off..off + page_size))
            .map(|page| page.iter().all(|b| *b == 0))
            .unwrap_or(false)
    {
        Some(old_ec)
    } else {
        None
    };
    let mut reused_physical_empty_candidate = false;
    let mut reused_baseline_idx: Option<u32> = None;
    let mut prev_tail = outcome.new_last_page;
    let mut pages_created_this_append = std::collections::HashSet::<u32>::new();
    while cursor < new_rows.len() {
        let (new_idx, appended_physical_page) = if let Some(idx) = reusable_empty_candidate.take() {
            reused_physical_empty_candidate = true;
            reused_baseline_idx = Some(idx);
            (idx, false)
        } else {
            // Allocate at max(physical_end, next_unused) so we don't overwrite
            // a page that another table's ec pointer is already pointing to.
            // next_unused in the header is incremented as each dict/artwork table
            // claims its virtual ec page; if we naively allocated at bytes.len()/page_size
            // we'd land in the middle of that reserved range and create a conflict.
            let physical_end = (bytes.len() / page_size) as u32;
            let next_u = read_u32_le_at(bytes, 0x0c).unwrap_or(physical_end);
            let idx = physical_end.max(next_u);
            let needed = (idx as usize + 1) * page_size;
            if bytes.len() < needed {
                bytes.resize(needed, 0u8);
            }
            (idx, true)
        };

        // Fit as many rows on this fresh page as possible.
        let mut fitted: Vec<Vec<u8>> = Vec::new();
        let mut local_used = 0usize;
        while cursor < new_rows.len() {
            let candidate = &new_rows[cursor];
            let aligned = align4(candidate.len());
            let new_count = fitted.len() + 1;
            let new_used = local_used + aligned;
            let new_footer = footer_size(new_count);
            if HEAP_START + new_used + new_footer > page_size || new_count > MAX_ROWS_PER_PAGE {
                break;
            }
            fitted.push(candidate.clone());
            local_used = new_used;
            cursor += 1;
        }

        // Defensive: the up-front MAX_ROW_LEN check guarantees at least
        // one row fits on a fresh page. If for some reason none did,
        // abort rather than loop forever.
        if fitted.is_empty() {
            return Err(BackendError::Validation(format!(
                "additive append: failed to place row {cursor} on a fresh page \
                 for table {table_type}"
            )));
        }

        for (slot, row) in fitted.iter_mut().enumerate() {
            apply_page_local_index_shift(table_type, row, slot);
        }

        // Initialize the fresh page header BEFORE writing rows so that
        // rewrite_variable_page_rows_in_place sees a recognizable page.
        //
        // Reference exporter convention for multi-page chains:
        //   - ALL pages must be SEAL (0x24), including the baseline page.
        //   - Only a single-page chain keeps ACTV (0x34) for tt=0/tt=19.
        // When adding to a chain that already has data pages, use SEAL so
        // the new page is consistent with the multi-page convention.
        let pf = if appended_physical_page || !chain.is_empty() {
            // Multi-page chain (reuse into populated chain, or true append):
            // all pages must be SEAL (0x24).
            PAGE_FLAGS_DATA
        } else {
            // Transitioning from empty → first data page: ACTV for tt=0/19.
            page_flags_for_table(table_type)
        };
        let new_off = (new_idx as usize) * page_size;
        bytes[new_off + 0x04..new_off + 0x08].copy_from_slice(&new_idx.to_le_bytes());
        bytes[new_off + 0x08..new_off + 0x0c].copy_from_slice(&table_type.to_le_bytes());
        bytes[new_off + 0x0c..new_off + 0x10].copy_from_slice(&0u32.to_le_bytes());
        // seq must be strictly greater than every existing data-page seq
        // so the new page is unambiguously the most-recent commit.
        let seq = max_seqpage_in_file(bytes, page_size)
            .saturating_add(1)
            .max(2);
        bytes[new_off + 0x10..new_off + 0x14].copy_from_slice(&seq.to_le_bytes());
        bytes[new_off + 0x1b] = pf;

        let ok = rewrite_variable_page_rows_in_place(bytes, new_idx, &fitted, page_size);
        if !ok {
            return Err(BackendError::Validation(format!(
                "additive append: rewrite_variable_page_rows_in_place rejected \
                 fresh page {new_idx} for table {table_type}"
            )));
        }
        // Apply the per-table footer convention over the all-bits footer
        // `rewrite_variable_page_rows_in_place` wrote. tt=19 never grows
        // through here (see `synthesize_t19_runtime_rows_in_place`), so only
        // its u5/num_rl are stamped, not the runtime-profile bits.
        let page = &mut bytes[new_off..new_off + page_size];
        if table_type == 19 {
            let (u5, num_rl) = data_page_footer_fields(19, fitted.len() as u16);
            page[0x20..0x22].copy_from_slice(&u5.to_le_bytes());
            page[0x22..0x24].copy_from_slice(&num_rl.to_le_bytes());
        } else {
            apply_page_footer_convention(page, table_type, fitted.len());
        }

        // Link the previous tail to this fresh page.
        if let Some(prev_off) = page_offset(prev_tail, page_size) {
            let _ = write_u32_le_at(bytes, prev_off + 0x0c, new_idx);
            // When the previous tail is the sentinel (first page), also
            // update 0x2c ("redundant next_page"). Sentinel pages written
            // for empty tables use magic 0x03FFFFFF at 0x2c; the first
            // real data-page append must flip it to the actual next_page
            // so firmware knows the table is populated.
            if prev_tail == first {
                let _ = write_u32_le_at(bytes, prev_off + 0x2c, new_idx);
            }
        }

        if appended_physical_page {
            outcome.pages_appended += 1;
        } else {
            outcome.pages_reused += 1;
        }
        outcome.new_last_page = new_idx;
        pages_created_this_append.insert(new_idx);
        prev_tail = new_idx;
    }

    // When this append grew the chain (pages_appended > 0), every data page that
    // preceded the new overflow page must be sealed (flags=0x24). Reference exports
    // confirm flags=0x24 on every page in multi-page tt=0 chains.
    //
    // Three cases:
    //   Case 1 — reused blank baseline AND overflow in same call: the reused page
    //            was written with ACTV (0x34) earlier in this loop; seal it now.
    //   Case 2 — overflow appended to a previously-single-page chain in a second
    //            additive call: chain[1] (the existing first data page) has ACTV
    //            (0x34) from when the chain was single-page; it must be sealed.
    //   Case 3 — overflow appended to a multi-page chain: the old tail page had
    //            ACTV (0x34) as the last page; it must be sealed now that it is
    //            no longer the tail.
    if outcome.pages_appended > 0 {
        // Case 1: reused blank baseline page in this same call.
        if let Some(base_idx) = reused_baseline_idx
            && let Some(base_off) = page_offset(base_idx, page_size)
            && bytes.get(base_off + 0x1b).copied() == Some(PAGE_FLAGS_DATA_TRACK)
        {
            bytes[base_off + 0x1b] = PAGE_FLAGS_DATA;
        }
        // Case 2: existing first data page that was single-page (ACTV) before this
        // call extended the chain with physical overflow pages.
        if chain.len() == 2
            && let Some(&first_data_idx) = chain.get(1)
            && let Some(fd_off) = page_offset(first_data_idx, page_size)
            && bytes.get(fd_off + 0x1b).copied() == Some(PAGE_FLAGS_DATA_TRACK)
        {
            bytes[fd_off + 0x1b] = PAGE_FLAGS_DATA;
        }
        // Case 3: the previous tail of a multi-page chain is now an overflow page.
        if chain.len() > 2
            && let Some(old_tail_off) = page_offset(last, page_size)
            && bytes.get(old_tail_off + 0x1b).copied() == Some(PAGE_FLAGS_DATA_TRACK)
        {
            bytes[old_tail_off + 0x1b] = PAGE_FLAGS_DATA;
        }
    }

    // Finalize the table pointer at the file header. `first` is unchanged
    // (topology-lock invariant). If the chain did not grow, keep the old
    // empty-candidate and tail next_page rather than normalizing them.
    let new_last = outcome.new_last_page;
    let total_pages = (bytes.len() / page_size) as u32;
    let chain_grew = new_last != last;
    let new_ec = if !chain_grew {
        old_ec
    } else if reused_physical_empty_candidate {
        read_u32_le_at(bytes, 0x0c).unwrap_or(0).max(total_pages)
    } else {
        new_last.saturating_add(1)
    };
    outcome.new_empty_candidate = new_ec;

    if chain_grew && let Some(last_off) = page_offset(new_last, page_size) {
        let _ = write_u32_le_at(bytes, last_off + 0x0c, new_ec);
    }

    if !set_table_ptr_fields(bytes, table_type, new_ec, first, new_last) {
        return Err(BackendError::Validation(format!(
            "additive append: set_table_ptr_fields rejected table {table_type}"
        )));
    }

    // Update the sentinel page's B-tree index to reflect the current data pages.
    // Entry format: page_index * 8 (sector address, 512-byte sectors).
    // DJ software validates that the B-tree covers all active data pages; for
    // multi-page tables an empty B-tree causes DJ software to reject as corrupted.
    //
    // Scan all pages by table type rather than following next_page chain pointers.
    // Chain pointers in old pages may be stale (pointing to pages repurposed for a
    // different table), so scanning by tt gives the definitive set of live pages.
    {
        if let Some(sent_off) = page_offset(first, page_size) {
            let total_phys = bytes.len() / page_size;
            let sentinel_tt = read_u32_le_at(bytes, sent_off + 8).unwrap_or(9999);
            // B-tree contains ONLY active (flags=0x34) pages. Settled/sealed pages
            // (0x24) are navigated via next_page chain pointers. Reference format:
            // one B-tree entry per table, pointing to the current write head.
            let mut data_pages: Vec<u32> = Vec::new();
            for j in 1..total_phys {
                if j * page_size == sent_off {
                    continue;
                } // skip sentinel itself
                let poff = j * page_size;
                let page_tt = read_u32_le_at(bytes, poff + 8).unwrap_or(9999);
                if page_tt != sentinel_tt {
                    continue;
                }
                let page_flags = read_u8_at(bytes, poff + 0x1b).unwrap_or(0);
                if page_flags != PAGE_FLAGS_DATA_TRACK {
                    continue;
                } // only active pages
                let used_s = read_u16_le_at(bytes, poff + 0x1e).unwrap_or(0);
                if used_s == 0 {
                    continue;
                } // skip empty pages
                data_pages.push(j as u32);
            }
            data_pages.sort_unstable();
            if let Some(page_size_u) = page_size.checked_sub(SENTINEL_TAIL_ZERO_BYTES) {
                let fill_end = sent_off + page_size_u;
                let mut cursor = sent_off + 0x3c;
                for &page_idx in &data_pages {
                    if cursor + 4 > fill_end {
                        break;
                    }
                    let ev = page_idx * 8;
                    if let Some(sl) = bytes.get_mut(cursor..cursor + 4) {
                        sl.copy_from_slice(&ev.to_le_bytes());
                    }
                    cursor += 4;
                }
                // Clear remaining slots
                while cursor + 4 <= fill_end {
                    if let Some(sl) = bytes.get_mut(cursor..cursor + 4) {
                        sl.copy_from_slice(&SENTINEL_EMPTY_ENTRY.to_le_bytes());
                    }
                    cursor += 4;
                }
            }
            let ne = (data_pages.len().min(0x1FFF)) as u16;
            if let Some(sl) = bytes.get_mut(sent_off + 0x38..sent_off + 0x3a) {
                sl.copy_from_slice(&ne.to_le_bytes());
            }
            if let Some(sl) = bytes.get_mut(sent_off + 0x3a..sent_off + 0x3c) {
                sl.copy_from_slice(&0x1fffu16.to_le_bytes());
            }
            if let Some(sl) = bytes.get_mut(sent_off + 0x26..sent_off + 0x28) {
                sl.copy_from_slice(&ne.to_le_bytes());
            }
        }
    }

    // Bump the global next_unused_page (file header offset 0x0c) so it
    // remains strictly greater than every empty_candidate in the file.
    let nextu = read_u32_le_at(bytes, 0x0c)
        .unwrap_or(0)
        .max(total_pages)
        .max(new_ec.saturating_add(1));
    let _ = write_u32_le_at(bytes, 0x0c, nextu);

    // Bump seqdb (file header offset 0x14) so it remains strictly
    // greater than every seqpage in the file.
    let max_seq = max_seqpage_in_file(bytes, page_size);
    let seqdb = read_u32_le_at(bytes, 0x14)
        .unwrap_or(0)
        .max(max_seq.saturating_add(1));
    let _ = write_u32_le_at(bytes, 0x14, seqdb);

    Ok(outcome)
}

// ── Diff classifier ─────────────────────────────────────────────────────────
//
// Decides whether the transition from an existing on-disk PDB to a new
// in-memory PdbData can be represented without rebuilding table topology.
// Accepted operations are row appends, target-playlist t08 removals, and
// bounded t00 metadata mutations. Any other shape is surfaced as an additive
// decline; production export treats that as a hard validation error.
//
// "Purely additive" means:
//
// - Every track id present in the existing PDB is also present in the
//   new PdbData. (Deletion would require row removal.)
// - Existing t00 rows may mutate known scalar fields and mapped string slots.
//   Same-size rows are patched directly; length-changing rows tombstone the old
//   active row and append a replacement with the same track id.
// - Every existing artist / album / genre / label / key / artwork id is
//   still present and its string is byte-identical.
// - Every existing playlist_tree id is still present with identical
//   parent_id / is_folder / name. `sort_order` may be patched in place
//   because the DJ software moves freshly exported playlists to the front of their
//   sibling list without rewriting the whole table.
// - t08 playlist_entries are represented as target-playlist removals plus
//   appended rows.
// - PDB menu tables t16/t17/t18 are never edited by normal export.

#[derive(Debug, Default)]
pub(crate) struct PdbAdditiveDiff {
    pub new_tracks: Vec<PdbTrackRowData>,
    pub mutated_tracks: Vec<PdbTrackRowMutation>,
    pub new_artists: Vec<(u32, String)>,
    pub new_albums: Vec<(u32, String, u32)>,
    pub new_genres: Vec<(u32, String)>,
    pub new_labels: Vec<(u32, String)>,
    pub new_keys: Vec<(u32, String)>,
    pub new_artwork: Vec<(u32, String)>,
    pub playlist_tree_sort_order_patches: Vec<PdbPlaylistTreeSortOrderPatch>,
    pub new_playlist_tree: Vec<(u32, u32, u32, bool, String)>,
    /// Desired final t08 entries; passed through to the existing in-place
    /// patch helpers, which figure out what to add and where.
    pub desired_t08_entries: Vec<T08EntryKey>,
    /// t08 entries whose `(playlist_id, track_id)` identity is no longer
    /// desired at all — to be removed in-place. Does NOT include entries
    /// that are still desired but at a different `entry_index`; those are
    /// `repositioned_t08_entries` instead, so a position change is never a
    /// remove+add pair. Non-empty when mirror-mode export shrinks an
    /// existing playlist, or an additive export drops a track from it.
    pub removed_t08_entries: Vec<T08EntryKey>,
    /// Existing `(playlist_id, track_id)` entries whose desired
    /// `entry_index` differs from what's currently on disk — patched in
    /// place via `patch_t08_entry_indices_in_place` rather than deleted and
    /// re-added, so a playlist whose track order changes can never
    /// accumulate duplicate rows across repeated exports.
    /// `(playlist_id, track_id, new_entry_index)`.
    pub repositioned_t08_entries: Vec<(u32, u32, u32)>,
    /// The export has tracks but tt=19 is still in template state, so its
    /// page gets activated (see [`activate_t19_template_page`]).
    pub activate_t19_template: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PdbTrackRowMutation {
    pub row: PdbTrackRowData,
    pub changed_fields: Vec<&'static str>,
}

#[derive(Debug, Clone)]
pub(crate) struct PdbPlaylistTreeSortOrderPatch {
    pub id: u32,
    pub sort_order: u32,
}

/// Reason a diff was rejected as non-additive. Returned for diagnostics
/// only — callers that don't care can collapse to `Option<diff>`.
#[derive(Debug, Clone)]
pub(crate) enum NonAdditiveReason {
    TrackIdRemoved(u32),
    DictionaryStringMutated {
        table: &'static str,
        id: u32,
        old: String,
        new: String,
    },
    DictionaryIdRemoved {
        table: &'static str,
        id: u32,
    },
    PlaylistTreeRowMutated {
        id: u32,
    },
    PlaylistTreeIdRemoved {
        id: u32,
    },
    Unparseable(String),
}

impl std::fmt::Display for NonAdditiveReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TrackIdRemoved(id) => write!(f, "track id {id} would be removed"),
            Self::DictionaryStringMutated {
                table,
                id,
                old,
                new,
            } => write!(f, "{table} id {id} renamed: {old:?} -> {new:?}"),
            Self::DictionaryIdRemoved { table, id } => {
                write!(f, "{table} id {id} would be removed")
            }
            Self::PlaylistTreeRowMutated { id } => {
                write!(f, "playlist_tree id {id} mutated (parent/folder/name)")
            }
            Self::PlaylistTreeIdRemoved { id } => {
                write!(f, "playlist_tree id {id} would be removed")
            }
            Self::Unparseable(msg) => write!(f, "PDB unparseable: {msg}"),
        }
    }
}

/// Classify the existing-vs-new transition. Returns `Ok(diff)` when the
/// transition is purely additive, `Err(reason)` otherwise. Callers map
/// `Err` to `None` under `auto`, or surface it under `locked`.
pub(crate) fn compute_additive_diff(
    existing_bytes: &[u8],
    new_data: &PdbData,
    desired_t08_entries: Vec<T08EntryKey>,
) -> Result<PdbAdditiveDiff, NonAdditiveReason> {
    let parsed = crate::pdb_reader::parse_pdb_bytes(existing_bytes)
        .map_err(|e| NonAdditiveReason::Unparseable(e.to_string()))?;

    // Older players count 0 songs while tt=19's page is still the init template
    // (one sealed row, flags=0x24, u5=1), however many tracks t00 holds.
    // rekordbox's own exports show that sealed single row is the
    // empty-device state, not a finished page: on its first export
    // rekordbox keeps the template row, appends one, and turns the page
    // active (a 1-track reference export: nrs=2, flags=0x34, u5=2,
    // rowpf=0x0002, tranrf=0x0003). So any export with tracks activates a
    // template-state page -- including one onto a PDB that already has
    // tracks, so a USB an older build left at 0 songs recovers on its next
    // export. A page rekordbox or a player has already advanced is not
    // template state and is left alone.
    let activate_t19_template = !new_data.tracks.is_empty()
        && t19_is_template_state(
            existing_bytes,
            read_u32_le_at(existing_bytes, 4).unwrap_or(0) as usize,
        );

    // ── Tracks ──────────────────────────────────────────────────────
    let new_track_ids: std::collections::HashSet<u32> =
        new_data.tracks.iter().map(|t| t.id).collect();
    for existing in &parsed.tracks {
        if !new_track_ids.contains(&existing.id) {
            return Err(NonAdditiveReason::TrackIdRemoved(existing.id));
        }
    }

    // For every existing track id, collect bounded row mutations. The
    // apply step patches known scalar offsets and same-length string slots
    // directly in t00, preserving the page footer and any inactive rows.
    let new_tracks_by_id: std::collections::HashMap<u32, &PdbTrackRowData> =
        new_data.tracks.iter().map(|t| (t.id, t)).collect();
    let mut mutated_tracks = Vec::<PdbTrackRowMutation>::new();
    for existing in &parsed.tracks {
        if let Some(new_row) = new_tracks_by_id.get(&existing.id) {
            let mut changed_fields = Vec::<&'static str>::new();
            if existing.artist_id != new_row.artist_id {
                changed_fields.push("artist_id");
            }
            if existing.album_id != new_row.album_id {
                changed_fields.push("album_id");
            }
            if existing.artwork_id != new_row.artwork_id {
                changed_fields.push("artwork_id");
            }
            if existing.key_id != new_row.key_id {
                changed_fields.push("key_id");
            }
            if existing.genre_id != new_row.genre_id {
                changed_fields.push("genre_id");
            }
            if existing.content_link.unwrap_or(0) != new_row.content_link.unwrap_or(0) {
                changed_fields.push("content_link");
            }
            if existing.sample_rate_hz.unwrap_or(0) != new_row.sample_rate_hz.unwrap_or(0) {
                changed_fields.push("sample_rate_hz");
            }
            if existing.file_size_bytes.unwrap_or(0) != new_row.file_size_bytes.unwrap_or(0) {
                changed_fields.push("file_size_bytes");
            }
            if existing.master_content_id.unwrap_or(0) != new_row.master_content_id.unwrap_or(0) {
                changed_fields.push("master_content_id");
            }
            if existing.master_db_id.unwrap_or(0) != new_row.master_db_id.unwrap_or(0) {
                changed_fields.push("master_db_id");
            }
            if existing.bitrate_kbps.unwrap_or(0) != new_row.bitrate_kbps.unwrap_or(0) {
                changed_fields.push("bitrate_kbps");
            }
            if existing.track_number != new_row.track_number.unwrap_or(0) {
                changed_fields.push("track_number");
            }
            let new_tempo_x100 = new_row
                .bpm
                .map(|v| (v * 100.0).round().max(0.0) as u32)
                .unwrap_or(0);
            if existing.tempo_x100 != new_tempo_x100 {
                changed_fields.push("tempo_x100");
            }
            if existing.release_year.unwrap_or(0) != new_row.release_year.unwrap_or(0) {
                changed_fields.push("release_year");
            }
            if existing.bit_depth.unwrap_or(0) != new_row.bit_depth.unwrap_or(0) {
                changed_fields.push("bit_depth");
            }
            let new_duration_seconds = new_row.duration_seconds.unwrap_or(0).min(u16::MAX as u32);
            if existing.duration_seconds.unwrap_or(0) != new_duration_seconds {
                changed_fields.push("duration_seconds");
            }
            if existing.file_type.unwrap_or(0) != new_row.file_type.unwrap_or(0) {
                changed_fields.push("file_type");
            }
            if existing.color_id != new_row.color_id.unwrap_or(0) {
                changed_fields.push("color_id");
            }
            if existing.rating != new_row.rating.unwrap_or(0).min(5) {
                changed_fields.push("rating");
            }
            let new_title = sanitize_metadata(&new_row.title);
            if existing.title != new_title.as_ref() {
                changed_fields.push("title");
            }
            if existing.anlz_path != new_row.anlz_path {
                changed_fields.push("anlz_path");
            }
            if existing.track_file_path != new_row.file_path {
                changed_fields.push("file_path");
            }
            if existing.isrc.as_deref().unwrap_or("") != new_row.isrc.as_deref().unwrap_or("") {
                changed_fields.push("isrc");
            }
            if existing.date_added.as_deref().unwrap_or("")
                != new_row.date_added.as_deref().unwrap_or("")
            {
                changed_fields.push("date_added");
            }
            if existing.release_date.as_deref().unwrap_or("")
                != new_row.release_date.as_deref().unwrap_or("")
            {
                changed_fields.push("release_date");
            }
            let new_dj_comment = sanitize_metadata(new_row.dj_comment.as_deref().unwrap_or(""));
            if existing.dj_comment.as_deref().unwrap_or("") != new_dj_comment.as_ref() {
                changed_fields.push("dj_comment");
            }
            let new_file_name =
                sanitize_metadata(new_row.file_name.as_deref().unwrap_or(&new_row.file_path));
            if existing.file_name.as_deref().unwrap_or("") != new_file_name.as_ref() {
                changed_fields.push("file_name");
            }
            let existing_publish = existing
                .publish_track_info
                .as_deref()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("on"))
                .unwrap_or(false);
            if existing_publish != new_row.publish_track_info_on.unwrap_or(false) {
                changed_fields.push("publish_track_info_on");
            }
            let existing_autoload = existing
                .autoload_hotcues
                .as_deref()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("on"))
                .unwrap_or(false);
            if existing_autoload != new_row.autoload_hotcues_on.unwrap_or(false) {
                changed_fields.push("autoload_hotcues_on");
            }
            if !changed_fields.is_empty() {
                mutated_tracks.push(PdbTrackRowMutation {
                    row: (**new_row).clone(),
                    changed_fields,
                });
            }
        }
    }

    let existing_track_ids: std::collections::HashSet<u32> =
        parsed.tracks.iter().map(|t| t.id).collect();
    let new_tracks: Vec<PdbTrackRowData> = new_data
        .tracks
        .iter()
        .filter(|t| !existing_track_ids.contains(&t.id))
        .cloned()
        .collect();

    // ── Dictionary tables (genres, labels, keys, artists, artworks) ─
    fn check_dict_map(
        table: &'static str,
        existing: &std::collections::HashMap<u32, String>,
        new: impl Iterator<Item = (u32, String)>,
    ) -> Result<Vec<(u32, String)>, NonAdditiveReason> {
        let new_pairs: Vec<(u32, String)> = new.collect();
        let new_map: std::collections::HashMap<u32, &str> =
            new_pairs.iter().map(|(id, n)| (*id, n.as_str())).collect();
        for (id, existing_name) in existing {
            match new_map.get(id) {
                None => return Err(NonAdditiveReason::DictionaryIdRemoved { table, id: *id }),
                Some(new_name) if *new_name != existing_name.as_str() => {
                    return Err(NonAdditiveReason::DictionaryStringMutated {
                        table,
                        id: *id,
                        old: existing_name.clone(),
                        new: new_name.to_string(),
                    });
                }
                _ => {}
            }
        }
        let added: Vec<(u32, String)> = new_pairs
            .into_iter()
            .filter(|(id, _)| !existing.contains_key(id))
            .collect();
        Ok(added)
    }

    let new_artists = check_dict_map(
        "artists",
        &parsed.artists,
        new_data.artists.iter().map(|r| (r.id, r.name.clone())),
    )?;
    let new_genres = check_dict_map(
        "genres",
        &parsed.genres,
        new_data.genres.iter().map(|r| (r.id, r.name.clone())),
    )?;
    let new_labels = check_dict_map(
        "labels",
        &parsed.labels,
        new_data.labels.iter().map(|r| (r.id, r.name.clone())),
    )?;
    let new_keys = check_dict_map(
        "keys",
        &parsed.keys,
        new_data.keys.iter().map(|r| (r.id, r.name.clone())),
    )?;
    let new_artwork = check_dict_map(
        "artwork",
        &parsed.artworks,
        new_data.artwork.iter().map(|r| (r.id, r.path.clone())),
    )?;

    // ── Albums (id -> (name, artist_id)) ────────────────────────────
    let mut existing_album_artist: std::collections::HashMap<u32, u32> =
        std::collections::HashMap::new();
    // The reader's `albums` map only carries id -> name. Re-derive the
    // artist linkage by walking the PdbData's tracks (which carry the
    // resolved ids) is wrong here because we need the *existing* album
    // -> artist mapping. The existing PDB's albums always have an
    // associated artist_id baked into the album row; the reader does
    // not currently surface it. For the strict-additive check we treat
    // the absence of the linkage as "unknown" and only validate name
    // immutability + id presence. If the linkage ever needs strict
    // validation, the reader needs to expose album.artist_id.
    let _ = &mut existing_album_artist;
    let new_album_pairs: Vec<(u32, String, u32)> = new_data
        .albums
        .iter()
        .map(|r| (r.id, r.name.clone(), r.artist_id))
        .collect();
    let new_album_map: std::collections::HashMap<u32, (&str, u32)> = new_album_pairs
        .iter()
        .map(|(id, n, a)| (*id, (n.as_str(), *a)))
        .collect();
    for (id, existing_name) in &parsed.albums {
        match new_album_map.get(id) {
            None => {
                return Err(NonAdditiveReason::DictionaryIdRemoved {
                    table: "albums",
                    id: *id,
                });
            }
            Some((new_name, _)) if *new_name != existing_name.as_str() => {
                return Err(NonAdditiveReason::DictionaryStringMutated {
                    table: "albums",
                    id: *id,
                    old: existing_name.clone(),
                    new: new_name.to_string(),
                });
            }
            _ => {}
        }
    }
    let new_albums: Vec<(u32, String, u32)> = new_album_pairs
        .into_iter()
        .filter(|(id, _, _)| !parsed.albums.contains_key(id))
        .collect();

    // ── Playlist tree ────────────────────────────────────────────────
    let existing_pt: std::collections::HashMap<u32, &crate::pdb_reader::PdbPlaylistTreeRow> =
        parsed.playlist_tree.iter().map(|r| (r.id, r)).collect();
    for new_row in &new_data.playlist_tree {
        if let Some(existing) = existing_pt.get(&new_row.id) {
            // Existing playlist identity must remain stable. Only sort_order
            // is allowed to change, because that is a fixed-width field and
            // Used to put the exported playlist first.
            if existing.parent_id != new_row.parent_id
                || existing.row_is_folder != new_row.is_folder
                || existing.name != new_row.name
            {
                return Err(NonAdditiveReason::PlaylistTreeRowMutated { id: new_row.id });
            }
        }
    }
    let new_pt_ids: std::collections::HashSet<u32> =
        new_data.playlist_tree.iter().map(|r| r.id).collect();
    for existing in &parsed.playlist_tree {
        if !new_pt_ids.contains(&existing.id) {
            return Err(NonAdditiveReason::PlaylistTreeIdRemoved { id: existing.id });
        }
    }
    let new_playlist_tree: Vec<(u32, u32, u32, bool, String)> = new_data
        .playlist_tree
        .iter()
        .filter(|r| !existing_pt.contains_key(&r.id))
        .map(|r| (r.id, r.parent_id, r.sort_order, r.is_folder, r.name.clone()))
        .collect();
    let playlist_tree_sort_order_patches: Vec<PdbPlaylistTreeSortOrderPatch> = new_data
        .playlist_tree
        .iter()
        .filter_map(|new_row| {
            let existing = existing_pt.get(&new_row.id)?;
            (existing.sort_order != new_row.sort_order).then_some(PdbPlaylistTreeSortOrderPatch {
                id: new_row.id,
                sort_order: new_row.sort_order,
            })
        })
        .collect();

    // ── t08 deletion + reposition check ─────────────────────────────
    //
    // `desired_t08_entries` describes the FINAL desired set for the
    // target playlist. An existing entry is matched against it by
    // `(playlist_id, track_id)` identity only -- `entry_index` is a
    // position, not part of identity (see `T08EntryKey`'s doc comment and
    // `patch_t08_entry_indices_in_place`). This keeps a track whose order
    // changed from ever being computed as a remove+add pair: it is either
    // unchanged, or its position is patched in place, never both removed
    // and re-added.
    let (removed_t08_entries, repositioned_t08_entries): (Vec<T08EntryKey>, Vec<(u32, u32, u32)>) =
        if !desired_t08_entries.is_empty() {
            let target_playlist_id = desired_t08_entries[0].playlist_id;
            let desired_by_identity: std::collections::HashMap<(u32, u32), u32> =
                desired_t08_entries
                    .iter()
                    .map(|k| ((k.playlist_id, k.track_id), k.entry_index))
                    .collect();
            let existing_t08 = collect_t08_entry_keys(existing_bytes, 4096);
            let mut removed = Vec::new();
            let mut repositioned = Vec::new();
            for k in existing_t08 {
                if k.playlist_id != target_playlist_id {
                    continue;
                }
                match desired_by_identity.get(&(k.playlist_id, k.track_id)) {
                    None => removed.push(k),
                    Some(&new_entry_index) if new_entry_index != k.entry_index => {
                        repositioned.push((k.playlist_id, k.track_id, new_entry_index));
                    }
                    _ => {}
                }
            }
            (removed, repositioned)
        } else {
            (Vec::new(), Vec::new())
        };

    Ok(PdbAdditiveDiff {
        new_tracks,
        mutated_tracks,
        new_artists,
        new_albums,
        new_genres,
        new_labels,
        new_keys,
        new_artwork,
        playlist_tree_sort_order_patches,
        new_playlist_tree,
        desired_t08_entries,
        removed_t08_entries,
        repositioned_t08_entries,
        activate_t19_template,
    })
}

// ── In-place removal helpers ────────────────────────────────────────────────

/// Remove t08 playlist_entries in place.
///
/// Walks every data page in the t08 chain; for each page that contains a
/// row whose decoded key is in `victims`, rewrites the page without that
/// row. Chain structure (next_page pointers, sentinel page) is preserved.
/// `last_page` in the table pointer steps back when the tail data page(s)
/// become empty after removal.
pub(crate) fn remove_t08_entries_in_place(
    bytes: &mut [u8],
    page_size: usize,
    victims: &[T08EntryKey],
) -> BackendResult<()> {
    if victims.is_empty() {
        return Ok(());
    }
    let victim_set: std::collections::HashSet<T08EntryKey> = victims.iter().copied().collect();

    let (_ec, first, last) = table_ptr_fields(bytes, 8).ok_or_else(|| {
        BackendError::Validation("remove_t08_entries: table 8 pointer missing".into())
    })?;
    let chain = collect_chain_pages(bytes, page_size, first, last).ok_or_else(|| {
        BackendError::Validation("remove_t08_entries: t08 chain unreachable".into())
    })?;

    let mut any_changed = false;

    for &page_idx in chain.iter().skip(1) {
        let Some(off) = page_offset(page_idx, page_size) else {
            continue;
        };
        let page_view: Vec<u8> = match bytes.get(off..off + page_size) {
            Some(s) => s.to_vec(),
            None => continue,
        };
        let present = read_present_page_rows(&page_view, page_size);
        let kept: Vec<Vec<u8>> = present
            .iter()
            .filter(|row| {
                if row.len() < 12 {
                    return true;
                }
                let k = T08EntryKey {
                    entry_index: u32::from_le_bytes([row[0], row[1], row[2], row[3]]),
                    track_id: u32::from_le_bytes([row[4], row[5], row[6], row[7]]),
                    playlist_id: u32::from_le_bytes([row[8], row[9], row[10], row[11]]),
                };
                !victim_set.contains(&k)
            })
            .cloned()
            .collect();

        if kept.len() == present.len() {
            continue;
        }
        any_changed = true;

        let ok = rewrite_variable_page_rows_in_place(bytes, page_idx, &kept, page_size);
        if !ok {
            return Err(BackendError::Validation(format!(
                "remove_t08_entries: rewrite failed for page {page_idx}"
            )));
        }
        // `rewrite_variable_page_rows_in_place` left `(u5, num_rl) = (trc,
        // trc-1)` and all-bits rowpf/tranrf in every footer group.  tt=8 is
        // a `u5=1` table, so re-stamp the full convention — not just
        // `(u5, num_rl)` — or a mirror-mode removal leaves every remaining
        // row falsely marked "active" in tranrf instead of just the last.
        if let Some(page) = bytes.get_mut(off..off + page_size) {
            apply_page_footer_convention(page, 8, kept.len());
        }
    }

    if !any_changed {
        return Ok(());
    }

    // Step last_page back past any tail pages that became empty.
    let new_last = chain
        .iter()
        .skip(1)
        .rev()
        .find(|&&page_idx| {
            page_offset(page_idx, page_size)
                .and_then(|off| bytes.get(off..off + page_size))
                .map(|p| !read_present_page_rows(p, page_size).is_empty())
                .unwrap_or(false)
        })
        .copied()
        .unwrap_or(first);

    if new_last != last {
        let new_ec = new_last.saturating_add(1);
        if !set_table_ptr_fields(bytes, 8, new_ec, first, new_last) {
            return Err(BackendError::Validation(
                "remove_t08_entries: set_table_ptr_fields failed".into(),
            ));
        }
    }

    Ok(())
}

/// Fixed width of a tt=19 (history_runtime) row.
pub(crate) const T19_ROW_LEN: usize = 40;

/// The single tt=19 row of an empty-device template (`initialize_usb`
/// seeds it; rekordbox's own template carries the same row with its own
/// date).
pub(crate) const T19_TEMPLATE_ROW: [u8; T19_ROW_LEN] = [
    0x80, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x17, 0x32, 0x30, 0x32,
    0x36, 0x2d, 0x30, 0x33, 0x2d, 0x30, 0x36, 0x19, 0x1e, 0x0b, 0x31, 0x30, 0x30, 0x30, 0x03, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// The tt=19 rows rekordbox leaves after its first export onto a template
/// whose single row is `row0`: that row kept, plus a copy of it with bytes
/// `0..4` = `0x0280 + idx * 0x0020_0000` and bytes `4..8` = `idx`. The
/// activation, its repair and the fresh writer all use this, so every path
/// produces the same page.
pub(crate) fn t19_rows_after_first_export(row0: &[u8; T19_ROW_LEN]) -> [Vec<u8>; 2] {
    let mut row1 = row0.to_vec();
    let idx = 1u32;
    let val = 0x0000_0280u32.saturating_add(idx.saturating_mul(0x0020_0000));
    row1[0..4].copy_from_slice(&val.to_le_bytes());
    row1[4..8].copy_from_slice(&idx.to_le_bytes());
    [row0.to_vec(), row1]
}

/// First data page of tt=19 (`chain[1]`, after the sentinel), if the table
/// has one.
fn t19_first_data_page(bytes: &[u8], page_size: usize) -> Option<u32> {
    let (_ec, first, last) = table_ptr_fields(bytes, 19)?;
    let chain = collect_chain_pages(bytes, page_size, first, last)?;
    chain.get(1).copied()
}

/// True when tt=19's first data page is still the untouched init template:
/// one sealed row (`nrs=1`, flags `0x24`, `u5=1`, group-0 `rowpf=0x0001`).
/// Both rekordbox's own empty-device template and the one
/// `initialize_usb` writes have this shape. Older players count 0 songs
/// while the page stays like this, whatever t00 holds. Any other shape (a page
/// rekordbox or a player has already advanced) is not template state.
pub(crate) fn t19_is_template_state(bytes: &[u8], page_size: usize) -> bool {
    if page_size < HEAP_START {
        return false;
    }
    let Some(page_idx) = t19_first_data_page(bytes, page_size) else {
        return false;
    };
    let Some(off) = page_offset(page_idx, page_size) else {
        return false;
    };
    let Some(page) = bytes.get(off..off + page_size) else {
        return false;
    };
    if pdb_page::read_slot_counts(page).map(|(nro, _)| nro) != Some(1)
        || page[0x1b] != 0x24
        || read_u16_le_at(page, 0x20) != Some(1)
    {
        return false;
    }
    pdb_page::footer_groups(page_size, 1)
        .next()
        .is_some_and(|group| group.rowpf(page) == 0x0001)
}

/// Turn a template-state tt=19 page (see [`t19_is_template_state`]) into
/// the active shape rekordbox writes on its first export: the template row
/// is kept and one row is appended, a copy of it with bytes `0..4` =
/// `0x0280 + idx * 0x0020_0000` and bytes `4..8` = `idx`. The page then
/// turns active (flags `0x34`, `u5=2`, `rowpf=0x0002`, `tranrf=0x0003`),
/// byte-for-byte what rekordbox wrote on a 1-track reference export apart
/// from seq and the row date. The tt=19 sentinel's B-tree then indexes the
/// active page, as rekordbox's does. Touches no other page.
pub(crate) fn activate_t19_template_page(bytes: &mut [u8], page_size: usize) -> BackendResult<()> {
    if page_size != PAGE_SIZE {
        return Err(BackendError::Validation(format!(
            "t19 activation requires page_size {PAGE_SIZE}, got {page_size}"
        )));
    }
    // KNOWN GAP: a tt=19 with no data page yet (chain is just the sentinel)
    // would need a page allocated, which this in-place `&mut [u8]` writer
    // cannot do. Not reachable in production: the `initialize_usb` template
    // always pre-seeds one tt=19 data page (`seed_rb_initialized_history_shape`
    // in usb_utils.rs). Tests must mirror that pre-seeded page.
    let page_idx = t19_first_data_page(bytes, page_size).ok_or_else(|| {
        BackendError::Validation("t19 activation: table 19 has no data page".into())
    })?;
    let off = page_offset(page_idx, page_size).ok_or_else(|| {
        BackendError::Validation(format!("t19 activation: page {page_idx} out of bounds"))
    })?;

    let row0 = {
        let page = bytes.get(off..off + page_size).ok_or_else(|| {
            BackendError::Validation(format!("t19 activation: page {page_idx} out of bounds"))
        })?;
        let row_off = pdb_page::footer_groups(page_size, 1)
            .next()
            .map(|group| HEAP_START + group.row_offset(page, 0))
            .unwrap_or(HEAP_START);
        let row: &[u8; T19_ROW_LEN] = page
            .get(row_off..row_off + T19_ROW_LEN)
            .and_then(|row| row.try_into().ok())
            .ok_or_else(|| {
                BackendError::Validation("t19 activation: template row out of bounds".into())
            })?;
        *row
    };
    let rows = t19_rows_after_first_export(&row0);

    if !rewrite_variable_page_rows_in_place(bytes, page_idx, &rows, page_size) {
        return Err(BackendError::Validation(format!(
            "t19 activation: rows do not fit on page {page_idx}"
        )));
    }

    let seq = max_seqpage_in_file(bytes, page_size)
        .saturating_add(1)
        .max(2);
    bytes[off + 0x10..off + 0x14].copy_from_slice(&seq.to_le_bytes());
    bytes[off + 0x18] = rows.len() as u8;
    bytes[off + 0x19] = 0x20;
    bytes[off + 0x1a] = 0;
    bytes[off + 0x1b] = page_flags_for_table(19);
    if let Some(page) = bytes.get_mut(off..off + page_size) {
        apply_page_footer_convention(page, 19, rows.len());
    }
    rebuild_sentinel_btrees_for_tables(bytes, &[19]);

    // seqdb must stay above every seqpage, the rebuilt sentinel's included.
    let seqdb = read_u32_le_at(bytes, 0x14)
        .unwrap_or(0)
        .max(max_seqpage_in_file(bytes, page_size).saturating_add(1));
    let _ = write_u32_le_at(bytes, 0x14, seqdb);

    Ok(())
}

// ── In-place t00 row mutation helpers ────────────────────────────────────────

fn field_changed(fields: &std::collections::HashSet<&'static str>, name: &'static str) -> bool {
    fields.contains(name)
}

fn patch_changed_string_slot(
    row: &mut [u8],
    fields: &std::collections::HashSet<&'static str>,
    field_name: &'static str,
    slot_index: usize,
    desired: Vec<u8>,
) -> bool {
    if !field_changed(fields, field_name) {
        return true;
    }
    let _ = field_name;
    patch_track_row_slot_bytes_if_len_matches(row, slot_index, &desired)
}

/// True if any of the row's 21 string slots is UTF-16 encoded (`0x90` marker) but starts at a
/// row-relative offset that isn't 4-byte aligned. MIPS-based player hardware requires 4-byte
/// aligned reads for that header; a misaligned slot froze/comm-errored CDJ hardware while
/// browsing (see `encode_track_row_with_profile`'s alignment padding, added to fix this at
/// write time). A same-size in-place patch never touches the offset table, so it can't fix an
/// already-misaligned slot — it would silently leave the row broken.
pub(super) fn track_row_has_misaligned_utf16_slot(row: &[u8]) -> bool {
    for index in 0..21usize {
        let Some(offset) = read_u16_le_at(row, 94 + 2 * index).map(|v| v as usize) else {
            continue;
        };
        if row.get(offset) == Some(&0x90) && offset % 4 != 0 {
            return true;
        }
    }
    false
}

pub(super) fn build_same_size_track_row_patch(
    row: &[u8],
    mutation: &PdbTrackRowMutation,
) -> BackendResult<Option<Vec<u8>>> {
    if row.len() < 136 {
        return Err(BackendError::Validation(format!(
            "PDB additive track mutation blocked: t00 row {} is too short ({})",
            mutation.row.id,
            row.len()
        )));
    }

    if track_row_has_misaligned_utf16_slot(row) {
        // Force the caller down the relocate path (full re-encode + tombstone + append) instead
        // of patching content back into the same misaligned slot bounds.
        return Ok(None);
    }

    let mut patched = row.to_vec();
    let fields: std::collections::HashSet<&'static str> =
        mutation.changed_fields.iter().copied().collect();
    let desired = &mutation.row;

    if field_changed(&fields, "content_link") {
        patched[4..8].copy_from_slice(&desired.content_link.unwrap_or(0).to_le_bytes());
    }
    if field_changed(&fields, "sample_rate_hz") {
        patched[8..12].copy_from_slice(&desired.sample_rate_hz.unwrap_or(0).to_le_bytes());
    }
    if field_changed(&fields, "file_size_bytes") {
        patched[16..20].copy_from_slice(&desired.file_size_bytes.unwrap_or(0).to_le_bytes());
    }
    if field_changed(&fields, "master_content_id") {
        patched[20..24].copy_from_slice(&desired.master_content_id.unwrap_or(0).to_le_bytes());
    }
    if field_changed(&fields, "master_db_id") {
        patched[24..28].copy_from_slice(&desired.master_db_id.unwrap_or(0).to_le_bytes());
    }
    if field_changed(&fields, "artwork_id") {
        patched[28..32].copy_from_slice(&desired.artwork_id.to_le_bytes());
    }
    if field_changed(&fields, "key_id") {
        patched[32..36].copy_from_slice(&desired.key_id.to_le_bytes());
    }
    if field_changed(&fields, "bitrate_kbps") {
        patched[48..52].copy_from_slice(&desired.bitrate_kbps.unwrap_or(0).to_le_bytes());
    }
    if field_changed(&fields, "track_number") {
        patched[52..56].copy_from_slice(&desired.track_number.unwrap_or(0).to_le_bytes());
    }
    if field_changed(&fields, "tempo_x100") {
        let tempo_x100 = desired
            .bpm
            .map(|v| (v * 100.0).round().max(0.0) as u32)
            .unwrap_or(0);
        patched[56..60].copy_from_slice(&tempo_x100.to_le_bytes());
    }
    if field_changed(&fields, "genre_id") {
        patched[60..64].copy_from_slice(&desired.genre_id.to_le_bytes());
    }
    if field_changed(&fields, "album_id") {
        patched[64..68].copy_from_slice(&desired.album_id.to_le_bytes());
    }
    if field_changed(&fields, "artist_id") {
        patched[68..72].copy_from_slice(&desired.artist_id.to_le_bytes());
    }
    if field_changed(&fields, "release_year") {
        patched[80..82].copy_from_slice(&desired.release_year.unwrap_or(0).to_le_bytes());
    }
    if field_changed(&fields, "bit_depth") {
        patched[82..84].copy_from_slice(&desired.bit_depth.unwrap_or(0).to_le_bytes());
    }
    if field_changed(&fields, "duration_seconds") {
        let duration_seconds = desired.duration_seconds.unwrap_or(0).min(u16::MAX as u32) as u16;
        patched[84..86].copy_from_slice(&duration_seconds.to_le_bytes());
    }
    if field_changed(&fields, "file_type") {
        patched[90..92].copy_from_slice(&desired.file_type.unwrap_or(0).to_le_bytes());
    }
    if field_changed(&fields, "color_id") {
        patched[88] = desired.color_id.unwrap_or(0);
    }
    if field_changed(&fields, "rating") {
        patched[89] = desired.rating.unwrap_or(0).min(5);
    }

    use crate::service::export_helpers::pdb_encoding::{
        encode_pdb_track_inline_string, encode_pdb_track_isrc_slot, encode_pdb_track_path_string,
        encode_pdb_track_range_string,
    };
    let string_patches_ok = patch_changed_string_slot(
        &mut patched,
        &fields,
        "isrc",
        0,
        encode_pdb_track_isrc_slot(desired.isrc.as_deref()),
    ) && patch_changed_string_slot(
        &mut patched,
        &fields,
        "publish_track_info_on",
        6,
        encode_pdb_track_inline_string(if desired.publish_track_info_on == Some(true) {
            "ON"
        } else {
            ""
        }),
    ) && patch_changed_string_slot(
        &mut patched,
        &fields,
        "autoload_hotcues_on",
        7,
        encode_pdb_track_inline_string(if desired.autoload_hotcues_on == Some(true) {
            "ON"
        } else {
            ""
        }),
    ) && patch_changed_string_slot(
        &mut patched,
        &fields,
        "date_added",
        10,
        encode_pdb_track_inline_string(desired.date_added.as_deref().unwrap_or("")),
    ) && patch_changed_string_slot(
        &mut patched,
        &fields,
        "release_date",
        11,
        encode_pdb_track_inline_string(desired.release_date.as_deref().unwrap_or("")),
    ) && patch_changed_string_slot(
        &mut patched,
        &fields,
        "anlz_path",
        14,
        encode_pdb_track_range_string(&desired.anlz_path),
    ) && patch_changed_string_slot(
        &mut patched,
        &fields,
        "dj_comment",
        16,
        encode_pdb_track_inline_string(&sanitize_metadata(
            desired.dj_comment.as_deref().unwrap_or(""),
        )),
    ) && patch_changed_string_slot(
        &mut patched,
        &fields,
        "title",
        17,
        encode_pdb_track_range_string(&sanitize_metadata(&desired.title)),
    ) && {
        // Slot 19 (filename) is followed by alignment padding to keep slot 20 (UTF-16 path)
        // at a 4-byte aligned offset. Include that same padding in the desired bytes so the
        // same-size check passes.
        let raw = encode_pdb_track_inline_string(&sanitize_metadata(
            desired.file_name.as_deref().unwrap_or(&desired.file_path),
        ));
        let slot19_start = read_u16_le_at(&patched, 94 + 19 * 2)
            .map(|v| v as usize)
            .unwrap_or(0);
        let pad = (4 - (slot19_start + raw.len()) % 4) % 4;
        let mut padded = raw;
        padded.resize(padded.len() + pad, 0u8);
        patch_changed_string_slot(&mut patched, &fields, "file_name", 19, padded)
    } && patch_changed_string_slot(
        &mut patched,
        &fields,
        "file_path",
        20,
        encode_pdb_track_path_string(&desired.file_path),
    );

    if string_patches_ok {
        Ok(Some(patched))
    } else {
        Ok(None)
    }
}

/// Tombstone a row's slot within its page (clear it from `rowpf`, mark it in `tranrf`, decrement
/// the page's active-row count) and zero its id field so the vacated slot isn't left as a live
/// duplicate of whatever row replaces it elsewhere in the chain.
///
/// `id_field_offset` is the row-relative byte offset of the table's id field: `0x48` (72) for
/// tracks (t00), `4` for artists (t02), `12` for albums (t03).
fn mark_row_slot_inactive(
    bytes: &mut [u8],
    page_idx: u32,
    slot: PageRowSlot,
    id_field_offset: usize,
    page_size: usize,
) -> BackendResult<()> {
    let off = page_offset(page_idx, page_size).ok_or_else(|| {
        BackendError::Validation(format!(
            "PDB additive track mutation blocked: page {page_idx} out of bounds"
        ))
    })?;
    let rowpf_off = off + slot.bits_off;
    let tranrf_off = rowpf_off + 2;
    let rowpf = read_u16_le_at(bytes, rowpf_off).ok_or_else(|| {
        BackendError::Validation("PDB additive track mutation blocked: rowpf missing".into())
    })?;
    let tranrf = read_u16_le_at(bytes, tranrf_off).ok_or_else(|| {
        BackendError::Validation("PDB additive track mutation blocked: tranrf missing".into())
    })?;
    if rowpf & slot.bit_mask == 0 {
        return Ok(());
    }
    let new_rowpf = rowpf & !slot.bit_mask;
    bytes[rowpf_off..rowpf_off + 2].copy_from_slice(&new_rowpf.to_le_bytes());
    let new_tranrf = tranrf | slot.bit_mask;
    bytes[tranrf_off..tranrf_off + 2].copy_from_slice(&new_tranrf.to_le_bytes());

    // Zero the id field in the row we're vacating. The replacement row carries the same id to a
    // new slot elsewhere in the chain, so leaving the old id in place here would make this
    // tombstoned slot a live duplicate of the new row's id — exactly what
    // `detect_pdb_tombstoned_playlist_tree_ids`/the "Repair Tombstoned Row
    // IDs" fix in service::repair exists to clean up after the fact. Zero it
    // at the source instead of relying on that separate repair pass.
    let row_abs = off + HEAP_START + slot.start;
    if let Some(id_off) = row_abs.checked_add(id_field_offset)
        && id_off + 4 <= bytes.len()
    {
        bytes[id_off..id_off + 4].copy_from_slice(&0u32.to_le_bytes());
    }

    let page = &mut bytes[off..];
    if let Some((num_row_offsets, num_rows)) = pdb_page::read_slot_counts(page) {
        pdb_page::write_slot_counts(page, num_row_offsets, num_rows.saturating_sub(1));
    }
    // Tombstone state is tracked only in the row footer bitmasks; page_flags
    // must not be changed here (see tombstone policy in remove_ids_from_pdb).
    Ok(())
}

pub(crate) fn patch_playlist_tree_sort_orders_in_place(
    bytes: &mut [u8],
    patches: &[PdbPlaylistTreeSortOrderPatch],
    page_size: usize,
) -> BackendResult<usize> {
    if patches.is_empty() {
        return Ok(0);
    }
    if page_size != PAGE_SIZE || bytes.len() < page_size || !bytes.len().is_multiple_of(page_size) {
        return Err(BackendError::Validation(
            "PDB additive playlist-tree sort patch blocked: invalid page alignment".to_string(),
        ));
    }

    let desired_by_id: std::collections::HashMap<u32, u32> =
        patches.iter().map(|p| (p.id, p.sort_order)).collect();
    let (_t07_ec, first, last) = table_ptr_fields(bytes, 7).ok_or_else(|| {
        BackendError::Validation(
            "PDB additive playlist-tree sort patch blocked: t07 pointer missing".into(),
        )
    })?;
    let chain = collect_chain_pages(bytes, page_size, first, last).ok_or_else(|| {
        BackendError::Validation(
            "PDB additive playlist-tree sort patch blocked: t07 chain invalid".into(),
        )
    })?;

    let mut patched_ids = std::collections::HashSet::<u32>::new();
    let mut patched_pages = std::collections::HashSet::<u32>::new();
    for page_idx in chain.iter().copied().skip(1) {
        let Some(off) = page_offset(page_idx, page_size) else {
            continue;
        };
        let Some(page) = bytes.get(off..off + page_size) else {
            continue;
        };
        let used_s = read_u16_le_at(page, 30).unwrap_or(0) as usize;
        if used_s == 0 {
            continue;
        }
        let payload_len = used_s.min(page_size.saturating_sub(HEAP_START));
        let slots = parse_page_row_slots(page, page_size);
        for slot in slots {
            if !slot.present || slot.end > payload_len || slot.start >= slot.end {
                continue;
            }
            let row_abs = off + HEAP_START + slot.start;
            let row_len = slot.end - slot.start;
            if row_len < 20 {
                continue;
            }
            let Some(playlist_id) = read_u32_le_at(bytes, row_abs + 12) else {
                continue;
            };
            let Some(new_sort_order) = desired_by_id.get(&playlist_id).copied() else {
                continue;
            };
            bytes[row_abs + 8..row_abs + 12].copy_from_slice(&new_sort_order.to_le_bytes());
            patched_ids.insert(playlist_id);
            patched_pages.insert(page_idx);
        }
    }

    if patched_ids.len() != desired_by_id.len() {
        let missing = desired_by_id
            .keys()
            .filter(|id| !patched_ids.contains(id))
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(BackendError::Validation(format!(
            "PDB additive playlist-tree sort patch incomplete: missing t07 row(s) {missing}"
        )));
    }

    if !patched_pages.is_empty() {
        let mut next_seq = max_seqpage_in_file(bytes, page_size)
            .saturating_add(1)
            .max(2);
        let mut pages: Vec<u32> = patched_pages.into_iter().collect();
        pages.sort_unstable();
        for page_idx in pages {
            if let Some(off) = page_offset(page_idx, page_size) {
                let _ = write_u32_le_at(bytes, off + 0x10, next_seq);
                next_seq = next_seq.saturating_add(1);
            }
        }
        let seqdb = read_u32_le_at(bytes, 0x14)
            .unwrap_or(0)
            .max(next_seq)
            .max(max_seqpage_in_file(bytes, page_size).saturating_add(1));
        let _ = write_u32_le_at(bytes, 0x14, seqdb);
    }

    Ok(patched_ids.len())
}

pub(crate) fn mutate_tracks_in_place(
    bytes: &mut Vec<u8>,
    mutations: &[PdbTrackRowMutation],
    profile: PdbLayoutProfile,
    page_size: usize,
) -> BackendResult<usize> {
    if mutations.is_empty() {
        return Ok(0);
    }
    if page_size != PAGE_SIZE || bytes.len() < page_size || !bytes.len().is_multiple_of(page_size) {
        return Err(BackendError::Validation(
            "PDB additive track mutation blocked: invalid page alignment".to_string(),
        ));
    }

    let desired_by_id: std::collections::HashMap<u32, &PdbTrackRowMutation> =
        mutations.iter().map(|m| (m.row.id, m)).collect();
    let (_t00_ec, first, last) = table_ptr_fields(bytes, 0).ok_or_else(|| {
        BackendError::Validation("PDB additive track mutation blocked: t00 pointer missing".into())
    })?;
    let chain = collect_chain_pages(bytes, page_size, first, last).ok_or_else(|| {
        BackendError::Validation("PDB additive track mutation blocked: t00 chain invalid".into())
    })?;

    let mut patched_ids = std::collections::HashSet::<u32>::new();
    let mut patched_pages = std::collections::HashSet::<u32>::new();
    let mut replacement_rows = Vec::<Vec<u8>>::new();
    for page_idx in chain.iter().copied().skip(1) {
        let Some(off) = page_offset(page_idx, page_size) else {
            continue;
        };
        let Some(page) = bytes.get(off..off + page_size) else {
            continue;
        };
        let used_s = read_u16_le_at(page, 30).unwrap_or(0) as usize;
        if used_s == 0 {
            continue;
        }
        let payload_len = used_s.min(page_size.saturating_sub(40));
        let slots = parse_page_row_slots(page, page_size);
        for slot in slots {
            if !slot.present || slot.end > payload_len || slot.start >= slot.end {
                continue;
            }
            let row_abs = off + 40 + slot.start;
            let row_len = slot.end - slot.start;
            if row_len < 136 {
                continue;
            }
            let Some(track_id) = read_u32_le_at(bytes, row_abs + 72) else {
                continue;
            };
            let Some(mutation) = desired_by_id.get(&track_id).copied() else {
                continue;
            };
            let Some(row_view) = bytes.get(row_abs..row_abs + row_len) else {
                continue;
            };
            if let Some(patched_row) = build_same_size_track_row_patch(row_view, mutation)? {
                let Some(row_mut) = bytes.get_mut(row_abs..row_abs + row_len) else {
                    continue;
                };
                row_mut.copy_from_slice(&patched_row);
            } else {
                let replacement = encode_track_row_with_profile(&mutation.row, profile)?;
                if replacement.len() > MAX_ROW_LEN {
                    return Err(BackendError::Validation(format!(
                        "PDB additive track mutation blocked: replacement t00 row {} exceeds page capacity ({} bytes; max {MAX_ROW_LEN})",
                        mutation.row.id,
                        replacement.len()
                    )));
                }
                mark_row_slot_inactive(bytes, page_idx, slot, 0x48, page_size)?;
                replacement_rows.push(replacement);
            }
            patched_ids.insert(track_id);
            patched_pages.insert(page_idx);
        }
    }

    if patched_ids.len() != desired_by_id.len() {
        let missing = desired_by_id
            .keys()
            .filter(|id| !patched_ids.contains(id))
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(BackendError::Validation(format!(
            "PDB additive track mutation incomplete: missing t00 row(s) {missing}"
        )));
    }

    if !replacement_rows.is_empty() {
        let outcome = append_rows_to_chain_in_place(bytes, 0, &replacement_rows, page_size)?;
        if outcome.pages_reused > 0 || outcome.pages_appended > 0 {
            patched_pages.insert(outcome.new_last_page);
        }
    }

    if !patched_pages.is_empty() {
        let mut next_seq = max_seqpage_in_file(bytes, page_size)
            .saturating_add(1)
            .max(2);
        let mut pages: Vec<u32> = patched_pages.into_iter().collect();
        pages.sort_unstable();
        for page_idx in pages {
            if let Some(off) = page_offset(page_idx, page_size) {
                let _ = write_u32_le_at(bytes, off + 0x10, next_seq);
                next_seq = next_seq.saturating_add(1);
            }
        }
        let seqdb = read_u32_le_at(bytes, 0x14)
            .unwrap_or(0)
            .max(next_seq)
            .max(max_seqpage_in_file(bytes, page_size).saturating_add(1));
        let _ = write_u32_le_at(bytes, 0x14, seqdb);
    }

    Ok(patched_ids.len())
}

/// Relocate existing dictionary-table (artists t02, albums t03) rows whose id matches one of
/// `rows`, replacing each with freshly-encoded bytes of a possibly different length. Unlike
/// `mutate_tracks_in_place`, there is no same-size in-place patch attempt: every current caller
/// (the string-alignment repair) always needs a full re-encode, since the row's content hasn't
/// changed — only its byte layout (the UTF-16 name slot moving to a 4-byte aligned offset).
///
/// `id_offset` is the row-relative byte offset of the table's id field (`4` for artists, `12`
/// for albums).
pub(crate) fn realign_dictionary_rows_in_place(
    bytes: &mut Vec<u8>,
    table_type: u32,
    id_offset: usize,
    rows: &[(u32, Vec<u8>)],
    page_size: usize,
) -> BackendResult<usize> {
    if rows.is_empty() {
        return Ok(0);
    }
    if page_size != PAGE_SIZE || bytes.len() < page_size || !bytes.len().is_multiple_of(page_size) {
        return Err(BackendError::Validation(
            "PDB dictionary row realignment blocked: invalid page alignment".to_string(),
        ));
    }

    let desired_by_id: std::collections::HashMap<u32, &Vec<u8>> =
        rows.iter().map(|(id, row)| (*id, row)).collect();
    let (_ec, first, last) = table_ptr_fields(bytes, table_type).ok_or_else(|| {
        BackendError::Validation(format!(
            "PDB dictionary row realignment blocked: table {table_type} pointer missing"
        ))
    })?;
    // Chain validity is still required below to append the relocated rows, but the *search* for
    // rows to relocate deliberately scans every physical page stamped with this table type,
    // exactly like `detect_pdb_string_misalignment` does -- not just pages reachable by walking
    // the first/last chain pointers. A page can hold a stale row that's still `present` (e.g.
    // left behind, orphaned from the live chain, by an earlier rewrite that only touched
    // currently-referenced rows) while `detect_pdb_string_misalignment`'s raw scan still finds
    // it; walking only the chain here would silently leave such rows unfixed while this function
    // still reports success.
    collect_chain_pages(bytes, page_size, first, last).ok_or_else(|| {
        BackendError::Validation(format!(
            "PDB dictionary row realignment blocked: table {table_type} chain invalid"
        ))
    })?;

    let mut patched_ids = std::collections::HashSet::<u32>::new();
    let mut patched_pages = std::collections::HashSet::<u32>::new();
    let mut replacement_rows = Vec::<Vec<u8>>::new();
    let total_pages = (bytes.len() / page_size) as u32;
    for page_idx in 1..total_pages {
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
        let used_s = read_u16_le_at(page, 30).unwrap_or(0) as usize;
        if used_s == 0 {
            continue;
        }
        let payload_len = used_s.min(page_size.saturating_sub(HEAP_START));
        let slots = parse_page_row_slots(page, page_size);
        for slot in slots {
            if !slot.present || slot.end > payload_len || slot.start >= slot.end {
                continue;
            }
            let row_abs = off + HEAP_START + slot.start;
            let row_len = slot.end - slot.start;
            if row_len < id_offset + 4 {
                continue;
            }
            let Some(id) = read_u32_le_at(bytes, row_abs + id_offset) else {
                continue;
            };
            let Some(replacement) = desired_by_id.get(&id).copied() else {
                continue;
            };
            if replacement.len() > MAX_ROW_LEN {
                return Err(BackendError::Validation(format!(
                    "PDB dictionary row realignment blocked: replacement table {table_type} row {id} exceeds page capacity ({} bytes; max {MAX_ROW_LEN})",
                    replacement.len()
                )));
            }
            mark_row_slot_inactive(bytes, page_idx, slot, id_offset, page_size)?;
            replacement_rows.push(replacement.clone());
            patched_ids.insert(id);
            patched_pages.insert(page_idx);
        }
    }

    if patched_ids.len() != desired_by_id.len() {
        let missing = desired_by_id
            .keys()
            .filter(|id| !patched_ids.contains(id))
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(BackendError::Validation(format!(
            "PDB dictionary row realignment incomplete: missing table {table_type} row(s) {missing}"
        )));
    }

    if !replacement_rows.is_empty() {
        let outcome =
            append_rows_to_chain_in_place(bytes, table_type, &replacement_rows, page_size)?;
        if outcome.pages_reused > 0 || outcome.pages_appended > 0 {
            patched_pages.insert(outcome.new_last_page);
        }
    }

    if !patched_pages.is_empty() {
        let mut next_seq = max_seqpage_in_file(bytes, page_size)
            .saturating_add(1)
            .max(2);
        let mut pages: Vec<u32> = patched_pages.into_iter().collect();
        pages.sort_unstable();
        for page_idx in pages {
            if let Some(off) = page_offset(page_idx, page_size) {
                let _ = write_u32_le_at(bytes, off + 0x10, next_seq);
                next_seq = next_seq.saturating_add(1);
            }
        }
        let seqdb = read_u32_le_at(bytes, 0x14)
            .unwrap_or(0)
            .max(next_seq)
            .max(max_seqpage_in_file(bytes, page_size).saturating_add(1));
        let _ = write_u32_le_at(bytes, 0x14, seqdb);
    }

    Ok(patched_ids.len())
}

// ── Orchestrator ────────────────────────────────────────────────────────────

/// Apply a fully-classified additive diff to a copy of the existing PDB
/// bytes. Drives every per-table wrapper in foreign-key dependency order
/// so that newly-allocated dictionary IDs are present in the file before
/// the rows that reference them.
///
/// `t08` playlist_entries handling is split across three steps below:
/// removal (mirror mode), appending new entries via the unified
/// `append_rows_to_chain_in_place` path, and repositioning existing entries
/// whose `entry_index` changed in place.
///
/// On success returns the patched bytes; on failure returns the same
/// `BackendError::Validation` variant the caller would see from the
/// legacy path so error messages match across both write modes.
pub(crate) fn apply_additive_diff(
    existing_bytes: &[u8],
    diff: &PdbAdditiveDiff,
    profile: PdbLayoutProfile,
    page_size: usize,
) -> BackendResult<Vec<u8>> {
    let mut out = existing_bytes.to_vec();

    // Order matters for the empty-candidate page numbers that older players
    // validate. Playlist-tree rows are independent of track/dictionary rows,
    // and reference exports assign tt=7 the first free tail candidate on first export.
    // Append tt=7 before metadata/track growth advances next_unused.
    let _ = patch_playlist_tree_sort_orders_in_place(
        &mut out,
        &diff.playlist_tree_sort_order_patches,
        page_size,
    )?;

    let pt: Vec<(u32, u32, u32, bool, &str)> = diff
        .new_playlist_tree
        .iter()
        .map(|(id, p, s, f, n)| (*id, *p, *s, *f, n.as_str()))
        .collect();
    let _ = append_playlist_tree_in_place(&mut out, &pt, page_size)?;

    // Dictionary tables before tracks, so foreign-key lookups resolve in the
    // final PDB. Playlist entries stay last because they reference both tracks
    // and playlists.
    let genres: Vec<(u32, &str)> = diff
        .new_genres
        .iter()
        .map(|(id, n)| (*id, n.as_str()))
        .collect();
    let _ = append_genres_in_place(&mut out, &genres, page_size)?;

    let labels: Vec<(u32, &str)> = diff
        .new_labels
        .iter()
        .map(|(id, n)| (*id, n.as_str()))
        .collect();
    let _ = append_labels_in_place(&mut out, &labels, page_size)?;

    let keys: Vec<(u32, &str)> = diff
        .new_keys
        .iter()
        .map(|(id, n)| (*id, n.as_str()))
        .collect();
    let _ = append_keys_in_place(&mut out, &keys, page_size)?;

    let artists: Vec<(u32, &str)> = diff
        .new_artists
        .iter()
        .map(|(id, n)| (*id, n.as_str()))
        .collect();
    let _ = append_artists_in_place(&mut out, &artists, page_size)?;

    let albums: Vec<(u32, &str, u32)> = diff
        .new_albums
        .iter()
        .map(|(id, n, a)| (*id, n.as_str(), *a))
        .collect();
    let _ = append_albums_in_place(&mut out, &albums, page_size)?;

    let artwork: Vec<(u32, &str)> = diff
        .new_artwork
        .iter()
        .map(|(id, p)| (*id, p.as_str()))
        .collect();
    let _ = append_artwork_in_place(&mut out, &artwork, page_size)?;

    let _ = append_tracks_in_place(&mut out, &diff.new_tracks, profile, page_size)?;
    let _ = mutate_tracks_in_place(&mut out, &diff.mutated_tracks, profile, page_size)?;

    // ── t08 playlist_entries: removal (mirror mode) ─────────────────
    if !diff.removed_t08_entries.is_empty() {
        remove_t08_entries_in_place(&mut out, page_size, &diff.removed_t08_entries)?;
    }

    // ── t08 playlist_entries: append new entries ─────────────────────
    //
    // Strategy: figure out which entries are NEW (in desired but not in
    // the existing chain) and run them through the unified appender.
    // For an existing populated chain that has free latent slots, this
    // ends up reusing pages just like the legacy
    // `try_patch_t08_with_context` helper, but it also handles the
    // first-export case where t08 has no data pages yet — the legacy
    // helpers refuse that case and would silently leave the new
    // entries unwritten.
    if !diff.desired_t08_entries.is_empty() {
        // Identity only (playlist_id, track_id) -- an entry whose
        // entry_index is about to be patched in place below is still
        // "existing" here and must not also be appended as new.
        let existing_identities: std::collections::HashSet<(u32, u32)> =
            collect_t08_entry_keys(&out, page_size)
                .into_iter()
                .map(|k| (k.playlist_id, k.track_id))
                .collect();
        let added_keys: Vec<T08EntryKey> = diff
            .desired_t08_entries
            .iter()
            .copied()
            .filter(|k| !existing_identities.contains(&(k.playlist_id, k.track_id)))
            .collect();
        if !added_keys.is_empty() {
            let added_rows: Vec<Vec<u8>> = added_keys
                .iter()
                .map(|k| encode_t08_row(*k).to_vec())
                .collect();
            let _ = append_rows_to_chain_in_place(&mut out, 8, &added_rows, page_size)?;
        }
    }

    // ── t08 playlist_entries: reposition existing entries in place ──────
    //
    // A track whose entry_index changed (e.g. the playlist was reordered)
    // but whose (playlist_id, track_id) identity is unchanged is patched
    // in place here rather than removed and re-added -- see
    // `patch_t08_entry_indices_in_place`'s doc comment for why that
    // matters.
    if !diff.repositioned_t08_entries.is_empty() {
        let _ =
            patch_t08_entry_indices_in_place(&mut out, &diff.repositioned_t08_entries, page_size);
    }

    if diff.activate_t19_template {
        activate_t19_template_page(&mut out, page_size)?;
    }

    Ok(out)
}

/// Convenience entry point used from the dispatch shim. Builds the diff,
/// applies it, validates, and returns the new bytes. Returns `Ok(None)`
/// when the transition is non-additive; production export treats that as a
/// hard stop because populated PDBs must not fall through to a fresh rebuild.
pub(crate) fn try_write_pdb_additive_in_place(
    existing_bytes: &[u8],
    new_data: &PdbData,
    desired_t08_entries: Vec<T08EntryKey>,
    page_size: usize,
) -> BackendResult<Option<(Vec<u8>, AdditiveWriteSummary)>> {
    let diff = match compute_additive_diff(existing_bytes, new_data, desired_t08_entries) {
        Ok(d) => d,
        Err(reason) => {
            crate::logging::emit(
                crate::logging::Level::Info,
                "pdb-additive",
                &format!("non-additive diff: {reason}; topology-locked export will stop"),
            );
            return Ok(None);
        }
    };

    let summary = AdditiveWriteSummary {
        new_tracks: diff.new_tracks.len(),
        new_playlist_tree: diff.new_playlist_tree.len(),
    };

    let mut bytes = apply_additive_diff(existing_bytes, &diff, new_data.profile, page_size)?;
    if let (Some(src), Some(dst)) = (existing_bytes.get(0x10..0x14), bytes.get_mut(0x10..0x14)) {
        dst.copy_from_slice(src);
    }

    // Rebuild sentinel B-trees for tt=0, tt=7, tt=19. The tt=19 (history_runtime)
    // B-tree is not updated by the regular append path, leaving ne=0. Every accepted
    // Reference exports — including single-page exports — have ne=1 for tt=19.
    // This call is idempotent: tt=0 and tt=7 sentinels are already correct.
    rebuild_sentinel_btrees_inplace(&mut bytes);

    // rebuild_sentinel_btrees_inplace raises sentinel seqpages to max_data_seqpage+1.
    // seqdb must be strictly GREATER than all seqpages (including sentinels), so
    // re-bump it here after the sentinels are updated.
    // .max(34) matches reference fresh-export values and is a no-op for existing
    // USBs that already have seqdb > 34.
    {
        let max_seq = max_seqpage_in_file(&bytes, PAGE_SIZE);
        let cur = read_u32_le_at(&bytes, 0x14).unwrap_or(0);
        let seqdb = cur.max(max_seq.saturating_add(1)).max(34);
        let _ = write_u32_le_at(&mut bytes, 0x14, seqdb);
    }

    fix_tt8_num_rl_conventions_inplace(&mut bytes);

    // Defense in depth: `repositioned_t08_entries` (above) prevents new t08
    // duplicates from being created, but this sweep also self-heals a USB
    // that already has duplicate (playlist_id, track_id) rows from before
    // that fix existed -- every additive export cleans them up a little
    // more, without requiring a manual repair action.
    remove_duplicate_playlist_entries_inplace(&mut bytes);

    let mismatches = crate::pdb_reader::validate_pdb_page_conventions(&bytes);
    if !mismatches.is_empty() {
        let detail = mismatches
            .iter()
            .take(8)
            .map(|m| m.to_string())
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(BackendError::Validation(format!(
            "PDB additive write blocked: page-header convention mismatches ({detail})"
        )));
    }

    // Do NOT pad to next_unused here. The blank ec pages come naturally from
    // the resize done in append_rows_to_chain_in_place when overflow is placed
    // at max(physical_end, next_unused) — pages between physical_end and the
    // overflow page are already zeroed by Vec::resize. Padding further to
    // next_unused would overshoot the expected file size; working reference
    // exports have next_unused > file_page_count.

    Ok(Some((bytes, summary)))
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AdditiveWriteSummary {
    pub new_tracks: usize,
    pub new_playlist_tree: usize,
}

// ── Per-table wrappers ──────────────────────────────────────────────────────
//
// Trivial typed entry points around `append_rows_to_chain_in_place`, one per
// table type the additive path handles. They exist to:
//
// 1. Centralize "row encoder per table type" so the dispatch caller doesn't
//    have to know each table's wire format.
// 2. Keep the topology-lock invariants (no chain truncation, `first`
//    immovable) provable from a small, table-shaped surface.
// 3. Give each table a focused unit test surface.

/// t01 genres: id + name string. Identical wire format to t04 labels.
pub(crate) fn append_genres_in_place(
    bytes: &mut Vec<u8>,
    new: &[(u32, &str)],
    page_size: usize,
) -> BackendResult<AppendOutcome> {
    let rows: Vec<Vec<u8>> = new
        .iter()
        .map(|(id, name)| encode_genre_row(*id, name))
        .collect();
    append_rows_to_chain_in_place(bytes, 1, &rows, page_size)
}

/// t04 labels: id + name string.
pub(crate) fn append_labels_in_place(
    bytes: &mut Vec<u8>,
    new: &[(u32, &str)],
    page_size: usize,
) -> BackendResult<AppendOutcome> {
    let rows: Vec<Vec<u8>> = new
        .iter()
        .map(|(id, name)| encode_label_row(*id, name))
        .collect();
    append_rows_to_chain_in_place(bytes, 4, &rows, page_size)
}

/// t05 keys: id + duplicate id + name string.
pub(crate) fn append_keys_in_place(
    bytes: &mut Vec<u8>,
    new: &[(u32, &str)],
    page_size: usize,
) -> BackendResult<AppendOutcome> {
    let rows: Vec<Vec<u8>> = new
        .iter()
        .map(|(id, name)| encode_key_row(*id, name))
        .collect();
    append_rows_to_chain_in_place(bytes, 5, &rows, page_size)
}

/// t02 artists: subtype 0x0060 near variant + id + name.
pub(crate) fn append_artists_in_place(
    bytes: &mut Vec<u8>,
    new: &[(u32, &str)],
    page_size: usize,
) -> BackendResult<AppendOutcome> {
    use crate::service::export_helpers::pdb_encoding::encode_artist_row;
    let rows: Vec<Vec<u8>> = new
        .iter()
        .map(|(id, name)| encode_artist_row(*id, name))
        .collect();
    append_rows_to_chain_in_place(bytes, 2, &rows, page_size)
}

/// t03 albums: subtype 0x0080 near variant + artist_id + id + name.
pub(crate) fn append_albums_in_place(
    bytes: &mut Vec<u8>,
    new: &[(u32, &str, u32)],
    page_size: usize,
) -> BackendResult<AppendOutcome> {
    use crate::service::export_helpers::pdb_encoding::encode_album_row;
    let rows: Vec<Vec<u8>> = new
        .iter()
        .map(|(id, name, artist_id)| encode_album_row(*id, name, *artist_id))
        .collect();
    append_rows_to_chain_in_place(bytes, 3, &rows, page_size)
}

/// t13 artwork: id + path string.
pub(crate) fn append_artwork_in_place(
    bytes: &mut Vec<u8>,
    new: &[(u32, &str)],
    page_size: usize,
) -> BackendResult<AppendOutcome> {
    use crate::service::export_helpers::pdb_encoding::encode_artwork_row;
    let rows: Vec<Vec<u8>> = new
        .iter()
        .map(|(id, path)| encode_artwork_row(*id, path))
        .collect();
    append_rows_to_chain_in_place(bytes, 13, &rows, page_size)
}

/// t00 tracks: variable-length string heap with 21 string slots plus a
/// fixed header (see `docs/PDB.md` "Track row fixed fields").
///
/// This is the riskiest in-place wrapper because rows are large
/// (typically 250-500 bytes; can reach ~3500 bytes when paths/comments
/// are long) and the encoded `header_flags_u32` must be preserved
/// across writes — that's handled inside `encode_track_row_with_profile`
/// itself, so the wrapper is just a thin row-encoding pass.
pub(crate) fn append_tracks_in_place(
    bytes: &mut Vec<u8>,
    new: &[crate::service::export_helpers::PdbTrackRowData],
    profile: PdbLayoutProfile,
    page_size: usize,
) -> BackendResult<AppendOutcome> {
    use crate::service::export_helpers::pdb_encoding::encode_track_row_with_profile;
    let mut rows = Vec::<Vec<u8>>::with_capacity(new.len());
    for track in new {
        rows.push(encode_track_row_with_profile(track, profile)?);
    }
    append_rows_to_chain_in_place(bytes, 0, &rows, page_size)
}

/// t07 playlist_tree: parent_id + sort_order + id + is_folder + name.
/// Typically 0 or 1 new rows per additive export (one new playlist).
pub(crate) fn append_playlist_tree_in_place(
    bytes: &mut Vec<u8>,
    new: &[(u32, u32, u32, bool, &str)],
    page_size: usize,
) -> BackendResult<AppendOutcome> {
    use crate::service::export_helpers::pdb_encoding::encode_playlist_tree_row;
    let rows: Vec<Vec<u8>> = new
        .iter()
        .map(|(id, parent_id, sort_order, is_folder, name)| {
            encode_playlist_tree_row(*id, *parent_id, *sort_order, *is_folder, name)
        })
        .collect();
    let (tombstone_pages_reused, consumed) =
        append_rows_to_t07_tombstone_pages_in_place(bytes, &rows, page_size)?;
    if consumed >= rows.len() {
        let (ec, _first, last) = table_ptr_fields(bytes, 7).ok_or_else(|| {
            BackendError::Validation(
                "t07 append: table pointer missing after tombstone append".into(),
            )
        })?;
        return Ok(AppendOutcome {
            pages_reused: tombstone_pages_reused,
            pages_appended: 0,
            new_last_page: last,
            new_empty_candidate: ec,
        });
    }

    let mut outcome = append_rows_to_chain_in_place(bytes, 7, &rows[consumed..], page_size)?;
    outcome.pages_reused += tombstone_pages_reused;
    Ok(outcome)
}
