//! Writing the export PDB: full rewrites, previews, playlist rewrites and the
//! byte-level checks that guard them.

use super::*;

/// Read PDB t16 row bytes from an existing PDB on disk, in their on-disk
/// order. Returns `None` when the input bytes are not a valid PDB image or
/// contain no t16 rows; callers should then fall back to the default seed.
pub(super) fn preserve_existing_t16_rows(bytes: &[u8]) -> Option<Vec<Vec<u8>>> {
    load_pdb_t16_rows_from_bytes(bytes)
}

/// Verify that `table_type`'s chain underwent, at most, a pure tail append
/// between `before` and `after`: new pages chained after the existing last
/// page, with every pre-existing page remaining at its same position in the
/// chain. Content on existing pages may legitimately change (e.g. sibling
/// sort_order patches, or a table nothing in this codebase writes to but
/// that reference exporters/hardware populate independently, such as legacy
/// history fallback tables) — only reordering, relocation, or a broken chain
/// is rejected. Pushes a description onto `issues` when the check fails;
/// does nothing when either side is missing a table pointer.
pub(super) fn validate_table_pure_append_or_unchanged(
    before: &[u8],
    after: &[u8],
    table_type: u32,
    page_size: usize,
    issues: &mut Vec<String>,
) {
    let (
        Some((_ec_before, before_first, before_last)),
        Some((_ec_after, _first_after, after_last)),
    ) = (
        table_ptr_fields(before, table_type),
        table_ptr_fields(after, table_type),
    )
    else {
        return;
    };

    if before_last == after_last {
        // Tail didn't move: its next_page must not have changed either, so
        // nothing was silently appended past a stale tail pointer.
        if let Some(last_off) = page_offset(before_last, page_size) {
            let before_next = before
                .get(last_off + 0x0c..last_off + 0x10)
                .and_then(|b| b.try_into().ok())
                .map(u32::from_le_bytes);
            let after_next = after
                .get(last_off + 0x0c..last_off + 0x10)
                .and_then(|b| b.try_into().ok())
                .map(u32::from_le_bytes);
            if before_next != after_next {
                issues.push(format!(
                    "t{table_type:02} tail next_page changed {:?}->{:?}",
                    before_next, after_next
                ));
            }
        }
    } else {
        // Tail moved: only acceptable as a pure append. The old chain's
        // page-index sequence must be an exact, unmoved prefix of the new
        // chain — content on those pages may legitimately change, but their
        // identity and order in the chain may not.
        let is_pure_append = match (
            collect_chain_pages(before, page_size, before_first, before_last),
            collect_chain_pages(after, page_size, before_first, after_last),
        ) {
            (Some(old_chain), Some(new_chain)) => {
                new_chain.len() > old_chain.len() && new_chain[..old_chain.len()] == old_chain[..]
            }
            _ => false,
        };
        if !is_pure_append {
            issues.push(format!(
                "t{table_type:02} last_page changed {before_last}->{after_last} (not a pure tail append)"
            ));
        }
    }
}

pub(super) fn validate_topology_locked_export_bytes(
    before: &[u8],
    after: &[u8],
) -> BackendResult<()> {
    const PAGE_SIZE: usize = 4096;
    const CRITICAL_FIRST_PAGE_TABLES: &[u32] = &[0, 7, 8, 11, 12, 16, 17, 18, 19];
    const MENU_TABLES: &[u32] = &[16, 17, 18];
    // Tables nothing in this codebase writes rows into. Reference exporters
    // and/or hardware are known to populate some of these independently
    // (e.g. legacy history fallback tables) as they're used over time, so a
    // tail-append is tolerated the same way t07 growth is — but their chain
    // must never be reordered or relocated, which previously went completely
    // unchecked (only first_page was verified).
    const UNWRITTEN_APPEND_ONLY_TABLES: &[u32] = &[9, 10, 11, 12, 14, 15];

    if before.len() < PAGE_SIZE
        || after.len() < PAGE_SIZE
        || !before.len().is_multiple_of(PAGE_SIZE)
        || !after.len().is_multiple_of(PAGE_SIZE)
    {
        return Err(BackendError::Validation(
            "PDB export blocked: non page-aligned PDB candidate".into(),
        ));
    }

    let mut issues = Vec::<String>::new();
    for &table_type in CRITICAL_FIRST_PAGE_TABLES {
        if let (
            Some((_before_ec, before_first, _before_last)),
            Some((_after_ec, after_first, _after_last)),
        ) = (
            table_ptr_fields(before, table_type),
            table_ptr_fields(after, table_type),
        ) && before_first != after_first
        {
            issues.push(format!(
                "t{table_type:02} first_page changed {before_first}->{after_first}"
            ));
        }
    }

    // t07 (playlist_tree) growth: this is how reference exporters themselves
    // grow playlist_tree once the existing tail page runs out of room, and
    // multi-page t07 chains are known to work on hardware. Blanket-rejecting
    // any last_page move made it impossible to add a new playlist to a USB
    // once t07 outgrew its initial page.
    validate_table_pure_append_or_unchanged(before, after, 7, PAGE_SIZE, &mut issues);

    for &table_type in UNWRITTEN_APPEND_ONLY_TABLES {
        validate_table_pure_append_or_unchanged(before, after, table_type, PAGE_SIZE, &mut issues);
    }

    for &table_type in MENU_TABLES {
        let Some(before_ptr) = table_ptr_fields(before, table_type) else {
            continue;
        };
        let Some(after_ptr) = table_ptr_fields(after, table_type) else {
            issues.push(format!("t{table_type:02} menu table disappeared"));
            continue;
        };
        if before_ptr != after_ptr {
            issues.push(format!(
                "t{table_type:02} menu pointer changed {:?}->{:?}",
                before_ptr, after_ptr
            ));
            continue;
        }
        let (_ec, first, last) = before_ptr;
        let Some(chain) = collect_chain_pages(before, PAGE_SIZE, first, last) else {
            issues.push(format!("t{table_type:02} before menu chain is invalid"));
            continue;
        };
        for page_idx in chain {
            let Some(off) = page_offset(page_idx, PAGE_SIZE) else {
                issues.push(format!(
                    "t{table_type:02} page {page_idx} has invalid offset"
                ));
                continue;
            };
            let Some(before_page) = before.get(off..off + PAGE_SIZE) else {
                issues.push(format!("t{table_type:02} page {page_idx} missing before"));
                continue;
            };
            let Some(after_page) = after.get(off..off + PAGE_SIZE) else {
                issues.push(format!("t{table_type:02} page {page_idx} missing after"));
                continue;
            };
            if before_page != after_page {
                issues.push(format!("t{table_type:02} menu page {page_idx} changed"));
            }
        }
    }

    match crate::pdb_reader::parse_pdb_bytes(after) {
        Ok(parsed) => {
            let mut seen = HashSet::<u32>::new();
            for row in parsed.playlist_tree.iter().filter(|row| !row.row_is_folder) {
                if !seen.insert(row.id) {
                    issues.push(format!("duplicate active playlist_tree id {}", row.id));
                }
            }
        }
        Err(err) => issues.push(format!("candidate PDB no longer parses: {err}")),
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(BackendError::Validation(format!(
            "PDB export blocked: topology-locked additive guard failed ({})",
            issues.join(" | ")
        )))
    }
}

/// Write PDB export data.
///
/// Existing PDBs are written through the topology-locked additive path only.
/// A full rebuild is allowed only when there is no PDB on disk yet; falling
/// back to a rebuilt blob for a populated USB is a known player database corruption class.
/// The eDB's `property` row as the PDB's t19 rows mirror it (see
/// [`crate::pdb_writer::T19Property`]), or `None` when the USB has no
/// readable eDB.
pub(crate) fn edb_t19_property(usb_root: &Path) -> Option<crate::pdb_writer::T19Property> {
    let mut warnings = Vec::<WarningEntry>::new();
    let conn = open_edb_rw(usb_root, &mut warnings)?;
    conn.query_row(
        "SELECT COALESCE(deviceName, ''), CAST(dbVersion AS TEXT), createdDate, \
         COALESCE(backGroundColorType, 0) FROM property LIMIT 1",
        [],
        |row| {
            let name: String = row.get(0)?;
            Ok(crate::pdb_writer::T19Property {
                created_date: row.get(2)?,
                db_version: row.get(1)?,
                device_name: (!name.is_empty()).then_some(name),
                background_color: u8::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
            })
        },
    )
    .ok()
}

pub fn write_pdb(
    usb_root: &Path,
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
    mirror_playlist_entries: bool,
    edb_playlist_id: Option<u32>,
    edb_sort_order: Option<u32>,
) -> BackendResult<WriteExportPdbResult> {
    write_pdb_fresh_with_overrides(
        usb_root,
        playlist,
        manifest,
        mirror_playlist_entries,
        edb_playlist_id,
        edb_sort_order,
        true,
    )
}

pub fn preview_pdb(
    usb_root: &Path,
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
    mirror_playlist_entries: bool,
    edb_playlist_id: Option<u32>,
    edb_sort_order: Option<u32>,
) -> BackendResult<WriteExportPdbResult> {
    write_pdb_fresh_with_overrides(
        usb_root,
        playlist,
        manifest,
        mirror_playlist_entries,
        edb_playlist_id,
        edb_sort_order,
        false,
    )
}

pub(super) fn normalize_pdb_key_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(super) fn move_playlist_tree_row_to_front(
    rows: &mut [crate::pdb_writer::PdbPlaylistTreeRow],
    target_id: u32,
) {
    let Some(target_idx) = rows.iter().position(|row| row.id == target_id) else {
        return;
    };
    let parent_id = rows[target_idx].parent_id;
    let mut siblings: Vec<(usize, u32, u32)> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.parent_id == parent_id)
        .map(|(idx, row)| (idx, row.sort_order, row.id))
        .collect();
    siblings.sort_by_key(|(_, sort_order, id)| (*sort_order, *id));

    let mut ordered = Vec::<usize>::with_capacity(siblings.len());
    ordered.push(target_idx);
    ordered.extend(
        siblings
            .into_iter()
            .map(|(idx, _, _)| idx)
            .filter(|idx| *idx != target_idx),
    );
    for (sort_order, idx) in ordered.into_iter().enumerate() {
        rows[idx].sort_order = sort_order as u32;
    }
}

pub(super) fn parse_key_pitch_mode(value: &str) -> Option<(u8, bool)> {
    let mut s = value.trim().to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }
    let is_minor = if s.ends_with("minor") {
        s.truncate(s.len().saturating_sub(5));
        true
    } else if s.ends_with("min") {
        s.truncate(s.len().saturating_sub(3));
        true
    } else if s.ends_with('m') {
        s.truncate(s.len().saturating_sub(1));
        true
    } else if s.ends_with("major") {
        s.truncate(s.len().saturating_sub(5));
        false
    } else if s.ends_with("maj") {
        s.truncate(s.len().saturating_sub(3));
        false
    } else {
        false
    };
    let root = s.trim();
    let pitch = match root {
        "c" => 0,
        "c#" | "db" => 1,
        "d" => 2,
        "d#" | "eb" => 3,
        "e" => 4,
        "f" => 5,
        "f#" | "gb" => 6,
        "g" => 7,
        "g#" | "ab" => 8,
        "a" => 9,
        "a#" | "bb" => 10,
        "b" => 11,
        _ => return None,
    };
    Some((pitch, is_minor))
}

pub(super) fn key_lookup_variants(value: &str) -> Vec<String> {
    let exact = normalize_pdb_key_name(value);
    if let Some((pitch, minor)) = parse_key_pitch_mode(value) {
        let (sharp, flat) = match pitch {
            0 => ("c", "c"),
            1 => ("c#", "db"),
            2 => ("d", "d"),
            3 => ("d#", "eb"),
            4 => ("e", "e"),
            5 => ("f", "f"),
            6 => ("f#", "gb"),
            7 => ("g", "g"),
            8 => ("g#", "ab"),
            9 => ("a", "a"),
            10 => ("a#", "bb"),
            11 => ("b", "b"),
            _ => ("", ""),
        };
        let suffix = if minor { "m" } else { "" };
        let mut out = vec![exact];
        out.push(format!("{sharp}{suffix}"));
        out.push(format!("{flat}{suffix}"));
        out.sort();
        out.dedup();
        out
    } else {
        vec![exact]
    }
}

/// Whether two key names denote the same key, by the same rule export uses
/// to reuse a PDB key row (case-insensitive, sharp/flat spellings equal).
pub(crate) fn key_names_match(a: &str, b: &str) -> bool {
    let b_variants = key_lookup_variants(b);
    key_lookup_variants(a)
        .iter()
        .any(|v| !v.is_empty() && b_variants.contains(v))
}

pub(super) fn resolve_key_id_by_name(
    key_id_by_name: &HashMap<String, u32>,
    key_name: &str,
) -> Option<u32> {
    key_lookup_variants(key_name)
        .into_iter()
        .find_map(|k| key_id_by_name.get(&k).copied())
}

pub(super) fn write_pdb_fresh_with_overrides(
    usb_root: &Path,
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
    mirror_playlist_entries: bool,
    override_playlist_id: Option<u32>,
    override_sort_order: Option<u32>,
    commit_write: bool,
) -> BackendResult<WriteExportPdbResult> {
    use crate::pdb_writer::{
        PdbAlbumRow, PdbArtistRow, PdbArtworkRow, PdbData, PdbKeyRow, PdbPlaylistEntryRow,
        PdbPlaylistTreeRow,
    };

    let pdb_path = crate::service::usb_staging::stage_pdb(usb_root)?;

    if std::env::var("PDB_WRITE_MODE")
        .ok()
        .map(|v| v.eq_ignore_ascii_case("skip"))
        .unwrap_or(false)
    {
        return Ok(WriteExportPdbResult {
            inserted_tracks: 0,
            inserted_playlists: 0,
            topology_issues: Vec::new(),
            writer_warnings: Vec::new(),
        });
    }

    let profile = PdbLayoutProfile::from_env();
    let export_date_added = normalize_export_date(Some(&manifest.generated_at))
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());

    // ── Load existing PDB data (for additive mode) ──────────────────────
    let existing = if pdb_path.is_file() {
        Some(parse_pdb(&pdb_path)?)
    } else {
        None
    };
    let existing_header_flags_by_track_id = if pdb_path.is_file() {
        crate::pdb_reader::parse_pdb_track_debug_rows(&pdb_path)
            .ok()
            .map(|rows| {
                rows.into_iter()
                    .filter_map(|r| {
                        r.fixed_fields
                            .get("header_flags_u32")
                            .and_then(|v| v.parse::<u32>().ok())
                            .map(|flags| (r.id, flags))
                    })
                    .collect::<HashMap<u32, u32>>()
            })
            .unwrap_or_default()
    } else {
        HashMap::new()
    };
    let existing_pdb_bytes_before = if pdb_path.is_file() {
        std::fs::read(&pdb_path).ok()
    } else {
        None
    };

    // ── Dictionary dedup maps ───────────────────────────────────────────
    let mut artist_id_by_name = HashMap::<String, u32>::new();
    let mut album_id_by_name = HashMap::<String, u32>::new();
    let mut key_id_by_name = HashMap::<String, u32>::new();
    let mut genre_id_by_name = HashMap::<String, u32>::new();
    let mut artwork_id_by_path = HashMap::<String, u32>::new();
    let mut track_id_by_path = HashMap::<String, u32>::new();
    let mut track_index_by_id = HashMap::<u32, usize>::new();

    let mut all_artists = Vec::<PdbArtistRow>::new();
    let mut all_albums = Vec::<PdbAlbumRow>::new();
    let mut all_keys = Vec::<PdbKeyRow>::new();
    let mut all_genres = Vec::<crate::pdb_writer::PdbDictRow>::new();
    let mut all_artwork = Vec::<PdbArtworkRow>::new();
    let mut all_tracks = Vec::<PdbTrackRowData>::new();
    let mut all_playlist_tree = Vec::<PdbPlaylistTreeRow>::new();
    let mut all_playlist_entries = Vec::<PdbPlaylistEntryRow>::new();

    let mut next_artist_id = 1u32;
    let mut next_album_id = 1u32;
    let mut next_key_id = 1u32;
    let mut next_genre_id = 1u32;
    let mut next_artwork_id = 1u32;
    let mut next_track_id = 1u32;
    let mut next_playlist_id = 1u32;
    let mut next_sort_order = 1u32;
    let mut next_entry_index = 1u32;
    let mut existing_playlist_pdb_id: Option<u32> = None;
    let mut desired_manifest_track_rows = HashMap::<u32, PdbTrackRowData>::new();

    // ── Seed from existing PDB ──────────────────────────────────────────
    if let Some(ref parsed) = existing {
        for (id, name) in &parsed.artists {
            let key = canonicalize_playlist_name(name);
            artist_id_by_name.insert(key, *id);
            all_artists.push(PdbArtistRow {
                id: *id,
                name: name.clone(),
            });
            next_artist_id = next_artist_id.max(id + 1);
        }
        for (id, name) in &parsed.albums {
            let key = canonicalize_playlist_name(name);
            album_id_by_name.insert(key, *id);
            all_albums.push(PdbAlbumRow {
                id: *id,
                name: name.clone(),
                artist_id: 0, // parser doesn't store album artist_id
            });
            next_album_id = next_album_id.max(id + 1);
        }
        for (id, name) in &parsed.keys {
            let key = normalize_pdb_key_name(name);
            key_id_by_name.insert(key, *id);
            all_keys.push(PdbKeyRow {
                id: *id,
                name: name.clone(),
            });
            next_key_id = next_key_id.max(id + 1);
        }
        for (id, name) in &parsed.genres {
            let key = canonicalize_playlist_name(name);
            genre_id_by_name.insert(key, *id);
            all_genres.push(crate::pdb_writer::PdbDictRow {
                id: *id,
                name: name.clone(),
            });
            next_genre_id = next_genre_id.max(id + 1);
        }
        for (id, path) in &parsed.artworks {
            let key = canonicalize_artwork_path_lookup(path);
            artwork_id_by_path.insert(key, *id);
            all_artwork.push(PdbArtworkRow {
                id: *id,
                path: path.clone(),
            });
            next_artwork_id = next_artwork_id.max(id + 1);
        }
        for track in &parsed.tracks {
            let path_key = canonicalize_track_path_identity(&track.track_file_path);
            if !path_key.is_empty() {
                track_id_by_path.insert(path_key, track.id);
            }
            track_index_by_id.insert(track.id, all_tracks.len());
            all_tracks.push(PdbTrackRowData {
                header_flags_u32: existing_header_flags_by_track_id.get(&track.id).copied(),
                content_link: track.content_link,
                sample_rate_hz: track.sample_rate_hz,
                file_size_bytes: track.file_size_bytes,
                master_content_id: track.master_content_id,
                master_db_id: track.master_db_id,
                id: track.id,
                artist_id: track.artist_id,
                album_id: track.album_id,
                artwork_id: track.artwork_id,
                key_id: track.key_id,
                genre_id: track.genre_id,
                bitrate_kbps: track.bitrate_kbps,
                track_number: Some(track.track_number),
                bpm: track.tempo_x100.checked_sub(0).map(|t| t as f64 / 100.0),
                release_year: track.release_year,
                bit_depth: track.bit_depth,
                duration_seconds: track.duration_seconds,
                file_type: track.file_type,
                isrc: track.isrc.clone(),
                date_added: track.date_added.clone(),
                release_date: track.release_date.clone(),
                dj_comment: track.dj_comment.clone(),
                file_name: track.file_name.clone(),
                publish_track_info_on: track
                    .publish_track_info
                    .as_deref()
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("on")),
                autoload_hotcues_on: track
                    .autoload_hotcues
                    .as_deref()
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("on")),
                color_id: Some(track.color_id),
                rating: Some(track.rating),
                title: track.title.clone(),
                anlz_path: track.anlz_path.clone(),
                file_path: track.track_file_path.clone(),
            });
            next_artist_id = next_artist_id.max(track.artist_id.saturating_add(1));
            next_album_id = next_album_id.max(track.album_id.saturating_add(1));
            next_key_id = next_key_id.max(track.key_id.saturating_add(1));
            next_genre_id = next_genre_id.max(track.genre_id.saturating_add(1));
            next_artwork_id = next_artwork_id.max(track.artwork_id.saturating_add(1));
            next_track_id = next_track_id.max(track.id + 1);
        }

        // Seed playlist tree preserving legacy rows as-is. Some real exports
        // carry duplicate names/IDs in ways that should not be normalized
        // during additive export writes.
        let same_name_playlists: Vec<_> = parsed
            .playlist_tree
            .iter()
            .filter(|p| !p.row_is_folder && p.name == playlist.name)
            .collect();
        let existing_pl = same_name_playlists
            .iter()
            .min_by_key(|p| p.sort_order)
            .copied();
        existing_playlist_pdb_id = existing_pl.map(|p| p.id);
        let same_name_ids: HashSet<u32> = same_name_playlists.iter().map(|p| p.id).collect();
        for row in &parsed.playlist_tree {
            all_playlist_tree.push(PdbPlaylistTreeRow {
                id: row.id,
                parent_id: row.parent_id,
                sort_order: row.sort_order,
                is_folder: row.row_is_folder,
                name: row.name.clone(),
            });
            next_playlist_id = next_playlist_id.max(row.id + 1);
            next_sort_order = next_sort_order.max(row.sort_order + 1);
        }

        // Seed playlist entries preserving existing rows.
        let playlist_pdb_id = existing_playlist_pdb_id;
        for entry in &parsed.playlist_entries {
            if mirror_playlist_entries && same_name_ids.contains(&entry.playlist_id) {
                continue; // will be rebuilt from manifest
            }
            if !mirror_playlist_entries && playlist_pdb_id == Some(entry.playlist_id) {
                // Keep existing entries for this playlist (additive)
                next_entry_index = next_entry_index.max(entry.entry_index + 1);
            }
            all_playlist_entries.push(PdbPlaylistEntryRow {
                entry_index: entry.entry_index,
                track_id: entry.track_id,
                playlist_id: entry.playlist_id,
            });
        }
    }

    // ── Ensure playlist exists in tree ──────────────────────────────────
    let playlist_pdb_id = if let Some(existing_id) = existing_playlist_pdb_id {
        // Preserve the existing row identity. Its fixed-width sort_order is
        // adjusted after this block for "exported playlist first"
        // behavior without rewriting the t07 row body.
        existing_id
    } else {
        let requested_id = override_playlist_id.unwrap_or(next_playlist_id);
        let id = if all_playlist_tree.iter().any(|row| row.id == requested_id) {
            next_playlist_id
        } else {
            requested_id
        };
        let so = override_sort_order.unwrap_or(next_sort_order);
        all_playlist_tree.push(PdbPlaylistTreeRow {
            id,
            parent_id: 0,
            sort_order: so,
            is_folder: false,
            name: playlist.name.clone(),
        });
        id
    };
    move_playlist_tree_row_to_front(&mut all_playlist_tree, playlist_pdb_id);

    // ── Existing playlist track IDs for dedup ───────────────────────────
    let existing_playlist_track_ids: HashSet<u32> = if existing_playlist_pdb_id.is_some() {
        all_playlist_entries
            .iter()
            .filter(|e| e.playlist_id == playlist_pdb_id)
            .map(|e| e.track_id)
            .collect()
    } else {
        HashSet::new()
    };

    let mut edb_key_by_path = HashMap::<String, String>::new();
    let mut edb_artist_by_path = HashMap::<String, String>::new();
    let mut edb_album_by_path = HashMap::<String, String>::new();
    let mut edb_artwork_by_path = HashMap::<String, String>::new();
    let mut edb_identity_by_path =
        HashMap::<String, (Option<u32>, Option<u32>, Option<u32>, Option<u16>)>::new();
    {
        let mut unlock_warnings = Vec::<WarningEntry>::new();
        if let Some(conn) = open_edb_rw(usb_root, &mut unlock_warnings)
            && table_exists(&conn, "content")
        {
            let has_key_table = table_exists(&conn, "key");
            let has_artist_table = table_exists(&conn, "artist");
            let has_album_table = table_exists(&conn, "album");
            let has_image_table = table_exists(&conn, "image");

            let image_fk_col = if has_image_table
                    && conn
                        .query_row(
                            "SELECT COUNT(1) FROM pragma_table_info('content') WHERE name = 'imageFilePath_id'",
                            [],
                            |row| row.get::<_, i64>(0),
                        )
                        .ok()
                        .unwrap_or(0)
                        > 0
                {
                    Some("c.imageFilePath_id")
                } else if has_image_table
                    && conn
                        .query_row(
                            "SELECT COUNT(1) FROM pragma_table_info('content') WHERE name = 'image_id'",
                            [],
                            |row| row.get::<_, i64>(0),
                        )
                        .ok()
                        .unwrap_or(0)
                        > 0
                {
                    Some("c.image_id")
                } else {
                    None
                };

            let key_select = if has_key_table { "k.name" } else { "NULL" };
            let key_join = if has_key_table {
                "LEFT JOIN \"key\" k ON k.key_id = c.key_id"
            } else {
                ""
            };
            let artist_select = if has_artist_table { "ar.name" } else { "NULL" };
            let artist_join = if has_artist_table {
                "LEFT JOIN artist ar ON ar.artist_id = c.artist_id_artist"
            } else {
                ""
            };
            let album_select = if has_album_table { "al.name" } else { "NULL" };
            let album_join = if has_album_table {
                "LEFT JOIN album al ON al.album_id = c.album_id"
            } else {
                ""
            };
            let image_select = if image_fk_col.is_some() {
                "img.path"
            } else {
                "NULL"
            };
            let image_join = if let Some(fk_col) = image_fk_col {
                format!("LEFT JOIN image img ON img.image_id = {fk_col}")
            } else {
                String::new()
            };

            let sql = format!(
                r#"
                    SELECT
                      c.path,
                      {key_select},
                      {artist_select},
                      {album_select},
                      {image_select},
                      CAST(c.contentLink AS INTEGER),
                      CAST(c.masterContentId AS INTEGER),
                      CAST(c.masterDbId AS INTEGER),
                      CAST(c.bitDepth AS INTEGER)
                    FROM content c
                    {key_join}
                    {artist_join}
                    {album_join}
                    {image_join}
                    WHERE c.path IS NOT NULL AND TRIM(c.path) != ''
                    "#,
            );

            if let Ok(mut stmt) = conn.prepare(&sql)
                && let Ok(rows) = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                    ))
                })
            {
                for row in rows.flatten() {
                    let key = canonicalize_track_path_identity(&row.0);
                    if key.is_empty() {
                        continue;
                    }
                    if let Some(name) = row.1
                        && !name.trim().is_empty()
                    {
                        edb_key_by_path.insert(key.clone(), name);
                    }
                    if let Some(name) = row.2
                        && !name.trim().is_empty()
                    {
                        edb_artist_by_path.insert(key.clone(), name);
                    }
                    if let Some(name) = row.3
                        && !name.trim().is_empty()
                    {
                        edb_album_by_path.insert(key.clone(), name);
                    }
                    if let Some(path) = row.4 {
                        let normalized = repair_utf8_mojibake(path.trim()).replace('\\', "/");
                        if !normalized.is_empty() {
                            edb_artwork_by_path.insert(key.clone(), normalized);
                        }
                    }
                    edb_identity_by_path.insert(
                        key,
                        (
                            row.5.and_then(|v| u32::try_from(v).ok()),
                            row.6.and_then(|v| u32::try_from(v).ok()),
                            row.7.and_then(|v| u32::try_from(v).ok()),
                            row.8.and_then(|v| u16::try_from(v).ok()),
                        ),
                    );
                }
            }
        }
    }

    // ── Process manifest tracks ─────────────────────────────────────────
    for (idx, track) in manifest.tracks.iter().enumerate() {
        // exported_path is already USB-relative (set by export.rs manifest builder)
        let file_path = track.exported_path.clone();
        let path_key = canonicalize_track_path_identity(&file_path);

        let pdb_track_id = if let Some(existing) = track_id_by_path.get(&path_key).copied() {
            // Update existing track metadata for exported members in all modes.
            if let Some(existing_track) = track_index_by_id
                .get(&existing)
                .copied()
                .and_then(|idx| all_tracks.get_mut(idx))
            {
                let edb_identity = edb_identity_by_path.get(&path_key).copied();
                // Update dictionary references — resolve fresh IDs from manifest data
                let resolved_artist_name = if !track.artist.trim().is_empty() {
                    Some(track.artist.as_str())
                } else {
                    edb_artist_by_path.get(&path_key).map(String::as_str)
                };
                if let Some(artist_name) = resolved_artist_name {
                    let artist_key = canonicalize_playlist_name(artist_name);
                    let aid = if let Some(id) = artist_id_by_name.get(&artist_key).copied() {
                        id
                    } else {
                        let new_id = next_artist_id;
                        next_artist_id += 1;
                        artist_id_by_name.insert(artist_key, new_id);
                        all_artists.push(PdbArtistRow {
                            id: new_id,
                            name: artist_name.to_string(),
                        });
                        new_id
                    };
                    existing_track.artist_id = aid;
                }
                if let Some(album) = track
                    .album
                    .as_deref()
                    .filter(|v| !v.trim().is_empty())
                    .or_else(|| edb_album_by_path.get(&path_key).map(String::as_str))
                {
                    let album_key = canonicalize_playlist_name(album);
                    let aid = if let Some(id) = album_id_by_name.get(&album_key).copied() {
                        id
                    } else {
                        let new_id = next_album_id;
                        next_album_id += 1;
                        album_id_by_name.insert(album_key, new_id);
                        all_albums.push(PdbAlbumRow {
                            id: new_id,
                            name: album.to_string(),
                            artist_id: existing_track.artist_id,
                        });
                        new_id
                    };
                    existing_track.album_id = aid;
                }
                let resolved_key_name = track
                    .key
                    .as_deref()
                    .filter(|v| !v.trim().is_empty())
                    .map(|v| v.to_string())
                    .or_else(|| edb_key_by_path.get(&path_key).cloned());
                if let Some(key_name) = resolved_key_name.as_deref() {
                    let key_lookup = normalize_pdb_key_name(key_name);
                    let kid = if let Some(id) = resolve_key_id_by_name(&key_id_by_name, key_name) {
                        id
                    } else {
                        let new_id = next_key_id;
                        next_key_id += 1;
                        key_id_by_name.insert(key_lookup, new_id);
                        all_keys.push(PdbKeyRow {
                            id: new_id,
                            name: key_name.to_string(),
                        });
                        new_id
                    };
                    existing_track.key_id = kid;
                }
                if let Some(genre_name) = track.genre.as_deref().filter(|v| !v.trim().is_empty()) {
                    let genre_key = canonicalize_playlist_name(genre_name);
                    let gid = if let Some(id) = genre_id_by_name.get(&genre_key).copied() {
                        id
                    } else {
                        let new_id = next_genre_id;
                        next_genre_id += 1;
                        genre_id_by_name.insert(genre_key, new_id);
                        all_genres.push(crate::pdb_writer::PdbDictRow {
                            id: new_id,
                            name: genre_name.to_string(),
                        });
                        new_id
                    };
                    existing_track.genre_id = gid;
                }
                if let Some(art_path) = track
                    .artwork_path
                    .as_deref()
                    .filter(|v| !v.trim().is_empty())
                    .map(|p| p.to_string())
                    .or_else(|| edb_artwork_by_path.get(&path_key).cloned())
                {
                    let relative = to_usb_relative_path(usb_root, &art_path)
                        .unwrap_or_else(|| art_path.to_string());
                    let artwork_key = canonicalize_artwork_path_lookup(&relative);
                    let aid = if let Some(id) = artwork_id_by_path.get(&artwork_key).copied() {
                        id
                    } else {
                        let new_id = next_artwork_id;
                        next_artwork_id += 1;
                        artwork_id_by_path.insert(artwork_key, new_id);
                        all_artwork.push(PdbArtworkRow {
                            id: new_id,
                            path: relative,
                        });
                        new_id
                    };
                    existing_track.artwork_id = aid;
                }
                // Update identity fields from manifest (only set for our library tracks)
                existing_track.master_db_id = edb_identity
                    .and_then(|(_, _, mdb, _)| mdb)
                    .or_else(|| track.master_db_id.and_then(|v| u32::try_from(v).ok()))
                    .or(existing_track.master_db_id);
                existing_track.master_content_id = edb_identity
                    .and_then(|(_, mci, _, _)| mci)
                    .or_else(|| track.master_content_id.and_then(|v| u32::try_from(v).ok()))
                    .or(existing_track.master_content_id);
                existing_track.content_link = edb_identity
                    .and_then(|(cl, _, _, _)| cl)
                    .or_else(|| track.content_link.and_then(|v| u32::try_from(v).ok()))
                    .or(existing_track.content_link);
                // Update scalar metadata from manifest
                if let Some(ms) = track.duration_ms {
                    existing_track.duration_seconds = Some(duration_ms_to_seconds(ms) as u32);
                }
                existing_track.bit_depth = edb_identity
                    .and_then(|(_, _, _, bd)| bd)
                    .or_else(|| track.bit_depth.map(u16::from))
                    .or(existing_track.bit_depth);
                if let Some(bpm) = track.bpm {
                    existing_track.bpm = Some(bpm);
                }
                if let Some(tn) = track.track_number {
                    existing_track.track_number = Some(tn);
                }
                // Same rule as the eDB row (`update_existing_content_row`).
                if let Some(color_id) = track.color_id.and_then(|v| u8::try_from(v).ok()) {
                    existing_track.color_id = Some(color_id);
                }
                if let Some(rating) = track.rating.and_then(|v| u8::try_from(v).ok()) {
                    existing_track.rating = Some(rating);
                }
                // These were previously never refreshed on an update, so a
                // re-exported track that got re-encoded/replaced (different
                // size, bitrate, sample rate) at the same path kept the
                // *original* insert's stale values forever, even though
                // duration/title/anlz_path above were refreshing correctly
                // -- a real PDB-vs-actual-file mismatch, not just a
                // collision-specific symptom.
                if let Some(size) = track.file_size_bytes.and_then(|v| u32::try_from(v).ok()) {
                    existing_track.file_size_bytes = Some(size);
                }
                if let Some(sr) = track.sample_rate_hz {
                    existing_track.sample_rate_hz = Some(sr);
                }
                if let Some(br) = track.bitrate_kbps {
                    existing_track.bitrate_kbps = Some(br);
                }
                if let Some(ft) = track.file_type.and_then(|v| u16::try_from(v).ok()) {
                    existing_track.file_type = Some(ft);
                }
                if let Some(ry) = resolve_track_release_year_for_export(track) {
                    existing_track.release_year = Some(ry);
                }
                if let Some(rd) = resolve_track_release_date_for_export(track) {
                    existing_track.release_date = Some(rd);
                }
                if let Some(ref isrc) = track.isrc {
                    existing_track.isrc = Some(isrc.clone());
                }
                if let Some(ref wp) = track.waveform_path {
                    existing_track.anlz_path =
                        to_usb_relative_path(usb_root, wp).unwrap_or_else(|| wp.clone());
                }
                existing_track.title = track.title.clone();
                existing_track.file_name = Some(content_file_name(&file_path));
                existing_track.file_path = file_path.clone();
            }
            existing
        } else {
            // Resolve dictionary IDs
            let resolved_artist_name = if !track.artist.trim().is_empty() {
                Some(track.artist.as_str())
            } else {
                edb_artist_by_path.get(&path_key).map(String::as_str)
            };
            let artist_id = if let Some(artist_name) = resolved_artist_name {
                let artist_key = canonicalize_playlist_name(artist_name);
                if let Some(existing) = artist_id_by_name.get(&artist_key).copied() {
                    existing
                } else {
                    let new_id = next_artist_id;
                    next_artist_id += 1;
                    artist_id_by_name.insert(artist_key, new_id);
                    all_artists.push(PdbArtistRow {
                        id: new_id,
                        name: artist_name.to_string(),
                    });
                    new_id
                }
            } else {
                0
            };
            let album_id = track
                .album
                .as_deref()
                .filter(|v| !v.trim().is_empty())
                .or_else(|| edb_album_by_path.get(&path_key).map(String::as_str))
                .map(|album| {
                    let album_key = canonicalize_playlist_name(album);
                    if let Some(existing) = album_id_by_name.get(&album_key).copied() {
                        existing
                    } else {
                        let new_id = next_album_id;
                        next_album_id += 1;
                        album_id_by_name.insert(album_key, new_id);
                        all_albums.push(PdbAlbumRow {
                            id: new_id,
                            name: album.to_string(),
                            artist_id,
                        });
                        new_id
                    }
                })
                .unwrap_or(0);
            let key_id = track
                .key
                .as_deref()
                .filter(|v| !v.trim().is_empty())
                .map(|v| v.to_string())
                .or_else(|| edb_key_by_path.get(&path_key).cloned())
                .map(|key_name| {
                    let key_lookup = normalize_pdb_key_name(&key_name);
                    if let Some(existing) = resolve_key_id_by_name(&key_id_by_name, &key_name) {
                        existing
                    } else {
                        let new_id = next_key_id;
                        next_key_id += 1;
                        key_id_by_name.insert(key_lookup, new_id);
                        all_keys.push(PdbKeyRow {
                            id: new_id,
                            name: key_name.to_string(),
                        });
                        new_id
                    }
                })
                .unwrap_or(0);
            let genre_id = track
                .genre
                .as_deref()
                .filter(|v| !v.trim().is_empty())
                .map(|genre_name| {
                    let genre_key = canonicalize_playlist_name(genre_name);
                    if let Some(existing) = genre_id_by_name.get(&genre_key).copied() {
                        existing
                    } else {
                        let new_id = next_genre_id;
                        next_genre_id += 1;
                        genre_id_by_name.insert(genre_key, new_id);
                        all_genres.push(crate::pdb_writer::PdbDictRow {
                            id: new_id,
                            name: genre_name.to_string(),
                        });
                        new_id
                    }
                })
                .unwrap_or(0);
            let artwork_id = track
                .artwork_path
                .as_deref()
                .filter(|v| !v.trim().is_empty())
                .map(|art_path| art_path.to_string())
                .or_else(|| edb_artwork_by_path.get(&path_key).cloned())
                .map(|art_path| {
                    let relative = to_usb_relative_path(usb_root, &art_path)
                        .unwrap_or_else(|| art_path.to_string());
                    let artwork_key = canonicalize_artwork_path_lookup(&relative);
                    if let Some(existing) = artwork_id_by_path.get(&artwork_key).copied() {
                        existing
                    } else {
                        let new_id = next_artwork_id;
                        next_artwork_id += 1;
                        artwork_id_by_path.insert(artwork_key, new_id);
                        all_artwork.push(PdbArtworkRow {
                            id: new_id,
                            path: relative,
                        });
                        new_id
                    }
                })
                .unwrap_or(0);

            let track_id = next_track_id;
            next_track_id += 1;
            // waveform_path is already USB-relative (set by export.rs manifest builder)
            let anlz_path = track.waveform_path.clone().unwrap_or_default();
            let edb_identity = edb_identity_by_path.get(&path_key).copied();
            all_tracks.push(PdbTrackRowData {
                // Bytes 2..4 are the page-local index_shift and are assigned
                // when the row is placed into its final PDB page.
                header_flags_u32: Some(0x0000_0024),
                content_link: edb_identity
                    .and_then(|(cl, _, _, _)| cl)
                    .or_else(|| track.content_link.and_then(|v| u32::try_from(v).ok())),
                sample_rate_hz: track.sample_rate_hz,
                file_size_bytes: track.file_size_bytes.and_then(|v| u32::try_from(v).ok()),
                master_content_id: edb_identity
                    .and_then(|(_, mci, _, _)| mci)
                    .or_else(|| track.master_content_id.and_then(|v| u32::try_from(v).ok())),
                master_db_id: edb_identity
                    .and_then(|(_, _, mdb, _)| mdb)
                    .or_else(|| track.master_db_id.and_then(|v| u32::try_from(v).ok())),
                id: track_id,
                artist_id,
                album_id,
                artwork_id,
                key_id,
                genre_id,
                bitrate_kbps: track.bitrate_kbps,
                track_number: track.track_number,
                bpm: track.bpm,
                release_year: resolve_track_release_year_for_export(track),
                bit_depth: edb_identity
                    .and_then(|(_, _, _, bd)| bd)
                    .or_else(|| track.bit_depth.map(u16::from)),
                duration_seconds: track
                    .duration_ms
                    .map(|ms| duration_ms_to_seconds(ms) as u32),
                file_type: track.file_type.and_then(|v| u16::try_from(v).ok()),
                isrc: track.isrc.clone(),
                date_added: Some(export_date_added.clone()),
                release_date: resolve_track_release_date_for_export(track),
                dj_comment: track.comment.clone(),
                file_name: Some(content_file_name(&track.exported_path)),
                publish_track_info_on: Some(true),
                autoload_hotcues_on: Some(true),
                color_id: track.color_id.and_then(|v| u8::try_from(v).ok()),
                rating: track.rating.and_then(|v| u8::try_from(v).ok()),
                title: track.title.clone(),
                anlz_path,
                file_path: file_path.clone(),
            });
            track_index_by_id.insert(track_id, all_tracks.len() - 1);
            track_id_by_path.insert(path_key, track_id);
            track_id
        };

        if let Some(track_row) = track_index_by_id
            .get(&pdb_track_id)
            .and_then(|&idx| all_tracks.get(idx))
        {
            desired_manifest_track_rows.insert(pdb_track_id, track_row.clone());
        }

        // Add playlist entry
        if !mirror_playlist_entries
            && existing_playlist_pdb_id.is_some()
            && existing_playlist_track_ids.contains(&pdb_track_id)
        {
            continue;
        }
        let entry_index = if mirror_playlist_entries {
            (idx as u32) + 1
        } else {
            let current = next_entry_index;
            next_entry_index += 1;
            current
        };
        all_playlist_entries.push(PdbPlaylistEntryRow {
            entry_index,
            track_id: pdb_track_id,
            playlist_id: playlist_pdb_id,
        });
    }

    // ── Build PdbData and write ─────────────────────────────────────────
    let pdb_data = PdbData {
        tracks: all_tracks,
        artists: all_artists,
        albums: all_albums,
        genres: all_genres,
        labels: if let Some(ref parsed) = existing {
            parsed
                .labels
                .iter()
                .map(|(id, name)| crate::pdb_writer::PdbDictRow {
                    id: *id,
                    name: name.clone(),
                })
                .collect()
        } else {
            Vec::new()
        },
        keys: all_keys,
        colors: crate::pdb_writer::standard_colors(),
        artwork: all_artwork,
        playlist_tree: all_playlist_tree,
        playlist_entries: all_playlist_entries,
        // PDB t16 (the player menu definition) is preserved byte-stable from the
        // source stick when one exists. Earlier we overwrote it with our
        // 10-row default which broke newer-player acceptance for sticks that
        // started with the standard 27-row t16. Only on a true
        // fresh init (no existing PDB on disk) do we fall back to the
        // default. User-driven menu edits go through the Menu Editor
        // which patches t16 in place — those edits also survive here.
        columns_raw_rows: existing_pdb_bytes_before
            .as_deref()
            .and_then(preserve_existing_t16_rows)
            .unwrap_or_else(crate::pdb_writer::standard_columns_raw),
        history_playlists: if let Some(ref parsed) = existing {
            parsed
                .history_playlists
                .iter()
                .filter(|h| h.source_table == 17)
                .map(|h| crate::pdb_writer::PdbDictRow {
                    id: h.id,
                    name: h.name.clone(),
                })
                .collect()
        } else {
            Vec::new()
        },
        history_entries: if let Some(ref parsed) = existing {
            parsed
                .history_entries
                .iter()
                .filter(|h| h.source_table == 18)
                .map(|h| crate::pdb_writer::PdbHistoryEntryRow {
                    track_id: h.track_id.unwrap_or(0),
                    playlist_id: h.playlist_id,
                    entry_index: h.entry_index,
                })
                .collect()
        } else {
            Vec::new()
        },
        // The PDB writers derive t19's runtime rows from these; see
        // `t19_ring_rows`.
        history_raw_rows: existing
            .as_ref()
            .map(|parsed| parsed.history_raw_rows_bytes.clone())
            .unwrap_or_default(),
        // The eDB export runs first, so this is the property it just wrote.
        t19_property: edb_t19_property(usb_root),
        profile,
    };

    // Topology-locked additive dispatch.
    //
    // Any existing PDB, including the initialize_usb template, must be patched
    // in place. Rebuilding and reshaping populated PDBs has produced player hardware
    // freezes and player "database corrupted" failures on real hardware.
    let pdb_write_mode = std::env::var("EXPORTER_PDB_WRITE_MODE")
        .ok()
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_else(|| "auto".to_string());
    if let Some(before) = existing_pdb_bytes_before.as_deref() {
        if pdb_write_mode == "fresh" {
            return Err(BackendError::Validation(
                "PDB export blocked: EXPORTER_PDB_WRITE_MODE=fresh is unsafe for existing PDBs"
                    .into(),
            ));
        }
        let desired_t08: Vec<T08EntryKey> = pdb_data
            .playlist_entries
            .iter()
            .filter(|e| e.playlist_id == playlist_pdb_id)
            .map(|e| T08EntryKey {
                entry_index: e.entry_index,
                track_id: e.track_id,
                playlist_id: e.playlist_id,
            })
            .collect();
        match crate::pdb_writer::try_write_pdb_additive_in_place(
            before,
            &pdb_data,
            desired_t08,
            4096,
        )? {
            Some((additive_bytes, summary)) => {
                validate_topology_locked_export_bytes(before, &additive_bytes)?;
                if commit_write {
                    crate::service::usb_staging::commit_and_write_back(
                        usb_root,
                        crate::service::usb_staging::DbKind::Pdb,
                        &additive_bytes,
                    )?;
                }
                return Ok(WriteExportPdbResult {
                    inserted_tracks: summary.new_tracks,
                    inserted_playlists: summary.new_playlist_tree,
                    topology_issues: Vec::new(),
                    writer_warnings: Vec::new(),
                });
            }
            None => {
                return Err(BackendError::Validation(
                    "PDB export blocked: topology-locked additive writer declined this change; \
                     refusing unsafe fresh rebuild"
                        .into(),
                ));
            }
        }
    }

    // No existing PDB on disk. initialize_usb must be called before export_to_usb.
    // The topology-locked additive writer is the only supported export path.
    //
    // This line is only reached if a caller skips initialize_usb entirely; all
    // production and test paths call initialize_usb first.
    Err(BackendError::Validation(
        "PDB export requires an initialized USB: call initialize_usb before export_to_usb".into(),
    ))
}

/// Rewrite a single playlist in the PDB using the ground-up writer.
/// Delegates to `write_pdb()` with `mirror_playlist_entries=true`.
/// Overrides the playlist's PDB id and sort_order to match eDB values.
#[cfg(test)]
pub fn rewrite_pdb_playlist_from_manifest(
    usb_root: &Path,
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
    playlist_pdb_id: u32,
    sort_order: u32,
) -> BackendResult<WriteExportPdbResult> {
    write_pdb_fresh_with_overrides(
        usb_root,
        playlist,
        manifest,
        true,
        Some(playlist_pdb_id),
        Some(sort_order),
        true,
    )
}
