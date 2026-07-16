/// dump_pdb_page_headers <pdb>
///
/// Structural summary of every page in a PDB — table type, key header fields,
/// sentinel B-tree entries, data page footer shape. Used to diff two PDBs at
/// the shape level without noise from content differences.
///
/// Usage:
///   cargo run --features dev-tools --manifest-path backend/Cargo.toml \
///       --bin dump_pdb_page_headers -- <usb_root_or_pdb>
///
/// Diff two PDBs structurally:
///   diff <(./dump_pdb_page_headers A.pdb) <(./dump_pdb_page_headers B.pdb)
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use backend::utils::{read_u8_at as ru8, read_u16_le_at as ru16, read_u32_le_at as ru32};

fn table_name(tt: u32) -> &'static str {
    match tt {
        0 => "tracks",
        1 => "genres",
        2 => "artists",
        3 => "albums",
        4 => "labels",
        5 => "keys",
        6 => "colors",
        7 => "playlist_tree",
        8 => "playlist_entries",
        13 => "artwork",
        16 => "columns",
        17 => "history_playlists",
        18 => "history_entries",
        19 => "history_runtime",
        _ => "other",
    }
}

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 2 {
        eprintln!(
            "usage: cargo run --features dev-tools --manifest-path backend/Cargo.toml \\\n\
             --bin dump_pdb_page_headers -- <usb_root_or_pdb>"
        );
        std::process::exit(2);
    }
    let input = Path::new(&args[1]);
    let pdb_path = resolve_pdb(input);
    let bytes = match fs::read(&pdb_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("cannot read {}: {e}", pdb_path.display());
            std::process::exit(1);
        }
    };

    let Some(page_size) = ru32(&bytes, 4).map(|v| v as usize) else {
        eprintln!("invalid PDB: cannot read page_size");
        std::process::exit(1);
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        eprintln!("invalid PDB: too small");
        std::process::exit(1);
    }
    let total_pages = bytes.len() / page_size;
    let num_tables = ru32(&bytes, 8).unwrap_or(0) as usize;
    let next_unused = ru32(&bytes, 0x0c).unwrap_or(0);
    let seqdb = ru32(&bytes, 0x14).unwrap_or(0);

    println!("pdb={}", pdb_path.display());
    println!(
        "pages={total_pages} page_size={page_size} num_tables={num_tables} next_unused={next_unused} seqdb={seqdb}"
    );
    println!();

    // Page 0: file header
    {
        // Table pointer array
        for i in 0..num_tables {
            let off = 0x1c + i * 16;
            if off + 16 > bytes.len() {
                break;
            }
            let tt = ru32(&bytes, off).unwrap_or(0);
            let ec = ru32(&bytes, off + 4).unwrap_or(0);
            let first = ru32(&bytes, off + 8).unwrap_or(0);
            let last = ru32(&bytes, off + 12).unwrap_or(0);
            println!(
                "table[{i:02}] tt={tt:2}({}) first={first} last={last} empty_candidate={ec}",
                table_name(tt)
            );
        }
        println!();
    }

    // Count data pages per table type for stale B-tree detection.
    let mut data_pages_per_tt: std::collections::HashMap<u32, usize> =
        std::collections::HashMap::new();
    for i in 1..total_pages {
        let off = i * page_size;
        let idx = ru32(&bytes, off + 4).unwrap_or(0);
        if idx == 0 {
            continue;
        }
        let flags = ru8(&bytes, off + 0x1b).unwrap_or(0);
        if flags == 0x64 {
            continue;
        }
        let used_s = ru16(&bytes, off + 0x1e).unwrap_or(0);
        if used_s == 0 {
            continue;
        }
        let tt = ru32(&bytes, off + 8).unwrap_or(9999);
        *data_pages_per_tt.entry(tt).or_insert(0) += 1;
    }

    // Pages 1..N
    for i in 1..total_pages {
        let off = i * page_size;
        let idx = ru32(&bytes, off + 4).unwrap_or(0);
        if idx == 0 {
            println!("page[{i:3}]  (blank/unused)");
            continue;
        }
        let tt = ru32(&bytes, off + 8).unwrap_or(0);
        let next = ru32(&bytes, off + 0x0c).unwrap_or(0);
        let seq = ru32(&bytes, off + 0x10).unwrap_or(0);
        let nrs = ru8(&bytes, off + 0x18).unwrap_or(0);
        let u3 = ru8(&bytes, off + 0x19).unwrap_or(0);
        let flags = ru8(&bytes, off + 0x1b).unwrap_or(0);
        let free_s = ru16(&bytes, off + 0x1c).unwrap_or(0);
        let used_s = ru16(&bytes, off + 0x1e).unwrap_or(0);
        let u5 = ru16(&bytes, off + 0x20).unwrap_or(0);
        let num_rl = ru16(&bytes, off + 0x22).unwrap_or(0);
        let u6 = ru16(&bytes, off + 0x24).unwrap_or(0);
        let u7 = ru16(&bytes, off + 0x26).unwrap_or(0);

        if flags == 0x64 {
            // Sentinel / index page
            let sent_next = ru32(&bytes, off + 0x0c).unwrap_or(0);
            let dup_next = ru32(&bytes, off + 0x2c).unwrap_or(0);
            let ne = ru16(&bytes, off + 0x38).unwrap_or(0);
            let fe = ru16(&bytes, off + 0x3a).unwrap_or(0);
            // Determine if B-tree is stale: has entries but doesn't cover all data pages.
            let actual = *data_pages_per_tt.get(&tt).unwrap_or(&0);
            let mut stale = ne > 0 && actual > ne as usize;
            let mut entry_strs: Vec<String> = Vec::new();
            for slot in 0..ne as usize {
                let ev = ru32(&bytes, off + 0x3c + slot * 4).unwrap_or(0xffff_ffff);
                if ev == 0x1fff_fff8 {
                    stale = true;
                    entry_strs.push(format!("s{slot}=EMPTY"));
                } else {
                    let pi = (ev / 8) as usize;
                    let poff = pi * page_size;
                    let valid = poff + page_size <= bytes.len() && {
                        let ptt = ru32(&bytes, poff + 8).unwrap_or(9999);
                        let pfl = ru8(&bytes, poff + 0x1b).unwrap_or(0);
                        ptt == tt && pfl != 0x64
                    };
                    if !valid {
                        stale = true;
                    }
                    entry_strs.push(format!("s{slot}=0x{ev:04x}(p{pi})"));
                }
            }
            let stale_tag = if stale { " [STALE]" } else { "" };
            let entries_str = if entry_strs.is_empty() {
                String::new()
            } else {
                format!(" entries=[{}]", entry_strs.join(", "))
            };
            println!(
                "page[{i:3}]  sentinel  tt={tt:2}({}) next={sent_next} seq={seq} \
                 dup_next=0x{dup_next:08x} ne={ne} fe=0x{fe:04x} u7={u7}{stale_tag}{entries_str}",
                table_name(tt)
            );
        } else {
            // Data page
            let page_type = match flags {
                0x34 => "data(track)",
                0x24 => "data",
                0x44 => "data(deleted)",
                0x54 => "data(track,del)",
                _ => "data(?)",
            };
            // Count active rows from rowpf (first group only for brevity)
            let row_slots = if num_rl == 0x1FFF {
                nrs as usize
            } else {
                (nrs as usize).max(num_rl as usize)
            };
            let groups = row_slots.div_ceil(16);
            let mut rowpf_active = 0u32;
            let mut tranrf_bits = 0u32;
            if groups > 0 && page_size >= groups * 4 + groups * 2 {
                let mut cursor = page_size;
                'footer: for g in 0..groups {
                    if cursor < 4 {
                        break 'footer;
                    }
                    cursor -= 4;
                    let rp =
                        ru16(bytes.get(off..off + page_size).unwrap_or(&[]), cursor).unwrap_or(0);
                    let tr = ru16(bytes.get(off..off + page_size).unwrap_or(&[]), cursor + 2)
                        .unwrap_or(0);
                    let glen = (row_slots - g * 16).min(16);
                    let mask = if glen == 16 {
                        u16::MAX
                    } else {
                        ((1u32 << glen) - 1) as u16
                    };
                    rowpf_active += (rp & mask).count_ones();
                    tranrf_bits += (tr & mask).count_ones();
                    if cursor < glen * 2 {
                        break 'footer;
                    }
                    cursor -= glen * 2;
                }
            }
            println!(
                "page[{i:3}]  {page_type}  tt={tt:2}({}) next={next} seq={seq} \
                 nrs={nrs} u3=0x{u3:02x} flags=0x{flags:02x} free={free_s} used={used_s} \
                 u5={u5} num_rl={num_rl} u6={u6} u7={u7} \
                 rowpf_active={rowpf_active} tranrf_bits={tranrf_bits}",
                table_name(tt)
            );
        }
    }
}

fn resolve_pdb(input: &Path) -> PathBuf {
    if input
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case("export.pdb"))
        .unwrap_or(false)
    {
        return input.to_path_buf();
    }
    input.join("PIONEER").join("rekordbox").join("export.pdb")
}
