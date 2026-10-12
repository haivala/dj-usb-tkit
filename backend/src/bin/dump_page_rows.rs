use std::path::PathBuf;

use backend::pdb_page::{self, FooterSlot};
use backend::utils::{
    packed_page_row_slot_count, read_u16_le_at as read_u16_le, read_u32_le_at as read_u32_le,
};

/// Every footer slot of the page, or none when the footer doesn't fit.
fn page_footer_slots(page: &[u8], used_s: usize) -> Vec<FooterSlot> {
    let n_header = packed_page_row_slot_count(page).unwrap_or(0);
    if used_s == 0 || page.len() < 64 || !pdb_page::footer_fits(page.len(), n_header) {
        return Vec::new();
    }
    pdb_page::footer_slots(page, n_header).collect()
}

fn parse_pdb_string(row: &[u8], offset: usize) -> String {
    let Some(&n0) = row.get(offset) else {
        return String::new();
    };
    let n = n0 as usize;
    if n % 2 == 1 {
        let r = (n - 1) / 2;
        let end = (offset + r).min(row.len());
        let start = (offset + 1).min(end);
        return String::from_utf8_lossy(&row[start..end])
            .trim_end_matches('\0')
            .to_string();
    }

    let Some(len) = read_u16_le(row, offset + 1).map(|v| v as usize) else {
        return String::new();
    };
    let end = offset.saturating_add(len);
    if offset + 4 > row.len() || end > row.len() || end < offset + 4 {
        return String::new();
    }
    let data = &row[offset + 4..end];

    let code = n0;
    let b5 = (code & 0b0001_0000) != 0;
    let b8 = (code & 0b1000_0000) != 0;
    if b5 && b8 {
        let mut u16s = Vec::with_capacity(data.len() / 2);
        for c in data.as_chunks::<2>().0 {
            u16s.push(u16::from_le_bytes([c[0], c[1]]));
        }
        String::from_utf16_lossy(&u16s)
            .trim_end_matches('\0')
            .to_string()
    } else {
        // ascii and utf8 flag bits both decode the same way here.
        String::from_utf8_lossy(data)
            .trim_end_matches('\0')
            .to_string()
    }
}

fn read_pdb_bytes(path: &str) -> Result<Vec<u8>, String> {
    let p = PathBuf::from(path);
    let file = if p.is_dir() {
        p.join("PIONEER").join("rekordbox").join("export.pdb")
    } else {
        p
    };
    std::fs::read(&file).map_err(|e| format!("failed to read {}: {e}", file.display()))
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 2 {
        eprintln!("usage: cargo run --bin dump_page_rows -- <pdb_or_usb_root> <page_idx>");
        std::process::exit(2);
    }
    let path = &args[0];
    let page_idx: usize = args[1].parse().unwrap_or(0);
    let pdb = match read_pdb_bytes(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
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

    let used_s = read_u16_le(page, 30).unwrap_or(0) as usize;
    let payload_len = used_s.min(ps - pdb_page::HEAP_START);
    let payload = &page[pdb_page::HEAP_START..pdb_page::HEAP_START + payload_len];
    let slots = page_footer_slots(page, used_s);
    for (i, s) in slots.iter().enumerate() {
        let start = s.heap_off.min(payload_len);
        let end = slots
            .get(i + 1)
            .map_or(payload_len, |next| next.heap_off.min(payload_len));
        let len = end.saturating_sub(start);
        let (id, so, pid, folder, name) = if start < end {
            let row = &payload[start..end];
            let id = read_u32_le(row, 12).unwrap_or(0);
            let so = read_u32_le(row, 8).unwrap_or(0);
            let pid = read_u32_le(row, 0).unwrap_or(0);
            let folder = read_u32_le(row, 16).unwrap_or(0);
            let name = if row.len() >= 21 {
                parse_pdb_string(row, 20)
            } else {
                String::new()
            };
            (id, so, pid, folder, name)
        } else {
            (0, 0, 0, 0, String::new())
        };
        let rowpf = s.group.rowpf(page);
        let tranrf = s.group.tranrf(page);
        println!(
            "row={} present={} len={} off=[{}..{}] bit=0x{:04x} rowpf=0x{:04x} tranrf=0x{:04x} id={} sort={} parent={} folder={} name={}",
            s.index, s.present, len, start, end, s.bit, rowpf, tranrf, id, so, pid, folder, name
        );
    }
}
