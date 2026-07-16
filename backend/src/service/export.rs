//! Export to USB implementation.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, params};
use serde_json::json;

use crate::error::{BackendError, BackendResult};
use crate::models::{ExportToUsbData, ExportToUsbRequest, WarningEntry};

use super::export_helpers::{
    ExportManifest, ExportManifestTrack, ExportPlaylistData, ExportTrackData,
    WriteExportLibraryDbResult, WriteExportPdbResult, collect_manifest_owned_paths,
    copy_if_different, copy_wav_normalized_if_needed, ensure_analysis_bundle_ppth,
    export_analysis_bundle_for_track, export_artwork_for_player, export_owned_files_setting_key,
    exported_media_target_path, filter_prunable_stale_paths_for_playlist, preview_pdb,
    prune_stale_export_owned_files, stable_u32_hash, to_usb_relative_path, verify_edb_content,
    verify_pdb_content, write_edb_playlist, write_pdb,
};
use super::export_log::append_export_log_record;
use super::usb_utils::{
    analysis_bundle_exists, canonicalize_playlist_name,
    load_existing_analysis_paths_by_content_path, load_existing_analysis_paths_by_pdb_track_path,
    resolve_usb_root, resolve_usb_side_path,
};
use super::usb_vendor_compat::{backup_usb_databases, vendor_pdb_path};
use super::{BackendService, SETTING_EXPORT_MASTER_DB_ID, now};
use crate::pdb_reader::parse_pdb;

fn existing_usb_relative_if_file(usb_root: &Path, path: Option<&str>) -> Option<String> {
    let candidate = path?.trim();
    if candidate.is_empty() {
        return None;
    }
    let rel = to_usb_relative_path(usb_root, candidate)?;
    let abs = resolve_usb_side_path(usb_root, &rel)?;
    if Path::new(&abs).is_file() {
        Some(rel)
    } else {
        None
    }
}

fn has_required_analysis_fields(track: &ExportTrackData) -> bool {
    let has_waveform_path = track
        .waveform_peaks_path
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let has_bpm = track.bpm.map(|v| v > 0.0).unwrap_or(false);
    let has_duration = track.duration_ms.map(|v| v > 0).unwrap_or(false);
    has_waveform_path && has_bpm && has_duration
}

fn has_analysis_bundle(usb_root: &Path, track: &ExportTrackData) -> bool {
    track
        .waveform_peaks_path
        .as_deref()
        .map(|path| analysis_bundle_exists(usb_root, path))
        .unwrap_or(false)
}

fn has_required_analysis(usb_root: &Path, track: &ExportTrackData) -> bool {
    has_required_analysis_fields(track) && has_analysis_bundle(usb_root, track)
}

fn export_warning_entry(message: String) -> WarningEntry {
    let lower = message.to_lowercase();
    let (level, code) = if lower.starts_with("export verification passed")
        || lower.starts_with("prune stale enabled:")
    {
        ("info", "export.info")
    } else if message.starts_with("slow-media suspected:")
        || lower.contains("missing")
        || lower.contains("skipped")
    {
        ("warn", "export.warn")
    } else if lower.contains("failed") || lower.contains("error") {
        ("error", "export.error")
    } else {
        ("info", "export.info")
    };
    WarningEntry {
        level: level.to_string(),
        code: code.to_string(),
        message,
        source: "export".to_string(),
    }
}

fn ensure_playlist_tracks_analysis_ready(
    usb_root: &Path,
    playlist: &ExportPlaylistData,
) -> BackendResult<()> {
    let total = playlist.tracks.len();
    let mut missing = 0usize;
    let mut missing_analysis_bundle = 0usize;
    for track in &playlist.tracks {
        if has_required_analysis(usb_root, track) {
            continue;
        }
        missing += 1;
        if has_required_analysis_fields(track) && !has_analysis_bundle(usb_root, track) {
            missing_analysis_bundle += 1;
        }
    }
    if missing == 0 {
        return Ok(());
    }
    Err(BackendError::ValidationWithDetails(
        format!(
            "export blocked: {missing}/{total} playlist tracks are missing required analysis (waveform, bpm, duration, DAT/EXT/2EX files); run analysis first"
        ),
        json!({
            "validationType": "missing_analysis",
            "requiredFields": ["waveform", "bpm", "duration"],
            "requiredFiles": ["DAT", "EXT", "2EX"],
            "missingTrackCount": missing,
            "missingAnalysisBundleCount": missing_analysis_bundle,
            "totalTrackCount": total,
        }),
    ))
}

impl BackendService {
    pub fn export_to_usb(&self, req: ExportToUsbRequest) -> BackendResult<ExportToUsbData> {
        self.export_to_usb_with_progress(req, |_, _, _| {})
    }

    pub fn export_to_usb_with_progress<F>(
        &self,
        req: ExportToUsbRequest,
        mut on_progress: F,
    ) -> BackendResult<ExportToUsbData>
    where
        F: FnMut(usize, usize, &str),
    {
        let playlist_id = req.playlist_id.trim().to_string();
        if playlist_id.is_empty() {
            return Err(BackendError::Validation(
                "playlistId must not be empty".to_string(),
            ));
        }

        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        if !usb_root.exists() || !usb_root.is_dir() {
            return Err(BackendError::NotFound(format!(
                "USB root does not exist: {}",
                usb_root.display()
            )));
        }
        let options = req.options.unwrap_or_default();
        let export_dry_run = std::env::var("USB_EXPORT_DRY_RUN")
            .ok()
            .map(|v| {
                let n = v.trim().to_ascii_lowercase();
                n == "1" || n == "true" || n == "yes" || n == "on"
            })
            .unwrap_or(false);
        let playlist = self.load_playlist_for_export(&playlist_id)?;
        if playlist.tracks.is_empty() {
            return Err(BackendError::Validation(
                "playlist has no tracks to export".to_string(),
            ));
        }

        let media_root = usb_root.join("Contents");
        std::fs::create_dir_all(&media_root)?;

        let total_steps = playlist.tracks.len() + 1;
        on_progress(0, total_steps, "USB: Preparing export");

        let mut warnings = Vec::<String>::new();
        let playlist = playlist;
        ensure_playlist_tracks_analysis_ready(&usb_root, &playlist)?;
        let local_conn = self.db.connect()?;
        Self::ensure_track_export_identity_schema(&local_conn)?;
        let app_master_db_id = Self::ensure_local_u32_setting(
            &local_conn,
            SETTING_EXPORT_MASTER_DB_ID,
            "master-db-id",
        )?;
        // contentLink low 16 bits encode a format/version identifier that the DJ software
        // checks when deciding whether analysis is "new generation".
        // Reference fixtures use 0x000C0700 as baseline for newly exported
        // content, then bump by +0x10000 on re-export (handled elsewhere).
        let app_content_link_id: i64 = 0x000C_0700;

        let mut existing_analysis_by_path =
            load_existing_analysis_paths_by_pdb_track_path(&usb_root);
        for (path_key, analysis_path) in
            load_existing_analysis_paths_by_content_path(&usb_root, &mut warnings)
        {
            existing_analysis_by_path
                .entry(path_key)
                .or_insert(analysis_path);
        }
        // Build identity lookup from existing USB PDB: identity fields + artwork path per track
        let existing_usb_identity_by_path = {
            let pdb_path = vendor_pdb_path(&usb_root);
            let mut map =
                HashMap::<String, (Option<u32>, Option<u32>, Option<u32>, Option<String>)>::new();
            if let Ok(parsed) = parse_pdb(&pdb_path) {
                for track in &parsed.tracks {
                    let path_key = canonicalize_playlist_name(&track.track_file_path);
                    let art = parsed.artworks.get(&track.artwork_id).cloned();
                    map.insert(
                        path_key,
                        (
                            track.master_db_id,
                            track.master_content_id,
                            track.content_link,
                            art,
                        ),
                    );
                }
            }
            map
        };

        let mut exported_tracks = 0usize;
        let mut skipped_tracks = 0usize;
        let mut exported_artworks = 0usize;
        let mut exported_analysis_files = 0usize;
        let mut manifest_tracks = Vec::<ExportManifestTrack>::new();

        for (idx, track) in playlist.tracks.iter().enumerate() {
            on_progress(
                idx + 1,
                total_steps,
                &format!(
                    "USB: Copying track {}/{}: {}",
                    idx + 1,
                    playlist.tracks.len(),
                    track.file_name
                ),
            );

            let source = PathBuf::from(&track.file_path);
            if !source.is_file() {
                skipped_tracks += 1;
                warnings.push(format!("missing source file: {}", source.display()));
                continue;
            }

            let extension = source
                .extension()
                .and_then(|s| s.to_str())
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_else(|| "bin".to_string());
            let target_base = exported_media_target_path(
                &media_root,
                &source,
                &track.artist,
                track.album.as_deref(),
                &track.title,
                &extension,
            );
            let target = target_base;
            let existing_exported_path =
                existing_usb_relative_if_file(&usb_root, Some(&source.to_string_lossy()))
                    .filter(|rel| rel.starts_with("/Contents/"));
            let owns_exported_media = existing_exported_path.is_none();
            if !export_dry_run && owns_exported_media {
                if extension == "wav" || extension == "wave" {
                    copy_wav_normalized_if_needed(&source, &target)?;
                } else {
                    copy_if_different(&source, &target)?;
                }
            }
            exported_tracks += 1;

            let exported_path = existing_exported_path.unwrap_or_else(|| {
                to_usb_relative_path(&usb_root, &target.to_string_lossy())
                    .unwrap_or_else(|| target.to_string_lossy().to_string())
            });
            let mut artwork_relative = None;
            let mut owns_artwork = false;
            if options.include_artwork {
                if let Some(existing_artwork) =
                    existing_usb_relative_if_file(&usb_root, track.artwork_path.as_deref())
                {
                    artwork_relative = Some(existing_artwork);
                } else if !export_dry_run
                    && let Some(path) = track.artwork_path.as_deref()
                    && let Some(asset_path) =
                        export_artwork_for_player(path, &usb_root, &track.id, &mut warnings)?
                {
                    artwork_relative =
                        to_usb_relative_path(&usb_root, &asset_path).or(Some(asset_path));
                    exported_artworks += 1;
                    owns_artwork = true;
                }
            }
            // Fallback: pick up artwork from existing USB PDB if we didn't resolve it
            if artwork_relative.is_none() {
                let exported_key = canonicalize_playlist_name(&exported_path);
                if let Some((_, _, _, Some(art_path))) =
                    existing_usb_identity_by_path.get(&exported_key)
                {
                    artwork_relative = Some(art_path.clone());
                }
            }

            let mut analysis_relative = None;
            let mut owns_waveform = false;
            if options.include_analysis {
                // Reuse an existing USB-side analysis bundle when the source track
                // already lives on this stick. Otherwise generate a fresh bundle
                // from local analysis inputs so waveform/beatgrid fixes apply.
                if let Some(existing_analysis) =
                    existing_usb_relative_if_file(&usb_root, track.waveform_peaks_path.as_deref())
                        .filter(|p| analysis_bundle_exists(usb_root.as_path(), p))
                {
                    if !export_dry_run {
                        ensure_analysis_bundle_ppth(&usb_root, &existing_analysis, &exported_path)?;
                    }
                    analysis_relative = Some(existing_analysis);
                } else {
                    let exported_key = canonicalize_playlist_name(&exported_path);
                    if let Some(existing_analysis) = existing_analysis_by_path
                        .get(&exported_key)
                        .cloned()
                        .filter(|p| analysis_bundle_exists(usb_root.as_path(), p))
                    {
                        if !export_dry_run {
                            ensure_analysis_bundle_ppth(
                                &usb_root,
                                &existing_analysis,
                                &exported_path,
                            )?;
                        }
                        analysis_relative = Some(existing_analysis);
                    } else if !export_dry_run
                        && track.waveform_peaks_path.is_some()
                        && let Some(relative) = export_analysis_bundle_for_track(
                            track,
                            &usb_root,
                            &exported_path,
                            &mut warnings,
                        )?
                    {
                        analysis_relative = Some(relative);
                        exported_analysis_files += 3;
                        owns_waveform = true;
                    }
                }
            }

            let exported_key = canonicalize_playlist_name(&exported_path);
            let existing_identity = existing_usb_identity_by_path.get(&exported_key);
            // Preserve prior USB identity for existing paths to keep eDB/PDB
            // track identity stable across additive exports.
            let (mdb, mci, cl) = Self::resolve_manifest_identity(
                &local_conn,
                &track.id,
                existing_identity,
                owns_waveform,
                app_master_db_id,
                app_content_link_id,
            )?;
            manifest_tracks.push(ExportManifestTrack {
                id: track.id.clone(),
                master_db_id: mdb,
                master_content_id: mci,
                content_link: cl,
                position: track.position + 1,
                track_number: track.track_number,
                title: track.title.clone(),
                artist: track.artist.clone(),
                album: track.album.clone(),
                bpm: track.bpm,
                key: track.key.clone(),
                source_path: source.to_string_lossy().to_string(),
                exported_path,
                file_modified_at: track.file_modified_at.clone(),
                file_size_bytes: track.file_size_bytes,
                sample_rate_hz: track.sample_rate_hz,
                bit_depth: track.bit_depth,
                bitrate_kbps: track.bitrate_kbps,
                disc_number: track.disc_number,
                subtitle: track.subtitle.clone(),
                comment: track.comment.clone(),
                title_for_search: track.title_for_search.clone(),
                kuvo_delivery_comment: track.kuvo_delivery_comment.clone(),
                dj_play_count: track.dj_play_count,
                rating: track.rating,
                color_id: track.color_id,
                artist_id_lyricist: track.artist_id_lyricist,
                artist_id_original_artist: track.artist_id_original_artist,
                artist_id_remixer: track.artist_id_remixer,
                artist_id_composer: track.artist_id_composer,
                genre_id: track.genre_id,
                genre: track.genre.clone(),
                label_id: track.label_id,
                isrc: track.isrc.clone(),
                release_year: track.release_year,
                release_date: track.release_date.clone(),
                recorded_date: track.recorded_date.clone(),
                file_type: track.file_type,
                owns_exported_media,
                owns_artwork,
                owns_waveform,
                artwork_path: artwork_relative,
                waveform_path: analysis_relative,
                duration_ms: track.duration_ms,
            });
        }

        on_progress(total_steps, total_steps, "USB: Finalizing export metadata");
        let manifest = ExportManifest {
            version: 1,
            generated_at: now(),
            playlist_id: playlist.id.clone(),
            playlist_name: playlist.name.clone(),
            usb_root: usb_root.to_string_lossy().to_string(),
            options: options.clone(),
            exported_tracks,
            skipped_tracks,
            warnings: warnings.clone(),
            tracks: manifest_tracks,
        };
        let mirror_playlist_entries = options.prune_stale;
        let skip_pdb_write = std::env::var("PDB_WRITE_MODE")
            .ok()
            .map(|v| v.eq_ignore_ascii_case("skip"))
            .unwrap_or(false);

        let mut edb_playlist_id: Option<u32> = None;
        let mut edb_sort_order: Option<u32> = None;
        if export_dry_run {
            warnings.push(
                "dry-run enabled: media/artwork/analysis/database writes skipped".to_string(),
            );
            match preview_pdb(
                &usb_root,
                &playlist,
                &manifest,
                mirror_playlist_entries,
                None,
                None,
            ) {
                Ok(WriteExportPdbResult {
                    inserted_tracks,
                    inserted_playlists,
                    topology_issues,
                    writer_warnings,
                }) => {
                    warnings.extend(writer_warnings);
                    warnings.push(format!(
                        "PDB preview (tracks: {inserted_tracks}, playlists: {inserted_playlists})"
                    ));
                    if topology_issues.is_empty() {
                        warnings.push(
                            "PDB preview topology: no critical table-chain delta detected"
                                .to_string(),
                        );
                    } else {
                        warnings.push(format!(
                            "PDB preview topology risk: {} issue(s)",
                            topology_issues.len()
                        ));
                        warnings.extend(
                            topology_issues
                                .into_iter()
                                .map(|issue| format!("PDB preview topology: {issue}")),
                        );
                    }
                }
                Err(err) => {
                    warnings.push(format!("PDB preview failed: {err}"));
                }
            }
        } else {
            if options.backup_before_export {
                warnings.extend(backup_usb_databases(&usb_root));
            }
            // Write eDB first (master), then sync PDB to match eDB playlist IDs
            match write_edb_playlist(&usb_root, &playlist, &manifest, mirror_playlist_entries) {
                Ok(WriteExportLibraryDbResult {
                    inserted_content,
                    linked_playlist_entries,
                    playlist_id,
                    sort_order,
                }) => {
                    edb_playlist_id = u32::try_from(playlist_id).ok();
                    edb_sort_order = u32::try_from(sort_order).ok();
                    // Always keep exportExt.pdb byte-stable in normal export.
                    // Strict integrity checks can fail when exportExt header/count
                    // bytes are rewritten from eDB-derived values.
                    let verify_warnings =
                        self.verify_export_outputs(&usb_root, &playlist, &manifest, true, false)?;
                    warnings.extend(verify_warnings);
                    warnings.push(format!(
                        "eDB updated (content rows: {inserted_content}, playlist entries: {linked_playlist_entries})"
                    ));
                }
                Err(err) => {
                    warnings.push(format!("eDB update skipped: {err}"));
                }
            }

            match write_pdb(
                &usb_root,
                &playlist,
                &manifest,
                mirror_playlist_entries,
                edb_playlist_id,
                edb_sort_order,
                false,
            ) {
                Ok(WriteExportPdbResult {
                    inserted_tracks,
                    inserted_playlists,
                    topology_issues: _,
                    writer_warnings,
                }) => {
                    let verify_pdb_warnings = self.verify_export_outputs(
                        &usb_root,
                        &playlist,
                        &manifest,
                        false,
                        !skip_pdb_write,
                    )?;
                    warnings.extend(verify_pdb_warnings);
                    warnings.extend(writer_warnings);
                    warnings.push(format!(
                        "PDB written (tracks: {inserted_tracks}, playlists: {inserted_playlists})"
                    ));
                }
                Err(err) => {
                    return Err(err);
                }
            }
        }

        if !export_dry_run {
            let owned_setting_key = export_owned_files_setting_key(&usb_root, &playlist.id);
            let current_owned = collect_manifest_owned_paths(&usb_root, &manifest);
            let previous_owned = self.load_export_owned_files(&owned_setting_key)?;
            if options.prune_stale {
                let stale = previous_owned
                    .difference(&current_owned)
                    .cloned()
                    .collect::<Vec<_>>();
                let prunable = filter_prunable_stale_paths_for_playlist(
                    &usb_root,
                    &playlist.name,
                    &stale,
                    &mut warnings,
                )?;
                let prune_result =
                    prune_stale_export_owned_files(&usb_root, &prunable, &mut warnings)?;
                warnings.push(format!(
                    "prune stale enabled: removed {}, missing {}, skipped {}",
                    prune_result.removed, prune_result.missing, prune_result.skipped
                ));
            }
            self.save_export_owned_files(&owned_setting_key, &current_owned)?;
            {
                let profile = crate::service::export_helpers::PdbLayoutProfile::from_env();
                let profile_key = format!("pdb_profile:{}", usb_root.to_string_lossy());
                self.db.connect()?.execute(
                    "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                    params![profile_key, profile.as_str(), now()],
                )?;
            }
            self.db.connect()?.execute(
                "UPDATE playlists SET last_exported_at = ?1, last_exported_usb_root = ?2, last_exported_track_count = ?3 WHERE id = ?4",
                params![
                    now(),
                    usb_root.to_string_lossy().to_string(),
                    manifest.tracks.len() as i64,
                    playlist.id
                ],
            )?;
            if let Err(err) = append_export_log_record(&usb_root, &playlist, &manifest) {
                warnings.push(format!("USB export log append skipped: {err}"));
            }
        }

        Ok(ExportToUsbData {
            job_id: String::new(),
            playlist_id: playlist.id,
            playlist_name: playlist.name,
            usb_root: usb_root.to_string_lossy().to_string(),
            exported_tracks,
            skipped_tracks,
            exported_artworks,
            exported_analysis_files,
            manifest_path: String::new(),
            warnings: warnings.into_iter().map(export_warning_entry).collect(),
        })
    }

    fn verify_export_outputs(
        &self,
        usb_root: &Path,
        playlist: &ExportPlaylistData,
        manifest: &ExportManifest,
        verify_db: bool,
        verify_pdb: bool,
    ) -> BackendResult<Vec<String>> {
        let mut warnings = Vec::<String>::new();

        for track in &manifest.tracks {
            let media_path = resolve_usb_side_path(usb_root, &track.exported_path)
                .unwrap_or_else(|| track.exported_path.clone());
            if !Path::new(&media_path).is_file() {
                return Err(BackendError::Internal(format!(
                    "export verification failed: media file missing for track '{}': {}",
                    track.id, media_path
                )));
            }

            if let Some(art) = track.artwork_path.as_deref() {
                let art_abs =
                    resolve_usb_side_path(usb_root, art).unwrap_or_else(|| art.to_string());
                if !Path::new(&art_abs).is_file() {
                    warnings.push(format!(
                        "export verification warning: artwork missing for track '{}': {}",
                        track.id, art_abs
                    ));
                }
            }

            if let Some(anlz) = track.waveform_path.as_deref() {
                let anlz_abs =
                    resolve_usb_side_path(usb_root, anlz).unwrap_or_else(|| anlz.to_string());
                if !Path::new(&anlz_abs).is_file() {
                    warnings.push(format!(
                        "export verification warning: analysis file missing for track '{}': {}",
                        track.id, anlz_abs
                    ));
                }
            }
        }

        if verify_db {
            verify_edb_content(usb_root, playlist, manifest)?;
        }
        if verify_pdb {
            verify_pdb_content(usb_root, playlist, manifest)?;
        }

        warnings.push(format!(
            "export verification passed (db: {}, pdb: {}, tracks: {})",
            if verify_db { "checked" } else { "skipped" },
            if verify_pdb { "checked" } else { "skipped" },
            manifest.tracks.len()
        ));
        Ok(warnings)
    }

    fn load_playlist_for_export(&self, playlist_id: &str) -> BackendResult<ExportPlaylistData> {
        let conn = self.db.connect()?;
        let playlist = conn
            .query_row(
                "SELECT id, name FROM playlists WHERE id = ?1",
                params![playlist_id],
                |row| {
                    Ok(ExportPlaylistData {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        tracks: Vec::new(),
                    })
                },
            )
            .optional()?;
        let Some(mut playlist) = playlist else {
            return Err(BackendError::NotFound(format!(
                "playlist not found: {playlist_id}"
            )));
        };

        let mut stmt = conn.prepare(
            r#"
            SELECT t.id, t.title, t.artist, t.album, t.track_number, t.bpm, t.tonality, t.file_path,
                   t.file_modified_at, t.file_size_bytes, t.sample_rate_hz, t.bit_depth, t.bitrate_kbps,
                   t.disc_number, t.subtitle, t.comment, t.title_for_search, t.kuvo_delivery_comment,
                   t.dj_play_count, t.rating, t.color_id, t.artist_id_lyricist, t.artist_id_original_artist,
                   t.artist_id_remixer, t.artist_id_composer, t.genre_id, t.genre, t.label_id, t.isrc, t.release_year,
                   t.release_date, t.recorded_date, t.artwork_path, t.waveform_peaks_path, t.duration_ms, pt.position,
                   t.format_ext, t.first_beat_ms
            FROM playlist_tracks pt
            JOIN tracks t ON t.id = pt.track_id
            WHERE pt.playlist_id = ?1
            ORDER BY pt.position ASC
            "#,
        )?;
        let rows = stmt.query_map(params![playlist_id], |row| {
            let file_path = row.get::<_, String>(7)?;
            let file_name = Path::new(&file_path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("track")
                .to_string();
            Ok(ExportTrackData {
                id: row.get(0)?,
                title: row.get(1)?,
                artist: row.get(2)?,
                album: row.get(3)?,
                track_number: row.get(4)?,
                bpm: row.get(5)?,
                key: row.get(6)?,
                file_path,
                file_name,
                file_modified_at: row.get(8)?,
                file_size_bytes: row.get(9)?,
                sample_rate_hz: row.get(10)?,
                bit_depth: row.get(11)?,
                bitrate_kbps: row.get(12)?,
                disc_number: row.get(13)?,
                subtitle: row.get(14)?,
                comment: row.get(15)?,
                title_for_search: row.get(16)?,
                kuvo_delivery_comment: row.get(17)?,
                dj_play_count: row.get(18)?,
                rating: row.get(19)?,
                color_id: row.get(20)?,
                artist_id_lyricist: row.get(21)?,
                artist_id_original_artist: row.get(22)?,
                artist_id_remixer: row.get(23)?,
                artist_id_composer: row.get(24)?,
                genre_id: row.get(25)?,
                genre: row.get(26)?,
                label_id: row.get(27)?,
                isrc: row.get(28)?,
                release_year: row.get(29)?,
                release_date: row.get(30)?,
                recorded_date: row.get(31)?,
                artwork_path: row.get(32)?,
                waveform_peaks_path: row.get(33)?,
                duration_ms: row.get(34)?,
                position: row.get::<_, i64>(35)? as usize,
                file_type: row
                    .get::<_, Option<String>>(36)?
                    .as_deref()
                    .map(Self::file_type_from_extension),
                first_beat_ms: row.get::<_, Option<i64>>(37)?.map(|v| v as u32),
            })
        })?;
        playlist.tracks = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(playlist)
    }

    fn file_type_from_extension(ext: &str) -> i64 {
        // File type codes (USB export format):
        // MP3=1, MP4=3, M4A=4, FLAC=5, ALAC=6, WAV=11, AIFF=12
        match ext.trim().to_ascii_lowercase().as_str() {
            "mp3" => 1,
            "mp4" => 3,
            "m4a" | "aac" | "m4p" => 4,
            "flac" => 5,
            "alac" => 6,
            "wav" => 11,
            "aif" | "aiff" => 12,
            _ => 0,
        }
    }

    fn resolve_manifest_identity(
        conn: &rusqlite::Connection,
        track_id: &str,
        existing_identity: Option<&(Option<u32>, Option<u32>, Option<u32>, Option<String>)>,
        owns_waveform: bool,
        app_master_db_id: i64,
        app_content_link_id: i64,
    ) -> BackendResult<(Option<i64>, Option<i64>, Option<i64>)> {
        if let Some((existing_mdb, existing_mci, existing_cl, _)) = existing_identity
            && (existing_mdb.is_some() || existing_mci.is_some() || existing_cl.is_some())
        {
            return Ok((
                existing_mdb.map(i64::from),
                existing_mci.map(i64::from),
                existing_cl.map(i64::from),
            ));
        }

        if owns_waveform {
            return Ok((
                Some(app_master_db_id),
                Some(Self::ensure_track_master_content_id(conn, track_id)?),
                Some(app_content_link_id),
            ));
        }

        Ok((None, None, None))
    }

    fn ensure_track_export_identity_schema(conn: &rusqlite::Connection) -> BackendResult<()> {
        conn.execute_batch(
            r#"
        CREATE TABLE IF NOT EXISTS track_export_identity (
          track_id TEXT PRIMARY KEY,
          master_content_id INTEGER NOT NULL UNIQUE,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        "#,
        )?;
        Ok(())
    }

    fn ensure_local_u32_setting(
        conn: &rusqlite::Connection,
        key: &str,
        seed_suffix: &str,
    ) -> BackendResult<i64> {
        let existing = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(existing) = existing
            && let Ok(parsed) = existing.parse::<i64>()
            && parsed > 0
        {
            return Ok(parsed);
        }

        let seed = format!(
            "{}:{}:{}",
            key,
            seed_suffix,
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let mut value = i64::from(stable_u32_hash(&seed));
        if value <= 0 {
            value = 1;
        }
        let now_ts = now();
        conn.execute(
            r#"
        INSERT INTO app_settings (key, value, updated_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(key) DO UPDATE SET
            value = excluded.value,
            updated_at = excluded.updated_at
        "#,
            params![key, value.to_string(), now_ts],
        )?;
        Ok(value)
    }

    fn ensure_track_master_content_id(
        conn: &rusqlite::Connection,
        track_id: &str,
    ) -> BackendResult<i64> {
        if let Some(existing) = conn
            .query_row(
                "SELECT master_content_id FROM track_export_identity WHERE track_id = ?1",
                params![track_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            && existing > 0
        {
            return Ok(existing);
        }

        let mut candidate = i64::from(stable_u32_hash(&format!("master-content:{track_id}")));
        if candidate <= 0 {
            candidate = 1;
        }
        while conn
            .query_row(
                "SELECT track_id FROM track_export_identity WHERE master_content_id = ?1 LIMIT 1",
                params![candidate],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .is_some()
        {
            candidate += 1;
            if candidate <= 0 {
                candidate = 1;
            }
        }

        let now_ts = now();
        conn.execute(
            r#"
        INSERT INTO track_export_identity (track_id, master_content_id, created_at, updated_at)
        VALUES (?1, ?2, ?3, ?3)
        ON CONFLICT(track_id) DO UPDATE SET
            master_content_id = excluded.master_content_id,
            updated_at = excluded.updated_at
        "#,
            params![track_id, candidate, now_ts],
        )?;
        Ok(candidate)
    }

    fn load_export_owned_files(&self, key: &str) -> BackendResult<HashSet<String>> {
        let conn = self.db.connect()?;
        let raw = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(raw) = raw else {
            return Ok(HashSet::new());
        };
        let parsed = serde_json::from_str::<Vec<String>>(&raw)
            .unwrap_or_default()
            .into_iter()
            .collect::<HashSet<_>>();
        Ok(parsed)
    }

    fn save_export_owned_files(&self, key: &str, paths: &HashSet<String>) -> BackendResult<()> {
        let conn = self.db.connect()?;
        let mut ordered = paths.iter().cloned().collect::<Vec<_>>();
        ordered.sort();
        let encoded = serde_json::to_string(&ordered).map_err(|err| {
            BackendError::Internal(format!("serialize export-owned files: {err}"))
        })?;
        let now = now();
        conn.execute(
            r#"
            INSERT INTO app_settings (key, value, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = excluded.updated_at
            "#,
            params![key, encoded, now],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_playlist_tracks_analysis_ready, export_warning_entry, has_required_analysis,
        has_required_analysis_fields,
    };
    use crate::error::BackendError;
    use crate::service::export_helpers::{ExportPlaylistData, ExportTrackData};
    use std::path::Path;
    use tempfile::tempdir;

    fn make_track() -> ExportTrackData {
        ExportTrackData {
            id: "t1".to_string(),
            title: "Track".to_string(),
            artist: "Artist".to_string(),
            album: Some("Album".to_string()),
            track_number: Some(1),
            bpm: Some(128.0),
            key: Some("Am".to_string()),
            file_path: "/tmp/track.mp3".to_string(),
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
            waveform_peaks_path: Some("/tmp/waveform.dat".to_string()),
            duration_ms: Some(120_000),
            first_beat_ms: None,
            position: 0,
        }
    }

    fn write_test_analysis_bundle(dir: &Path, stem: &str) -> String {
        let dat = dir.join(format!("{stem}.DAT"));
        let ext = dir.join(format!("{stem}.EXT"));
        let twoex = dir.join(format!("{stem}.2EX"));
        std::fs::write(&dat, b"bad-dat").expect("write DAT");
        std::fs::write(&ext, b"bad-ext").expect("write EXT");
        std::fs::write(&twoex, b"bad-2ex").expect("write 2EX");
        dat.to_string_lossy().to_string()
    }

    #[test]
    fn required_analysis_allows_missing_key() {
        let mut track = make_track();
        track.key = None;
        let dir = tempdir().expect("tempdir");
        track.waveform_peaks_path = Some(write_test_analysis_bundle(dir.path(), "waveform"));
        assert!(has_required_analysis(dir.path(), &track));
    }

    #[test]
    fn required_analysis_still_requires_waveform_bpm_and_duration() {
        let mut track = make_track();
        track.waveform_peaks_path = None;
        assert!(!has_required_analysis_fields(&track));

        let mut track = make_track();
        track.bpm = None;
        assert!(!has_required_analysis_fields(&track));

        let mut track = make_track();
        track.duration_ms = None;
        assert!(!has_required_analysis_fields(&track));
    }

    #[test]
    fn required_analysis_requires_existing_anlz_bundle_but_not_valid_content() {
        let dir = tempdir().expect("tempdir");
        let mut track = make_track();
        track.waveform_peaks_path = Some(write_test_analysis_bundle(dir.path(), "invalid"));

        assert!(
            has_required_analysis(dir.path(), &track),
            "export readiness should require files, not validate ANLZ quality"
        );

        let mut missing_bundle = make_track();
        missing_bundle.waveform_peaks_path =
            Some(dir.path().join("missing.DAT").to_string_lossy().to_string());
        assert!(!has_required_analysis(dir.path(), &missing_bundle));
    }

    #[test]
    fn playlist_guard_message_mentions_current_required_fields() {
        let mut track = make_track();
        track.key = None;
        let playlist = ExportPlaylistData {
            id: "pl1".to_string(),
            name: "Playlist".to_string(),
            tracks: vec![track],
        };
        let dir = tempdir().expect("tempdir");
        let mut playlist = playlist;
        playlist.tracks[0].waveform_peaks_path =
            Some(write_test_analysis_bundle(dir.path(), "ready"));
        assert!(ensure_playlist_tracks_analysis_ready(dir.path(), &playlist).is_ok());

        let mut missing_waveform = make_track();
        missing_waveform.waveform_peaks_path = None;
        let playlist = ExportPlaylistData {
            id: "pl1".to_string(),
            name: "Playlist".to_string(),
            tracks: vec![missing_waveform],
        };
        let err = ensure_playlist_tracks_analysis_ready(dir.path(), &playlist).unwrap_err();
        match err {
            BackendError::ValidationWithDetails(msg, details) => {
                assert!(msg.contains("missing required analysis"));
                assert_eq!(details["validationType"], "missing_analysis");
                assert_eq!(
                    details["requiredFields"],
                    serde_json::json!(["waveform", "bpm", "duration"])
                );
                assert_eq!(details["missingTrackCount"], 1);
                assert_eq!(details["missingAnalysisBundleCount"], 0);
                assert_eq!(details["totalTrackCount"], 1);
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn playlist_guard_reports_missing_analysis_bundle() {
        let dir = tempdir().expect("tempdir");
        let mut track = make_track();
        track.waveform_peaks_path =
            Some(dir.path().join("missing.DAT").to_string_lossy().to_string());
        let playlist = ExportPlaylistData {
            id: "pl1".to_string(),
            name: "Playlist".to_string(),
            tracks: vec![track],
        };

        let err = ensure_playlist_tracks_analysis_ready(dir.path(), &playlist).unwrap_err();
        match err {
            BackendError::ValidationWithDetails(msg, details) => {
                assert!(msg.contains("missing required analysis"));
                assert_eq!(details["validationType"], "missing_analysis");
                assert_eq!(details["missingTrackCount"], 1);
                assert_eq!(details["missingAnalysisBundleCount"], 1);
                assert_eq!(details["totalTrackCount"], 1);
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn export_verification_and_prune_summary_are_info_not_warn() {
        let verification = export_warning_entry(
            "export verification passed (db: skipped, pdb: checked, tracks: 2)".to_string(),
        );
        assert_eq!(verification.level, "info");
        assert_eq!(verification.code, "export.info");

        let prune = export_warning_entry(
            "prune stale enabled: removed 0, missing 0, skipped 0".to_string(),
        );
        assert_eq!(prune.level, "info");
        assert_eq!(prune.code, "export.info");
    }
}
