use backend::utils::{read_u16_le_at as read_u16_le, read_u32_le_at as read_u32_le};
use backend::{pdb_page, pdb_row};

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 2 {
        eprintln!("usage: cargo run --bin dump_page_rows -- <pdb_or_usb_root> <page_idx>");
        std::process::exit(2);
    }
    let path = &args[0];
    let page_idx: usize = args[1].parse().unwrap_or(0);
    let pdb_path = backend::pdb_reader::pdb_file_at(std::path::Path::new(path));
    let pdb = match std::fs::read(&pdb_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("failed to read {}: {e}", pdb_path.display());
            std::process::exit(1);
        }
    };

    let ps = 4096usize;
    let off = page_idx.saturating_mul(ps);
    let Some(page) = pdb.get(off..off + ps) else {
        eprintln!("page out of range");
        std::process::exit(1);
    };

    let tt = read_u32_le(page, 0x08).unwrap_or(0);
    let next = read_u32_le(page, 0x0c).unwrap_or(0);
    let seq = read_u32_le(page, 0x10).unwrap_or(0);
    let nrs = page[24];
    let u3 = page[25];
    let u5 = read_u16_le(page, 32).unwrap_or(0);
    let num_rl = read_u16_le(page, 34).unwrap_or(0);
    let rowpf0 = read_u16_le(page, ps - 4).unwrap_or(0);
    let tranrf0 = read_u16_le(page, ps - 2).unwrap_or(0);
    println!(
        "page={} tt={} next={} seq={} nrs={} u3={} u5={} num_rl={} rowpf0=0x{:04x} tranrf0=0x{:04x}",
        page_idx, tt, next, seq, nrs, u3, u5, num_rl, rowpf0, tranrf0
    );

    for slot in pdb_page::row_slots(page) {
        let (start, end) = (slot.start, slot.end);
        let row = &page[pdb_page::HEAP_START + start..pdb_page::HEAP_START + end];
        let len = row.len();
        let id = read_u32_le(row, 12).unwrap_or(0);
        let so = read_u32_le(row, 8).unwrap_or(0);
        let pid = read_u32_le(row, 0).unwrap_or(0);
        let folder = read_u32_le(row, 16).unwrap_or(0);
        let name = pdb_row::get_string_from_pdb(row, 20).unwrap_or_default();
        let s = slot.footer;
        let rowpf = s.group.rowpf(page);
        let tranrf = s.group.tranrf(page);
        println!(
            "row={} present={} len={} off=[{}..{}] bit=0x{:04x} rowpf=0x{:04x} tranrf=0x{:04x} id={} sort={} parent={} folder={} name={}",
            s.index, s.present, len, start, end, s.bit, rowpf, tranrf, id, so, pid, folder, name
        );
    }
}
