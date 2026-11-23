use backend::pdb_page;
use backend::utils::{collect_chain as collect_chain_pages, read_u32_le_at, table_ptr_fields};

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
        for row in pdb_page::present_rows(page) {
            let (Some(entry_index), Some(track_id), Some(playlist_id)) = (
                read_u32_le_at(&row, 0),
                read_u32_le_at(&row, 4),
                read_u32_le_at(&row, 8),
            ) else {
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
        eprintln!("usage: dump_t08_rows <pdb_or_usb_root>");
        std::process::exit(2);
    };
    let path = backend::pdb_reader::pdb_file_at(std::path::Path::new(&path_arg));
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
