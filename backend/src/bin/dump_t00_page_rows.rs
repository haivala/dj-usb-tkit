use std::env;
use std::path::PathBuf;

use backend::utils::{read_u16_le_at as read_u16_le, read_u32_le_at as read_u32_le};

const PAGE_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug)]
struct Slot {
    row_index: usize,
    start: usize,
    end: usize,
    present: bool,
    bits_off: usize,
    bit_mask: u16,
}

fn parse_slots(page: &[u8]) -> Vec<Slot> {
    if page.len() < PAGE_SIZE {
        return Vec::new();
    }
    let used_s = read_u16_le(page, 30).unwrap_or(0) as usize;
    if used_s == 0 {
        return Vec::new();
    }
    let payload_len = used_s.min(PAGE_SIZE.saturating_sub(40));
    let nrs = page[24] as usize;
    let num_rl = read_u16_le(page, 34).unwrap_or(0) as usize;
    let n_header = if num_rl == 8191 { nrs } else { nrs.max(num_rl) };
    if n_header == 0 {
        return Vec::new();
    }

    let mut m = PAGE_SIZE;
    let mut row_offsets = Vec::<usize>::with_capacity(n_header);
    let mut row_bits = Vec::<(u16, u16, usize)>::with_capacity(n_header.div_ceil(16));
    for i in 0..n_header {
        if i % 16 == 0 {
            if m < 4 {
                break;
            }
            m -= 4;
            let rowpf = read_u16_le(page, m).unwrap_or(0);
            let tranrf = read_u16_le(page, m + 2).unwrap_or(0);
            row_bits.push((rowpf, tranrf, m));
        }
        if m < 2 {
            break;
        }
        m -= 2;
        row_offsets.push(read_u16_le(page, m).unwrap_or(0) as usize);
    }

    let mut out = Vec::<Slot>::new();
    for i in 0..row_offsets.len() {
        let start = row_offsets[i];
        let end = row_offsets.get(i + 1).copied().unwrap_or(payload_len);
        if end <= start || end > payload_len {
            continue;
        }
        let group_idx = i / 16;
        let bit = i % 16;
        let (rowpf, _tranrf, bits_off) = row_bits.get(group_idx).copied().unwrap_or((0, 0, 0));
        let bit_mask = 1u16 << bit;
        out.push(Slot {
            row_index: i,
            start,
            end,
            present: (rowpf & bit_mask) != 0,
            bits_off,
            bit_mask,
        });
    }
    out
}

fn parse_pdb_arg(arg: &str) -> PathBuf {
    let p = PathBuf::from(arg);
    if p.is_file() {
        p
    } else {
        p.join("PIONEER").join("rekordbox").join("export.pdb")
    }
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        eprintln!("usage: cargo run --bin dump_t00_page_rows -- <pdb_or_usb_root> <page_idx>");
        std::process::exit(2);
    }
    let pdb = parse_pdb_arg(&args[1]);
    let page_idx: usize = match args[2].parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("page_idx must be a non-negative integer");
            std::process::exit(2);
        }
    };
    let bytes = match std::fs::read(&pdb) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("failed to read {}: {err}", pdb.display());
            std::process::exit(1);
        }
    };
    let off = page_idx * PAGE_SIZE;
    if off + PAGE_SIZE > bytes.len() {
        eprintln!("page_idx out of range for {}", pdb.display());
        std::process::exit(1);
    }
    let page = &bytes[off..off + PAGE_SIZE];

    let nrs = page[24];
    let used_s = read_u16_le(page, 30).unwrap_or(0);
    let u5 = read_u16_le(page, 32).unwrap_or(0);
    let num_rl = read_u16_le(page, 34).unwrap_or(0);
    println!(
        "page={} nrs={} used_s={} u5={} num_rl={} rowpf0=0x{:04x} tranrf0=0x{:04x}",
        page_idx,
        nrs,
        used_s,
        u5,
        num_rl,
        read_u16_le(page, PAGE_SIZE - 4).unwrap_or(0),
        read_u16_le(page, PAGE_SIZE - 2).unwrap_or(0)
    );

    let payload_end = 40usize.saturating_add(used_s as usize).min(PAGE_SIZE);
    let payload = &page[40..payload_end];
    for s in parse_slots(page) {
        let row = &payload[s.start..s.end];
        let track_id = read_u32_le(row, 72).unwrap_or(0);
        let rowpf = read_u16_le(page, s.bits_off).unwrap_or(0);
        let tranrf = read_u16_le(page, s.bits_off + 2).unwrap_or(0);
        println!(
            "row={} present={} len={} track_id={} rowpf=0x{:04x} tranrf=0x{:04x} bit=0x{:04x}",
            s.row_index,
            s.present,
            row.len(),
            track_id,
            rowpf,
            tranrf,
            s.bit_mask
        );
    }
}
