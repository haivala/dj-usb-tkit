/// check_pdb_crossrefs <pdb_or_usb_root>
///
/// Check PDB internal cross-reference integrity:
///   - every active track row's artist_id, album_id, artwork_id, key_id resolves
///   - every playlist_entry's track_id and playlist_id resolves
///   - every playlist_tree row's parent_id resolves
///
/// Reports orphaned references that would be detected as database corruption.
use std::env;
use std::path::{Path, PathBuf};

use backend::pdb_reader::parse_pdb;

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: check_pdb_crossrefs <pdb_or_usb_root>");
        std::process::exit(2);
    }
    let input = Path::new(&args[1]);
    let pdb_path = resolve_pdb(input);

    let parsed = match parse_pdb(&pdb_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("parse error: {e}");
            std::process::exit(1);
        }
    };

    println!("pdb: {}", pdb_path.display());
    println!(
        "tracks={} artists={} albums={} artworks={} keys={} \
         playlist_tree={} playlist_entries={}",
        parsed.tracks.len(),
        parsed.artists.len(),
        parsed.albums.len(),
        parsed.artworks.len(),
        parsed.keys.len(),
        parsed.playlist_tree.len(),
        parsed.playlist_entries.len(),
    );
    println!();

    let mut errors = 0u32;

    // Track cross-refs
    let track_ids: std::collections::HashSet<u32> = parsed.tracks.iter().map(|t| t.id).collect();

    println!("=== Track → dict cross-refs ===");
    for t in &parsed.tracks {
        if t.artist_id != 0 && !parsed.artists.contains_key(&t.artist_id) {
            println!(
                "  ORPHAN track_id={} artist_id={} — no artist row with this id",
                t.id, t.artist_id
            );
            errors += 1;
        }
        if t.album_id != 0 && !parsed.albums.contains_key(&t.album_id) {
            println!(
                "  ORPHAN track_id={} album_id={} — no album row with this id",
                t.id, t.album_id
            );
            errors += 1;
        }
        if t.artwork_id != 0 && !parsed.artworks.contains_key(&t.artwork_id) {
            println!(
                "  ORPHAN track_id={} artwork_id={} — no artwork row with this id",
                t.id, t.artwork_id
            );
            errors += 1;
        }
        if t.key_id != 0 && !parsed.keys.contains_key(&t.key_id) {
            println!(
                "  ORPHAN track_id={} key_id={} — no key row with this id",
                t.id, t.key_id
            );
            errors += 1;
        }
    }
    if errors == 0 {
        println!("  all OK");
    }
    println!();

    // Playlist entry cross-refs
    println!("=== Playlist entry cross-refs ===");
    let playlist_tree_ids: std::collections::HashSet<u32> =
        parsed.playlist_tree.iter().map(|p| p.id).collect();
    let mut entry_errors = 0u32;
    for e in &parsed.playlist_entries {
        if !track_ids.contains(&e.track_id) {
            println!(
                "  ORPHAN entry playlist_id={} entry_index={} track_id={} — no track row",
                e.playlist_id, e.entry_index, e.track_id
            );
            entry_errors += 1;
            errors += 1;
        }
        if !playlist_tree_ids.contains(&e.playlist_id) {
            println!(
                "  ORPHAN entry playlist_id={} entry_index={} — no playlist_tree row",
                e.playlist_id, e.entry_index
            );
            entry_errors += 1;
            errors += 1;
        }
    }
    if entry_errors == 0 {
        println!("  all OK");
    }
    println!();

    // Playlist tree parent refs
    println!("=== Playlist tree parent cross-refs ===");
    let mut tree_errors = 0u32;
    for p in &parsed.playlist_tree {
        if p.parent_id != 0 && !playlist_tree_ids.contains(&p.parent_id) {
            println!(
                "  ORPHAN playlist_tree id={} parent_id={} — no parent row",
                p.id, p.parent_id
            );
            tree_errors += 1;
            errors += 1;
        }
    }
    if tree_errors == 0 {
        println!("  all OK");
    }
    println!();

    // Duplicate track IDs
    println!("=== Duplicate track IDs ===");
    let mut seen: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for t in &parsed.tracks {
        *seen.entry(t.id).or_insert(0) += 1;
    }
    let mut dup_errors = 0u32;
    for (id, count) in &seen {
        if *count > 1 {
            println!("  DUPLICATE track_id={} appears {} times", id, count);
            dup_errors += 1;
            errors += 1;
        }
    }
    if dup_errors == 0 {
        println!("  all OK");
    }
    println!();

    if errors == 0 {
        println!("result: PASS — no cross-reference issues found");
    } else {
        println!("result: FAIL — {errors} cross-reference issue(s) found");
        std::process::exit(1);
    }
}

fn resolve_pdb(input: &Path) -> PathBuf {
    if input
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case("export.pdb"))
        .unwrap_or(false)
    {
        return input.to_path_buf();
    }
    input.join("PIONEER").join("rekordbox").join("export.pdb")
}
