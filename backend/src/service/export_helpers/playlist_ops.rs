//! Playlist and track removal from eDB and PDB, content verification.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{OptionalExtension, params};

use super::{
    ExclusiveTrackInfo, ExportManifest, ExportPlaylistData, PlaylistRemovalPdbResult,
    normalize_owned_export_path,
};
use crate::edb::{
    open_edb_from_usb_root, open_edb_rw, preferred_export_playlist_row_id, table_exists,
};
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::WarningEntry;
use crate::pdb_reader::parse_pdb;
use crate::service::usb_utils::canonicalize_playlist_name;

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
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<usize> {
    let mut unlock_warnings = Vec::<WarningEntry>::new();
    let Some(mut conn) = open_edb_rw(usb_root, &mut unlock_warnings) else {
        if !unlock_warnings.is_empty() {
            warnings.push(logging::log(
                Level::Warn,
                "usb-import",
                "usb.remove.edb-open-failed",
                format!(
                    "eDB playlist removal skipped: {}",
                    unlock_warnings
                        .iter()
                        .map(|w| w.message.as_str())
                        .collect::<Vec<_>>()
                        .join(" | ")
                ),
            ));
        }
        return Ok(0);
    };
    let tx = conn.transaction()?;
    if !table_exists(&tx, "playlist") || !table_exists(&tx, "playlist_content") {
        warnings.push(logging::log(
            Level::Warn,
            "usb-import",
            "usb.remove.missing-tables",
            "eDB playlist removal skipped: missing playlist tables",
        ));
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
                warnings.push(logging::log(
                    Level::Info,
                    "usb-import",
                    "usb.remove.db-cleaned",
                    format!("removed content row: {path}"),
                ));
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
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<PlaylistRemovalPdbResult> {
    let empty_result = PlaylistRemovalPdbResult {
        removed_playlist_count: 0,
        exclusive_tracks: Vec::new(),
        shared_track_count: 0,
    };
    let pdb_path = crate::service::usb_staging::stage_pdb(usb_root)?;
    if !pdb_path.is_file() {
        warnings.push(logging::log(
            Level::Warn,
            "usb-import",
            "usb.remove.pdb-missing",
            "PDB playlist removal skipped: PDB missing",
        ));
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
            && target_track_ids.contains(&tid)
        {
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

    crate::service::usb_staging::commit_and_write_back(
        usb_root,
        crate::service::usb_staging::DbKind::Pdb,
        &pdb_bytes,
    )?;

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

    let pdb_path = crate::service::usb_staging::stage_pdb(usb_root)?;
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MirrorOrphanCleanup {
    pub removed_edb_content: usize,
    pub removed_pdb_tracks: usize,
    pub removed_pdb_artwork: usize,
}

/// Mirror export only. Given the set of stale media paths the prune step has
/// already cleared for deletion — i.e. tracks dropped from the exported
/// playlist that `filter_prunable_stale_paths_for_playlist` confirmed are not
/// referenced by any other USB playlist and not in device play history —
/// delete the matching eDB `content` rows and tombstone the matching PDB
/// `tt=0` track rows in place (plus any now-exclusive `tt=13` artwork rows and
/// orphaned eDB `image` rows).
///
/// Without this the rows outlive the files the prune step removes, and strict
/// parity then reports "N of M indexed audio file(s) missing from USB".
///
/// PDB removal uses the same in-place tombstone path as `remove_playlist_and_
/// tracks_from_pdb` (`remove_rows_inplace` + `rebuild_sentinel_btrees_inplace`):
/// row-presence bits are cleared, page layout and table chains are untouched.
pub fn remove_orphaned_dropped_tracks_from_dbs(
    usb_root: &Path,
    prunable_media_paths: &[String],
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<MirrorOrphanCleanup> {
    let mut result = MirrorOrphanCleanup::default();

    let target_keys: HashSet<String> = prunable_media_paths
        .iter()
        .filter_map(|p| normalize_owned_export_path(usb_root, p))
        .filter(|p| p.starts_with("/Contents/"))
        .map(|p| canonicalize_playlist_name(&p))
        .collect();
    if target_keys.is_empty() {
        return Ok(result);
    }

    let pdb_path = crate::service::usb_staging::stage_pdb(usb_root)?;
    if !pdb_path.is_file() {
        return Ok(result);
    }
    let parsed = parse_pdb(&pdb_path)?;

    // Post-write "still referenced somewhere" guard. After the mirror write the
    // target playlist's entries are exactly the manifest, so a dropped track
    // that is still a member of another playlist (or in history) shows up here
    // and is left alone.
    let referenced_track_ids: HashSet<u32> = parsed
        .playlist_entries
        .iter()
        .map(|e| e.track_id)
        .chain(parsed.history_entries.iter().filter_map(|h| h.track_id))
        .collect();

    let orphan_track_ids: HashSet<u32> = parsed
        .tracks
        .iter()
        .filter(|t| !referenced_track_ids.contains(&t.id))
        .filter(|t| {
            normalize_owned_export_path(usb_root, &t.track_file_path)
                .map(|norm| target_keys.contains(&canonicalize_playlist_name(&norm)))
                .unwrap_or(false)
        })
        .map(|t| t.id)
        .collect();
    if orphan_track_ids.is_empty() {
        return Ok(result);
    }

    // --- eDB: delete content rows for those paths, then orphaned image rows ---
    let mut unlock_warnings = Vec::<WarningEntry>::new();
    if let Some(conn) = open_edb_rw(usb_root, &mut unlock_warnings) {
        if table_exists(&conn, "content") {
            let mut to_delete = Vec::<i64>::new();
            {
                let mut stmt = conn.prepare("SELECT content_id, path FROM content")?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
                })?;
                for row in rows {
                    let (content_id, path) = row?;
                    let Some(norm) = path
                        .as_deref()
                        .and_then(|p| normalize_owned_export_path(usb_root, p))
                    else {
                        continue;
                    };
                    if target_keys.contains(&canonicalize_playlist_name(&norm)) {
                        to_delete.push(content_id);
                    }
                }
            }
            let has_playlist_content = table_exists(&conn, "playlist_content");
            for content_id in &to_delete {
                // Never delete a content row still linked to any playlist. The
                // mirror write only unlinks dropped tracks from the target
                // playlist and never touches others, so this should not fire —
                // it is a guard against a path-normalisation mismatch with the
                // PDB-side check above.
                if has_playlist_content {
                    let still_linked: Option<i64> = conn
                        .query_row(
                            "SELECT 1 FROM playlist_content WHERE content_id = ?1 LIMIT 1",
                            params![content_id],
                            |row| row.get(0),
                        )
                        .optional()?;
                    if still_linked.is_some() {
                        continue;
                    }
                }
                result.removed_edb_content += conn.execute(
                    "DELETE FROM content WHERE content_id = ?1",
                    params![content_id],
                )?;
            }
            if result.removed_edb_content > 0 && table_exists(&conn, "image") {
                let _ = conn.execute(
                    "DELETE FROM image WHERE image_id NOT IN (SELECT DISTINCT image_id FROM content WHERE image_id IS NOT NULL)",
                    [],
                );
            }
        }
        drop(conn);
        crate::service::usb_staging::write_back_if_changed(
            usb_root,
            crate::service::usb_staging::DbKind::Edb,
        )?;
    } else {
        warnings.extend(unlock_warnings);
    }

    // --- PDB: tombstone track rows + now-exclusive artwork rows in place ---
    use crate::pdb_writer::{
        extract_artwork_id, extract_track_id, rebuild_sentinel_btrees_inplace, remove_rows_inplace,
    };

    let exclusive_artwork_ids: HashSet<u32> = {
        let removed: HashSet<u32> = parsed
            .tracks
            .iter()
            .filter(|t| orphan_track_ids.contains(&t.id) && t.artwork_id != 0)
            .map(|t| t.artwork_id)
            .collect();
        let remaining: HashSet<u32> = parsed
            .tracks
            .iter()
            .filter(|t| !orphan_track_ids.contains(&t.id) && t.artwork_id != 0)
            .map(|t| t.artwork_id)
            .collect();
        removed.difference(&remaining).copied().collect()
    };

    let mut pdb_bytes = std::fs::read(&pdb_path)?;
    result.removed_pdb_tracks =
        remove_rows_inplace(&mut pdb_bytes, 0, &orphan_track_ids, extract_track_id);
    if !exclusive_artwork_ids.is_empty() {
        result.removed_pdb_artwork = remove_rows_inplace(
            &mut pdb_bytes,
            13,
            &exclusive_artwork_ids,
            extract_artwork_id,
        );
    }
    if result.removed_pdb_tracks > 0 || result.removed_pdb_artwork > 0 {
        rebuild_sentinel_btrees_inplace(&mut pdb_bytes);
        crate::service::usb_staging::commit_and_write_back(
            usb_root,
            crate::service::usb_staging::DbKind::Pdb,
            &pdb_bytes,
        )?;
    }

    if result != MirrorOrphanCleanup::default() {
        warnings.push(logging::log(
            Level::Info,
            "export",
            "export.orphan-tracks-removed",
            format!(
                "mirror export removed {} dropped-track content row(s) from eDB and {} track row(s) ({} artwork row(s)) from PDB — no other playlist or history references them",
                result.removed_edb_content, result.removed_pdb_tracks, result.removed_pdb_artwork
            ),
        ));
    }

    Ok(result)
}

pub fn verify_edb_content(
    usb_root: &Path,
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
) -> BackendResult<()> {
    let mut warnings = Vec::<WarningEntry>::new();
    let Some(conn) = open_edb_from_usb_root(usb_root, &mut warnings) else {
        return Err(BackendError::Internal(format!(
            "export verification failed: unable to read eDB ({})",
            warnings
                .iter()
                .map(|w| w.message.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        )));
    };
    verify_edb_content_with_conn(usb_root, &conn, playlist, manifest)
}

pub fn verify_edb_content_with_conn(
    usb_root: &Path,
    conn: &rusqlite::Connection,
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
) -> BackendResult<()> {
    let playlist_id: i64 =
        preferred_export_playlist_row_id(conn, &playlist.name)?.ok_or_else(|| {
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
    let pdb_path = crate::service::usb_staging::stage_pdb(usb_root)?;
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
                && expected_paths.contains(path)
            {
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
