//! Ground-up PDB (PDB) writer.
//!
//! Produces a structurally correct USB PDB file from an in-memory
//! representation, with no template file dependency. Every byte is intentional
//! and follows the Deep Symmetry specification.

#[cfg(test)]
use std::path::Path;

use crate::error::BackendResult;
use crate::service::export_helpers::pdb_encoding::apply_page_local_index_shift;
use crate::service::export_helpers::{
    PdbLayoutProfile, PdbTrackRowData, encode_album_row, encode_artist_row, encode_artwork_row,
    encode_key_row, encode_pdb_string, encode_playlist_entry_row, encode_playlist_tree_row,
    encode_track_row_with_profile, sanitize_metadata,
};

// ── Constants ────────────────────────────────────────────────────────────────

const PAGE_SIZE: usize = 4096;
const HEAP_START: usize = 0x28; // 40 bytes: 32 common header + 8 extended header
const NUM_TABLES: u32 = 20;
const TABLE_POINTER_SIZE: usize = 16;
const TABLE_POINTERS_OFFSET: usize = 0x1c;

// Sentinel/index page magic values
const SENTINEL_UNKNOWNA: u16 = 0x1fff;
const SENTINEL_UNKNOWNB: u16 = 0x1fff;
const SENTINEL_MAGIC_03EC: u16 = 0x03ec;
const SENTINEL_MAGIC_03FFFFFF: u32 = 0x03ff_ffff;
const SENTINEL_FIRST_EMPTY: u16 = 0x1fff;
const SENTINEL_EMPTY_ENTRY: u32 = 0x1fff_fff8;
const SENTINEL_TAIL_ZERO_BYTES: usize = 20;

// Page flags
const PAGE_FLAGS_INDEX: u8 = 0x64;
const PAGE_FLAGS_DATA: u8 = 0x24;
const PAGE_FLAGS_DATA_TRACK: u8 = 0x34;

// ── Data Model ───────────────────────────────────────────────────────────────

/// Complete in-memory representation of a PDB file's content.
#[derive(Clone)]
pub struct PdbData {
    pub tracks: Vec<PdbTrackRowData>,
    pub artists: Vec<PdbArtistRow>,
    pub albums: Vec<PdbAlbumRow>,
    pub genres: Vec<PdbDictRow>,
    pub labels: Vec<PdbDictRow>,
    pub keys: Vec<PdbKeyRow>,
    pub colors: Vec<PdbColorRow>,
    pub artwork: Vec<PdbArtworkRow>,
    pub playlist_tree: Vec<PdbPlaylistTreeRow>,
    pub playlist_entries: Vec<PdbPlaylistEntryRow>,
    // Tables 9-12, 14-15: always empty
    // Table 16 (columns): raw pass-through
    pub columns_raw_rows: Vec<Vec<u8>>,
    // History tables
    pub history_playlists: Vec<PdbDictRow>,
    pub history_entries: Vec<PdbHistoryEntryRow>,
    pub history_raw_rows: Vec<Vec<u8>>,
    /// Layout profile for track row encoding
    pub profile: PdbLayoutProfile,
}

#[derive(Clone, Debug)]
pub struct PdbDictRow {
    pub id: u32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct PdbArtistRow {
    pub id: u32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct PdbAlbumRow {
    pub id: u32,
    pub name: String,
    pub artist_id: u32,
}

#[derive(Clone, Debug)]
pub struct PdbKeyRow {
    pub id: u32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct PdbColorRow {
    pub id: u16,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct PdbArtworkRow {
    pub id: u32,
    pub path: String,
}

#[derive(Clone, Debug)]
pub struct PdbPlaylistTreeRow {
    pub id: u32,
    pub parent_id: u32,
    pub sort_order: u32,
    pub is_folder: bool,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct PdbPlaylistEntryRow {
    pub entry_index: u32,
    pub track_id: u32,
    pub playlist_id: u32,
}

#[derive(Clone, Debug)]
pub struct PdbHistoryEntryRow {
    pub track_id: u32,
    pub playlist_id: u32,
    pub entry_index: u32,
}

impl PdbData {
    /// Create an empty PDB with no data in any table.
    pub fn empty() -> Self {
        Self {
            tracks: Vec::new(),
            artists: Vec::new(),
            albums: Vec::new(),
            genres: Vec::new(),
            labels: Vec::new(),
            keys: Vec::new(),
            colors: Vec::new(),
            artwork: Vec::new(),
            playlist_tree: Vec::new(),
            playlist_entries: Vec::new(),
            columns_raw_rows: Vec::new(),
            history_playlists: Vec::new(),
            history_entries: Vec::new(),
            history_raw_rows: Vec::new(),
            profile: PdbLayoutProfile::DEFAULT,
        }
    }
}

// ── Standard Table Data ──────────────────────────────────────────────────────
//
// Colors and columns tables are required for player compatibility.
// Values extracted from working reference exports.

/// Standard player color definitions (8 rows: Pink, Red, Orange, Yellow, Green, Aqua, Blue, Purple).
pub fn standard_colors() -> Vec<PdbColorRow> {
    vec![
        PdbColorRow {
            id: 1,
            name: "Pink".to_string(),
        },
        PdbColorRow {
            id: 2,
            name: "Red".to_string(),
        },
        PdbColorRow {
            id: 3,
            name: "Orange".to_string(),
        },
        PdbColorRow {
            id: 4,
            name: "Yellow".to_string(),
        },
        PdbColorRow {
            id: 5,
            name: "Green".to_string(),
        },
        PdbColorRow {
            id: 6,
            name: "Aqua".to_string(),
        },
        PdbColorRow {
            id: 7,
            name: "Blue".to_string(),
        },
        PdbColorRow {
            id: 8,
            name: "Purple".to_string(),
        },
    ]
}

/// Full player browse catalog written to PDB t16 on fresh USBs and clean exports.
///
/// Matches the standard 27-row PDB t16 written on initialized USB exports.
/// Older players read all of these rows as available browse categories; eDB
/// `.category.isVisible` / `sequenceNo` then controls what newer players and
/// desktop library software shows. Keeping all 27 rows in PDB preserves the complete catalog
/// regardless of which subset the user has marked visible.
///
/// Order matches the eDB `menuItem_id` order seeded by `initialize_usb` so
/// that `pdb_missing_kinds` stays empty on a fresh USB.
///
/// Rows are encoded via `encode_pdb_t16_row` so they round-trip byte-for-byte
/// against reference t16 rows on the same kind/name (see
/// `encode_pdb_t16_row_roundtrips_*` tests).
pub fn standard_columns_raw() -> Vec<Vec<u8>> {
    use crate::service::export_helpers::encode_pdb_t16_row;
    DEFAULT_PLAYER_MENU_FULL
        .iter()
        .enumerate()
        .map(|(idx, (kind, name))| {
            let id = u16::try_from(idx + 1).expect("default menu fits in u16");
            encode_pdb_t16_row(id, *kind, name)
        })
        .collect()
}

/// All 27 player browse categories, in eDB menuItem_id order. Written to PDB
/// t16 as the full catalog; eDB.category controls visibility.
pub(crate) const DEFAULT_PLAYER_MENU_FULL: &[(u16, &str)] = &[
    (128, "GENRE"),
    (129, "ARTIST"),
    (130, "ALBUM"),
    (131, "TRACK"),
    (133, "BPM"),
    (134, "RATING"),
    (135, "YEAR"),
    (136, "REMIXER"),
    (137, "LABEL"),
    (138, "ORIGINAL ARTIST"),
    (139, "KEY"),
    (141, "CUE"),
    (142, "COLOR"),
    (146, "TIME"),
    (147, "BITRATE"),
    (148, "FILE NAME"),
    (132, "PLAYLIST"),
    (152, "HOT CUE BANK"),
    (149, "HISTORY"),
    (145, "SEARCH"),
    (150, "COMMENTS"),
    (140, "DATE ADDED"),
    (151, "DJ PLAY COUNT"),
    (144, "FOLDER"),
    (161, "DEFAULT"),
    (162, "ALPHABET"),
    (170, "MATCHING"),
];

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

/// Calculate row index footer size for a given number of rows.
fn footer_size(row_count: usize) -> usize {
    if row_count == 0 {
        return 0;
    }
    let groups = row_count.div_ceil(16);
    groups * 4 + row_count * 2
}

/// Round up to 4-byte alignment.
fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// Build a sentinel/index page.
/// `has_data`: if false, next_page_dup at 0x2c is set to magic 0x03FFFFFF
/// (matches reference-export behavior for empty tables).
fn build_sentinel_page(
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
fn pack_rows_into_pages(
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

        if needed > PAGE_SIZE && !current_rows.is_empty() {
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
fn build_data_page(table_type: u32, pf: u8, seq: u32, rows: &[&[u8]], sealed: bool) -> Vec<u8> {
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
        let packed = (n & 0x1FFF) | ((n & 0x7FF) << 13);
        page[0x18] = (packed & 0xFF) as u8;
        page[0x19] = ((packed >> 8) & 0xFF) as u8;
        page[0x1a] = ((packed >> 16) & 0xFF) as u8;
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

    // Row index footer: grows backward from page end
    // Layout: [ofs_N-1] ... [ofs_0] [rowpf] [tranrf]
    //         tranrf is at the very LAST 2 bytes of the page
    write_row_index_footer(&mut page, &row_offsets);

    if u5 == 1 && table_type != 19 && !row_offsets.is_empty() {
        // Pages with u5=1 convention: tranrf must have only the last row's bit set.
        // Formula: tranrf[group] = 1<<(num_rl%16) for group==num_rl/16, else 0.
        // Applies to both sealed and active pages. Tables with u5=trc keep tranrf=rowpf.
        // Entry format: bit N = row N within a 16-row group; group 0 at page end.
        let n = row_offsets.len();
        let last_bit = (n - 1) % 16;
        let last_group_start = (n - 1) / 16 * 16;
        let mut pos = PAGE_SIZE;
        for group_start in (0..n).step_by(16) {
            let group_len = (n - group_start).min(16);
            let tranrf: u16 = if group_start == last_group_start {
                1u16 << last_bit
            } else {
                0u16
            };
            page[pos - 2..pos].copy_from_slice(&tranrf.to_le_bytes());
            pos -= 2 + 2 + group_len * 2;
        }
    }

    if table_type == 19 {
        // Runtime profile: rowpf marks the last row bit and tranrf carries
        // rowpf plus the preceding transaction-row bit.
        let n = rows.len();
        if (1..=16).contains(&n) {
            let rowpf = 1u16 << (n - 1);
            let tranrf = if n > 1 {
                rowpf | (1u16 << (n - 2))
            } else {
                rowpf
            };
            page[PAGE_SIZE - 4..PAGE_SIZE - 2].copy_from_slice(&rowpf.to_le_bytes());
            page[PAGE_SIZE - 2..PAGE_SIZE].copy_from_slice(&tranrf.to_le_bytes());
        }
    }

    page
}

/// Write the row index footer into a page buffer.
/// Layout per group of 16: [ofs_N..ofs_0] [rowpf] [tranrf]
/// Group 0's tranrf is at the very end of the page.
fn write_row_index_footer(page: &mut [u8], row_offsets: &[u16]) {
    let n = row_offsets.len();
    if n == 0 {
        return;
    }

    let mut cursor = PAGE_SIZE;
    for group_start in (0..n).step_by(16) {
        let group_len = (n - group_start).min(16);
        let bits = ((1u32 << group_len) - 1) as u16;

        // tranrf at end of group (highest address)
        cursor -= 2;
        page[cursor..cursor + 2].copy_from_slice(&bits.to_le_bytes());
        // rowpf before tranrf
        cursor -= 2;
        page[cursor..cursor + 2].copy_from_slice(&bits.to_le_bytes());
        // row offsets: ofs[0] closest to rowpf (highest address),
        // ofs[N-1] at lowest address — matches parser read order
        for j in 0..group_len {
            cursor -= 2;
            page[cursor..cursor + 2].copy_from_slice(&row_offsets[group_start + j].to_le_bytes());
        }
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
        .chunks_exact(PAGE_SIZE)
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

/// Write a PDB file to disk.
pub(crate) fn write_pdb_to_bytes(data: &PdbData) -> BackendResult<Vec<u8>> {
    write_pdb(data)
}

#[cfg(test)]
pub(crate) fn write_pdb_to_file(path: &Path, data: &PdbData) -> BackendResult<()> {
    let bytes = write_pdb(data)?;
    std::fs::write(path, &bytes)?;
    Ok(())
}

// ── In-place row deletion ───────────────────────────────────────────────────

/// Remove rows from a PDB file in-place, preserving page layout.
///
/// For each page of the given `table_type`, rows whose ID (extracted by
/// `extract_row_id`) is in `ids_to_remove` have their `rowpf` bit cleared.
/// Page headers are updated (num_rows decremented, D flag set) but the heap
/// data and page count are left unchanged.
///
/// Returns the number of rows actually removed.
pub(crate) fn remove_rows_inplace(
    bytes: &mut [u8],
    table_type: u32,
    ids_to_remove: &std::collections::HashSet<u32>,
    extract_row_id: fn(&[u8]) -> Option<u32>,
) -> usize {
    if ids_to_remove.is_empty() || bytes.len() < PAGE_SIZE * 2 {
        return 0;
    }
    let Some(len_page) = read_u32_le_at(bytes, 4).map(|v| v as usize) else {
        return 0;
    };
    if len_page != PAGE_SIZE || bytes.len() < len_page * 2 {
        return 0;
    }

    let total_pages = bytes.len() / len_page;
    let mut removed = 0usize;

    for page_idx in 1..total_pages {
        let page_start = page_idx * len_page;
        let page_end = page_start + len_page;
        if page_end > bytes.len() {
            break;
        }

        // Check table type
        let Some(pt) = read_u32_le_at(bytes, page_start + 0x08) else {
            continue;
        };
        if pt != table_type {
            continue;
        }

        // Skip index/sentinel pages (bit 6 set in page_flags)
        let page_flags = bytes[page_start + 0x1b];
        if page_flags & 0x40 != 0 {
            continue;
        }

        // Read packed row counts
        let packed = u32::from(bytes[page_start + 0x18])
            | (u32::from(bytes[page_start + 0x19]) << 8)
            | (u32::from(bytes[page_start + 0x1a]) << 16);
        let num_row_offsets = (packed & 0x1FFF) as usize;
        let num_rows = ((packed >> 13) & 0x7FF) as usize;

        if num_row_offsets == 0 {
            continue;
        }

        let Some(used_size) = read_u16_le_at(bytes, page_start + 0x1e).map(|v| v as usize) else {
            continue;
        };
        let payload_start = page_start + HEAP_START;
        let payload_end = payload_start + used_size;
        if payload_end > page_end {
            continue;
        }

        // Walk footer groups backwards from page end.
        // Collect (tranrf_off, rowpf_off, changed_mask) so we can do a second
        // pass to write the formula-derived tranrf after knowing the highest removed slot.
        let mut cursor = page_end;
        let mut page_removed = 0usize;
        let mut highest_removed_slot: Option<usize> = None;
        // (tranrf_off, group_start_row) for every group on this page
        let mut group_offsets: Vec<(usize, usize)> = Vec::new();

        for group_start_row in (0..num_row_offsets).step_by(16) {
            let group_len = (num_row_offsets - group_start_row).min(16);
            if cursor < page_start + 4 + group_len * 2 {
                break;
            }

            // Read tranrf and rowpf
            cursor -= 2;
            let tranrf_off = cursor;
            cursor -= 2;
            let rowpf_off = cursor;
            let Some(rowpf) = read_u16_le_at(bytes, rowpf_off) else {
                continue;
            };

            group_offsets.push((tranrf_off, group_start_row));

            // Read row offsets for this group
            let mut offsets = Vec::with_capacity(group_len);
            for _ in 0..group_len {
                cursor -= 2;
                let Some(off) = read_u16_le_at(bytes, cursor).map(|v| v as usize) else {
                    offsets.clear();
                    break;
                };
                offsets.push(off);
            }
            if offsets.len() != group_len {
                continue;
            }

            // Check each row in this group
            let mut new_rowpf = rowpf;
            for (j, &heap_off) in offsets.iter().enumerate() {
                let bit = 1u16 << (j as u16);
                if rowpf & bit == 0 {
                    continue; // already deleted
                }
                let row_abs = payload_start + heap_off;
                if row_abs >= payload_end {
                    continue;
                }
                let row_data = &bytes[row_abs..payload_end.min(page_end)];
                if let Some(id) = extract_row_id(row_data)
                    && ids_to_remove.contains(&id)
                {
                    new_rowpf &= !bit;
                    page_removed += 1;
                    // Track the absolute slot index of the highest removed row.
                    let abs_slot = group_start_row + j;
                    highest_removed_slot =
                        Some(highest_removed_slot.map_or(abs_slot, |prev| prev.max(abs_slot)));
                }
            }

            if new_rowpf != rowpf {
                bytes[rowpf_off..rowpf_off + 2].copy_from_slice(&new_rowpf.to_le_bytes());
                // tranrf is rewritten in the second pass below
            }
        }

        if page_removed > 0 {
            removed += page_removed;

            // Update num_rows (bits 13-23), keep num_row_offsets (bits 0-12)
            let new_num_rows = num_rows.saturating_sub(page_removed);
            let new_packed =
                (num_row_offsets as u32 & 0x1FFF) | ((new_num_rows as u32 & 0x7FF) << 13);
            bytes[page_start + 0x18] = (new_packed & 0xFF) as u8;
            bytes[page_start + 0x19] = ((new_packed >> 8) & 0xFF) as u8;
            bytes[page_start + 0x1a] = ((new_packed >> 16) & 0xFF) as u8;

            // Tombstone state is tracked only in the row footer bitmasks
            // (rowpf/tranrf). Page_flags must not be modified — setting 0x10
            // on a sealed (0x24) overflow page would produce 0x34 which DJ
            // players reject as corrupted on non-tail pages.

            // Update u5=1, num_rl=last removed slot. One-row transaction
            // per tombstone operation, num_rl = highest-indexed tombstoned slot.
            if let Some(slot) = highest_removed_slot {
                bytes[page_start + 0x20..page_start + 0x22].copy_from_slice(&1u16.to_le_bytes());
                bytes[page_start + 0x22..page_start + 0x24]
                    .copy_from_slice(&(slot as u16).to_le_bytes());

                // Rewrite tranrf for ALL groups: only the group containing
                // `slot` gets 1<<(slot%16); all other groups get 0.
                // This matches the pattern confirmed from working multi-track tombstone exports.
                let last_group = slot / 16;
                let last_bit = (slot % 16) as u32;
                for &(tranrf_off, group_start) in &group_offsets {
                    let tranrf_val: u16 = if group_start / 16 == last_group {
                        1u16 << last_bit
                    } else {
                        0
                    };
                    bytes[tranrf_off..tranrf_off + 2].copy_from_slice(&tranrf_val.to_le_bytes());
                }
            }
        }
    }

    removed
}

/// Normalise tt=8 (playlist_entry) page footer fields to `(u5=1, num_rl=trc-1)`.
///
/// Exports produced by other software occasionally carry interior pages whose
/// `num_rl` does not follow the `trc-1` convention required by strict player
/// validation. This pass corrects those pages before the post-write convention
/// check so the writer's output is always conformant.
pub(crate) fn fix_tt8_num_rl_conventions_inplace(bytes: &mut [u8]) {
    use crate::utils::{read_u8_at, read_u16_le_at, read_u32_le_at};
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
/// as-is (preserving the SENTINEL_EMPTY entry written by the original exporter).
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
    const SENTINEL_EMPTY: u32 = 0x1fff_fff8;
    const TAIL_RESERVE: usize = 20;
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

        let fill_end = sent_off + page_size.saturating_sub(TAIL_RESERVE);
        let mut cursor = sent_off + 0x3c;
        for &pi in &active {
            if cursor + 4 > fill_end {
                break;
            }
            bytes[cursor..cursor + 4].copy_from_slice(&(pi * 8).to_le_bytes());
            cursor += 4;
        }
        while cursor + 4 <= fill_end {
            bytes[cursor..cursor + 4].copy_from_slice(&SENTINEL_EMPTY.to_le_bytes());
            cursor += 4;
        }
        bytes[sent_off + 0x38..sent_off + 0x3a].copy_from_slice(&ne.to_le_bytes());
        bytes[sent_off + 0x3a..sent_off + 0x3c].copy_from_slice(&0x1fffu16.to_le_bytes());
        bytes[sent_off + 0x26..sent_off + 0x28].copy_from_slice(&ne.to_le_bytes());
    }
}

fn read_u16_le_at(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod writer_tests {
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
        let (pages, _) = pack_rows_into_pages(2, &[row.clone()], &mut seq);
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
        };
        let row = encode_track_row_with_profile(&track, PdbLayoutProfile::Current).unwrap();
        let mut seq = 100u32;
        let (pages, _) = pack_rows_into_pages(0, &[row], &mut seq);
        assert_eq!(pages[0][0x1b], PAGE_FLAGS_DATA_TRACK);
    }

    // ── Row encoding tests ──────────────────────────────────────────────

    #[test]
    fn test_artist_row_near_variant() {
        let row = encode_artist_row(1, "Abc");
        // Near variant: subtype=0x0060, id at 4, const 3 at 8, ofs at 9, 2 padding bytes at 10-11, name at 12
        // Reference layout confirmed; MIPS-based player hardware freezes with ofs=10 on UTF-16 names.
        assert_eq!(read_u16_le(&row, 0), 0x0060);
        assert_eq!(read_u32_le(&row, 4), 1);
        assert_eq!(row[8], 3);
        assert_eq!(row[9], 12); // ofs_name_near = 12 (0x0c)
        assert_eq!(row[10], 0); // padding
        assert_eq!(row[11], 0); // padding
        // Short string "Abc": lk=3*2+3=9, then 'A','b','c'
        assert_eq!(row[12], 9);
        assert_eq!(&row[13..16], b"Abc");
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
        for chunk in bytes.chunks_exact(PAGE_SIZE).skip(1) {
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
            0x80, 0x02, 0x80, 0x01, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x15, 0x17, 0x32,
            0x30, 0x32, 0x36, 0x2d, 0x30, 0x34, 0x2d, 0x31, 0x35, 0x01, 0x01, 0x0b, 0x31, 0x30,
            0x30, 0x30, 0x00, 0x00,
        ]);
        data.tracks = (1..=250)
            .map(|id| PdbTrackRowData {
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
            name: " DnB 2024".into(),
        });
        for i in 0..317u32 {
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

        assert_eq!(entries.len(), 317);
        assert_eq!(entries.first().map(|e| e.entry_index), Some(1));
        assert_eq!(entries.last().map(|e| e.entry_index), Some(317));
        assert_eq!(entries.first().map(|e| e.track_id), Some(10_000));
        assert_eq!(entries.last().map(|e| e.track_id), Some(10_316));
    }

    #[test]
    fn test_large_multi_playlist_entries_roundtrip() {
        let mut data = PdbData::empty();
        data.playlist_tree.push(PdbPlaylistTreeRow {
            id: 1,
            parent_id: 0,
            sort_order: 1,
            is_folder: false,
            name: "-=[LABEL_SORT]=-".into(),
        });
        data.playlist_tree.push(PdbPlaylistTreeRow {
            id: 24,
            parent_id: 0,
            sort_order: 3,
            is_folder: false,
            name: " DnB 2024".into(),
        });
        for i in 0..9_995u32 {
            data.playlist_entries.push(PdbPlaylistEntryRow {
                entry_index: i + 1,
                track_id: 20_000 + i,
                playlist_id: 1,
            });
        }
        for i in 0..317u32 {
            data.playlist_entries.push(PdbPlaylistEntryRow {
                entry_index: i + 1,
                track_id: 40_000 + i,
                playlist_id: 24,
            });
        }

        let bytes = write_pdb(&data).unwrap();
        let parsed = crate::pdb_reader::parse_pdb_bytes(&bytes).unwrap();

        let label_count = parsed
            .playlist_entries
            .iter()
            .filter(|e| e.playlist_id == 1)
            .count();
        let dnb_count = parsed
            .playlist_entries
            .iter()
            .filter(|e| e.playlist_id == 24)
            .count();

        assert_eq!(label_count, 9_995);
        assert_eq!(dnb_count, 317);
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
            });
        }

        let bytes = write_pdb(&data).unwrap();

        // Find all tt=0 data pages and verify page_flags == 0x24 (SEAL)
        let (_, first, last) = table_ptr_fields(&bytes, 0).expect("tt=0 pointer");
        let chain =
            crate::utils::collect_chain(&bytes, PAGE_SIZE, first, last).expect("tt=0 chain");
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
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Topology-locked additive writer (merged from pdb_additive.rs)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use crate::error::BackendError;
use crate::utils::{
    collect_chain as collect_chain_pages, page_offset, read_u8_at, set_table_ptr_fields,
    table_ptr_fields, write_u32_le_at,
};
use std::collections::{HashMap, HashSet};

const PAGE_HEADER_SIZE: usize = 40;
/// Maximum encoded row length that can fit on any single page after 4-byte heap
/// alignment, the 40-byte page header, and the minimum 6-byte single-row footer
/// (`[ofs0][rowpf][tranrf]`). Rows larger than this cannot be placed anywhere
/// (existing or fresh) and must be rejected up-front.
pub(crate) const MAX_ROW_LEN: usize = (PAGE_SIZE - PAGE_HEADER_SIZE - 6) & !3;

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

fn page_has_transaction_tombstones(page: &[u8], page_size: usize) -> bool {
    if page.len() < page_size || page_size < PAGE_HEADER_SIZE {
        return false;
    }
    let used_s = read_u16_le_at(page, 0x1e).unwrap_or(0);
    if used_s == 0 {
        return false;
    }
    let nrs = page.get(0x18).copied().unwrap_or(0) as usize;
    let num_rl = read_u16_le_at(page, 0x22).unwrap_or(0) as usize;
    let row_slots = if num_rl == 8191 { nrs } else { nrs.max(num_rl) };
    if row_slots == 0 {
        return false;
    }

    let mut cursor = page_size;
    for group_start in (0..row_slots).step_by(16) {
        if cursor < 4 {
            return false;
        }
        cursor -= 4;
        let rowpf = read_u16_le_at(page, cursor).unwrap_or(0);
        let tranrf = read_u16_le_at(page, cursor + 2).unwrap_or(0);
        let group_len = (row_slots - group_start).min(16);
        let mask = if group_len == 16 {
            u16::MAX
        } else {
            ((1u32 << group_len) - 1) as u16
        };
        if (tranrf & !rowpf & mask) != 0 {
            return true;
        }
        let footer_offsets = group_len.saturating_mul(2);
        if cursor < footer_offsets {
            return false;
        }
        cursor -= footer_offsets;
    }
    false
}

struct PageFooterState {
    offsets: Vec<usize>,
    rowpf_by_group: Vec<u16>,
    tranrf_by_group: Vec<u16>,
}

fn read_page_footer_state(page: &[u8], page_size: usize) -> Option<PageFooterState> {
    if page.len() < page_size || page_size < PAGE_HEADER_SIZE {
        return None;
    }
    let used_s = read_u16_le_at(page, 0x1e).unwrap_or(0) as usize;
    if used_s == 0 {
        return None;
    }
    let nrs = page.get(0x18).copied().unwrap_or(0) as usize;
    let num_rl = read_u16_le_at(page, 0x22).unwrap_or(0) as usize;
    let row_slots = if num_rl == 8191 { nrs } else { nrs.max(num_rl) };
    if row_slots == 0 {
        return None;
    }

    let mut cursor = page_size;
    let mut offsets = Vec::<usize>::with_capacity(row_slots);
    let mut rowpf_by_group = Vec::<u16>::with_capacity(row_slots.div_ceil(16));
    let mut tranrf_by_group = Vec::<u16>::with_capacity(row_slots.div_ceil(16));
    for group_start in (0..row_slots).step_by(16) {
        if cursor < 4 {
            return None;
        }
        cursor -= 4;
        rowpf_by_group.push(read_u16_le_at(page, cursor).unwrap_or(0));
        tranrf_by_group.push(read_u16_le_at(page, cursor + 2).unwrap_or(0));

        let group_len = (row_slots - group_start).min(16);
        for _ in 0..group_len {
            if cursor < 2 {
                return None;
            }
            cursor -= 2;
            offsets.push(read_u16_le_at(page, cursor).unwrap_or(0) as usize);
        }
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
        let new_footer = footer_size_for_rows(new_row_count);
        if PAGE_HEADER_SIZE + aligned_end + new_footer > page_size {
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
            page[PAGE_HEADER_SIZE + used_s..PAGE_HEADER_SIZE + start].fill(0);
        }
        let slot = footer.offsets.len();
        let mut row_bytes = row.clone();
        apply_page_local_index_shift(table_type, &mut row_bytes, slot);
        let end = start + row_bytes.len();
        page[PAGE_HEADER_SIZE + start..PAGE_HEADER_SIZE + end].copy_from_slice(&row_bytes);
        let aligned_end = align4(end);
        if aligned_end > end {
            page[PAGE_HEADER_SIZE + end..PAGE_HEADER_SIZE + aligned_end].fill(0);
        }

        footer.offsets.push(start);
        let group = slot / 16;
        let bit = slot % 16;
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
    let active_count: u32 = footer
        .rowpf_by_group
        .iter()
        .enumerate()
        .map(|(group, bits)| {
            let remaining = row_count.saturating_sub(group * 16);
            let group_len = remaining.min(16);
            let mask = if group_len == 16 {
                u16::MAX
            } else {
                ((1u32 << group_len) - 1) as u16
            };
            (bits & mask).count_ones()
        })
        .sum();
    let packed = ((row_count as u32) & 0x1fff) | ((active_count & 0x7ff) << 13);
    page[0x18] = (packed & 0xff) as u8;
    page[0x19] = ((packed >> 8) & 0xff) as u8;
    page[0x1a] = ((packed >> 16) & 0xff) as u8;
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

    let footer_size = footer_size_for_rows(row_count);
    let footer_start = page_size - footer_size;
    if PAGE_HEADER_SIZE + used_s <= footer_start {
        page[PAGE_HEADER_SIZE + used_s..footer_start].fill(0);
    }
    let free_s = page_size.saturating_sub(PAGE_HEADER_SIZE + used_s + footer_size);
    page[0x1c..0x1e].copy_from_slice(&(free_s as u16).to_le_bytes());

    // Existing tranrf groups are validated transaction history on player hardware
    // family players. Preserve them and OR in only the appended row bits.
    let mut footer_cursor = page_size;
    for group_start in (0..row_count).step_by(16) {
        let group = group_start / 16;
        let group_len = (row_count - group_start).min(16);
        footer_cursor -= 2;
        page[footer_cursor..footer_cursor + 2]
            .copy_from_slice(&footer.tranrf_by_group[group].to_le_bytes());
        footer_cursor -= 2;
        page[footer_cursor..footer_cursor + 2]
            .copy_from_slice(&footer.rowpf_by_group[group].to_le_bytes());
        for j in 0..group_len {
            footer_cursor -= 2;
            let offset = footer.offsets[group_start + j] as u16;
            page[footer_cursor..footer_cursor + 2].copy_from_slice(&offset.to_le_bytes());
        }
    }

    Ok(absorbed)
}

fn append_rows_to_t07_tombstone_pages_in_place(
    bytes: &mut Vec<u8>,
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

/// Compute remaining heap capacity on a populated data page if we were to
/// add `extra_rows` more rows to it.
///
/// Returns `Some(bytes_free)` when the page is recognizable as a populated
/// data page (non-zero stored index, non-sentinel `pf`, non-empty heap),
/// `None` otherwise.
///
/// Currently only used as a self-documenting reference for the inline
/// capacity check in `append_rows_to_chain_in_place`. Kept here so the
/// capacity model lives next to the constants that define it.
#[allow(dead_code)]
fn remaining_heap_capacity(
    page: &[u8],
    present_used_bytes: usize,
    present_row_count: usize,
    extra_rows: usize,
) -> Option<usize> {
    let stored_idx = read_u32_le_at(page, 0x04)?;
    if stored_idx == 0 {
        return None;
    }
    let pf = *page.get(0x1b)?;
    if pf == 0x64 {
        return None;
    }
    let new_footer = footer_size_for_rows(present_row_count + extra_rows);
    let occupied = PAGE_HEADER_SIZE + present_used_bytes + new_footer;
    if PAGE_SIZE >= occupied {
        Some(PAGE_SIZE - occupied)
    } else {
        Some(0)
    }
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
            let new_footer = footer_size_for_rows(new_count);
            if PAGE_HEADER_SIZE + new_used + new_footer > page_size {
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
        // Apply the per-table footer convention.
        let trc = fitted.len() as u16;
        let (u5, num_rl) = data_page_footer_fields(table_type, trc);
        bytes[new_off + 0x20..new_off + 0x22].copy_from_slice(&u5.to_le_bytes());
        bytes[new_off + 0x22..new_off + 0x24].copy_from_slice(&num_rl.to_le_bytes());
        // Fix tranrf: rewrite_variable_page_rows_in_place wrote all-bits; for u5=1
        // tables the writer uses exactly one bit: tranrf[group] = 1<<(num_rl%16) in the
        // group containing num_rl, 0 elsewhere.
        if u5 == 1 && table_type != 19 {
            let last_group = num_rl as usize / 16;
            let last_bit = num_rl as usize % 16;
            let nrows = fitted.len();
            let mut footer_pos = page_size;
            for group_start in (0..nrows).step_by(16) {
                let group_len = (nrows - group_start).min(16);
                let tranrf_val: u16 = if group_start / 16 == last_group {
                    1u16 << last_bit
                } else {
                    0
                };
                bytes[new_off + footer_pos - 2..new_off + footer_pos]
                    .copy_from_slice(&tranrf_val.to_le_bytes());
                footer_pos = footer_pos.saturating_sub(2 + 2 + group_len * 2);
            }
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
        const SENTINEL_EMPTY_ENTRY: u32 = 0x1fff_fff8;
        const SENTINEL_TAIL_ZERO_BYTES: usize = 20;
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
    /// t08 entries to remove in-place before appending new ones.
    /// Non-empty when mirror-mode export shrinks an existing playlist.
    pub removed_t08_entries: Vec<T08EntryKey>,
    /// Number of t19 runtime-history rows to synthesize in place for a
    /// template first export. Includes the seed row, so this is usually
    /// `track_count + 1`.
    pub synthesize_t19_runtime_rows: Option<usize>,
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

    let synthesize_t19_runtime_rows = if parsed.tracks.is_empty() && !new_data.tracks.is_empty() {
        Some(new_data.tracks.len().saturating_add(1))
    } else {
        None
    };

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

    // ── t08 deletion check ──────────────────────────────────────────
    //
    // `desired_t08_entries` describes the FINAL desired set for the
    // target playlist. Existing entries for that playlist that are not
    // in the desired set will be removed in-place by the apply step.
    // Record them here rather than rejecting the diff.
    let removed_t08_entries: Vec<T08EntryKey> = if !desired_t08_entries.is_empty() {
        let target_playlist_id = desired_t08_entries[0].playlist_id;
        let desired_set: std::collections::HashSet<T08EntryKey> =
            desired_t08_entries.iter().copied().collect();
        let existing_t08 = collect_t08_entry_keys(existing_bytes, 4096);
        existing_t08
            .into_iter()
            .filter(|k| k.playlist_id == target_playlist_id && !desired_set.contains(k))
            .collect()
    } else {
        Vec::new()
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
        synthesize_t19_runtime_rows,
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
    bytes: &mut Vec<u8>,
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
        let trc = kept.len() as u16;
        let (u5, num_rl) = data_page_footer_fields(8, trc);
        bytes[off + 0x20..off + 0x22].copy_from_slice(&u5.to_le_bytes());
        bytes[off + 0x22..off + 0x24].copy_from_slice(&num_rl.to_le_bytes());
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

fn synthesize_t19_runtime_rows_in_place(
    bytes: &mut Vec<u8>,
    row_count: usize,
    page_size: usize,
) -> BackendResult<()> {
    if row_count == 0 {
        return Ok(());
    }
    if page_size != PAGE_SIZE {
        return Err(BackendError::Validation(format!(
            "t19 runtime synthesis requires page_size {PAGE_SIZE}, got {page_size}"
        )));
    }

    let (_ec, first, last) = table_ptr_fields(bytes, 19).ok_or_else(|| {
        BackendError::Validation("t19 runtime synthesis: table 19 pointer missing".into())
    })?;
    let chain = collect_chain_pages(bytes, page_size, first, last).ok_or_else(|| {
        BackendError::Validation("t19 runtime synthesis: t19 chain unreachable".into())
    })?;
    let page_idx = chain.get(1).copied().unwrap_or(last);

    let rows: Vec<Vec<u8>> = (0..row_count)
        .map(|idx| {
            let mut row = vec![0u8; 40];
            let val = 0x0000_0280u32.saturating_add((idx as u32).saturating_mul(0x0020_0000));
            row[0..4].copy_from_slice(&val.to_le_bytes());
            row[4..8].copy_from_slice(&(idx as u32).to_le_bytes());
            row[26..30].copy_from_slice(b"1000");
            row
        })
        .collect();

    if !rewrite_variable_page_rows_in_place(bytes, page_idx, &rows, page_size) {
        return Err(BackendError::Validation(format!(
            "t19 runtime synthesis: {row_count} rows do not fit on page {page_idx}"
        )));
    }

    let off = page_offset(page_idx, page_size).ok_or_else(|| {
        BackendError::Validation(format!(
            "t19 runtime synthesis: page {page_idx} out of bounds"
        ))
    })?;
    let seq = max_seqpage_in_file(bytes, page_size)
        .saturating_add(1)
        .max(2);
    bytes[off + 0x10..off + 0x14].copy_from_slice(&seq.to_le_bytes());
    bytes[off + 0x18] = (row_count & 0xff) as u8;
    bytes[off + 0x19] = 0x20;
    bytes[off + 0x1a] = 0;
    bytes[off + 0x1b] = page_flags_for_table(19);
    let (u5, num_rl) = data_page_footer_fields(19, row_count as u16);
    bytes[off + 0x20..off + 0x22].copy_from_slice(&u5.to_le_bytes());
    bytes[off + 0x22..off + 0x24].copy_from_slice(&num_rl.to_le_bytes());
    if (1..=16).contains(&row_count) {
        let rowpf = 1u16 << (row_count - 1);
        let tranrf = if row_count > 1 {
            rowpf | (1u16 << (row_count - 2))
        } else {
            rowpf
        };
        bytes[off + page_size - 4..off + page_size - 2].copy_from_slice(&rowpf.to_le_bytes());
        bytes[off + page_size - 2..off + page_size].copy_from_slice(&tranrf.to_le_bytes());
    }

    let seqdb = read_u32_le_at(bytes, 0x14)
        .unwrap_or(0)
        .max(seq.saturating_add(1));
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

fn build_same_size_track_row_patch(
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

fn mark_track_slot_inactive(
    bytes: &mut [u8],
    page_idx: u32,
    slot: PageRowSlot,
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

    let packed = u32::from(bytes[off + 0x18])
        | (u32::from(bytes[off + 0x19]) << 8)
        | (u32::from(bytes[off + 0x1a]) << 16);
    let num_row_offsets = packed & 0x1fff;
    let num_rows = (packed >> 13) & 0x7ff;
    let new_num_rows = num_rows.saturating_sub(1);
    let new_packed = (num_row_offsets & 0x1fff) | ((new_num_rows & 0x7ff) << 13);
    bytes[off + 0x18] = (new_packed & 0xff) as u8;
    bytes[off + 0x19] = ((new_packed >> 8) & 0xff) as u8;
    bytes[off + 0x1a] = ((new_packed >> 16) & 0xff) as u8;
    // Tombstone state is tracked only in the row footer bitmasks; page_flags
    // must not be changed here (see tombstone policy in remove_ids_from_pdb).
    Ok(())
}

pub(crate) fn patch_playlist_tree_sort_orders_in_place(
    bytes: &mut Vec<u8>,
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
        let payload_len = used_s.min(page_size.saturating_sub(PAGE_HEADER_SIZE));
        let slots = parse_page_row_slots(page, page_size);
        for slot in slots {
            if !slot.present || slot.end > payload_len || slot.start >= slot.end {
                continue;
            }
            let row_abs = off + PAGE_HEADER_SIZE + slot.start;
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
                mark_track_slot_inactive(bytes, page_idx, slot, page_size)?;
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

// ── Orchestrator ────────────────────────────────────────────────────────────

/// Apply a fully-classified additive diff to a copy of the existing PDB
/// bytes. Drives every per-table wrapper in foreign-key dependency order
/// so that newly-allocated dictionary IDs are present in the file before
/// the rows that reference them.
///
/// `t08` playlist_entries handling is delegated entirely to the existing
/// `try_patch_t08_with_context` and `try_patch_t08_with_multi_page_growth`
/// helpers, which understand the latent-slot activation pattern that
/// produces exactly the byte-shape DJ software emits.
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
        let existing_keys: std::collections::HashSet<T08EntryKey> =
            collect_t08_entry_keys(&out, page_size)
                .into_iter()
                .collect();
        let added_keys: Vec<T08EntryKey> = diff
            .desired_t08_entries
            .iter()
            .copied()
            .filter(|k| !existing_keys.contains(k))
            .collect();
        if !added_keys.is_empty() {
            let added_rows: Vec<Vec<u8>> = added_keys
                .iter()
                .map(|k| encode_t08_row(*k).to_vec())
                .collect();
            let _ = append_rows_to_chain_in_place(&mut out, 8, &added_rows, page_size)?;
        }
    }

    if let Some(row_count) = diff.synthesize_t19_runtime_rows {
        synthesize_t19_runtime_rows_in_place(&mut out, row_count, page_size)?;
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

#[cfg(test)]
mod additive_tests {
    use super::*;

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
            let payload_len = used_s.min(PAGE_SIZE.saturating_sub(PAGE_HEADER_SIZE));
            let payload = page.get(PAGE_HEADER_SIZE..PAGE_HEADER_SIZE + payload_len)?;
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

    fn playlist_tree_row_loc_by_id(
        bytes: &[u8],
        id: u32,
    ) -> Option<(u32, usize, usize, PageRowSlot)> {
        let (_ec, first, last) = table_ptr_fields(bytes, 7)?;
        let chain = collect_chain_pages(bytes, PAGE_SIZE, first, last)?;
        for page_idx in chain.into_iter().skip(1) {
            let off = page_offset(page_idx, PAGE_SIZE)?;
            let page = bytes.get(off..off + PAGE_SIZE)?;
            let used_s = read_u16_le_at(page, 0x1e)? as usize;
            let payload_len = used_s.min(PAGE_SIZE.saturating_sub(PAGE_HEADER_SIZE));
            for slot in parse_page_row_slots(page, PAGE_SIZE) {
                if slot.end > payload_len || slot.start >= slot.end {
                    continue;
                }
                let row_abs = off + PAGE_HEADER_SIZE + slot.start;
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
        let (ec_before, first_before, last_before) =
            table_ptr_fields(&bytes, 2).expect("artist ptr");
        let last_off = page_offset(last_before, PAGE_SIZE).expect("artist data page");
        let used_before =
            u16::from_le_bytes(bytes[last_off + 0x1e..last_off + 0x20].try_into().unwrap())
                as usize;
        let payload_before =
            bytes[last_off + PAGE_HEADER_SIZE..last_off + PAGE_HEADER_SIZE + used_before].to_vec();
        let u5_before =
            u16::from_le_bytes(bytes[last_off + 0x20..last_off + 0x22].try_into().unwrap());

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
            &bytes[last_off + PAGE_HEADER_SIZE..last_off + PAGE_HEADER_SIZE + used_before],
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
        let new_rows = vec![
            crate::service::export_helpers::pdb_encoding::encode_artist_row(101, "Test Ártist Ä"),
        ];

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
        bytes[old_off + PAGE_SIZE - 2..old_off + PAGE_SIZE]
            .copy_from_slice(&0x0001u16.to_le_bytes());
        let old_tail_before = bytes[old_off..old_off + PAGE_SIZE].to_vec();

        let mut huge_track = vec![0u8; MAX_ROW_LEN];
        huge_track[0..2].copy_from_slice(&0x0024u16.to_le_bytes());
        huge_track[2..4].copy_from_slice(&0xffffu16.to_le_bytes());
        huge_track[72..76].copy_from_slice(&2u32.to_le_bytes());

        let outcome =
            append_rows_to_chain_in_place(&mut bytes, 0, &[huge_track], PAGE_SIZE).unwrap();
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
        let (_ec_before, first_before, last_before) =
            table_ptr_fields(&bytes, 2).expect("artist ptr");

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
        let (ec_before, first_before, last_before) =
            table_ptr_fields(&bytes, 2).expect("artist ptr");
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
        let parsed_ids: std::collections::HashSet<u32> =
            parsed.tracks.iter().map(|t| t.id).collect();
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
        let old_payload = bytes
            [old_last_off + PAGE_HEADER_SIZE..old_last_off + PAGE_HEADER_SIZE + old_used_s]
            .to_vec();

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

        let outcome = append_playlist_tree_in_place(
            &mut bytes,
            &[(2, 0, 1, false, "Added playlist")],
            PAGE_SIZE,
        )
        .unwrap();

        assert_eq!(outcome.pages_reused, 1);
        assert_eq!(outcome.pages_appended, 0);
        assert_eq!(outcome.new_last_page, old_last);
        assert_eq!(table_ptr_fields(&bytes, 7).unwrap().2, old_last);
        assert_eq!(
            &bytes[old_last_off + PAGE_HEADER_SIZE..old_last_off + PAGE_HEADER_SIZE + old_used_s],
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
        };
        let encoded = encode_track_row_with_profile(&base, PdbLayoutProfile::DEFAULT)
            .expect("encode base track");

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
        let (_ea, first_after, _la) =
            crate::utils::table_ptr_fields(&new_bytes, 0).expect("ptr after");
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
            title: "First Track".into(),
            anlz_path: "/PIONEER/USBANLZ/P000/00000001/ANLZ0000.DAT".into(),
            file_path: "/Contents/first.flac".into(),
        });

        let diff = compute_additive_diff(&bytes, &next, vec![]).expect("additive first export");
        assert_eq!(diff.new_tracks.len(), 1);
        assert_eq!(diff.synthesize_t19_runtime_rows, Some(2));
    }

    #[test]
    fn additive_first_export_preserves_template_menu_pages() {
        use crate::service::export_helpers::PdbTrackRowData;

        let mut empty = PdbData::empty();
        empty.colors = standard_colors();
        empty.columns_raw_rows = standard_columns_raw();
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
        assert_eq!(after[off19 + 0x18], 2, "t19 should have seed + track row");
        assert_eq!(read_u16_le_at(&after, off19 + PAGE_SIZE - 4), Some(0x0002));
        assert_eq!(read_u16_le_at(&after, off19 + PAGE_SIZE - 2), Some(0x0003));
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
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Page/row primitives and T08 playlist-entry patch helpers
// (moved from pdb_patch.rs)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub(crate) fn patch_track_row_slot_bytes_if_len_matches(
    row: &mut [u8],
    slot_index: usize,
    desired: &[u8],
) -> bool {
    if slot_index >= 21 || row.len() < 136 {
        return false;
    }
    let Some(slot_start) = read_u16_le_at(row, 94 + slot_index * 2).map(|v| v as usize) else {
        return false;
    };
    let slot_end = if slot_index + 1 < 21 {
        let Some(next) = read_u16_le_at(row, 94 + (slot_index + 1) * 2).map(|v| v as usize) else {
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
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct T08EntryKey {
    pub entry_index: u32,
    pub track_id: u32,
    pub playlist_id: u32,
}

#[derive(Clone, Debug)]
struct T08EntryLoc {
    key: T08EntryKey,
    page_index: u32,
    raw: [u8; 12],
}

pub(crate) fn footer_size_for_rows(row_count: usize) -> usize {
    if row_count == 0 {
        return 0;
    }
    let groups = row_count.div_ceil(16);
    groups * 4 + row_count * 2
}

pub(crate) fn encode_t08_row(key: T08EntryKey) -> [u8; 12] {
    let mut out = [0u8; 12];
    out[0..4].copy_from_slice(&key.entry_index.to_le_bytes());
    out[4..8].copy_from_slice(&key.track_id.to_le_bytes());
    out[8..12].copy_from_slice(&key.playlist_id.to_le_bytes());
    out
}

fn parse_t08_entries_from_page(page: &[u8], page_index: u32, len_page: usize) -> Vec<T08EntryLoc> {
    if page.len() < len_page || len_page < 64 {
        return Vec::new();
    }
    if read_u32_le_at(page, 0x04).unwrap_or(0) == 0 {
        return Vec::new();
    }
    let used_s = read_u16_le_at(page, 30).unwrap_or(0) as usize;
    if used_s == 0 {
        return Vec::new();
    }
    let payload_start = 40usize;
    let payload_end = payload_start.saturating_add(used_s).min(len_page);
    if payload_end <= payload_start {
        return Vec::new();
    }
    let payload = &page[payload_start..payload_end];
    let nrs = page[24] as usize;
    let num_rl = read_u16_le_at(page, 34).unwrap_or(0) as usize;
    let n_header = if num_rl == 8191 { nrs } else { nrs.max(num_rl) };

    // Compute the maximum number of rows the index space can hold, to detect
    // nrs u8 wrapping on pages with >255 rows.
    let index_space = len_page.saturating_sub(payload_end);
    let full_groups = index_space / 36; // 4 presence bytes + 16×2 offset bytes
    let leftover = index_space % 36;
    let partial_rows = if leftover >= 6 { (leftover - 4) / 2 } else { 0 };
    let n_max = full_groups * 16 + partial_rows;

    if n_header == 0 && n_max == 0 {
        return Vec::new();
    }

    let mut m = len_page;
    let mut row_offsets = Vec::<usize>::with_capacity(n_max + 1);
    let mut row_presence = Vec::<u16>::with_capacity((n_max / 16) + 2);

    // Phase 1: trust the page header count.
    for i in 0..n_header {
        if i % 16 == 0 {
            if m < 4 {
                break;
            }
            m -= 4;
            let Some(bits) = read_u16_le_at(page, m) else {
                break;
            };
            row_presence.push(bits);
        }
        if m < 2 {
            break;
        }
        m -= 2;
        let Some(off) = read_u16_le_at(page, m) else {
            break;
        };
        row_offsets.push(off as usize);
    }

    // Phase 2: if the index space can hold more rows than n_header (nrs likely
    // wrapped at 256), extend the scan with strict validation.
    let gap = n_max.saturating_sub(n_header);
    if gap > 0 && gap <= 512 && row_offsets.len() == n_header {
        let mut prev_off = row_offsets.last().copied();
        for i in n_header..n_max {
            if i % 16 == 0 {
                if m < 4 || m.saturating_sub(4) < payload_end {
                    break;
                }
                m -= 4;
                let Some(bits) = read_u16_le_at(page, m) else {
                    break;
                };
                row_presence.push(bits);
            }
            if m < 2 || m.saturating_sub(2) < payload_end {
                break;
            }
            m -= 2;
            let Some(off) = read_u16_le_at(page, m) else {
                break;
            };
            let off = off as usize;
            if off >= payload.len() {
                break;
            }
            if let Some(prev) = prev_off
                && off < prev
            {
                break; // offsets must be non-decreasing
            }
            prev_off = Some(off);
            row_offsets.push(off);
        }
    }

    let mut out = Vec::<T08EntryLoc>::new();
    for i in 0..row_offsets.len() {
        let group_idx = i / 16;
        let bit = i % 16;
        let present = row_presence
            .get(group_idx)
            .map(|bits| ((bits >> bit) & 1) == 1)
            .unwrap_or(false);
        if !present {
            continue;
        }
        let start = row_offsets[i];
        let end = row_offsets.get(i + 1).copied().unwrap_or(payload.len());
        if end <= start || end > payload.len() || start + 12 > payload.len() {
            continue;
        }
        let mut raw = [0u8; 12];
        raw.copy_from_slice(&payload[start..start + 12]);
        let Some(entry_index) = read_u32_le_at(&raw, 0) else {
            continue;
        };
        let Some(track_id) = read_u32_le_at(&raw, 4) else {
            continue;
        };
        let Some(playlist_id) = read_u32_le_at(&raw, 8) else {
            continue;
        };
        out.push(T08EntryLoc {
            key: T08EntryKey {
                entry_index,
                track_id,
                playlist_id,
            },
            page_index,
            raw,
        });
    }
    out
}

pub(crate) fn collect_t08_entry_keys(bytes: &[u8], page_size: usize) -> Vec<T08EntryKey> {
    let Some((_ec, first, last)) = table_ptr_fields(bytes, 8) else {
        return Vec::new();
    };
    let chain = collect_chain_pages(bytes, page_size, first, last).unwrap_or_default();
    parse_t08_entries_with_locs(bytes, &chain, page_size)
        .into_iter()
        .map(|e| e.key)
        .collect()
}

fn parse_t08_entries_with_locs(bytes: &[u8], chain: &[u32], page_size: usize) -> Vec<T08EntryLoc> {
    let mut out = Vec::<T08EntryLoc>::new();
    for &page_idx in chain.iter().skip(1) {
        let Some(off) = page_offset(page_idx, page_size) else {
            continue;
        };
        let Some(page) = bytes.get(off..off + page_size) else {
            continue;
        };
        out.extend(parse_t08_entries_from_page(page, page_idx, page_size));
    }
    out
}

fn rewrite_t08_page_rows_in_place(
    bytes: &mut [u8],
    page_index: u32,
    rows: &[[u8; 12]],
    page_size: usize,
) -> bool {
    let Some(off) = page_offset(page_index, page_size) else {
        return false;
    };
    let Some(page) = bytes.get_mut(off..off + page_size) else {
        return false;
    };
    let mut payload = Vec::<u8>::new();
    let mut row_offsets = Vec::<u16>::with_capacity(rows.len());
    for row in rows {
        row_offsets.push(payload.len() as u16);
        payload.extend_from_slice(row);
    }
    let used_s = payload.len();
    let footer_size = footer_size_for_rows(rows.len());
    if 40 + used_s + footer_size > page_size {
        return false;
    }

    // Preserve stable page identity/chain/header fields; rebuild row payload/index metadata.
    page[24] = (rows.len() & 0xff) as u8;
    let n = rows.len() as u32;
    let packed = (n & 0x1fff) | ((n & 0x7ff) << 13);
    page[0x18] = (packed & 0xff) as u8;
    page[0x19] = ((packed >> 8) & 0xff) as u8;
    page[0x1a] = ((packed >> 16) & 0xff) as u8;
    // tt=8 (playlist_entries) convention: u5=1 (transaction_row_count of the
    // most recent commit), num_rl=trc-1. The earlier `u5=trc` form caused
    // "communication error" — see docs/PDB.md "Per-table page-header
    // conventions".
    page[0x20..0x22].copy_from_slice(&1u16.to_le_bytes());
    page[30..32].copy_from_slice(&(used_s as u16).to_le_bytes());
    let free_s = page_size.saturating_sub(40 + used_s + footer_size);
    page[28..30].copy_from_slice(&(free_s as u16).to_le_bytes());
    let num_rl = if rows.is_empty() {
        0u16
    } else {
        (rows.len() - 1) as u16
    };
    page[34..36].copy_from_slice(&num_rl.to_le_bytes());

    if page_size > 40 {
        page[40..].fill(0u8);
    }
    if !payload.is_empty() {
        page[40..40 + payload.len()].copy_from_slice(&payload);
    }
    if !row_offsets.is_empty() {
        let mut cursor = page_size;
        for group_start in (0..row_offsets.len()).step_by(16) {
            let group_len = (row_offsets.len() - group_start).min(16);
            let bits = ((1u32 << group_len) - 1) as u16;
            cursor -= 2;
            page[cursor..cursor + 2].copy_from_slice(&bits.to_le_bytes());
            cursor -= 2;
            page[cursor..cursor + 2].copy_from_slice(&bits.to_le_bytes());
            for j in 0..group_len {
                cursor -= 2;
                page[cursor..cursor + 2]
                    .copy_from_slice(&row_offsets[group_start + j].to_le_bytes());
            }
        }
    }
    true
}

fn try_activate_t08_latent_slots_in_place(
    bytes: &mut [u8],
    page_index: u32,
    rows: &[[u8; 12]],
    page_size: usize,
) -> bool {
    if rows.is_empty() {
        return false;
    }
    let Some(off) = page_offset(page_index, page_size) else {
        return false;
    };
    let Some(page) = bytes.get_mut(off..off + page_size) else {
        return false;
    };
    let used_s = read_u16_le_at(page, 30).unwrap_or(0) as usize;
    if used_s < 12 {
        return false;
    }
    let nrs = page[24] as usize;
    let num_rl = read_u16_le_at(page, 34).unwrap_or(0) as usize;
    if num_rl == 8191 {
        return false;
    }
    let current = nrs.max(num_rl);
    if current == 0 {
        return false;
    }
    // Parse existing footer for current slots and remember row-presence locations.
    let mut m = page_size;
    let mut rowpf_off_by_group = Vec::<usize>::new();
    for i in 0..current {
        if i % 16 == 0 {
            if m < 4 {
                return false;
            }
            m -= 4;
            rowpf_off_by_group.push(m);
        }
        if m < 2 {
            return false;
        }
        m -= 2;
    }

    // Activate preallocated latent slots in-footer, extending into additional
    // row groups when present (stale-slot reuse behavior).
    for (k, row) in rows.iter().enumerate() {
        let slot = current + k;
        if slot.is_multiple_of(16) {
            if m < 4 {
                return false;
            }
            m -= 4;
            rowpf_off_by_group.push(m);
        }
        if m < 2 {
            return false;
        }
        m -= 2;
        let Some(slot_off) = read_u16_le_at(page, m).map(|v| v as usize) else {
            return false;
        };
        if slot_off + 12 > used_s {
            return false;
        }
        let payload_start = 40 + slot_off;
        let Some(dst) = page.get_mut(payload_start..payload_start + 12) else {
            return false;
        };
        dst.copy_from_slice(row);

        let group = slot / 16;
        let bit = slot % 16;
        let Some(&rowpf_off) = rowpf_off_by_group.get(group) else {
            return false;
        };
        let Some(rowpf_bytes) = page.get_mut(rowpf_off..rowpf_off + 2) else {
            return false;
        };
        let mut rowpf = u16::from_le_bytes([rowpf_bytes[0], rowpf_bytes[1]]);
        rowpf |= 1u16 << bit;
        rowpf_bytes.copy_from_slice(&rowpf.to_le_bytes());

        // Mark touched row bits in transaction flags for activated slots.
        if let Some(tranrf) = page.get_mut(rowpf_off + 2..rowpf_off + 4) {
            let mut v = u16::from_le_bytes([tranrf[0], tranrf[1]]);
            v |= 1u16 << bit;
            tranrf.copy_from_slice(&v.to_le_bytes());
        }
    }

    let new_num_rl = (current + rows.len()) as u16;
    page[34..36].copy_from_slice(&new_num_rl.to_le_bytes());
    // Keep transaction-row-index non-zero for pages modified in-place.
    page[32..34].copy_from_slice(&1u16.to_le_bytes());
    true
}

#[derive(Debug, Clone)]
pub struct T08PatchContext {
    pub playlist_id: u32,
    pub desired_entries: Vec<T08EntryKey>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PageRowSlot {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) present: bool,
    pub(crate) bits_off: usize,
    pub(crate) bit_mask: u16,
}

pub(crate) fn parse_page_row_slots(page: &[u8], len_page: usize) -> Vec<PageRowSlot> {
    if page.len() < len_page || len_page < 64 {
        return Vec::new();
    }
    let used_s = read_u16_le_at(page, 30).unwrap_or(0) as usize;
    if used_s == 0 {
        return Vec::new();
    }
    let payload_len = used_s.min(len_page.saturating_sub(40));
    let nrs = page[24] as usize;
    let num_rl = read_u16_le_at(page, 34).unwrap_or(0) as usize;
    let n_header = if num_rl == 8191 { nrs } else { nrs.max(num_rl) };
    if n_header == 0 {
        return Vec::new();
    }

    let mut m = len_page;
    let mut row_offsets = Vec::<usize>::with_capacity(n_header);
    let mut row_bits = Vec::<(u16, usize)>::with_capacity(n_header.div_ceil(16));
    for i in 0..n_header {
        if i % 16 == 0 {
            if m < 4 {
                break;
            }
            m -= 4;
            let bits = read_u16_le_at(page, m).unwrap_or(0);
            row_bits.push((bits, m));
        }
        if m < 2 {
            break;
        }
        m -= 2;
        row_offsets.push(read_u16_le_at(page, m).unwrap_or(0) as usize);
    }

    let mut slots = Vec::<PageRowSlot>::new();
    for i in 0..row_offsets.len() {
        let start = row_offsets[i];
        let end = row_offsets.get(i + 1).copied().unwrap_or(payload_len);
        if end <= start || end > payload_len {
            continue;
        }
        let group_idx = i / 16;
        let bit = i % 16;
        let (bits, bits_off) = row_bits.get(group_idx).copied().unwrap_or((0, 0));
        let bit_mask = 1u16 << bit;
        slots.push(PageRowSlot {
            start,
            end,
            present: (bits & bit_mask) != 0,
            bits_off,
            bit_mask,
        });
    }
    slots
}

pub(crate) fn read_present_page_rows(page: &[u8], len_page: usize) -> Vec<Vec<u8>> {
    let used_s = read_u16_le_at(page, 30).unwrap_or(0) as usize;
    if used_s == 0 || page.len() < len_page {
        return Vec::new();
    }
    let payload = &page[40..40 + used_s.min(len_page.saturating_sub(40))];
    let slots = parse_page_row_slots(page, len_page);
    let mut rows = Vec::<Vec<u8>>::new();
    for slot in slots {
        if !slot.present || slot.end > payload.len() || slot.start >= slot.end {
            continue;
        }
        rows.push(payload[slot.start..slot.end].to_vec());
    }
    rows
}

pub(crate) fn rewrite_variable_page_rows_in_place(
    bytes: &mut [u8],
    page_index: u32,
    rows: &[Vec<u8>],
    page_size: usize,
) -> bool {
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
    let footer_size = footer_size_for_rows(row_offsets.len());
    if 40 + used_s + footer_size > page_size {
        return false;
    }

    let n = row_offsets.len() as u32;
    page[24] = (n & 0xff) as u8;
    let packed = (n & 0x1fff) | ((n & 0x7ff) << 13);
    page[0x18] = (packed & 0xff) as u8;
    page[0x19] = ((packed >> 8) & 0xff) as u8;
    page[0x1a] = ((packed >> 16) & 0xff) as u8;
    page[0x20..0x22].copy_from_slice(&(row_offsets.len() as u16).to_le_bytes());
    page[30..32].copy_from_slice(&(used_s as u16).to_le_bytes());
    let free_s = page_size.saturating_sub(40 + used_s + footer_size);
    page[28..30].copy_from_slice(&(free_s as u16).to_le_bytes());
    let num_rl = if row_offsets.is_empty() {
        0u16
    } else {
        (row_offsets.len() - 1) as u16
    };
    page[34..36].copy_from_slice(&num_rl.to_le_bytes());

    if page_size > 40 {
        page[40..].fill(0u8);
    }
    if !payload.is_empty() {
        page[40..40 + payload.len()].copy_from_slice(&payload);
    }

    if !row_offsets.is_empty() {
        let mut cursor = page_size;
        for group_start in (0..row_offsets.len()).step_by(16) {
            let group_len = (row_offsets.len() - group_start).min(16);
            let bits = ((1u32 << group_len) - 1) as u16;
            cursor -= 2;
            page[cursor..cursor + 2].copy_from_slice(&bits.to_le_bytes());
            cursor -= 2;
            page[cursor..cursor + 2].copy_from_slice(&bits.to_le_bytes());
            for j in 0..group_len {
                cursor -= 2;
                page[cursor..cursor + 2]
                    .copy_from_slice(&row_offsets[group_start + j].to_le_bytes());
            }
        }
    }
    true
}

pub fn try_patch_t08_with_context(
    before_bytes: &[u8],
    out: &mut [u8],
    old_chain: &[u32],
    page_size: usize,
    ctx: &T08PatchContext,
) -> bool {
    let debug_t08 = std::env::var("RE_DEBUG_T08")
        .ok()
        .map(|v| {
            let n = v.trim().to_ascii_lowercase();
            n == "1" || n == "true" || n == "yes" || n == "on"
        })
        .unwrap_or(false);
    if ctx.playlist_id == 0 || ctx.desired_entries.is_empty() {
        return false;
    }
    let old_entries = parse_t08_entries_with_locs(before_bytes, old_chain, page_size);
    if old_entries.is_empty() {
        return false;
    }
    let old_set: HashSet<T08EntryKey> = old_entries.iter().map(|r| r.key).collect();
    let mut added = ctx
        .desired_entries
        .iter()
        .copied()
        .filter(|k| !old_set.contains(k))
        .collect::<Vec<_>>();
    if added.is_empty() {
        return false;
    }
    added.sort_by_key(|k| k.entry_index);

    // Prefer pages already used by this playlist.
    let existing_pages = old_entries
        .iter()
        .filter(|r| r.key.playlist_id == ctx.playlist_id)
        .map(|r| r.page_index)
        .collect::<Vec<_>>();

    // For brand new playlists, pick the first page whose playlist-id range
    // starts at or above the target playlist id. This matches observed placement
    // behavior better than blindly appending into the tail page.
    let mut boundary_page: Option<u32> = None;
    if existing_pages.is_empty() {
        let mut page_pid_bounds = HashMap::<u32, (u32, u32)>::new();
        for row in &old_entries {
            let entry = page_pid_bounds
                .entry(row.page_index)
                .or_insert((row.key.playlist_id, row.key.playlist_id));
            entry.0 = entry.0.min(row.key.playlist_id);
            entry.1 = entry.1.max(row.key.playlist_id);
        }
        let mut ordered = page_pid_bounds
            .into_iter()
            .map(|(page, (min_pid, max_pid))| (page, min_pid, max_pid))
            .collect::<Vec<_>>();
        ordered.sort_by_key(|(page, _, _)| *page);
        boundary_page = ordered
            .iter()
            .find(|(_, min_pid, _)| *min_pid >= ctx.playlist_id)
            .map(|(page, _, _)| *page);
    }

    let mut candidates = Vec::<u32>::new();
    if !existing_pages.is_empty() {
        candidates.extend(existing_pages.iter().copied());
        for page in old_chain.iter().skip(1).copied() {
            if !candidates.contains(&page) {
                candidates.push(page);
            }
        }
    } else if let Some(page) = boundary_page {
        // New playlist: write only into the id-boundary page.
        candidates.push(page);
    } else {
        // Fallback when boundary cannot be inferred.
        candidates.extend(old_chain.iter().skip(1).copied());
    }
    if debug_t08 {
        crate::logging::emit(
            crate::logging::Level::Info,
            "pdb-patch.t08",
            &format!(
                "playlist_id={} existing_pages={:?} boundary_page={:?} candidates={:?} added={}",
                ctx.playlist_id,
                existing_pages,
                boundary_page,
                candidates,
                added.len()
            ),
        );
    }

    for target_page in candidates {
        let mut page_rows = old_entries
            .iter()
            .filter(|r| r.page_index == target_page)
            .map(|r| r.raw)
            .collect::<Vec<_>>();
        if page_rows.is_empty() {
            continue;
        }
        if existing_pages.is_empty() && boundary_page == Some(target_page) {
            let latent_rows = added.iter().map(|k| encode_t08_row(*k)).collect::<Vec<_>>();
            if try_activate_t08_latent_slots_in_place(out, target_page, &latent_rows, page_size) {
                if debug_t08 {
                    crate::logging::emit(
                        crate::logging::Level::Info,
                        "pdb-patch.t08",
                        &format!(
                            "success latent-page={} rows_added={}",
                            target_page,
                            latent_rows.len()
                        ),
                    );
                }
                return true;
            }
        }
        // If this is a brand new playlist insertion page, front-load new rows
        // so they are written before higher playlist ids on that page.
        if existing_pages.is_empty() && boundary_page == Some(target_page) {
            let mut prefixed = added.iter().map(|k| encode_t08_row(*k)).collect::<Vec<_>>();
            prefixed.extend(page_rows);
            page_rows = prefixed;
        } else {
            for key in &added {
                page_rows.push(encode_t08_row(*key));
            }
        }
        if rewrite_t08_page_rows_in_place(out, target_page, &page_rows, page_size) {
            if debug_t08 {
                crate::logging::emit(
                    crate::logging::Level::Info,
                    "pdb-patch.t08",
                    &format!(
                        "success page={} rows_before={} rows_after={}",
                        target_page,
                        old_entries
                            .iter()
                            .filter(|r| r.page_index == target_page)
                            .count(),
                        page_rows.len()
                    ),
                );
            }
            return true;
        }
        if debug_t08 {
            crate::logging::emit(
                crate::logging::Level::Info,
                "pdb-patch.t08",
                &format!(
                    "failed page={} rows_before={} rows_after={}",
                    target_page,
                    old_entries
                        .iter()
                        .filter(|r| r.page_index == target_page)
                        .count(),
                    page_rows.len()
                ),
            );
        }
    }
    false
}

/// Multi-page fallback for t08 growth in the export path.
/// Called when `try_patch_t08_with_context` fails because entries don't fit
/// on any single page. Distributes ALL entries (existing + new) evenly across
/// existing data pages and appends new pages for overflow.
pub fn try_patch_t08_with_multi_page_growth(
    out: &mut Vec<u8>,
    old_chain: &[u32],
    old_first: u32,
    old_last: u32,
    page_size: usize,
    ctx: &T08PatchContext,
) -> bool {
    if ctx.playlist_id == 0 || ctx.desired_entries.is_empty() || old_chain.len() <= 1 {
        return false;
    }

    // Parse all existing t08 entries
    let old_entries = parse_t08_entries_with_locs(out, old_chain, page_size);

    // Build merged entry list: keep other playlists' entries, replace target playlist's
    let mut all_rows: Vec<[u8; 12]> = Vec::new();
    for entry in &old_entries {
        if entry.key.playlist_id == ctx.playlist_id {
            continue; // will be replaced by desired_entries
        }
        all_rows.push(entry.raw);
    }
    for key in &ctx.desired_entries {
        all_rows.push(encode_t08_row(*key));
    }

    let data_pages: Vec<u32> = old_chain.iter().skip(1).copied().collect();

    // Calculate max rows per page (fixed 12-byte t08 rows)
    let max_rows_per_page = {
        let mut cap = 0usize;
        loop {
            let next = cap + 1;
            let footer = footer_size_for_rows(next);
            if 40 + next * 12 + footer > page_size {
                break cap;
            }
            cap = next;
        }
    };

    let total_desired = all_rows.len();
    let mut row_cursor = 0usize;

    // Distribute evenly across existing data pages
    for (i, &page_idx) in data_pages.iter().enumerate() {
        let pages_remaining = data_pages.len() - i;
        let rows_remaining = total_desired.saturating_sub(row_cursor);
        let rows_for_this_page = if rows_remaining <= pages_remaining * max_rows_per_page {
            rows_remaining.div_ceil(pages_remaining)
        } else {
            max_rows_per_page
        };
        let rows_for_this_page = rows_for_this_page
            .min(max_rows_per_page)
            .min(total_desired.saturating_sub(row_cursor));

        let page_rows: Vec<Vec<u8>> = all_rows[row_cursor..row_cursor + rows_for_this_page]
            .iter()
            .map(|r| r.to_vec())
            .collect();
        rewrite_variable_page_rows_in_place(out, page_idx, &page_rows, page_size);
        row_cursor += rows_for_this_page;
    }

    // Append new pages for overflow
    let mut current_last = old_last;
    while row_cursor < all_rows.len() {
        let new_page_idx = (out.len() / page_size) as u32;
        out.resize(out.len() + page_size, 0u8);
        let off = new_page_idx as usize * page_size;
        out[off + 0x04..off + 0x08].copy_from_slice(&new_page_idx.to_le_bytes());
        out[off + 0x08..off + 0x0c].copy_from_slice(&8u32.to_le_bytes());
        out[off + 0x10..off + 0x14].copy_from_slice(&1u32.to_le_bytes());
        out[off + 0x1b] = 0x24;

        if let Some(prev_off) = page_offset(current_last, page_size) {
            let _ = write_u32_le_at(out, prev_off + 0x0c, new_page_idx);
        }

        let page_rows: Vec<Vec<u8>> = all_rows[row_cursor..]
            .iter()
            .take(max_rows_per_page)
            .map(|r| r.to_vec())
            .collect();
        let n = page_rows.len();
        if n == 0 {
            break;
        }
        rewrite_variable_page_rows_in_place(out, new_page_idx, &page_rows, page_size);
        row_cursor += n;
        current_last = new_page_idx;
    }

    // Update t08 table pointers if growth happened
    let new_last = current_last;
    let new_ec = if new_last != old_last {
        new_last + 1
    } else {
        // ec unchanged
        table_ptr_fields(out, 8).map(|(ec, _, _)| ec).unwrap_or(0)
    };
    set_table_ptr_fields(out, 8, new_ec, old_first, new_last);

    if new_last != old_last {
        let global_next = read_u32_le_at(out, 0x0c).unwrap_or(0);
        let file_pages = (out.len() / page_size) as u32;
        if file_pages > global_next {
            let _ = write_u32_le_at(out, 0x0c, file_pages);
        }
    }

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

/// Validate that no data page in a table chain has zero rows.
/// Empty data pages mid-chain can break desktop library software.
pub fn validate_no_empty_data_pages(
    bytes: &[u8],
    page_size: usize,
    table_type: u32,
    first: u32,
    last: u32,
) -> crate::error::BackendResult<()> {
    let chain = collect_chain_pages(bytes, page_size, first, last).ok_or_else(|| {
        crate::error::BackendError::Validation(format!(
            "PDB validation failed: invalid t{table_type:02} chain"
        ))
    })?;
    for &page_idx in chain.iter().skip(1) {
        let Some(off) = page_offset(page_idx, page_size) else {
            continue;
        };
        let flags = bytes.get(off + 0x1b).copied().unwrap_or(0);
        if flags & 0x40 != 0 {
            continue;
        }
        let used_size = read_u16_le_at(bytes, off + 30).unwrap_or(0);
        let nrs = bytes.get(off + 0x18).copied().unwrap_or(0);
        if used_size == 0 && nrs == 0 {
            return Err(crate::error::BackendError::Validation(format!(
                "PDB validation failed: t{table_type:02} data page {page_idx} has 0 rows \
                 (empty data pages in chain can break desktop library software)"
            )));
        }
    }
    Ok(())
}
