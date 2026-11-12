//! Playlist and track removal from eDB and PDB, content verification.

use crate::utils::counted;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{OptionalExtension, params};

use super::{
    ExclusiveTrackInfo, ExportManifest, ExportPlaylistData, PlaylistRemovalPdbResult,
    canonicalize_track_path_identity, normalize_owned_export_path,
};
use crate::edb::{open_edb_from_usb_root, preferred_export_playlist_row_id, table_exists};
use crate::error::{BackendError, BackendResult};
use crate::i18n::tr;
use crate::logging::{self, Level};
use crate::models::WarningEntry;
use crate::pdb_reader::parse_pdb;
use crate::service::track_matching::{ContentFingerprint, pdb_track_fingerprint};
use crate::service::usb_staging::{UsbDatabases, UsbDatabasesExt, UsbWriteSession};
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
            return Err(BackendError::Internal(format!(
                "export verification failed: {label} file missing on disk: {}",
                abs.display()
            )));
        }
    }
    Ok(())
}

pub fn remove_playlist_from_edb(
    session: &mut UsbWriteSession<'_>,
    name_candidates: &[String],
    exclusive_track_paths: &[String],
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<usize> {
    // A USB without an eDB has nothing to remove.
    if session.databases()?.edb().is_none() {
        return Ok(0);
    }
    let Some(conn) = session.edb()? else {
        // One that has it but can't be opened is worth a warning (how the
        // open failed is in the log file).
        warnings.push(logging::log_text(
            Level::Warn,
            "usb-import",
            "usb.remove.edb-open-failed",
            &tr::usb_playlists_warning_edb_removal_skipped(),
        ));
        return Ok(0);
    };
    let tx = conn.transaction()?;
    if !table_exists(&tx, "playlist") || !table_exists(&tx, "playlist_content") {
        logging::debug(
            "usb-import",
            "eDB playlist removal skipped: missing playlist tables",
        );
        return Ok(0);
    }

    let ids = edb_playlist_ids_named(&tx, name_candidates)?;
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
                logging::debug("usb-import", format!("removed content row: {path}"));
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

/// The eDB's playlists (not folders) named one of `name_candidates`.
fn edb_playlist_ids_named(
    conn: &rusqlite::Connection,
    name_candidates: &[String],
) -> BackendResult<Vec<i64>> {
    let wanted = name_candidates
        .iter()
        .map(|n| canonicalize_playlist_name(n))
        .collect::<HashSet<_>>();
    let mut ids = Vec::<i64>::new();
    let mut stmt = conn.prepare("SELECT playlist_id, name FROM playlist WHERE attribute = 0")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    for (playlist_id, name) in rows.flatten() {
        if wanted.contains(&canonicalize_playlist_name(&name)) {
            ids.push(playlist_id);
        }
    }
    Ok(ids)
}

/// The tracks only the named playlist(s) hold, from the eDB's playlist and
/// history membership: what a stick without a PDB removes with them. Read
/// before [`remove_playlist_from_edb`] drops the membership. Artwork no
/// other track uses comes as `exclusive_artwork_paths`; `artwork_id` is 0.
pub fn edb_playlist_exclusive_tracks(
    session: &mut UsbWriteSession<'_>,
    name_candidates: &[String],
) -> BackendResult<PlaylistRemovalPdbResult> {
    let mut result = PlaylistRemovalPdbResult {
        removed_playlist_count: 0,
        exclusive_tracks: Vec::new(),
        exclusive_artwork_paths: Vec::new(),
        shared_track_count: 0,
    };
    let Some(conn) = session.edb()? else {
        return Ok(result);
    };
    if !["playlist", "playlist_content", "content"]
        .iter()
        .all(|table| table_exists(conn, table))
    {
        return Ok(result);
    }
    let ids = edb_playlist_ids_named(conn, name_candidates)?
        .into_iter()
        .collect::<HashSet<_>>();
    if ids.is_empty() {
        return Ok(result);
    }
    let mut target = HashSet::<i64>::new();
    let mut elsewhere = HashSet::<i64>::new();
    {
        let mut stmt = conn.prepare("SELECT playlist_id, content_id FROM playlist_content")?;
        let rows = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
        for (playlist_id, content_id) in rows.flatten() {
            if ids.contains(&playlist_id) {
                target.insert(content_id);
            } else {
                elsewhere.insert(content_id);
            }
        }
    }
    // History tracks stay, like the PDB's.
    if table_exists(conn, "history_content") {
        let mut stmt = conn.prepare("SELECT content_id FROM history_content")?;
        elsewhere.extend(stmt.query_map([], |row| row.get::<_, i64>(0))?.flatten());
    }
    let shared = target.intersection(&elsewhere).count();
    let exclusive = target
        .difference(&elsewhere)
        .copied()
        .collect::<HashSet<_>>();
    result.shared_track_count = shared;

    let image_fk = if crate::edb::content_column_or_null(conn, "imageFilePath_id") == "NULL" {
        "image_id"
    } else {
        "imageFilePath_id"
    };
    let has_images =
        table_exists(conn, "image") && crate::edb::content_column_or_null(conn, image_fk) != "NULL";
    let analysis = crate::edb::content_column_or_null(conn, "analysisDataFilePath");
    let image = if has_images {
        format!("c.{image_fk}")
    } else {
        "NULL".to_string()
    };
    let mut kept_images = HashSet::<i64>::new();
    let mut exclusive_images = HashSet::<i64>::new();
    {
        let mut stmt = conn.prepare(&format!(
            "SELECT c.content_id, c.path, {analysis}, {image} FROM content c"
        ))?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?;
        for (content_id, path, anlz_path, image_id) in rows.flatten() {
            let image_id = image_id.filter(|id| *id != 0);
            if !exclusive.contains(&content_id) {
                kept_images.extend(image_id);
                continue;
            }
            exclusive_images.extend(image_id);
            result.exclusive_tracks.push(ExclusiveTrackInfo {
                track_file_path: path.unwrap_or_default(),
                anlz_path: anlz_path.unwrap_or_default(),
                artwork_id: 0,
            });
        }
    }
    if has_images {
        let mut stmt = conn.prepare("SELECT path FROM image WHERE image_id = ?1")?;
        let mut exclusive_images = exclusive_images
            .difference(&kept_images)
            .copied()
            .collect::<Vec<_>>();
        exclusive_images.sort_unstable();
        for image_id in exclusive_images {
            if let Some(path) = stmt
                .query_row(params![image_id], |row| row.get::<_, Option<String>>(0))
                .optional()?
                .flatten()
                .filter(|path| !path.trim().is_empty())
            {
                result.exclusive_artwork_paths.push(path);
            }
        }
    }
    Ok(result)
}

/// [`remove_playlist_and_tracks_from_pdb_in`] in a write session of its own,
/// written back to the USB.
pub fn remove_playlist_and_tracks_from_pdb(
    usb_root: &Path,
    playlist_id_hint: Option<&str>,
    name_candidates: &[String],
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<PlaylistRemovalPdbResult> {
    let mut session = UsbWriteSession::without_service(usb_root);
    let result = remove_playlist_and_tracks_from_pdb_in(
        &mut session,
        playlist_id_hint,
        name_candidates,
        warnings,
    )?;
    session.commit()?;
    Ok(result)
}

/// Removes the playlist from the session's staged PDB, with the tracks no
/// other playlist or history holds.
pub fn remove_playlist_and_tracks_from_pdb_in(
    session: &mut UsbWriteSession<'_>,
    playlist_id_hint: Option<&str>,
    name_candidates: &[String],
    warnings: &mut Vec<WarningEntry>,
) -> BackendResult<PlaylistRemovalPdbResult> {
    let empty_result = PlaylistRemovalPdbResult {
        removed_playlist_count: 0,
        exclusive_tracks: Vec::new(),
        exclusive_artwork_paths: Vec::new(),
        shared_track_count: 0,
    };
    let Some(pdb_path) = session.databases()?.pdb().map(Path::to_path_buf) else {
        warnings.push(logging::log_text(
            Level::Warn,
            "usb-import",
            "usb.remove.pdb-missing",
            &tr::usb_playlists_warning_pdb_removal_skipped(),
        ));
        return Ok(empty_result);
    };
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

    session.write_pdb(&pdb_bytes)?;

    Ok(PlaylistRemovalPdbResult {
        removed_playlist_count: remove_ids.len(),
        exclusive_tracks,
        // The caller looks the artwork up in the PDB as written.
        exclusive_artwork_paths: Vec::new(),
        shared_track_count,
    })
}

/// What `remove_tracks_from_pdb` removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PdbTrackRemoval {
    pub playlist_entries: usize,
    pub track_rows: usize,
    pub artwork_rows: usize,
}

/// Remove tracks from the staged PDB in place: their playlist entries
/// (`tt=8`), their track rows (`tt=0`) and the artwork rows (`tt=13`) no
/// remaining track uses. Same tombstone path as the mirror-export cleanup
/// (`remove_rows_inplace` + `rebuild_sentinel_btrees_inplace`): row-presence
/// bits are cleared, page layout and table chains are untouched. History
/// entries are not touched; callers keep history tracks out of `track_ids`.
pub fn remove_tracks_from_pdb(
    usb_root: &Path,
    track_ids: &HashSet<u32>,
) -> BackendResult<PdbTrackRemoval> {
    use crate::pdb_writer::{
        extract_artwork_id, extract_playlist_entry_track_id, extract_track_id,
        rebuild_sentinel_btrees_inplace, remove_rows_inplace,
    };

    let mut result = PdbTrackRemoval::default();
    if track_ids.is_empty() {
        return Ok(result);
    }
    // The caller's write session writes the staged PDB back.
    let Some(pdb_path) = UsbDatabases::present(usb_root)?
        .pdb()
        .map(Path::to_path_buf)
    else {
        return Ok(result);
    };
    let parsed = parse_pdb(&pdb_path)?;
    let artwork_of = |keep: bool| {
        parsed
            .tracks
            .iter()
            .filter(|t| track_ids.contains(&t.id) != keep && t.artwork_id != 0)
            .map(|t| t.artwork_id)
            .collect::<HashSet<u32>>()
    };
    let exclusive_artwork_ids = artwork_of(false)
        .difference(&artwork_of(true))
        .copied()
        .collect::<HashSet<_>>();

    let mut pdb_bytes = std::fs::read(&pdb_path)?;
    result.playlist_entries = remove_rows_inplace(
        &mut pdb_bytes,
        8,
        track_ids,
        extract_playlist_entry_track_id,
    );
    result.track_rows = remove_rows_inplace(&mut pdb_bytes, 0, track_ids, extract_track_id);
    if !exclusive_artwork_ids.is_empty() {
        result.artwork_rows = remove_rows_inplace(
            &mut pdb_bytes,
            13,
            &exclusive_artwork_ids,
            extract_artwork_id,
        );
    }
    if result == PdbTrackRemoval::default() {
        return Ok(result);
    }
    if result.track_rows > 0 || result.artwork_rows > 0 {
        rebuild_sentinel_btrees_inplace(&mut pdb_bytes);
    }
    std::fs::write(&pdb_path, &pdb_bytes)?;
    Ok(result)
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
    session: &mut UsbWriteSession<'_>,
    prunable_media_paths: &[String],
) -> BackendResult<MirrorOrphanCleanup> {
    let usb_root = session.usb_root().to_path_buf();
    let usb_root = usb_root.as_path();
    let mut result = MirrorOrphanCleanup::default();

    let target_keys: HashSet<String> = prunable_media_paths
        .iter()
        .filter_map(|p| normalize_owned_export_path(usb_root, p))
        .filter(|p| p.starts_with("/Contents/"))
        .map(|p| canonicalize_track_path_identity(&p))
        .collect();
    if target_keys.is_empty() {
        return Ok(result);
    }

    let Some(pdb_path) = session.databases()?.pdb().map(Path::to_path_buf) else {
        return Ok(result);
    };
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

    let orphan_tracks: Vec<_> = parsed
        .tracks
        .iter()
        .filter(|t| !referenced_track_ids.contains(&t.id))
        .filter(|t| {
            normalize_owned_export_path(usb_root, &t.track_file_path)
                .map(|norm| target_keys.contains(&canonicalize_track_path_identity(&norm)))
                .unwrap_or(false)
        })
        .collect();
    if orphan_tracks.is_empty() {
        return Ok(result);
    }
    let orphan_track_ids: HashSet<u32> = orphan_tracks.iter().map(|t| t.id).collect();
    // The eDB rows of those tracks: the ones with their content fingerprint
    // (size, title, artist), or, for a track without one, at its exact path.
    let mut orphan_fingerprints = HashSet::<ContentFingerprint>::new();
    let mut orphan_path_keys = HashSet::<String>::new();
    for track in &orphan_tracks {
        match pdb_track_fingerprint(&parsed, track) {
            Some(fingerprint) => {
                orphan_fingerprints.insert(fingerprint);
            }
            None => {
                orphan_path_keys.insert(canonicalize_track_path_identity(&track.track_file_path));
            }
        }
    }

    // --- eDB: delete content rows for those paths, then orphaned image rows ---
    if let Some(conn) = session.edb()?
        && table_exists(conn, "content")
    {
        let to_delete = crate::service::usb_track_facts::read_edb_content_tracks(conn)?
            .into_iter()
            .filter(|row| match &row.fingerprint {
                Some(fingerprint) => orphan_fingerprints.contains(fingerprint),
                None => orphan_path_keys.contains(&canonicalize_track_path_identity(&row.path)),
            })
            .filter_map(|row| row.content_id)
            .collect::<Vec<i64>>();
        let has_playlist_content = table_exists(conn, "playlist_content");
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
        if result.removed_edb_content > 0 && table_exists(conn, "image") {
            let _ = conn.execute(
                "DELETE FROM image WHERE image_id NOT IN (SELECT DISTINCT image_id FROM content WHERE image_id IS NOT NULL)",
                [],
            );
        }
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
        session.write_pdb(&pdb_bytes)?;
    }

    if result != MirrorOrphanCleanup::default() {
        logging::debug(
            "export",
            format!(
                "mirror export removed {} from eDB and {} ({}) from PDB — no other playlist or history references them",
                counted(
                    result.removed_edb_content,
                    "dropped-track content row",
                    "dropped-track content rows"
                ),
                counted(result.removed_pdb_tracks, "track row", "track rows"),
                counted(result.removed_pdb_artwork, "artwork row", "artwork rows")
            ),
        );
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
    let Some(pdb_path) = UsbDatabases::of(usb_root)?.pdb().map(Path::to_path_buf) else {
        return Err(BackendError::Internal(
            "export verification failed: the USB has no PDB".to_string(),
        ));
    };
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
        .map(|t| (t.id, canonicalize_track_path_identity(&t.track_file_path)))
        .collect::<HashMap<_, _>>();
    let expected_paths = manifest
        .tracks
        .iter()
        .map(|t| canonicalize_track_path_identity(&t.exported_path))
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
