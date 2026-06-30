/// Scan a PDB for known structural anomalies that cause player rejection.
///
/// Checks:
///   - seqdb > max(seqpage) across all data pages
///   - empty_candidate == last_page.next for every table pointer
///   - data pages with u5=0x1FFF (sentinel on non-empty page)
///   - data pages with wrong page_flags (0x24 vs 0x34 per table type)
///   - tt=16/17/18 pages with wrong (1, nrs-1) u5/num_rl shape instead of (nrs, 0)
///   - sentinel pages with stale B-tree index entries (num_entries > 0)
///   - per-page footer rowpf/tranrf for tt=0 (tracks) pages
///
/// Usage:
///   cargo run --features dev-tools --manifest-path backend/Cargo.toml \
///       --bin dump_pdb_anomalies -- <usb_root_or_pdb>
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use backend::utils::{read_u8_at as ru8, read_u16_le_at as ru16, read_u32_le_at as ru32};

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 2 {
        eprintln!(
            "usage: cargo run --features dev-tools --manifest-path backend/Cargo.toml \\\n\
             --bin dump_pdb_anomalies -- <usb_root_or_pdb>"
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
        eprintln!("invalid PDB: cannot read page size");
        std::process::exit(1);
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        eprintln!("invalid PDB: file too small");
        std::process::exit(1);
    }
    let total_pages = bytes.len() / page_size;
    let num_tables = ru32(&bytes, 8).unwrap_or(0) as usize;
    let seqdb = ru32(&bytes, 0x14).unwrap_or(0);
    let next_unused = ru32(&bytes, 0x0c).unwrap_or(0);

    println!("pdb: {}", pdb_path.display());
    println!("pages={total_pages}  page_size={page_size}  num_tables={num_tables}");
    println!("seqdb={seqdb}  next_unused={next_unused}");
    println!();

    // ── seqdb constraint ──────────────────────────────────────────────────
    let max_seqpage = (1..total_pages)
        .filter_map(|i| {
            let off = i * page_size;
            let idx = ru32(&bytes, off + 4)?;
            if idx == 0 {
                return None;
            }
            ru32(&bytes, off + 0x10)
        })
        .max()
        .unwrap_or(0);
    let seqdb_ok = seqdb > max_seqpage;
    println!(
        "seqdb constraint: seqdb={seqdb} max(seqpage)={max_seqpage}  {}",
        if seqdb_ok {
            "OK"
        } else {
            "FAIL — seqdb must be > max(seqpage)"
        }
    );
    println!();

    // ── empty_candidate == last_page.next ─────────────────────────────────
    println!("Table pointer empty_candidate vs last_page.next:");
    let mut ec_ok_all = true;
    for i in 0..num_tables {
        let off = 28 + i * 16;
        if off + 16 > bytes.len() {
            break;
        }
        let tt = ru32(&bytes, off).unwrap_or(0);
        let ec = ru32(&bytes, off + 4).unwrap_or(0);
        let last_page = ru32(&bytes, off + 12).unwrap_or(0) as usize;
        let last_next = if last_page * page_size + 16 <= bytes.len() {
            ru32(&bytes, last_page * page_size + 12).unwrap_or(u32::MAX)
        } else {
            u32::MAX
        };
        let ok = ec == last_next;
        if !ok {
            ec_ok_all = false;
        }
        println!(
            "  t{tt:02}  empty_candidate={ec}  last({last_page}).next={last_next}  {}",
            if ok { "OK" } else { "MISMATCH" }
        );
    }
    if ec_ok_all {
        println!("  all OK");
    }
    println!();

    // ── per-page anomaly scan ─────────────────────────────────────────────
    println!("Per-page anomalies (non-empty data pages only):");
    let mut found_any = false;
    // Count transaction-class (0x34) pages per table for sentinel B-tree checks.
    let mut active_pages_per_tt: std::collections::HashMap<u32, Vec<usize>> =
        std::collections::HashMap::new();
    let mut t00_pages = Vec::<(usize, u8, u16, u16)>::new();

    for i in 1..total_pages {
        let off = i * page_size;
        let idx = ru32(&bytes, off + 4).unwrap_or(0);
        if idx == 0 {
            continue;
        }
        let used_s = ru16(&bytes, off + 0x1e).unwrap_or(0);
        if used_s == 0 {
            continue;
        }
        let tt = ru32(&bytes, off + 8).unwrap_or(0);
        let flags = ru8(&bytes, off + 0x1b).unwrap_or(0);
        if flags == 0x64 {
            continue;
        } // sentinel page
        let nrs = ru8(&bytes, off + 0x18).unwrap_or(0);
        let u5 = ru16(&bytes, off + 0x20).unwrap_or(0);
        let num_rl = ru16(&bytes, off + 0x22).unwrap_or(0);
        if tt == 0 {
            t00_pages.push((i, flags, u5, num_rl));
        }

        // Valid page_flags for data pages: 0x24 (normal/sealed) or 0x34
        // (transaction-class). Large reference exports can contain multiple 0x34
        // pages in the same table.
        let flag_bad = flags != 0x24 && flags != 0x34;
        if flags == 0x34 {
            active_pages_per_tt.entry(tt).or_default().push(i);
        }
        let u5_bad = u5 == 0x1FFF && flags != 0x34;
        let rl_bad = num_rl != 0x1FFF
            && nrs > 0
            && num_rl as usize >= nrs as usize
            // tt=8 playlist-entry pages can exceed 255 rows. In that case
            // nrs wraps but num_rl keeps the real row index.
            && tt != 8;
        // tt=16/17/18: correct shape is (nrs, 0); detect old-bug pattern (1, nrs-1).
        let history_shape_bad = matches!(tt, 16 | 17 | 18)
            && nrs > 1
            && u5 == 1
            && num_rl == (nrs as u16).saturating_sub(1);

        if flag_bad || u5_bad || rl_bad || history_shape_bad {
            found_any = true;
            println!("  page[{i}] tt={tt} flags={flags:#04x} u5={u5} num_rl={num_rl} nrs={nrs}");
            if flag_bad {
                println!("    ANOMALY: page_flags={flags:#04x} not a valid data page flag");
            }
            if u5_bad {
                println!("    ANOMALY: u5=0x1FFF (sentinel) on non-empty data page");
            }
            if rl_bad {
                println!("    ANOMALY: num_rl={num_rl} >= nrs={nrs} (out of range)");
            }
            if history_shape_bad {
                println!(
                    "    ANOMALY: tt={tt} u5=1 num_rl={num_rl} — wrong (1,nrs-1) shape; expected (nrs={nrs},0)"
                );
            }
        }
    }
    // Multiple 0x34 pages per table are valid in working large exports.
    // No separate "only one active page" rule is enforced here.
    if t00_pages.len() > 1
        && t00_pages
            .iter()
            .all(|(_, flags, u5, num_rl)| *flags == 0x34 && *u5 == 2 && *num_rl == 0)
    {
        found_any = true;
        let pages = t00_pages
            .iter()
            .map(|(page, _, _, _)| page.to_string())
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "  t00 pages [{pages}] all use single-page active convention \
             (flags=0x34,u5=2,num_rl=0)  ANOMALY: multi-page track chains must finalize \
             earlier pages or DJ software reports corruption"
        );
    }
    // ── tt=0 page-flag convention check ──────────────────────────────────────
    // Convention (verified on working reference exports):
    //   - Single-page chain (only baseline): must be ACTV (0x34)
    //   - Multi-page chain (baseline + overflow): ALL pages must be SEAL (0x24)
    //
    // Single-page export (9 tracks, 1 page): pf=0x34 ✓
    // Two-page export (10 tracks, 2 pages): both pf=0x24 ✓
    // Large export (60 tracks, multi-page): all pf=0x24 ✓
    {
        let sentinel_page = {
            let mut found = 0usize;
            for i in 0..num_tables {
                let off = 28 + i * 16;
                if off + 16 > bytes.len() {
                    break;
                }
                if ru32(&bytes, off).unwrap_or(9999) == 0 {
                    found = ru32(&bytes, off + 8).unwrap_or(0) as usize;
                    break;
                }
            }
            found
        };
        if sentinel_page > 0 && sentinel_page * page_size + 16 <= bytes.len() {
            let first_data = ru32(&bytes, sentinel_page * page_size + 0xc).unwrap_or(0) as usize;
            if first_data > 0 && first_data < total_pages {
                let is_multi_page = t00_pages.len() > 1;
                if is_multi_page {
                    // All pages in a multi-page chain must be SEAL (0x24).
                    for &(pg, pf, _u5, _num_rl) in &t00_pages {
                        let used = ru16(&bytes, pg * page_size + 0x1e).unwrap_or(0);
                        if used > 0 && pf == 0x34 {
                            found_any = true;
                            let role = if pg == first_data {
                                "first"
                            } else {
                                "overflow"
                            };
                            println!(
                                "  t00 page[{pg}] {role} data page of multi-page chain has \
                                 pf=0x34 (ACTV) but must be SEAL (0x24)  \
                                 ANOMALY: DJ software rejects as corrupted"
                            );
                        }
                    }
                } else {
                    // Single-page chain must be ACTV (0x34).
                    let first_pf = ru8(&bytes, first_data * page_size + 0x1b).unwrap_or(0);
                    if first_pf != 0x34 {
                        found_any = true;
                        println!(
                            "  t00 page[{first_data}] single data page has pf={first_pf:#04x} \
                             but must be ACTV (0x34)  ANOMALY: DJ software rejects as corrupted"
                        );
                    }
                }
            }
        }
    }
    if !found_any {
        println!("  none");
    }
    println!();

    // ── sentinel page stale B-tree check ─────────────────────────────────
    // B-tree entries must match the ACTIVE (flags=0x34) pages for that table.
    // Sealed (0x24) pages are navigated via next_page chain and are not indexed.
    println!("Sentinel B-tree index check:");
    // Count active (0x34) pages and all data pages per table type.
    let mut data_pages_per_tt: std::collections::HashMap<u32, usize> =
        std::collections::HashMap::new();
    let mut active_per_tt: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
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
        if flags == 0x34 {
            *active_per_tt.entry(tt).or_insert(0) += 1;
        }
    }
    let mut sentinel_btree_ok = true;
    for i in 1..total_pages {
        let off = i * page_size;
        let idx = ru32(&bytes, off + 4).unwrap_or(0);
        if idx == 0 {
            continue;
        }
        let flags = ru8(&bytes, off + 0x1b).unwrap_or(0);
        if flags != 0x64 {
            continue;
        }
        let ne = ru16(&bytes, off + 0x38).unwrap_or(0);
        let u7_pre = ru16(&bytes, off + 0x26).unwrap_or(0);
        let tt = ru32(&bytes, off + 8).unwrap_or(0);
        // B-tree indexes transaction-class (0x34) pages only. Large reference exports
        // may contain empty or duplicate slots, so do not require an exact
        // `ne == active_pages` count here.
        let actual_dp = *data_pages_per_tt.get(&tt).unwrap_or(&0);
        let active_dp = *active_per_tt.get(&tt).unwrap_or(&0);
        if ne == 0 && u7_pre == 0 && actual_dp == 0 {
            continue;
        }
        let fe = ru16(&bytes, off + 0x3a).unwrap_or(0);

        let mut stale = ne == 0 && active_dp > 0; // any transaction page but no index
        let mut entries: Vec<String> = Vec::new();
        for slot in 0..ne as usize {
            let entry_off = off + 0x3c + slot * 4;
            if let Some(v) = ru32(&bytes, entry_off) {
                if v == 0x1fff_fff8 {
                    entries.push(format!("slot{slot}=EMPTY"));
                } else {
                    let pi = (v / 8) as usize;
                    let poff = pi * page_size;
                    let valid = poff + page_size <= bytes.len() && {
                        let ptt = ru32(&bytes, poff + 8).unwrap_or(9999);
                        let pfl = ru8(&bytes, poff + 0x1b).unwrap_or(0);
                        ptt == tt && pfl != 0x64
                    };
                    if !valid {
                        stale = true;
                    }
                    entries.push(format!(
                        "slot{slot}=0x{v:04x}(p{pi}{})",
                        if valid { "" } else { "?INVALID" }
                    ));
                }
            }
        }
        let u7 = u7_pre;
        if stale {
            sentinel_btree_ok = false;
            println!(
                "  page[{i}] tt={tt} ne={ne} u7={u7} active_pages={active_dp} all_pages={actual_dp} fe=0x{fe:04x} \
                 entries=[{}]  ANOMALY: stale B-tree/write-pointer (DJ software rejects as corrupted)",
                entries.join(", ")
            );
        }
    }
    if sentinel_btree_ok {
        println!("  all OK");
    }
    println!();

    // ── sentinel page 0x2c field check ───────────────────────────────────
    println!("Sentinel 0x2c (redundant next_page) check:");
    let mut sentinel_ok_all = true;
    for i in 1..total_pages {
        let off = i * page_size;
        let idx = ru32(&bytes, off + 4).unwrap_or(0);
        if idx == 0 {
            continue;
        }
        let used_s = ru16(&bytes, off + 0x1e).unwrap_or(0);
        let pf = ru8(&bytes, off + 0x1b).unwrap_or(0);
        if used_s != 0 || pf != 0x64 {
            continue;
        } // only sentinel pages
        let tt = ru32(&bytes, off + 8).unwrap_or(0);
        let next_page = ru32(&bytes, off + 0x0c).unwrap_or(0);
        let dup_np = ru32(&bytes, off + 0x2c).unwrap_or(0);
        let is_magic = dup_np == 0x03ff_ffff;
        let has_data = next_page != idx.saturating_add(1) || {
            // heuristic: if next points to the immediately-following page and
            // that page has nrs>0, the table has data.
            let noff = next_page as usize * page_size;
            noff + 0x19 < bytes.len() && ru8(&bytes, noff + 0x18).unwrap_or(0) > 0
        };
        let bad = has_data && is_magic;
        if bad {
            sentinel_ok_all = false;
            println!(
                "  page[{i}] tt={tt} next_page={next_page} 0x2c={dup_np:#010x} \
                 ANOMALY: has-data sentinel still carries magic (should be {next_page})"
            );
        }
    }
    if sentinel_ok_all {
        println!("  all OK");
    }
    println!();

    // ── per-page footer rowpf/tranrf (all data pages) ────────────────────
    println!("Data page footer rowpf/tranrf (all tables):");
    for i in 1..total_pages {
        let off = i * page_size;
        let idx = ru32(&bytes, off + 4).unwrap_or(0);
        if idx == 0 {
            continue;
        }
        let used_s = ru16(&bytes, off + 0x1e).unwrap_or(0);
        if used_s == 0 {
            continue;
        }

        let tt = ru32(&bytes, off + 8).unwrap_or(0);
        let nrs = ru8(&bytes, off + 0x18).unwrap_or(0) as usize;
        let num_rl = ru16(&bytes, off + 0x22).unwrap_or(0) as usize;
        let row_slots = if num_rl == 8191 { nrs } else { nrs.max(num_rl) };
        if row_slots == 0 {
            continue;
        }

        let page = &bytes[off..off + page_size];
        let groups = row_slots.div_ceil(16);
        let mut cursor = page_size;
        let mut rowpf_groups = Vec::with_capacity(groups);
        let mut tranrf_groups = Vec::with_capacity(groups);
        let mut parse_ok = true;
        for g in 0..groups {
            if cursor < 4 {
                parse_ok = false;
                break;
            }
            cursor -= 4;
            rowpf_groups.push(ru16(page, cursor).unwrap_or(0));
            tranrf_groups.push(ru16(page, cursor + 2).unwrap_or(0));
            let glen = (row_slots - g * 16).min(16);
            if cursor < glen * 2 {
                parse_ok = false;
                break;
            }
            cursor -= glen * 2;
        }
        if !parse_ok {
            println!("  page[{i}] nrs={nrs} row_slots={row_slots}  footer parse failed");
            continue;
        }

        let rowpf_str: Vec<String> = rowpf_groups.iter().map(|v| format!("{v:#06x}")).collect();
        let tranrf_str: Vec<String> = tranrf_groups.iter().map(|v| format!("{v:#06x}")).collect();

        // Zero tranrf on normal data pages with active rows is a rejection
        // trigger. Working large exports can carry tranrf=0 on transaction
        // pages with sentinel-style u5/num_rl=8191, so do not flag those.
        let has_active = rowpf_groups.iter().any(|&v| v != 0);
        let tranrf_all_zero = tranrf_groups.iter().all(|&v| v == 0);
        let flags = ru8(page, 0x1b).unwrap_or(0);
        let u5 = ru16(page, 0x20).unwrap_or(0);
        let tranrf_zero_anomaly = has_active && tranrf_all_zero && !(flags == 0x34 && u5 == 0x1fff);

        let note = if tranrf_zero_anomaly {
            " ANOMALY: tranrf=0 with active rows — player rejects"
        } else {
            ""
        };
        println!(
            "  page[{i}] tt={tt} nrs={nrs} row_slots={row_slots}  \
             rowpf=[{}]  tranrf=[{}]{note}",
            rowpf_str.join(","),
            tranrf_str.join(","),
        );
    }

    // ── ec / data-page conflict check ────────────────────────────────────────
    // Detects cases where one table's empty_candidate pointer targets the same
    // physical page as another table's data. This happens when the additive
    // writer appends an overflow page at bytes.len()/page_size without checking
    // that next_unused has already reserved that slot for another table's ec.
    // DJ software rejects the resulting PDB as "database corrupted".
    println!();
    println!("ec / data-page conflict check:");
    // Build map: page_index -> table_type for all non-sentinel data pages.
    let mut data_page_owner: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for p in 1..total_pages {
        let off = p * page_size;
        let stored_idx = ru32(&bytes, off + 4).unwrap_or(0);
        if stored_idx == 0 {
            continue; // blank/unallocated page
        }
        let pf = ru8(&bytes, off + 0x1b).unwrap_or(0);
        if pf == 0x64 {
            continue; // sentinel page
        }
        let tt = ru32(&bytes, off + 8).unwrap_or(0);
        let used_s = ru16(&bytes, off + 0x1e).unwrap_or(0);
        if used_s == 0 {
            continue; // empty data page
        }
        data_page_owner.insert(stored_idx, tt);
    }
    let mut conflicts_found = false;
    for i in 0..num_tables {
        let toff = 0x1c + i * 16;
        let tt = ru32(&bytes, toff).unwrap_or(0);
        let ec = ru32(&bytes, toff + 4).unwrap_or(0);
        let fp = ru32(&bytes, toff + 8).unwrap_or(0);
        let lp = ru32(&bytes, toff + 12).unwrap_or(0);
        if fp == lp {
            continue; // empty table — ec points to blank baseline slot
        }
        if let Some(&owner_tt) = data_page_owner.get(&ec) {
            if owner_tt != tt {
                println!(
                    "  CONFLICT: tt={tt} ec={ec} points to data page owned by tt={owner_tt}  \
                     ANOMALY: DJ software rejects as corrupted"
                );
                conflicts_found = true;
            }
        }
    }
    if !conflicts_found {
        println!("  none");
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
