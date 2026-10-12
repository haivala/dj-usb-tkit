use std::env;
use std::path::PathBuf;

use backend::pdb_page;
use backend::utils::{
    packed_page_row_slot_count, read_u16_le_at as read_u16_le, read_u32_le_at as read_u32_le,
};

const PAGE_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug)]
struct Slot {
    footer: pdb_page::FooterSlot,
    start: usize,
    end: usize,
}

fn parse_slots(page: &[u8]) -> Vec<Slot> {
    if page.len() < PAGE_SIZE {
        return Vec::new();
    }
    let used_s = read_u16_le(page, 30).unwrap_or(0) as usize;
    if used_s == 0 {
        return Vec::new();
    }
    let payload_len = used_s.min(PAGE_SIZE.saturating_sub(pdb_page::HEAP_START));
    let n_header = packed_page_row_slot_count(page).unwrap_or(0);
    let footer: Vec<_> = pdb_page::footer_slots(&page[..PAGE_SIZE], n_header).collect();

    let mut out = Vec::<Slot>::new();
    for (i, slot) in footer.iter().enumerate() {
        let start = slot.heap_off;
        let end = footer.get(i + 1).map_or(payload_len, |next| next.heap_off);
        if end <= start || end > payload_len {
            continue;
        }
        out.push(Slot {
            footer: *slot,
            start,
            end,
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
        let rowpf = s.footer.group.rowpf(page);
        let tranrf = s.footer.group.tranrf(page);
        println!(
            "row={} present={} len={} track_id={} rowpf=0x{:04x} tranrf=0x{:04x} bit=0x{:04x}",
            s.footer.index,
            s.footer.present,
            row.len(),
            track_id,
            rowpf,
            tranrf,
            s.footer.bit
        );
    }
}
