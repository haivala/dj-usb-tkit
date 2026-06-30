use std::env;
use std::path::PathBuf;

use backend::pdb_reader::{parse_pdb, parse_pdb_track_debug_rows};

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("error: {}", message.as_ref());
    std::process::exit(1);
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 3 || args.len() > 4 {
        fail("usage: cargo run --bin dump_pdb_track_debug -- <export.pdb> <track_id> | --simple");
    }

    let pdb_path = PathBuf::from(&args[1]);

    if args[2] == "--simple" {
        let parsed = parse_pdb(&pdb_path)
            .unwrap_or_else(|err| fail(format!("failed to parse {}: {err}", pdb_path.display())));
        println!("track_id|content_link|master_content_id|master_db_id|track_file_path|anlz_path");
        for track in parsed.tracks {
            println!(
                "{}|{}|{}|{}|{}|{}",
                track.id,
                track
                    .content_link
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                track
                    .master_content_id
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                track
                    .master_db_id
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                track.track_file_path,
                track.anlz_path
            );
        }
        return;
    }

    let wanted_id = args[2]
        .parse::<u32>()
        .unwrap_or_else(|_| fail("track_id must be numeric or use --simple"));

    let rows = parse_pdb_track_debug_rows(&pdb_path)
        .unwrap_or_else(|err| fail(format!("failed to parse {}: {err}", pdb_path.display())));

    let row = rows
        .into_iter()
        .find(|row| row.id == wanted_id)
        .unwrap_or_else(|| fail(format!("track_id {wanted_id} not found")));

    println!("id={}", row.id);
    println!("row_len={}", row.row_len);
    println!("fixed_block_hex={}", row.fixed_block_hex);
    println!("fixed_fields:");
    let mut fixed = row.fixed_fields.into_iter().collect::<Vec<_>>();
    fixed.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in fixed {
        println!("  {k}={v}");
    }
    println!("string_slots:");
    for slot in row.string_slots {
        println!(
            "  idx={} label={} offset={} raw_hex={} decoded={}",
            slot.index,
            slot.label,
            slot.offset,
            slot.raw_hex,
            slot.decoded_value.unwrap_or_default()
        );
    }
}
