//! Playlist and track removal from eDB and PDB, content verification.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{OptionalExtension, params};

use super::{ExclusiveTrackInfo, ExportManifest, ExportPlaylistData, PlaylistRemovalPdbResult};
use crate::edb::{
    open_edb_from_usb_root, open_edb_rw, preferred_export_playlist_row_id, table_exists,
};
use crate::error::{BackendError, BackendResult};
use crate::pdb_reader::parse_pdb;
use crate::service::usb_utils::canonicalize_playlist_name;
use crate::service::usb_vendor_compat::{USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR};

fn parse_usb_playlist_numeric_id(raw: Option<&str>) -> Option<u32> {
    let id_part = raw?.trim().strip_prefix("usb-pl-")?;
    id_part.parse::<u32>().ok()
}

fn verify_optional_export_asset_exists(
    usb_root: &Path,
    path: Option<&str>,
    label: &str,
) -> BackendResult<()> {
    if let Some(path) = path.filter(|v| !v.trim().is_empty()) {
        let abs = usb_root.join(path.trim_start_matches('/'));
        if !abs.is_file() {
            return Err(BackendError::Validation(format!(
                "export verification failed: {label} file missing on disk: {}",
                abs.display()
            )));
        }
    }
    Ok(())
}

pub fn remove_playlist_from_edb(
    usb_root: &Path,
    name_candidates: &[String],
    exclusive_track_paths: &[String],
    warnings: &mut Vec<String>,
) -> BackendResult<usize> {
    let mut unlock_warnings = Vec::<String>::new();
    let Some(mut conn) = open_edb_rw(usb_root, &mut unlock_warnings) else {
        if !unlock_warnings.is_empty() {
            warnings.push(format!(
                "eDB playlist removal skipped: {}",
                unlock_warnings.join(" | ")
            ));
        }
        return Ok(0);
    };
    let tx = conn.transaction()?;
    if !table_exists(&tx, "playlist") || !table_exists(&tx, "playlist_content") {
        warnings.push("eDB playlist removal skipped: missing playlist tables".to_string());
        return Ok(0);
    }

    let wanted = name_candidates
        .iter()
        .map(|n| canonicalize_playlist_name(n))
        .collect::<HashSet<_>>();
    let mut ids = Vec::<i64>::new();
    {
        let mut stmt = tx.prepare("SELECT playlist_id, name FROM playlist WHERE attribute = 0")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let Ok((playlist_id, name)) = row else {
                continue;
            };
            if wanted.contains(&canonicalize_playlist_name(&name)) {
                ids.push(playlist_id);
            }
        }
    }
    if ids.is_empty() {
        return Ok(0);
    }
    for playlist_id in &ids {
        tx.execute(
            "DELETE FROM playlist_content WHERE playlist_id = ?1",
            params![playlist_id],
        )?;
        tx.execute(
            "DELETE FROM playlist WHERE playlist_id = ?1",
            params![playlist_id],
        )?;
    }

    // Delete content rows for exclusive tracks
    if !exclusive_track_paths.is_empty() && table_exists(&tx, "content") {
        for path in exclusive_track_paths {
            let deleted = tx.execute("DELETE FROM content WHERE path = ?1", params![path])?;
            if deleted > 0 {
                warnings.push(format!("removed content row: {path}"));
            }
        }
        // Also clean up image rows that are no longer referenced
        if table_exists(&tx, "image") {
            let _ = tx.execute(
                "DELETE FROM image WHERE image_id NOT IN (SELECT DISTINCT image_id FROM content WHERE image_id IS NOT NULL)",
                [],
            );
        }
    }

    tx.commit()?;
    Ok(ids.len())
}

pub fn remove_playlist_and_tracks_from_pdb(
    usb_root: &Path,
    playlist_id_hint: Option<&str>,
    name_candidates: &[String],
    warnings: &mut Vec<String>,
) -> BackendResult<PlaylistRemovalPdbResult> {
    let empty_result = PlaylistRemovalPdbResult {
        removed_playlist_count: 0,
        exclusive_tracks: Vec::new(),
        shared_track_count: 0,
    };
    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    if !pdb_path.is_file() {
        warnings.push("PDB playlist removal skipped: PDB missing".to_string());
        return Ok(empty_result);
    }
    let parsed = parse_pdb(&pdb_path)?;

    let wanted_names = name_candidates
        .iter()
        .map(|n| canonicalize_playlist_name(n))
        .collect::<HashSet<_>>();
    let hinted_id = parse_usb_playlist_numeric_id(playlist_id_hint);
    let remove_ids = parsed
        .playlist_tree
        .iter()
        .filter(|row| !row.row_is_folder)
        .filter(|row| {
            hinted_id.is_some_and(|id| id == row.id)
                || wanted_names.contains(&canonicalize_playlist_name(&row.name))
        })
        .map(|row| row.id)
        .collect::<HashSet<_>>();
    if remove_ids.is_empty() {
        return Ok(empty_result);
    }

    // --- Shared-track detection (includes history entries) ---
    let target_track_ids: HashSet<u32> = parsed
        .playlist_entries
        .iter()
        .filter(|e| remove_ids.contains(&e.playlist_id))
        .map(|e| e.track_id)
        .collect();
    let mut shared_track_ids: HashSet<u32> = parsed
        .playlist_entries
        .iter()
        .filter(|e| !remove_ids.contains(&e.playlist_id))
        .filter(|e| target_track_ids.contains(&e.track_id))
        .map(|e| e.track_id)
        .collect();
    // Also protect tracks referenced by history entries
    for he in &parsed.history_entries {
        if let Some(tid) = he.track_id
            && target_track_ids.contains(&tid) {
                shared_track_ids.insert(tid);
            }
    }
    let exclusive_track_ids: HashSet<u32> = target_track_ids
        .difference(&shared_track_ids)
        .copied()
        .collect();

    // Collect info about exclusive tracks for file cleanup
    let exclusive_tracks: Vec<ExclusiveTrackInfo> = parsed
        .tracks
        .iter()
        .filter(|t| exclusive_track_ids.contains(&t.id))
        .map(|t| ExclusiveTrackInfo {
            track_file_path: t.track_file_path.clone(),
            anlz_path: t.anlz_path.clone(),
            artwork_id: t.artwork_id,
        })
        .collect();

    let shared_track_count = shared_track_ids.len();

    // --- In-place deletion: mark rows as not-in-use, preserving page layout ---
    use crate::pdb_writer::{
        extract_artwork_id, extract_playlist_entry_playlist_id, extract_playlist_tree_id,
        extract_track_id, rebuild_sentinel_btrees_inplace, remove_rows_inplace,
    };

    let mut pdb_bytes = std::fs::read(&pdb_path)?;

    // Determine exclusive artwork IDs before mutating
    let exclusive_artwork_ids: HashSet<u32> = {
        let removed_art_ids: HashSet<u32> = exclusive_tracks
            .iter()
            .filter(|t| t.artwork_id != 0)
            .map(|t| t.artwork_id)
            .collect();
        // Artwork still referenced by remaining (non-exclusive) tracks
        let remaining_art_ids: HashSet<u32> = parsed
            .tracks
            .iter()
            .filter(|t| !exclusive_track_ids.contains(&t.id))
            .filter(|t| t.artwork_id != 0)
            .map(|t| t.artwork_id)
            .collect();
        removed_art_ids
            .difference(&remaining_art_ids)
            .copied()
            .collect()
    };

    // Mark playlist tree rows as deleted (table type 7)
    remove_rows_inplace(&mut pdb_bytes, 7, &remove_ids, extract_playlist_tree_id);

    // Mark playlist entries as deleted (table type 8)
    remove_rows_inplace(
        &mut pdb_bytes,
        8,
        &remove_ids,
        extract_playlist_entry_playlist_id,
    );

    // Mark exclusive tracks as deleted (table type 0)
    remove_rows_inplace(&mut pdb_bytes, 0, &exclusive_track_ids, extract_track_id);

    // Mark exclusive artwork as deleted (table type 13)
    if !exclusive_artwork_ids.is_empty() {
        remove_rows_inplace(
            &mut pdb_bytes,
            13,
            &exclusive_artwork_ids,
            extract_artwork_id,
        );
    }

    // Rebuild sentinel B-trees: tombstone ops set the D flag (0x10) on modified
    // pages (0x24 → 0x34). The sentinel must index all 0x34 pages; an outdated
    // B-tree causes DJ software to reject the database as corrupted.
    rebuild_sentinel_btrees_inplace(&mut pdb_bytes);

    std::fs::write(&pdb_path, &pdb_bytes)?;

    Ok(PlaylistRemovalPdbResult {
        removed_playlist_count: remove_ids.len(),
        exclusive_tracks,
        shared_track_count,
    })
}

pub fn remove_track_ids_from_pdb_playlist_entries(
    usb_root: &Path,
    track_ids_to_remove: &HashSet<u32>,
) -> BackendResult<usize> {
    use crate::pdb_writer::{extract_playlist_entry_track_id, remove_rows_inplace};

    if track_ids_to_remove.is_empty() {
        return Ok(0);
    }

    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    if !pdb_path.is_file() {
        return Ok(0);
    }

    let mut pdb_bytes = std::fs::read(&pdb_path)?;
    let removed = remove_rows_inplace(
        &mut pdb_bytes,
        8,
        track_ids_to_remove,
        extract_playlist_entry_track_id,
    );
    if removed == 0 {
        return Ok(0);
    }

    std::fs::write(&pdb_path, &pdb_bytes)?;
    Ok(removed)
}

pub fn verify_edb_content(
    usb_root: &Path,
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
) -> BackendResult<()> {
    let mut warnings = Vec::<String>::new();
    let Some(conn) = open_edb_from_usb_root(usb_root, &mut warnings) else {
        return Err(BackendError::Internal(format!(
            "export verification failed: unable to read eDB ({})",
            warnings.join(" | ")
        )));
    };

    let playlist_id: i64 =
        preferred_export_playlist_row_id(&conn, &playlist.name)?.ok_or_else(|| {
            BackendError::Internal(format!(
                "export verification failed: playlist not found in eDB: {}",
                playlist.name
            ))
        })?;

    for track in &manifest.tracks {
        let row = conn
            .query_row(
                r#"
                SELECT c.content_id, c.analysisDataFilePath, img.path
                FROM content c
                LEFT JOIN image img ON img.image_id = c.image_id
                WHERE c.path = ?1
                LIMIT 1
                "#,
                params![track.exported_path],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;

        let Some((content_id, analysis_path, image_path)) = row else {
            return Err(BackendError::Internal(format!(
                "export verification failed: content row missing for exported path {}",
                track.exported_path
            )));
        };

        let linked: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM playlist_content WHERE playlist_id = ?1 AND content_id = ?2 LIMIT 1",
                params![playlist_id, content_id],
                |row| row.get(0),
            )
            .optional()?;
        if linked.is_none() {
            return Err(BackendError::Internal(format!(
                "export verification failed: playlist_content missing for content_id {content_id}"
            )));
        }

        if track.owns_waveform {
            verify_optional_export_asset_exists(usb_root, analysis_path.as_deref(), "analysis")?;
        }
        if track.owns_artwork {
            verify_optional_export_asset_exists(usb_root, image_path.as_deref(), "artwork")?;
        }
    }

    Ok(())
}

pub fn verify_pdb_content(
    usb_root: &Path,
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
) -> BackendResult<()> {
    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let parsed = parse_pdb(&pdb_path)?;

    let playlist_ids = parsed
        .playlist_tree
        .iter()
        .filter(|node| !node.row_is_folder && node.name == playlist.name)
        .map(|node| node.id)
        .collect::<Vec<_>>();
    if playlist_ids.is_empty() {
        return Err(BackendError::Internal(format!(
            "export verification failed: playlist not found in PDB: {}",
            playlist.name
        )));
    }

    let track_path_by_id = parsed
        .tracks
        .iter()
        .map(|t| {
            (
                t.id,
                canonicalize_playlist_name(&t.track_file_path.replace('\\', "/")),
            )
        })
        .collect::<HashMap<_, _>>();
    let expected_paths = manifest
        .tracks
        .iter()
        .map(|t| canonicalize_playlist_name(&t.exported_path))
        .collect::<HashSet<_>>();

    let mut matched = HashSet::<String>::new();
    for playlist_id in playlist_ids {
        for entry in parsed
            .playlist_entries
            .iter()
            .filter(|e| e.playlist_id == playlist_id)
        {
            if let Some(path) = track_path_by_id.get(&entry.track_id)
                && expected_paths.contains(path) {
                    matched.insert(path.clone());
                }
        }
    }

    if matched.len() < expected_paths.len() {
        return Err(BackendError::Internal(format!(
            "export verification failed: PDB playlist entries matched {}/{} exported tracks",
            matched.len(),
            expected_paths.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_usb_playlist_numeric_id;

    #[test]
    fn parse_usb_playlist_numeric_id_valid() {
        assert_eq!(parse_usb_playlist_numeric_id(Some("usb-pl-42")), Some(42));
        assert_eq!(parse_usb_playlist_numeric_id(Some("usb-pl-0")), Some(0));
        assert_eq!(
            parse_usb_playlist_numeric_id(Some("usb-pl-4294967295")),
            Some(u32::MAX)
        );
    }

    #[test]
    fn parse_usb_playlist_numeric_id_trims_whitespace() {
        assert_eq!(parse_usb_playlist_numeric_id(Some("  usb-pl-7  ")), Some(7));
    }

    #[test]
    fn parse_usb_playlist_numeric_id_rejects_bad_input() {
        assert_eq!(parse_usb_playlist_numeric_id(None), None);
        assert_eq!(parse_usb_playlist_numeric_id(Some("")), None);
        assert_eq!(parse_usb_playlist_numeric_id(Some("42")), None);
        assert_eq!(parse_usb_playlist_numeric_id(Some("pl-42")), None);
        assert_eq!(parse_usb_playlist_numeric_id(Some("usb-pl-")), None);
        assert_eq!(parse_usb_playlist_numeric_id(Some("usb-pl-abc")), None);
    }
}
