//! The check every PDB write passes before it reaches the USB.
//!
//! Whatever wrote it (a fresh export, an export onto a stick, a repair, a
//! metadata or cue edit, a playlist change), the PDB must stay one that
//! rekordbox and the players read. The shapes that break them are the ones
//! the repair detectors find, plus references and ids that must resolve. A
//! write may leave a problem the stick already had (the repairs handle
//! those), but it must not add one: [`pdb_write_problems`] lists what the
//! new PDB has that the old one didn't.

use super::*;

/// What `after` has wrong that `before` (the PDB as it was, `None` when the
/// stick had none) didn't, one line each. Empty when the write is fine.
pub fn pdb_write_problems(before: Option<&[u8]>, after: &[u8]) -> Vec<String> {
    let old: HashSet<String> = before
        .map(pdb_shape_problems)
        .unwrap_or_default()
        .into_iter()
        .collect();
    pdb_shape_problems(after)
        .into_iter()
        .filter(|problem| !old.contains(problem))
        .collect()
}

/// Everything in `bytes` that rekordbox or the players can't read, one line
/// each. A PDB rekordbox wrote has none, unless rekordbox crashed mid-write.
pub fn pdb_shape_problems(bytes: &[u8]) -> Vec<String> {
    let Ok(page_size) = pdb_page_size(bytes) else {
        return vec!["the file is too short for a PDB header".to_string()];
    };
    if page_size == 0 || bytes.len() < page_size * 2 {
        return vec!["the file has no data pages".to_string()];
    }
    let mut problems = Vec::new();
    let found = |kind: &str, finding: &dyn std::fmt::Debug| format!("{kind}: {finding:?}");
    for page in crate::pdb_reader::validate_pdb_page_conventions(bytes) {
        problems.push(found("page footer breaks the transaction rule", &page));
    }
    for page in detect_pdb_damaged_page_headers(bytes) {
        problems.push(found("damaged page header", &page));
    }
    for page in footers_into_rows(bytes) {
        problems.push(found("page footer runs into the rows", &page));
    }
    for slot in detect_pdb_string_misalignment(bytes) {
        problems.push(found(
            "track or album string not where players read it",
            &slot,
        ));
    }
    for chain in detect_pdb_ec_data_page_conflicts(bytes) {
        problems.push(found(
            "table's next free page is another table's data",
            &chain,
        ));
    }
    for chain in detect_pdb_truncated_table_chains(bytes) {
        problems.push(found("table chain ends past the file", &chain));
    }
    let torn = detect_pdb_torn_growth_pages(bytes);
    if !torn.garbage_ec_pages.is_empty() || torn.truncate_to_pages.is_some() {
        problems.push(found("half-written growth pages", &torn));
    }
    problems.extend(broken_table_pointers(bytes, page_size));
    let max_seq = crate::pdb_writer::max_seqpage_in_file(bytes, page_size);
    let seqdb = crate::utils::read_u32_le_at(bytes, 0x14).unwrap_or(0);
    if seqdb <= max_seq {
        problems.push(format!(
            "header seq {seqdb} isn't above the highest page seq {max_seq}"
        ));
    }
    for sentinel in detect_pdb_stale_sentinel_btree(bytes) {
        problems.push(found(
            "index page doesn't point at its table's pages",
            &sentinel,
        ));
    }
    match crate::pdb_reader::parse_pdb_bytes(bytes) {
        Ok(parsed) => problems.extend(unresolved_rows(&parsed)),
        Err(err) => problems.push(format!("the PDB can't be read: {}", err.english())),
    }
    problems
}

/// Tables whose chain doesn't run from its first page to its last, or whose
/// next free page isn't the one the last page points at. Every PDB
/// rekordbox writes has both.
fn broken_table_pointers(bytes: &[u8], page_size: usize) -> Vec<String> {
    use crate::utils::read_u32_le_at;
    let tables = read_u32_le_at(bytes, 8).unwrap_or(0) as usize;
    let mut problems = Vec::new();
    for i in 0..tables {
        let at = 0x1c + i * 16;
        let (Some(table), Some(free), Some(first), Some(last)) = (
            read_u32_le_at(bytes, at),
            read_u32_le_at(bytes, at + 4),
            read_u32_le_at(bytes, at + 8),
            read_u32_le_at(bytes, at + 12),
        ) else {
            problems.push(format!("table pointer {i} is past the header"));
            continue;
        };
        if crate::utils::collect_chain(bytes, page_size, first, last).is_none() {
            problems.push(format!(
                "table {table}: its pages don't link from page {first} to page {last}"
            ));
            continue;
        }
        let next = read_u32_le_at(bytes, last as usize * page_size + 0x0c).unwrap_or(0);
        if free != next {
            problems.push(format!(
                "table {table}: next free page {free}, but its last page {last} points at {next}"
            ));
        }
    }
    problems
}

/// Data pages whose footer, for the slot count in their header, overlaps
/// the row heap: the footer and the rows overwrite each other.
fn footers_into_rows(bytes: &[u8]) -> Vec<u32> {
    pdb_page::data_pages(bytes)
        .filter(|(_, page)| {
            pdb_page::read_slot_counts(page)
                .is_some_and(|(slots, _)| !pdb_page::footer_clear_of_heap(page, slots))
        })
        .map(|(index, _)| index)
        .collect()
}

/// Live rows that share an id, and references that name no row: a playlist
/// entry's track or playlist, and a track's dictionary rows. rekordbox lost
/// a stick's playlists when two live playlists had one id.
fn unresolved_rows(parsed: &ParsedPdb) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = HashSet::new();
    for track in &parsed.tracks {
        if !seen.insert(track.id) {
            problems.push(format!("two live track rows with id {}", track.id));
        }
    }
    let mut seen = HashSet::new();
    for row in &parsed.playlist_tree {
        if !seen.insert(row.id) {
            problems.push(format!("two live playlist rows with id {}", row.id));
        }
    }
    let track_ids: HashSet<u32> = parsed.tracks.iter().map(|t| t.id).collect();
    let playlist_ids: HashSet<u32> = parsed
        .playlist_tree
        .iter()
        .filter(|row| !row.is_folder)
        .map(|row| row.id)
        .collect();
    for entry in &parsed.playlist_entries {
        if !track_ids.contains(&entry.track_id) {
            problems.push(format!(
                "playlist {} lists track {}, which has no row",
                entry.playlist_id, entry.track_id
            ));
        }
        if !playlist_ids.contains(&entry.playlist_id) {
            problems.push(format!(
                "entry for track {} names playlist {}, which has no row",
                entry.track_id, entry.playlist_id
            ));
        }
    }
    let folder_ids: HashSet<u32> = parsed
        .playlist_tree
        .iter()
        .filter(|row| row.is_folder)
        .map(|row| row.id)
        .collect();
    for row in &parsed.playlist_tree {
        if row.parent_id != 0 && !folder_ids.contains(&row.parent_id) {
            problems.push(format!(
                "playlist {} sits in folder {}, which has no row",
                row.id, row.parent_id
            ));
        }
    }
    for track in &parsed.tracks {
        for (what, id, rows) in [
            ("artist", track.artist_id, &parsed.artists),
            ("album", track.album_id, &parsed.albums),
            ("artwork", track.artwork_id, &parsed.artworks),
            ("key", track.key_id, &parsed.keys),
            ("genre", track.genre_id, &parsed.genres),
            ("label", track.label_id, &parsed.labels),
            ("composer", track.composer_id, &parsed.artists),
            ("remixer", track.remixer_id, &parsed.artists),
        ] {
            if id != 0 && !rows.contains_key(&id) {
                problems.push(format!(
                    "track {} names {what} {id}, which has no row",
                    track.id
                ));
            }
        }
    }
    problems
}
