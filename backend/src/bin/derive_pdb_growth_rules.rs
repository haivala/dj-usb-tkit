use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::env;
use std::path::PathBuf;

use backend::utils::{collect_chain, read_u32_le_at as read_u32_le};
use serde::Serialize;

const PAGE_SIZE: usize = 4096;
const TABLE_COUNT: u32 = 20;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct HeaderFields {
    total_pages: u32,
    next_unused_page: u32,
    seqdb: u32,
}

#[derive(Debug, Clone, Copy)]
struct TablePtrs {
    ec: u32,
    first: u32,
    last: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TableGrowth {
    table_type: u32,
    old_first: u32,
    old_last: u32,
    old_ec: u32,
    new_first: u32,
    new_last: u32,
    new_ec: u32,
    old_chain_len: usize,
    new_chain_len: usize,
    first_preserved: bool,
    chain_prefix_preserved: bool,
    appended_pages: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GrowthSummary {
    old_header: HeaderFields,
    new_header: HeaderFields,
    grew_tables: Vec<u32>,
    all_grown_tables_preserve_first: bool,
    all_grown_tables_append_old_chain_as_prefix: bool,
    appended_page_count_by_table: BTreeMap<u32, usize>,
    appended_pages_from_virtual_ids: usize,
    appended_pages_total: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GrowthReport {
    old_pdb: String,
    new_pdb: String,
    tables: Vec<TableGrowth>,
    summary: GrowthSummary,
}

fn header_fields(bytes: &[u8]) -> Option<HeaderFields> {
    let total_pages = (bytes.len() / PAGE_SIZE).checked_sub(1).map(|v| v as u32)?;
    let next_unused_page = read_u32_le(bytes, 0x0c)?;
    let seqdb = read_u32_le(bytes, 0x14)?;
    Some(HeaderFields {
        total_pages,
        next_unused_page,
        seqdb,
    })
}

fn table_ptrs(bytes: &[u8], table_type: u32) -> Option<TablePtrs> {
    let off = 0x1cusize + table_type as usize * 16;
    Some(TablePtrs {
        ec: read_u32_le(bytes, off + 4)?,
        first: read_u32_le(bytes, off + 8)?,
        last: read_u32_le(bytes, off + 12)?,
    })
}

fn starts_with_chain(new_chain: &[u32], old_chain: &[u32]) -> bool {
    if old_chain.len() > new_chain.len() {
        return false;
    }
    old_chain.iter().zip(new_chain.iter()).all(|(a, b)| a == b)
}

fn parse_pdb_arg(input: &str) -> PathBuf {
    let as_path = PathBuf::from(input);
    if as_path.is_file() {
        return as_path;
    }
    as_path.join("PIONEER").join("rekordbox").join("export.pdb")
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        eprintln!(
            "usage: cargo run --bin derive_pdb_growth_rules -- <old_pdb_or_usb_root> <new_pdb_or_usb_root>"
        );
        std::process::exit(2);
    }

    let old_path = parse_pdb_arg(&args[1]);
    let new_path = parse_pdb_arg(&args[2]);
    let old_bytes = match std::fs::read(&old_path) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("failed to read {}: {err}", old_path.display());
            std::process::exit(1);
        }
    };
    let new_bytes = match std::fs::read(&new_path) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("failed to read {}: {err}", new_path.display());
            std::process::exit(1);
        }
    };

    let Some(old_header) = header_fields(&old_bytes) else {
        eprintln!("failed to read old header fields");
        std::process::exit(1);
    };
    let Some(new_header) = header_fields(&new_bytes) else {
        eprintln!("failed to read new header fields");
        std::process::exit(1);
    };

    let mut table_rows = Vec::<TableGrowth>::new();
    let mut old_used_pages = BTreeSet::<u32>::new();

    for tt in 0..TABLE_COUNT {
        let Some(op) = table_ptrs(&old_bytes, tt) else {
            continue;
        };
        if let Some(chain) = collect_chain(&old_bytes, PAGE_SIZE, op.first, op.last) {
            for page in chain {
                old_used_pages.insert(page);
            }
        }
    }

    let mut grew_tables = Vec::<u32>::new();
    let mut all_preserve_first = true;
    let mut all_prefix = true;
    let mut appended_count_by_table = BTreeMap::<u32, usize>::new();
    let mut appended_pages_total = 0usize;
    let mut appended_pages_from_virtual_ids = 0usize;

    for tt in 0..TABLE_COUNT {
        let Some(op) = table_ptrs(&old_bytes, tt) else {
            continue;
        };
        let Some(np) = table_ptrs(&new_bytes, tt) else {
            continue;
        };
        let old_chain = collect_chain(&old_bytes, PAGE_SIZE, op.first, op.last).unwrap_or_default();
        let new_chain = collect_chain(&new_bytes, PAGE_SIZE, np.first, np.last).unwrap_or_default();

        let first_preserved = op.first == np.first;
        let prefix = starts_with_chain(&new_chain, &old_chain);
        let appended_pages = if prefix {
            new_chain
                .iter()
                .skip(old_chain.len())
                .copied()
                .collect::<Vec<_>>()
        } else {
            let old_set = old_chain.iter().copied().collect::<HashSet<_>>();
            new_chain
                .iter()
                .filter(|p| !old_set.contains(p))
                .copied()
                .collect::<Vec<_>>()
        };

        if new_chain.len() > old_chain.len() {
            grew_tables.push(tt);
            all_preserve_first &= first_preserved;
            all_prefix &= prefix;
        }

        for page in &appended_pages {
            if !old_used_pages.contains(page) {
                appended_pages_from_virtual_ids += 1;
            }
        }
        appended_pages_total += appended_pages.len();
        appended_count_by_table.insert(tt, appended_pages.len());

        table_rows.push(TableGrowth {
            table_type: tt,
            old_first: op.first,
            old_last: op.last,
            old_ec: op.ec,
            new_first: np.first,
            new_last: np.last,
            new_ec: np.ec,
            old_chain_len: old_chain.len(),
            new_chain_len: new_chain.len(),
            first_preserved,
            chain_prefix_preserved: prefix,
            appended_pages,
        });
    }

    let report = GrowthReport {
        old_pdb: old_path.to_string_lossy().to_string(),
        new_pdb: new_path.to_string_lossy().to_string(),
        tables: table_rows,
        summary: GrowthSummary {
            old_header,
            new_header,
            grew_tables,
            all_grown_tables_preserve_first: all_preserve_first,
            all_grown_tables_append_old_chain_as_prefix: all_prefix,
            appended_page_count_by_table: appended_count_by_table,
            appended_pages_from_virtual_ids,
            appended_pages_total,
        },
    };

    match serde_json::to_string_pretty(&report) {
        Ok(json) => println!("{json}"),
        Err(err) => {
            eprintln!("failed to serialize report: {err}");
            std::process::exit(1);
        }
    }
}
