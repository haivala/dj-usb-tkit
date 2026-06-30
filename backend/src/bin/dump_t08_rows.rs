use std::path::PathBuf;

use backend::utils::{
    collect_chain as collect_chain_pages, read_u16_le_at, read_u32_le_at, table_ptr_fields,
};

fn parse_t08_rows_with_pages(pdb: &[u8]) -> Vec<(u32, u32, u32, u32)> {
    let page_size = 4096usize;
    let Some((_ec, first, last)) = table_ptr_fields(pdb, 8) else {
        return Vec::new();
    };
    let chain = collect_chain_pages(pdb, page_size, first, last).unwrap_or_default();
    let mut out = Vec::<(u32, u32, u32, u32)>::new();
    for page_idx in chain.into_iter().skip(1) {
        let off = page_idx as usize * page_size;
        let Some(page) = pdb.get(off..off + page_size) else {
            continue;
        };
        let used_s = read_u16_le_at(page, 30).unwrap_or(0) as usize;
        if used_s == 0 {
            continue;
        }
        let payload_start = 40usize;
        let payload_end = payload_start.saturating_add(used_s).min(page_size);
        if payload_end <= payload_start {
            continue;
        }
        let payload = &page[payload_start..payload_end];
        let nrs = page[24] as usize;
        let num_rl = read_u16_le_at(page, 34).unwrap_or(0) as usize;
        let n_header = if num_rl == 8191 { nrs } else { nrs.max(num_rl) };
        if n_header == 0 {
            continue;
        }
        let mut m = page_size;
        let mut row_offsets = Vec::<usize>::with_capacity(n_header + 1);
        let mut row_presence = Vec::<u16>::with_capacity((n_header / 16) + 1);
        for i in 0..n_header {
            if i % 16 == 0 {
                if m < 4 {
                    break;
                }
                m -= 4;
                let Some(bits) = read_u16_le_at(page, m) else {
                    break;
                };
                row_presence.push(bits);
            }
            if m < 2 {
                break;
            }
            m -= 2;
            let Some(off) = read_u16_le_at(page, m) else {
                break;
            };
            row_offsets.push(off as usize);
        }
        for i in 0..row_offsets.len() {
            let group_idx = i / 16;
            let bit = i % 16;
            let present = row_presence
                .get(group_idx)
                .map(|bits| ((bits >> bit) & 1) == 1)
                .unwrap_or(false);
            if !present {
                continue;
            }
            let start = row_offsets[i];
            let end = row_offsets.get(i + 1).copied().unwrap_or(payload.len());
            if end <= start || start + 12 > payload.len() || end > payload.len() {
                continue;
            }
            let row = &payload[start..start + 12];
            let Some(entry_index) = read_u32_le_at(row, 0) else {
                continue;
            };
            let Some(track_id) = read_u32_le_at(row, 4) else {
                continue;
            };
            let Some(playlist_id) = read_u32_le_at(row, 8) else {
                continue;
            };
            out.push((page_idx, playlist_id, entry_index, track_id));
        }
    }
    out.sort_by_key(|(page, playlist, entry, _track)| (*playlist, *entry, *page));
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path_arg) = args.next() else {
        eprintln!("usage: dump_t08_rows <export.pdb>");
        std::process::exit(2);
    };
    let path = PathBuf::from(path_arg);
    let bytes = match std::fs::read(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("read failed: {e}");
            std::process::exit(1);
        }
    };
    println!("pdb={}", path.display());
    for (page, playlist_id, entry_index, track_id) in parse_t08_rows_with_pages(&bytes) {
        println!(
            "row|page={}|playlist_id={}|entry_index={}|track_id={}",
            page, playlist_id, entry_index, track_id
        );
    }
}
