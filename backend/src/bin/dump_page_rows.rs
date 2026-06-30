use std::path::PathBuf;

use backend::utils::{read_u16_le_at as read_u16_le, read_u32_le_at as read_u32_le};

#[derive(Clone, Copy)]
struct PageRowSlot {
    row_index: usize,
    start: usize,
    end: usize,
    present: bool,
    bits_off: usize,
    bit_mask: u16,
}

fn parse_page_row_slots(page: &[u8], len_page: usize) -> Vec<PageRowSlot> {
    if page.len() < len_page || len_page < 64 {
        return Vec::new();
    }
    let used_s = read_u16_le(page, 30).unwrap_or(0) as usize;
    if used_s == 0 {
        return Vec::new();
    }
    let payload_len = used_s.min(len_page.saturating_sub(40));
    let nrs = page[24] as usize;
    let num_rl = read_u16_le(page, 34).unwrap_or(0) as usize;
    let n_header = if num_rl == 8191 { nrs } else { nrs.max(num_rl) };
    if n_header == 0 {
        return Vec::new();
    }

    let mut m = len_page;
    let mut row_offsets = Vec::<usize>::with_capacity(n_header);
    let mut row_bits = Vec::<(u16, u16, usize)>::with_capacity(n_header.div_ceil(16));
    for i in 0..n_header {
        if i % 16 == 0 {
            if m < 4 {
                return Vec::new();
            }
            m -= 2;
            let tranrf = read_u16_le(page, m).unwrap_or(0);
            m -= 2;
            let rowpf = read_u16_le(page, m).unwrap_or(0);
            row_bits.push((rowpf, tranrf, m));
        }
        if m < 2 {
            return Vec::new();
        }
        m -= 2;
        let off = read_u16_le(page, m).unwrap_or(0) as usize;
        row_offsets.push(off.min(payload_len));
    }

    let mut slots = Vec::<PageRowSlot>::with_capacity(n_header);
    for i in 0..n_header {
        let start = row_offsets[i];
        let end = row_offsets.get(i + 1).copied().unwrap_or(payload_len);
        let group_idx = i / 16;
        let bit = (i % 16) as u16;
        let (rowpf, _tranrf, bits_off) = row_bits.get(group_idx).copied().unwrap_or((0, 0, 0));
        let bit_mask = 1u16 << bit;
        slots.push(PageRowSlot {
            row_index: i,
            start,
            end,
            present: (rowpf & bit_mask) != 0,
            bits_off,
            bit_mask,
        });
    }
    slots
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
    let b7 = (code & 0b0100_0000) != 0;
    let b6 = (code & 0b0010_0000) != 0;
    let b5 = (code & 0b0001_0000) != 0;
    let b8 = (code & 0b1000_0000) != 0;
    if b5 && b8 {
        let mut u16s = Vec::with_capacity(data.len() / 2);
        for c in data.chunks_exact(2) {
            u16s.push(u16::from_le_bytes([c[0], c[1]]));
        }
        String::from_utf16_lossy(&u16s)
            .trim_end_matches('\0')
            .to_string()
    } else if b6 {
        String::from_utf8_lossy(data)
            .trim_end_matches('\0')
            .to_string()
    } else if b7 {
        String::from_utf8_lossy(data)
            .trim_end_matches('\0')
            .to_string()
    } else {
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

    let slots = parse_page_row_slots(page, ps);
    let used_s = read_u16_le(page, 30).unwrap_or(0) as usize;
    let payload = &page[40..40 + used_s.min(ps - 40)];
    for s in slots {
        let len = s.end.saturating_sub(s.start);
        let (id, so, pid, folder, name) = if s.end <= payload.len() && s.start < s.end {
            let row = &payload[s.start..s.end];
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
        let rowpf = read_u16_le(page, s.bits_off).unwrap_or(0);
        let tranrf = read_u16_le(page, s.bits_off + 2).unwrap_or(0);
        println!(
            "row={} present={} len={} off=[{}..{}] bit=0x{:04x} rowpf=0x{:04x} tranrf=0x{:04x} id={} sort={} parent={} folder={} name={}",
            s.row_index,
            s.present,
            len,
            s.start,
            s.end,
            s.bit_mask,
            rowpf,
            tranrf,
            id,
            so,
            pid,
            folder,
            name
        );
    }
}
