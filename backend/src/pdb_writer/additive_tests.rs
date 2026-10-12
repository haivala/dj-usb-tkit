use super::*;

// t08 playlist_entries rows are 12 bytes, so byte capacity alone lets a
// single 4096-byte page hold well over 255 rows (see docs/PDB.md "Page
// Footer Conventions"). Our own additive writer never produces such a
// page (`MAX_ROWS_PER_PAGE` caps every growth path at 255), but a page
// authored by real rekordbox for a large playlist can legitimately have
// more, and this tool must still read it correctly when patching it.
#[test]
fn parse_page_row_slots_handles_more_than_255_rows_on_a_page() {
    let row_count = 260usize;
    let rows: Vec<[u8; 12]> = (0..row_count as u32)
        .map(|idx| {
            encode_t08_row(T08EntryKey {
                entry_index: idx + 1,
                track_id: idx + 1,
                playlist_id: 1,
            })
        })
        .collect();
    let row_refs: Vec<&[u8]> = rows.iter().map(|r| r.as_slice()).collect();
    let page = build_data_page(8, page_flags_for_table(8), 1, &row_refs, false);

    let slots = parse_page_row_slots(&page, PAGE_SIZE);

    assert_eq!(
        slots.len(),
        row_count,
        "page header row-slot count must not wrap/undercount past 255 rows"
    );
}

fn make_empty_pdb_with_artists(count: usize) -> Vec<u8> {
    let mut data = PdbData::empty();
    for i in 1..=count {
        data.artists.push(PdbArtistRow {
            id: i as u32,
            name: format!("Artist {i:03}"),
        });
    }
    data.colors = standard_colors();
    data.columns_raw_rows = standard_columns_raw();
    write_pdb(&data).expect("write pdb fixture")
}

fn encode_artist_row(id: u32, name: &str) -> Vec<u8> {
    // Match the writer's near-variant artist encoding closely enough
    // for footer-capacity bookkeeping. The exact byte layout doesn't
    // matter for the topology tests; we only need consistent lengths
    // and that read_present_page_rows can locate the rows again.
    let mut out = Vec::<u8>::new();
    out.extend_from_slice(&0x0060u16.to_le_bytes()); // subtype
    out.push(0x03); // index_shift
    out.push(0x09); // ofs_name
    out.extend_from_slice(&id.to_le_bytes());
    // String: short-ASCII header (0x40 | (len*2 + 1)) then bytes
    let name_bytes = name.as_bytes();
    let header = (((name_bytes.len() * 2 + 1) & 0x7F) | 0x40) as u8;
    out.push(header);
    out.extend_from_slice(name_bytes);
    out
}

fn indexed_table_row_shift_by_id(
    bytes: &[u8],
    table_type: u32,
    id_offset: usize,
    id: u32,
) -> Option<(u32, usize, u16)> {
    let (_ec, first, last) = table_ptr_fields(bytes, table_type)?;
    let chain = collect_chain_pages(bytes, PAGE_SIZE, first, last)?;
    for page_idx in chain.into_iter().skip(1) {
        let off = page_offset(page_idx, PAGE_SIZE)?;
        let page = bytes.get(off..off + PAGE_SIZE)?;
        let used_s = read_u16_le_at(page, 0x1e)? as usize;
        let payload_len = used_s.min(PAGE_SIZE.saturating_sub(HEAP_START));
        let payload = page.get(HEAP_START..HEAP_START + payload_len)?;
        for (row_idx, slot) in parse_page_row_slots(page, PAGE_SIZE)
            .into_iter()
            .enumerate()
        {
            if !slot.present || slot.end > payload.len() || slot.start >= slot.end {
                continue;
            }
            let row = &payload[slot.start..slot.end];
            if read_u32_le_at(row, id_offset) == Some(id) {
                return Some((page_idx, row_idx, read_u16_le_at(row, 2)?));
            }
        }
    }
    None
}

fn playlist_tree_row_loc_by_id(bytes: &[u8], id: u32) -> Option<(u32, usize, usize, PageRowSlot)> {
    let (_ec, first, last) = table_ptr_fields(bytes, 7)?;
    let chain = collect_chain_pages(bytes, PAGE_SIZE, first, last)?;
    for page_idx in chain.into_iter().skip(1) {
        let off = page_offset(page_idx, PAGE_SIZE)?;
        let page = bytes.get(off..off + PAGE_SIZE)?;
        let used_s = read_u16_le_at(page, 0x1e)? as usize;
        let payload_len = used_s.min(PAGE_SIZE.saturating_sub(HEAP_START));
        for slot in parse_page_row_slots(page, PAGE_SIZE) {
            if slot.end > payload_len || slot.start >= slot.end {
                continue;
            }
            let row_abs = off + HEAP_START + slot.start;
            let row_len = slot.end - slot.start;
            if row_len >= 20 && read_u32_le_at(bytes, row_abs + 12) == Some(id) {
                return Some((page_idx, row_abs, row_len, slot));
            }
        }
    }
    None
}

fn mark_playlist_tree_row_inactive_for_test(bytes: &mut [u8], id: u32) {
    let (page_idx, _row_abs, _row_len, slot) =
        playlist_tree_row_loc_by_id(bytes, id).expect("t07 row loc");
    let off = page_offset(page_idx, PAGE_SIZE).expect("page off");
    let rowpf_off = off + slot.bits_off;
    let tranrf_off = rowpf_off + 2;
    let rowpf = read_u16_le_at(bytes, rowpf_off).expect("rowpf");
    let tranrf = read_u16_le_at(bytes, tranrf_off).expect("tranrf");
    bytes[rowpf_off..rowpf_off + 2].copy_from_slice(&(rowpf & !slot.bit_mask).to_le_bytes());
    bytes[tranrf_off..tranrf_off + 2].copy_from_slice(&(tranrf | slot.bit_mask).to_le_bytes());

    let packed = u32::from(bytes[off + 0x18])
        | (u32::from(bytes[off + 0x19]) << 8)
        | (u32::from(bytes[off + 0x1a]) << 16);
    let num_row_offsets = packed & 0x1fff;
    let active_rows = (packed >> 13) & 0x7ff;
    let new_packed = num_row_offsets | ((active_rows.saturating_sub(1) & 0x7ff) << 13);
    bytes[off + 0x18] = (new_packed & 0xff) as u8;
    bytes[off + 0x19] = ((new_packed >> 8) & 0xff) as u8;
    bytes[off + 0x1a] = ((new_packed >> 16) & 0xff) as u8;
}

fn raw_track_row(id: u32) -> Vec<u8> {
    let mut row = vec![0u8; 136];
    row[0..2].copy_from_slice(&0x0024u16.to_le_bytes());
    row[2..4].copy_from_slice(&0xffffu16.to_le_bytes());
    row[72..76].copy_from_slice(&id.to_le_bytes());
    row
}

#[test]
fn append_rows_to_chain_in_place_no_growth_when_capacity_available() {
    let mut bytes = make_empty_pdb_with_artists(2);
    let pages_before = bytes.len() / PAGE_SIZE;
    let (ec_before, first_before, last_before) = table_ptr_fields(&bytes, 2).expect("artist ptr");
    let last_off = page_offset(last_before, PAGE_SIZE).expect("artist data page");
    let used_before =
        u16::from_le_bytes(bytes[last_off + 0x1e..last_off + 0x20].try_into().unwrap()) as usize;
    let payload_before = bytes[last_off + HEAP_START..last_off + HEAP_START + used_before].to_vec();
    let u5_before = u16::from_le_bytes(bytes[last_off + 0x20..last_off + 0x22].try_into().unwrap());

    // Manually set existing footer masks to simulate a page that had
    // prior transaction state. Reused-page append must preserve it and
    // OR in only the appended row bit.
    let rowpf_off = last_off + PAGE_SIZE - 4;
    let tranrf_off = last_off + PAGE_SIZE - 2;
    bytes[rowpf_off..rowpf_off + 2].copy_from_slice(&0x0003u16.to_le_bytes());
    bytes[tranrf_off..tranrf_off + 2].copy_from_slice(&0x0002u16.to_le_bytes());

    let new_rows = vec![encode_artist_row(101, "Added Artist")];
    let outcome = append_rows_to_chain_in_place(&mut bytes, 2, &new_rows, PAGE_SIZE).unwrap();

    let pages_after = bytes.len() / PAGE_SIZE;
    assert_eq!(
        pages_after, pages_before,
        "appending one short row to a sparsely-populated existing page \
         must not grow the file"
    );
    assert_eq!(outcome.pages_reused, 1);
    assert_eq!(outcome.pages_appended, 0);

    let (ec_after, first_after, last_after) =
        table_ptr_fields(&bytes, 2).expect("artist ptr after");
    assert_eq!(first_after, first_before, "first must not move");
    assert_eq!(last_after, last_before, "last unchanged when no append");
    assert_eq!(
        ec_after, ec_before,
        "empty_candidate stays byte-stable when no chain growth happens"
    );
    assert_eq!(
        &bytes[last_off + HEAP_START..last_off + HEAP_START + used_before],
        payload_before.as_slice(),
        "existing row payload bytes must stay stable on reused-page append"
    );
    assert_eq!(
        u16::from_le_bytes(bytes[rowpf_off..rowpf_off + 2].try_into().unwrap()),
        0x0007,
        "rowpf should retain existing bits and add only the appended row"
    );
    assert_eq!(
        u16::from_le_bytes(bytes[tranrf_off..tranrf_off + 2].try_into().unwrap()),
        0x0006,
        "tranrf should preserve existing bits and add only the appended row"
    );
    assert_eq!(
        u16::from_le_bytes(bytes[last_off + 0x20..last_off + 0x22].try_into().unwrap()),
        u5_before,
        "generic reused-page append keeps the existing u5 convention"
    );
}

#[test]
fn append_rows_assigns_index_shift_on_reused_indexed_page() {
    let mut bytes = make_empty_pdb_with_artists(2);
    let new_rows =
        vec![crate::service::export_helpers::pdb_encoding::encode_artist_row(101, "Test Ártist Ä")];

    append_rows_to_chain_in_place(&mut bytes, 2, &new_rows, PAGE_SIZE).unwrap();

    let (_page, slot, shift) =
        indexed_table_row_shift_by_id(&bytes, 2, 4, 101).expect("appended artist row");
    assert_eq!(slot, 2);
    assert_eq!(shift, 64, "artist index_shift must be slot * 32");
}

#[test]
fn append_rows_assigns_index_shift_on_fresh_indexed_page() {
    let mut data = PdbData::empty();
    data.colors = standard_colors();
    data.columns_raw_rows = standard_columns_raw();
    let mut bytes = write_pdb(&data).expect("write empty pdb fixture");
    let rows = vec![raw_track_row(11), raw_track_row(12), raw_track_row(13)];

    append_rows_to_chain_in_place(&mut bytes, 0, &rows, PAGE_SIZE).unwrap();

    for (id, expected_slot, expected_shift) in [(11, 0, 0), (12, 1, 32), (13, 2, 64)] {
        let (_page, slot, shift) =
            indexed_table_row_shift_by_id(&bytes, 0, 72, id).expect("appended track row");
        assert_eq!(slot, expected_slot, "track {id} slot");
        assert_eq!(shift, expected_shift, "track {id} index_shift");
    }
}

#[test]
fn append_track_growth_preserves_existing_tail_transaction_state() {
    let mut data = PdbData::empty();
    data.colors = standard_colors();
    data.columns_raw_rows = standard_columns_raw();
    let mut bytes = write_pdb(&data).expect("write empty pdb fixture");

    append_rows_to_chain_in_place(&mut bytes, 0, &[raw_track_row(1)], PAGE_SIZE)
        .expect("seed real track data page");
    let (_old_ec, _first, old_last) = table_ptr_fields(&bytes, 0).expect("track ptr");
    let old_off = page_offset(old_last, PAGE_SIZE).expect("track tail page");
    bytes[old_off + 0x1b] = PAGE_FLAGS_DATA_TRACK;
    bytes[old_off + 0x20..old_off + 0x22].copy_from_slice(&2u16.to_le_bytes());
    bytes[old_off + 0x22..old_off + 0x24].copy_from_slice(&0u16.to_le_bytes());
    bytes[old_off + PAGE_SIZE - 4..old_off + PAGE_SIZE - 2]
        .copy_from_slice(&0x0001u16.to_le_bytes());
    bytes[old_off + PAGE_SIZE - 2..old_off + PAGE_SIZE].copy_from_slice(&0x0001u16.to_le_bytes());
    let old_tail_before = bytes[old_off..old_off + PAGE_SIZE].to_vec();

    let mut huge_track = vec![0u8; MAX_ROW_LEN];
    huge_track[0..2].copy_from_slice(&0x0024u16.to_le_bytes());
    huge_track[2..4].copy_from_slice(&0xffffu16.to_le_bytes());
    huge_track[72..76].copy_from_slice(&2u32.to_le_bytes());

    let outcome = append_rows_to_chain_in_place(&mut bytes, 0, &[huge_track], PAGE_SIZE).unwrap();
    assert_eq!(
        outcome.pages_reused + outcome.pages_appended,
        1,
        "growth should allocate exactly one new chain page"
    );
    assert_ne!(outcome.new_last_page, old_last);

    let old_tail_after = &bytes[old_off..old_off + PAGE_SIZE];
    let mut expected_old_tail = old_tail_before;
    // When overflow is appended to a previously-single-page chain, the original
    // first data page must be sealed (0x24) — all pages in a multi-page tt=0
    // chain must be SEAL per reference-export convention.
    expected_old_tail[0x0c..0x10].copy_from_slice(&outcome.new_last_page.to_le_bytes());
    expected_old_tail[0x1b] = PAGE_FLAGS_DATA; // sealed: was ACTV (0x34) as single-page chain
    assert_eq!(
        old_tail_after,
        expected_old_tail.as_slice(),
        "growing t00 must link the previous tail and seal it to 0x24; \
         other footer bytes (u5/num_rl/tranrf history) stay stable"
    );

    let new_off = page_offset(outcome.new_last_page, PAGE_SIZE).expect("new track page");
    // Overflow pages are SEAL (0x24) — all pages in a multi-page tt=0 chain
    // must use SEAL per reference-export convention.
    assert_eq!(bytes[new_off + 0x1b], PAGE_FLAGS_DATA);
    assert_eq!(read_u16_le_at(&bytes, new_off + 0x20), Some(1));
    // num_rl = trc-1 for sealed tt=0 page (same u5=1 convention as active pages)
}

#[test]
fn append_track_growth_finalizes_pages_created_in_same_batch() {
    let mut data = PdbData::empty();
    data.colors = standard_colors();
    data.columns_raw_rows = standard_columns_raw();
    let mut bytes = write_pdb(&data).expect("write empty pdb fixture");

    let mut rows = Vec::<Vec<u8>>::new();
    for id in 1..=3u32 {
        let mut row = vec![0u8; 2000];
        row[0..2].copy_from_slice(&0x0024u16.to_le_bytes());
        row[2..4].copy_from_slice(&0xffffu16.to_le_bytes());
        row[72..76].copy_from_slice(&id.to_le_bytes());
        rows.push(row);
    }

    let outcome = append_rows_to_chain_in_place(&mut bytes, 0, &rows, PAGE_SIZE).unwrap();
    assert_ne!(outcome.new_last_page, 0);
    let (_ec, first, last) = table_ptr_fields(&bytes, 0).expect("track ptr");
    let chain = collect_chain_pages(&bytes, PAGE_SIZE, first, last).expect("track chain");
    let data_pages: Vec<u32> = chain.into_iter().skip(1).collect();
    assert_eq!(
        data_pages.len(),
        2,
        "three large rows should create two data pages"
    );

    // Reference convention: when a chain has multiple data pages, ALL are SEAL (0x24).
    // Only single-page chains keep ACTV (0x34). Verified on working reference exports
    // which have flags=0x24 on every tt=0 page in multi-page chains.
    let first_data_off = page_offset(data_pages[0], PAGE_SIZE).expect("first data page");
    assert_eq!(bytes[first_data_off + 0x1b], PAGE_FLAGS_DATA); // SEAL — multi-page chain
    assert_eq!(read_u16_le_at(&bytes, first_data_off + 0x20), Some(1));
    assert_eq!(read_u16_le_at(&bytes, first_data_off + 0x22), Some(1));
    assert_eq!(
        read_u16_le_at(&bytes, first_data_off + PAGE_SIZE - 2),
        Some(0x0002)
    );

    let last_data_off = page_offset(data_pages[1], PAGE_SIZE).expect("last data page");
    assert_eq!(bytes[last_data_off + 0x1b], PAGE_FLAGS_DATA); // SEAL — overflow addition
    assert_eq!(read_u16_le_at(&bytes, last_data_off + 0x20), Some(1));
    // num_rl = trc-1 for the sealed overflow tt=0 page (same u5=1 convention)
}

#[test]
fn append_rows_to_chain_in_place_appends_when_capacity_exhausted() {
    // Build a file where the artists chain has only 1 data page that
    // we manually fill near capacity. Then append one row and assert
    // the file grew by exactly one page.
    let mut data = PdbData::empty();
    for i in 1..=120 {
        data.artists.push(PdbArtistRow {
            id: i as u32,
            name: "ArtistWithSomewhatLongerName".repeat(3),
        });
    }
    data.colors = standard_colors();
    data.columns_raw_rows = standard_columns_raw();
    let mut bytes = write_pdb(&data).expect("write pdb fixture");

    let pages_before = bytes.len() / PAGE_SIZE;
    let (_ec_before, first_before, last_before) = table_ptr_fields(&bytes, 2).expect("artist ptr");

    // Append a single short row; if the existing tail is full, this
    // forces a fresh page.
    let new_rows = vec![encode_artist_row(9999, "X")];
    let outcome = append_rows_to_chain_in_place(&mut bytes, 2, &new_rows, PAGE_SIZE).unwrap();

    let pages_after = bytes.len() / PAGE_SIZE;
    // Either we reused (no growth) or appended one fresh page.
    assert!(
        pages_after == pages_before || pages_after == pages_before + 1,
        "expected grow-by-0-or-1, got {} -> {}",
        pages_before,
        pages_after
    );
    assert_eq!(outcome.pages_reused + outcome.pages_appended, 1);

    let (_ec_after, first_after, last_after) =
        table_ptr_fields(&bytes, 2).expect("artist ptr after");
    assert_eq!(first_after, first_before, "first must not move");
    if outcome.pages_appended > 0 {
        assert_ne!(last_after, last_before, "last must advance on append");
    }
}

#[test]
fn append_rows_to_chain_in_place_never_exceeds_255_rows_per_page() {
    // Regression: a page's row-slot count (`nrs`) is a single byte and
    // wraps silently past 255 -- docs/PDB.md already documented this as
    // a hazard the writer must never rely on. tt=8 (playlist_entries)
    // rows are only 12 bytes, so a large enough playlist (confirmed via
    // a real 559-track playlist crashing rekordbox desktop outright,
    // with no error dialog) could previously pack 250+ rows onto a
    // single page purely because there was still byte capacity left,
    // producing an invalid `nrs` value real player parsers are not
    // verified to tolerate.
    let mut data = PdbData::empty();
    data.colors = standard_colors();
    data.columns_raw_rows = standard_columns_raw();
    let bytes = write_pdb(&data).expect("write empty seed");
    let mut bytes = bytes;

    let new_rows: Vec<Vec<u8>> = (1u32..=600)
        .map(|i| {
            encode_t08_row(T08EntryKey {
                entry_index: i,
                track_id: i,
                playlist_id: 1,
            })
            .to_vec()
        })
        .collect();
    append_rows_to_chain_in_place(&mut bytes, 8, &new_rows, PAGE_SIZE)
        .expect("append 600 playlist entries");

    let (_ec, first, last) = table_ptr_fields(&bytes, 8).expect("tt=8 pointer");
    let chain = collect_chain_pages(&bytes, PAGE_SIZE, first, last).expect("tt=8 chain");
    let mut total_rows = 0usize;
    for &page_idx in chain.iter().skip(1) {
        let off = page_offset(page_idx, PAGE_SIZE).expect("page offset");
        let page = &bytes[off..off + PAGE_SIZE];
        if page[0x1b] == 0x64 {
            continue; // sentinel/blank
        }
        let packed = (page[0x18] as u32) | ((page[0x19] as u32) << 8) | ((page[0x1a] as u32) << 16);
        let row_count = ((packed >> 13) & 0x7FF) as usize;
        assert!(
            row_count <= MAX_ROWS_PER_PAGE,
            "page[{page_idx}] has {row_count} rows, exceeding the 255-row cap"
        );
        total_rows += row_count;
    }
    assert_eq!(
        total_rows, 600,
        "all 600 rows must be present across the chain"
    );
}

#[test]
fn append_overflow_skips_pages_reserved_by_ec_pointers() {
    // Regression test: when next_unused in the PDB header has been advanced
    // past bytes.len()/page_size by earlier ec allocations (e.g. dict tables
    // claimed virtual ec pages 43-50 before tracks needed an overflow), the
    // overflow must go to max(physical_end, next_unused) — not to physical_end.
    // Otherwise the overflow page lands at the same index as another table's ec,
    // creating a data/ec conflict that DJ software rejects as "database corrupted".

    // Build an empty template and manually advance next_unused to simulate
    // the state after several dict-table ec assignments.
    let data = PdbData {
        colors: standard_colors(),
        columns_raw_rows: standard_columns_raw(),
        ..PdbData::empty()
    };
    let mut bytes = write_pdb(&data).expect("write empty pdb");
    let physical_pages = (bytes.len() / PAGE_SIZE) as u32;

    // Simulate: tt=16 (columns) has ec=physical_pages (first virtual slot).
    // The header's next_unused is advanced to physical_pages+3, as if three
    // other tables also claimed virtual ec pages before we need overflow.
    let simulated_next_unused = physical_pages + 3;
    write_u32_le_at(&mut bytes, 0x0c, simulated_next_unused);

    // Now append large track rows that require an overflow page.
    let mut big_rows = Vec::<Vec<u8>>::new();
    for id in 1..=3u32 {
        let mut row = vec![0u8; 2000];
        row[72..76].copy_from_slice(&id.to_le_bytes());
        big_rows.push(row);
    }
    let _ = append_rows_to_chain_in_place(&mut bytes, 0, &big_rows, PAGE_SIZE).unwrap();

    // The overflow must be placed at simulated_next_unused or later —
    // never at physical_pages (which would conflict with the existing ec pointers).
    let (ec, _first, last) = table_ptr_fields(&bytes, 0).expect("track ptr");
    let overflow_idx = last;
    assert!(
        overflow_idx >= simulated_next_unused,
        "overflow page {overflow_idx} must be >= next_unused {simulated_next_unused} to avoid ec conflict"
    );
    assert!(
        ec > overflow_idx,
        "ec {ec} must point beyond the overflow page {overflow_idx}"
    );
}

#[test]
fn append_rows_oversized_row_rejected() {
    let mut bytes = make_empty_pdb_with_artists(1);
    let huge = vec![0u8; MAX_ROW_LEN + 1];
    let err = append_rows_to_chain_in_place(&mut bytes, 2, &[huge], PAGE_SIZE).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("exceeds page capacity"),
        "expected capacity error, got: {msg}"
    );
}

#[test]
fn append_rows_seq_strictly_greater_when_appending() {
    // Force a fresh-page append by adding many small rows to a small
    // initial chain, then assert the appended page's seq beats the
    // file's previous max.
    let mut data = PdbData::empty();
    for i in 1..=200 {
        data.artists.push(PdbArtistRow {
            id: i as u32,
            name: "BulkArtist".repeat(5),
        });
    }
    data.colors = standard_colors();
    data.columns_raw_rows = standard_columns_raw();
    let mut bytes = write_pdb(&data).expect("write pdb fixture");

    let max_seq_before = max_seqpage_in_file(&bytes, PAGE_SIZE);
    let mut new_rows = Vec::<Vec<u8>>::new();
    for i in 1000..1100 {
        new_rows.push(encode_artist_row(i, &format!("Append-{i}")));
    }
    let outcome = append_rows_to_chain_in_place(&mut bytes, 2, &new_rows, PAGE_SIZE).unwrap();
    if outcome.pages_appended > 0 {
        let max_seq_after = max_seqpage_in_file(&bytes, PAGE_SIZE);
        assert!(
            max_seq_after > max_seq_before,
            "appended page seq must beat existing max ({} -> {})",
            max_seq_before,
            max_seq_after
        );
    }
}

#[test]
fn append_rows_empty_input_is_a_noop() {
    let mut bytes = make_empty_pdb_with_artists(3);
    let pages_before = bytes.len() / PAGE_SIZE;
    let (ec_before, first_before, last_before) = table_ptr_fields(&bytes, 2).expect("artist ptr");
    let outcome = append_rows_to_chain_in_place(&mut bytes, 2, &[], PAGE_SIZE).unwrap();
    let pages_after = bytes.len() / PAGE_SIZE;
    assert_eq!(pages_after, pages_before);
    assert_eq!(outcome.pages_reused, 0);
    assert_eq!(outcome.pages_appended, 0);
    let (ec_after, first_after, last_after) =
        table_ptr_fields(&bytes, 2).expect("artist ptr after");
    assert_eq!(
        (ec_after, first_after, last_after),
        (ec_before, first_before, last_before)
    );
}

fn make_pdb_with_seed(
    artists: Vec<PdbArtistRow>,
    albums: Vec<PdbAlbumRow>,
    genres: Vec<PdbDictRow>,
    labels: Vec<PdbDictRow>,
    keys: Vec<PdbKeyRow>,
    artwork: Vec<PdbArtworkRow>,
    playlist_tree: Vec<PdbPlaylistTreeRow>,
) -> Vec<u8> {
    let mut data = PdbData::empty();
    data.artists = artists;
    data.albums = albums;
    data.genres = genres;
    data.labels = labels;
    data.keys = keys;
    data.artwork = artwork;
    data.playlist_tree = playlist_tree;
    data.colors = standard_colors();
    data.columns_raw_rows = standard_columns_raw();
    write_pdb(&data).expect("write pdb fixture")
}

fn parsed_table_row_count(bytes: &[u8], table_type: u32) -> usize {
    crate::pdb_reader::parse_pdb_bytes(bytes)
        .map(|p| match table_type {
            1 => p.genres.len(),
            2 => p.artists.len(),
            3 => p.albums.len(),
            4 => p.labels.len(),
            5 => p.keys.len(),
            7 => p.playlist_tree.len(),
            13 => p.artworks.len(),
            _ => 0,
        })
        .unwrap_or(0)
}

fn assert_first_unchanged(before: &[u8], after: &[u8], table_type: u32) {
    let (_eb, fb, _lb) = table_ptr_fields(before, table_type).expect("before ptr");
    let (_ea, fa, _la) = table_ptr_fields(after, table_type).expect("after ptr");
    assert_eq!(
        fa, fb,
        "table {table_type}: first must not move (topology-lock)"
    );
}

fn assert_parsed_names(bytes: &[u8], table_type: u32, expected: &[(u32, &str)]) {
    let parsed = crate::pdb_reader::parse_pdb_bytes(bytes).unwrap();
    for (id, name) in expected {
        let actual = match table_type {
            1 => parsed.genres.get(id),
            2 => parsed.artists.get(id),
            3 => parsed.albums.get(id),
            4 => parsed.labels.get(id),
            5 => parsed.keys.get(id),
            _ => panic!("unsupported name table {table_type}"),
        };
        assert_eq!(
            actual.map(String::as_str),
            Some(*name),
            "table {table_type} id {id}"
        );
    }
}

#[test]
fn append_dictionary_rows_round_trip_through_parser() {
    {
        let mut bytes = make_pdb_with_seed(
            vec![],
            vec![],
            vec![PdbDictRow {
                id: 1,
                name: "House".into(),
            }],
            vec![],
            vec![],
            vec![],
            vec![],
        );
        let before = bytes.clone();
        let count_before = parsed_table_row_count(&bytes, 1);
        append_genres_in_place(
            &mut bytes,
            &[(2, "Techno"), (3, "Drum and Bass")],
            PAGE_SIZE,
        )
        .unwrap();
        assert_first_unchanged(&before, &bytes, 1);
        assert_eq!(parsed_table_row_count(&bytes, 1), count_before + 2);
        assert_parsed_names(&bytes, 1, &[(2, "Techno"), (3, "Drum and Bass")]);
    }

    {
        let mut bytes = make_pdb_with_seed(
            vec![],
            vec![],
            vec![],
            vec![PdbDictRow {
                id: 1,
                name: "Hospital".into(),
            }],
            vec![],
            vec![],
            vec![],
        );
        let before = bytes.clone();
        append_labels_in_place(&mut bytes, &[(2, "Metalheadz"), (3, "Hyperdub")], PAGE_SIZE)
            .unwrap();
        assert_first_unchanged(&before, &bytes, 4);
        assert_parsed_names(&bytes, 4, &[(2, "Metalheadz"), (3, "Hyperdub")]);
    }

    {
        let mut bytes = make_pdb_with_seed(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![PdbKeyRow {
                id: 1,
                name: "Am".into(),
            }],
            vec![],
            vec![],
        );
        let before = bytes.clone();
        append_keys_in_place(&mut bytes, &[(2, "Bm"), (3, "C")], PAGE_SIZE).unwrap();
        assert_first_unchanged(&before, &bytes, 5);
        assert_parsed_names(&bytes, 5, &[(2, "Bm"), (3, "C")]);
    }

    {
        let mut bytes = make_pdb_with_seed(
            vec![PdbArtistRow {
                id: 1,
                name: "First".into(),
            }],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        );
        let before = bytes.clone();
        append_artists_in_place(&mut bytes, &[(2, "Second"), (3, "Third")], PAGE_SIZE).unwrap();
        assert_first_unchanged(&before, &bytes, 2);
        assert_parsed_names(&bytes, 2, &[(2, "Second"), (3, "Third")]);
    }

    {
        let mut bytes = make_pdb_with_seed(
            vec![PdbArtistRow {
                id: 1,
                name: "ArtistOne".into(),
            }],
            vec![PdbAlbumRow {
                id: 1,
                name: "First Album".into(),
                artist_id: 1,
            }],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        );
        let before = bytes.clone();
        append_albums_in_place(
            &mut bytes,
            &[(2, "Second Album", 1), (3, "Third Album", 1)],
            PAGE_SIZE,
        )
        .unwrap();
        assert_first_unchanged(&before, &bytes, 3);
        assert_parsed_names(&bytes, 3, &[(2, "Second Album"), (3, "Third Album")]);
    }
}

#[test]
fn append_artwork_round_trips_through_parser() {
    let mut bytes = make_pdb_with_seed(
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![PdbArtworkRow {
            id: 1,
            path: "/PIONEER/Artwork/00001/a1.jpg".into(),
        }],
        vec![],
    );
    let before = bytes.clone();
    append_artwork_in_place(
        &mut bytes,
        &[
            (2, "/PIONEER/Artwork/00001/a2.jpg"),
            (3, "/PIONEER/Artwork/00001/a3.jpg"),
        ],
        PAGE_SIZE,
    )
    .unwrap();
    assert_first_unchanged(&before, &bytes, 13);
    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes).unwrap();
    // Artwork is stored as id -> path map under different field name.
    // Just verify parser found the new ids.
    let artwork_ids: std::collections::HashSet<u32> = parsed.artworks.keys().copied().collect();
    assert!(artwork_ids.contains(&1));
    assert!(artwork_ids.contains(&2));
    assert!(artwork_ids.contains(&3));
}

#[test]
fn append_tracks_round_trips_through_parser() {
    use crate::service::export_helpers::PdbTrackRowData;
    use PdbLayoutProfile;
    let mk = |id: u32, title: &str, path: &str| PdbTrackRowData {
        header_flags_u32: None,
        id,
        artist_id: 1,
        album_id: 1,
        artwork_id: 1,
        key_id: 1,
        genre_id: 0,
        title: title.into(),
        anlz_path: format!("/PIONEER/USBANLZ/P{:03X}/{:08X}/ANLZ0000.DAT", id, id * 16),
        file_path: path.into(),
        content_link: None,
        sample_rate_hz: Some(44100),
        file_size_bytes: Some(1_000_000),
        master_content_id: Some(id * 1000),
        master_db_id: Some(0xDEADBEEF),
        bitrate_kbps: Some(320),
        track_number: Some(id),
        bpm: Some(120.0),
        release_year: Some(2024),
        bit_depth: Some(16),
        duration_seconds: Some(180),
        file_type: Some(1),
        isrc: None,
        date_added: Some("2026-04-30".into()),
        release_date: None,
        dj_comment: None,
        file_name: None,
        publish_track_info_on: None,
        autoload_hotcues_on: None,
        color_id: None,
        rating: None,
    };
    // Seed an existing track so the t00 chain has at least one
    // populated data page to grow.
    let mut data = PdbData::empty();
    data.artists.push(PdbArtistRow {
        id: 1,
        name: "Seed Artist".into(),
    });
    data.albums.push(PdbAlbumRow {
        id: 1,
        name: "Seed Album".into(),
        artist_id: 1,
    });
    data.keys.push(PdbKeyRow {
        id: 1,
        name: "Am".into(),
    });
    data.artwork.push(PdbArtworkRow {
        id: 1,
        path: "/PIONEER/Artwork/00001/a1.jpg".into(),
    });
    data.tracks.push(mk(1, "Seed Track", "/Contents/seed.mp3"));
    data.colors = standard_colors();
    data.columns_raw_rows = standard_columns_raw();
    let mut bytes = write_pdb(&data).expect("seed pdb with tracks");

    let before = bytes.clone();
    let new_tracks: Vec<PdbTrackRowData> = (10..=14)
        .map(|i| {
            mk(
                i,
                &format!("Added Track {i}"),
                &format!("/Contents/added{i}.flac"),
            )
        })
        .collect();
    let outcome = append_tracks_in_place(
        &mut bytes,
        &new_tracks,
        PdbLayoutProfile::DEFAULT,
        PAGE_SIZE,
    )
    .unwrap();
    assert!(
        outcome.pages_reused > 0 || outcome.pages_appended > 0,
        "expected at least one page to be touched"
    );
    assert_first_unchanged(&before, &bytes, 0);

    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes).unwrap();
    let parsed_ids: std::collections::HashSet<u32> = parsed.tracks.iter().map(|t| t.id).collect();
    for added_id in 10..=14 {
        assert!(
            parsed_ids.contains(&added_id),
            "added track {} not visible after in-place append",
            added_id
        );
    }
    assert!(
        parsed_ids.contains(&1),
        "seed track 1 must still be present after additive append"
    );
    // Verify the new track titles round-tripped (string heap fidelity).
    let by_id: std::collections::HashMap<u32, &str> = parsed
        .tracks
        .iter()
        .map(|t| (t.id, t.title.as_str()))
        .collect();
    for added_id in 10..=14 {
        assert_eq!(
            by_id.get(&added_id).copied(),
            Some(format!("Added Track {added_id}").as_str())
        );
    }
}

#[test]
fn additive_diff_patches_existing_track_scalars_in_place() {
    use PdbLayoutProfile;

    let mk_track = |key_id: u32, duration_seconds: Option<u32>| PdbTrackRowData {
        header_flags_u32: None,
        id: 1,
        artist_id: 1,
        album_id: 1,
        artwork_id: 1,
        key_id,
        genre_id: 0,
        title: "Seed Track".into(),
        anlz_path: "/PIONEER/USBANLZ/P001/00000010/ANLZ0000.DAT".into(),
        file_path: "/Contents/seed.mp3".into(),
        content_link: None,
        sample_rate_hz: Some(44100),
        file_size_bytes: Some(1_000_000),
        master_content_id: Some(1000),
        master_db_id: Some(0xDEADBEEF),
        bitrate_kbps: Some(320),
        track_number: Some(1),
        bpm: Some(120.0),
        release_year: Some(2024),
        bit_depth: Some(16),
        duration_seconds,
        file_type: Some(1),
        isrc: None,
        date_added: Some("2026-04-30".into()),
        release_date: None,
        dj_comment: None,
        file_name: None,
        publish_track_info_on: None,
        autoload_hotcues_on: None,
        color_id: None,
        rating: None,
    };

    let mut existing = PdbData::empty();
    existing.artists.push(PdbArtistRow {
        id: 1,
        name: "Seed Artist".into(),
    });
    existing.albums.push(PdbAlbumRow {
        id: 1,
        name: "Seed Album".into(),
        artist_id: 1,
    });
    existing.keys.push(PdbKeyRow {
        id: 1,
        name: "Am".into(),
    });
    existing.artwork.push(PdbArtworkRow {
        id: 1,
        path: "/PIONEER/Artwork/00001/a1.jpg".into(),
    });
    existing.tracks.push(mk_track(1, None));
    existing.colors = standard_colors();
    existing.columns_raw_rows = standard_columns_raw();
    let existing_bytes = write_pdb(&existing).expect("write seed pdb");
    let t00_ptr_before = table_ptr_fields(&existing_bytes, 0).expect("t00 ptr before");

    let mut next = existing.clone();
    next.keys.push(PdbKeyRow {
        id: 2,
        name: "C#".into(),
    });
    next.tracks = vec![mk_track(2, Some(180))];

    let diff = compute_additive_diff(&existing_bytes, &next, vec![]).expect("additive diff");
    assert_eq!(diff.new_keys, vec![(2, "C#".to_string())]);
    assert_eq!(diff.mutated_tracks.len(), 1);
    assert!(diff.mutated_tracks[0].changed_fields.contains(&"key_id"));
    assert!(
        diff.mutated_tracks[0]
            .changed_fields
            .contains(&"duration_seconds")
    );

    let out = apply_additive_diff(&existing_bytes, &diff, PdbLayoutProfile::DEFAULT, PAGE_SIZE)
        .expect("apply additive diff");
    assert_eq!(table_ptr_fields(&out, 0), Some(t00_ptr_before));

    let parsed = crate::pdb_reader::parse_pdb_bytes(&out).expect("parse patched pdb");
    let track = parsed.tracks.iter().find(|t| t.id == 1).expect("track 1");
    assert_eq!(track.key_id, 2);
    assert_eq!(track.duration_seconds, Some(180));
    assert_eq!(parsed.keys.get(&2).map(String::as_str), Some("C#"));
}

#[test]
fn append_playlist_tree_round_trips_through_parser() {
    let mut bytes = make_pdb_with_seed(
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![PdbPlaylistTreeRow {
            id: 1,
            parent_id: 0,
            sort_order: 0,
            is_folder: false,
            name: "Root playlist".into(),
        }],
    );
    let before = bytes.clone();
    append_playlist_tree_in_place(&mut bytes, &[(2, 0, 1, false, "Added playlist")], PAGE_SIZE)
        .unwrap();
    assert_first_unchanged(&before, &bytes, 7);
    // Parser should now see two playlist tree rows.
    assert_eq!(parsed_table_row_count(&bytes, 7), 2);
}

#[test]
fn append_playlist_tree_preserves_transaction_tombstone_page() {
    let mut bytes = make_pdb_with_seed(
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![PdbPlaylistTreeRow {
            id: 1,
            parent_id: 0,
            sort_order: 0,
            is_folder: false,
            name: "Deleted playlist".into(),
        }],
    );
    let (_ec, _first, old_last) = table_ptr_fields(&bytes, 7).unwrap();
    let old_last_off = page_offset(old_last, PAGE_SIZE).unwrap();
    let old_used_s = u16::from_le_bytes(
        bytes[old_last_off + 0x1e..old_last_off + 0x20]
            .try_into()
            .unwrap(),
    ) as usize;
    let old_payload =
        bytes[old_last_off + HEAP_START..old_last_off + HEAP_START + old_used_s].to_vec();

    // Simulate a preserved delete tombstone: row data remains in the page,
    // rowpf marks it inactive, tranrf records the transaction bit.
    bytes[old_last_off + 0x19] = 0;
    let rowpf_off = old_last_off + PAGE_SIZE - 4;
    let tranrf_off = old_last_off + PAGE_SIZE - 2;
    bytes[rowpf_off..rowpf_off + 2].copy_from_slice(&0u16.to_le_bytes());
    bytes[tranrf_off..tranrf_off + 2].copy_from_slice(&1u16.to_le_bytes());
    assert!(page_has_transaction_tombstones(
        &bytes[old_last_off..old_last_off + PAGE_SIZE],
        PAGE_SIZE
    ));

    let outcome =
        append_playlist_tree_in_place(&mut bytes, &[(2, 0, 1, false, "Added playlist")], PAGE_SIZE)
            .unwrap();

    assert_eq!(outcome.pages_reused, 1);
    assert_eq!(outcome.pages_appended, 0);
    assert_eq!(outcome.new_last_page, old_last);
    assert_eq!(table_ptr_fields(&bytes, 7).unwrap().2, old_last);
    assert_eq!(
        &bytes[old_last_off + HEAP_START..old_last_off + HEAP_START + old_used_s],
        old_payload.as_slice(),
        "inactive playlist-tree row payload must stay byte-stable"
    );
    assert_eq!(
        u16::from_le_bytes(bytes[rowpf_off..rowpf_off + 2].try_into().unwrap()),
        0x0002
    );
    assert_eq!(
        u16::from_le_bytes(bytes[tranrf_off..tranrf_off + 2].try_into().unwrap()),
        0x0003
    );
    let used_s = u16::from_le_bytes(
        bytes[old_last_off + 0x1e..old_last_off + 0x20]
            .try_into()
            .unwrap(),
    );
    assert_eq!(
        used_s % 4,
        0,
        "t07 appended row heap must stay 4-byte aligned"
    );

    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes).unwrap();
    assert!(!parsed.playlist_tree.iter().any(|row| row.id == 1));
    assert!(parsed.playlist_tree.iter().any(|row| row.id == 2));
}

fn build_seed_pdb_data() -> PdbData {
    let mut data = PdbData::empty();
    data.artists.push(PdbArtistRow {
        id: 1,
        name: "Existing Artist".into(),
    });
    data.albums.push(PdbAlbumRow {
        id: 1,
        name: "Existing Album".into(),
        artist_id: 1,
    });
    data.genres.push(PdbDictRow {
        id: 1,
        name: "Existing Genre".into(),
    });
    data.keys.push(PdbKeyRow {
        id: 1,
        name: "Am".into(),
    });
    // Seed one track so classifier tests can exercise existing-row
    // dictionary/playlist behavior.
    data.tracks.push(PdbTrackRowData {
        header_flags_u32: None,
        id: 1,
        artist_id: 1,
        album_id: 1,
        artwork_id: 0,
        key_id: 1,
        genre_id: 0,
        title: "Existing Track".into(),
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
        color_id: None,
        rating: None,
    });
    data.colors = standard_colors();
    data.columns_raw_rows = standard_columns_raw();
    data
}

#[test]
fn additive_diff_recognizes_pure_additions() {
    let seed = build_seed_pdb_data();
    let bytes = write_pdb(&seed).expect("write seed");
    // Build a new PdbData that adds: 1 artist, 1 genre, 1 key, 0 album.
    let mut next = seed.clone();
    next.artists.push(PdbArtistRow {
        id: 2,
        name: "Added Artist".into(),
    });
    next.genres.push(PdbDictRow {
        id: 2,
        name: "Added Genre".into(),
    });
    next.keys.push(PdbKeyRow {
        id: 2,
        name: "Bm".into(),
    });
    let diff = compute_additive_diff(&bytes, &next, vec![]).expect("additive ok");
    assert_eq!(diff.new_artists.len(), 1);
    assert_eq!(diff.new_artists[0].0, 2);
    assert_eq!(diff.new_genres.len(), 1);
    assert_eq!(diff.new_keys.len(), 1);
    assert_eq!(diff.new_albums.len(), 0);
    assert_eq!(diff.new_tracks.len(), 0);
    assert_eq!(diff.new_playlist_tree.len(), 0);
}

#[test]
fn additive_diff_rejects_renamed_artist() {
    let seed = build_seed_pdb_data();
    let bytes = write_pdb(&seed).expect("write seed");
    let mut next = seed.clone();
    next.artists[0].name = "Renamed Artist".into();
    let err = compute_additive_diff(&bytes, &next, vec![]).unwrap_err();
    match err {
        NonAdditiveReason::DictionaryStringMutated { table, id, .. } => {
            assert_eq!(table, "artists");
            assert_eq!(id, 1);
        }
        other => panic!("expected DictionaryStringMutated, got {other:?}"),
    }
}

#[test]
fn additive_diff_rejects_removed_track() {
    use crate::service::export_helpers::PdbTrackRowData;
    let mk_track = || PdbTrackRowData {
        header_flags_u32: None,
        id: 42,
        artist_id: 1,
        album_id: 1,
        artwork_id: 0,
        key_id: 1,
        genre_id: 0,
        title: "Will be removed".into(),
        anlz_path: "/PIONEER/USBANLZ/P000/00000001/ANLZ0000.DAT".into(),
        file_path: "/Contents/x.flac".into(),
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
        color_id: None,
        rating: None,
    };
    let mut seed = build_seed_pdb_data();
    seed.tracks.push(mk_track());
    let bytes = write_pdb(&seed).expect("write seed with track");
    let mut next = seed.clone();
    // Remove only the id=42 track; keep the seed track so the diff
    // classifier doesn't trip the empty-existing-PDB gate.
    next.tracks.retain(|t| t.id != 42);
    let err = compute_additive_diff(&bytes, &next, vec![]).unwrap_err();
    assert!(matches!(err, NonAdditiveReason::TrackIdRemoved(42)));
}

#[test]
fn additive_diff_rejects_renamed_playlist() {
    let mut seed = build_seed_pdb_data();
    seed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 1,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: "Original Name".into(),
    });
    let bytes = write_pdb(&seed).expect("write seed with playlist");
    let mut next = seed.clone();
    next.playlist_tree[0].name = "Renamed".into();
    let err = compute_additive_diff(&bytes, &next, vec![]).unwrap_err();
    match err {
        NonAdditiveReason::PlaylistTreeRowMutated { id } => assert_eq!(id, 1),
        other => panic!("expected PlaylistTreeRowMutated, got {other:?}"),
    }
}

#[test]
fn additive_diff_accepts_playlist_tree_sort_order_patches() {
    let mut seed = build_seed_pdb_data();
    seed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 10,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: "Old First".into(),
    });
    seed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 11,
        parent_id: 0,
        sort_order: 1,
        is_folder: false,
        name: "Exported".into(),
    });
    let bytes = write_pdb(&seed).expect("write seed with playlists");

    let mut next = seed.clone();
    next.playlist_tree[0].sort_order = 1;
    next.playlist_tree[1].sort_order = 0;

    let diff = compute_additive_diff(&bytes, &next, vec![]).expect("sort-only t07 patch");
    assert_eq!(diff.playlist_tree_sort_order_patches.len(), 2);
    assert_eq!(diff.new_playlist_tree.len(), 0);
}

#[test]
fn playlist_tree_sort_patch_preserves_inactive_rows() {
    let mut seed = build_seed_pdb_data();
    seed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 10,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: "Active".into(),
    });
    seed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 11,
        parent_id: 0,
        sort_order: 1,
        is_folder: false,
        name: "Inactive Tombstone".into(),
    });
    let mut bytes = write_pdb(&seed).expect("write seed with playlists");
    mark_playlist_tree_row_inactive_for_test(&mut bytes, 11);

    let (_page, inactive_abs, inactive_len, inactive_slot) =
        playlist_tree_row_loc_by_id(&bytes, 11).expect("inactive row loc");
    assert!(!inactive_slot.present);
    let inactive_before = bytes[inactive_abs..inactive_abs + inactive_len].to_vec();

    let patched = patch_playlist_tree_sort_orders_in_place(
        &mut bytes,
        &[PdbPlaylistTreeSortOrderPatch {
            id: 10,
            sort_order: 7,
        }],
        PAGE_SIZE,
    )
    .expect("patch t07 sort");
    assert_eq!(patched, 1);

    let (_page, inactive_abs_after, inactive_len_after, inactive_slot_after) =
        playlist_tree_row_loc_by_id(&bytes, 11).expect("inactive row loc after");
    assert!(!inactive_slot_after.present);
    assert_eq!(inactive_len_after, inactive_len);
    assert_eq!(
        &bytes[inactive_abs_after..inactive_abs_after + inactive_len_after],
        inactive_before.as_slice(),
        "inactive t07 row payload must remain byte-stable"
    );

    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes).expect("parse patched pdb");
    assert_eq!(
        parsed
            .playlist_tree
            .iter()
            .find(|row| row.id == 10)
            .map(|row| row.sort_order),
        Some(7)
    );
    assert!(
        parsed.playlist_tree.iter().all(|row| row.id != 11),
        "inactive t07 row must remain inactive"
    );
}

#[test]
fn try_write_preserves_header_unknown_and_non_decreasing_seqdb() {
    let mut seed = build_seed_pdb_data();
    seed.playlist_tree.push(PdbPlaylistTreeRow {
        id: 10,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: "Existing".into(),
    });
    let mut before = write_pdb(&seed).expect("write seed with playlist");
    before[0x10..0x14].copy_from_slice(&5u32.to_le_bytes());
    before[0x14..0x18].copy_from_slice(&50_000u32.to_le_bytes());

    let mut next = seed.clone();
    next.playlist_tree[0].sort_order = 1;
    next.playlist_tree.push(PdbPlaylistTreeRow {
        id: 11,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: "New First".into(),
    });

    let (after, summary) = try_write_pdb_additive_in_place(&before, &next, vec![], PAGE_SIZE)
        .expect("try write")
        .expect("accepted");
    assert_eq!(summary.new_playlist_tree, 1);
    assert_eq!(read_u32_le_at(&after, 0x10), Some(5));
    assert!(
        read_u32_le_at(&after, 0x14).unwrap_or(0) >= 50_000,
        "seqdb must not decrease"
    );

    let parsed = crate::pdb_reader::parse_pdb_bytes(&after).expect("parse patched pdb");
    assert_eq!(
        parsed
            .playlist_tree
            .iter()
            .find(|row| row.id == 11)
            .map(|row| row.sort_order),
        Some(0)
    );
    assert_eq!(
        parsed
            .playlist_tree
            .iter()
            .find(|row| row.id == 10)
            .map(|row| row.sort_order),
        Some(1)
    );
}

#[test]
fn mutate_tracks_in_place_keeps_header_seqdb_non_decreasing() {
    let seed = build_seed_pdb_data();
    let mut bytes = write_pdb(&seed).expect("write seed");
    bytes[0x14..0x18].copy_from_slice(&50_000u32.to_le_bytes());

    let mut row = seed.tracks[0].clone();
    row.track_number = Some(7);
    let patched = mutate_tracks_in_place(
        &mut bytes,
        &[PdbTrackRowMutation {
            row,
            changed_fields: vec!["track_number"],
        }],
        PdbLayoutProfile::DEFAULT,
        PAGE_SIZE,
    )
    .expect("mutate track");
    assert_eq!(patched, 1);
    assert!(
        read_u32_le_at(&bytes, 0x14).unwrap_or(0) >= 50_000,
        "track mutation must not lower seqdb"
    );
}

fn track_row_loc_by_id(bytes: &[u8], id: u32) -> Option<(u32, usize, usize, PageRowSlot)> {
    let (_ec, first, last) = table_ptr_fields(bytes, 0)?;
    let chain = collect_chain_pages(bytes, PAGE_SIZE, first, last)?;
    for page_idx in chain.into_iter().skip(1) {
        let off = page_offset(page_idx, PAGE_SIZE)?;
        let page = bytes.get(off..off + PAGE_SIZE)?;
        let used_s = read_u16_le_at(page, 0x1e)? as usize;
        let payload_len = used_s.min(PAGE_SIZE.saturating_sub(HEAP_START));
        for slot in parse_page_row_slots(page, PAGE_SIZE) {
            if slot.end > payload_len || slot.start >= slot.end {
                continue;
            }
            let row_abs = off + HEAP_START + slot.start;
            let row_len = slot.end - slot.start;
            if slot.present && row_len >= 136 && read_u32_le_at(bytes, row_abs + 0x48) == Some(id) {
                return Some((page_idx, row_abs, row_len, slot));
            }
        }
    }
    None
}

#[test]
fn mutate_tracks_in_place_zeroes_id_on_relocated_slot() {
    // Regression: when a track's row can't be patched in place (a
    // changed string field no longer fits the original slot length),
    // `mutate_tracks_in_place` relocates it via `mark_row_slot_inactive`
    // + a freshly appended replacement row. The vacated slot must have
    // its id field zeroed — otherwise it's a live duplicate of the
    // relocated row's id, which is exactly what
    // `detect_pdb_tombstoned_playlist_tree_ids` flags as PDB corruption.
    let seed = build_seed_pdb_data();
    let mut bytes = write_pdb(&seed).expect("write seed");

    let (orig_page_idx, orig_row_abs, _orig_row_len, orig_slot) =
        track_row_loc_by_id(&bytes, 1).expect("original track row loc");

    let mut row = seed.tracks[0].clone();
    // Long enough that the encoded slot 19 byte length can't possibly
    // match the original (short) file_name/file_path-derived slot.
    row.file_name = Some("A".repeat(200));
    let patched = mutate_tracks_in_place(
        &mut bytes,
        &[PdbTrackRowMutation {
            row,
            changed_fields: vec!["file_name"],
        }],
        PdbLayoutProfile::DEFAULT,
        PAGE_SIZE,
    )
    .expect("mutate track");
    assert_eq!(patched, 1);

    let off = page_offset(orig_page_idx, PAGE_SIZE).expect("orig page offset");
    let rowpf = read_u16_le_at(&bytes, off + orig_slot.bits_off).expect("rowpf");
    assert_eq!(
        rowpf & orig_slot.bit_mask,
        0,
        "original track slot must be tombstoned (relocated, not patched in place)"
    );
    assert_eq!(
        read_u32_le_at(&bytes, orig_row_abs + 0x48),
        Some(0),
        "tombstoned slot's id field must be zeroed, not left as a duplicate of the \
         relocated row's id"
    );

    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes).expect("parse patched pdb");
    assert_eq!(
        parsed.tracks.iter().filter(|t| t.id == 1).count(),
        1,
        "exactly one active track row should carry id 1 after relocation"
    );
}

/// Simulate a row whose UTF-16 slot isn't padded to a 4-byte aligned offset: insert one byte
/// right before slot 17's (title) content and bump
/// the recorded offsets for slots 17..21 (which all shifted right by that insertion) by 1.
/// The row's fixed header/offset table (bytes 0..136) is untouched since the insertion point
/// is always >= 136.
fn corrupt_track_row_title_alignment(row: &[u8]) -> Vec<u8> {
    let mut row = row.to_vec();
    let off17 = read_u16_le_at(&row, 94 + 2 * 17).expect("slot 17 offset present") as usize;
    assert_eq!(
        off17 % 4,
        0,
        "test precondition: current writer must produce an aligned slot 17"
    );
    assert_eq!(
        row[off17], 0x90,
        "test precondition: non-ASCII title must be UTF-16 encoded"
    );
    row.insert(off17, 0u8);
    for index in 17..21usize {
        let off_pos = 94 + 2 * index;
        let cur = read_u16_le_at(&row, off_pos).expect("slot offset present");
        row[off_pos..off_pos + 2].copy_from_slice(&(cur + 1).to_le_bytes());
    }
    row
}

#[test]
fn track_row_has_misaligned_utf16_slot_detects_bad_offset() {
    let mut seed = build_seed_pdb_data();
    seed.tracks[0].title = "Café Track".into();
    let bytes = write_pdb(&seed).expect("write seed");
    let (_page_idx, row_abs, row_len, _slot) =
        track_row_loc_by_id(&bytes, 1).expect("track row loc");
    let row = &bytes[row_abs..row_abs + row_len];
    assert!(
        !track_row_has_misaligned_utf16_slot(row),
        "current writer must produce an aligned row"
    );

    let corrupted = corrupt_track_row_title_alignment(row);
    assert!(
        track_row_has_misaligned_utf16_slot(&corrupted),
        "corrupted row must be detected as misaligned"
    );
}

#[test]
fn mutate_tracks_in_place_fixes_preexisting_misaligned_title_slot() {
    // Regression: a track row can already be on disk with a misaligned UTF-16 title slot.
    // Re-running
    // `mutate_tracks_in_place` on it — even with the title content itself unchanged — must
    // actually fix the alignment, not silently same-size-patch content back into the same
    // broken slot bounds (which is what happened before the `track_row_has_misaligned_utf16_slot`
    // guard was added: same content -> same encoded length -> "successful" same-size patch
    // that never touches the offset table).
    let mut seed = build_seed_pdb_data();
    seed.tracks[0].title = "Café Track".into();
    let mut bytes = write_pdb(&seed).expect("write seed");

    let (page_idx, _row_abs, _row_len, _slot) =
        track_row_loc_by_id(&bytes, 1).expect("track row loc");
    let off = page_offset(page_idx, PAGE_SIZE).expect("page offset");
    let page = bytes[off..off + PAGE_SIZE].to_vec();
    let mut page_rows = read_present_page_rows(&page, PAGE_SIZE);
    for candidate in page_rows.iter_mut() {
        if read_u32_le_at(candidate, 72) == Some(1) {
            *candidate = corrupt_track_row_title_alignment(candidate);
        }
    }
    assert!(
        rewrite_variable_page_rows_in_place(&mut bytes, page_idx, &page_rows, PAGE_SIZE),
        "repack page with corrupted row"
    );

    let (_pi, corrupted_row_abs, corrupted_row_len, _slot2) =
        track_row_loc_by_id(&bytes, 1).expect("corrupted track row loc");
    assert!(
        track_row_has_misaligned_utf16_slot(
            &bytes[corrupted_row_abs..corrupted_row_abs + corrupted_row_len]
        ),
        "test setup must actually produce a misaligned row"
    );

    let mutation_row = seed.tracks[0].clone();
    let patched = mutate_tracks_in_place(
        &mut bytes,
        &[PdbTrackRowMutation {
            row: mutation_row,
            changed_fields: vec!["title"],
        }],
        PdbLayoutProfile::DEFAULT,
        PAGE_SIZE,
    )
    .expect("mutate track");
    assert_eq!(patched, 1);

    let (_pi2, fixed_row_abs, fixed_row_len, _slot3) =
        track_row_loc_by_id(&bytes, 1).expect("fixed track row loc");
    let fixed_row = &bytes[fixed_row_abs..fixed_row_abs + fixed_row_len];
    assert!(
        !track_row_has_misaligned_utf16_slot(fixed_row),
        "row must be relocated and re-encoded with a correctly aligned slot"
    );

    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes).expect("parse fixed pdb");
    assert_eq!(
        parsed
            .tracks
            .iter()
            .find(|t| t.id == 1)
            .map(|t| t.title.as_str()),
        Some("Café Track"),
        "content must be unchanged, only the encoding"
    );
}

/// Simulate a misaligned album row: insert one byte before the name's content and bump the
/// single-byte `ofs_name_near` field at row offset 21 (see `encode_album_row`).
fn corrupt_album_row_name_alignment(row: &[u8]) -> Vec<u8> {
    let mut row = row.to_vec();
    let name_offset = row[21] as usize;
    assert_eq!(
        name_offset % 4,
        0,
        "test precondition: current writer must produce an aligned album name offset"
    );
    assert_eq!(
        row[name_offset], 0x90,
        "test precondition: non-ASCII album name must be UTF-16 encoded"
    );
    row.insert(name_offset, 0u8);
    row[21] = (name_offset + 1) as u8;
    row
}

#[test]
fn realign_dictionary_rows_in_place_fixes_misaligned_album_name() {
    let mut seed = build_seed_pdb_data();
    seed.albums[0].name = "Café Album".into();
    let mut bytes = write_pdb(&seed).expect("write seed");

    let (_ec, first, last) = table_ptr_fields(&bytes, 3).expect("t03 pointer");
    let chain = collect_chain_pages(&bytes, PAGE_SIZE, first, last).expect("t03 chain");
    let page_idx = chain[1];
    let off = page_offset(page_idx, PAGE_SIZE).expect("page offset");
    let page = bytes[off..off + PAGE_SIZE].to_vec();
    let mut page_rows = read_present_page_rows(&page, PAGE_SIZE);
    for candidate in page_rows.iter_mut() {
        if read_u32_le_at(candidate, 12) == Some(1) {
            *candidate = corrupt_album_row_name_alignment(candidate);
        }
    }
    assert!(
        rewrite_variable_page_rows_in_place(&mut bytes, page_idx, &page_rows, PAGE_SIZE),
        "repack page with corrupted album row"
    );

    // Confirm the corrupted row is actually misaligned before repairing it.
    let find_album_row = |bytes: &[u8]| -> (u32, usize, usize) {
        let (_ec, first, last) = table_ptr_fields(bytes, 3).expect("t03 pointer");
        let chain = collect_chain_pages(bytes, PAGE_SIZE, first, last).expect("t03 chain");
        for page_idx in chain.into_iter().skip(1) {
            let off = page_offset(page_idx, PAGE_SIZE).expect("page offset");
            let page = bytes.get(off..off + PAGE_SIZE).expect("page bytes");
            let used_s = read_u16_le_at(page, 30).unwrap_or(0) as usize;
            let payload_len = used_s.min(PAGE_SIZE.saturating_sub(HEAP_START));
            for slot in parse_page_row_slots(page, PAGE_SIZE) {
                if !slot.present || slot.end > payload_len || slot.start >= slot.end {
                    continue;
                }
                let row_abs = off + HEAP_START + slot.start;
                let row_len = slot.end - slot.start;
                if read_u32_le_at(bytes, row_abs + 12) == Some(1) {
                    return (page_idx, row_abs, row_len);
                }
            }
        }
        panic!("album row 1 not found");
    };

    let (_pi, corrupted_row_abs, corrupted_row_len) = find_album_row(&bytes);
    let corrupted_row = &bytes[corrupted_row_abs..corrupted_row_abs + corrupted_row_len];
    let corrupted_name_offset = corrupted_row[21] as usize;
    assert_eq!(
        corrupted_name_offset % 4,
        1,
        "test setup must actually produce a misaligned album row"
    );

    let fresh_row =
        crate::service::export_helpers::pdb_encoding::encode_album_row(1, "Café Album", 1);
    let patched =
        realign_dictionary_rows_in_place(&mut bytes, 3, 12, &[(1u32, fresh_row)], PAGE_SIZE)
            .expect("realign album row");
    assert_eq!(patched, 1);

    let (_pi2, fixed_row_abs, fixed_row_len) = find_album_row(&bytes);
    let fixed_row = &bytes[fixed_row_abs..fixed_row_abs + fixed_row_len];
    let fixed_name_offset = fixed_row[21] as usize;
    assert_eq!(
        fixed_name_offset % 4,
        0,
        "album name offset must be 4-byte aligned after repair"
    );
    assert_eq!(fixed_row[fixed_name_offset], 0x90);

    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes).expect("parse fixed pdb");
    assert_eq!(
        parsed.albums.get(&1).map(String::as_str),
        Some("Café Album"),
        "content must be unchanged, only the encoding"
    );
}

#[test]
fn same_size_track_row_patch_sanitizes_metadata_strings() {
    use crate::service::export_helpers::pdb_encoding::{
        encode_pdb_track_inline_string, encode_pdb_track_range_string,
    };

    fn slot_bytes(row: &[u8], index: usize) -> &[u8] {
        let start_offset = 94 + index * 2;
        let start = u16::from_le_bytes([row[start_offset], row[start_offset + 1]]) as usize;
        let end = if index == 20 {
            row.len()
        } else {
            let end_offset = 94 + (index + 1) * 2;
            u16::from_le_bytes([row[end_offset], row[end_offset + 1]]) as usize
        };
        &row[start..end]
    }

    let base = PdbTrackRowData {
        header_flags_u32: None,
        id: 1,
        artist_id: 1,
        album_id: 1,
        artwork_id: 0,
        key_id: 0,
        genre_id: 0,
        title: "GoodTitle".into(),
        anlz_path: "/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT".into(),
        file_path: "/Contents/track.mp3".into(),
        content_link: None,
        sample_rate_hz: Some(44_100),
        file_size_bytes: Some(1234),
        master_content_id: None,
        master_db_id: None,
        bitrate_kbps: Some(320),
        track_number: Some(1),
        bpm: Some(120.0),
        release_year: Some(2024),
        bit_depth: Some(16),
        duration_seconds: Some(180),
        file_type: Some(1),
        isrc: None,
        date_added: Some("2026-06-17".into()),
        release_date: None,
        dj_comment: Some("Comment".into()),
        file_name: Some("track.mp3".into()),
        publish_track_info_on: None,
        autoload_hotcues_on: None,
        color_id: None,
        rating: None,
    };
    let encoded =
        encode_track_row_with_profile(&base, PdbLayoutProfile::DEFAULT).expect("encode base track");

    let mut desired = base.clone();
    desired.title = "Good\0Title".into();
    desired.dj_comment = Some("Com\0ment".into());
    desired.file_name = Some("tra\0ck.mp3".into());
    let patched = build_same_size_track_row_patch(
        &encoded,
        &PdbTrackRowMutation {
            row: desired,
            changed_fields: vec!["title", "dj_comment", "file_name"],
        },
    )
    .expect("patch row")
    .expect("same-size sanitized patch");

    assert_eq!(
        slot_bytes(&patched, 17),
        encode_pdb_track_range_string("GoodTitle")
    );
    assert_eq!(
        slot_bytes(&patched, 16),
        encode_pdb_track_inline_string("Comment")
    );
    // Slot 19 includes alignment padding bytes after the filename string
    // (to keep slot 20 at a 4-byte aligned offset), so check the string prefix only.
    assert!(
        slot_bytes(&patched, 19).starts_with(&encode_pdb_track_inline_string("track.mp3")),
        "slot 19 should start with filename bytes"
    );
}

#[test]
fn apply_additive_diff_grows_file_minimally() {
    use crate::service::export_helpers::PdbTrackRowData;
    use PdbLayoutProfile;

    // Build a PDB with 3 tracks + supporting dictionary rows.
    let mk_track = |id: u32, title: &str| PdbTrackRowData {
        header_flags_u32: None,
        id,
        artist_id: 1,
        album_id: 1,
        artwork_id: 1,
        key_id: 1,
        genre_id: 0,
        title: title.into(),
        anlz_path: format!("/PIONEER/USBANLZ/P{:03X}/{:08X}/ANLZ0000.DAT", id, id * 16),
        file_path: format!("/Contents/track{id}.flac"),
        content_link: None,
        sample_rate_hz: Some(44100),
        file_size_bytes: Some(1_000_000),
        master_content_id: Some(id * 1000),
        master_db_id: Some(0xDEADBEEF),
        bitrate_kbps: Some(320),
        track_number: Some(id),
        bpm: Some(120.0),
        release_year: Some(2024),
        bit_depth: Some(16),
        duration_seconds: Some(180),
        file_type: Some(1),
        isrc: None,
        date_added: Some("2026-04-30".into()),
        release_date: None,
        dj_comment: None,
        file_name: None,
        publish_track_info_on: None,
        autoload_hotcues_on: None,
        color_id: None,
        rating: None,
    };
    let mut data = PdbData::empty();
    data.artists.push(PdbArtistRow {
        id: 1,
        name: "Existing Artist".into(),
    });
    data.albums.push(PdbAlbumRow {
        id: 1,
        name: "Existing Album".into(),
        artist_id: 1,
    });
    data.keys.push(PdbKeyRow {
        id: 1,
        name: "Am".into(),
    });
    data.artwork.push(PdbArtworkRow {
        id: 1,
        path: "/PIONEER/Artwork/00001/a1.jpg".into(),
    });
    for i in 1..=3 {
        data.tracks.push(mk_track(i, &format!("Track {i}")));
    }
    data.colors = standard_colors();
    data.columns_raw_rows = standard_columns_raw();
    let existing_bytes = write_pdb(&data).expect("write seed");

    // Build a "next" PdbData that adds 2 new tracks + 1 new artist
    // (used by the new tracks).
    let mut next = data.clone();
    next.artists.push(PdbArtistRow {
        id: 2,
        name: "Added Artist".into(),
    });
    for i in 4..=5 {
        let mut t = mk_track(i, &format!("Added Track {i}"));
        t.artist_id = 2;
        next.tracks.push(t);
    }

    let pages_before = existing_bytes.len() / PAGE_SIZE;
    let diff = compute_additive_diff(&existing_bytes, &next, vec![]).expect("additive");
    assert_eq!(diff.new_tracks.len(), 2);
    assert_eq!(diff.new_artists.len(), 1);

    let new_bytes =
        apply_additive_diff(&existing_bytes, &diff, PdbLayoutProfile::DEFAULT, PAGE_SIZE)
            .expect("apply ok");
    let pages_after = new_bytes.len() / PAGE_SIZE;

    // 2 new track rows should fit on or near the existing tracks
    // page; growth should be at most a couple of pages.
    assert!(
        pages_after <= pages_before + 3,
        "additive growth budget exceeded: {pages_before} -> {pages_after}"
    );

    // Topology-lock: tracks `first_page` must not move.
    let (_eb, first_before, _lb) =
        crate::utils::table_ptr_fields(&existing_bytes, 0).expect("ptr before");
    let (_ea, first_after, _la) = crate::utils::table_ptr_fields(&new_bytes, 0).expect("ptr after");
    assert_eq!(
        first_before, first_after,
        "tracks first_page must not move (topology-lock)"
    );

    // Parser sees both old and new tracks.
    let parsed = crate::pdb_reader::parse_pdb_bytes(&new_bytes).unwrap();
    let ids: std::collections::HashSet<u32> = parsed.tracks.iter().map(|t| t.id).collect();
    for i in 1..=5 {
        assert!(
            ids.contains(&i),
            "track id {i} missing after additive write"
        );
    }
    assert_eq!(
        parsed.artists.get(&2).map(String::as_str),
        Some("Added Artist")
    );

    // Page-header conventions hold.
    let mismatches = crate::pdb_reader::validate_pdb_page_conventions(&new_bytes);
    assert!(
        mismatches.is_empty(),
        "convention mismatches: {:?}",
        mismatches
    );
}

#[test]
fn additive_diff_collects_new_playlist_tree_row() {
    let seed = build_seed_pdb_data();
    let bytes = write_pdb(&seed).expect("write seed");
    let mut next = seed.clone();
    next.playlist_tree.push(PdbPlaylistTreeRow {
        id: 100,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: "New Playlist".into(),
    });
    let diff = compute_additive_diff(&bytes, &next, vec![]).expect("additive");
    assert_eq!(diff.new_playlist_tree.len(), 1);
    assert_eq!(diff.new_playlist_tree[0].0, 100);
    assert_eq!(diff.new_playlist_tree[0].4, "New Playlist");
}

#[test]
fn additive_diff_accepts_empty_existing_pdb_for_first_export() {
    use crate::service::export_helpers::PdbTrackRowData;

    // Existing PDB has no tracks (mirrors the `initialize_usb`
    // template). The classifier must now keep this in the additive
    // path and ask the apply step to synthesize t19 runtime rows.
    let mut empty = PdbData::empty();
    empty.colors = standard_colors();
    empty.columns_raw_rows = standard_columns_raw();
    let bytes = write_pdb(&empty).expect("write empty seed");

    let mut next = empty.clone();
    next.artists.push(PdbArtistRow {
        id: 1,
        name: "Added Artist".into(),
    });
    next.tracks.push(PdbTrackRowData {
        header_flags_u32: None,
        content_link: None,
        sample_rate_hz: None,
        file_size_bytes: None,
        master_content_id: None,
        master_db_id: None,
        id: 1,
        artist_id: 1,
        album_id: 0,
        artwork_id: 0,
        key_id: 0,
        genre_id: 0,
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
        file_name: Some("first.flac".into()),
        publish_track_info_on: None,
        autoload_hotcues_on: None,
        color_id: None,
        rating: None,
        title: "First Track".into(),
        anlz_path: "/PIONEER/USBANLZ/P000/00000001/ANLZ0000.DAT".into(),
        file_path: "/Contents/first.flac".into(),
    });

    let diff = compute_additive_diff(&bytes, &next, vec![]).expect("additive first export");
    assert_eq!(diff.new_tracks.len(), 1);
    assert_eq!(diff.synthesize_t19_runtime_rows, Some(1));
}

#[test]
fn mirror_removal_leaves_tt8_pages_with_single_bit_tranrf() {
    // Regression: a mirror-mode export that removes playlist entries used
    // to leave the rewritten tt=8 (playlist_entries) pages with every
    // footer group's tranrf bitmask at the `rewrite_variable_page_rows_in_place`
    // default of "all rows active" -- `remove_t08_entries_in_place` only
    // re-stamped (u5, num_rl) afterward, never tranrf. Same footer-shape
    // bug class as the tt=19 corruption fix, just for a different table
    // and a different trigger (removal, not synthesis). Fixed by routing
    // through the shared `apply_page_footer_convention`.
    use crate::service::export_helpers::PdbTrackRowData;

    let track = |id: u32| PdbTrackRowData {
        header_flags_u32: None,
        content_link: None,
        sample_rate_hz: None,
        file_size_bytes: None,
        master_content_id: None,
        master_db_id: None,
        id,
        artist_id: 0,
        album_id: 0,
        artwork_id: 0,
        key_id: 0,
        genre_id: 0,
        bitrate_kbps: None,
        track_number: None,
        bpm: None,
        release_year: None,
        bit_depth: None,
        duration_seconds: Some(120),
        file_type: None,
        isrc: None,
        date_added: None,
        release_date: None,
        dj_comment: None,
        file_name: Some(format!("t{id:03}.flac")),
        publish_track_info_on: None,
        autoload_hotcues_on: None,
        color_id: None,
        rating: None,
        title: format!("Track {id:03}"),
        anlz_path: String::new(),
        file_path: format!("/Contents/t{id:03}.flac"),
    };

    let mut seed = PdbData::empty();
    seed.colors = standard_colors();
    seed.columns_raw_rows = standard_columns_raw();
    let before = write_pdb(&seed).expect("write seed");

    let mut full = seed.clone();
    full.playlist_tree.push(PdbPlaylistTreeRow {
        id: 1,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: "P".into(),
    });
    for id in 1..=20u32 {
        full.tracks.push(track(id));
        full.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: id,
            track_id: id,
            playlist_id: 1,
        });
    }
    let all_desired: Vec<T08EntryKey> = full
        .playlist_entries
        .iter()
        .map(|e| T08EntryKey {
            entry_index: e.entry_index,
            track_id: e.track_id,
            playlist_id: e.playlist_id,
        })
        .collect();
    let (after_first, _) = try_write_pdb_additive_in_place(&before, &full, all_desired, PAGE_SIZE)
        .expect("first export")
        .expect("first export accepted");
    assert_eq!(collect_t08_entry_keys(&after_first, PAGE_SIZE).len(), 20);

    // Mirror re-export: keep every track, but only the first 8 playlist
    // entries. The other 12 become `removed_t08_entries`.
    let mut pruned = full.clone();
    pruned.playlist_entries.retain(|e| e.entry_index <= 8);
    let pruned_desired: Vec<T08EntryKey> = pruned
        .playlist_entries
        .iter()
        .map(|e| T08EntryKey {
            entry_index: e.entry_index,
            track_id: e.track_id,
            playlist_id: e.playlist_id,
        })
        .collect();
    let (after_prune, _) =
        try_write_pdb_additive_in_place(&after_first, &pruned, pruned_desired, PAGE_SIZE)
            .expect("mirror export")
            .expect("mirror export accepted");

    assert!(
        crate::pdb_reader::validate_pdb_page_conventions(&after_prune).is_empty(),
        "mirror-pruned pdb violates page conventions: {:?}",
        crate::pdb_reader::validate_pdb_page_conventions(&after_prune)
    );
    assert_eq!(collect_t08_entry_keys(&after_prune, PAGE_SIZE).len(), 8);

    let (_ec, first, last) = table_ptr_fields(&after_prune, 8).expect("t08 ptr");
    let chain = collect_chain_pages(&after_prune, PAGE_SIZE, first, last).expect("t08 chain");
    for &pi in chain.iter().skip(1) {
        let off = page_offset(pi, PAGE_SIZE).unwrap();
        let page = &after_prune[off..off + PAGE_SIZE];
        if page[0x18] == 0 {
            continue;
        }
        let fs = read_page_footer_state(page, PAGE_SIZE).expect("footer state");
        let total_tranrf_bits: u32 = fs.tranrf_by_group.iter().map(|g| g.count_ones()).sum();
        assert_eq!(
            total_tranrf_bits, 1,
            "tt=8 page {pi} must carry exactly one tranrf bit after mirror removal, \
             got groups {:?}",
            fs.tranrf_by_group
        );
    }
}

#[test]
fn additive_first_export_large_library_synthesizes_single_t19_row() {
    use crate::service::export_helpers::PdbTrackRowData;

    // Regression: a first export of a large library (559 tracks, matching
    // the real-world report this test is modeled on) to a freshly
    // initialized USB used to hard-fail with "t19 runtime synthesis: 560
    // rows do not fit on page 40" -- synthesize_t19_runtime_rows_in_place
    // fabricated one placeholder tt=19 row per track with no cap, and a
    // tt=19 page can only ever hold ~96 40-byte rows. There is no real
    // play history to represent on a never-played device, and no
    // reference export (real rekordbox or an independent working DJ
    // export tool) has ever shown tt=19 needing more than one row for
    // a fresh export, so this must synthesize exactly one row regardless
    // of library size, not scale with track count at all.
    let mut empty = PdbData::empty();
    empty.colors = standard_colors();
    empty.columns_raw_rows = standard_columns_raw();
    // Real `initialize_usb` templates always seed exactly one tt=19 row
    // up front (`seed_rb_initialized_history_shape` in usb_utils.rs) --
    // mirror that here so this "before" baseline has a real tt=19 data
    // page to patch, matching what compute_additive_diff actually sees
    // in production instead of the degenerate no-data-page case.
    empty.history_raw_rows = vec![vec![0u8; 40]];
    let bytes = write_pdb(&empty).expect("write empty seed");

    let mut next = empty.clone();
    for id in 1u32..=559 {
        next.artists.push(PdbArtistRow {
            id,
            name: format!("Artist {id}"),
        });
        next.tracks.push(PdbTrackRowData {
            header_flags_u32: None,
            content_link: None,
            sample_rate_hz: None,
            file_size_bytes: None,
            master_content_id: None,
            master_db_id: None,
            id,
            artist_id: id,
            album_id: 0,
            artwork_id: 0,
            key_id: 0,
            genre_id: 0,
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
            file_name: Some(format!("track{id}.flac")),
            publish_track_info_on: None,
            autoload_hotcues_on: None,
            color_id: None,
            rating: None,
            title: format!("Track {id}"),
            anlz_path: format!("/PIONEER/USBANLZ/P000/{id:08}/ANLZ0000.DAT"),
            file_path: format!("/Contents/track{id}.flac"),
        });
    }

    let (out, _summary) = try_write_pdb_additive_in_place(&bytes, &next, vec![], PAGE_SIZE)
        .expect("additive write must not error")
        .expect("additive write must not decline (fall through to unsafe fresh rebuild)");

    let (_, first, last) = table_ptr_fields(&out, 19).expect("tt=19 pointer");
    let chain = collect_chain_pages(&out, PAGE_SIZE, first, last).expect("tt=19 chain");
    // chain[0] = sentinel, chain[1] = the single data page.
    assert_eq!(chain.len(), 2, "t19 synthesis must stay on a single page");
    let off = page_offset(chain[1], PAGE_SIZE).expect("page offset");
    let nrs = out[off + 0x18] as usize;
    assert_eq!(
        nrs, 1,
        "synthesized t19 row count must always be 1, not scaled with track count"
    );
    let rowpf = u16::from_le_bytes([out[off + PAGE_SIZE - 4], out[off + PAGE_SIZE - 3]]);
    let tranrf = u16::from_le_bytes([out[off + PAGE_SIZE - 2], out[off + PAGE_SIZE - 1]]);
    assert_eq!(rowpf, 0b1, "the single row must be marked active");
    assert_eq!(
        tranrf, 0b1,
        "no preceding row exists to also mark in tranrf"
    );
}

#[test]
fn additive_first_export_preserves_template_menu_pages() {
    use crate::service::export_helpers::PdbTrackRowData;

    let mut empty = PdbData::empty();
    empty.colors = standard_colors();
    empty.columns_raw_rows = standard_columns_raw();
    // Real `initialize_usb` templates always seed exactly one tt=19 row
    // up front (`seed_rb_initialized_history_shape` in usb_utils.rs) --
    // mirror that here so synthesize_t19_runtime_rows_in_place has a real
    // data page to patch (see its "KNOWN GAP" comment).
    empty.history_raw_rows = vec![vec![0u8; 40]];
    let before = write_pdb(&empty).expect("write empty seed");
    let t16_before = table_ptr_fields(&before, 16).expect("t16 before");
    let t17_before = table_ptr_fields(&before, 17).expect("t17 before");
    let t18_before = table_ptr_fields(&before, 18).expect("t18 before");

    let mut next = empty.clone();
    next.artists.push(PdbArtistRow {
        id: 1,
        name: "First Artist".into(),
    });
    next.tracks.push(PdbTrackRowData {
        header_flags_u32: None,
        content_link: None,
        sample_rate_hz: None,
        file_size_bytes: None,
        master_content_id: None,
        master_db_id: None,
        id: 1,
        artist_id: 1,
        album_id: 0,
        artwork_id: 0,
        key_id: 0,
        genre_id: 0,
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
        file_name: Some("first.flac".into()),
        publish_track_info_on: None,
        autoload_hotcues_on: None,
        color_id: None,
        rating: None,
        title: "First Track".into(),
        anlz_path: "/PIONEER/USBANLZ/P000/00000001/ANLZ0000.DAT".into(),
        file_path: "/Contents/first.flac".into(),
    });
    next.playlist_tree.push(PdbPlaylistTreeRow {
        id: 1,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: "First Playlist".into(),
    });
    next.playlist_entries.push(PdbPlaylistEntryRow {
        entry_index: 1,
        track_id: 1,
        playlist_id: 1,
    });

    let desired = vec![T08EntryKey {
        entry_index: 1,
        track_id: 1,
        playlist_id: 1,
    }];
    let (after, summary) = try_write_pdb_additive_in_place(&before, &next, desired, PAGE_SIZE)
        .expect("additive first export result")
        .expect("additive first export accepted");

    assert_eq!(summary.new_tracks, 1);
    assert_eq!(summary.new_playlist_tree, 1);
    assert_eq!(table_ptr_fields(&after, 16), Some(t16_before));
    assert_eq!(table_ptr_fields(&after, 17), Some(t17_before));
    assert_eq!(table_ptr_fields(&after, 18), Some(t18_before));
    for page_idx in [33usize, 34, 35, 36, 37, 38] {
        let off = page_idx * PAGE_SIZE;
        assert_eq!(
            &after[off..off + PAGE_SIZE],
            &before[off..off + PAGE_SIZE],
            "menu page {page_idx} changed"
        );
    }

    let (_ec0, first0_before, _last0_before) = table_ptr_fields(&before, 0).unwrap();
    let (_ec0_after, first0_after, last0_after) = table_ptr_fields(&after, 0).unwrap();
    assert_eq!(first0_after, first0_before);
    assert_eq!(last0_after, 2, "template t00 data page should be reused");

    let (_ec19, _first19, last19) = table_ptr_fields(&after, 19).unwrap();
    let off19 = last19 as usize * PAGE_SIZE;
    assert_eq!(
        after[off19 + 0x18],
        1,
        "t19 always synthesizes exactly one row, regardless of track count"
    );
    assert_eq!(read_u16_le_at(&after, off19 + PAGE_SIZE - 4), Some(0x0001));
    assert_eq!(read_u16_le_at(&after, off19 + PAGE_SIZE - 2), Some(0x0001));
}

#[test]
fn additive_first_export_keeps_t07_on_initial_tail_candidate() {
    use crate::service::export_helpers::PdbTrackRowData;

    let mut empty = PdbData::empty();
    empty.colors = standard_colors();
    empty.columns_raw_rows = standard_columns_raw();
    let before = write_pdb(&empty).expect("write empty seed");
    let initial_next_unused = read_u32_le_at(&before, 0x0c).expect("initial next_unused");

    let mut next = empty.clone();
    next.playlist_tree.push(PdbPlaylistTreeRow {
        id: 1,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: "First Playlist".into(),
    });

    for id in 1..=10u32 {
        next.tracks.push(PdbTrackRowData {
            header_flags_u32: None,
            content_link: None,
            sample_rate_hz: None,
            file_size_bytes: None,
            master_content_id: None,
            master_db_id: None,
            id,
            artist_id: 0,
            album_id: 0,
            artwork_id: 0,
            key_id: 0,
            genre_id: 0,
            bitrate_kbps: None,
            track_number: Some(id),
            bpm: None,
            release_year: None,
            bit_depth: None,
            duration_seconds: Some(180),
            file_type: None,
            isrc: None,
            date_added: None,
            release_date: None,
            dj_comment: Some("first export growth row ".repeat(30)),
            file_name: Some(format!("first-export-growth-{id}.flac")),
            publish_track_info_on: None,
            autoload_hotcues_on: None,
            color_id: None,
            rating: None,
            title: format!("First Export Growth Track {id} {}", "x".repeat(240)),
            anlz_path: format!("/PIONEER/USBANLZ/P000/{id:08X}/ANLZ0000.DAT"),
            file_path: format!(
                "/Contents/first-export-growth/{id:02}/{}.flac",
                "x".repeat(180)
            ),
        });
        next.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: id,
            track_id: id,
            playlist_id: 1,
        });
    }

    let desired = next
        .playlist_entries
        .iter()
        .map(|entry| T08EntryKey {
            entry_index: entry.entry_index,
            track_id: entry.track_id,
            playlist_id: entry.playlist_id,
        })
        .collect::<Vec<_>>();
    let (after, summary) = try_write_pdb_additive_in_place(&before, &next, desired, PAGE_SIZE)
        .expect("additive first export result")
        .expect("additive first export accepted");

    assert_eq!(summary.new_playlist_tree, 1);
    let (ec7, first7, last7) = table_ptr_fields(&after, 7).expect("t07 ptr");
    assert_eq!(first7, 15);
    assert_eq!(last7, 16);
    assert_eq!(
        ec7, initial_next_unused,
        "tt=7 must keep the initialized free-tail candidate before other table growth advances next_unused"
    );
    let last7_off = page_offset(last7, PAGE_SIZE).expect("t07 last offset");
    assert_eq!(read_u32_le_at(&after, last7_off + 0x0c), Some(ec7));

    let ec7_off = page_offset(ec7, PAGE_SIZE).expect("t07 ec offset");
    assert!(
        ec7_off + PAGE_SIZE <= after.len(),
        "later growth should materialize the tt=7 empty candidate page"
    );
    assert!(
        after[ec7_off..ec7_off + PAGE_SIZE].iter().all(|b| *b == 0),
        "tt=7 empty candidate page should remain blank"
    );
}

fn reposition_test_track(id: u32) -> crate::service::export_helpers::PdbTrackRowData {
    use crate::service::export_helpers::PdbTrackRowData;
    PdbTrackRowData {
        header_flags_u32: None,
        content_link: None,
        sample_rate_hz: None,
        file_size_bytes: None,
        master_content_id: None,
        master_db_id: None,
        id,
        artist_id: 0,
        album_id: 0,
        artwork_id: 0,
        key_id: 0,
        genre_id: 0,
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
        file_name: Some(format!("reposition-track-{id}.flac")),
        publish_track_info_on: None,
        autoload_hotcues_on: None,
        color_id: None,
        rating: None,
        title: format!("Reposition Track {id}"),
        anlz_path: format!("/PIONEER/USBANLZ/P000/{id:08X}/ANLZ0000.DAT"),
        file_path: format!("/Contents/reposition-track-{id}.flac"),
    }
}

#[test]
fn additive_diff_reposition_does_not_duplicate_or_reappend() {
    // A playlist whose members' entry_index changes (e.g. a reorder)
    // must be patched in place, never removed and re-added -- that's
    // what makes duplication structurally impossible. This is the
    // pure-reposition case: same track set, only order changes.
    let mut seed = PdbData::empty();
    seed.colors = standard_colors();
    seed.columns_raw_rows = standard_columns_raw();
    let before = write_pdb(&seed).expect("write empty seed");

    let mut first = seed.clone();
    for id in 1..=3u32 {
        first.tracks.push(reposition_test_track(id));
    }
    first.playlist_tree.push(PdbPlaylistTreeRow {
        id: 1,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: "Reposition Playlist".into(),
    });
    for id in 1..=3u32 {
        first.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: id,
            track_id: id,
            playlist_id: 1,
        });
    }
    let first_desired = first
        .playlist_entries
        .iter()
        .map(|e| T08EntryKey {
            entry_index: e.entry_index,
            track_id: e.track_id,
            playlist_id: e.playlist_id,
        })
        .collect::<Vec<_>>();
    let (after_first, _) =
        try_write_pdb_additive_in_place(&before, &first, first_desired, PAGE_SIZE)
            .expect("first export result")
            .expect("first export accepted");
    assert_eq!(collect_t08_entry_keys(&after_first, PAGE_SIZE).len(), 3);

    // Reorder: track 3 moves to the front, everything else shifts down.
    let mut reordered = first.clone();
    reordered.playlist_entries = vec![
        PdbPlaylistEntryRow {
            entry_index: 1,
            track_id: 3,
            playlist_id: 1,
        },
        PdbPlaylistEntryRow {
            entry_index: 2,
            track_id: 1,
            playlist_id: 1,
        },
        PdbPlaylistEntryRow {
            entry_index: 3,
            track_id: 2,
            playlist_id: 1,
        },
    ];
    let reordered_desired = reordered
        .playlist_entries
        .iter()
        .map(|e| T08EntryKey {
            entry_index: e.entry_index,
            track_id: e.track_id,
            playlist_id: e.playlist_id,
        })
        .collect::<Vec<_>>();
    let (after_reorder, summary) =
        try_write_pdb_additive_in_place(&after_first, &reordered, reordered_desired, PAGE_SIZE)
            .expect("reorder export result")
            .expect("reorder export accepted");

    assert_eq!(
        summary.new_tracks, 0,
        "reordering existing tracks must not classify any of them as new"
    );
    let mut final_rows = collect_t08_entry_keys(&after_reorder, PAGE_SIZE);
    assert_eq!(
        final_rows.len(),
        3,
        "reorder must not leave duplicate playlist-entry rows behind"
    );
    final_rows.sort_by_key(|k| k.track_id);
    assert_eq!(final_rows[0].track_id, 1);
    assert_eq!(final_rows[0].entry_index, 2);
    assert_eq!(final_rows[1].track_id, 2);
    assert_eq!(final_rows[1].entry_index, 3);
    assert_eq!(final_rows[2].track_id, 3);
    assert_eq!(final_rows[2].entry_index, 1);
}

#[test]
fn additive_diff_repeated_reposition_never_accumulates_duplicates() {
    // Regression test for a real-world defect found on a hardware
    // export: a playlist re-exported many times, each time with a
    // handful of tracks' entry_index shifted (as repeated reordering
    // would produce), must never accumulate duplicate
    // (playlist_id, track_id) rows -- the real drive had one track
    // duplicated 7 times this way.
    let mut seed = PdbData::empty();
    seed.colors = standard_colors();
    seed.columns_raw_rows = standard_columns_raw();
    let before = write_pdb(&seed).expect("write empty seed");

    let track_count = 10u32;
    let mut data = seed.clone();
    for id in 1..=track_count {
        data.tracks.push(reposition_test_track(id));
    }
    data.playlist_tree.push(PdbPlaylistTreeRow {
        id: 1,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: "Repeated Reposition Playlist".into(),
    });

    let mut order: Vec<u32> = (1..=track_count).collect();
    let mut current = before;
    for cycle in 0..8u32 {
        // Rotate the order each cycle so a broad, shifting subset of
        // tracks' entry_index changes every time, the same way
        // incremental local reordering across many sessions would.
        order.rotate_left(1 + (cycle as usize % 3));
        data.playlist_entries = order
            .iter()
            .enumerate()
            .map(|(i, &track_id)| PdbPlaylistEntryRow {
                entry_index: (i as u32) + 1,
                track_id,
                playlist_id: 1,
            })
            .collect();
        let desired = data
            .playlist_entries
            .iter()
            .map(|e| T08EntryKey {
                entry_index: e.entry_index,
                track_id: e.track_id,
                playlist_id: e.playlist_id,
            })
            .collect::<Vec<_>>();
        let (after, _) = try_write_pdb_additive_in_place(&current, &data, desired, PAGE_SIZE)
            .unwrap_or_else(|e| panic!("cycle {cycle} export failed: {e}"))
            .unwrap_or_else(|| panic!("cycle {cycle} export not accepted as additive"));

        let rows = collect_t08_entry_keys(&after, PAGE_SIZE);
        assert_eq!(
            rows.len(),
            track_count as usize,
            "cycle {cycle}: expected exactly {track_count} playlist-entry rows, found {} \
             (duplicates accumulated)",
            rows.len()
        );
        current = after;
    }

    // Final state must match the last cycle's order exactly.
    let mut final_rows = collect_t08_entry_keys(&current, PAGE_SIZE);
    final_rows.sort_by_key(|k| k.entry_index);
    let final_order: Vec<u32> = final_rows.iter().map(|k| k.track_id).collect();
    assert_eq!(final_order, order);
}

/// PdbData fields used by the diff classifier (deferred to a later
/// task) are referenced here only to make sure the imports stay
/// active. Remove this once the diff classifier lands.
#[test]
fn pdb_data_imports_compile() {
    let _ = PdbData::empty();
    let _ = PdbDictRow {
        id: 0,
        name: String::new(),
    };
    let _ = PdbAlbumRow {
        id: 0,
        name: String::new(),
        artist_id: 0,
    };
    let _ = PdbKeyRow {
        id: 0,
        name: String::new(),
    };
    let _ = PdbColorRow {
        id: 0,
        name: String::new(),
    };
    let _ = PdbArtworkRow {
        id: 0,
        path: String::new(),
    };
    let _ = PdbPlaylistTreeRow {
        id: 0,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: String::new(),
    };
    let _ = PdbPlaylistEntryRow {
        entry_index: 0,
        track_id: 0,
        playlist_id: 0,
    };
}
