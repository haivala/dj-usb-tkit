use super::*;

fn read_u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}
fn read_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn raw_indexed_track_row(id: u32) -> Vec<u8> {
    let mut row = vec![0u8; 136];
    row[0..2].copy_from_slice(&0x0024u16.to_le_bytes());
    row[2..4].copy_from_slice(&0xffffu16.to_le_bytes());
    row[72..76].copy_from_slice(&id.to_le_bytes());
    row
}

// ── Sentinel page tests ─────────────────────────────────────────────

#[test]
fn test_sentinel_page_structure() {
    let page = build_sentinel_page(5, 2, 6, true); // page 5, artist table, next=6
    assert_eq!(page.len(), PAGE_SIZE);

    // Common header
    assert_eq!(read_u32_le(&page, 0x00), 0); // zeros
    assert_eq!(read_u32_le(&page, 0x04), 5); // page_index
    assert_eq!(read_u32_le(&page, 0x08), 2); // table_type = artists
    assert_eq!(read_u32_le(&page, 0x0c), 6); // next_page
    assert_eq!(read_u32_le(&page, 0x10), 1); // seqpage
    assert_eq!(page[0x1b], PAGE_FLAGS_INDEX); // page_flags = 0x64

    // Index extended header
    assert_eq!(read_u16_le(&page, 0x20), SENTINEL_UNKNOWNA);
    assert_eq!(read_u16_le(&page, 0x22), SENTINEL_UNKNOWNB);
    assert_eq!(read_u16_le(&page, 0x24), SENTINEL_MAGIC_03EC);
    assert_eq!(read_u32_le(&page, 0x28), 5); // redundant page_index
    assert_eq!(read_u32_le(&page, 0x2c), 6); // redundant next_page
    assert_eq!(read_u32_le(&page, 0x30), SENTINEL_MAGIC_03FFFFFF);
    assert_eq!(read_u32_le(&page, 0x34), 0); // zeros
    assert_eq!(read_u16_le(&page, 0x38), 0); // num_entries
    assert_eq!(read_u16_le(&page, 0x3a), SENTINEL_FIRST_EMPTY);
    assert_eq!(read_u32_le(&page, 0x3c), SENTINEL_EMPTY_ENTRY);
}

// ── Data page footer tests ──────────────────────────────────────────

#[test]
fn test_footer_layout_single_row() {
    let row = encode_artist_row(1, "Test");
    let mut seq = 100u32;
    let (pages, _) = pack_rows_into_pages(2, &[row], &mut seq);
    assert_eq!(pages.len(), 1);
    let page = &pages[0];

    // Footer: [ofs0] [rowpf] [tranrf] at end of page
    let tranrf = read_u16_le(page, PAGE_SIZE - 2);
    let rowpf = read_u16_le(page, PAGE_SIZE - 4);
    let ofs0 = read_u16_le(page, PAGE_SIZE - 6);

    assert_eq!(ofs0, 0, "first row at heap offset 0");
    assert_eq!(rowpf, 0x0001, "1 row present = bit 0");
    assert_eq!(tranrf, 0x0001, "transaction flags match presence");
}

#[test]
fn test_footer_layout_four_rows() {
    let rows: Vec<Vec<u8>> = (1..=4)
        .map(|i| encode_key_row(i, &format!("K{i}")))
        .collect();
    let mut seq = 100u32;
    let (pages, _) = pack_rows_into_pages(5, &rows, &mut seq);
    assert_eq!(pages.len(), 1);
    let page = &pages[0];

    // Footer layout: [ofs3] [ofs2] [ofs1] [ofs0] [rowpf] [tranrf]
    // Reading backward from page end:
    let tranrf = read_u16_le(page, PAGE_SIZE - 2);
    let rowpf = read_u16_le(page, PAGE_SIZE - 4);
    assert_eq!(rowpf, 0x000f, "4 rows = bits 0-3");
    assert_eq!(tranrf, 0x0008, "4 rows u5=1: tranrf = 1<<(3%16) = 0x0008");

    // Offsets are stored with ofs0 closest to rowpf (highest address)
    // and ofs3 at the lowest address
    let ofs0 = read_u16_le(page, PAGE_SIZE - 6);
    let ofs1 = read_u16_le(page, PAGE_SIZE - 8);
    let ofs2 = read_u16_le(page, PAGE_SIZE - 10);
    let ofs3 = read_u16_le(page, PAGE_SIZE - 12);
    assert_eq!(ofs0, 0);
    assert!(ofs1 > ofs0, "ofs1={ofs1} should be > ofs0={ofs0}");
    assert!(ofs2 > ofs1);
    assert!(ofs3 > ofs2);

    // Check all offsets are 4-byte aligned
    assert_eq!(ofs0 as usize % 4, 0);
    assert_eq!(ofs1 as usize % 4, 0);
    assert_eq!(ofs2 as usize % 4, 0);
    assert_eq!(ofs3 as usize % 4, 0);
}

// ── Page header field tests ─────────────────────────────────────────

#[test]
fn test_data_page_header_fields() {
    let row = encode_artist_row(1, "Test");
    let mut seq = 100u32;
    let (pages, _) = pack_rows_into_pages(2, std::slice::from_ref(&row), &mut seq);
    let page = &pages[0];

    // table_type at 0x08
    assert_eq!(read_u32_le(page, 0x08), 2);

    // Packed row counts at 0x18-0x1a
    let packed = read_u32_le(page, 0x18) & 0x00FF_FFFF;
    let num_row_offsets = packed & 0x1FFF;
    let num_rows = (packed >> 13) & 0x7FF;
    assert_eq!(num_row_offsets, 1);
    assert_eq!(num_rows, 1);

    // page_flags
    assert_eq!(page[0x1b], PAGE_FLAGS_DATA); // 0x24 for non-track

    // used_size at 0x1e
    let used = read_u16_le(page, 0x1e);
    assert!(used > 0, "used_size should be nonzero");
    assert_eq!(used as usize, align4(row.len()));

    // Verify row data at heap start
    assert_eq!(&page[HEAP_START..HEAP_START + row.len()], &row[..]);
}

#[test]
fn test_track_page_uses_0x34_flags() {
    // Tracks (type 0) should use page_flags 0x34
    let track = PdbTrackRowData {
        composer_id: 0,
        label_id: 0,
        remixer_id: 0,
        header_flags_u32: None,
        id: 1,
        artist_id: 0,
        album_id: 0,
        artwork_id: 0,
        key_id: 0,
        genre_id: 0,
        title: "Test".into(),
        anlz_path: String::new(),
        file_path: "/test.mp3".into(),
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
        duration_seconds: None,
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
    let row = encode_track_row_with_profile(&track, PdbLayoutProfile::Current).unwrap();
    let mut seq = 100u32;
    let (pages, _) = pack_rows_into_pages(0, &[row], &mut seq);
    assert_eq!(pages[0][0x1b], PAGE_FLAGS_DATA_TRACK);
}

fn minimal_track_row(title: &str, file_name: &str, file_path: &str) -> PdbTrackRowData {
    PdbTrackRowData {
        composer_id: 0,
        label_id: 0,
        remixer_id: 0,
        header_flags_u32: None,
        id: 1,
        artist_id: 0,
        album_id: 0,
        artwork_id: 0,
        key_id: 0,
        genre_id: 0,
        title: title.into(),
        anlz_path: String::new(),
        file_path: file_path.into(),
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
        duration_seconds: None,
        file_type: None,
        isrc: None,
        date_added: None,
        release_date: None,
        dj_comment: None,
        file_name: Some(file_name.into()),
        publish_track_info_on: None,
        autoload_hotcues_on: None,
        color_id: None,
        rating: None,
    }
}

#[test]
fn test_non_ascii_title_slot_is_4byte_aligned() {
    // A non-ASCII title (slot 17) is UTF-16 encoded (0x90 header). Only slot 20 used to be
    // padded to a 4-byte row-relative offset; slot 17 wasn't, which produced a MIPS
    // unaligned-read freeze on real CDJ hardware while listing tracks (title is what the
    // browse/list view reads), independent of any load-time slot-20 alignment.
    let track = minimal_track_row("We\u{2019}re Guilty", "track.mp3", "/track.mp3");
    let row = encode_track_row_with_profile(&track, PdbLayoutProfile::Current).unwrap();
    let title_offset = read_u16_le_at(&row, 94 + 17 * 2).unwrap() as usize;
    assert_eq!(row[title_offset], 0x90, "title must be UTF-16 encoded");
    assert_eq!(
        title_offset % 4,
        0,
        "non-ASCII title slot must start at a 4-byte aligned row offset, was {title_offset}"
    );
}

#[test]
fn test_non_ascii_filename_slot_is_4byte_aligned() {
    // Same class of bug as the title case above, but for slot 19 (filename). Also verify
    // slot 20 (path) — which sits immediately after — keeps its own alignment guarantee.
    let track = minimal_track_row(
        "Title",
        "We\u{2019}re Guilty.mp3",
        "/Contents/Artist/We\u{2019}re Guilty.mp3",
    );
    let row = encode_track_row_with_profile(&track, PdbLayoutProfile::Current).unwrap();
    let filename_offset = read_u16_le_at(&row, 94 + 19 * 2).unwrap() as usize;
    assert_eq!(
        row[filename_offset], 0x90,
        "filename must be UTF-16 encoded"
    );
    assert_eq!(
        filename_offset % 4,
        0,
        "non-ASCII filename slot must start at a 4-byte aligned row offset, was {filename_offset}"
    );
    let path_offset = read_u16_le_at(&row, 94 + 20 * 2).unwrap() as usize;
    assert_eq!(
        path_offset % 4,
        0,
        "slot 20 must still be 4-byte aligned, was {path_offset}"
    );
}

// ── Row encoding tests ──────────────────────────────────────────────

#[test]
fn test_artist_row_near_variant() {
    let row = encode_artist_row(1, "Abc");
    // Near variant: subtype=0x0060, id at 4, const 3 at 8, ofs at 9. rekordbox puts a short
    // ASCII name at 10 (Engine DJ's PDB import reads it only there).
    assert_eq!(read_u16_le(&row, 0), 0x0060);
    assert_eq!(read_u32_le(&row, 4), 1);
    assert_eq!(row[8], 3);
    assert_eq!(row[9], 10); // ofs_name_near
    // Short string "Abc": lk=3*2+3=9, then 'A','b','c'
    assert_eq!(row[10], 9);
    assert_eq!(&row[11..14], b"Abc");
    assert_eq!(row.len(), 14);
}

#[test]
fn test_artist_row_near_variant_utf16_name_is_aligned() {
    // A UTF-16 name goes at the 4-byte aligned 12 (2 zero bytes at 10-11), as rekordbox
    // writes it; MIPS-based player hardware freezes on one at 10.
    let row = encode_artist_row(1, "Test Ärtist");
    assert_eq!(row[9], 12);
    assert_eq!(&row[10..12], &[0, 0]);
    assert_eq!(row[12], 0x90);
}

#[test]
fn test_album_row_near_variant() {
    let row = encode_album_row(1, "Test Album", 1);
    // Near variant: subtype=0x0080
    assert_eq!(read_u16_le(&row, 0), 0x0080);
    assert_eq!(read_u32_le(&row, 8), 1); // artist_id
    assert_eq!(read_u32_le(&row, 12), 1); // id
    assert_eq!(row[20], 3); // const 3 at byte 0x14
    assert_eq!(row[21], 22); // ofs_name_near = 22
    // Name string at offset 22
    let name_lk = row[22];
    assert!(name_lk % 2 == 1, "short ASCII format");
}

#[test]
fn test_key_row_has_duplicate_id() {
    let row = encode_key_row(7, "Am");
    assert_eq!(read_u32_le(&row, 0), 7); // id
    assert_eq!(read_u32_le(&row, 4), 7); // duplicate id
    // Short string "Am": lk=2*2+3=7
    assert_eq!(row[8], 7);
    assert_eq!(&row[9..11], b"Am");
}

#[test]
fn test_string_encoding_short_ascii() {
    let s = encode_pdb_string("Gm");
    // Short: lk=2*2+3=7, data="Gm"
    assert_eq!(s, vec![7, b'G', b'm']);
}

#[test]
fn test_string_encoding_long_ascii() {
    let long = "A".repeat(200);
    let s = encode_pdb_string(&long);
    assert_eq!(s[0], 0x40); // long ASCII flag
    let total_len = u16::from_le_bytes([s[1], s[2]]) as usize;
    assert_eq!(total_len, 4 + 200);
    assert_eq!(s[3], 0); // pad
    assert_eq!(&s[4..], long.as_bytes());
}

#[test]
fn test_string_encoding_non_ascii_uses_utf16le() {
    // "ä" = U+00E4; UTF-16LE = [0xE4, 0x00]; total_len = 4 + 2 = 6
    let s = encode_pdb_string("ä");
    assert_eq!(s[0], 0x90, "non-ASCII must use UTF-16LE header 0x90");
    let total_len = u16::from_le_bytes([s[1], s[2]]) as usize;
    assert_eq!(total_len, 6);
    assert_eq!(s[3], 0);
    assert_eq!(&s[4..], &[0xE4u8, 0x00], "ä in UTF-16LE");
}

#[test]
fn test_string_encoding_non_ascii_roundtrip() {
    let title = "Häiriö";
    let artist = "Päivi Tähti";
    let album = "Yö-albumi";

    let mut data = PdbData::empty();
    data.artists.push(PdbArtistRow {
        id: 1,
        name: artist.into(),
    });
    data.albums.push(PdbAlbumRow {
        id: 1,
        name: album.into(),
        artist_id: 1,
    });
    data.tracks.push(PdbTrackRowData {
        composer_id: 0,
        label_id: 0,
        remixer_id: 0,
        header_flags_u32: None,
        id: 1,
        artist_id: 1,
        album_id: 1,
        artwork_id: 0,
        key_id: 0,
        genre_id: 0,
        title: title.into(),
        anlz_path: "/PIONEER/USBANLZ/P000/00000001/ANLZ0000".into(),
        file_path: "/Contents/test.wav".into(),
        bpm: Some(120.0),
        duration_seconds: Some(180),
        track_number: None,
        file_type: None,
        file_name: None,
        content_link: None,
        sample_rate_hz: None,
        file_size_bytes: None,
        master_content_id: None,
        master_db_id: None,
        bitrate_kbps: None,
        release_year: None,
        bit_depth: None,
        isrc: None,
        date_added: None,
        release_date: None,
        dj_comment: None,
        publish_track_info_on: None,
        autoload_hotcues_on: None,
        color_id: None,
        rating: None,
    });

    let bytes = write_pdb(&data).unwrap();
    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes).unwrap();

    let t = parsed.tracks.first().expect("track row");
    assert_eq!(t.title, title, "title must roundtrip through UTF-16LE");
    assert_eq!(
        parsed.artists.get(&1).map(String::as_str),
        Some(artist),
        "artist must roundtrip through UTF-16LE"
    );
    assert_eq!(
        parsed.albums.get(&1).map(String::as_str),
        Some(album),
        "album must roundtrip through UTF-16LE"
    );
}

// ── Row alignment tests ─────────────────────────────────────────────

#[test]
fn pack_rows_skips_oversized_row_instead_of_panicking() {
    // A row that doesn't fit on a page should be skipped, not panic.
    let oversized: Vec<u8> = vec![0u8; PAGE_SIZE]; // way bigger than HEAP_START..footer
    let small = encode_key_row(1, "Am");
    let rows = vec![oversized, small.clone()];
    let mut seq = 100u32;
    let (pages, warnings) = pack_rows_into_pages(5, &rows, &mut seq);
    // Only the small row should make it through.
    assert_eq!(pages.len(), 1, "should produce one page from the small row");
    assert_eq!(warnings.len(), 1, "expected one structured warning");
    assert!(
        warnings[0].contains("oversized row"),
        "warning text should mention oversized row, got: {}",
        warnings[0]
    );
    // Writing again with the small row alone should match (modulo the seq
    // header, which advances each call).
    let mut seq2 = 100u32;
    let (pages_alone, _) = pack_rows_into_pages(5, &[small], &mut seq2);
    let mut a = pages[0].clone();
    let mut b = pages_alone[0].clone();
    a[0x10..0x14].fill(0);
    b[0x10..0x14].fill(0);
    assert_eq!(a, b);
}

#[test]
fn test_row_offsets_are_4byte_aligned() {
    // Key rows "Ab" = 11 bytes each, should be padded to 12
    let rows: Vec<Vec<u8>> = (1..=3).map(|i| encode_key_row(i, "Ab")).collect();
    let mut seq = 100u32;
    let (pages, _) = pack_rows_into_pages(5, &rows, &mut seq);
    let page = &pages[0];

    // Footer: [ofs2] [ofs1] [ofs0] [rowpf] [tranrf]
    let ofs0 = read_u16_le(page, PAGE_SIZE - 6);
    let ofs1 = read_u16_le(page, PAGE_SIZE - 8);
    let ofs2 = read_u16_le(page, PAGE_SIZE - 10);

    assert_eq!(ofs0, 0);
    assert_eq!(ofs1, 12); // 11 bytes padded to 12
    assert_eq!(ofs2, 24);
}

#[test]
fn test_indexed_rows_get_page_local_index_shift() {
    let rows = vec![
        raw_indexed_track_row(1),
        raw_indexed_track_row(2),
        raw_indexed_track_row(3),
    ];
    let mut seq = 100u32;
    let (pages, warnings) = pack_rows_into_pages(0, &rows, &mut seq);
    assert!(warnings.is_empty());
    let page = &pages[0];

    let ofs0 = read_u16_le(page, PAGE_SIZE - 6) as usize;
    let ofs1 = read_u16_le(page, PAGE_SIZE - 8) as usize;
    let ofs2 = read_u16_le(page, PAGE_SIZE - 10) as usize;

    assert_eq!(read_u16_le(page, HEAP_START + ofs0 + 2), 0);
    assert_eq!(read_u16_le(page, HEAP_START + ofs1 + 2), 32);
    assert_eq!(read_u16_le(page, HEAP_START + ofs2 + 2), 64);
}

// ── Full PDB assembly tests ─────────────────────────────────────────

#[test]
fn test_empty_pdb_structure() {
    let data = PdbData::empty();
    let bytes = write_pdb(&data).unwrap();

    // 1 header page + 20 tables × 2 pages = 41 pages
    assert_eq!(bytes.len(), 41 * PAGE_SIZE);

    // File header
    assert_eq!(read_u32_le(&bytes, 0x04), PAGE_SIZE as u32);
    assert_eq!(read_u32_le(&bytes, 0x08), NUM_TABLES);

    // Verify all 20 table pointers
    for i in 0..20 {
        let off = TABLE_POINTERS_OFFSET + i * TABLE_POINTER_SIZE;
        let tt = read_u32_le(&bytes, off);
        let _ec = read_u32_le(&bytes, off + 4);
        let first = read_u32_le(&bytes, off + 8);
        let last = read_u32_le(&bytes, off + 12);

        assert_eq!(tt, i as u32, "table {i} type");
        // Empty table: first = last = sentinel page
        assert_eq!(first, last, "empty table {i}: first should equal last");

        // Sentinel page should have correct flags
        let sentinel_page = &bytes[first as usize * PAGE_SIZE..];
        assert_eq!(sentinel_page[0x1b], PAGE_FLAGS_INDEX);
        assert_eq!(read_u32_le(sentinel_page, 0x08), i as u32); // table_type
    }
}

#[test]
fn test_pdb_with_one_artist() {
    let mut data = PdbData::empty();
    data.artists.push(PdbArtistRow {
        id: 1,
        name: "Test Artist".into(),
    });

    let bytes = write_pdb(&data).unwrap();

    // Artist table (type 2) should have first != last
    let off = TABLE_POINTERS_OFFSET + 2 * TABLE_POINTER_SIZE;
    let first = read_u32_le(&bytes, off + 8);
    let last = read_u32_le(&bytes, off + 12);
    assert_ne!(first, last, "artist table should have sentinel + data page");

    // Sentinel page
    let sentinel = &bytes[first as usize * PAGE_SIZE..];
    assert_eq!(sentinel[0x1b], PAGE_FLAGS_INDEX);
    assert_eq!(read_u32_le(sentinel, 0x0c), last); // next = data page

    // Data page
    let data_page = &bytes[last as usize * PAGE_SIZE..];
    assert_eq!(data_page[0x1b], PAGE_FLAGS_DATA);
    let packed = read_u32_le(data_page, 0x18) & 0x00FF_FFFF;
    let num_rows = (packed >> 13) & 0x7FF;
    assert_eq!(num_rows, 1);

    // Verify row data
    let row_data = &data_page[HEAP_START..];
    assert_eq!(read_u16_le(row_data, 0), 0x0060); // artist near subtype
    assert_eq!(read_u32_le(row_data, 4), 1); // id = 1
}

/// Synthetic golden fixture: build a PdbData touching every table
/// type whose footer fields the writer is responsible for, write it,
/// and assert each data page's `(u5, num_rl)` matches the documented
/// per-table convention plus that data-page seq values vary (i.e. are
/// not all collapsed to 1, which player firmware appears to reject).
///
/// This is the regression gate for the comm-error class of bugs:
/// any future change to `build_data_page` that reverts to `num_rl=0`
/// or all-equal seq must fail this test.
#[test]
fn pdb_page_conventions_synthetic_fixture() {
    use crate::pdb_reader::{collect_pdb_data_page_seqs, validate_pdb_page_conventions};

    let mut data = PdbData::empty();

    // Force at least one row in every table type the convention covers.
    data.tracks.push(PdbTrackRowData {
        composer_id: 0,
        label_id: 0,
        remixer_id: 0,
        header_flags_u32: None,
        id: 1,
        artist_id: 1,
        album_id: 1,
        artwork_id: 1,
        key_id: 1,
        genre_id: 0,
        title: "T".into(),
        anlz_path: String::new(),
        file_path: "/t.mp3".into(),
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
        duration_seconds: None,
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
    data.artists.push(PdbArtistRow {
        id: 1,
        name: "A".into(),
    });
    data.albums.push(PdbAlbumRow {
        id: 1,
        name: "Al".into(),
        artist_id: 1,
    });
    data.genres.push(PdbDictRow {
        id: 1,
        name: "G".into(),
    });
    data.labels.push(PdbDictRow {
        id: 1,
        name: "L".into(),
    });
    data.keys.push(PdbKeyRow {
        id: 1,
        name: "C".into(),
    });
    data.colors = standard_colors();
    data.artwork.push(PdbArtworkRow {
        id: 1,
        path: "/PIONEER/Artwork/00001/a1.jpg".into(),
    });
    data.playlist_tree.push(PdbPlaylistTreeRow {
        id: 1,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: "P".into(),
    });
    data.playlist_entries.push(PdbPlaylistEntryRow {
        entry_index: 1,
        track_id: 1,
        playlist_id: 1,
    });
    data.columns_raw_rows = standard_columns_raw();

    let bytes = write_pdb(&data).expect("write synthetic pdb");

    // (1) Footer-field convention check.
    let mismatches = validate_pdb_page_conventions(&bytes);
    assert!(
        mismatches.is_empty(),
        "page-header convention violations:\n{}",
        mismatches
            .iter()
            .map(|m| m.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );

    // (2) seq variation check: data pages must not all share seq=1.
    let data_seqs = collect_pdb_data_page_seqs(&bytes);
    assert!(
        data_seqs.len() >= 2,
        "expected several data pages in fixture, found {}",
        data_seqs.len()
    );
    let unique_seqs: std::collections::HashSet<u32> = data_seqs.iter().copied().collect();
    assert!(
        unique_seqs.len() > 1,
        "all data pages collapsed to a single seq value {:?} — player firmware rejects this",
        data_seqs
    );
    assert!(
        !data_seqs.iter().all(|&s| s == 1),
        "all data pages have seq=1; expected an incrementing per-page counter"
    );

    // (3) tt=8 (playlist_entries) page must specifically use u5=1 — this
    // was the failing field on the broken USB and the most strictly
    // observed value across reference exports.
    for chunk in bytes.as_chunks::<PAGE_SIZE>().0.iter().skip(1) {
        let stored_idx = read_u32_le(chunk, 0x04);
        if stored_idx == 0 {
            continue;
        }
        if chunk[0x1b] == PAGE_FLAGS_INDEX {
            continue;
        }
        let tt = read_u32_le(chunk, 0x08);
        if tt != 8 {
            continue;
        }
        let nrs = chunk[0x18];
        if nrs == 0 {
            continue;
        }
        let u5 = read_u16_le(chunk, 0x20);
        assert_eq!(
            u5, 1,
            "tt=8 page at index {} has u5={} — convention requires u5=1 \
             for playlist_entries; older hardware can reject u5=trc with a comm error",
            stored_idx, u5
        );
    }
}

#[test]
fn test_growth_does_not_shift_legacy_history_block() {
    let mut data = PdbData::empty();
    data.columns_raw_rows = standard_columns_raw();
    data.history_playlists = (1..=22)
        .map(|id| PdbDictRow {
            id,
            name: format!("History {id}"),
        })
        .collect();
    data.history_entries = (1..=17)
        .map(|id| PdbHistoryEntryRow {
            track_id: id,
            playlist_id: 1,
            entry_index: id,
        })
        .collect();
    data.history_raw_rows.push(vec![
        0x80, 0x02, 0x80, 0x01, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x15, 0x17, 0x32, 0x30,
        0x32, 0x36, 0x2d, 0x30, 0x34, 0x2d, 0x31, 0x35, 0x01, 0x01, 0x0b, 0x31, 0x30, 0x30, 0x30,
        0x00, 0x00,
    ]);
    data.tracks = (1..=250)
        .map(|id| PdbTrackRowData {
            composer_id: 0,
            label_id: 0,
            remixer_id: 0,
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
            duration_seconds: None,
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
            title: format!("Track {id}"),
            anlz_path: format!("/PIONEER/USBANLZ/P{id:08}/ANLZ0000.DAT"),
            file_path: format!("/Contents/Artist/Album/T{id}.mp3"),
        })
        .collect();
    data.playlist_tree.push(PdbPlaylistTreeRow {
        id: 1,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: "Big Playlist".into(),
    });
    data.playlist_entries = (1..=250)
        .map(|id| PdbPlaylistEntryRow {
            entry_index: id,
            track_id: id,
            playlist_id: 1,
        })
        .collect();

    let bytes = write_pdb(&data).unwrap();

    let off16 = TABLE_POINTERS_OFFSET + 16 * TABLE_POINTER_SIZE;
    let off17 = TABLE_POINTERS_OFFSET + 17 * TABLE_POINTER_SIZE;
    let off18 = TABLE_POINTERS_OFFSET + 18 * TABLE_POINTER_SIZE;
    let off19 = TABLE_POINTERS_OFFSET + 19 * TABLE_POINTER_SIZE;
    assert_eq!(read_u32_le(&bytes, off16 + 8), 33);
    assert_eq!(read_u32_le(&bytes, off16 + 12), 34);
    assert_eq!(read_u32_le(&bytes, off17 + 8), 35);
    assert_eq!(read_u32_le(&bytes, off17 + 12), 36);
    assert_eq!(read_u32_le(&bytes, off18 + 8), 37);
    assert_eq!(read_u32_le(&bytes, off18 + 12), 38);
    assert_eq!(read_u32_le(&bytes, off19 + 8), 39);
    assert_eq!(read_u32_le(&bytes, off19 + 12), 40);
    assert!(bytes.len() > 41 * PAGE_SIZE);
}

#[test]
fn test_pdb_round_trip_parser() {
    // Write a PDB with known data, then parse it back
    let mut data = PdbData::empty();
    data.artists.push(PdbArtistRow {
        id: 1,
        name: "Artist A".into(),
    });
    data.albums.push(PdbAlbumRow {
        id: 1,
        name: "Album One".into(),
        artist_id: 1,
    });
    data.keys.push(PdbKeyRow {
        id: 1,
        name: "Gm".into(),
    });
    data.keys.push(PdbKeyRow {
        id: 2,
        name: "Fm".into(),
    });
    data.artwork.push(PdbArtworkRow {
        id: 1,
        path: "/PIONEER/USBANLZ/art.jpg".into(),
    });
    data.playlist_tree.push(PdbPlaylistTreeRow {
        id: 1,
        parent_id: 0,
        sort_order: 0,
        is_folder: false,
        name: "Test Playlist".into(),
    });
    data.playlist_entries.push(PdbPlaylistEntryRow {
        entry_index: 0,
        track_id: 1,
        playlist_id: 1,
    });

    let bytes = write_pdb(&data).unwrap();

    // Parse it back using the existing PDB parser
    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes).unwrap();

    assert_eq!(parsed.artists.get(&1).map(String::as_str), Some("Artist A"));
    assert_eq!(parsed.albums.get(&1).map(String::as_str), Some("Album One"));
    assert_eq!(parsed.keys.get(&1).map(String::as_str), Some("Gm"));
    assert_eq!(parsed.keys.get(&2).map(String::as_str), Some("Fm"));
    assert_eq!(
        parsed.artworks.get(&1).map(String::as_str),
        Some("/PIONEER/USBANLZ/art.jpg")
    );
    assert_eq!(parsed.playlist_tree.len(), 1);
    assert_eq!(parsed.playlist_tree[0].name, "Test Playlist");
    assert_eq!(parsed.playlist_entries.len(), 1);
}

#[test]
fn test_pdb_round_trip_non_ascii_album_name() {
    // Non-ASCII album names are written at a different (4-byte-aligned)
    // row offset than ASCII names, to avoid a MIPS unaligned-read
    // freeze on real CDJ hardware. The reader must follow the
    // self-describing offset rather than assuming a fixed one — this
    // exercises writer and reader together, not just the writer's byte
    // layout in isolation.
    let mut data = PdbData::empty();
    data.artists.push(PdbArtistRow {
        id: 1,
        name: "Artist A".into(),
    });
    data.albums.push(PdbAlbumRow {
        id: 1,
        name: "Álbum Ñoño".into(),
        artist_id: 1,
    });

    let bytes = write_pdb(&data).unwrap();
    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes).unwrap();

    assert_eq!(
        parsed.albums.get(&1).map(String::as_str),
        Some("Álbum Ñoño")
    );
}

#[test]
fn test_page_chain_linking() {
    // Create enough key rows to overflow one page
    let mut data = PdbData::empty();
    for i in 1..=500 {
        data.keys.push(PdbKeyRow {
            id: i,
            name: format!("Key{i:04}"),
        });
    }

    let bytes = write_pdb(&data).unwrap();

    // Key table (type 5) should have multiple data pages
    let off = TABLE_POINTERS_OFFSET + 5 * TABLE_POINTER_SIZE;
    let first = read_u32_le(&bytes, off + 8);
    let last = read_u32_le(&bytes, off + 12);
    assert!(last > first + 1, "should have >1 data page for 500 keys");

    // Walk the chain from sentinel
    let sentinel = &bytes[first as usize * PAGE_SIZE..];
    let mut current = read_u32_le(sentinel, 0x0c); // sentinel's next_page
    let mut page_count = 0;

    while current <= last {
        let page = &bytes[current as usize * PAGE_SIZE..];
        assert_eq!(read_u32_le(page, 0x04), current); // page_index matches
        assert_eq!(read_u32_le(page, 0x08), 5); // table_type = keys
        current = read_u32_le(page, 0x0c); // next
        page_count += 1;
    }

    assert!(page_count >= 2, "should have walked at least 2 data pages");
    // ec (current) points beyond the physical file — no padding applied,
    // reference exports have next_unused > file_page_count.
    let total = bytes.len() / PAGE_SIZE;
    assert!(
        (current as usize) >= total,
        "empty_candidate should be beyond physical file"
    );
}

#[test]
fn test_large_playlist_entries_roundtrip() {
    let mut data = PdbData::empty();
    data.playlist_tree.push(PdbPlaylistTreeRow {
        id: 24,
        parent_id: 0,
        sort_order: 3,
        is_folder: false,
        name: "Medium Playlist".into(),
    });
    for i in 0..300u32 {
        data.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: i + 1,
            track_id: 10_000 + i,
            playlist_id: 24,
        });
    }

    let bytes = write_pdb(&data).unwrap();
    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes).unwrap();

    let mut entries = parsed
        .playlist_entries
        .iter()
        .filter(|e| e.playlist_id == 24)
        .cloned()
        .collect::<Vec<_>>();
    entries.sort_by_key(|e| e.entry_index);

    assert_eq!(entries.len(), 300);
    assert_eq!(entries.first().map(|e| e.entry_index), Some(1));
    assert_eq!(entries.last().map(|e| e.entry_index), Some(300));
    assert_eq!(entries.first().map(|e| e.track_id), Some(10_000));
    assert_eq!(entries.last().map(|e| e.track_id), Some(10_299));
}

#[test]
fn test_large_multi_playlist_entries_roundtrip() {
    let mut data = PdbData::empty();
    data.playlist_tree.push(PdbPlaylistTreeRow {
        id: 1,
        parent_id: 0,
        sort_order: 1,
        is_folder: false,
        name: "Large Playlist".into(),
    });
    data.playlist_tree.push(PdbPlaylistTreeRow {
        id: 24,
        parent_id: 0,
        sort_order: 3,
        is_folder: false,
        name: "Medium Playlist".into(),
    });
    for i in 0..10_000u32 {
        data.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: i + 1,
            track_id: 20_000 + i,
            playlist_id: 1,
        });
    }
    for i in 0..300u32 {
        data.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: i + 1,
            track_id: 40_000 + i,
            playlist_id: 24,
        });
    }

    let bytes = write_pdb(&data).unwrap();
    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes).unwrap();

    let large_count = parsed
        .playlist_entries
        .iter()
        .filter(|e| e.playlist_id == 1)
        .count();
    let medium_count = parsed
        .playlist_entries
        .iter()
        .filter(|e| e.playlist_id == 24)
        .count();

    assert_eq!(large_count, 10_000);
    assert_eq!(medium_count, 300);
}

// ── remove_duplicate_playlist_entries_inplace tests ─────────────────

/// Build a minimal 2-page PDB byte buffer (page 0 = file header, page 1 =
/// a single `tt=8` data page) containing exactly `rows`
/// `(entry_index, track_id, playlist_id)` tuples, all active, laid out
/// using the wide `num_row_offsets`/`num_rows` packed footer field (the
/// same convention `remove_rows_inplace` and
/// `remove_duplicate_playlist_entries_inplace` read).
pub(crate) fn build_tt8_test_page(rows: &[(u32, u32, u32)]) -> Vec<u8> {
    assert!(
        rows.len() <= 16,
        "test helper only supports one footer group"
    );
    let mut bytes = vec![0u8; PAGE_SIZE * 2];
    bytes[4..8].copy_from_slice(&(PAGE_SIZE as u32).to_le_bytes());

    let page_start = PAGE_SIZE;
    let page_end = PAGE_SIZE * 2;
    bytes[page_start + 4..page_start + 8].copy_from_slice(&1u32.to_le_bytes());
    bytes[page_start + 8..page_start + 12].copy_from_slice(&8u32.to_le_bytes());
    bytes[page_start + 0x1b] = PAGE_FLAGS_DATA;

    let payload_start = page_start + HEAP_START;
    for (i, &(entry_index, track_id, playlist_id)) in rows.iter().enumerate() {
        let row_off = payload_start + i * 12;
        bytes[row_off..row_off + 4].copy_from_slice(&entry_index.to_le_bytes());
        bytes[row_off + 4..row_off + 8].copy_from_slice(&track_id.to_le_bytes());
        bytes[row_off + 8..row_off + 12].copy_from_slice(&playlist_id.to_le_bytes());
    }
    let used_s = (rows.len() * 12) as u16;
    bytes[page_start + 0x1e..page_start + 0x20].copy_from_slice(&used_s.to_le_bytes());

    let n = rows.len() as u32;
    let packed: u32 = (n & 0x1FFF) | ((n & 0x7FF) << 13);
    bytes[page_start + 0x18] = (packed & 0xFF) as u8;
    bytes[page_start + 0x19] = ((packed >> 8) & 0xFF) as u8;
    bytes[page_start + 0x1a] = ((packed >> 16) & 0xFF) as u8;

    let rowpf: u16 = if rows.is_empty() {
        0
    } else {
        ((1u32 << rows.len()) - 1) as u16
    };
    let rowpf_off = page_end - 4;
    let tranrf_off = page_end - 2;
    bytes[rowpf_off..rowpf_off + 2].copy_from_slice(&rowpf.to_le_bytes());
    bytes[tranrf_off..tranrf_off + 2].copy_from_slice(&0u16.to_le_bytes());

    for (i, _) in rows.iter().enumerate() {
        let off_pos = rowpf_off - 2 * (i + 1);
        let heap_off = (i * 12) as u16;
        bytes[off_pos..off_pos + 2].copy_from_slice(&heap_off.to_le_bytes());
    }

    bytes
}

fn active_tt8_rows(bytes: &[u8]) -> Vec<(u32, u32, u32)> {
    let parsed = crate::pdb_reader::parse_pdb_bytes(bytes).unwrap();
    parsed
        .playlist_entries
        .iter()
        .map(|e| (e.entry_index, e.track_id, e.playlist_id))
        .collect()
}

#[test]
fn remove_duplicate_playlist_entries_collapses_stale_copy() {
    // Same shape as the real-world defect this repairs: a track is
    // duplicated under the same playlist_id with a different
    // entry_index, and an unrelated row must survive untouched. The
    // highest entry_index is the most recently written (current)
    // position -- see the doc comment on the removal sort in
    // `remove_duplicate_playlist_entries_inplace`.
    let mut bytes = build_tt8_test_page(&[
        (1, 100, 1), // stale duplicate, must be removed
        (2, 200, 1), // untouched: distinct track
        (5, 100, 1), // kept: highest entry_index for (playlist=1, track=100)
    ]);

    let removed = remove_duplicate_playlist_entries_inplace(&mut bytes);
    assert_eq!(removed, 1);

    let mut rows = active_tt8_rows(&bytes);
    rows.sort();
    assert_eq!(rows, vec![(2, 200, 1), (5, 100, 1)]);
}

#[test]
fn remove_duplicate_playlist_entries_keeps_same_track_in_different_playlists() {
    // The same track_id under two different playlist_ids is not a
    // duplicate and must survive.
    let mut bytes = build_tt8_test_page(&[(1, 100, 1), (1, 100, 2)]);

    let removed = remove_duplicate_playlist_entries_inplace(&mut bytes);
    assert_eq!(removed, 0);

    let mut rows = active_tt8_rows(&bytes);
    rows.sort();
    assert_eq!(rows, vec![(1, 100, 1), (1, 100, 2)]);
}

#[test]
fn remove_duplicate_playlist_entries_is_noop_when_clean() {
    let mut bytes = build_tt8_test_page(&[(1, 100, 1), (2, 200, 1), (3, 300, 1)]);
    let before = active_tt8_rows(&bytes);

    let removed = remove_duplicate_playlist_entries_inplace(&mut bytes);
    assert_eq!(removed, 0);
    assert_eq!(active_tt8_rows(&bytes), before);
}

#[test]
fn remove_duplicate_playlist_entries_removes_all_but_one_of_triple() {
    // Three copies of (playlist=33, track=900) at entry_index 4/11/26;
    // the highest (26, most recently written) survives.
    let mut bytes =
        build_tt8_test_page(&[(4, 900, 33), (11, 900, 33), (26, 900, 33), (12, 950, 33)]);

    let removed = remove_duplicate_playlist_entries_inplace(&mut bytes);
    assert_eq!(removed, 2);

    let mut rows = active_tt8_rows(&bytes);
    rows.sort();
    assert_eq!(rows, vec![(12, 950, 33), (26, 900, 33)]);
}

#[test]
fn remove_duplicate_playlist_entries_sets_trc_minus_one_num_rl_when_removed_row_is_not_last_slot() {
    // Regression for a real export failure: tombstoning a duplicate that
    // isn't the page's last row slot must still leave num_rl at
    // num_row_offsets-1, not at the tombstoned row's own slot index.
    // Slots 0 and 1 get removed here (their entry_index 4/11 lose to
    // slot 2's entry_index 26), while slot 3 stays -- so the page's last
    // slot (3) is never touched by the tombstone itself.
    let mut bytes =
        build_tt8_test_page(&[(4, 900, 33), (11, 900, 33), (26, 900, 33), (12, 950, 33)]);

    let removed = remove_duplicate_playlist_entries_inplace(&mut bytes);
    assert_eq!(removed, 2);

    let page = &bytes[PAGE_SIZE..PAGE_SIZE * 2];
    let u5 = read_u16_le_at(page, 0x20).unwrap();
    let num_rl = read_u16_le_at(page, 0x22).unwrap();
    assert_eq!(
        (u5, num_rl),
        (1, 3),
        "num_rl must stay trc-1 (3), not the removed slot"
    );

    let mismatches = crate::pdb_reader::validate_pdb_page_conventions(&bytes);
    assert!(
        mismatches.is_empty(),
        "unexpected mismatches: {mismatches:?}"
    );
}

#[test]
fn test_pdb_with_tracks_round_trip() {
    let mut data = PdbData::empty();
    data.artists.push(PdbArtistRow {
        id: 1,
        name: "Artist A".into(),
    });
    data.albums.push(PdbAlbumRow {
        id: 1,
        name: "Album A".into(),
        artist_id: 1,
    });
    data.keys.push(PdbKeyRow {
        id: 1,
        name: "Am".into(),
    });
    data.tracks.push(PdbTrackRowData {
        composer_id: 0,
        label_id: 0,
        remixer_id: 0,
        header_flags_u32: None,
        id: 1,
        artist_id: 1,
        album_id: 1,
        artwork_id: 0,
        key_id: 1,
        genre_id: 0,
        title: "Track One".into(),
        anlz_path: "/PIONEER/USBANLZ/P001/0001/ANLZ0000.DAT".into(),
        file_path: "/PIONEER/Contents/track1.mp3".into(),
        bpm: Some(128.0),
        duration_seconds: Some(240),
        track_number: Some(1),
        file_type: Some(1), // MP3
        file_name: Some("track1.mp3".into()),
        content_link: None,
        sample_rate_hz: Some(44100),
        file_size_bytes: Some(5_000_000),
        master_content_id: None,
        master_db_id: None,
        bitrate_kbps: Some(320),
        release_year: Some(2024),
        bit_depth: Some(16),
        isrc: None,
        date_added: Some("2024-01-15".into()),
        release_date: None,
        dj_comment: None,
        publish_track_info_on: None,
        autoload_hotcues_on: None,
        color_id: None,
        rating: None,
    });

    let bytes = write_pdb(&data).unwrap();
    let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes).unwrap();

    assert_eq!(parsed.tracks.len(), 1);
    let t = &parsed.tracks[0];
    assert_eq!(t.id, 1);
    assert_eq!(t.title, "Track One");
    assert_eq!(t.artist_id, 1);
    assert_eq!(t.album_id, 1);
    assert_eq!(t.key_id, 1);
    assert_eq!(t.tempo_x100, 12800);
    assert_eq!(t.track_file_path, "/PIONEER/Contents/track1.mp3");
}

#[test]
fn test_fresh_write_multipage_tt0_all_pages_sealed() {
    // Write enough large tracks to force multiple tt=0 data pages.
    // All tt=0 data pages in a multi-page chain must use SEAL (0x24).
    let mut data = PdbData::empty();
    data.colors = standard_colors();
    data.columns_raw_rows = standard_columns_raw();
    for id in 1u32..=30 {
        data.tracks.push(PdbTrackRowData {
            composer_id: 0,
            label_id: 0,
            remixer_id: 0,
            header_flags_u32: None,
            id,
            artist_id: 0,
            album_id: 0,
            artwork_id: 0,
            key_id: 0,
            genre_id: 0,
            title: format!("Track {:03} - {}", id, "x".repeat(200)),
            anlz_path: format!("/PIONEER/USBANLZ/P{:03}/ANLZ0000.DAT", id),
            file_path: format!("/Contents/track{id:03}.mp3"),
            content_link: None,
            sample_rate_hz: None,
            file_size_bytes: None,
            master_content_id: None,
            master_db_id: None,
            bitrate_kbps: None,
            track_number: Some(id),
            bpm: Some(128.0),
            release_year: None,
            bit_depth: None,
            duration_seconds: Some(240),
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
    }

    let bytes = write_pdb(&data).unwrap();

    // Find all tt=0 data pages and verify page_flags == 0x24 (SEAL)
    let (_, first, last) = table_ptr_fields(&bytes, 0).expect("tt=0 pointer");
    let chain = crate::utils::collect_chain(&bytes, PAGE_SIZE, first, last).expect("tt=0 chain");
    // chain[0] = sentinel, chain[1..] = data pages
    let data_pages: Vec<u32> = chain[1..]
        .iter()
        .copied()
        .filter(|&pg| {
            let off = page_offset(pg, PAGE_SIZE).expect("page offset");
            bytes[off + 0x1b] != 0x64 // not sentinel
        })
        .collect();

    assert!(data_pages.len() > 1, "test needs multiple tt=0 data pages");
    for &pg in &data_pages {
        let off = page_offset(pg, PAGE_SIZE).expect("page offset");
        assert_eq!(
            bytes[off + 0x1b],
            PAGE_FLAGS_DATA,
            "tt=0 data page[{pg}] must be SEAL (0x24) in a multi-page chain"
        );
    }

    // Also verify tt=19 ec does not conflict with any tt=0 data page
    let (ec19, _, _) = table_ptr_fields(&bytes, 19).expect("tt=19 pointer");
    assert!(
        !data_pages.contains(&ec19),
        "tt=19 empty_candidate={ec19} must not point to a tt=0 data page"
    );
}

#[test]
fn test_history_runtime_footer_only_last_row_active_beyond_one_group() {
    // Regression: a tt=19 (history_runtime) page with more than 16 rows
    // used to leave every row's rowpf/tranrf bit set — the "only the
    // last row(s) active" override only handled n<=16, so any group
    // past the first silently kept write_row_index_footer's default
    // "all rows active" pattern. Verified wrong against real
    // rekordbox's own multi-group tt=19 output (a reference export of 88
    // history rows across 6 groups of 16): rowpf has exactly one bit
    // set globally, tranrf exactly two, both in the group holding the
    // final row.
    let mut data = PdbData::empty();
    data.colors = standard_colors();
    data.columns_raw_rows = standard_columns_raw();
    // 20 dummy raw rows forces a second footer group (rows 16..20).
    data.history_raw_rows = (0..20u32).map(|_| vec![0u8; 16]).collect();

    let bytes = write_pdb(&data).unwrap();

    let (_, first, last) = table_ptr_fields(&bytes, 19).expect("tt=19 pointer");
    let chain = crate::utils::collect_chain(&bytes, PAGE_SIZE, first, last).expect("tt=19 chain");
    // chain[0] = sentinel, chain[1] = the single data page (20 small rows fit on one page)
    let data_page = chain[1];
    let off = page_offset(data_page, PAGE_SIZE).expect("page offset");

    // Group 0 (rows 0..16): must be fully inactive now, not "all active".
    let group0_rowpf = u16::from_le_bytes([bytes[off + PAGE_SIZE - 4], bytes[off + PAGE_SIZE - 3]]);
    let group0_tranrf =
        u16::from_le_bytes([bytes[off + PAGE_SIZE - 2], bytes[off + PAGE_SIZE - 1]]);
    assert_eq!(
        group0_rowpf, 0,
        "earlier footer group must not be marked active"
    );
    assert_eq!(
        group0_tranrf, 0,
        "earlier footer group must not be marked active"
    );

    // Group 1 (rows 16..20, group_len=4): only the true last row (index
    // 19, local bit 3) plus the row before it (local bit 2) marked.
    let g1_rowpf_off = off + PAGE_SIZE - 40;
    let g1_tranrf_off = off + PAGE_SIZE - 38;
    let group1_rowpf = u16::from_le_bytes([bytes[g1_rowpf_off], bytes[g1_rowpf_off + 1]]);
    let group1_tranrf = u16::from_le_bytes([bytes[g1_tranrf_off], bytes[g1_tranrf_off + 1]]);
    assert_eq!(
        group1_rowpf, 0b1000,
        "only the final row's bit should be set in rowpf"
    );
    assert_eq!(
        group1_tranrf, 0b1100,
        "tranrf should carry the final row plus the one before it"
    );
}
