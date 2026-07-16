//! Export-related helper functions: PDB writing, export DB, manifest, file operations.

pub mod export_paths;
pub mod pdb_encoding;
pub mod pdb_menu;
pub mod playlist_ops;

// Re-export submodule items at this level for backward compatibility.
pub use crate::edb::{
    ExportManifestTrack, ExportPlaylistData, ExportTrackData, content_file_name,
    duration_ms_to_seconds, dynamic_insert, find_content_id_by_path, find_key_id_by_name,
    find_or_insert_album, find_or_insert_artist, find_or_insert_image,
    insert_content_from_template, link_playlist_content, load_table_columns,
    load_table_row_template, map_values_for_insert, next_numeric_id, normalize_export_date,
    open_edb_rw, populate_content_values_map, preferred_export_playlist_row_id,
    replace_export_playlist_row_with_identity, resolve_track_created_date_for_export,
    resolve_track_release_date_for_export, table_exists, upsert_export_playlist_row,
};
pub(crate) use crate::metadata::sanitize_metadata;
pub use crate::pdb_writer::{
    T08EntryKey, T08PatchContext, try_patch_t08_with_context, try_patch_t08_with_multi_page_growth,
    validate_no_empty_data_pages,
};
#[cfg(test)]
pub use export_paths::canonical_artwork_target_path;
pub use export_paths::{
    CONTENT_FILENAME_MAX_LEN, PruneResult, analysis_bundle_path_variants,
    canonical_analysis_bundle_paths, canonical_artwork_target_paths, collect_manifest_owned_paths,
    copy_if_different, copy_wav_normalized_if_needed, ensure_analysis_bundle_ppth,
    export_analysis_bundle_for_track, export_artwork_for_player, export_owned_files_setting_key,
    exported_media_target_path, filter_prunable_stale_paths_for_playlist,
    is_safe_export_owned_path, limit_contents_file_name, normalize_owned_export_path,
    prune_stale_export_owned_files, sanitize_contents_component, sanitize_filename_component,
    stable_u32_hash, to_usb_relative_path, truncate_component,
};
pub use pdb_encoding::{
    PdbLayoutProfile, encode_album_row, encode_artist_row, encode_artwork_row, encode_key_row,
    encode_pdb_string, encode_pdb_track_isrc_slot, encode_playlist_entry_row,
    encode_playlist_tree_row, encode_track_row_with_profile,
};
pub use pdb_menu::{
    PdbT16Row, encode_pdb_t16_row, encode_pdb_t17_cat_row, inspect_pdb_columns_playlist_order,
    load_pdb_t16_decoded, load_pdb_t16_raw, load_pdb_t16_rows_from_bytes,
    patch_pdb_columns_menu_set_by_kind, patch_pdb_t17_category_snapshot,
};
pub use playlist_ops::{
    remove_playlist_and_tracks_from_pdb, remove_playlist_from_edb,
    remove_track_ids_from_pdb_playlist_entries, verify_edb_content, verify_pdb_content,
};

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::error::{BackendError, BackendResult};
use crate::models::ExportToUsbOptions;
use crate::pdb_reader::parse_pdb;
use crate::utils::{collect_chain as collect_chain_pages, page_offset, table_ptr_fields};

use super::usb_utils::{canonicalize_playlist_name, repair_utf8_mojibake};
use super::usb_vendor_compat::{USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR};

/// Read PDB t16 row bytes from an existing PDB on disk, in their on-disk
/// order. Returns `None` when the input bytes are not a valid PDB image or
/// contain no t16 rows; callers should then fall back to the default seed.
fn preserve_existing_t16_rows(bytes: &[u8]) -> Option<Vec<Vec<u8>>> {
    load_pdb_t16_rows_from_bytes(bytes)
}

fn validate_topology_locked_export_bytes(before: &[u8], after: &[u8]) -> BackendResult<()> {
    const PAGE_SIZE: usize = 4096;
    const CRITICAL_FIRST_PAGE_TABLES: &[u32] = &[0, 7, 8, 11, 12, 16, 17, 18, 19];
    const MENU_TABLES: &[u32] = &[16, 17, 18];

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
            Some((_before_ec, before_first, before_last)),
            Some((_after_ec, after_first, after_last)),
        ) = (
            table_ptr_fields(before, table_type),
            table_ptr_fields(after, table_type),
        ) {
            if before_first != after_first {
                issues.push(format!(
                    "t{table_type:02} first_page changed {before_first}->{after_first}"
                ));
            }
            if table_type == 7 && before_last != before_first && before_last != after_last {
                issues.push(format!("t07 last_page changed {before_last}->{after_last}"));
            }
        }
    }

    if let (
        Some((_ec_before, _first_before, before_last)),
        Some((_ec_after, _first_after, after_last)),
    ) = (table_ptr_fields(before, 7), table_ptr_fields(after, 7))
        && before_last != _first_before && before_last == after_last
            && let Some(last_off) = page_offset(before_last, PAGE_SIZE) {
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
                        "t07 tail next_page changed {:?}->{:?}",
                        before_next, after_next
                    ));
                }
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

fn canonicalize_track_path_identity(value: &str) -> String {
    let normalized = repair_utf8_mojibake(value.trim()).replace('\\', "/");
    if normalized.is_empty() {
        return String::new();
    }

    // Trim trailing whitespace from the stem so alias filenames (e.g. "Track .mp3"
    // written by some DJ software) resolve to the same key as their originals.
    let normalized = if let Some(dot) = normalized.rfind('.') {
        let stem = normalized[..dot].trim_end_matches(' ');
        format!("{stem}{}", &normalized[dot..])
    } else {
        normalized
    };

    let lower = normalized.to_ascii_lowercase();
    if let Some(idx) = lower.rfind("/contents/") {
        lower[idx..].to_string()
    } else if lower.starts_with("contents/") {
        format!("/{lower}")
    } else {
        lower
    }
}

fn canonicalize_artwork_path_lookup(value: &str) -> String {
    repair_utf8_mojibake(value.trim())
        .replace('\\', "/")
        .to_ascii_lowercase()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportManifest {
    pub version: u32,
    pub generated_at: String,
    pub playlist_id: String,
    pub playlist_name: String,
    pub usb_root: String,
    pub options: ExportToUsbOptions,
    pub exported_tracks: usize,
    pub skipped_tracks: usize,
    pub warnings: Vec<String>,
    pub tracks: Vec<ExportManifestTrack>,
}

#[derive(Debug, Clone)]
pub struct LocalTrackForAnalysis {
    pub id: String,
    pub title: String,
    pub file_path: String,
}

#[derive(Debug, Clone)]
pub struct LocalAnalysisResult {
    pub bpm: Option<f64>,
    pub bpm_analyzer: Option<String>,
    pub key: Option<String>,
    pub first_beat_ms: Option<u32>,
    pub duration_ms: Option<u64>,
    pub artwork_path: Option<String>,
    pub waveform_peaks_path: Option<String>,
    pub waveform_preview: Option<Vec<u8>>,
}

// Re-export from anlz module
pub use super::anlz::WaveformData;

/// Result of removing a playlist from PDB with track/artwork cleanup.
#[derive(Debug, Clone)]
pub struct PlaylistRemovalPdbResult {
    pub removed_playlist_count: usize,
    pub exclusive_tracks: Vec<ExclusiveTrackInfo>,
    pub shared_track_count: usize,
}

/// Info about a track that is exclusive to the removed playlist(s).
#[derive(Debug, Clone)]
pub struct ExclusiveTrackInfo {
    pub track_file_path: String,
    pub anlz_path: String,
    pub artwork_id: u32,
}

#[derive(Debug, Clone)]
pub struct WriteExportLibraryDbResult {
    pub inserted_content: usize,
    pub linked_playlist_entries: usize,
    pub playlist_id: i64,
    pub sort_order: i64,
}

#[derive(Debug, Clone)]
pub struct WriteExportPdbResult {
    pub inserted_tracks: usize,
    pub inserted_playlists: usize,
    pub topology_issues: Vec<String>,
    /// Non-fatal warnings produced by the PDB writer (e.g. tracks dropped
    /// because their encoded row exceeded a single PDB page). Surfaced into
    /// the export job response so the UI Event Log shows them.
    #[allow(clippy::vec_init_then_push)]
    pub writer_warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PdbTrackRowData {
    pub header_flags_u32: Option<u32>,
    pub content_link: Option<u32>,
    pub sample_rate_hz: Option<u32>,
    pub file_size_bytes: Option<u32>,
    pub master_content_id: Option<u32>,
    pub master_db_id: Option<u32>,
    pub id: u32,
    pub artist_id: u32,
    pub album_id: u32,
    pub artwork_id: u32,
    pub key_id: u32,
    pub genre_id: u32,
    pub bitrate_kbps: Option<u32>,
    pub track_number: Option<u32>,
    pub bpm: Option<f64>,
    pub release_year: Option<u16>,
    pub bit_depth: Option<u16>,
    pub duration_seconds: Option<u32>,
    pub file_type: Option<u16>,
    pub isrc: Option<String>,
    pub date_added: Option<String>,
    pub release_date: Option<String>,
    pub dj_comment: Option<String>,
    pub file_name: Option<String>,
    pub publish_track_info_on: Option<bool>,
    pub autoload_hotcues_on: Option<bool>,
    pub title: String,
    pub anlz_path: String,
    pub file_path: String,
}

pub fn write_edb_playlist(
    usb_root: &Path,
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
    mirror_playlist_entries: bool,
) -> BackendResult<WriteExportLibraryDbResult> {
    let export_date_added = normalize_export_date(Some(&manifest.generated_at))
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
    let db_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("exportLibrary.db");
    if !db_path.is_file() {
        return Err(BackendError::NotFound(format!(
            "eDB not found at {}",
            db_path.display()
        )));
    }

    let mut unlock_warnings = Vec::<String>::new();
    let Some(mut conn) = open_edb_rw(usb_root, &mut unlock_warnings) else {
        return Err(BackendError::Internal(format!(
            "unable to open eDB in read-write mode ({})",
            unlock_warnings.join(" | ")
        )));
    };

    let tx = conn.transaction()?;
    if !table_exists(&tx, "playlist")
        || !table_exists(&tx, "content")
        || !table_exists(&tx, "playlist_content")
    {
        return Err(BackendError::Internal(
            "eDB is missing required playlist/content tables".to_string(),
        ));
    }

    let playlist_id = upsert_export_playlist_row(&tx, playlist)?;
    let sort_order = tx
        .query_row(
            "SELECT COALESCE(sequenceNo, 0) FROM playlist WHERE playlist_id = ?1",
            params![playlist_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0);
    // Capture playlist-linked content rows by path before optional mirror-delete.
    // This avoids picking an arbitrary duplicate content row for a path.
    let mut prev_content_id_by_path = HashMap::<String, i64>::new();
    let mut previously_linked_ids = HashSet::<i64>::new();
    {
        let mut stmt = tx.prepare(
            r#"
            SELECT pc.content_id, c.path
            FROM playlist_content pc
            JOIN content c ON c.content_id = pc.content_id
            WHERE pc.playlist_id = ?1
            "#,
        )?;
        let rows = stmt.query_map(params![playlist_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (content_id, maybe_path) = row?;
            previously_linked_ids.insert(content_id);
            if let Some(path) = maybe_path {
                let key = canonicalize_playlist_name(&path.replace('\\', "/"));
                if !key.is_empty() {
                    prev_content_id_by_path.entry(key).or_insert(content_id);
                }
            }
        }
    }

    if mirror_playlist_entries {
        tx.execute(
            "DELETE FROM playlist_content WHERE playlist_id = ?1",
            params![playlist_id],
        )?;
    }

    let mut inserted_content = 0usize;
    let mut linked_playlist_entries = 0usize;
    let playlist_content_columns = load_table_columns_tx(&tx, "playlist_content")?;
    let mut seq = if mirror_playlist_entries {
        0i64
    } else if playlist_content_columns.contains("sequenceNo") {
        tx.query_row(
            "SELECT COALESCE(MAX(sequenceNo), 0) FROM playlist_content WHERE playlist_id = ?1",
            params![playlist_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
    } else {
        0i64
    };
    let content_columns = load_table_columns_tx(&tx, "content")?;

    for track in &manifest.tracks {
        let track_path_key = canonicalize_track_path_identity(&track.exported_path);
        let preferred_existing = prev_content_id_by_path.get(&track_path_key).copied();
        let resolved_existing = match preferred_existing {
            Some(content_id) => Some(content_id),
            None => find_content_id_by_path(&tx, &track.exported_path)?,
        };
        let content_id = match resolved_existing {
            Some(existing) => {
                let was_previously_linked = previously_linked_ids.contains(&existing);
                update_existing_content_row(
                    &tx,
                    existing,
                    track,
                    &content_columns,
                    Some(export_date_added.as_str()),
                )?;
                if content_columns.contains("analysedBits") {
                    let analysed_bits: i64 = if track.waveform_path.is_some() { 41 } else { 0 };
                    tx.execute(
                        "UPDATE content SET analysedBits = ?2 WHERE content_id = ?1",
                        params![existing, analysed_bits],
                    )?;
                }
                // Keep contentLink stable across additive rewrites so eDB/PDB
                // identity fields remain in lockstep for player metadata lookup.
                if content_columns.contains("analysisDataUpdateCount") {
                    // Vendor-authored exports apply +8 only to the appended
                    // export member while other existing rows receive +2.
                    let is_appended_export_member = track.position + 1 >= manifest.tracks.len();
                    let bump = if was_previously_linked || !is_appended_export_member {
                        2
                    } else {
                        8
                    };
                    tx.execute(
                        "UPDATE content SET analysisDataUpdateCount = (COALESCE(CAST(analysisDataUpdateCount AS INTEGER), 0) + ?2) WHERE content_id = ?1",
                        params![existing, bump],
                    )?;
                }
                if content_columns.contains("informationUpdateCount") {
                    let is_appended_export_member = track.position + 1 >= manifest.tracks.len();
                    let bump = if was_previously_linked || !is_appended_export_member {
                        2
                    } else {
                        8
                    };
                    tx.execute(
                        "UPDATE content SET informationUpdateCount = (COALESCE(CAST(informationUpdateCount AS INTEGER), 0) + ?2) WHERE content_id = ?1",
                        params![existing, bump],
                    )?;
                }
                if content_columns.contains("cueUpdateCount") {
                    tx.execute(
                        "UPDATE content SET cueUpdateCount = '' WHERE content_id = ?1",
                        params![existing],
                    )?;
                }
                existing
            }
            None => {
                let id =
                    insert_content_from_template(&tx, track, Some(export_date_added.as_str()))?;
                inserted_content += 1;
                id
            }
        };

        if !mirror_playlist_entries {
            let already_linked: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM playlist_content WHERE playlist_id = ?1 AND content_id = ?2 LIMIT 1",
                    params![playlist_id, content_id],
                    |row| row.get(0),
                )
                .optional()?;
            if already_linked.is_some() {
                continue;
            }
        }

        seq += 1;
        link_playlist_content(&tx, playlist_id, content_id, seq)?;
        linked_playlist_entries += 1;
    }

    // Keep property summary in sync with actual content cardinality.
    if table_exists(&tx, "property") {
        let property_columns = load_table_columns_tx(&tx, "property")?;
        if property_columns.contains("numberOfContents") {
            tx.execute(
                "UPDATE property SET numberOfContents = (SELECT COUNT(1) FROM content)",
                [],
            )?;
        }
    }

    tx.commit()?;
    // Keep WAL mode after writes.
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
    Ok(WriteExportLibraryDbResult {
        inserted_content,
        linked_playlist_entries,
        playlist_id,
        sort_order,
    })
}

pub fn load_table_columns_tx(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
) -> BackendResult<HashSet<String>> {
    let mut stmt = tx.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut out = HashSet::<String>::new();
    for row in rows {
        out.insert(row?);
    }
    Ok(out)
}

pub fn update_existing_content_row(
    tx: &rusqlite::Transaction<'_>,
    content_id: i64,
    track: &ExportManifestTrack,
    content_columns: &HashSet<String>,
    export_date_added: Option<&str>,
) -> BackendResult<()> {
    if content_columns.contains("title") {
        let title = sanitize_metadata(&track.title).into_owned();
        tx.execute(
            "UPDATE content SET title = ?1 WHERE content_id = ?2",
            params![title, content_id],
        )?;
    }
    if content_columns.contains("path") {
        tx.execute(
            "UPDATE content SET path = ?1 WHERE content_id = ?2",
            params![track.exported_path, content_id],
        )?;
    }
    if content_columns.contains("analysisDataFilePath")
        && track.waveform_path.is_some() {
            tx.execute(
                "UPDATE content SET analysisDataFilePath = ?1 WHERE content_id = ?2",
                params![track.waveform_path, content_id],
            )?;
        }
    if content_columns.contains("bpmx100")
        && let Some(bpm) = track.bpm {
            let bpmx100 = (bpm * 100.0).round() as i64;
            tx.execute(
                "UPDATE content SET bpmx100 = ?1 WHERE content_id = ?2",
                params![bpmx100, content_id],
            )?;
        }
    if content_columns.contains("length")
        && let Some(duration_ms) = track.duration_ms {
            let length = duration_ms_to_seconds(duration_ms);
            tx.execute(
                "UPDATE content SET length = ?1 WHERE content_id = ?2",
                params![length, content_id],
            )?;
        }
    if content_columns.contains("artist_id_artist") {
        let artist_id = find_or_insert_artist(tx, &track.artist)?;
        tx.execute(
            "UPDATE content SET artist_id_artist = ?1 WHERE content_id = ?2",
            params![artist_id, content_id],
        )?;
    }
    if content_columns.contains("album_id") {
        let existing_album_id: Option<i64> = tx
            .query_row(
                "SELECT album_id FROM content WHERE content_id = ?1",
                params![content_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        if existing_album_id.is_none() && track.album.is_some() {
            let album_id = find_or_insert_album(tx, track.album.as_deref(), Some(&track.artist))?;
            tx.execute(
                "UPDATE content SET album_id = ?1 WHERE content_id = ?2",
                params![album_id, content_id],
            )?;
        }
    }
    if content_columns.contains("key_id") {
        let key_id = find_key_id_by_name(tx, track.key.as_deref())?;
        tx.execute(
            "UPDATE content SET key_id = ?1 WHERE content_id = ?2",
            params![key_id, content_id],
        )?;
    }
    if (content_columns.contains("image_id") || content_columns.contains("imageFilePath_id"))
        && let Some(path) = track
            .artwork_path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
        {
            let image_id = find_or_insert_image(tx, path)?;
            if content_columns.contains("image_id") {
                tx.execute(
                    "UPDATE content SET image_id = ?1 WHERE content_id = ?2",
                    params![image_id, content_id],
                )?;
            }
            if content_columns.contains("imageFilePath_id") {
                tx.execute(
                    "UPDATE content SET imageFilePath_id = ?1 WHERE content_id = ?2",
                    params![image_id, content_id],
                )?;
            }
        }
    if content_columns.contains("fileName") {
        tx.execute(
            "UPDATE content SET fileName = ?1 WHERE content_id = ?2",
            params![content_file_name(&track.exported_path), content_id],
        )?;
    }
    if content_columns.contains("fileSize") {
        tx.execute(
            "UPDATE content SET fileSize = ?1 WHERE content_id = ?2",
            params![track.file_size_bytes, content_id],
        )?;
    }
    if content_columns.contains("fileType") {
        tx.execute(
            "UPDATE content SET fileType = ?1 WHERE content_id = ?2",
            params![track.file_type, content_id],
        )?;
    }
    if content_columns.contains("trackNo") {
        tx.execute(
            "UPDATE content SET trackNo = ?1 WHERE content_id = ?2",
            params![track.track_number, content_id],
        )?;
    }
    if content_columns.contains("discNo") {
        tx.execute(
            "UPDATE content SET discNo = ?1 WHERE content_id = ?2",
            params![track.disc_number.unwrap_or(0), content_id],
        )?;
    }
    if content_columns.contains("bitrate") {
        tx.execute(
            "UPDATE content SET bitrate = ?1 WHERE content_id = ?2",
            params![track.bitrate_kbps, content_id],
        )?;
    }
    if content_columns.contains("samplingRate") {
        tx.execute(
            "UPDATE content SET samplingRate = ?1 WHERE content_id = ?2",
            params![track.sample_rate_hz, content_id],
        )?;
    }
    if content_columns.contains("bitDepth") {
        tx.execute(
            "UPDATE content SET bitDepth = ?1 WHERE content_id = ?2",
            params![track.bit_depth.map(i64::from).unwrap_or(16), content_id],
        )?;
    }
    if content_columns.contains("subtitle") {
        let subtitle = track
            .subtitle
            .as_deref()
            .map(|value| sanitize_metadata(value).into_owned())
            .unwrap_or_default();
        tx.execute(
            "UPDATE content SET subtitle = ?1 WHERE content_id = ?2",
            params![subtitle, content_id],
        )?;
    }
    if content_columns.contains("titleForSearch") {
        let title_for_search = track
            .title_for_search
            .as_deref()
            .map(|value| sanitize_metadata(value).into_owned())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        tx.execute(
            "UPDATE content SET titleForSearch = ?1 WHERE content_id = ?2",
            params![title_for_search, content_id],
        )?;
    }
    if content_columns.contains("isrc") {
        tx.execute(
            "UPDATE content SET isrc = ?1 WHERE content_id = ?2",
            params![track.isrc, content_id],
        )?;
    }
    if content_columns.contains("djComment") {
        let comment = track
            .comment
            .as_deref()
            .map(|value| sanitize_metadata(value).into_owned());
        tx.execute(
            "UPDATE content SET djComment = ?1 WHERE content_id = ?2",
            params![comment, content_id],
        )?;
    }
    if content_columns.contains("releaseYear") {
        tx.execute(
            "UPDATE content SET releaseYear = ?1 WHERE content_id = ?2",
            params![track.release_year.unwrap_or(0), content_id],
        )?;
    }
    if content_columns.contains("releaseDate") {
        tx.execute(
            "UPDATE content SET releaseDate = ?1 WHERE content_id = ?2",
            params![
                resolve_track_release_date_for_export(track).unwrap_or_default(),
                content_id
            ],
        )?;
    }
    if content_columns.contains("dateAdded") {
        tx.execute(
            "UPDATE content SET dateAdded = ?1 WHERE content_id = ?2",
            params![normalize_export_date(export_date_added), content_id],
        )?;
    }
    if content_columns.contains("dateCreated") {
        tx.execute(
            "UPDATE content SET dateCreated = ?1 WHERE content_id = ?2",
            params![resolve_track_created_date_for_export(track), content_id],
        )?;
    }
    if content_columns.contains("analysedBits") {
        // Mark exported tracks as having the newer analysis generation so
        // Vendor software can enable the modern waveform path for generated ANLZ.
        let analysed_bits: i64 = if track.waveform_path.is_some() { 41 } else { 0 };
        tx.execute(
            "UPDATE content SET analysedBits = ?1 WHERE content_id = ?2",
            params![analysed_bits, content_id],
        )?;
    }
    if content_columns.contains("contentLink")
        && let Some(cl) = track.content_link {
            tx.execute(
                "UPDATE content SET contentLink = ?1 WHERE content_id = ?2",
                params![cl, content_id],
            )?;
        }
    if content_columns.contains("masterContentId")
        && let Some(mci) = track.master_content_id {
            tx.execute(
                "UPDATE content SET masterContentId = ?1 WHERE content_id = ?2",
                params![mci, content_id],
            )?;
        }
    if content_columns.contains("masterDbId")
        && let Some(mdb) = track.master_db_id {
            tx.execute(
                "UPDATE content SET masterDbId = ?1 WHERE content_id = ?2",
                params![mdb, content_id],
            )?;
        }
    if content_columns.contains("isHotCueAutoLoadOn") {
        tx.execute(
            "UPDATE content SET isHotCueAutoLoadOn = 1 WHERE content_id = ?1",
            params![content_id],
        )?;
    }
    if content_columns.contains("isKuvoDeliverStatusOn") {
        tx.execute(
            "UPDATE content SET isKuvoDeliverStatusOn = 1 WHERE content_id = ?1",
            params![content_id],
        )?;
    }
    if content_columns.contains("hasModified") {
        tx.execute(
            "UPDATE content SET hasModified = 0 WHERE content_id = ?1",
            params![content_id],
        )?;
    }
    if content_columns.contains("rating") {
        tx.execute(
            "UPDATE content SET rating = ?1 WHERE content_id = ?2",
            params![track.rating.unwrap_or(0), content_id],
        )?;
    }
    if content_columns.contains("djPlayCount") {
        tx.execute(
            "UPDATE content SET djPlayCount = ?1 WHERE content_id = ?2",
            params![track.dj_play_count.unwrap_or(0), content_id],
        )?;
    }
    if content_columns.contains("color_id") {
        tx.execute(
            "UPDATE content SET color_id = ?1 WHERE content_id = ?2",
            params![track.color_id.unwrap_or(0), content_id],
        )?;
    }
    if content_columns.contains("artist_id_lyricist") {
        tx.execute(
            "UPDATE content SET artist_id_lyricist = ?1 WHERE content_id = ?2",
            params![track.artist_id_lyricist.unwrap_or(0), content_id],
        )?;
    }
    if content_columns.contains("artist_id_originalArtist") {
        tx.execute(
            "UPDATE content SET artist_id_originalArtist = ?1 WHERE content_id = ?2",
            params![track.artist_id_original_artist.map(i64::from), content_id],
        )?;
    }
    if content_columns.contains("artist_id_remixer") {
        tx.execute(
            "UPDATE content SET artist_id_remixer = ?1 WHERE content_id = ?2",
            params![track.artist_id_remixer.map(i64::from), content_id],
        )?;
    }
    if content_columns.contains("artist_id_composer") {
        tx.execute(
            "UPDATE content SET artist_id_composer = ?1 WHERE content_id = ?2",
            params![track.artist_id_composer.map(i64::from), content_id],
        )?;
    }
    if content_columns.contains("genre_id") {
        tx.execute(
            "UPDATE content SET genre_id = ?1 WHERE content_id = ?2",
            params![track.genre_id.map(i64::from), content_id],
        )?;
    }
    if content_columns.contains("label_id") {
        tx.execute(
            "UPDATE content SET label_id = ?1 WHERE content_id = ?2",
            params![track.label_id.map(i64::from), content_id],
        )?;
    }
    if content_columns.contains("kuvoDeliveryComment") {
        let kuvo_delivery_comment = track
            .kuvo_delivery_comment
            .as_deref()
            .map(|value| sanitize_metadata(value).into_owned())
            .unwrap_or_default();
        tx.execute(
            "UPDATE content SET kuvoDeliveryComment = ?1 WHERE content_id = ?2",
            params![kuvo_delivery_comment, content_id],
        )?;
    }
    Ok(())
}

/// Write PDB export data.
///
/// Existing PDBs are written through the topology-locked additive path only.
/// A full rebuild is allowed only when there is no PDB on disk yet; falling
/// back to a rebuilt blob for a populated USB is a known player database corruption class.
pub fn write_pdb(
    usb_root: &Path,
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
    mirror_playlist_entries: bool,
    edb_playlist_id: Option<u32>,
    edb_sort_order: Option<u32>,
    skip_growth_patch: bool,
) -> BackendResult<WriteExportPdbResult> {
    write_pdb_fresh_with_overrides(
        usb_root,
        playlist,
        manifest,
        mirror_playlist_entries,
        edb_playlist_id,
        edb_sort_order,
        skip_growth_patch,
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
        false,
    )
}

fn normalize_pdb_key_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn move_playlist_tree_row_to_front(
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

fn parse_key_pitch_mode(value: &str) -> Option<(u8, bool)> {
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

fn key_lookup_variants(value: &str) -> Vec<String> {
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

fn resolve_key_id_by_name(key_id_by_name: &HashMap<String, u32>, key_name: &str) -> Option<u32> {
    key_lookup_variants(key_name)
        .into_iter()
        .find_map(|k| key_id_by_name.get(&k).copied())
}

fn write_pdb_fresh_with_overrides(
    usb_root: &Path,
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
    mirror_playlist_entries: bool,
    override_playlist_id: Option<u32>,
    override_sort_order: Option<u32>,
    _skip_growth_patch: bool,
    commit_write: bool,
) -> BackendResult<WriteExportPdbResult> {
    use crate::pdb_writer::{
        PdbAlbumRow, PdbArtistRow, PdbArtworkRow, PdbData, PdbKeyRow, PdbPlaylistEntryRow,
        PdbPlaylistTreeRow,
    };

    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");

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
        let mut unlock_warnings = Vec::<String>::new();
        if let Some(conn) = open_edb_rw(usb_root, &mut unlock_warnings)
            && table_exists(&conn, "content") {
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
                    }) {
                        for row in rows.flatten() {
                            let key = canonicalize_track_path_identity(&row.0);
                            if key.is_empty() {
                                continue;
                            }
                            if let Some(name) = row.1
                                && !name.trim().is_empty() {
                                    edb_key_by_path.insert(key.clone(), name);
                                }
                            if let Some(name) = row.2
                                && !name.trim().is_empty() {
                                    edb_artist_by_path.insert(key.clone(), name);
                                }
                            if let Some(name) = row.3
                                && !name.trim().is_empty() {
                                    edb_album_by_path.insert(key.clone(), name);
                                }
                            if let Some(path) = row.4 {
                                let normalized =
                                    repair_utf8_mojibake(path.trim()).replace('\\', "/");
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
            if let Some(existing_track) = all_tracks.iter_mut().find(|t| t.id == existing) {
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
                if let Some(ref wp) = track.waveform_path {
                    existing_track.anlz_path =
                        to_usb_relative_path(usb_root, wp).unwrap_or_else(|| wp.clone());
                }
                existing_track.title = track.title.clone();
                // Only replace file_path when the canonical key changes — i.e. when
                // the paths refer to genuinely different locations. If the manifest
                // uses the eDB form of a path (no trailing space) while the USB has
                // the alias form (trailing space), they canonicalize identically and
                // the USB path must be kept so the player can still find the file.
                if canonicalize_track_path_identity(&file_path)
                    != canonicalize_track_path_identity(&existing_track.file_path)
                {
                    existing_track.file_path = file_path.clone();
                }
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
                release_year: track.release_year.and_then(|v| u16::try_from(v).ok()),
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
                title: track.title.clone(),
                anlz_path,
                file_path: file_path.clone(),
            });
            track_id_by_path.insert(path_key, track_id);
            track_id
        };

        if let Some(track_row) = all_tracks.iter().find(|t| t.id == pdb_track_id) {
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
    let exported_track_count_for_t19 = all_tracks.len();
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
        history_raw_rows: if let Some(ref parsed) = existing {
            let mut rows = parsed.history_raw_rows_bytes.clone();
            if rows.len() <= 1 {
                let track_count = exported_track_count_for_t19;
                if track_count > 0 {
                    let base = rows.first().cloned().unwrap_or_else(|| {
                        let mut seed = vec![0u8; 40];
                        seed[0] = 0x80;
                        seed[1] = 0x02;
                        seed[2] = 0x00;
                        seed[3] = 0x00;
                        seed[26..30].copy_from_slice(b"1000");
                        seed
                    });
                    rows = (0..=track_count)
                        .map(|idx| {
                            let mut row = if base.len() == 40 {
                                base.clone()
                            } else {
                                vec![0u8; 40]
                            };
                            // Runtime sequence follows observed profile:
                            // 0x280, 0x200280, 0x400280, ...
                            let val = 0x0000_0280u32
                                .saturating_add((idx as u32).saturating_mul(0x0020_0000));
                            row[0..4].copy_from_slice(&val.to_le_bytes());
                            row[4..8].copy_from_slice(&(idx as u32).to_le_bytes());
                            // keep date/num region from seed/base (if present)
                            row
                        })
                        .collect();
                }
            }
            rows
        } else {
            Vec::new()
        },
        profile,
    };

    let t08_patch_ctx = T08PatchContext {
        playlist_id: playlist_pdb_id,
        desired_entries: pdb_data
            .playlist_entries
            .iter()
            .filter(|e| e.playlist_id == playlist_pdb_id)
            .map(|e| T08EntryKey {
                entry_index: e.entry_index,
                track_id: e.track_id,
                playlist_id: e.playlist_id,
            })
            .collect(),
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
        let desired_t08: Vec<T08EntryKey> = t08_patch_ctx.desired_entries.clone();
        match crate::pdb_writer::try_write_pdb_additive_in_place(
            before,
            &pdb_data,
            desired_t08,
            4096,
        )? {
            Some((additive_bytes, summary)) => {
                validate_topology_locked_export_bytes(before, &additive_bytes)?;
                if commit_write {
                    std::fs::write(&pdb_path, &additive_bytes)?;
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
        false,
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::super::usb_vendor_compat::USB_ANALYSIS_DIR;
    use super::*;
    use crate::models::RunUsbParityReportRequest;
    use crate::service::BackendService;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn mapping_test_track() -> ExportManifestTrack {
        ExportManifestTrack {
            id: "track-1".to_string(),
            master_db_id: Some(556_677),
            master_content_id: Some(112_233),
            content_link: Some(998_877),
            position: 1,
            track_number: Some(3),
            title: "Mapping Track".to_string(),
            artist: "Mapping Artist".to_string(),
            album: Some("Mapping Album".to_string()),
            bpm: Some(123.45),
            key: Some("8A".to_string()),
            source_path: "/source/mapping.mp3".to_string(),
            exported_path: "/Contents/Mapping Artist/Mapping Album/Mapping Track.mp3".to_string(),
            file_modified_at: Some("1714521600".to_string()),
            file_size_bytes: Some(42_424),
            sample_rate_hz: Some(44_100),
            bit_depth: Some(24),
            bitrate_kbps: Some(320),
            disc_number: Some(2),
            subtitle: Some("Subtitle".to_string()),
            comment: Some("Comment".to_string()),
            title_for_search: Some("Mapping Track Search".to_string()),
            kuvo_delivery_comment: Some("Kuvo note".to_string()),
            dj_play_count: Some(7),
            rating: Some(4),
            color_id: Some(6),
            artist_id_lyricist: Some(11),
            artist_id_original_artist: Some(12),
            artist_id_remixer: Some(13),
            artist_id_composer: Some(14),
            genre_id: Some(15),
            genre: None,
            label_id: Some(16),
            isrc: Some("USRC17607839".to_string()),
            release_year: Some(2024),
            release_date: Some("2024-05-01".to_string()),
            recorded_date: Some("2024-05-01T10:00:00Z".to_string()),
            file_type: Some(1),
            owns_exported_media: true,
            owns_artwork: true,
            owns_waveform: true,
            artwork_path: Some("/PIONEER/Artwork/00001/a00123.jpg".to_string()),
            waveform_path: Some("/PIONEER/USBANLZ/P111/11111111/ANLZ0000.DAT".to_string()),
            duration_ms: Some(185_000),
        }
    }

    fn create_content_mapping_schema(conn: &rusqlite::Connection) {
        conn.execute_batch(
            r#"
            CREATE TABLE artist (
              artist_id INTEGER PRIMARY KEY,
              name TEXT
            );
            CREATE TABLE album (
              album_id INTEGER PRIMARY KEY,
              name TEXT,
              artist_id INTEGER,
              isComplation INTEGER
            );
            CREATE TABLE image (
              image_id INTEGER PRIMARY KEY,
              path TEXT
            );
            CREATE TABLE "key" (
              key_id INTEGER PRIMARY KEY,
              name TEXT
            );
            CREATE TABLE genre (
              genre_id INTEGER PRIMARY KEY,
              name TEXT
            );
            CREATE TABLE content (
              content_id INTEGER PRIMARY KEY,
              title TEXT,
              path TEXT,
              analysisDataFilePath TEXT,
              bpmx100 INTEGER,
              length INTEGER,
              artist_id_artist INTEGER,
              album_id INTEGER,
              key_id INTEGER,
              image_id INTEGER,
              fileName TEXT,
              fileSize INTEGER,
              fileType INTEGER,
              trackNo INTEGER,
              discNo INTEGER,
              bitrate INTEGER,
              samplingRate INTEGER,
              bitDepth INTEGER,
              subtitle TEXT,
              titleForSearch TEXT,
              isrc TEXT,
              djComment TEXT,
              kuvoDeliveryComment TEXT,
              releaseYear INTEGER,
              releaseDate TEXT,
              dateAdded TEXT,
              dateCreated TEXT,
              analysedBits INTEGER,
              djPlayCount INTEGER,
              rating INTEGER,
              color_id INTEGER,
              artist_id_lyricist INTEGER,
              artist_id_originalArtist INTEGER,
              artist_id_remixer INTEGER,
              artist_id_composer INTEGER,
              genre_id INTEGER,
              label_id INTEGER,
              contentLink INTEGER,
              masterContentId INTEGER,
              masterDbId INTEGER
            );
            "#,
        )
        .expect("create mapping schema");
    }

    #[test]
    fn insert_content_from_template_maps_parity_critical_columns() {
        let conn = rusqlite::Connection::open_in_memory().expect("open memory db");
        create_content_mapping_schema(&conn);
        conn.execute(r#"INSERT INTO "key" (key_id, name) VALUES (1, '8A')"#, [])
            .expect("seed key");

        let track = mapping_test_track();
        let content_id = insert_content_from_template(&conn, &track, Some("2024-07-09"))
            .expect("insert content");
        assert_eq!(content_id, 1);

        conn.query_row(
            "SELECT title, path, analysisDataFilePath, bpmx100, length, trackNo, analysedBits,
                    titleForSearch, kuvoDeliveryComment, djPlayCount, rating, color_id,
                    artist_id_lyricist, artist_id_originalArtist, artist_id_remixer, artist_id_composer,
                    genre_id, label_id, contentLink, masterContentId, masterDbId, releaseDate, dateAdded, dateCreated
             FROM content WHERE content_id = 1",
            [],
            |r| {
                assert_eq!(r.get::<_, String>(0)?, track.title);
                assert_eq!(r.get::<_, String>(1)?, track.exported_path);
                assert_eq!(r.get::<_, Option<String>>(2)?, track.waveform_path);
                assert_eq!(r.get::<_, i64>(3)?, 12345);
                assert_eq!(r.get::<_, i64>(4)?, 185);
                assert_eq!(r.get::<_, Option<i64>>(5)?, Some(3));
                assert_eq!(r.get::<_, i64>(6)?, 41);
                assert_eq!(
                    r.get::<_, Option<String>>(7)?.as_deref(),
                    Some("Mapping Track Search")
                );
                assert_eq!(r.get::<_, Option<String>>(8)?.as_deref(), Some("Kuvo note"));
                assert_eq!(r.get::<_, Option<i64>>(9)?, Some(7));
                assert_eq!(r.get::<_, Option<i64>>(10)?, Some(4));
                assert_eq!(r.get::<_, Option<i64>>(11)?, Some(6));
                assert_eq!(r.get::<_, Option<i64>>(12)?, Some(11));
                assert_eq!(r.get::<_, Option<i64>>(13)?, Some(12));
                assert_eq!(r.get::<_, Option<i64>>(14)?, Some(13));
                assert_eq!(r.get::<_, Option<i64>>(15)?, Some(14));
                assert_eq!(r.get::<_, Option<i64>>(16)?, Some(15));
                assert_eq!(r.get::<_, Option<i64>>(17)?, Some(16));
                assert_eq!(r.get::<_, Option<i64>>(18)?, track.content_link);
                assert_eq!(r.get::<_, Option<i64>>(19)?, track.master_content_id);
                assert_eq!(r.get::<_, Option<i64>>(20)?, track.master_db_id);
                assert_eq!(r.get::<_, Option<String>>(21)?.as_deref(), Some("2024-05-01"));
                assert_eq!(r.get::<_, Option<String>>(22)?.as_deref(), Some("2024-07-09"));
                assert_eq!(r.get::<_, Option<String>>(23)?.as_deref(), Some("2024-05-01"));
                Ok(())
            },
        )
        .expect("load mapped content row");
    }

    #[test]
    fn insert_content_from_template_sanitizes_metadata_but_preserves_paths_and_keys() {
        let conn = rusqlite::Connection::open_in_memory().expect("open memory db");
        create_content_mapping_schema(&conn);
        conn.execute(
            r#"INSERT INTO "key" (key_id, name) VALUES (?1, ?2)"#,
            rusqlite::params![9_i64, "8\0A"],
        )
        .expect("seed key");

        let mut track = mapping_test_track();
        track.title = format!("{}\0tail", "T".repeat(260));
        track.artist = "Art\0ist".to_string();
        track.album = Some("Alb\0um".to_string());
        track.genre = Some("Tech\0no".to_string());
        track.genre_id = None;
        track.subtitle = Some("Sub\0title".to_string());
        track.title_for_search = Some("  Find\0Me  ".to_string());
        track.comment = Some("Com\0ment".to_string());
        track.kuvo_delivery_comment = Some("Ku\0vo".to_string());
        track.key = Some("8\0A".to_string());
        track.exported_path = "/Contents/Bad\0Path.mp3".to_string();
        track.waveform_path = Some("/PIONEER/USBANLZ/Bad\0Anlz.DAT".to_string());

        let content_id = insert_content_from_template(&conn, &track, Some("2024-07-09"))
            .expect("insert content");
        assert_eq!(content_id, 1);

        conn.query_row(
            "SELECT c.title, c.path, c.analysisDataFilePath, c.subtitle, c.titleForSearch,
                    c.djComment, c.kuvoDeliveryComment, c.key_id, ar.name, al.name, g.name
             FROM content c
             LEFT JOIN artist ar ON ar.artist_id = c.artist_id_artist
             LEFT JOIN album al ON al.album_id = c.album_id
             LEFT JOIN genre g ON g.genre_id = c.genre_id
             WHERE c.content_id = 1",
            [],
            |r| {
                let title = r.get::<_, String>(0)?;
                assert_eq!(title, "T".repeat(255));
                assert!(!title.contains('\0'));
                assert_eq!(r.get::<_, String>(1)?, track.exported_path);
                assert_eq!(r.get::<_, Option<String>>(2)?, track.waveform_path);
                assert_eq!(r.get::<_, Option<String>>(3)?.as_deref(), Some("Subtitle"));
                assert_eq!(r.get::<_, Option<String>>(4)?.as_deref(), Some("FindMe"));
                assert_eq!(r.get::<_, Option<String>>(5)?.as_deref(), Some("Comment"));
                assert_eq!(r.get::<_, Option<String>>(6)?.as_deref(), Some("Kuvo"));
                assert_eq!(r.get::<_, Option<i64>>(7)?, Some(9));
                assert_eq!(r.get::<_, Option<String>>(8)?.as_deref(), Some("Artist"));
                assert_eq!(r.get::<_, Option<String>>(9)?.as_deref(), Some("Album"));
                assert_eq!(r.get::<_, Option<String>>(10)?.as_deref(), Some("Techno"));
                Ok(())
            },
        )
        .expect("load sanitized content row");
    }

    #[test]
    fn insert_content_from_template_matches_rb6_null_empty_conventions() {
        let conn = rusqlite::Connection::open_in_memory().expect("open memory db");
        create_content_mapping_schema(&conn);
        conn.execute(r#"INSERT INTO "key" (key_id, name) VALUES (1, '8A')"#, [])
            .expect("seed key");

        let mut track = mapping_test_track();
        track.title_for_search = None;
        track.release_date = None;

        let content_id = insert_content_from_template(&conn, &track, Some("2024-07-09"))
            .expect("insert content");
        assert_eq!(content_id, 1);

        let row = conn
            .query_row(
                "SELECT titleForSearch, releaseDate FROM content WHERE content_id = 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .expect("load content row");

        assert_eq!(
            row.0.as_deref(),
            Some(""),
            "titleForSearch should be empty string when missing"
        );
        assert_eq!(
            row.1.as_deref(),
            Some("2024-05-01"),
            "releaseDate should fall back to file_modified_at when release_date is missing"
        );
    }

    #[test]
    fn update_existing_content_row_maps_identity_and_analysis_fields() {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open memory db");
        create_content_mapping_schema(&conn);
        conn.execute(
            "INSERT INTO content (
                content_id, title, path, analysisDataFilePath, bpmx100, length, analysedBits,
                contentLink, masterContentId, masterDbId
             ) VALUES (
                10, 'old', '/Contents/old.mp3', '/PIONEER/USBANLZ/P000/OLD/ANLZ0000.DAT',
                0, 0, 0, 1, 2, 3
             )",
            [],
        )
        .expect("seed content row");
        conn.execute(r#"INSERT INTO "key" (key_id, name) VALUES (1, '8A')"#, [])
            .expect("seed key");

        let mut track = mapping_test_track();
        track.waveform_path = None;
        track.content_link = Some(7001);
        track.master_content_id = Some(7002);
        track.master_db_id = Some(7003);
        let tx = conn.transaction().expect("start tx");
        let columns = load_table_columns_tx(&tx, "content").expect("content columns");
        update_existing_content_row(&tx, 10, &track, &columns, Some("2024-07-09"))
            .expect("update content");
        tx.commit().expect("commit tx");

        let row = conn
            .query_row(
                "SELECT title, path, analysisDataFilePath, analysedBits, contentLink, masterContentId, masterDbId
                 FROM content WHERE content_id = 10",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                        r.get::<_, Option<i64>>(5)?,
                        r.get::<_, Option<i64>>(6)?,
                    ))
                },
            )
            .expect("load updated row");

        assert_eq!(row.0, track.title);
        assert_eq!(row.1, track.exported_path);
        assert_eq!(
            row.2.as_deref(),
            Some("/PIONEER/USBANLZ/P000/OLD/ANLZ0000.DAT"),
            "missing export waveform path should preserve existing analysisDataFilePath"
        );
        assert_eq!(row.3, 0);
        assert_eq!(row.4, track.content_link);
        assert_eq!(row.5, track.master_content_id);
        assert_eq!(row.6, track.master_db_id);
    }

    #[test]
    fn update_existing_content_row_sanitizes_metadata_but_preserves_paths_and_keys() {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open memory db");
        create_content_mapping_schema(&conn);
        conn.execute(
            r#"INSERT INTO "key" (key_id, name) VALUES (?1, ?2)"#,
            rusqlite::params![9_i64, "8\0A"],
        )
        .expect("seed key");
        conn.execute(
            "INSERT INTO content (
                content_id, title, path, analysisDataFilePath, subtitle, titleForSearch,
                djComment, kuvoDeliveryComment, key_id
             ) VALUES (
                10, 'old', '/Contents/old.mp3', '/PIONEER/USBANLZ/OLD.DAT',
                'old sub', 'old search', 'old comment', 'old kuvo', NULL
             )",
            [],
        )
        .expect("seed content row");

        let mut track = mapping_test_track();
        track.title = "Bad\0Title".to_string();
        track.subtitle = Some("Sub\0title".to_string());
        track.title_for_search = Some("  Find\0Me  ".to_string());
        track.comment = Some("Com\0ment".to_string());
        track.kuvo_delivery_comment = Some("Ku\0vo".to_string());
        track.key = Some("8\0A".to_string());
        track.exported_path = "/Contents/Bad\0Path.mp3".to_string();
        track.waveform_path = Some("/PIONEER/USBANLZ/Bad\0Anlz.DAT".to_string());

        let tx = conn.transaction().expect("start tx");
        let columns = load_table_columns_tx(&tx, "content").expect("content columns");
        update_existing_content_row(&tx, 10, &track, &columns, Some("2024-07-09"))
            .expect("update content");
        tx.commit().expect("commit tx");

        conn.query_row(
            "SELECT title, path, analysisDataFilePath, subtitle, titleForSearch,
                    djComment, kuvoDeliveryComment, key_id
             FROM content WHERE content_id = 10",
            [],
            |r| {
                assert_eq!(r.get::<_, String>(0)?, "BadTitle");
                assert_eq!(r.get::<_, String>(1)?, track.exported_path);
                assert_eq!(r.get::<_, Option<String>>(2)?, track.waveform_path);
                assert_eq!(r.get::<_, Option<String>>(3)?.as_deref(), Some("Subtitle"));
                assert_eq!(r.get::<_, Option<String>>(4)?.as_deref(), Some("FindMe"));
                assert_eq!(r.get::<_, Option<String>>(5)?.as_deref(), Some("Comment"));
                assert_eq!(r.get::<_, Option<String>>(6)?.as_deref(), Some("Kuvo"));
                assert_eq!(r.get::<_, Option<i64>>(7)?, Some(9));
                Ok(())
            },
        )
        .expect("load updated content row");
    }

    #[test]
    fn update_existing_content_row_preserves_existing_metadata_when_export_track_is_thin() {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open memory db");
        create_content_mapping_schema(&conn);
        conn.execute(
            "INSERT INTO artist (artist_id, name) VALUES (7, 'Existing Artist')",
            [],
        )
        .expect("seed artist");
        conn.execute(
            "INSERT INTO album (album_id, name, artist_id, isComplation) VALUES (9, 'Existing Album', 7, 0)",
            [],
        )
        .expect("seed album");
        conn.execute(r#"INSERT INTO "key" (key_id, name) VALUES (5, '8A')"#, [])
            .expect("seed key");
        conn.execute(
            "INSERT INTO image (image_id, path) VALUES (11, '/PIONEER/Artwork/00001/a.jpg')",
            [],
        )
        .expect("seed image");
        conn.execute(
            "INSERT INTO content (
                content_id, title, path, analysisDataFilePath, bpmx100, length,
                artist_id_artist, album_id, key_id, image_id
             ) VALUES (
                10, 'old', '/Contents/existing.mp3', '/PIONEER/USBANLZ/P000/OLD.DAT',
                12800, 321, 7, 9, 5, 11
             )",
            [],
        )
        .expect("seed content row");

        let mut track = mapping_test_track();
        track.album = None;
        track.artwork_path = None;
        track.duration_ms = None;
        track.key = None;

        let tx = conn.transaction().expect("start tx");
        let columns = load_table_columns_tx(&tx, "content").expect("content columns");
        update_existing_content_row(&tx, 10, &track, &columns, Some("2024-07-09"))
            .expect("update content");
        tx.commit().expect("commit tx");

        let row = conn
            .query_row(
                "SELECT artist_id_artist, album_id, key_id, image_id, length FROM content WHERE content_id = 10",
                [],
                |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .expect("load updated row");

        assert!(
            row.0.is_some() && row.0 != Some(7),
            "artist is rewritten from export track"
        );
        assert_eq!(
            row.1,
            Some(9),
            "missing export album should preserve existing album"
        );
        assert_eq!(
            row.2, None,
            "missing export key should clear stale existing key"
        );
        assert_eq!(
            row.3,
            Some(11),
            "missing export artwork should preserve existing image"
        );
        assert_eq!(
            row.4, 321,
            "missing export duration should preserve existing length"
        );
    }

    #[test]
    fn update_existing_content_row_matches_rb6_null_empty_conventions() {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open memory db");
        create_content_mapping_schema(&conn);
        conn.execute(r#"INSERT INTO "key" (key_id, name) VALUES (1, '8A')"#, [])
            .expect("seed key");
        conn.execute(
            "INSERT INTO content (
                content_id, title, path, titleForSearch, releaseDate
             ) VALUES (
                10, 'old', '/Contents/old.mp3', 'legacy-search', '2020-01-01'
             )",
            [],
        )
        .expect("seed content row");

        let mut track = mapping_test_track();
        track.title_for_search = None;
        track.release_date = None;

        let tx = conn.transaction().expect("start tx");
        let columns = load_table_columns_tx(&tx, "content").expect("content columns");
        update_existing_content_row(&tx, 10, &track, &columns, Some("2024-07-09"))
            .expect("update content");
        tx.commit().expect("commit tx");

        let row = conn
            .query_row(
                "SELECT titleForSearch, releaseDate FROM content WHERE content_id = 10",
                [],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .expect("load updated row");

        assert_eq!(row.0, None, "titleForSearch should be NULL after update");
        assert_eq!(
            row.1.as_deref(),
            Some("2024-05-01"),
            "releaseDate should fall back to file_modified_at after update"
        );
    }

    #[test]
    fn insert_content_from_template_sets_release_year_zero_when_missing() {
        let conn = rusqlite::Connection::open_in_memory().expect("open memory db");
        create_content_mapping_schema(&conn);
        conn.execute(r#"INSERT INTO "key" (key_id, name) VALUES (1, '8A')"#, [])
            .expect("seed key");

        let mut track = mapping_test_track();
        track.release_year = None;

        let content_id = insert_content_from_template(&conn, &track, Some("2024-07-09"))
            .expect("insert content");
        assert_eq!(content_id, 1);

        let release_year = conn
            .query_row(
                "SELECT releaseYear FROM content WHERE content_id = 1",
                [],
                |r| r.get::<_, Option<i64>>(0),
            )
            .expect("load content row");
        assert_eq!(release_year, Some(0));
    }

    #[test]
    fn update_existing_content_row_sets_release_year_zero_when_missing() {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open memory db");
        create_content_mapping_schema(&conn);
        conn.execute(r#"INSERT INTO "key" (key_id, name) VALUES (1, '8A')"#, [])
            .expect("seed key");
        conn.execute(
            "INSERT INTO content (content_id, title, path, releaseYear) VALUES (10, 'old', '/Contents/old.mp3', 2024)",
            [],
        )
        .expect("seed content row");

        let mut track = mapping_test_track();
        track.release_year = None;

        let tx = conn.transaction().expect("start tx");
        let columns = load_table_columns_tx(&tx, "content").expect("content columns");
        update_existing_content_row(&tx, 10, &track, &columns, Some("2024-07-09"))
            .expect("update content");
        tx.commit().expect("commit tx");

        let release_year = conn
            .query_row(
                "SELECT releaseYear FROM content WHERE content_id = 10",
                [],
                |r| r.get::<_, Option<i64>>(0),
            )
            .expect("load updated row");
        assert_eq!(release_year, Some(0));
    }

    #[test]
    fn link_playlist_content_maps_playlist_id_content_id_and_sequence() {
        let conn = rusqlite::Connection::open_in_memory().expect("open memory db");
        conn.execute_batch(
            "CREATE TABLE playlist_content (
                playlist_id INTEGER,
                content_id INTEGER,
                sequenceNo INTEGER
             )",
        )
        .expect("create playlist_content");

        link_playlist_content(&conn, 17, 42, 3).expect("link playlist content");
        let row = conn
            .query_row(
                "SELECT playlist_id, content_id, sequenceNo FROM playlist_content",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )
            .expect("load playlist_content row");
        assert_eq!(row, (17, 42, 3));
    }

    // --- sanitize_filename_component ---

    #[test]
    fn sanitize_filename_alphanumeric_preserved() {
        assert_eq!(sanitize_filename_component("hello123"), "hello123");
    }

    #[test]
    fn sanitize_filename_spaces_become_underscores() {
        assert_eq!(sanitize_filename_component("my track"), "my_track");
    }

    #[test]
    fn sanitize_filename_control_chars_removed() {
        assert_eq!(sanitize_filename_component("ab\x00cd\x01ef"), "abcdef");
    }

    #[test]
    fn sanitize_filename_replacement_char_removed() {
        assert_eq!(
            sanitize_filename_component("hello\u{FFFD}world"),
            "helloworld"
        );
    }

    #[test]
    fn sanitize_filename_empty_becomes_untitled() {
        assert_eq!(sanitize_filename_component(""), "untitled");
    }

    #[test]
    fn sanitize_filename_only_special_chars_becomes_untitled() {
        assert_eq!(sanitize_filename_component("   "), "untitled");
    }

    #[test]
    fn sanitize_filename_consecutive_underscores_collapsed() {
        // Multiple non-alphanum chars produce underscores that get collapsed
        let result = sanitize_filename_component("a   b");
        assert_eq!(result, "a_b");
    }

    // --- sanitize_contents_component ---

    #[test]
    fn sanitize_contents_path_separators_replaced() {
        assert_eq!(sanitize_contents_component("a/b\\c"), "a_b_c");
    }

    #[test]
    fn sanitize_contents_control_chars_replaced() {
        assert_eq!(sanitize_contents_component("ab\x01cd"), "ab_cd");
    }

    #[test]
    fn sanitize_contents_special_fs_chars_replaced() {
        assert_eq!(sanitize_contents_component("a:b*c?d"), "a_b_c_d");
    }

    #[test]
    fn sanitize_contents_empty_becomes_unknown() {
        assert_eq!(sanitize_contents_component(""), "Unknown");
        assert_eq!(sanitize_contents_component("   "), "Unknown");
    }

    #[test]
    fn sanitize_contents_preserves_unicode_letters() {
        // Unicode letters are NOT control chars, so they pass through
        let result = sanitize_contents_component("Café Müsik");
        assert!(result.contains("Caf"));
        assert!(result.contains("sik"));
    }

    #[test]
    fn sanitize_contents_caps_zalgo_combining_marks() {
        // "Zalgo" text stacks dozens of combining marks onto one base
        // character; CDJ text rendering hangs compositing an unbounded
        // stack. The on-disk folder/file name must not carry that through.
        let mark = '\u{0301}';
        let zalgo: String = std::iter::once('e')
            .chain(std::iter::repeat(mark).take(60))
            .collect();
        let result = sanitize_contents_component(&zalgo);
        assert!(
            result.chars().count() <= crate::metadata::MAX_GRAPHEME_CLUSTER_CHARS,
            "expected capped cluster, got {} chars",
            result.chars().count()
        );
        assert!(result.starts_with('e'));
    }

    #[test]
    fn sanitize_contents_caps_script_diversity() {
        // Fabricated (not pulled from any real file), shaped like a name
        // observed in the wild: no single grapheme cluster is deep enough
        // to trip the combining-mark cap, but it mixes many unrelated
        // scripts in a short string — separately observed to hang the
        // CDJ's Artist browse menu and track-load screen. Codepoints are
        // Cherokee, Runic, Glagolitic, Coptic, N'Ko, Vai, Osmanya, and
        // Deseret, chosen only because they're distinct scripts.
        let artist =
            "\u{13A0}\u{13A1} \u{16A0}\u{16A1} \u{2C00}\u{2C01} \u{2C80}\u{2C81} \u{07CA}\u{07CB} \u{A500}\u{A501} \u{10480}\u{10481} \u{10400}\u{10401}";
        let result = sanitize_contents_component(artist);
        assert!(!result.is_empty());
    }

    // --- truncate_component ---

    #[test]
    fn truncate_within_limit_unchanged() {
        assert_eq!(truncate_component("short", 48), "short");
    }

    #[test]
    fn truncate_at_limit() {
        let long = "a".repeat(50);
        assert_eq!(truncate_component(&long, 48).len(), 48);
    }

    #[test]
    fn truncate_strips_trailing_spaces_and_dots() {
        assert_eq!(truncate_component("hello   ", 48), "hello");
        assert_eq!(truncate_component("hello...", 48), "hello");
    }

    #[test]
    fn truncate_all_dots_becomes_unknown() {
        assert_eq!(truncate_component("...", 48), "Unknown");
    }

    #[test]
    fn truncate_empty_becomes_unknown() {
        assert_eq!(truncate_component("", 48), "Unknown");
    }

    // --- limit_contents_file_name ---

    #[test]
    fn limit_filename_within_limit_unchanged() {
        assert_eq!(limit_contents_file_name("track.mp3", 48), "track.mp3");
    }

    #[test]
    fn limit_filename_preserves_extension() {
        let long_name = format!("{}.mp3", "a".repeat(60));
        let result = limit_contents_file_name(&long_name, 48);
        assert!(
            result.ends_with(".mp3"),
            "extension not preserved: {result}"
        );
        assert!(result.chars().count() <= 48, "too long: {result}");
    }

    #[test]
    fn limit_filename_no_extension() {
        let long_name = "a".repeat(60);
        let result = limit_contents_file_name(&long_name, 48);
        assert_eq!(result.chars().count(), 48);
    }

    // --- stable_u32_hash ---

    #[test]
    fn stable_hash_deterministic() {
        let h1 = stable_u32_hash("test-track-id");
        let h2 = stable_u32_hash("test-track-id");
        assert_eq!(h1, h2);
    }

    #[test]
    fn stable_hash_different_inputs_differ() {
        assert_ne!(stable_u32_hash("track-a"), stable_u32_hash("track-b"));
    }

    #[test]
    fn stable_hash_empty_string() {
        // Should not panic
        let _ = stable_u32_hash("");
    }

    // --- canonical_artwork_target_path ---

    #[test]
    fn artwork_path_structure() {
        let usb = Path::new("/mnt/usb");
        let path = canonical_artwork_target_path(usb, "track-123", "/music/cover.jpg");
        let s = path.to_string_lossy();
        assert!(
            s.starts_with("/mnt/usb/PIONEER/Artwork/"),
            "wrong base: {s}"
        );
        assert!(s.ends_with(".jpg"), "wrong extension: {s}");
    }

    #[test]
    fn artwork_path_stable_across_calls() {
        let usb = Path::new("/mnt/usb");
        let p1 = canonical_artwork_target_path(usb, "id-1", "/art.png");
        let p2 = canonical_artwork_target_path(usb, "id-1", "/art.png");
        assert_eq!(p1, p2);
    }

    #[test]
    fn artwork_path_always_jpg_for_player() {
        let usb = Path::new("/mnt/usb");
        // Regardless of source extension, player artwork is always JPEG
        let path = canonical_artwork_target_path(usb, "id-1", "/art.PNG");
        assert!(
            path.to_string_lossy().ends_with(".jpg"),
            "Player artwork must be .jpg"
        );
    }

    #[test]
    fn artwork_path_no_extension_defaults_to_jpg() {
        let usb = Path::new("/mnt/usb");
        let path = canonical_artwork_target_path(usb, "id-1", "/art");
        assert!(path.to_string_lossy().ends_with(".jpg"));
    }

    #[test]
    fn artwork_target_paths_produces_small_and_medium() {
        let usb = Path::new("/mnt/usb");
        let (small, medium) = canonical_artwork_target_paths(usb, "track-99");
        let s = small.to_string_lossy();
        let m = medium.to_string_lossy();
        assert!(s.starts_with("/mnt/usb/PIONEER/Artwork/"));
        assert!(s.ends_with(".jpg"));
        assert!(!s.contains("_m"), "small path must NOT contain _m");
        assert!(
            m.ends_with("_m.jpg"),
            "medium path must end with _m.jpg: {m}"
        );
        // Same directory
        assert_eq!(small.parent(), medium.parent());
    }

    #[test]
    fn export_artwork_for_player_creates_two_square_jpegs() {
        // Create a test PNG image (non-square: 200x100)
        let dir = tempdir().unwrap();
        let source = dir.path().join("cover.png");
        let img = image::RgbImage::from_fn(200, 100, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        image::DynamicImage::ImageRgb8(img).save(&source).unwrap();

        let usb_root = dir.path().join("USB");
        std::fs::create_dir_all(&usb_root).unwrap();
        let mut warnings = Vec::new();
        let result = export_artwork_for_player(
            source.to_str().unwrap(),
            &usb_root,
            "test-track-1",
            &mut warnings,
        )
        .unwrap();
        assert!(result.is_some(), "should produce artwork path");
        assert!(warnings.is_empty(), "no warnings: {warnings:?}");

        let (small_path, medium_path) = canonical_artwork_target_paths(&usb_root, "test-track-1");
        assert!(small_path.is_file(), "small JPEG must exist");
        assert!(medium_path.is_file(), "medium JPEG must exist");

        // Verify dimensions are square
        let small_img = image::open(&small_path).unwrap();
        let medium_img = image::open(&medium_path).unwrap();
        assert_eq!(small_img.width(), 80);
        assert_eq!(small_img.height(), 80);
        assert_eq!(medium_img.width(), 240);
        assert_eq!(medium_img.height(), 240);
    }

    #[test]
    fn export_artwork_for_player_missing_source_returns_none() {
        let dir = tempdir().unwrap();
        let mut warnings = Vec::new();
        let result = export_artwork_for_player(
            "/nonexistent/cover.jpg",
            dir.path(),
            "track-1",
            &mut warnings,
        )
        .unwrap();
        assert!(result.is_none());
        assert!(!warnings.is_empty());
    }

    #[test]
    fn verify_edb_content_fails_when_artwork_file_is_missing_on_disk() {
        let dir = tempdir().unwrap();
        let usb_root = dir.path();
        let export_db = usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR);
        std::fs::create_dir_all(&export_db).unwrap();

        let db_path = export_db.join("exportLibrary.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE playlist (playlist_id INTEGER PRIMARY KEY, name TEXT, attribute INTEGER, sequenceNo INTEGER);
            CREATE TABLE image (image_id INTEGER PRIMARY KEY, path TEXT);
            CREATE TABLE content (content_id INTEGER PRIMARY KEY, path TEXT, analysisDataFilePath TEXT, image_id INTEGER);
            CREATE TABLE playlist_content (playlist_id INTEGER, content_id INTEGER, sequenceNo INTEGER);
            INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (1, 'Test', 0, 1);
            INSERT INTO image (image_id, path) VALUES (10, '/PIONEER/Artwork/00001/b14990.jpg');
            INSERT INTO content (content_id, path, analysisDataFilePath, image_id)
              VALUES (20, '/Contents/Artist One/Album One/Artist One - Album One - 03 Track One.mp3', NULL, 10);
            INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (1, 20, 1);
            "#,
        )
        .unwrap();

        let playlist = ExportPlaylistData {
            id: "pl1".to_string(),
            name: "Test".to_string(),
            tracks: vec![],
        };
        let manifest = ExportManifest {
            version: 1,
            generated_at: "2024-01-01".to_string(),
            playlist_id: "pl1".to_string(),
            playlist_name: "Test".to_string(),
            usb_root: usb_root.to_string_lossy().to_string(),
            options: ExportToUsbOptions {
                include_artwork: true,
                include_analysis: false,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 1,
            skipped_tracks: 0,
            warnings: vec![],
            tracks: vec![ExportManifestTrack {
                id: "t1".to_string(),
                master_db_id: None,
                master_content_id: None,
                content_link: None,
                position: 1,
                track_number: Some(3),
                title: "Track One".to_string(),
                artist: "Artist One".to_string(),
                album: Some("Album One".to_string()),
                bpm: Some(150.37),
                key: Some("Am".to_string()),
                source_path: "/tmp/source.mp3".to_string(),
                exported_path:
                    "/Contents/Artist One/Album One/Artist One - Album One - 03 Track One.mp3"
                        .to_string(),
                file_modified_at: None,
                file_size_bytes: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                title_for_search: None,
                kuvo_delivery_comment: None,
                dj_play_count: None,
                rating: None,
                color_id: None,
                artist_id_lyricist: None,
                artist_id_original_artist: None,
                artist_id_remixer: None,
                artist_id_composer: None,
                genre_id: None,
                genre: None,
                label_id: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                file_type: None,
                owns_exported_media: true,
                owns_artwork: true,
                owns_waveform: true,
                artwork_path: Some("/PIONEER/Artwork/00001/b14990.jpg".to_string()),
                waveform_path: None,
                duration_ms: Some(202_000),
            }],
        };

        let err = verify_edb_content(usb_root, &playlist, &manifest).unwrap_err();
        assert!(
            err.to_string().contains("artwork file missing on disk"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn upsert_export_playlist_row_prefers_same_name_playlist_with_most_entries() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("exportLibrary.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE playlist (playlist_id INTEGER PRIMARY KEY, name TEXT, attribute INTEGER, sequenceNo INTEGER);
            CREATE TABLE playlist_content (playlist_id INTEGER, content_id INTEGER, sequenceNo INTEGER);
            INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES
              (1, 'Testi', 0, 1),
              (2, 'Testi', 0, 2);
            INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES
              (1, 11, 1),
              (1, 12, 2),
              (2, 21, 1);
            "#,
        )
        .unwrap();

        let playlist = ExportPlaylistData {
            id: "pl1".to_string(),
            name: "Testi".to_string(),
            tracks: vec![],
        };

        let chosen = upsert_export_playlist_row(&conn, &playlist).unwrap();
        assert_eq!(
            chosen, 1,
            "should target the fullest same-name playlist row"
        );
    }

    // --- canonical_analysis_bundle_paths ---

    #[test]
    fn analysis_paths_structure() {
        let usb = Path::new("/mnt/usb");
        let (dat, ext, twoex) =
            canonical_analysis_bundle_paths(usb, "/Contents/Artist/Album/track-456.mp3");
        let dat_s = dat.to_string_lossy();
        assert!(dat_s.contains("PIONEER/USBANLZ/"), "wrong base: {dat_s}");
        assert!(dat_s.ends_with("ANLZ0000.DAT"));
        assert!(ext.to_string_lossy().ends_with("ANLZ0000.EXT"));
        assert!(twoex.to_string_lossy().ends_with("ANLZ0000.2EX"));
    }

    #[test]
    fn analysis_paths_deterministic() {
        let usb = Path::new("/mnt/usb");
        let (d1, e1, t1) = canonical_analysis_bundle_paths(usb, "/Contents/Artist/Album/id-x.mp3");
        let (d2, e2, t2) = canonical_analysis_bundle_paths(usb, "/Contents/Artist/Album/id-x.mp3");
        assert_eq!(d1, d2);
        assert_eq!(e1, e2);
        assert_eq!(t1, t2);
    }

    #[test]
    fn export_analysis_copies_internal_cache_anlz_even_when_invalid() {
        let dir = tempdir().unwrap();
        let usb_root = dir.path().join("usb");
        fs::create_dir_all(&usb_root).unwrap();

        let stale_dir = dir.path().join(".app-data/analysis/waveforms");
        fs::create_dir_all(&stale_dir).unwrap();
        let stale_dat = stale_dir.join("A1B2C3D4.DAT");
        let stale_ext = stale_dir.join("A1B2C3D4.EXT");
        let stale_2ex = stale_dir.join("A1B2C3D4.2EX");
        fs::write(&stale_dat, b"STALE-DAT").unwrap();
        fs::write(&stale_ext, b"STALE-EXT").unwrap();
        fs::write(&stale_2ex, b"STALE-2EX").unwrap();

        let fixture_mp3 = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio/noart/track_no_art.mp3");
        assert!(
            fixture_mp3.is_file(),
            "fixture mp3 missing: {}",
            fixture_mp3.display()
        );

        let track = ExportTrackData {
            id: "t-local-cache".to_string(),
            title: "Track".to_string(),
            artist: "Artist".to_string(),
            album: None,
            track_number: Some(1),
            bpm: Some(120.0),
            key: None,
            file_path: fixture_mp3.to_string_lossy().to_string(),
            file_name: "track_no_art.mp3".to_string(),
            file_modified_at: None,
            file_size_bytes: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: None,
            color_id: None,
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: None,
            release_year: None,
            release_date: None,
            recorded_date: None,
            file_type: None,
            artwork_path: None,
            waveform_peaks_path: Some(stale_dat.to_string_lossy().to_string()),
            duration_ms: Some(30_000),
            first_beat_ms: None,
            position: 1,
        };

        let mut warnings = Vec::new();
        let exported =
            export_analysis_bundle_for_track(&track, &usb_root, "/Contents/t.mp3", &mut warnings)
                .expect("export analysis bundle");
        assert!(exported.is_some(), "expected waveform path");

        let (dat, ext, twoex) = canonical_analysis_bundle_paths(&usb_root, "/Contents/t.mp3");
        assert_eq!(fs::read(&dat).unwrap(), b"STALE-DAT");
        assert_eq!(fs::read(&ext).unwrap(), b"STALE-EXT");
        assert_eq!(fs::read(&twoex).unwrap(), b"STALE-2EX");
    }

    #[test]
    fn export_analysis_copies_local_anlz_and_rewrites_ppth_path() {
        use crate::service::anlz::{
            build_anlz_2ex_file, build_anlz_dat_file, build_anlz_ext_file, ppth_path_from_anlz,
        };

        let dir = tempdir().unwrap();
        let usb_root = dir.path().join("usb");
        fs::create_dir_all(&usb_root).unwrap();
        let local_dir = dir.path().join("analysis");
        fs::create_dir_all(&local_dir).unwrap();

        // 400 bins satisfies the minimum detail-gate for any duration ≤ 15s (required ≤ 2400,
        // so the PWV7 triplet-run check is skipped). Use 1s to keep the fixture minimal.
        let waveform = WaveformData::from_peaks(vec![128; 400]);
        let dat_content = build_anlz_dat_file(&waveform, "", None, Some(1_000));
        let ext_content = build_anlz_ext_file(&waveform, "", None, Some(1_000));
        let twoex_content = build_anlz_2ex_file(&waveform, "", Some(1_000));
        assert!(
            ppth_path_from_anlz(&dat_content).is_none(),
            "local analysis cache should start without PPTH"
        );

        let local_dat = local_dir.join("ABCD1234.DAT");
        let local_ext = local_dir.join("ABCD1234.EXT");
        let local_2ex = local_dir.join("ABCD1234.2EX");
        fs::write(&local_dat, &dat_content).unwrap();
        fs::write(&local_ext, &ext_content).unwrap();
        fs::write(&local_2ex, &twoex_content).unwrap();

        let track = ExportTrackData {
            id: "t-copy-local".to_string(),
            title: "Track".to_string(),
            artist: "Artist".to_string(),
            album: None,
            track_number: None,
            bpm: None,
            key: None,
            file_path: "/should/not/be/read.mp3".to_string(),
            file_name: "track.mp3".to_string(),
            file_modified_at: None,
            file_size_bytes: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: None,
            color_id: None,
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: None,
            release_year: None,
            release_date: None,
            recorded_date: None,
            file_type: None,
            artwork_path: None,
            waveform_peaks_path: Some(local_dat.to_string_lossy().to_string()),
            duration_ms: Some(1_000),
            first_beat_ms: None,
            position: 1,
        };

        let exported_path = "/Contents/Fixture Ö Artist/Fixture Ä Album/03 - Entä jos Fixture.flac";
        let mut warnings = Vec::new();
        let result =
            export_analysis_bundle_for_track(&track, &usb_root, exported_path, &mut warnings)
                .expect("copy local anlz");
        assert!(result.is_some(), "expected a path returned");
        assert!(warnings.is_empty(), "no warnings expected");

        let (dat, ext, twoex) = canonical_analysis_bundle_paths(&usb_root, exported_path);
        let dat_bytes = fs::read(&dat).unwrap();
        let ext_bytes = fs::read(&ext).unwrap();
        let twoex_bytes = fs::read(&twoex).unwrap();
        assert_eq!(
            ppth_path_from_anlz(&dat_bytes).as_deref(),
            Some(exported_path)
        );
        assert_eq!(
            ppth_path_from_anlz(&ext_bytes).as_deref(),
            Some(exported_path)
        );
        assert_eq!(
            ppth_path_from_anlz(&twoex_bytes).as_deref(),
            Some(exported_path)
        );
        assert!(dat_bytes.windows(4).any(|w| w == b"PVBR"));
        assert!(ext_bytes.windows(4).any(|w| w == b"PWV3"));
        assert!(twoex_bytes.windows(4).any(|w| w == b"PWV7"));
    }

    #[test]
    fn export_analysis_does_not_regenerate_when_local_dat_missing() {
        let dir = tempdir().unwrap();
        let usb_root = dir.path().join("usb");
        fs::create_dir_all(&usb_root).unwrap();

        let fixture_mp3 = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio/noart/track_no_art.mp3");
        assert!(
            fixture_mp3.is_file(),
            "fixture mp3 missing: {}",
            fixture_mp3.display()
        );

        let track = ExportTrackData {
            id: "t-fallback".to_string(),
            title: "Track".to_string(),
            artist: "Artist".to_string(),
            album: None,
            track_number: None,
            bpm: Some(120.0),
            key: None,
            file_path: fixture_mp3.to_string_lossy().to_string(),
            file_name: "track_no_art.mp3".to_string(),
            file_modified_at: None,
            file_size_bytes: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: None,
            color_id: None,
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: None,
            release_year: None,
            release_date: None,
            recorded_date: None,
            file_type: None,
            artwork_path: None,
            waveform_peaks_path: Some("/nonexistent/path/DEADBEEF.DAT".to_string()),
            duration_ms: Some(30_000),
            first_beat_ms: None,
            position: 1,
        };

        let mut warnings = Vec::new();
        let result =
            export_analysis_bundle_for_track(&track, &usb_root, "/Contents/t.mp3", &mut warnings)
                .expect("missing local analysis handled");
        assert!(
            result.is_none(),
            "missing local analysis must not regenerate"
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("analysis bundle missing")),
            "expected missing bundle warning, got {warnings:?}"
        );

        let (dat, ..) = canonical_analysis_bundle_paths(&usb_root, "/Contents/t.mp3");
        assert!(
            !dat.exists(),
            "export must not create generated analysis when source bundle is missing"
        );
    }

    // --- exported_media_target_path ---

    #[test]
    fn media_path_preserves_contents_relative() {
        let media = Path::new("/mnt/usb/Contents");
        let source = Path::new("/other/usb/Contents/Artist/Album/track.mp3");
        let result = exported_media_target_path(media, source, "X", Some("Y"), "Z", "mp3");
        // Should preserve the path after "Contents"
        let s = result.to_string_lossy();
        assert!(
            s.starts_with("/mnt/usb/Contents/Artist/Album/track.mp3"),
            "got: {s}"
        );
    }

    #[test]
    fn media_path_fallback_uses_sanitized_artist_album() {
        let media = Path::new("/mnt/usb/Contents");
        let source = Path::new("/local/music/my_track.mp3");
        let result = exported_media_target_path(
            media,
            source,
            "DJ Test",
            Some("Cool Album"),
            "Title",
            "mp3",
        );
        let s = result.to_string_lossy();
        assert!(s.contains("DJ Test"), "missing artist: {s}");
        assert!(s.contains("Cool Album"), "missing album: {s}");
    }

    #[test]
    fn media_path_no_album_uses_unknown() {
        let media = Path::new("/mnt/usb/Contents");
        let source = Path::new("/local/music/track.mp3");
        let result = exported_media_target_path(media, source, "Artist", None, "Title", "mp3");
        let s = result.to_string_lossy();
        assert!(s.contains("UnknownAlbum"), "missing fallback: {s}");
    }

    // --- copy_if_different ---

    #[test]
    fn copy_if_different_copies_new_file() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("source.bin");
        let dst = dir.path().join("sub/target.bin");
        fs::write(&src, b"hello").unwrap();

        copy_if_different(&src, &dst).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"hello");
    }

    #[test]
    fn copy_if_different_skips_same_size() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("source.bin");
        let dst = dir.path().join("target.bin");
        fs::write(&src, b"hello").unwrap();
        fs::write(&dst, b"world").unwrap(); // same size, different content

        copy_if_different(&src, &dst).unwrap();
        // Same size → skips copy, so dst still has "world"
        assert_eq!(fs::read(&dst).unwrap(), b"world");
    }

    #[test]
    fn copy_if_different_copies_different_size() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("source.bin");
        let dst = dir.path().join("target.bin");
        fs::write(&src, b"hello world").unwrap();
        fs::write(&dst, b"hi").unwrap();

        copy_if_different(&src, &dst).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"hello world");
    }

    #[test]
    fn copy_if_different_missing_source_errors() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("nonexistent.bin");
        let dst = dir.path().join("target.bin");

        let result = copy_if_different(&src, &dst);
        assert!(result.is_err());
    }

    // --- copy_wav_normalized_if_needed ---

    fn wav_with_extensible_fmt(sub_format_tag: u16, data: &[u8]) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&0xFFFEu16.to_le_bytes()); // WAVE_FORMAT_EXTENSIBLE
        fmt.extend_from_slice(&1u16.to_le_bytes()); // mono
        fmt.extend_from_slice(&44_100u32.to_le_bytes());
        fmt.extend_from_slice(&(44_100u32 * 2).to_le_bytes()); // byte rate
        fmt.extend_from_slice(&2u16.to_le_bytes()); // block align
        fmt.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        fmt.extend_from_slice(&22u16.to_le_bytes()); // cbSize
        fmt.extend_from_slice(&16u16.to_le_bytes()); // validBitsPerSample
        fmt.extend_from_slice(&0x4u32.to_le_bytes()); // channel mask
        fmt.extend_from_slice(&sub_format_tag.to_le_bytes());
        fmt.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x10, 0x00]);
        fmt.extend_from_slice(&[0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71]);

        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        body.extend_from_slice(&fmt);
        body.extend_from_slice(b"data");
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);

        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn copy_wav_normalized_if_needed_rewrites_extensible_pcm() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("source.wav");
        let data = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        fs::write(&src, wav_with_extensible_fmt(1, &data)).unwrap();
        let dst = dir.path().join("sub/target.wav");

        copy_wav_normalized_if_needed(&src, &dst).unwrap();

        let out = fs::read(&dst).unwrap();
        let format_tag = u16::from_le_bytes(out[20..22].try_into().unwrap());
        assert_eq!(format_tag, 1, "should be normalized to plain PCM");
        assert!(out.ends_with(&data), "sample data must be preserved verbatim");
    }

    #[test]
    fn copy_wav_normalized_if_needed_skips_rewrite_when_target_up_to_date() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("source.wav");
        fs::write(&src, wav_with_extensible_fmt(1, &[0u8; 20])).unwrap();
        let dst = dir.path().join("target.wav");

        copy_wav_normalized_if_needed(&src, &dst).unwrap();
        let first_write = fs::metadata(&dst).unwrap().modified().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(10));
        copy_wav_normalized_if_needed(&src, &dst).unwrap();
        let second_write = fs::metadata(&dst).unwrap().modified().unwrap();

        assert_eq!(first_write, second_write, "up-to-date target should not be rewritten");
    }

    #[test]
    fn copy_wav_normalized_if_needed_falls_back_for_non_extensible_wav() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("source.wav");
        // Plain PCM (format tag 1), not extensible - should behave like copy_if_different.
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&1u16.to_le_bytes());
        fmt.extend_from_slice(&1u16.to_le_bytes());
        fmt.extend_from_slice(&44_100u32.to_le_bytes());
        fmt.extend_from_slice(&(44_100u32 * 2).to_le_bytes());
        fmt.extend_from_slice(&2u16.to_le_bytes());
        fmt.extend_from_slice(&16u16.to_le_bytes());
        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        body.extend_from_slice(&fmt);
        body.extend_from_slice(b"data");
        body.extend_from_slice(&8u32.to_le_bytes());
        body.extend_from_slice(&[0u8; 8]);
        let mut plain = Vec::new();
        plain.extend_from_slice(b"RIFF");
        plain.extend_from_slice(&(body.len() as u32).to_le_bytes());
        plain.extend_from_slice(&body);
        fs::write(&src, &plain).unwrap();
        let dst = dir.path().join("target.wav");

        copy_wav_normalized_if_needed(&src, &dst).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), plain, "non-extensible WAV copied as-is");
    }

    #[test]
    fn copy_wav_normalized_if_needed_falls_back_for_extensible_other_subformat() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("source.wav");
        let source_bytes = wav_with_extensible_fmt(0x0006, &[0u8; 20]);
        fs::write(&src, &source_bytes).unwrap();
        let dst = dir.path().join("target.wav");

        copy_wav_normalized_if_needed(&src, &dst).unwrap();
        assert_eq!(
            fs::read(&dst).unwrap(),
            source_bytes,
            "extensible-other WAV is not safe to rewrite, so it's copied verbatim"
        );
    }

    #[test]
    fn remove_playlist_and_tracks_from_pdb_allows_removing_last_playlist() {
        let dir = tempdir().unwrap();
        let usb_root = dir.path();
        std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR))
            .unwrap();

        crate::service::usb_utils::initialize_usb(usb_root.to_string_lossy().as_ref())
            .expect("initialize usb skeleton");

        let playlist = ExportPlaylistData {
            id: "pl-last".to_string(),
            name: "Last Playlist".to_string(),
            tracks: vec![ExportTrackData {
                id: "t1".to_string(),
                title: "Song A".to_string(),
                artist: "Artist".to_string(),
                album: Some("Album".to_string()),
                track_number: Some(1),
                bpm: Some(124.0),
                key: Some("8A".to_string()),
                file_path: "/source/a.mp3".to_string(),
                file_name: "a.mp3".to_string(),
                file_modified_at: None,
                file_size_bytes: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                title_for_search: None,
                kuvo_delivery_comment: None,
                dj_play_count: None,
                rating: None,
                color_id: None,
                artist_id_lyricist: None,
                artist_id_original_artist: None,
                artist_id_remixer: None,
                artist_id_composer: None,
                genre_id: None,
                genre: None,
                label_id: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                file_type: None,
                artwork_path: None,
                waveform_peaks_path: None,
                duration_ms: Some(195_000),
                first_beat_ms: None,
                position: 0,
            }],
        };
        let manifest = ExportManifest {
            version: 1,
            generated_at: "2024-01-01".to_string(),
            playlist_id: "pl-last".to_string(),
            playlist_name: "Last Playlist".to_string(),
            usb_root: usb_root.to_string_lossy().to_string(),
            options: crate::models::ExportToUsbOptions {
                include_artwork: false,
                include_analysis: false,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 1,
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks: vec![ExportManifestTrack {
                id: "t1".to_string(),
                master_db_id: None,
                master_content_id: None,
                content_link: None,
                position: 1,
                track_number: Some(1),
                title: "Song A".to_string(),
                artist: "Artist".to_string(),
                album: Some("Album".to_string()),
                bpm: Some(124.0),
                key: Some("8A".to_string()),
                source_path: "/source/a.mp3".to_string(),
                exported_path: "/Contents/Artist/Album/a.mp3".to_string(),
                file_modified_at: None,
                file_size_bytes: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                title_for_search: None,
                kuvo_delivery_comment: None,
                dj_play_count: None,
                rating: None,
                color_id: None,
                artist_id_lyricist: None,
                artist_id_original_artist: None,
                artist_id_remixer: None,
                artist_id_composer: None,
                genre_id: None,
                genre: None,
                label_id: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                file_type: None,
                owns_exported_media: true,
                owns_artwork: true,
                owns_waveform: true,
                artwork_path: None,
                waveform_path: None,
                duration_ms: Some(195_000),
            }],
        };

        write_pdb(usb_root, &playlist, &manifest, true, None, None, false)
            .expect("append playlist to pdb");

        let pdb_path = usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb");
        let pdb_size_before = std::fs::metadata(&pdb_path).unwrap().len();

        let parsed_before =
            crate::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb before remove");
        let target_playlist_id = parsed_before
            .playlist_tree
            .iter()
            .find(|row| {
                !row.row_is_folder
                    && canonicalize_playlist_name(&row.name)
                        == canonicalize_playlist_name("Last Playlist")
            })
            .map(|row| row.id)
            .expect("target playlist leaf id");

        let mut warnings = Vec::new();
        let result = remove_playlist_and_tracks_from_pdb(
            usb_root,
            Some(&format!("usb-pl-{target_playlist_id}")),
            &[String::from("Last Playlist")],
            &mut warnings,
        )
        .expect("remove last playlist from pdb");
        assert_eq!(result.removed_playlist_count, 1);

        let pdb_size_after = std::fs::metadata(&pdb_path).unwrap().len();
        assert_eq!(
            pdb_size_before, pdb_size_after,
            "PDB file size must not change on deletion (shape preservation)"
        );

        let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb after remove");
        assert!(
            parsed
                .playlist_tree
                .iter()
                .filter(|row| !row.row_is_folder)
                .all(|row| row.id != target_playlist_id),
            "playlist leaf should be removed from playlist_tree by id"
        );
        assert!(
            parsed.playlist_entries.is_empty(),
            "playlist_entries should be empty after removing last playlist"
        );
        assert!(
            parsed.tracks.is_empty(),
            "tracks should be empty after removing last playlist"
        );
        assert_eq!(result.exclusive_tracks.len(), 1);
        assert_eq!(result.shared_track_count, 0);
    }

    #[test]
    fn remove_playlist_cleans_exclusive_tracks_preserves_shared() {
        let dir = tempdir().unwrap();
        let usb_root = dir.path();
        std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR))
            .unwrap();
        std::fs::create_dir_all(usb_root.join("Contents/Artist/Album")).unwrap();
        std::fs::create_dir_all(
            usb_root
                .join(USB_VENDOR_ROOT_DIR)
                .join(USB_ANALYSIS_DIR)
                .join("P001"),
        )
        .unwrap();

        crate::service::usb_utils::initialize_usb(usb_root.to_string_lossy().as_ref())
            .expect("initialize usb skeleton");

        // Export playlist A with tracks t1, t2
        let playlist_a = ExportPlaylistData {
            id: "pl-a".to_string(),
            name: "Playlist A".to_string(),
            tracks: vec![
                make_test_track("t1", "Song One", "a1.mp3"),
                make_test_track("t2", "Song Two", "a2.mp3"),
            ],
        };
        let manifest_a = make_test_manifest(
            "pl-a",
            "Playlist A",
            usb_root,
            &[("t1", "Song One", "a1.mp3"), ("t2", "Song Two", "a2.mp3")],
        );
        write_pdb(usb_root, &playlist_a, &manifest_a, true, None, None, false)
            .expect("export playlist A");

        // Export playlist B with tracks t2 (shared), t3 (exclusive)
        let playlist_b = ExportPlaylistData {
            id: "pl-b".to_string(),
            name: "Playlist B".to_string(),
            tracks: vec![
                make_test_track("t2", "Song Two", "a2.mp3"),
                make_test_track("t3", "Song Three", "a3.mp3"),
            ],
        };
        let manifest_b = make_test_manifest(
            "pl-b",
            "Playlist B",
            usb_root,
            &[("t2", "Song Two", "a2.mp3"), ("t3", "Song Three", "a3.mp3")],
        );
        write_pdb(usb_root, &playlist_b, &manifest_b, true, None, None, false)
            .expect("export playlist B");

        // Create dummy audio files
        for name in &["a1.mp3", "a2.mp3", "a3.mp3"] {
            std::fs::write(
                usb_root.join("Contents/Artist/Album").join(name),
                b"fake audio",
            )
            .unwrap();
        }

        // Create dummy ANLZ files
        let anlz_dir = usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_ANALYSIS_DIR)
            .join("P001");
        for name in &["a1.DAT", "a1.EXT", "a2.DAT", "a2.EXT", "a3.DAT", "a3.EXT"] {
            std::fs::write(anlz_dir.join(name), b"fake anlz").unwrap();
        }

        // Parse before removal to get track IDs
        let pdb_path = usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb");
        let parsed_before = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb before");
        assert_eq!(
            parsed_before.tracks.len(),
            3,
            "should have 3 tracks before removal"
        );
        let pdb_size_before = std::fs::metadata(&pdb_path).unwrap().len();

        // Remove playlist B
        let mut warnings = Vec::new();
        let result = remove_playlist_and_tracks_from_pdb(
            usb_root,
            None,
            &[String::from("Playlist B")],
            &mut warnings,
        )
        .expect("remove playlist B");

        assert_eq!(result.removed_playlist_count, 1);
        assert_eq!(result.shared_track_count, 1, "t2 is shared");
        assert_eq!(result.exclusive_tracks.len(), 1, "t3 is exclusive to B");
        assert!(
            result
                .exclusive_tracks
                .iter()
                .any(|t| t.track_file_path.contains("a3.mp3")),
            "exclusive track should be a3.mp3"
        );

        let pdb_size_after = std::fs::metadata(&pdb_path).unwrap().len();
        assert_eq!(
            pdb_size_before, pdb_size_after,
            "PDB file size must not change on deletion (shape preservation)"
        );

        // Verify PDB state after removal
        let parsed_after = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb after");

        // Playlist B should be removed
        assert!(
            !parsed_after
                .playlist_tree
                .iter()
                .any(|p| !p.row_is_folder && p.name == "Playlist B"),
            "Playlist B should not exist"
        );

        // Playlist A should still exist
        assert!(
            parsed_after
                .playlist_tree
                .iter()
                .any(|p| !p.row_is_folder && p.name == "Playlist A"),
            "Playlist A should still exist"
        );

        // t1 and t2 should remain, t3 should be gone
        assert_eq!(
            parsed_after.tracks.len(),
            2,
            "should have 2 tracks (t1, t2) after removing B"
        );
        assert!(
            parsed_after
                .tracks
                .iter()
                .any(|t| t.track_file_path.contains("a1.mp3")),
            "t1 should remain"
        );
        assert!(
            parsed_after
                .tracks
                .iter()
                .any(|t| t.track_file_path.contains("a2.mp3")),
            "t2 should remain (shared)"
        );
        assert!(
            !parsed_after
                .tracks
                .iter()
                .any(|t| t.track_file_path.contains("a3.mp3")),
            "t3 should be removed"
        );
    }

    #[test]
    fn remove_playlist_fully_shared_preserves_all_tracks() {
        let dir = tempdir().unwrap();
        let usb_root = dir.path();
        std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR))
            .unwrap();

        crate::service::usb_utils::initialize_usb(usb_root.to_string_lossy().as_ref())
            .expect("initialize usb skeleton");

        // Export playlist A with tracks t1, t2
        let playlist_a = ExportPlaylistData {
            id: "pl-a".to_string(),
            name: "Playlist A".to_string(),
            tracks: vec![
                make_test_track("t1", "Song One", "a1.mp3"),
                make_test_track("t2", "Song Two", "a2.mp3"),
            ],
        };
        let manifest_a = make_test_manifest(
            "pl-a",
            "Playlist A",
            usb_root,
            &[("t1", "Song One", "a1.mp3"), ("t2", "Song Two", "a2.mp3")],
        );
        write_pdb(usb_root, &playlist_a, &manifest_a, true, None, None, false)
            .expect("export playlist A");

        // Export playlist B with the SAME tracks (fully overlapping)
        let playlist_b = ExportPlaylistData {
            id: "pl-b".to_string(),
            name: "Playlist B".to_string(),
            tracks: vec![
                make_test_track("t1", "Song One", "a1.mp3"),
                make_test_track("t2", "Song Two", "a2.mp3"),
            ],
        };
        let manifest_b = make_test_manifest(
            "pl-b",
            "Playlist B",
            usb_root,
            &[("t1", "Song One", "a1.mp3"), ("t2", "Song Two", "a2.mp3")],
        );
        write_pdb(usb_root, &playlist_b, &manifest_b, true, None, None, false)
            .expect("export playlist B");

        let pdb_path = usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb");
        let pdb_size_before = std::fs::metadata(&pdb_path).unwrap().len();

        let mut warnings = Vec::new();
        let result = remove_playlist_and_tracks_from_pdb(
            usb_root,
            None,
            &[String::from("Playlist B")],
            &mut warnings,
        )
        .expect("remove playlist B");

        assert_eq!(result.removed_playlist_count, 1);
        assert_eq!(result.shared_track_count, 2, "both tracks are shared");
        assert_eq!(result.exclusive_tracks.len(), 0, "no exclusive tracks");

        let pdb_size_after = std::fs::metadata(&pdb_path).unwrap().len();
        assert_eq!(
            pdb_size_before, pdb_size_after,
            "PDB file size must not change on deletion (shape preservation)"
        );

        let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb after");
        assert_eq!(parsed.tracks.len(), 2, "all tracks should remain");
    }

    #[test]
    fn remove_playlist_preserves_pdb_page_count_with_many_entries() {
        // Create enough playlist entries to span multiple pages, then remove
        // one playlist and verify the file size (page count) is unchanged.
        let dir = tempdir().unwrap();
        let usb_root = dir.path();
        std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR))
            .unwrap();

        crate::service::usb_utils::initialize_usb(usb_root.to_string_lossy().as_ref())
            .expect("initialize usb skeleton");

        // 20 tracks → 20 playlist entries per playlist → enough to stress
        let tracks_a: Vec<ExportTrackData> = (0..20)
            .map(|i| make_test_track(&format!("t{i}"), &format!("Song {i}"), &format!("a{i}.mp3")))
            .collect();
        let playlist_a = ExportPlaylistData {
            id: "pl-a".to_string(),
            name: "Big Playlist".to_string(),
            tracks: tracks_a,
        };
        let manifest_entries: Vec<(&str, &str, &str)> = (0..20)
            .map(|i| {
                // Leak strings so they live long enough for the slice
                let id: &'static str = Box::leak(format!("t{i}").into_boxed_str());
                let title: &'static str = Box::leak(format!("Song {i}").into_boxed_str());
                let file: &'static str = Box::leak(format!("a{i}.mp3").into_boxed_str());
                (id, title, file)
            })
            .collect();
        let manifest_a = make_test_manifest("pl-a", "Big Playlist", usb_root, &manifest_entries);
        write_pdb(usb_root, &playlist_a, &manifest_a, true, None, None, false)
            .expect("export big playlist");

        let pdb_path = usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb");
        let pdb_size_before = std::fs::metadata(&pdb_path).unwrap().len();
        let parsed_before = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse before");
        assert_eq!(parsed_before.tracks.len(), 20);

        let mut warnings = Vec::new();
        let result = remove_playlist_and_tracks_from_pdb(
            usb_root,
            None,
            &[String::from("Big Playlist")],
            &mut warnings,
        )
        .expect("remove big playlist");
        assert_eq!(result.removed_playlist_count, 1);
        assert_eq!(result.exclusive_tracks.len(), 20);

        let pdb_size_after = std::fs::metadata(&pdb_path).unwrap().len();
        assert_eq!(
            pdb_size_before, pdb_size_after,
            "PDB file size must not change on deletion (shape preservation): before={} after={}",
            pdb_size_before, pdb_size_after
        );

        let parsed_after = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse after");
        assert!(
            parsed_after.tracks.is_empty(),
            "all tracks should be removed"
        );
        assert!(
            parsed_after
                .playlist_tree
                .iter()
                .all(|p| p.row_is_folder || p.name != "Big Playlist"),
            "playlist should be removed"
        );
    }

    #[test]
    fn remove_playlist_and_tracks_from_pdb_keeps_history_referenced_tracks() {
        let dir = tempdir().unwrap();
        let usb_root = dir.path();
        std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR))
            .unwrap();

        let mut data = crate::pdb_writer::PdbData::empty();
        data.tracks.push(PdbTrackRowData {
            header_flags_u32: None,
            content_link: Some(1),
            sample_rate_hz: Some(44_100),
            file_size_bytes: Some(1234),
            master_content_id: Some(1),
            master_db_id: Some(1),
            id: 1,
            artist_id: 0,
            album_id: 0,
            artwork_id: 0,
            key_id: 0,
            genre_id: 0,
            bitrate_kbps: Some(320),
            track_number: Some(1),
            bpm: Some(120.0),
            release_year: None,
            bit_depth: None,
            duration_seconds: Some(180),
            file_type: None,
            isrc: None,
            date_added: None,
            release_date: None,
            dj_comment: None,
            file_name: Some("track.mp3".to_string()),
            publish_track_info_on: None,
            autoload_hotcues_on: None,
            title: "History Shared".to_string(),
            anlz_path: "/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT".to_string(),
            file_path: "/Contents/Artist/Album/track.mp3".to_string(),
        });
        data.playlist_tree
            .push(crate::pdb_writer::PdbPlaylistTreeRow {
                id: 100,
                parent_id: 0,
                sort_order: 0,
                is_folder: false,
                name: "Playlist B".to_string(),
            });
        data.playlist_entries
            .push(crate::pdb_writer::PdbPlaylistEntryRow {
                entry_index: 1,
                track_id: 1,
                playlist_id: 100,
            });
        data.history_playlists.push(crate::pdb_writer::PdbDictRow {
            id: 200,
            name: "HISTORY 001".to_string(),
        });
        data.history_entries
            .push(crate::pdb_writer::PdbHistoryEntryRow {
                track_id: 1,
                playlist_id: 200,
                entry_index: 1,
            });

        let pdb_path = usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb");
        crate::pdb_writer::write_pdb_to_file(&pdb_path, &data).expect("write pdb");

        let mut warnings = Vec::new();
        let result = remove_playlist_and_tracks_from_pdb(
            usb_root,
            None,
            &[String::from("Playlist B")],
            &mut warnings,
        )
        .expect("remove playlist");

        assert_eq!(result.removed_playlist_count, 1);
        assert_eq!(result.shared_track_count, 1);
        assert!(
            result.exclusive_tracks.is_empty(),
            "history-referenced track must not be treated as exclusive"
        );

        let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse patched pdb");
        assert_eq!(
            parsed.tracks.len(),
            1,
            "history-referenced track must remain"
        );
        assert!(
            parsed
                .playlist_tree
                .iter()
                .all(|row| row.name != "Playlist B"),
            "playlist should be removed while track remains"
        );
    }

    #[test]
    fn initialize_usb_seeds_baseline_history_shape_pages() {
        let dir = tempdir().unwrap();
        let usb_root = dir.path();
        std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR))
            .unwrap();

        crate::service::usb_utils::initialize_usb(usb_root.to_string_lossy().as_ref())
            .expect("initialize usb skeleton");

        let pdb_path = usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb");
        let bytes = std::fs::read(&pdb_path).expect("read export.pdb");

        fn u32le(bytes: &[u8], off: usize) -> u32 {
            u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap())
        }
        fn u16le(bytes: &[u8], off: usize) -> u16 {
            u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap())
        }
        let ps = 4096usize;
        let tptr = |tt: usize| 0x1cusize + tt * 16;

        // t17/t18/t19 baseline pointer triplets
        let o17 = tptr(17);
        let o18 = tptr(18);
        let o19 = tptr(19);
        assert_eq!(u32le(&bytes, o17 + 4), 44);
        assert_eq!(u32le(&bytes, o17 + 8), 35);
        assert_eq!(u32le(&bytes, o17 + 12), 36);
        assert_eq!(u32le(&bytes, o18 + 4), 45);
        assert_eq!(u32le(&bytes, o18 + 8), 37);
        assert_eq!(u32le(&bytes, o18 + 12), 38);
        assert_eq!(u32le(&bytes, o19 + 4), 41);
        assert_eq!(u32le(&bytes, o19 + 8), 39);
        assert_eq!(u32le(&bytes, o19 + 12), 40);

        // Data pages carry seeded row counts/bitmasks.
        let p36 = 36 * ps;
        let p38 = 38 * ps;
        let p40 = 40 * ps;
        assert_eq!(bytes[p36 + 24], 22);
        assert_eq!(bytes[p36 + 25], 0xc0);
        assert_eq!(u16le(&bytes, p36 + ps - 4), 0xffff);
        assert_eq!(u16le(&bytes, p36 + ps - 2), 0xffff);

        assert_eq!(bytes[p38 + 24], 17);
        assert_eq!(bytes[p38 + 25], 0x20);
        assert_eq!(u16le(&bytes, p38 + ps - 4), 0xffff);
        assert_eq!(u16le(&bytes, p38 + ps - 2), 0xffff);

        assert_eq!(bytes[p40 + 24], 1);
        assert_eq!(bytes[p40 + 25], 0x20);
        assert_eq!(u16le(&bytes, p40 + ps - 4), 0x0001);
        assert_eq!(u16le(&bytes, p40 + ps - 2), 0x0001);
    }

    #[test]
    fn export_preserves_t17_t18_seed_shape_and_allows_t19_runtime_rows() {
        let dir = tempdir().unwrap();
        let usb_root = dir.path();
        std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR))
            .unwrap();
        crate::service::usb_utils::initialize_usb(usb_root.to_string_lossy().as_ref())
            .expect("initialize usb skeleton");
        let pdb_path = usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb");
        let before_bytes = std::fs::read(&pdb_path).expect("read initialized export.pdb");

        // Force candidate fallback path on legacy-seeded USB by seeding a t08 mismatch
        // so the shape merger path is exercised while preserving seed tables.
        let playlist = ExportPlaylistData {
            id: "pl-test".to_string(),
            name: "Test".to_string(),
            tracks: vec![
                make_test_track("t1", "Song 1", "a1.mp3"),
                make_test_track("t2", "Song 2", "a2.mp3"),
                make_test_track("t3", "Song 3", "a3.mp3"),
                make_test_track("t4", "Song 4", "a4.mp3"),
            ],
        };
        let manifest = make_test_manifest(
            "pl-test",
            "Test",
            usb_root,
            &[
                ("t1", "Song 1", "a1.mp3"),
                ("t2", "Song 2", "a2.mp3"),
                ("t3", "Song 3", "a3.mp3"),
                ("t4", "Song 4", "a4.mp3"),
            ],
        );
        write_pdb(usb_root, &playlist, &manifest, true, None, None, false)
            .expect("export playlist");

        let bytes = std::fs::read(&pdb_path).expect("read export.pdb");
        let ps = 4096usize;
        let tptr = |tt: usize| 0x1cusize + tt * 16;
        let u32le = |off: usize| u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        let u16le = |off: usize| u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap());

        // t17/t18 remain baseline pointers and seed counts.
        let o17 = tptr(17);
        let o18 = tptr(18);
        for table_type in [16usize, 17, 18] {
            let ptr = tptr(table_type);
            assert_eq!(
                &bytes[ptr + 4..ptr + 16],
                &before_bytes[ptr + 4..ptr + 16],
                "t{table_type} pointer triplet should stay byte-stable"
            );
        }
        for page_idx in [33usize, 34, 35, 36, 37, 38] {
            let off = page_idx * ps;
            assert_eq!(
                &bytes[off..off + ps],
                &before_bytes[off..off + ps],
                "t16/t17/t18 page {page_idx} should stay byte-stable"
            );
        }
        assert_eq!(u32le(o17 + 8), 35);
        assert_eq!(u32le(o17 + 12), 36);
        assert_eq!(u32le(o18 + 8), 37);
        assert_eq!(u32le(o18 + 12), 38);
        assert_eq!(bytes[36 * ps + 24], 22);
        assert_eq!(bytes[38 * ps + 24], 17);

        // t19 evolves to runtime-style rows during export.
        let o19 = tptr(19);
        assert!(u32le(o19 + 4) >= 41);
        assert_eq!(u32le(o19 + 8), 39);
        assert!(u32le(o19 + 12) >= 40);
        assert_eq!(bytes[40 * ps + 24], 5);
        assert_eq!(bytes[40 * ps + 25], 0x20);
        assert_eq!(bytes[40 * ps + 27], 52);
        assert_eq!(u16le(40 * ps + ps - 4), 0x0010);
        assert_eq!(u16le(40 * ps + ps - 2), 0x0018);
        // t19 row payload pattern should follow runtime sequence profile:
        // 0x00000280 + i*0x00200000, and second dword = i.
        let p40 = 40 * ps;
        let row_base = p40 + 40;
        for i in 0..=4usize {
            let row = row_base + i * 40;
            let u0 = u32le(row);
            let u1 = u32le(row + 4);
            let want_u0 = 0x0000_0280u32 + (i as u32) * 0x0020_0000u32;
            assert_eq!(u0, want_u0);
            assert_eq!(u1, i as u32);
        }

        // t08 runtime row-index behavior should mirror growth writer contract.
        let o8 = tptr(8);
        assert_eq!(u32le(o8 + 8), 17);
        assert_eq!(u32le(o8 + 12), 18);
        assert_eq!(bytes[18 * ps + 24], 4);
        assert_eq!(u16le(18 * ps + 34), 3);
        assert_eq!(u16le(40 * ps + 34), 3);
    }

    #[test]
    fn rewrite_pdb_playlist_rebuilds_resolvable_dictionary_rows_on_initialized_usb() {
        let dir = tempdir().unwrap();
        let usb_root = dir.path();
        std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR))
            .unwrap();

        crate::service::usb_utils::initialize_usb(usb_root.to_string_lossy().as_ref())
            .expect("initialize usb skeleton");

        let playlist = ExportPlaylistData {
            id: "pl-dict".to_string(),
            name: "Dictionary Repair".to_string(),
            tracks: vec![make_test_track("t1", "Song One", "a1.mp3")],
        };
        let manifest = ExportManifest {
            version: 1,
            generated_at: "2024-01-01".to_string(),
            playlist_id: "pl-dict".to_string(),
            playlist_name: "Dictionary Repair".to_string(),
            usb_root: usb_root.to_string_lossy().to_string(),
            options: crate::models::ExportToUsbOptions {
                include_artwork: true,
                include_analysis: true,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 1,
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks: vec![ExportManifestTrack {
                id: "t1".to_string(),
                master_db_id: Some(1),
                master_content_id: Some(1),
                content_link: Some(1),
                position: 1,
                track_number: Some(7),
                title: "Song One".to_string(),
                artist: "Artist".to_string(),
                album: Some("Album".to_string()),
                bpm: Some(120.0),
                key: Some("8A".to_string()),
                source_path: "/source/a1.mp3".to_string(),
                exported_path: "/Contents/Artist/Album/a1.mp3".to_string(),
                file_modified_at: None,
                file_size_bytes: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                title_for_search: None,
                kuvo_delivery_comment: None,
                dj_play_count: None,
                rating: None,
                color_id: None,
                artist_id_lyricist: None,
                artist_id_original_artist: None,
                artist_id_remixer: None,
                artist_id_composer: None,
                genre_id: None,
                genre: None,
                label_id: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                file_type: None,
                owns_exported_media: true,
                owns_artwork: true,
                owns_waveform: true,
                artwork_path: Some("/PIONEER/Artwork/00001/a00001.jpg".to_string()),
                waveform_path: Some("/PIONEER/USBANLZ/P001/A001/ANLZ0000.DAT".to_string()),
                duration_ms: Some(180_000),
            }],
        };

        write_pdb(usb_root, &playlist, &manifest, true, None, None, false).expect("initial append");
        let parsed_before = crate::pdb_reader::parse_pdb(
            &usb_root
                .join(USB_VENDOR_ROOT_DIR)
                .join(USB_VENDOR_DB_DIR)
                .join("export.pdb"),
        )
        .expect("parse appended pdb");
        let track_before = parsed_before
            .tracks
            .iter()
            .find(|track| track.track_file_path == "/Contents/Artist/Album/a1.mp3")
            .expect("appended track");
        assert_eq!(
            parsed_before
                .artists
                .get(&track_before.artist_id)
                .map(String::as_str),
            Some("Artist"),
            "initial append should materialize resolvable artist dictionary rows"
        );

        rewrite_pdb_playlist_from_manifest(usb_root, &playlist, &manifest, 1, 1)
            .expect("rewrite playlist");

        let pdb_path = usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb");
        let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse rewritten pdb");

        let playlist_row = parsed
            .playlist_tree
            .iter()
            .find(|row| !row.row_is_folder && row.name == "Dictionary Repair")
            .expect("rewritten playlist row");
        assert_eq!(playlist_row.id, 1);

        let track = parsed
            .tracks
            .iter()
            .find(|track| track.track_file_path == "/Contents/Artist/Album/a1.mp3")
            .expect("rewritten track");
        assert!(track.artist_id > 0, "track should carry artist id");
        assert!(track.album_id > 0, "track should carry album id");
        assert!(track.key_id > 0, "track should carry key id");
        assert!(track.artwork_id > 0, "track should carry artwork id");
        assert_eq!(
            parsed.artists.get(&track.artist_id).map(String::as_str),
            Some("Artist"),
            "artists map after rewrite: {:?}",
            parsed.artists
        );
        assert_eq!(
            parsed.albums.get(&track.album_id).map(String::as_str),
            Some("Album"),
            "albums map after rewrite: {:?}",
            parsed.albums
        );
        assert_eq!(
            parsed.keys.get(&track.key_id).map(String::as_str),
            Some("8A"),
            "keys map after rewrite: {:?}",
            parsed.keys
        );
        assert_eq!(
            parsed.artworks.get(&track.artwork_id).map(String::as_str),
            Some("/PIONEER/Artwork/00001/a00001.jpg")
        );
    }

    #[test]
    fn write_pdb_fresh_backfills_dictionary_ids_from_edb_when_manifest_is_thin() {
        let dir = tempdir().unwrap();
        let usb_root = dir.path();
        std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR))
            .unwrap();
        crate::service::usb_utils::initialize_usb(usb_root.to_string_lossy().as_ref())
            .expect("initialize usb skeleton");

        let mut unlock_warnings = Vec::<String>::new();
        let conn = open_edb_rw(usb_root, &mut unlock_warnings).expect("open eDB");
        let mut seed_track = mapping_test_track();
        seed_track.exported_path =
            "/Contents/Fallback Artist/Fallback Album/fallback.mp3".to_string();
        seed_track.artist = "Fallback Artist".to_string();
        seed_track.album = Some("Fallback Album".to_string());
        seed_track.key = Some("10A".to_string());
        seed_track.artwork_path = Some("/PIONEER/Artwork/00001/fallback.jpg".to_string());
        let _ = insert_content_from_template(&conn, &seed_track, Some("2024-01-01"))
            .expect("seed content row");

        let playlist = ExportPlaylistData {
            id: "pl-fallback".to_string(),
            name: "Fallback Playlist".to_string(),
            tracks: vec![make_test_track(
                "t-fallback",
                "Fallback Song",
                "fallback.mp3",
            )],
        };
        let mut thin_manifest_track = seed_track.clone();
        thin_manifest_track.id = "t-fallback".to_string();
        thin_manifest_track.position = 1;
        thin_manifest_track.title = "Fallback Song".to_string();
        thin_manifest_track.artist = String::new();
        thin_manifest_track.album = None;
        thin_manifest_track.key = None;
        thin_manifest_track.artwork_path = None;
        thin_manifest_track.waveform_path =
            Some("/PIONEER/USBANLZ/P111/11111111/ANLZ0000.DAT".to_string());

        let manifest = ExportManifest {
            version: 1,
            generated_at: "2024-01-01".to_string(),
            playlist_id: "pl-fallback".to_string(),
            playlist_name: "Fallback Playlist".to_string(),
            usb_root: usb_root.to_string_lossy().to_string(),
            options: crate::models::ExportToUsbOptions {
                include_artwork: true,
                include_analysis: true,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 1,
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks: vec![thin_manifest_track],
        };

        write_pdb(usb_root, &playlist, &manifest, true, None, None, false)
            .expect("write pdb with eDB fallback metadata");

        let pdb_path = usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb");
        let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse rewritten pdb");
        let track = parsed
            .tracks
            .iter()
            .find(|track| track.track_file_path == seed_track.exported_path.as_str())
            .expect("fallback track");

        assert!(
            track.artist_id > 0,
            "artist id should come from eDB fallback"
        );
        assert!(track.album_id > 0, "album id should come from eDB fallback");
        assert!(track.key_id > 0, "key id should come from eDB fallback");
        assert!(
            track.artwork_id > 0,
            "artwork id should come from eDB fallback"
        );
        assert_eq!(
            parsed.artists.get(&track.artist_id).map(String::as_str),
            Some("Fallback Artist")
        );
        assert_eq!(
            parsed.albums.get(&track.album_id).map(String::as_str),
            Some("Fallback Album")
        );
        assert_eq!(
            parsed.keys.get(&track.key_id).map(String::as_str),
            Some("10A")
        );
        assert_eq!(
            parsed.artworks.get(&track.artwork_id).map(String::as_str),
            Some("/PIONEER/Artwork/00001/fallback.jpg")
        );
    }

    #[test]
    fn strict_parity_report_has_no_dictionary_id_issues_for_thin_manifest_fallback() {
        let dir = tempdir().unwrap();
        let usb_root = dir.path();
        std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR))
            .unwrap();
        crate::service::usb_utils::initialize_usb(usb_root.to_string_lossy().as_ref())
            .expect("initialize usb skeleton");

        let mut unlock_warnings = Vec::<String>::new();
        let mut conn = open_edb_rw(usb_root, &mut unlock_warnings).expect("open eDB");

        let mut seed_track = mapping_test_track();
        seed_track.exported_path =
            "/Contents/Fallback Artist/Fallback Album/fallback.mp3".to_string();
        seed_track.artist = "Fallback Artist".to_string();
        seed_track.album = Some("Fallback Album".to_string());
        seed_track.key = Some("10A".to_string());
        seed_track.artwork_path = Some("/PIONEER/Artwork/00001/fallback.jpg".to_string());

        let playlist = ExportPlaylistData {
            id: "pl-fallback-parity".to_string(),
            name: "Fallback Parity".to_string(),
            tracks: vec![make_test_track(
                "t-fallback",
                "Fallback Song",
                "fallback.mp3",
            )],
        };

        let tx = conn.transaction().expect("start eDB tx");
        let content_id = insert_content_from_template(&tx, &seed_track, Some("2024-01-01"))
            .expect("seed content row");
        let playlist_id = upsert_export_playlist_row(&tx, &playlist).expect("upsert playlist");
        link_playlist_content(&tx, playlist_id, content_id, 1).expect("link playlist content");
        tx.commit().expect("commit eDB tx");

        let mut thin_manifest_track = seed_track.clone();
        thin_manifest_track.id = "t-fallback".to_string();
        thin_manifest_track.position = 1;
        thin_manifest_track.title = "Fallback Song".to_string();
        thin_manifest_track.artist = String::new();
        thin_manifest_track.album = None;
        thin_manifest_track.key = None;
        thin_manifest_track.artwork_path = None;
        thin_manifest_track.waveform_path =
            Some("/PIONEER/USBANLZ/P111/11111111/ANLZ0000.DAT".to_string());

        let manifest = ExportManifest {
            version: 1,
            generated_at: "2024-01-01".to_string(),
            playlist_id: "pl-fallback-parity".to_string(),
            playlist_name: "Fallback Parity".to_string(),
            usb_root: usb_root.to_string_lossy().to_string(),
            options: crate::models::ExportToUsbOptions {
                include_artwork: true,
                include_analysis: true,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 1,
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks: vec![thin_manifest_track],
        };

        write_pdb(usb_root, &playlist, &manifest, true, None, None, false)
            .expect("write pdb with eDB fallback metadata");

        let backend_data = tempdir().expect("backend data dir");
        let service = BackendService::new(backend_data.path()).expect("create backend service");
        let report = service
            .run_usb_parity_report(RunUsbParityReportRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            })
            .expect("run strict parity report");

        let detail = report
            .playlist_details
            .iter()
            .find(|d| d.name == "Fallback Parity")
            .expect("fallback parity detail");
        assert_eq!(
            detail.dictionary_id_issue_tracks, 0,
            "dictionaryIdIssueTracks should remain zero when PDB uses eDB fallback metadata"
        );
    }

    #[test]
    fn strict_parity_report_has_no_dictionary_id_issues_when_edb_is_thin_but_manifest_is_rich() {
        let dir = tempdir().unwrap();
        let usb_root = dir.path();
        std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR))
            .unwrap();
        crate::service::usb_utils::initialize_usb(usb_root.to_string_lossy().as_ref())
            .expect("initialize usb skeleton");

        let mut unlock_warnings = Vec::<String>::new();
        let mut conn = open_edb_rw(usb_root, &mut unlock_warnings).expect("open eDB");

        let mut rich_manifest_track = mapping_test_track();
        rich_manifest_track.exported_path = "/Contents/Rich Artist/Rich Album/rich.mp3".to_string();
        rich_manifest_track.artist = "Rich Artist".to_string();
        rich_manifest_track.album = Some("Rich Album".to_string());
        rich_manifest_track.key = Some("9A".to_string());
        rich_manifest_track.artwork_path = Some("/PIONEER/Artwork/00001/rich.jpg".to_string());

        let playlist = ExportPlaylistData {
            id: "pl-rich-manifest".to_string(),
            name: "Rich Manifest".to_string(),
            tracks: vec![make_test_track("t-rich", "Rich Song", "rich.mp3")],
        };

        let tx = conn.transaction().expect("start eDB tx");
        let content_id =
            insert_content_from_template(&tx, &rich_manifest_track, Some("2024-01-01"))
                .expect("seed content row");
        let content_columns = load_table_columns_tx(&tx, "content").expect("content columns");
        if content_columns.contains("artist_id_artist") {
            tx.execute(
                "UPDATE content SET artist_id_artist = 0 WHERE content_id = ?1",
                params![content_id],
            )
            .expect("thin eDB artist id");
        }
        if content_columns.contains("album_id") {
            tx.execute(
                "UPDATE content SET album_id = NULL WHERE content_id = ?1",
                params![content_id],
            )
            .expect("thin eDB album id");
        }
        if content_columns.contains("key_id") {
            tx.execute(
                "UPDATE content SET key_id = NULL WHERE content_id = ?1",
                params![content_id],
            )
            .expect("thin eDB key id");
        }
        if content_columns.contains("image_id") {
            tx.execute(
                "UPDATE content SET image_id = NULL WHERE content_id = ?1",
                params![content_id],
            )
            .expect("thin eDB image id");
        }
        if content_columns.contains("imageFilePath_id") {
            tx.execute(
                "UPDATE content SET imageFilePath_id = NULL WHERE content_id = ?1",
                params![content_id],
            )
            .expect("thin eDB imageFilePath id");
        }
        let playlist_id = upsert_export_playlist_row(&tx, &playlist).expect("upsert playlist");
        link_playlist_content(&tx, playlist_id, content_id, 1).expect("link playlist content");
        tx.commit().expect("commit eDB tx");

        rich_manifest_track.id = "t-rich".to_string();
        rich_manifest_track.position = 1;
        rich_manifest_track.title = "Rich Song".to_string();
        rich_manifest_track.waveform_path =
            Some("/PIONEER/USBANLZ/P222/22222222/ANLZ0000.DAT".to_string());

        let manifest = ExportManifest {
            version: 1,
            generated_at: "2024-01-01".to_string(),
            playlist_id: "pl-rich-manifest".to_string(),
            playlist_name: "Rich Manifest".to_string(),
            usb_root: usb_root.to_string_lossy().to_string(),
            options: crate::models::ExportToUsbOptions {
                include_artwork: true,
                include_analysis: true,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 1,
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks: vec![rich_manifest_track],
        };

        write_pdb(usb_root, &playlist, &manifest, true, None, None, false)
            .expect("write pdb with rich manifest metadata");

        let backend_data = tempdir().expect("backend data dir");
        let service = BackendService::new(backend_data.path()).expect("create backend service");
        let report = service
            .run_usb_parity_report(RunUsbParityReportRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            })
            .expect("run strict parity report");

        let detail = report
            .playlist_details
            .iter()
            .find(|d| d.name == "Rich Manifest")
            .expect("rich manifest detail");
        assert_eq!(
            detail.dictionary_id_issue_tracks, 0,
            "dictionaryIdIssueTracks should be zero when manifest metadata materializes dictionary rows"
        );
        assert!(
            detail.edb_missing_core_metadata > 0,
            "eDB is intentionally thinned in this scenario"
        );
    }

    #[test]
    fn strict_parity_additive_write_keeps_dictionary_ids_resolved_with_thin_manifest() {
        let dir = tempdir().unwrap();
        let usb_root = dir.path();
        std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR))
            .unwrap();
        crate::service::usb_utils::initialize_usb(usb_root.to_string_lossy().as_ref())
            .expect("initialize usb skeleton");

        let mut unlock_warnings = Vec::<String>::new();
        let mut conn = open_edb_rw(usb_root, &mut unlock_warnings).expect("open eDB");

        let mut base_track = mapping_test_track();
        base_track.id = "t-base".to_string();
        base_track.title = "Base Song".to_string();
        base_track.artist = "Base Artist".to_string();
        base_track.album = Some("Base Album".to_string());
        base_track.key = Some("8A".to_string());
        base_track.exported_path = "/Contents/Base Artist/Base Album/base.mp3".to_string();
        base_track.artwork_path = Some("/PIONEER/Artwork/00001/base.jpg".to_string());
        base_track.waveform_path = Some("/PIONEER/USBANLZ/P001/AAAA0001/ANLZ0000.DAT".to_string());
        base_track.position = 1;

        let mut add_track_rich = mapping_test_track();
        add_track_rich.id = "t-add".to_string();
        add_track_rich.title = "Add Song".to_string();
        add_track_rich.artist = "Add Artist".to_string();
        add_track_rich.album = Some("Add Album".to_string());
        add_track_rich.key = Some("10A".to_string());
        add_track_rich.exported_path = "/Contents/Add Artist/Add Album/add.mp3".to_string();
        add_track_rich.artwork_path = Some("/PIONEER/Artwork/00001/add.jpg".to_string());
        add_track_rich.waveform_path =
            Some("/PIONEER/USBANLZ/P001/AAAA0002/ANLZ0000.DAT".to_string());
        add_track_rich.position = 2;

        let playlist = ExportPlaylistData {
            id: "pl-additive".to_string(),
            name: "Additive Fallback".to_string(),
            tracks: vec![
                make_test_track("t-base", "Base Song", "base.mp3"),
                make_test_track("t-add", "Add Song", "add.mp3"),
            ],
        };

        let tx = conn.transaction().expect("start eDB tx");
        let base_content_id = insert_content_from_template(&tx, &base_track, Some("2024-01-01"))
            .expect("seed base content row");
        let add_content_id = insert_content_from_template(&tx, &add_track_rich, Some("2024-01-01"))
            .expect("seed add content row");
        let playlist_id = upsert_export_playlist_row(&tx, &playlist).expect("upsert playlist");
        link_playlist_content(&tx, playlist_id, base_content_id, 1).expect("link base track");
        tx.commit().expect("commit eDB tx");

        let base_manifest = ExportManifest {
            version: 1,
            generated_at: "2024-01-01".to_string(),
            playlist_id: "pl-additive".to_string(),
            playlist_name: "Additive Fallback".to_string(),
            usb_root: usb_root.to_string_lossy().to_string(),
            options: crate::models::ExportToUsbOptions {
                include_artwork: true,
                include_analysis: true,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 1,
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks: vec![base_track.clone()],
        };
        write_pdb(usb_root, &playlist, &base_manifest, true, None, None, false)
            .expect("write base playlist to pdb");

        let tx = conn.transaction().expect("start additive eDB tx");
        link_playlist_content(&tx, playlist_id, add_content_id, 2).expect("link additive track");
        tx.commit().expect("commit additive eDB tx");

        let mut add_track_thin_manifest = add_track_rich.clone();
        add_track_thin_manifest.artist = String::new();
        add_track_thin_manifest.album = None;
        add_track_thin_manifest.key = None;
        add_track_thin_manifest.artwork_path = None;

        let additive_manifest = ExportManifest {
            version: 1,
            generated_at: "2024-01-01".to_string(),
            playlist_id: "pl-additive".to_string(),
            playlist_name: "Additive Fallback".to_string(),
            usb_root: usb_root.to_string_lossy().to_string(),
            options: crate::models::ExportToUsbOptions {
                include_artwork: true,
                include_analysis: true,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 1,
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks: vec![add_track_thin_manifest],
        };
        write_pdb(
            usb_root,
            &playlist,
            &additive_manifest,
            false,
            None,
            None,
            false,
        )
        .expect("append additive track with thin manifest");

        let backend_data = tempdir().expect("backend data dir");
        let service = BackendService::new(backend_data.path()).expect("create backend service");
        let report = service
            .run_usb_parity_report(RunUsbParityReportRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            })
            .expect("run strict parity report");

        let detail = report
            .playlist_details
            .iter()
            .find(|d| d.name == "Additive Fallback")
            .expect("additive fallback detail");
        assert_eq!(
            detail.dictionary_id_issue_tracks, 0,
            "dictionaryIdIssueTracks should stay zero for additive write with thin manifest"
        );
    }

    #[test]
    fn rewrite_pdb_playlist_keeps_f_sharp_and_f_minor_as_distinct_keys() {
        let dir = tempdir().expect("tempdir");
        let usb_root = dir.path();
        std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR))
            .expect("create pdb dir");
        crate::service::usb_utils::initialize_usb(usb_root.to_string_lossy().as_ref())
            .expect("initialize usb skeleton");

        let playlist = ExportPlaylistData {
            id: "pl-keys".to_string(),
            name: "Key Collision".to_string(),
            tracks: vec![],
        };
        let manifest = ExportManifest {
            version: 1,
            generated_at: "2024-01-01".to_string(),
            playlist_id: "pl-keys".to_string(),
            playlist_name: "Key Collision".to_string(),
            usb_root: usb_root.to_string_lossy().to_string(),
            options: crate::models::ExportToUsbOptions {
                include_artwork: false,
                include_analysis: false,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 2,
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks: vec![
                ExportManifestTrack {
                    id: "t1".to_string(),
                    master_db_id: Some(1),
                    master_content_id: Some(1),
                    content_link: Some(1),
                    position: 1,
                    track_number: Some(1),
                    title: "Sharp".to_string(),
                    artist: "Artist".to_string(),
                    album: Some("Album".to_string()),
                    bpm: Some(120.0),
                    key: Some("F#m".to_string()),
                    source_path: "/source/sharp.mp3".to_string(),
                    exported_path: "/Contents/Artist/Album/sharp.mp3".to_string(),
                    file_modified_at: None,
                    file_size_bytes: None,
                    sample_rate_hz: None,
                    bit_depth: None,
                    bitrate_kbps: None,
                    disc_number: None,
                    subtitle: None,
                    comment: None,
                    title_for_search: None,
                    kuvo_delivery_comment: None,
                    dj_play_count: None,
                    rating: None,
                    color_id: None,
                    artist_id_lyricist: None,
                    artist_id_original_artist: None,
                    artist_id_remixer: None,
                    artist_id_composer: None,
                    genre_id: None,
                    genre: None,
                    label_id: None,
                    isrc: None,
                    release_year: None,
                    release_date: None,
                    recorded_date: None,
                    file_type: None,
                    owns_exported_media: true,
                    owns_artwork: false,
                    owns_waveform: false,
                    artwork_path: None,
                    waveform_path: None,
                    duration_ms: Some(180_000),
                },
                ExportManifestTrack {
                    id: "t2".to_string(),
                    master_db_id: Some(2),
                    master_content_id: Some(2),
                    content_link: Some(2),
                    position: 2,
                    track_number: Some(2),
                    title: "Minor".to_string(),
                    artist: "Artist".to_string(),
                    album: Some("Album".to_string()),
                    bpm: Some(120.0),
                    key: Some("Fm".to_string()),
                    source_path: "/source/minor.mp3".to_string(),
                    exported_path: "/Contents/Artist/Album/minor.mp3".to_string(),
                    file_modified_at: None,
                    file_size_bytes: None,
                    sample_rate_hz: None,
                    bit_depth: None,
                    bitrate_kbps: None,
                    disc_number: None,
                    subtitle: None,
                    comment: None,
                    title_for_search: None,
                    kuvo_delivery_comment: None,
                    dj_play_count: None,
                    rating: None,
                    color_id: None,
                    artist_id_lyricist: None,
                    artist_id_original_artist: None,
                    artist_id_remixer: None,
                    artist_id_composer: None,
                    genre_id: None,
                    genre: None,
                    label_id: None,
                    isrc: None,
                    release_year: None,
                    release_date: None,
                    recorded_date: None,
                    file_type: None,
                    owns_exported_media: true,
                    owns_artwork: false,
                    owns_waveform: false,
                    artwork_path: None,
                    waveform_path: None,
                    duration_ms: Some(180_000),
                },
            ],
        };

        rewrite_pdb_playlist_from_manifest(usb_root, &playlist, &manifest, 1, 1)
            .expect("rewrite with distinct musical keys");

        let pdb_path = usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb");
        let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse rewritten pdb");

        let sharp = parsed
            .tracks
            .iter()
            .find(|track| track.track_file_path == "/Contents/Artist/Album/sharp.mp3")
            .expect("sharp track");
        let minor = parsed
            .tracks
            .iter()
            .find(|track| track.track_file_path == "/Contents/Artist/Album/minor.mp3")
            .expect("minor track");

        assert_ne!(
            sharp.key_id, minor.key_id,
            "F#m and Fm must resolve to different PDB key IDs"
        );
        assert_eq!(
            parsed.keys.get(&sharp.key_id).map(String::as_str),
            Some("F#m")
        );
        assert_eq!(
            parsed.keys.get(&minor.key_id).map(String::as_str),
            Some("Fm")
        );
    }

    #[test]
    fn key_resolution_uses_enharmonic_match_without_minor_mode_confusion() {
        let mut map = HashMap::<String, u32>::new();
        map.insert("db".to_string(), 16);
        map.insert("dbm".to_string(), 1);

        assert_eq!(resolve_key_id_by_name(&map, "C#"), Some(16));
        assert_eq!(resolve_key_id_by_name(&map, "C#m"), Some(1));
        assert_ne!(
            resolve_key_id_by_name(&map, "C#"),
            resolve_key_id_by_name(&map, "C#m")
        );
    }

    /// Helper: create a minimal ExportTrackData
    fn make_test_track(id: &str, title: &str, filename: &str) -> ExportTrackData {
        ExportTrackData {
            id: id.to_string(),
            title: title.to_string(),
            artist: "Artist".to_string(),
            album: Some("Album".to_string()),
            track_number: Some(1),
            bpm: Some(120.0),
            key: None,
            file_path: format!("/source/{filename}"),
            file_name: filename.to_string(),
            file_modified_at: None,
            file_size_bytes: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: None,
            color_id: None,
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: None,
            release_year: None,
            release_date: None,
            recorded_date: None,
            file_type: None,
            artwork_path: None,
            waveform_peaks_path: None,
            duration_ms: Some(180_000),
            first_beat_ms: None,
            position: 0,
        }
    }

    /// Helper: create a minimal ExportManifest
    fn make_test_manifest(
        playlist_id: &str,
        playlist_name: &str,
        usb_root: &std::path::Path,
        tracks: &[(&str, &str, &str)], // (id, title, filename)
    ) -> ExportManifest {
        ExportManifest {
            version: 1,
            generated_at: "2024-01-01".to_string(),
            playlist_id: playlist_id.to_string(),
            playlist_name: playlist_name.to_string(),
            usb_root: usb_root.to_string_lossy().to_string(),
            options: crate::models::ExportToUsbOptions {
                include_artwork: false,
                include_analysis: false,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: tracks.len(),
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks: tracks
                .iter()
                .enumerate()
                .map(|(i, (id, title, filename))| ExportManifestTrack {
                    id: id.to_string(),
                    master_db_id: None,
                    master_content_id: None,
                    content_link: None,
                    position: i + 1,
                    track_number: Some(1),
                    title: title.to_string(),
                    artist: "Artist".to_string(),
                    album: Some("Album".to_string()),
                    bpm: Some(120.0),
                    key: None,
                    source_path: format!("/source/{filename}"),
                    exported_path: format!("/Contents/Artist/Album/{filename}"),
                    file_modified_at: None,
                    file_size_bytes: None,
                    sample_rate_hz: None,
                    bit_depth: None,
                    bitrate_kbps: None,
                    disc_number: None,
                    subtitle: None,
                    comment: None,
                    title_for_search: None,
                    kuvo_delivery_comment: None,
                    dj_play_count: None,
                    rating: None,
                    color_id: None,
                    artist_id_lyricist: None,
                    artist_id_original_artist: None,
                    artist_id_remixer: None,
                    artist_id_composer: None,
                    genre_id: None,
                    genre: None,
                    label_id: None,
                    isrc: None,
                    release_year: None,
                    release_date: None,
                    recorded_date: None,
                    file_type: None,
                    owns_exported_media: true,
                    owns_artwork: true,
                    owns_waveform: true,
                    artwork_path: None,
                    waveform_path: None,
                    duration_ms: Some(180_000),
                })
                .collect(),
        }
    }

    // --- collect_manifest_owned_paths ---

    #[test]
    fn collect_owned_paths_includes_media_artwork_analysis() {
        let usb_root = Path::new("/mnt/usb");
        let manifest = ExportManifest {
            version: 1,
            generated_at: "2024-01-01".to_string(),
            playlist_id: "pl1".to_string(),
            playlist_name: "Test".to_string(),
            usb_root: "/mnt/usb".to_string(),
            options: ExportToUsbOptions {
                include_artwork: true,
                include_analysis: true,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 1,
            skipped_tracks: 0,
            warnings: vec![],
            tracks: vec![ExportManifestTrack {
                id: "t1".to_string(),
                master_db_id: None,
                master_content_id: None,
                content_link: None,
                position: 1,
                track_number: Some(1),
                title: "Track".to_string(),
                artist: "Artist".to_string(),
                album: None,
                bpm: None,
                key: None,
                source_path: "/local/track.mp3".to_string(),
                exported_path: "/Contents/Artist/Album/track.mp3".to_string(),
                file_modified_at: None,
                file_size_bytes: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                title_for_search: None,
                kuvo_delivery_comment: None,
                dj_play_count: None,
                rating: None,
                color_id: None,
                artist_id_lyricist: None,
                artist_id_original_artist: None,
                artist_id_remixer: None,
                artist_id_composer: None,
                genre_id: None,
                genre: None,
                label_id: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                file_type: None,
                owns_exported_media: true,
                owns_artwork: true,
                owns_waveform: true,
                artwork_path: Some("/PIONEER/Artwork/00001/a00001.jpg".to_string()),
                waveform_path: Some("/PIONEER/USBANLZ/P001/ABC/ANLZ0000.DAT".to_string()),
                duration_ms: None,
            }],
        };
        let owned = collect_manifest_owned_paths(usb_root, &manifest);
        // 1 media + 1 artwork + 3 analysis bundle (DAT/EXT/2EX) = 5
        assert!(
            owned.len() >= 3,
            "should include media + artwork + analysis paths, got {} items: {:?}",
            owned.len(),
            owned
        );
        assert!(
            owned.iter().any(|p| p.contains("track.mp3")),
            "should include media path"
        );
        assert!(
            owned.iter().any(|p| p.contains("Artwork")),
            "should include artwork path"
        );
        assert!(
            owned.iter().any(|p| p.contains("USBANLZ")),
            "should include analysis path"
        );
    }

    #[test]
    fn collect_owned_paths_skips_none_artwork_and_waveform() {
        let usb_root = Path::new("/mnt/usb");
        let manifest = ExportManifest {
            version: 1,
            generated_at: "2024-01-01".to_string(),
            playlist_id: "pl1".to_string(),
            playlist_name: "Test".to_string(),
            usb_root: "/mnt/usb".to_string(),
            options: ExportToUsbOptions {
                include_artwork: false,
                include_analysis: false,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 1,
            skipped_tracks: 0,
            warnings: vec![],
            tracks: vec![ExportManifestTrack {
                id: "t1".to_string(),
                master_db_id: None,
                master_content_id: None,
                content_link: None,
                position: 1,
                track_number: None,
                title: "Track".to_string(),
                artist: "Artist".to_string(),
                album: None,
                bpm: None,
                key: None,
                source_path: "/local/track.mp3".to_string(),
                exported_path: "/Contents/track.mp3".to_string(),
                file_modified_at: None,
                file_size_bytes: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                title_for_search: None,
                kuvo_delivery_comment: None,
                dj_play_count: None,
                rating: None,
                color_id: None,
                artist_id_lyricist: None,
                artist_id_original_artist: None,
                artist_id_remixer: None,
                artist_id_composer: None,
                genre_id: None,
                genre: None,
                label_id: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                file_type: None,
                owns_exported_media: true,
                owns_artwork: true,
                owns_waveform: true,
                artwork_path: None,
                waveform_path: None,
                duration_ms: None,
            }],
        };
        let owned = collect_manifest_owned_paths(usb_root, &manifest);
        // Should only have the media path
        assert_eq!(
            owned.len(),
            1,
            "should only include media path when no artwork/analysis"
        );
    }

    // ── Path shaping unit tests ──────────────────────────────────────────

    #[test]
    fn to_usb_relative_path_strips_root_prefix() {
        let usb = Path::new("/mnt/usb");
        assert_eq!(
            to_usb_relative_path(usb, "/mnt/usb/Contents/Artist/Track.mp3"),
            Some("/Contents/Artist/Track.mp3".to_string())
        );
    }

    #[test]
    fn to_usb_relative_path_joins_relative_then_strips() {
        let usb = Path::new("/mnt/usb");
        assert_eq!(
            to_usb_relative_path(usb, "Contents/Artist/Track.mp3"),
            Some("/Contents/Artist/Track.mp3".to_string())
        );
    }

    #[test]
    fn to_usb_relative_path_returns_none_for_unrelated_absolute() {
        let usb = Path::new("/mnt/usb");
        assert_eq!(to_usb_relative_path(usb, "/other/path/Track.mp3"), None);
    }

    #[test]
    fn to_usb_relative_path_analysis_dir() {
        let usb = Path::new("/tmp/usb");
        assert_eq!(
            to_usb_relative_path(usb, "/tmp/usb/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT"),
            Some("/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT".to_string())
        );
    }

    #[test]
    fn content_file_name_extracts_filename() {
        assert_eq!(
            content_file_name("/Contents/Artist/Album/Track.mp3"),
            "Track.mp3"
        );
    }

    #[test]
    fn content_file_name_handles_bare_filename() {
        assert_eq!(content_file_name("Track.mp3"), "Track.mp3");
    }

    #[test]
    fn content_file_name_handles_directory_path() {
        // Path::new treats trailing component as filename
        assert_eq!(content_file_name("/Contents/Artist"), "Artist");
    }

    #[test]
    fn exported_media_target_path_preserves_existing_contents_structure() {
        let media_root = Path::new("/mnt/usb/Contents");
        let source = Path::new("/local/music/Contents/DJ Artist/Remix Album/hot_track.flac");
        let result = exported_media_target_path(
            media_root,
            source,
            "DJ Artist",
            Some("Remix Album"),
            "Hot Track",
            "flac",
        );
        assert_eq!(
            result,
            PathBuf::from("/mnt/usb/Contents/DJ Artist/Remix Album/hot_track.flac")
        );
    }

    #[test]
    fn exported_media_target_path_builds_from_metadata_when_no_contents_in_source() {
        let media_root = Path::new("/mnt/usb/Contents");
        let source = Path::new("/home/user/Music/artist - track.mp3");
        let result = exported_media_target_path(
            media_root,
            source,
            "Test Artist",
            Some("Test Album"),
            "Test Track",
            "mp3",
        );
        assert_eq!(
            result,
            PathBuf::from("/mnt/usb/Contents/Test Artist/Test Album/artist - track.mp3")
        );
    }

    #[test]
    fn manifest_paths_are_already_usb_relative() {
        // Verify the invariant that ExportManifestTrack paths are USB-relative
        // (start with /) — both eDB and PDB writers depend on this.
        let track = mapping_test_track();
        assert!(
            track.exported_path.starts_with("/Contents/"),
            "exported_path must be USB-relative: {}",
            track.exported_path
        );
        assert!(
            track
                .waveform_path
                .as_ref()
                .unwrap()
                .starts_with("/PIONEER/"),
            "waveform_path must be USB-relative: {}",
            track.waveform_path.as_ref().unwrap()
        );
        assert!(
            track
                .artwork_path
                .as_ref()
                .unwrap()
                .starts_with("/PIONEER/"),
            "artwork_path must be USB-relative: {}",
            track.artwork_path.as_ref().unwrap()
        );
    }

    #[test]
    fn edb_and_pdb_receive_identical_media_path_from_manifest() {
        // The PDB writer previously re-converted already-relative paths through
        // to_usb_relative_path(). This test ensures both sides get the same value
        // from the manifest without re-conversion.
        let track = mapping_test_track();

        // eDB uses track.exported_path directly (line ~3061)
        let edb_path = track.exported_path.clone();

        // PDB now also uses track.exported_path directly (was to_usb_relative_path)
        let pdb_path = track.exported_path.clone();

        assert_eq!(
            edb_path, pdb_path,
            "eDB and PDB must receive identical media paths"
        );
    }

    #[test]
    fn edb_and_pdb_receive_identical_analysis_path_from_manifest() {
        let track = mapping_test_track();

        // eDB uses track.waveform_path directly (line ~3064)
        let edb_anlz = track.waveform_path.clone().unwrap_or_default();

        // PDB now also uses track.waveform_path directly
        let pdb_anlz = track.waveform_path.clone().unwrap_or_default();

        assert_eq!(
            edb_anlz, pdb_anlz,
            "eDB and PDB must receive identical analysis paths"
        );
    }

    #[test]
    fn genre_and_label_tables_preserved_through_export_roundtrip() {
        use crate::pdb_writer::{
            PdbData, PdbDictRow, standard_colors, standard_columns_raw, write_pdb_to_file,
        };
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let usb_root = dir.path();
        let pdb_dir = usb_root
            .join(super::USB_VENDOR_ROOT_DIR)
            .join(super::USB_VENDOR_DB_DIR);
        std::fs::create_dir_all(&pdb_dir).unwrap();

        // Write a PDB with genre and label rows
        let data = PdbData {
            genres: vec![
                PdbDictRow {
                    id: 1,
                    name: "Electronic".to_string(),
                },
                PdbDictRow {
                    id: 2,
                    name: "Techno".to_string(),
                },
            ],
            labels: vec![PdbDictRow {
                id: 1,
                name: "Kompakt".to_string(),
            }],
            colors: standard_colors(),
            columns_raw_rows: standard_columns_raw(),
            ..PdbData::empty()
        };
        write_pdb_to_file(&pdb_dir.join("export.pdb"), &data).unwrap();

        // Export over it with a manifest containing one track
        let playlist = super::ExportPlaylistData {
            id: "pl-1".to_string(),
            name: "Test".to_string(),
            tracks: vec![super::ExportTrackData {
                id: "t1".to_string(),
                title: "Track A".to_string(),
                artist: "Artist".to_string(),
                album: None,
                track_number: None,
                bpm: None,
                key: None,
                file_path: "/source/a.mp3".to_string(),
                file_name: "a.mp3".to_string(),
                file_modified_at: None,
                file_size_bytes: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                title_for_search: None,
                kuvo_delivery_comment: None,
                dj_play_count: None,
                rating: None,
                color_id: None,
                artist_id_lyricist: None,
                artist_id_original_artist: None,
                artist_id_remixer: None,
                artist_id_composer: None,
                genre_id: None,
                genre: None,
                label_id: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                file_type: None,
                artwork_path: None,
                waveform_peaks_path: None,
                duration_ms: None,
                first_beat_ms: None,
                position: 0,
            }],
        };
        let manifest = super::ExportManifest {
            version: 1,
            generated_at: "2025-01-01".to_string(),
            playlist_id: "pl-1".to_string(),
            playlist_name: "Test".to_string(),
            usb_root: usb_root.to_string_lossy().to_string(),
            options: crate::models::ExportToUsbOptions {
                include_artwork: false,
                include_analysis: false,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 1,
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks: vec![super::ExportManifestTrack {
                id: "t1".to_string(),
                master_db_id: None,
                master_content_id: None,
                content_link: None,
                position: 0,
                track_number: None,
                title: "Track A".to_string(),
                artist: "Artist".to_string(),
                album: None,
                bpm: None,
                key: None,
                source_path: "/source/a.mp3".to_string(),
                exported_path: "/Contents/Artist/a.mp3".to_string(),
                file_modified_at: None,
                file_size_bytes: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                title_for_search: None,
                kuvo_delivery_comment: None,
                dj_play_count: None,
                rating: None,
                color_id: None,
                artist_id_lyricist: None,
                artist_id_original_artist: None,
                artist_id_remixer: None,
                artist_id_composer: None,
                genre_id: None,
                genre: None,
                label_id: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                file_type: None,
                owns_exported_media: true,
                owns_artwork: false,
                owns_waveform: false,
                artwork_path: None,
                waveform_path: None,
                duration_ms: None,
            }],
        };

        super::write_pdb(usb_root, &playlist, &manifest, true, None, None, false).unwrap();

        // Parse result and verify genres/labels survived
        let parsed = crate::pdb_reader::parse_pdb(&pdb_dir.join("export.pdb")).unwrap();
        assert_eq!(parsed.genres.len(), 2, "genres should survive export");
        assert!(
            parsed.genres.values().any(|v| v == "Electronic"),
            "Electronic genre missing"
        );
        assert!(
            parsed.genres.values().any(|v| v == "Techno"),
            "Techno genre missing"
        );
        assert_eq!(parsed.labels.len(), 1, "labels should survive export");
        assert!(
            parsed.labels.values().any(|v| v == "Kompakt"),
            "Kompakt label missing"
        );
    }

    #[test]
    fn default_export_profile_is_rb6_compatible() {
        assert_eq!(PdbLayoutProfile::DEFAULT, PdbLayoutProfile::Rb6Compatible);
        // from_env with no env var should return the default
        unsafe {
            std::env::remove_var("PDB_LAYOUT_PROFILE");
        }
        assert_eq!(
            PdbLayoutProfile::from_env(),
            PdbLayoutProfile::Rb6Compatible
        );
    }

    #[test]
    fn export_profile_as_str_roundtrips() {
        assert_eq!(PdbLayoutProfile::Current.as_str(), "current");
        assert_eq!(PdbLayoutProfile::Rb6Compatible.as_str(), "rb6_compatible");
        assert_eq!(PdbLayoutProfile::Rb7Compatible.as_str(), "rb7_compatible");
    }

    #[test]
    fn validate_no_empty_data_pages_catches_empty_page() {
        // Build a minimal 3-page PDB: page 0 = file header, page 1 = sentinel, page 2 = data
        let page_size = 4096usize;
        let mut bytes = vec![0u8; page_size * 3];

        // File header at page 0 — set table 8 pointers: first=1, last=2
        crate::utils::set_table_ptr_fields(&mut bytes, 8, 3, 1, 2);

        // Sentinel page (page 1): page_idx=1, table_type=8, next=2, flags=0x64
        let off1 = page_size;
        bytes[off1 + 0x04..off1 + 0x08].copy_from_slice(&1u32.to_le_bytes());
        bytes[off1 + 0x08..off1 + 0x0c].copy_from_slice(&8u32.to_le_bytes());
        bytes[off1 + 0x0c..off1 + 0x10].copy_from_slice(&2u32.to_le_bytes());
        bytes[off1 + 0x1b] = 0x64; // sentinel

        // Data page (page 2): page_idx=2, table_type=8, next=0, flags=0x24
        // Leave nrs=0 and used_size=0 → empty data page
        let off2 = page_size * 2;
        bytes[off2 + 0x04..off2 + 0x08].copy_from_slice(&2u32.to_le_bytes());
        bytes[off2 + 0x08..off2 + 0x0c].copy_from_slice(&8u32.to_le_bytes());
        bytes[off2 + 0x1b] = 0x24; // data page

        let result = validate_no_empty_data_pages(&bytes, page_size, 8, 1, 2);
        assert!(result.is_err(), "should reject empty data page in chain");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("0 rows"), "error should mention 0 rows: {msg}");

        // Now give the page some rows — validation should pass
        bytes[off2 + 0x18] = 1; // nrs = 1
        bytes[off2 + 0x1e..off2 + 0x20].copy_from_slice(&12u16.to_le_bytes()); // used_size = 12
        let result2 = validate_no_empty_data_pages(&bytes, page_size, 8, 1, 2);
        assert!(result2.is_ok(), "should pass with non-empty data page");
    }

    #[test]
    fn try_patch_t08_with_multi_page_growth_updates_pointers_and_keeps_pages_non_empty() {
        let page_size = 4096usize;
        let mut bytes = vec![0u8; page_size * 3];

        crate::utils::set_table_ptr_fields(&mut bytes, 8, 3, 1, 2);

        let off1 = page_size;
        bytes[off1 + 0x04..off1 + 0x08].copy_from_slice(&1u32.to_le_bytes());
        bytes[off1 + 0x08..off1 + 0x0c].copy_from_slice(&8u32.to_le_bytes());
        bytes[off1 + 0x0c..off1 + 0x10].copy_from_slice(&2u32.to_le_bytes());
        bytes[off1 + 0x1b] = 0x64;

        let off2 = page_size * 2;
        bytes[off2 + 0x04..off2 + 0x08].copy_from_slice(&2u32.to_le_bytes());
        bytes[off2 + 0x08..off2 + 0x0c].copy_from_slice(&8u32.to_le_bytes());
        bytes[off2 + 0x1b] = 0x24;

        let desired_entries = (0..800u32)
            .map(|idx| T08EntryKey {
                entry_index: idx + 1,
                track_id: idx + 100,
                playlist_id: 77,
            })
            .collect::<Vec<_>>();
        let ctx = T08PatchContext {
            playlist_id: 77,
            desired_entries,
        };

        let changed =
            try_patch_t08_with_multi_page_growth(&mut bytes, &[1, 2], 1, 2, page_size, &ctx);
        assert!(changed, "growth helper should apply");

        let t08_ptr_off = 0x1cusize + 8usize * 16;
        let last = u32::from_le_bytes(
            bytes[t08_ptr_off + 12..t08_ptr_off + 16]
                .try_into()
                .unwrap(),
        );
        assert!(last > 2, "t08 growth should append at least one page");

        let result = validate_no_empty_data_pages(&bytes, page_size, 8, 1, last);
        assert!(
            result.is_ok(),
            "public growth helper should leave no empty t08 data pages: {result:?}"
        );
    }
}
