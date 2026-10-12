//! Whole-file writer: encodes a `PdbData` into pages and assembles a
//! complete `export.pdb` with no template dependency.

use super::*;

// ── Row Encoding ─────────────────────────────────────────────────────────────
//
// Delegates to existing functions in export_helpers where available.
// Simple row types are encoded inline here.

pub(crate) fn encode_genre_row(id: u32, name: &str) -> Vec<u8> {
    let name_bytes = encode_pdb_string(&sanitize_metadata(name));
    let mut row = vec![0u8; 4 + name_bytes.len()];
    row[0..4].copy_from_slice(&id.to_le_bytes());
    row[4..].copy_from_slice(&name_bytes);
    row
}

pub(crate) fn encode_label_row(id: u32, name: &str) -> Vec<u8> {
    encode_genre_row(id, name) // identical format
}

fn encode_color_row(row: &PdbColorRow) -> Vec<u8> {
    let name_bytes = encode_pdb_string(&row.name);
    // Color row: unknown(5) + id(2) + unknown(1) + name string
    let mut out = vec![0u8; 8 + name_bytes.len()];
    // Reference layout: byte 4 = low byte of id (duplicate), bytes 5-6 = id (u16)
    out[4] = row.id as u8;
    out[5..7].copy_from_slice(&row.id.to_le_bytes());
    out[8..].copy_from_slice(&name_bytes);
    out
}

fn encode_history_entry_row(row: &PdbHistoryEntryRow) -> Vec<u8> {
    let mut out = vec![0u8; 12];
    out[0..4].copy_from_slice(&row.track_id.to_le_bytes());
    out[4..8].copy_from_slice(&row.playlist_id.to_le_bytes());
    out[8..12].copy_from_slice(&row.entry_index.to_le_bytes());
    out
}

fn push_rows<T>(dst: &mut Vec<Vec<u8>>, src: &[T], mut encode: impl FnMut(&T) -> Vec<u8>) {
    dst.extend(src.iter().map(&mut encode));
}

fn push_try_rows<T>(
    dst: &mut Vec<Vec<u8>>,
    src: &[T],
    mut encode: impl FnMut(&T) -> BackendResult<Vec<u8>>,
) -> BackendResult<()> {
    for row in src {
        dst.push(encode(row)?);
    }
    Ok(())
}

/// Encode all tables in PdbData into raw row bytes, indexed by table type.
fn encode_all_tables(data: &PdbData) -> BackendResult<[Vec<Vec<u8>>; 20]> {
    let mut tables: [Vec<Vec<u8>>; 20] = Default::default();

    // t00: tracks
    push_try_rows(&mut tables[0], &data.tracks, |track| {
        encode_track_row_with_profile(track, data.profile)
    })?;
    // t01: genres
    push_rows(&mut tables[1], &data.genres, |g| {
        encode_genre_row(g.id, &g.name)
    });
    // t02: artists
    push_rows(&mut tables[2], &data.artists, |a| {
        encode_artist_row(a.id, &a.name)
    });
    // t03: albums
    push_rows(&mut tables[3], &data.albums, |a| {
        encode_album_row(a.id, &a.name, a.artist_id)
    });
    // t04: labels
    push_rows(&mut tables[4], &data.labels, |l| {
        encode_label_row(l.id, &l.name)
    });
    // t05: keys
    push_rows(&mut tables[5], &data.keys, |k| {
        encode_key_row(k.id, &k.name)
    });
    // t06: colors
    push_rows(&mut tables[6], &data.colors, encode_color_row);
    // t07: playlist tree
    push_rows(&mut tables[7], &data.playlist_tree, |p| {
        encode_playlist_tree_row(p.id, p.parent_id, p.sort_order, p.is_folder, &p.name)
    });
    // t08: playlist entries
    push_rows(&mut tables[8], &data.playlist_entries, |e| {
        encode_playlist_entry_row(e.entry_index, e.track_id, e.playlist_id)
    });
    // t09-t12: always empty (tables 9, 10, 11, 12)
    // t13: artwork
    push_rows(&mut tables[13], &data.artwork, |a| {
        encode_artwork_row(a.id, &a.path)
    });
    // t14-t15: always empty
    // t16: columns (raw pass-through)
    tables[16] = data.columns_raw_rows.clone();
    // t17: history playlists
    push_rows(&mut tables[17], &data.history_playlists, |h| {
        encode_genre_row(h.id, &h.name)
    }); // same format as genre
    // t18: history entries
    push_rows(
        &mut tables[18],
        &data.history_entries,
        encode_history_entry_row,
    );
    // t19: history (raw pass-through)
    tables[19] = data.history_raw_rows.clone();

    Ok(tables)
}

// ── Page Building ────────────────────────────────────────────────────────────

/// Round up to 4-byte alignment.
pub(super) fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// Build a sentinel/index page.
/// `has_data`: if false, next_page_dup at 0x2c is set to magic 0x03FFFFFF
/// (matches reference-export behavior for empty tables).
pub(super) fn build_sentinel_page(
    page_index: u32,
    table_type: u32,
    next_page: u32,
    has_data: bool,
) -> Vec<u8> {
    let mut page = vec![0u8; PAGE_SIZE];

    // Common header (0x00-0x1f)
    page[0x04..0x08].copy_from_slice(&page_index.to_le_bytes());
    page[0x08..0x0c].copy_from_slice(&table_type.to_le_bytes());
    page[0x0c..0x10].copy_from_slice(&next_page.to_le_bytes());
    page[0x10..0x14].copy_from_slice(&1u32.to_le_bytes()); // seqpage = 1
    // 0x18-0x1a: num_row_offsets=0, num_rows=0 → all zero
    page[0x1b] = PAGE_FLAGS_INDEX;
    // 0x1c-0x1d: free_size = 0
    // 0x1e-0x1f: used_size = 0

    // Index page extended header (0x20+)
    page[0x20..0x22].copy_from_slice(&SENTINEL_UNKNOWNA.to_le_bytes());
    page[0x22..0x24].copy_from_slice(&SENTINEL_UNKNOWNB.to_le_bytes());
    page[0x24..0x26].copy_from_slice(&SENTINEL_MAGIC_03EC.to_le_bytes());
    // 0x26-0x27: next_offset = 0
    page[0x28..0x2c].copy_from_slice(&page_index.to_le_bytes()); // redundant page_index
    if has_data {
        page[0x2c..0x30].copy_from_slice(&next_page.to_le_bytes()); // redundant next_page
    } else {
        // Reference exports use magic at 0x2c instead of next_page
        page[0x2c..0x30].copy_from_slice(&SENTINEL_MAGIC_03FFFFFF.to_le_bytes());
    }
    page[0x30..0x34].copy_from_slice(&SENTINEL_MAGIC_03FFFFFF.to_le_bytes());
    // 0x34-0x37: zeros
    // 0x38-0x39: num_entries = 0
    page[0x3a..0x3c].copy_from_slice(&SENTINEL_FIRST_EMPTY.to_le_bytes());
    page[0x3c..0x40].copy_from_slice(&SENTINEL_EMPTY_ENTRY.to_le_bytes());

    // Initialized sentinels carry repeated empty-entry markers
    // through the page footer area, with a small trailing zero tail.
    let mut off = 0x40usize;
    while off + 4 <= PAGE_SIZE - SENTINEL_TAIL_ZERO_BYTES {
        page[off..off + 4].copy_from_slice(&SENTINEL_EMPTY_ENTRY.to_le_bytes());
        off += 4;
    }

    page
}

/// Determine page_flags for a given table type.
pub(crate) fn page_flags_for_table(table_type: u32) -> u8 {
    match table_type {
        // 0x34: tables where the DJ software marks the last/active page with the transaction flag.
        // tt=0 (tracks) and tt=19 (history_runtime) use 0x34.
        // tt=7 (playlist_tree) uses 0x24 — reference exports confirmed 0x24 on playlist_tree
        // pages; using 0x34 here caused DJ software to reject the PDB as corrupted.
        0 | 19 => PAGE_FLAGS_DATA_TRACK,
        _ => PAGE_FLAGS_DATA,
    }
}

/// Pack encoded rows into one or more data pages.
/// Returns a list of complete 4096-byte pages plus any warnings produced
/// while packing (e.g. rows skipped because they were too large to fit on a
/// single page). page_index and next_page fields are NOT set here (set by
/// assembler).
///
/// Rows that cannot fit on any page on their own (e.g. a track row with
/// pathological field lengths) are skipped with a structured warning rather
/// than panicking — they would not load on a real player either.
pub(super) fn pack_rows_into_pages(
    table_type: u32,
    rows: &[Vec<u8>],
    seq_counter: &mut u32,
) -> (Vec<Vec<u8>>, Vec<String>) {
    let mut warnings = Vec::<String>::new();
    if rows.is_empty() {
        return (Vec::new(), warnings);
    }

    // Active (last) page uses 0x34 for tables that track current-transaction state.
    // All full preceding pages are sealed (0x24) with settled-transaction footer.
    // Reference format: DJ software rejects tables where multiple pages
    // have flags=0x34 simultaneously.
    let active_pf = page_flags_for_table(table_type);
    let mut pages: Vec<Vec<u8>> = Vec::new();
    let mut current_rows: Vec<&[u8]> = Vec::new();
    let mut current_heap_used = 0usize;

    let solo_capacity = PAGE_SIZE.saturating_sub(HEAP_START + footer_size(1));

    let mut next_seq = || {
        let s = *seq_counter;
        *seq_counter += 1;
        s
    };

    for (idx, row) in rows.iter().enumerate() {
        let aligned_len = align4(row.len());
        if row.len() > solo_capacity {
            let msg = format!(
                "pdb-writer: skipping oversized row {idx} in table {table_type} ({} bytes; max {} per page)",
                row.len(),
                solo_capacity
            );
            crate::logging::emit(crate::logging::Level::Warn, "pdb-writer", &msg);
            warnings.push(msg);
            continue;
        }
        let new_footer = footer_size(current_rows.len() + 1);
        let needed = HEAP_START + current_heap_used + aligned_len + new_footer;

        if (needed > PAGE_SIZE || current_rows.len() >= MAX_ROWS_PER_PAGE)
            && !current_rows.is_empty()
        {
            // First flushed page keeps the active flag (reference exporter convention:
            // the baseline/template page stays ACTV). All subsequent overflow flushes
            // are sealed (0x24, committed-transaction footer).
            let pf = if pages.is_empty() {
                active_pf
            } else {
                PAGE_FLAGS_DATA
            };
            pages.push(build_data_page(
                table_type,
                pf,
                next_seq(),
                &current_rows,
                true,
            ));
            current_rows.clear();
            current_heap_used = 0;
        }

        current_rows.push(row);
        current_heap_used += aligned_len;
    }

    if !current_rows.is_empty() {
        // Only/first page: ACTV. Any overflow tail page: SEAL (0x24).
        // Reference exporter convention: the initial (baseline) page
        // is active, while overflow additions are sealed/committed.
        let last_pf = if pages.is_empty() {
            active_pf
        } else {
            PAGE_FLAGS_DATA
        };
        pages.push(build_data_page(
            table_type,
            last_pf,
            next_seq(),
            &current_rows,
            false,
        ));
    }
    (pages, warnings)
}

/// Conformant `(u5, num_rl)` pair for a data page.
///
/// player firmware validates these per-table conventions; pages with values
/// matching only our internal reader's tolerant fallback (e.g. `num_rl=0`
/// with the real count in `nrs`) are rejected at mount time with a
/// "communication error". See `docs/PDB.md` "Per-table page-header
/// conventions" for the source-of-truth table.
pub fn data_page_footer_fields(table_type: u32, trc: u16) -> (u16, u16) {
    if trc == 0 {
        return (0, 0);
    }
    match table_type {
        0 | 1 | 2 | 3 | 4 | 5 | 8 | 13 => (1, trc.saturating_sub(1)),
        6 | 7 | 16 | 17 | 18 => (trc, 0),
        19 => {
            let num_rl = if trc > 1 { trc.saturating_sub(2) } else { 0 };
            (2, num_rl)
        }
        _ => (1, trc.saturating_sub(1)),
    }
}

/// Build a single data page from rows.
///
/// `seq` is the per-page transaction-commit sequence number written at
/// offset 0x10. Callers should pass an incrementing counter so data pages
/// don't all collapse to `seq=1` (which player firmware appears to validate).
///
/// `sealed`: when `true` the page uses the settled-transaction footer
/// `(u5=1, num_rl=nrs-1)` regardless of table type. Use `true` for every
/// full page except the last in a table chain. The last (active) page uses
/// `false` so `data_page_footer_fields` supplies the per-table convention.
pub(super) fn build_data_page(
    table_type: u32,
    pf: u8,
    seq: u32,
    rows: &[&[u8]],
    sealed: bool,
) -> Vec<u8> {
    let mut page = vec![0u8; PAGE_SIZE];

    // Common header — page_index and next_page set by assembler
    page[0x08..0x0c].copy_from_slice(&table_type.to_le_bytes());
    page[0x10..0x14].copy_from_slice(&seq.to_le_bytes());

    // Row count header bytes.
    let n = rows.len() as u32;
    if table_type == 19 {
        // History runtime pages use explicit nrs/u3 profile in observed exports.
        page[0x18] = (n & 0xFF) as u8;
        page[0x19] = 0x20;
        page[0x1a] = 0;
    } else {
        // Standard packed form: bits 0-12 = num_row_offsets, bits 13-23 = num_rows
        pdb_page::write_slot_counts(&mut page, rows.len(), rows.len());
    }
    page[0x1b] = pf;

    // Data page extended header (0x20-0x27)
    let trc = rows.len() as u16;
    let (u5, num_rl) = if sealed {
        // Settled-transaction format: all rows treated as pre-existing.
        // Reference exports use (1, nrs-1) for full non-last pages of every table type.
        (1u16, trc.saturating_sub(1))
    } else {
        data_page_footer_fields(table_type, trc)
    };
    page[0x20..0x22].copy_from_slice(&u5.to_le_bytes());
    page[0x22..0x24].copy_from_slice(&num_rl.to_le_bytes());
    // 0x24-0x25: u6 = 0
    // 0x26-0x27: u7 = 0

    // Write rows into heap with 4-byte alignment. `pack_rows_into_pages` is
    // responsible for never handing us a set of rows that exceeds page size;
    // this loop defensively checks bounds so a sizing miscalculation surfaces
    // as a dropped row + log line instead of a panic.
    let mut heap_offset = 0usize;
    let mut row_offsets: Vec<u16> = Vec::with_capacity(rows.len());
    for row in rows {
        let row_slot = row_offsets.len();
        let mut row_bytes = row.to_vec();
        apply_page_local_index_shift(table_type, &mut row_bytes, row_slot);
        let start = HEAP_START + heap_offset;
        if start.saturating_add(row_bytes.len()) > PAGE_SIZE {
            crate::logging::emit(
                crate::logging::Level::Error,
                "pdb-writer",
                &format!(
                    "dropping row in table {table_type} that does not fit on page ({} bytes at offset {})",
                    row_bytes.len(),
                    start
                ),
            );
            continue;
        }
        row_offsets.push(heap_offset as u16);
        page[start..start + row_bytes.len()].copy_from_slice(&row_bytes);
        heap_offset += align4(row_bytes.len());
    }

    // used_size = actual bytes used in heap (including alignment padding)
    page[0x1e..0x20].copy_from_slice(&(heap_offset as u16).to_le_bytes());

    // free_size = space between heap end and footer start
    let ft_size = footer_size(rows.len());
    let free = PAGE_SIZE.saturating_sub(HEAP_START + heap_offset + ft_size);
    page[0x1c..0x1e].copy_from_slice(&(free as u16).to_le_bytes());

    // Row index footer: grows backward from page end (see `pdb_page`).
    pdb_page::write_footer_all_present(&mut page, &row_offsets);

    // Re-stamp u5/num_rl/tranrf to the per-table convention.
    // `write_footer_all_present` above left every group's rowpf/tranrf at
    // "all rows active", which is only correct for `(trc, 0)`-family tables.
    // A sealed page already forced `(u5, num_rl) = (1, trc-1)` above
    // regardless of table type, so pass table 0's convention (also `(1,
    // trc-1)`) here to get the matching single-bit tranrf shape for it too.
    if !row_offsets.is_empty() {
        let conv_table = if sealed { 0 } else { table_type };
        apply_page_footer_convention(&mut page, conv_table, row_offsets.len());
    }

    page
}

/// Single source of truth for a data page's footer-convention fields: `u5`
/// (0x20), `num_rl` (0x22), and the per-16-row-group `tranrf` bitmasks.
///
/// `pdb_page::write_footer_all_present` and
/// `rewrite_variable_page_rows_in_place` both leave a page with all-bits `rowpf`/`tranrf` in every group — correct only
/// for `(trc, 0)`-convention tables (t06/t07/t16/t17/t18). Every other
/// in-place rewrite must call this afterward so the page's shape matches
/// what [`build_data_page`] (the from-scratch writer) would emit for the
/// same row set. See docs/PDB.md "Page Footer Conventions" and "Row Footer".
///
/// `row_count` is the page's active row-slot count (`trc`). `page` must be a
/// full page slice. For `tt=19` this assumes the page holds the
/// table's one live history row (see [`write_t19_runtime_footer_groups`]).
pub(super) fn apply_page_footer_convention(page: &mut [u8], table_type: u32, row_count: usize) {
    if page.len() < 0x24 {
        return;
    }
    if row_count == 0 {
        page[0x20..0x22].copy_from_slice(&0u16.to_le_bytes());
        page[0x22..0x24].copy_from_slice(&0u16.to_le_bytes());
        return;
    }

    let trc = row_count as u16;
    let (u5, num_rl) = data_page_footer_fields(table_type, trc);
    page[0x20..0x22].copy_from_slice(&u5.to_le_bytes());
    page[0x22..0x24].copy_from_slice(&num_rl.to_le_bytes());

    if table_type == 19 {
        // tt=19 "runtime profile": exactly one active row, in the last group.
        write_t19_runtime_footer_groups(page, row_count);
        return;
    }

    // `(trc, 0)` tables (t06/t07/t16/t17/t18) keep `tranrf == rowpf ==
    // all-bits`, which is exactly what the row-index footer writer already
    // wrote — nothing left to do.
    if u5 != 1 {
        return;
    }

    // `u5 == 1` tables: `tranrf` carries a single bit — the page's last row
    // slot (`num_rl`), in the group that holds it. Every other group's
    // `tranrf` is 0.
    pdb_page::write_single_slot_tranrf(page, row_count, num_rl as usize);
}

/// Write the tt=19 (history_runtime) "runtime profile" rowpf/tranrf footer
/// for a page holding `row_count` rows, into a full page buffer.
///
/// Only the most recently added row (and, when it shares a footer group
/// with it, the row immediately before it) is "active"/current — every
/// earlier row is committed history. This must hold across ALL footer
/// groups, not just the first: verified against real rekordbox's own
/// multi-group tt=19 output (a reference export of 88 rows across 6 groups
/// of 16), which has exactly one rowpf bit and two tranrf bits set globally,
/// both in the group holding the final row. `write_footer_all_present`
/// defaults every group's rowpf/tranrf to "all rows active", which is
/// correct for ordinary tables but wrong here for any group — this
/// overwrites every group's rowpf/tranrf accordingly.
///
/// Both call sites of this used to only handle the single-group case,
/// `row_count <= 16`; a page with more than 16 history rows silently kept
/// every row falsely marked simultaneously active once history rows
/// accumulated past one footer group (a real-world case: a drive from a
/// corruption report already had 75 history rows across 5 groups, all
/// falsely marked active).
fn write_t19_runtime_footer_groups(page: &mut [u8], row_count: usize) {
    if row_count == 0 {
        return;
    }
    let last_slot = row_count - 1;
    for group in pdb_page::footer_groups(page.len(), row_count) {
        let (rowpf, tranrf) = if group.contains_slot(last_slot) {
            let last_bit = last_slot - group.first_slot;
            let rowpf = 1u16 << last_bit;
            let tranrf = if last_bit >= 1 {
                rowpf | (1u16 << (last_bit - 1))
            } else {
                rowpf
            };
            (rowpf, tranrf)
        } else {
            (0u16, 0u16)
        };
        group.set_rowpf(page, rowpf);
        group.set_tranrf(page, tranrf);
    }
}

// ── File Assembly ────────────────────────────────────────────────────────────

struct TablePointer {
    table_type: u32,
    empty_candidate: u32,
    first_page: u32,
    last_page: u32,
}

fn baseline_sentinel_page(table_type: u32) -> u32 {
    1 + table_type * 2
}

fn baseline_payload_page(table_type: u32) -> u32 {
    baseline_sentinel_page(table_type) + 1
}

fn set_page_indices(page: &mut [u8], page_index: u32, next_page: u32) {
    page[0x04..0x08].copy_from_slice(&page_index.to_le_bytes());
    page[0x0c..0x10].copy_from_slice(&next_page.to_le_bytes());
}

fn write_page_to_baseline_slot(file: &mut [u8], page_index: u32, page: &[u8]) {
    let offset = page_index as usize * PAGE_SIZE;
    file[offset..offset + PAGE_SIZE].copy_from_slice(page);
}

/// Write a complete PDB file from a PdbData model.
///
/// Backwards-compatible entry point. Any warnings produced while packing
/// rows are emitted to stderr and otherwise dropped; callers that need
/// structured warnings (e.g. surfaced in the export response so they reach
/// the Event Log in the UI) should use [`write_pdb_with_warnings`] instead.
pub fn write_pdb(data: &PdbData) -> BackendResult<Vec<u8>> {
    let (bytes, _warnings) = write_pdb_with_warnings(data)?;
    Ok(bytes)
}

/// Write a complete PDB file from a PdbData model and return any warnings
/// produced while packing rows (e.g. tracks dropped because their encoded
/// row exceeded a single PDB page).
pub(crate) fn write_pdb_with_warnings(data: &PdbData) -> BackendResult<(Vec<u8>, Vec<String>)> {
    let encoded = encode_all_tables(data)?;
    let mut warnings = Vec::<String>::new();

    let mut file: Vec<u8> = Vec::new();
    let mut table_pointers: Vec<TablePointer> = Vec::with_capacity(20);

    // Page 0: file header, plus canonical baseline pages 1..40.
    // Keep the baseline table layout stable and append growth afterward so
    // late legacy tables (notably t16..t19) never get shifted by earlier growth.
    let baseline_page_count = 1 + NUM_TABLES as usize * 2;
    file.resize(baseline_page_count * PAGE_SIZE, 0);
    let mut page_idx: u32 = baseline_page_count as u32;

    // Per-page transaction sequence counter. Starts well above 1 (the value
    // sentinel/index pages keep) so that every data page across every table
    // gets a distinct, non-default seq. player firmware appears to validate that
    // an export's data pages are not all collapsed to seq=1.
    let mut seq_counter: u32 = 100;

    for table_type in 0..NUM_TABLES {
        let rows = &encoded[table_type as usize];
        let (data_pages, table_warnings) = pack_rows_into_pages(table_type, rows, &mut seq_counter);
        warnings.extend(table_warnings);

        let sentinel_idx = baseline_sentinel_page(table_type);
        let baseline_idx = baseline_payload_page(table_type);
        let sentinel = build_sentinel_page(
            sentinel_idx,
            table_type,
            baseline_idx,
            !data_pages.is_empty(),
        );
        write_page_to_baseline_slot(&mut file, sentinel_idx, &sentinel);

        if data_pages.is_empty() {
            // Empty table: keep the canonical blank payload page zeroed.
            table_pointers.push(TablePointer {
                table_type,
                empty_candidate: 0, // set later
                first_page: sentinel_idx,
                last_page: sentinel_idx, // reference exports: last = sentinel for empty tables
            });
        } else {
            // Table with data: first data page always occupies the canonical
            // baseline payload slot. Overflow pages, if any, are appended later.
            let mut first_page = data_pages[0].clone();
            let baseline_next = if data_pages.len() > 1 { page_idx } else { 0 };
            set_page_indices(&mut first_page, baseline_idx, baseline_next);
            // Multi-page tt=0 chains: ALL pages must be SEAL (0x24). The baseline
            // page was built as ACTV (0x34) by build_data_pages_from_rows (because
            // it was the first page flushed). Seal it now if overflow pages follow.
            if data_pages.len() > 1 && table_type == 0 {
                first_page[0x1b] = PAGE_FLAGS_DATA;
            }
            write_page_to_baseline_slot(&mut file, baseline_idx, &first_page);

            let mut last_data_idx = baseline_idx;
            let overflow_pages = data_pages.len().saturating_sub(1);
            for (overflow_idx, dp) in data_pages.iter().skip(1).enumerate() {
                let my_idx = page_idx;
                page_idx += 1;
                let next = if overflow_idx + 1 < overflow_pages {
                    page_idx
                } else {
                    0
                };

                let mut page = dp.clone();
                set_page_indices(&mut page, my_idx, next);
                file.extend_from_slice(&page);
                last_data_idx = my_idx;
            }

            table_pointers.push(TablePointer {
                table_type,
                empty_candidate: 0,
                first_page: sentinel_idx,
                last_page: last_data_idx,
            });
        }
    }

    let total_pages = file.len() / PAGE_SIZE;
    let mut virtual_page = total_pages as u32;

    // Assign empty_candidate values and fix tail-page next_page pointers.
    // Reference-export pattern: empty_candidate ALWAYS equals last_page.next_page.
    for tp in &mut table_pointers {
        if tp.first_page == tp.last_page {
            // Empty table: sentinel's next_page already points to the zeroed
            // blank page. empty_candidate = that blank page index (matches reference exports).
            let sentinel_off = tp.first_page as usize * PAGE_SIZE;
            let blank_page = u32::from_le_bytes(
                file[sentinel_off + 0x0c..sentinel_off + 0x10]
                    .try_into()
                    .unwrap(),
            );
            tp.empty_candidate = blank_page;

            // Observed behavior: selected empty tables keep the redundant next pointer
            // at 0x2c synchronized to the blank-page index instead of 0x03FFFFFF.
            if matches!(tp.table_type, 17..=19) {
                file[sentinel_off + 0x2c..sentinel_off + 0x30]
                    .copy_from_slice(&blank_page.to_le_bytes());
            }
        } else {
            // Table with data: assign ONE virtual page. Set it as both
            // empty_candidate AND last_data_page.next_page (matches reference exports).
            let last_off = tp.last_page as usize * PAGE_SIZE;
            if tp.table_type == 19 {
                // Keep runtime history table empty-candidate aligned to the page
                // right after the data tail (reference behavior).
                let candidate = tp.last_page + 1;
                // If last_page+1 is within the physical file it may be occupied by
                // an overflow page for another table; use a fresh virtual page instead.
                let ec = if (candidate as usize * PAGE_SIZE) < file.len() {
                    let vp = virtual_page;
                    virtual_page += 1;
                    vp
                } else {
                    candidate
                };
                tp.empty_candidate = ec;
                file[last_off + 0x0c..last_off + 0x10].copy_from_slice(&ec.to_le_bytes());
            } else {
                let vp = virtual_page;
                virtual_page += 1;
                tp.empty_candidate = vp;
                file[last_off + 0x0c..last_off + 0x10].copy_from_slice(&vp.to_le_bytes());
            }
        }
    }

    // Sync sentinel redundant next_page fields for tables with data
    for tp in &table_pointers {
        if tp.first_page != tp.last_page {
            let sentinel_off = tp.first_page as usize * PAGE_SIZE;
            let next = u32::from_le_bytes(
                file[sentinel_off + 0x0c..sentinel_off + 0x10]
                    .try_into()
                    .unwrap(),
            );
            file[sentinel_off + 0x2c..sentinel_off + 0x30].copy_from_slice(&next.to_le_bytes());
        }
    }

    // Populate sentinel B-tree entries.
    //
    // Entry format: entry = page_index * 8 (sector address, 512-byte sectors).
    // B-tree contains ONLY pages with flags=0x34 (active/current-transaction pages).
    // Sealed pages (0x24) are not indexed — DJ software navigates them via next_page
    // chain pointers. Reference format: a single entry pointing to
    // the last (active) page in each track-type table chain.
    // Tables whose pages are all 0x24 (artists, albums, etc.) get ne=0.
    {
        let total_physical = file.len() / PAGE_SIZE;
        for tp in &table_pointers {
            if tp.first_page == tp.last_page {
                continue; // empty table — no entries needed
            }
            let sentinel_off = tp.first_page as usize * PAGE_SIZE;
            // Collect pages with flags=0x34 by walking the chain.
            let first_data = u32::from_le_bytes(
                file[sentinel_off + 0x0c..sentinel_off + 0x10]
                    .try_into()
                    .unwrap(),
            );
            let mut active_pages: Vec<u32> = Vec::new();
            let mut current = first_data;
            for _ in 0..=total_physical {
                if current as usize >= total_physical {
                    break;
                }
                let page_off = current as usize * PAGE_SIZE;
                if file[page_off + 0x1b] == PAGE_FLAGS_DATA_TRACK {
                    active_pages.push(current);
                }
                let next =
                    u32::from_le_bytes(file[page_off + 0x0c..page_off + 0x10].try_into().unwrap());
                if next as usize >= total_physical || next == current {
                    break;
                }
                current = next;
            }
            if active_pages.is_empty() {
                continue; // no active pages — leave sentinel with ne=0
            }
            let n = active_pages.len();
            let fill_end = sentinel_off + PAGE_SIZE - SENTINEL_TAIL_ZERO_BYTES;
            for (i, &page_idx) in active_pages.iter().enumerate() {
                let entry_off = sentinel_off + 0x3c + i * 4;
                if entry_off + 4 > fill_end {
                    break;
                }
                let entry_val = page_idx * 8;
                file[entry_off..entry_off + 4].copy_from_slice(&entry_val.to_le_bytes());
            }
            let ne = n.min(0x1FFF) as u16;
            file[sentinel_off + 0x38..sentinel_off + 0x3a].copy_from_slice(&ne.to_le_bytes());
            // first_empty = 0x1FFF (no free list)
            file[sentinel_off + 0x3a..sentinel_off + 0x3c]
                .copy_from_slice(&0x1fffu16.to_le_bytes());
            // u7 (write-pointer) = n
            file[sentinel_off + 0x26..sentinel_off + 0x28].copy_from_slice(&ne.to_le_bytes());
        }
    }

    // Write file header (page 0)
    file[0x04..0x08].copy_from_slice(&(PAGE_SIZE as u32).to_le_bytes());
    file[0x08..0x0c].copy_from_slice(&NUM_TABLES.to_le_bytes());
    file[0x0c..0x10].copy_from_slice(&virtual_page.to_le_bytes()); // next_unused_page
    file[0x10..0x14].copy_from_slice(&5u32.to_le_bytes()); // matches working reference exports
    // seqdb must be > max(seqpage) across all data pages. t19 history pages
    // use seqpage=29 to match reference exports, so we cannot hardcode a low
    // constant here — compute the actual maximum and add 1.
    let max_seqpage = file
        .as_chunks::<PAGE_SIZE>()
        .0
        .iter()
        .skip(1) // skip header page 0
        .map(|p| u32::from_le_bytes(p[0x10..0x14].try_into().unwrap_or([0; 4])))
        .max()
        .unwrap_or(0);
    let seqdb = max_seqpage.saturating_add(1).max(6);
    file[0x14..0x18].copy_from_slice(&seqdb.to_le_bytes()); // seqdb

    // Table pointer array at 0x1c
    for (i, tp) in table_pointers.iter().enumerate() {
        let off = TABLE_POINTERS_OFFSET + i * TABLE_POINTER_SIZE;
        file[off..off + 4].copy_from_slice(&tp.table_type.to_le_bytes());
        file[off + 4..off + 8].copy_from_slice(&tp.empty_candidate.to_le_bytes());
        file[off + 8..off + 12].copy_from_slice(&tp.first_page.to_le_bytes());
        file[off + 12..off + 16].copy_from_slice(&tp.last_page.to_le_bytes());
    }

    // Do NOT pad to virtual_page here. The fresh writer is only used for the
    // initialize_usb template (colors + columns + history shape), never for
    // the actual export payload. The file size is determined by what was written.

    Ok((file, warnings))
}

#[cfg(test)]
pub(crate) fn write_pdb_to_file(path: &Path, data: &PdbData) -> BackendResult<()> {
    let bytes = write_pdb(data)?;
    std::fs::write(path, &bytes)?;
    Ok(())
}
