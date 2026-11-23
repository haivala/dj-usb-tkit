use std::env;

use backend::pdb_page;
use backend::utils::{read_u16_le_at as read_u16_le, read_u32_le_at as read_u32_le};

const PAGE_SIZE: usize = 4096;

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        eprintln!("usage: cargo run --bin dump_t00_page_rows -- <pdb_or_usb_root> <page_idx>");
        std::process::exit(2);
    }
    let pdb = backend::pdb_reader::pdb_file_at(std::path::Path::new(&args[1]));
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

    for s in pdb_page::row_slots(page) {
        let row = &page[s.range()];
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
