use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use backend::utils::{
    read_u8_at as read_u8, read_u16_le_at as read_u16_le, read_u32_le_at as read_u32_le,
};

#[derive(Debug, Default)]
struct TableLayoutStats {
    pages: usize,
    pages_with_rows: usize,
    empty_pages: usize,
    sentinel_8191_pages: usize,
    nrs_wrapping_pages: usize,
    max_rows_on_page: usize,
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 2 {
        eprintln!("usage: cargo run --bin dump_pdb_layout -- <usb_root_or_pdb_path>");
        std::process::exit(2);
    }

    let input = Path::new(&args[1]);
    let pdb_path = resolve_pdb_path(input);
    let bytes = match fs::read(&pdb_path) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("failed to read {}: {err}", pdb_path.display());
            std::process::exit(1);
        }
    };

    if bytes.len() < 8 {
        eprintln!("invalid PDB: file too short");
        std::process::exit(1);
    }
    let len_page = read_u32_le(&bytes, 4).unwrap_or(0) as usize;
    if len_page == 0 {
        eprintln!("invalid PDB: len_page=0");
        std::process::exit(1);
    }

    let max_page = bytes.len() / len_page;
    let mut by_type = BTreeMap::<u32, TableLayoutStats>::new();
    let mut total_pages = 0usize;
    let mut total_sentinel = 0usize;
    let mut total_nrs_wrap = 0usize;

    for page_idx in 1..max_page {
        let offset = page_idx * len_page;
        if offset + 40 > bytes.len() {
            break;
        }
        total_pages += 1;
        let page = &bytes[offset..offset + len_page];

        let table_type = read_u32_le(page, 8).unwrap_or(0);
        let nrs = read_u8(page, 24).unwrap_or(0);
        let num_rl = read_u16_le(page, 34).unwrap_or(0);
        let rows = parse_row_count(page, len_page).unwrap_or(0);

        let stats = by_type.entry(table_type).or_default();
        stats.pages += 1;
        if rows == 0 {
            stats.empty_pages += 1;
        } else {
            stats.pages_with_rows += 1;
        }
        stats.max_rows_on_page = stats.max_rows_on_page.max(rows);

        if num_rl == 8191 {
            stats.sentinel_8191_pages += 1;
            total_sentinel += 1;
        }
        if rows > nrs as usize {
            stats.nrs_wrapping_pages += 1;
            total_nrs_wrap += 1;
        }
    }

    println!("pdb_path={}", pdb_path.display());
    println!("len_page={}", len_page);
    println!("total_pages={}", total_pages);
    println!("pages_with_num_rl_8191={}", total_sentinel);
    println!("nrs_wrapping_pages={}", total_nrs_wrap);
    println!("tables={}", by_type.len());
    let num_tables = read_u32_le(&bytes, 8).unwrap_or(0) as usize;
    println!("header_num_tables={}", num_tables);
    for i in 0..num_tables {
        let off = 28 + i * 16;
        if off + 16 > bytes.len() {
            break;
        }
        let table_type = read_u32_le(&bytes, off).unwrap_or(0);
        let empty_candidate = read_u32_le(&bytes, off + 4).unwrap_or(0);
        let first_page = read_u32_le(&bytes, off + 8).unwrap_or(0);
        let last_page = read_u32_le(&bytes, off + 12).unwrap_or(0);
        // Verify empty_candidate == last_page.next_page (invariant from debug log)
        let last_next = if last_page as usize * len_page + 16 <= bytes.len() {
            read_u32_le(&bytes, last_page as usize * len_page + 12).unwrap_or(0)
        } else {
            u32::MAX
        };
        let ec_ok = empty_candidate == last_next;
        println!(
            "table_ptr[{}] type={} ({}) first={} last={} empty_candidate={}{ec_note}",
            i,
            table_type,
            table_type_name(table_type),
            first_page,
            last_page,
            empty_candidate,
            ec_note = if ec_ok { "" } else { " MISMATCH(last.next)" },
        );
    }

    for (table_type, stats) in by_type {
        println!(
            "table_type={} ({}) pages={} rows_pages={} empty_pages={} sentinel_8191={} nrs_wrap={} max_rows={}",
            table_type,
            table_type_name(table_type),
            stats.pages,
            stats.pages_with_rows,
            stats.empty_pages,
            stats.sentinel_8191_pages,
            stats.nrs_wrapping_pages,
            stats.max_rows_on_page
        );
    }

    for page_idx in 1..max_page {
        let offset = page_idx * len_page;
        if offset + 40 > bytes.len() {
            break;
        }
        let page = &bytes[offset..offset + len_page];
        let page_index = read_u32_le(page, 4).unwrap_or(0);
        let table_type = read_u32_le(page, 8).unwrap_or(0);
        let next_page = read_u32_le(page, 12).unwrap_or(0);
        let seq = read_u32_le(page, 16).unwrap_or(0);
        let nrs = read_u8(page, 24).unwrap_or(0);
        let u3 = read_u8(page, 25).unwrap_or(0);
        let pf = read_u8(page, 26).unwrap_or(0);
        let u4 = read_u8(page, 27).unwrap_or(0);
        let free_s = read_u16_le(page, 28).unwrap_or(0);
        let used_s = read_u16_le(page, 30).unwrap_or(0);
        let u5 = read_u16_le(page, 32).unwrap_or(0);
        let num_rl = read_u16_le(page, 34).unwrap_or(0);
        println!(
            "page[{}] idx={} tt={} ({}) next={} seq={} nrs={} u3={} pf={} u4={} free_s={} used_s={} u5={} num_rl={}",
            page_idx,
            page_index,
            table_type,
            table_type_name(table_type),
            next_page,
            seq,
            nrs,
            u3,
            pf,
            u4,
            free_s,
            used_s,
            u5,
            num_rl
        );
    }
}

fn resolve_pdb_path(input: &Path) -> PathBuf {
    if input
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("export.pdb"))
        .unwrap_or(false)
    {
        return input.to_path_buf();
    }
    input.join("PIONEER").join("rekordbox").join("export.pdb")
}

fn table_type_name(raw: u32) -> &'static str {
    match raw {
        0 => "tracks",
        1 => "genres",
        2 => "artists",
        3 => "albums",
        4 => "labels",
        5 => "keys",
        6 => "colors",
        7 => "playlist_tree",
        8 => "playlist_entries",
        10 => "history_playlists",
        11 => "history_playlists_alt",
        12 => "history_entries_alt",
        13 => "artwork",
        16 => "columns",
        17 => "history_playlists",
        18 => "history_entries",
        19 => "history",
        _ => "other",
    }
}

fn parse_row_count(page: &[u8], len_page: usize) -> Option<usize> {
    let nrs = read_u8(page, 24)? as usize;
    let num_rl = read_u16_le(page, 34)?;
    if num_rl == 8191 {
        return Some(parse_rows_from_nrs_fallback(page, len_page, nrs));
    }
    Some(((num_rl as usize) & 0x1FFF).min(nrs))
}

fn parse_rows_from_nrs_fallback(page: &[u8], len_page: usize, nrs: usize) -> usize {
    if nrs == 0 {
        return 0;
    }
    let mut cursor = len_page;
    let mut rows = 0usize;
    let groups = nrs.div_ceil(16);
    for _ in 0..groups {
        if cursor < 4 {
            break;
        }
        cursor -= 4;
        let active = match read_u16_le(page, cursor) {
            Some(v) => v,
            None => break,
        };
        let declared = usize::try_from(active.count_ones()).unwrap_or(0);
        if declared == 0 {
            continue;
        }
        if cursor < declared * 2 {
            break;
        }
        cursor -= declared * 2;
        rows += declared;
    }
    rows.min(nrs)
}
