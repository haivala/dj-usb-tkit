use std::path::PathBuf;

use backend::pdb_page;
use backend::utils::{
    collect_chain as collect_chain_pages, packed_page_row_slot_count, read_u16_le_at,
    read_u32_le_at, table_ptr_fields,
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
        let n_header = packed_page_row_slot_count(page).unwrap_or(0);
        let footer: Vec<_> = pdb_page::footer_slots(page, n_header).collect();
        for (i, slot) in footer.iter().enumerate() {
            if !slot.present {
                continue;
            }
            let start = slot.heap_off;
            let end = footer
                .get(i + 1)
                .map_or(payload.len(), |next| next.heap_off);
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
