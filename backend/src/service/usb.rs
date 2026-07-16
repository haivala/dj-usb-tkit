//! USB validation, playlist/history fetching, track inspection.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::edb::{
    open_edb_from_usb_root, table_exists, try_read_content_date_created_index_from_edb,
    try_read_playlists_with_metadata_from_edb, try_read_track_index_from_edb,
};
use crate::error::{BackendError, BackendResult};
use crate::models::{
    FetchUsbHistoriesData, FetchUsbHistoriesRequest, FetchUsbPlaylistsData,
    FetchUsbPlaylistsRequest, InspectUsbTrackData, InspectUsbTrackRequest, RemoveUsbPlaylistData,
    RemoveUsbPlaylistRequest, UsbHistory, UsbHistoryCounts, UsbImportStats, UsbPlaylist, UsbTrack,
    ValidateUsbRootData, ValidateUsbRootRequest, WarningEntry,
};
use crate::pdb_reader::{PdbHistoryEntryRow, PdbHistoryPlaylistRow, parse_pdb};

use super::analysis::normalize_text;
use super::export_helpers::{
    analysis_bundle_path_variants, prune_stale_export_owned_files,
    remove_playlist_and_tracks_from_pdb, remove_playlist_from_edb,
};
use super::usb_helpers::{
    PlaylistCandidate, build_usb_track_id_index, decode_history_playlist_id,
    decode_history_track_id, dedupe_usb_playlists_by_name, history_entry_sort_key,
    lookup_playlist_tracks, merge_playlist_tracks, normalize_packed_id,
    parse_history_name_numeric_id, parse_history_slot_id, sanitize_history_name, sanitize_text,
};
use super::usb_utils::{
    artwork_path_to_data_url, canonicalize_or_self, canonicalize_playlist_name, has_write_access,
    load_waveform_preview_from_analysis_path, normalize_usb_root_path, parse_history_numeric_id,
    resolve_usb_root, resolve_usb_side_path,
};
use super::usb_vendor_compat::{
    USB_CONTENTS_DIR, USB_VENDOR_ROOT_DIR, vendor_edb_path, vendor_pdb_path,
};
use super::{BackendService, build_track_match_fingerprint, export_log, now};

const SLOW_USB_STAGE_MS: u128 = 3_000;

fn build_usb_track_index(
    parsed: &crate::pdb_reader::ParsedPdb,
    usb_root: &std::path::Path,
) -> HashMap<u32, UsbTrack> {
    parsed
        .tracks
        .iter()
        .map(|t| {
            let artist = parsed
                .artists
                .get(&t.artist_id)
                .cloned()
                .unwrap_or_else(|| "Unknown Artist".to_string());
            let album = parsed.albums.get(&t.album_id).cloned();
            let key = parsed.keys.get(&t.key_id).cloned();
            let artwork_path = parsed
                .artworks
                .get(&t.artwork_id)
                .and_then(|p| resolve_usb_side_path(usb_root, p));
            let resolved_file_path = resolve_usb_side_path(usb_root, &t.track_file_path)
                .unwrap_or_else(|| t.track_file_path.clone());
            let usb_analysis_path = resolve_usb_side_path(usb_root, &t.anlz_path);
            (
                t.id,
                UsbTrack {
                    id: t.id.to_string(),
                    local_track_id: None,
                    title: if t.title.is_empty() {
                        "Unknown Title".to_string()
                    } else {
                        t.title.clone()
                    },
                    artist,
                    album,
                    track_number: (t.track_number > 0).then_some(t.track_number),
                    bpm: if t.tempo_x100 > 0 {
                        Some(t.tempo_x100 as f64 / 100.0)
                    } else {
                        None
                    },
                    key,
                    file_path: resolved_file_path,
                    usb_media_path: Some(t.track_file_path.clone()),
                    artwork_data_url: None,
                    artwork_path,
                    waveform_peaks_path: usb_analysis_path.clone(),
                    usb_analysis_path,
                    usb_analysis_path_raw: Some(t.anlz_path.clone()),
                    waveform_preview: None,
                    duration_ms: t.duration_seconds.map(|s| u64::from(s) * 1000),
                },
            )
        })
        .collect()
}

fn edb_track_index_from_playlist_tracks(
    playlist_tracks: Option<&HashMap<String, Vec<UsbTrack>>>,
) -> HashMap<u32, UsbTrack> {
    playlist_tracks
        .map(build_usb_track_id_index)
        .unwrap_or_default()
}

fn merge_full_edb_track_index(
    usb_root: &std::path::Path,
    track_by_id: &mut HashMap<u32, UsbTrack>,
    warnings: &mut Vec<String>,
) {
    if let Some(all_edb_tracks) = try_read_track_index_from_edb(usb_root, warnings) {
        track_by_id.extend(all_edb_tracks);
    }
}

fn select_history_rows(
    playlists: &[PdbHistoryPlaylistRow],
    entries: &[PdbHistoryEntryRow],
) -> (Vec<PdbHistoryPlaylistRow>, Vec<PdbHistoryEntryRow>) {
    let t17_playlists = playlists
        .iter()
        .filter(|row| row.source_table == 17)
        .cloned()
        .collect::<Vec<_>>();
    let t18_entries = entries
        .iter()
        .filter(|row| row.source_table == 18)
        .cloned()
        .collect::<Vec<_>>();
    let t11_playlists = playlists
        .iter()
        .filter(|row| row.source_table == 11)
        .cloned()
        .collect::<Vec<_>>();
    let t12_entries = entries
        .iter()
        .filter(|row| row.source_table == 12)
        .cloned()
        .collect::<Vec<_>>();

    // Runtime history import policy: prefer t11/t12 whenever present.
    // t17/t18 on initialized/exported sticks can contain seed/template rows
    // that should not be surfaced as user history playlists.
    if !t11_playlists.is_empty() || !t12_entries.is_empty() {
        (t11_playlists, t12_entries)
    } else {
        (t17_playlists, t18_entries)
    }
}

fn usb_warning(level: &str, code: &str, message: String) -> WarningEntry {
    WarningEntry {
        level: level.to_string(),
        code: code.to_string(),
        message,
        source: "usb-import".to_string(),
    }
}

fn playlist_log_entry_info(message: String) -> WarningEntry {
    usb_warning("info", "usb.playlists.info", message)
}

fn playlist_log_entry_warn(code: &str, message: String) -> WarningEntry {
    usb_warning("warn", code, message)
}

fn remove_playlist_warning_entry(message: String) -> WarningEntry {
    let (level, code) = if message.starts_with("deleted:") {
        ("info", "usb.remove.file-deleted")
    } else if message.starts_with("removed content row") {
        ("info", "usb.remove.db-cleaned")
    } else if message.contains("not found") || message.contains("missing") {
        ("warn", "usb.remove.file-missing")
    } else if message.starts_with("slow-media suspected:") {
        ("warn", "usb.remove.slow-media")
    } else {
        ("info", "usb.remove.info")
    };
    usb_warning(level, code, message)
}

fn history_warning_entry(message: String) -> WarningEntry {
    let (level, code) = if message.starts_with("slow-media suspected:") {
        ("warn", "usb.histories.slow-media")
    } else {
        ("info", "usb.histories.info")
    };
    usb_warning(level, code, message)
}

fn normalize_date_created(value: &str) -> Option<NaiveDate> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let ten = trimmed.chars().take(10).collect::<String>();
    NaiveDate::parse_from_str(&ten, "%Y-%m-%d").ok()
}

fn apply_history_dates_from_track_date_created(
    histories: &mut [UsbHistory],
    date_created_by_track_id: &HashMap<u32, String>,
) {
    if histories.is_empty() || date_created_by_track_id.is_empty() {
        return;
    }

    let mut carry = None::<NaiveDate>;
    for history in histories.iter_mut() {
        let existing = history
            .created_at
            .as_deref()
            .and_then(normalize_date_created);
        if let Some(date) = existing {
            carry = Some(carry.map_or(date, |prev| prev.max(date)));
            continue;
        }

        let own_latest = history
            .tracks
            .iter()
            .filter_map(|t| t.id.parse::<u32>().ok())
            .filter_map(|id| date_created_by_track_id.get(&id))
            .filter_map(|raw| normalize_date_created(raw))
            .max();

        let resolved = match (carry, own_latest) {
            (Some(prev), Some(current)) => Some(prev.max(current)),
            (Some(prev), None) => Some(prev),
            (None, Some(current)) => Some(current),
            (None, None) => None,
        };

        if let Some(date) = resolved {
            history.created_at = Some(date.format("%Y-%m-%d").to_string());
            carry = Some(date);
        }
    }
}

fn build_history_track_date_index(
    parsed_tracks: &[crate::pdb_reader::PdbTrackRow],
) -> HashMap<u32, String> {
    parsed_tracks
        .iter()
        .filter_map(|track| {
            let date = track
                .date_added
                .as_deref()
                .map(sanitize_text)
                .filter(|value| normalize_date_created(value).is_some())?;
            Some((track.id, date))
        })
        .collect()
}

/// Recursively remove empty directories bottom-up.
fn cleanup_empty_dirs_recursive(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            cleanup_empty_dirs_recursive(&path);
            // Try to remove if now empty (ignore errors)
            let _ = std::fs::remove_dir(&path);
        }
    }
}

fn push_usb_stage_timing(
    warnings: &mut Vec<String>,
    stage: &str,
    started: &mut std::time::Instant,
) {
    let elapsed = started.elapsed().as_millis();
    warnings.push(format!("stage timing: {stage}: {elapsed}ms"));
    if elapsed >= SLOW_USB_STAGE_MS {
        warnings.push(format!(
            "slow-media suspected: stage '{stage}' took {elapsed}ms"
        ));
    }
    *started = std::time::Instant::now();
}

impl BackendService {
    pub fn validate_usb_root(
        &self,
        req: ValidateUsbRootRequest,
    ) -> BackendResult<ValidateUsbRootData> {
        let mut warnings = Vec::<String>::new();
        let trimmed = req.path.trim();
        if trimmed.is_empty() {
            warnings.push("USB path is empty".to_string());
            return Ok(ValidateUsbRootData {
                valid: false,
                has_write_access: false,
                normalized_root: None,
                has_vendor_root: false,
                has_contents: false,
                has_pdb: false,
                has_edb: false,
                warnings,
            });
        }

        let raw = std::path::PathBuf::from(trimmed);
        let candidate = if raw.exists() {
            raw
        } else if raw.is_relative() {
            std::env::current_dir()?.join(raw)
        } else {
            raw
        };
        if !candidate.exists() {
            warnings.push(format!("Path does not exist: {}", candidate.display()));
            return Ok(ValidateUsbRootData {
                valid: false,
                has_write_access: false,
                normalized_root: None,
                has_vendor_root: false,
                has_contents: false,
                has_pdb: false,
                has_edb: false,
                warnings,
            });
        }

        let normalized = normalize_usb_root_path(canonicalize_or_self(candidate));
        let has_vendor_root = normalized.join(USB_VENDOR_ROOT_DIR).is_dir();
        let has_contents = normalized.join(USB_CONTENTS_DIR).is_dir();
        let has_pdb = vendor_pdb_path(&normalized).is_file();
        let has_edb = vendor_edb_path(&normalized).is_file();

        if !has_vendor_root {
            warnings.push("Missing vendor root directory".to_string());
        }
        if !has_contents {
            warnings.push("Missing contents directory".to_string());
        }
        if !has_pdb {
            warnings.push("Missing PDB in vendor db directory".to_string());
        }

        let has_write_access = has_write_access(&normalized);
        if !has_write_access {
            warnings
                .push("USB appears read-only: imports may work but export will fail".to_string());
        }

        Ok(ValidateUsbRootData {
            valid: has_vendor_root && has_contents,
            has_write_access,
            normalized_root: Some(normalized.to_string_lossy().to_string()),
            has_vendor_root,
            has_contents,
            has_pdb,
            has_edb,
            warnings,
        })
    }

    pub fn fetch_usb_playlists(
        &self,
        req: FetchUsbPlaylistsRequest,
    ) -> BackendResult<FetchUsbPlaylistsData> {
        self.fetch_usb_playlists_with_progress(req, |_, _, _| {})
    }

    pub fn fetch_usb_playlists_with_progress<F>(
        &self,
        req: FetchUsbPlaylistsRequest,
        mut on_progress: F,
    ) -> BackendResult<FetchUsbPlaylistsData>
    where
        F: FnMut(usize, usize, &str),
    {
        let mut warnings = Vec::<String>::new();
        let mut stage_started = std::time::Instant::now();
        on_progress(2, 100, "USB: Resolving root");
        on_progress(10, 100, "USB: Parsing PDB");
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        push_usb_stage_timing(&mut warnings, "resolve usb root", &mut stage_started);
        let pdb_path = vendor_pdb_path(&usb_root);
        let parsed = if pdb_path.exists() {
            let parsed = parse_pdb(&pdb_path)?;
            warnings.extend(parsed.warnings.clone());
            Some(parsed)
        } else {
            warnings.push(format!(
                "PDB not found under {}; continuing with eDB-only mode",
                usb_root.display()
            ));
            None
        };
        push_usb_stage_timing(&mut warnings, "parse PDB", &mut stage_started);

        on_progress(30, 100, "USB: Reading eDB");
        let edb_playlists = try_read_playlists_with_metadata_from_edb(&usb_root, &mut warnings);
        let edb_playlist_tracks = edb_playlists.as_ref().map(|m| {
            m.iter()
                .map(|(name, playlist)| (name.clone(), playlist.tracks.clone()))
                .collect::<HashMap<_, _>>()
        });
        let mut edb_track_by_id =
            edb_track_index_from_playlist_tracks(edb_playlist_tracks.as_ref());
        merge_full_edb_track_index(&usb_root, &mut edb_track_by_id, &mut warnings);
        let eedb_playlist_tracks_canonical = edb_playlist_tracks.as_ref().map(|m| {
            m.iter()
                .map(|(name, tracks)| (canonicalize_playlist_name(name), tracks.clone()))
                .collect::<HashMap<_, _>>()
        });
        push_usb_stage_timing(&mut warnings, "read eDB", &mut stage_started);

        let mut track_by_id = HashMap::<u32, UsbTrack>::new();
        let mut entries_by_playlist =
            HashMap::<u32, Vec<crate::pdb_reader::PdbPlaylistEntryRow>>::new();
        let mut playlist_candidates = Vec::<PlaylistCandidate>::new();

        if let Some(parsed) = &parsed {
            track_by_id = build_usb_track_index(parsed, &usb_root);

            entries_by_playlist =
                parsed
                    .playlist_entries
                    .iter()
                    .fold(HashMap::<u32, Vec<_>>::new(), |mut acc, e| {
                        acc.entry(e.playlist_id).or_default().push(e.clone());
                        acc
                    });

            let mut leaves = parsed
                .playlist_tree
                .iter()
                .filter(|n| !n.row_is_folder)
                .cloned()
                .collect::<Vec<_>>();
            leaves.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then(a.name.cmp(&b.name)));
            for node in leaves {
                let folder_name = parsed
                    .playlist_tree
                    .iter()
                    .find(|p| p.id == node.parent_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                let display_name = if folder_name.is_empty() {
                    node.name.clone()
                } else {
                    format!("{folder_name} / {}", node.name)
                };
                playlist_candidates.push(PlaylistCandidate {
                    pdb_id: Some(node.id),
                    short_name: node.name,
                    display_name,
                    sort_order: node.sort_order,
                });
            }
        }

        on_progress(50, 100, "USB: Resolving eDB candidates");
        let mut seen = playlist_candidates
            .iter()
            .map(|c| canonicalize_playlist_name(&c.display_name))
            .collect::<HashSet<_>>();
        if let Some(map) = &edb_playlist_tracks {
            for (idx, name) in map.keys().enumerate() {
                let key = canonicalize_playlist_name(name);
                if seen.insert(key) {
                    playlist_candidates.push(PlaylistCandidate {
                        pdb_id: None,
                        short_name: name.clone(),
                        display_name: name.clone(),
                        sort_order: u32::MAX.saturating_sub(20000).saturating_add(idx as u32),
                    });
                }
            }
        }
        playlist_candidates.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then(a.display_name.cmp(&b.display_name))
        });

        on_progress(70, 100, "USB: Resolving playlists");
        let mut items = Vec::new();
        let mut referenced_track_ids = HashSet::<u32>::new();
        let mut playlist_entries_total = 0usize;
        let mut source_counts = HashMap::<&'static str, usize>::new();
        let mut empty_source_playlists = Vec::<String>::new();
        for candidate in playlist_candidates {
            let mut pdb_tracks = Vec::<UsbTrack>::new();
            if let Some(pdb_id) = candidate.pdb_id {
                let mut rows = entries_by_playlist.remove(&pdb_id).unwrap_or_default();
                rows.sort_by_key(|e| e.entry_index);
                for entry in rows {
                    referenced_track_ids.insert(entry.track_id);
                    if let Some(track) = track_by_id.get(&entry.track_id) {
                        pdb_tracks.push(track.clone());
                    } else if let Some(track) = edb_track_by_id.get(&entry.track_id) {
                        pdb_tracks.push(track.clone());
                    }
                }
            }

            let export_tracks = lookup_playlist_tracks(
                &edb_playlist_tracks,
                &eedb_playlist_tracks_canonical,
                &candidate.short_name,
                &candidate.display_name,
            )
            .cloned()
            .unwrap_or_default();
            let (playlist_tracks, source) = merge_playlist_tracks(&pdb_tracks, &export_tracks);
            for t in &playlist_tracks {
                if let Ok(id) = t.id.parse::<u32>() {
                    referenced_track_ids.insert(id);
                }
            }
            if playlist_tracks.is_empty() {
                empty_source_playlists.push(candidate.display_name.clone());
            }
            *source_counts.entry(source).or_insert(0) += 1;
            playlist_entries_total += playlist_tracks.len();

            items.push(UsbPlaylist {
                id: candidate
                    .pdb_id
                    .map(|id| format!("usb-pl-{id}"))
                    .unwrap_or_else(|| {
                        format!(
                            "usb-pl-name-{}",
                            canonicalize_playlist_name(&candidate.display_name)
                        )
                    }),
                name: candidate.display_name.clone(),
                source: source.to_string(),
                track_count: playlist_tracks.len(),
                tracks: playlist_tracks,
            });
        }
        push_usb_stage_timing(&mut warnings, "resolve playlists", &mut stage_started);
        let (deduped_items, collapsed) = dedupe_usb_playlists_by_name(items);
        items = deduped_items;
        if collapsed > 0 {
            warnings.push(format!(
                "collapsed {collapsed} duplicate playlist name(s) from USB sources"
            ));
        }

        on_progress(90, 100, "USB: Finalizing playlist import");
        let stats = UsbImportStats {
            indexed_tracks: parsed.as_ref().map(|p| p.tracks.len()).unwrap_or(0),
            playlist_referenced_tracks: referenced_track_ids.len(),
            playlist_entries: playlist_entries_total,
        };
        let materialized_tracks = self.materialize_usb_playlist_tracks(&mut items)?;
        push_usb_stage_timing(
            &mut warnings,
            "finalize playlist import",
            &mut stage_started,
        );

        warnings.insert(0, format!("USB root in use: {}", usb_root.display()));

        Ok(FetchUsbPlaylistsData {
            items,
            stats,
            warnings: {
                if !source_counts.is_empty() {
                    let pdb_count = source_counts.get("pdb").copied().unwrap_or(0);
                    let edb_count = source_counts.get("eDB").copied().unwrap_or(0);
                    if edb_count > 0 {
                        warnings.push(format!(
                            "used eDB as playlist source for {edb_count} playlist(s)"
                        ));
                    }
                    if pdb_count > 0 {
                        warnings.push(format!(
                            "used PDB as playlist source for {pdb_count} playlist(s)"
                        ));
                    }
                }
                if !empty_source_playlists.is_empty() {
                    warnings.push(format!(
                        "{} playlist(s) had zero static track entries in export data: {}",
                        empty_source_playlists.len(),
                        empty_source_playlists.join(", ")
                    ));
                }
                if materialized_tracks > 0 {
                    warnings.push(format!(
                        "materialized {materialized_tracks} USB track row(s) into local library"
                    ));
                }
                warnings
                    .into_iter()
                    .map(|message| {
                        if message.starts_with("slow-media suspected:") {
                            playlist_log_entry_warn("usb.playlists.slow-media", message)
                        } else {
                            playlist_log_entry_info(message)
                        }
                    })
                    .collect()
            },
        })
    }

    pub fn fetch_usb_histories(
        &self,
        req: FetchUsbHistoriesRequest,
    ) -> BackendResult<FetchUsbHistoriesData> {
        self.fetch_usb_histories_with_progress(req, |_, _, _| {})
    }

    pub fn remove_usb_playlist(
        &self,
        req: RemoveUsbPlaylistRequest,
    ) -> BackendResult<RemoveUsbPlaylistData> {
        self.remove_usb_playlist_with_progress(req, |_, _, _| {})
    }

    pub fn remove_usb_playlist_with_progress<F>(
        &self,
        req: RemoveUsbPlaylistRequest,
        mut on_progress: F,
    ) -> BackendResult<RemoveUsbPlaylistData>
    where
        F: FnMut(usize, usize, &str),
    {
        let mut warnings = Vec::<String>::new();
        let mut stage_started = std::time::Instant::now();
        let name = req.playlist_name.trim().to_string();
        if name.is_empty() {
            return Err(BackendError::Validation(
                "playlistName must not be empty".to_string(),
            ));
        }

        // Stage 1: Resolve USB root, name candidates (0-10%)
        on_progress(0, 100, "USB: Resolving USB root");
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        push_usb_stage_timing(&mut warnings, "resolve usb root", &mut stage_started);
        on_progress(5, 100, "USB: Resolving playlist identifiers");
        let mut name_candidates = vec![name.clone()];
        if let Some((_, leaf)) = name.rsplit_once(" / ") {
            let leaf_trimmed = leaf.trim().to_string();
            if !leaf_trimmed.is_empty()
                && !name_candidates.iter().any(|n| {
                    canonicalize_playlist_name(n) == canonicalize_playlist_name(&leaf_trimmed)
                })
            {
                name_candidates.push(leaf_trimmed);
            }
        }
        push_usb_stage_timing(
            &mut warnings,
            "resolve playlist identifiers",
            &mut stage_started,
        );

        // Stage 2: Remove playlist + detect shared tracks in PDB (10-30%)
        on_progress(10, 100, "USB: Analyzing playlists and shared tracks");
        let pdb_result = remove_playlist_and_tracks_from_pdb(
            &usb_root,
            req.playlist_id.as_deref(),
            &name_candidates,
            &mut warnings,
        )?;
        push_usb_stage_timing(
            &mut warnings,
            "remove playlist and tracks from PDB",
            &mut stage_started,
        );

        let removed_pdb = pdb_result.removed_playlist_count;
        let tracks_removed = pdb_result.exclusive_tracks.len();
        let tracks_kept_shared = pdb_result.shared_track_count;

        if tracks_kept_shared > 0 {
            warnings.push(format!(
                "{tracks_kept_shared} shared tracks preserved (used by other playlists)"
            ));
        }

        // Stage 3: Delete exclusive track files (30-65%)
        on_progress(30, 100, "USB: Deleting exclusive track files");
        let mut stale_paths = Vec::<String>::new();
        let mut exclusive_track_file_paths = Vec::<String>::new();

        for track in &pdb_result.exclusive_tracks {
            // Audio file
            if !track.track_file_path.is_empty() {
                stale_paths.push(track.track_file_path.clone());
                exclusive_track_file_paths.push(track.track_file_path.clone());
            }
            // ANLZ bundle (.DAT/.EXT/.2EX)
            if !track.anlz_path.is_empty() {
                for variant in analysis_bundle_path_variants(&track.anlz_path) {
                    stale_paths.push(variant);
                }
            }
        }

        // Artwork files for exclusive artwork_ids
        let exclusive_artwork_ids: std::collections::HashSet<u32> = pdb_result
            .exclusive_tracks
            .iter()
            .filter(|t| t.artwork_id != 0)
            .map(|t| t.artwork_id)
            .collect();
        // Read artwork paths from parsed PDB artworks map
        if !exclusive_artwork_ids.is_empty() {
            let pdb_path = usb_root
                .join(USB_VENDOR_ROOT_DIR)
                .join("rekordbox")
                .join("export.pdb");
            if pdb_path.is_file()
                && let Ok(parsed) = parse_pdb(&pdb_path) {
                    for art_id in &exclusive_artwork_ids {
                        if let Some(art_path) = parsed.artworks.get(art_id) {
                            // Small artwork
                            stale_paths.push(art_path.clone());
                            // Medium variant: replace .jpg with _m.jpg
                            let medium =
                                art_path.replace(".jpg", "_m.jpg").replace(".png", "_m.png");
                            if medium != *art_path {
                                stale_paths.push(medium);
                            }
                        }
                    }
                }
        }

        let mut files_deleted = 0usize;
        if !stale_paths.is_empty() {
            let prune_result =
                prune_stale_export_owned_files(&usb_root, &stale_paths, &mut warnings)?;
            files_deleted = prune_result.removed;
            if prune_result.missing > 0 {
                warnings.push(format!(
                    "{} files already missing from USB",
                    prune_result.missing
                ));
            }
            for path in &stale_paths {
                warnings.push(format!("deleted: {path}"));
            }
        }
        push_usb_stage_timing(
            &mut warnings,
            "delete exclusive track files",
            &mut stage_started,
        );

        // Stage 4: Remove from eDB (65-80%)
        on_progress(65, 100, "USB: Cleaning eDB");
        let removed_edb = remove_playlist_from_edb(
            &usb_root,
            &name_candidates,
            &exclusive_track_file_paths,
            &mut warnings,
        )?;
        push_usb_stage_timing(
            &mut warnings,
            "remove playlist from eDB",
            &mut stage_started,
        );

        // Stage 5: Clean up empty directories under Contents/ (80-95%)
        on_progress(80, 100, "USB: Cleaning empty directories");
        let contents_dir = usb_root.join("Contents");
        if contents_dir.is_dir() {
            cleanup_empty_dirs_recursive(&contents_dir);
        }
        push_usb_stage_timing(
            &mut warnings,
            "cleanup empty directories",
            &mut stage_started,
        );

        if removed_edb == 0 && removed_pdb == 0 {
            return Err(BackendError::NotFound(format!(
                "USB playlist not found: {}",
                name
            )));
        }

        // Stage 6: Finalize (95-100%)
        on_progress(95, 100, "USB: Finalizing");
        push_usb_stage_timing(
            &mut warnings,
            "finalize playlist removal",
            &mut stage_started,
        );
        on_progress(100, 100, "USB: Playlist removal completed");

        Ok(RemoveUsbPlaylistData {
            playlist_name: name,
            removed_from_edb: removed_edb,
            removed_from_pdb: removed_pdb,
            tracks_removed,
            files_deleted,
            tracks_kept_shared,
            warnings: warnings
                .into_iter()
                .map(remove_playlist_warning_entry)
                .collect(),
        })
    }

    pub fn fetch_usb_histories_with_progress<F>(
        &self,
        req: FetchUsbHistoriesRequest,
        mut on_progress: F,
    ) -> BackendResult<FetchUsbHistoriesData>
    where
        F: FnMut(usize, usize, &str),
    {
        let mut stage_warnings = Vec::<String>::new();
        let mut stage_started = std::time::Instant::now();
        on_progress(2, 100, "USB: Resolving root");
        on_progress(10, 100, "USB: Parsing PDB");
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        push_usb_stage_timing(&mut stage_warnings, "resolve usb root", &mut stage_started);
        let pdb_path = vendor_pdb_path(&usb_root);
        if !pdb_path.exists() {
            return Ok(FetchUsbHistoriesData {
                items: Vec::new(),
                counts: UsbHistoryCounts {
                    imported_playlists: 0,
                    imported_tracks: 0,
                    pdb_t11_playlists: 0,
                    pdb_t12_entries: 0,
                    pdb_t17_playlists: 0,
                    pdb_t18_entries: 0,
                    edb_history_rows: 0,
                    edb_history_content_rows: 0,
                },
                warnings: vec![
                    format!("USB root in use: {}", usb_root.display()),
                    format!(
                        "PDB not found under {}; history import requires PDB",
                        usb_root.display()
                    ),
                ]
                .into_iter()
                .map(history_warning_entry)
                .collect(),
            });
        }

        let parsed = parse_pdb(&pdb_path)?;
        push_usb_stage_timing(&mut stage_warnings, "parse PDB", &mut stage_started);
        on_progress(30, 100, "USB: Reading supplemental databases");
        let mut supplemental_warnings = Vec::<String>::new();
        let supplemental_edb_playlist_tracks =
            try_read_playlists_with_metadata_from_edb(&usb_root, &mut supplemental_warnings).map(
                |m| {
                    m.into_iter()
                        .map(|(name, playlist)| (name, playlist.tracks))
                        .collect::<HashMap<_, _>>()
                },
            );
        let mut supplemental_track_by_id =
            edb_track_index_from_playlist_tracks(supplemental_edb_playlist_tracks.as_ref());
        merge_full_edb_track_index(
            &usb_root,
            &mut supplemental_track_by_id,
            &mut supplemental_warnings,
        );
        let mut date_created_by_track_id = build_history_track_date_index(&parsed.tracks);
        if date_created_by_track_id.is_empty() {
            date_created_by_track_id =
                try_read_content_date_created_index_from_edb(&usb_root, &mut supplemental_warnings)
                    .unwrap_or_default();
        }
        let export_log = match export_log::load_export_log(&usb_root) {
            Ok(log) => log,
            Err(err) => {
                supplemental_warnings.push(format!("USB export log ignored: {err}"));
                None
            }
        };
        let (edb_history_rows, edb_history_content_rows) =
            if let Some(conn) = open_edb_from_usb_root(&usb_root, &mut supplemental_warnings) {
                let history_rows = if table_exists(&conn, "history") {
                    conn.query_row(
                        "SELECT COUNT(*) FROM history",
                        [],
                        |row: &rusqlite::Row<'_>| row.get::<_, i64>(0),
                    )
                    .ok()
                    .unwrap_or(0)
                    .max(0) as usize
                } else {
                    0
                };
                let history_content_rows = if table_exists(&conn, "history_content") {
                    conn.query_row(
                        "SELECT COUNT(*) FROM history_content",
                        [],
                        |row: &rusqlite::Row<'_>| row.get::<_, i64>(0),
                    )
                    .ok()
                    .unwrap_or(0)
                    .max(0) as usize
                } else {
                    0
                };
                (history_rows, history_content_rows)
            } else {
                (0, 0)
            };
        push_usb_stage_timing(
            &mut stage_warnings,
            "read supplemental databases",
            &mut stage_started,
        );
        let history_date_by_num = parsed.history_rows.iter().fold(
            std::collections::HashMap::<u32, String>::new(),
            |mut acc, row| {
                let date = row
                    .date
                    .as_deref()
                    .map(sanitize_text)
                    .filter(|v| !v.is_empty());
                let num = row.num.as_deref().and_then(parse_history_slot_id);
                if let (Some(num), Some(date)) = (num, date) {
                    acc.entry(num).or_insert(date);
                }
                acc
            },
        );

        on_progress(50, 100, "USB: Building track index");
        let track_by_id = build_usb_track_index(&parsed, &usb_root);
        push_usb_stage_timing(&mut stage_warnings, "build track index", &mut stage_started);

        on_progress(70, 100, "USB: Resolving history entries");
        let (history_playlists, selected_history_entries) =
            select_history_rows(&parsed.history_playlists, &parsed.history_entries);

        let known_history_ids = history_playlists
            .iter()
            .map(|row| normalize_packed_id(row.id))
            .collect::<HashSet<_>>();

        let mut entries_by_history = selected_history_entries.iter().fold(
            std::collections::HashMap::<u32, Vec<_>>::new(),
            |mut acc, e| {
                let decoded =
                    decode_history_playlist_id(e.playlist_id, e.entry_index, &known_history_ids)
                        .unwrap_or_else(|| normalize_packed_id(e.playlist_id));
                acc.entry(decoded).or_default().push(e);
                acc
            },
        );

        let mut items = history_playlists
            .iter()
            .map(|row| {
                let logical_playlist_id = normalize_packed_id(row.id);
                let mut entries = entries_by_history
                    .remove(&logical_playlist_id)
                    .unwrap_or_default();
                entries.sort_by_key(|e| history_entry_sort_key(e.entry_index));
                let tracks = entries
                    .iter()
                    .filter_map(|e| {
                        e.track_id
                            .or_else(|| decode_history_track_id(e.playlist_id, e.entry_index))
                    })
                    .map(|track_id| {
                        track_by_id
                            .get(&track_id)
                            .cloned()
                            .or_else(|| supplemental_track_by_id.get(&track_id).cloned())
                            .unwrap_or_else(|| {
                                supplemental_track_by_id.get(&track_id).cloned().unwrap_or(
                                    UsbTrack {
                                        id: track_id.to_string(),
                                        local_track_id: None,
                                        title: format!("Unknown Track #{track_id}"),
                                        artist: "Unknown Artist".to_string(),
                                        album: None,
                                        track_number: None,
                                        bpm: None,
                                        key: None,
                                        file_path: String::new(),
                                        usb_media_path: None,
                                        artwork_path: None,
                                        artwork_data_url: None,
                                        waveform_peaks_path: None,
                                        usb_analysis_path: None,
                                        usb_analysis_path_raw: None,
                                        waveform_preview: None,
                                        duration_ms: None,
                                    },
                                )
                            })
                    })
                    .collect::<Vec<_>>();

                let cleaned_name = sanitize_history_name(&row.name);
                let created_at = history_date_by_num
                    .get(&logical_playlist_id)
                    .cloned()
                    .or_else(|| {
                        parse_history_name_numeric_id(&cleaned_name)
                            .and_then(|n| history_date_by_num.get(&n).cloned())
                    });
                UsbHistory {
                    id: format!("usb-h-{}", logical_playlist_id),
                    name: if cleaned_name.is_empty() {
                        format!("History {logical_playlist_id}")
                    } else {
                        cleaned_name
                    },
                    created_at,
                    tracks,
                }
            })
            .collect::<Vec<_>>();
        push_usb_stage_timing(
            &mut stage_warnings,
            "resolve history entries",
            &mut stage_started,
        );

        items.sort_by_key(|h| parse_history_numeric_id(&h.id));
        export_log::apply_history_dates_from_export_log(&mut items, export_log.as_ref());
        apply_history_dates_from_track_date_created(&mut items, &date_created_by_track_id);

        on_progress(90, 100, "USB: Finalizing history import");
        let mut warnings = parsed.warnings;
        warnings.push(format!(
            "history import: selected populated history table family from t11/t12 vs t17/t18: playlists={}, entries={}",
            history_playlists.len(),
            selected_history_entries.len()
        ));
        warnings.insert(0, format!("USB root in use: {}", usb_root.display()));
        warnings.extend(supplemental_warnings);
        warnings.extend(stage_warnings);
        let materialized_tracks = self.materialize_usb_history_tracks(&mut items)?;
        push_usb_stage_timing(&mut warnings, "finalize history import", &mut stage_started);
        if materialized_tracks > 0 {
            warnings.push(format!(
                "materialized {materialized_tracks} USB history track row(s) into local library"
            ));
        }

        let imported_tracks = items.iter().map(|history| history.tracks.len()).sum();
        Ok(FetchUsbHistoriesData {
            items,
            counts: UsbHistoryCounts {
                imported_playlists: history_playlists.len(),
                imported_tracks,
                pdb_t11_playlists: parsed
                    .history_playlists
                    .iter()
                    .filter(|row| row.source_table == 11)
                    .count(),
                pdb_t12_entries: parsed
                    .history_entries
                    .iter()
                    .filter(|row| row.source_table == 12)
                    .count(),
                pdb_t17_playlists: parsed
                    .history_playlists
                    .iter()
                    .filter(|row| row.source_table == 17)
                    .count(),
                pdb_t18_entries: parsed
                    .history_entries
                    .iter()
                    .filter(|row| row.source_table == 18)
                    .count(),
                edb_history_rows,
                edb_history_content_rows,
            },
            warnings: warnings.into_iter().map(history_warning_entry).collect(),
        })
    }

    fn materialize_usb_playlist_tracks(
        &self,
        playlists: &mut [UsbPlaylist],
    ) -> BackendResult<usize> {
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        let now_ts = now();
        let mut materialized = 0usize;

        for playlist in playlists {
            for track in &mut playlist.tracks {
                if self.materialize_usb_track_row(&tx, track, &now_ts)? {
                    materialized += 1;
                }
            }
        }

        tx.commit()?;
        Ok(materialized)
    }

    fn materialize_usb_history_tracks(&self, histories: &mut [UsbHistory]) -> BackendResult<usize> {
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        let now_ts = now();
        let mut materialized = 0usize;

        for history in histories {
            for track in &mut history.tracks {
                if self.materialize_usb_track_row(&tx, track, &now_ts)? {
                    materialized += 1;
                }
            }
        }

        tx.commit()?;
        Ok(materialized)
    }

    fn materialize_usb_track_row(
        &self,
        tx: &rusqlite::Transaction<'_>,
        track: &mut UsbTrack,
        now_ts: &str,
    ) -> BackendResult<bool> {
        let file_path = track.file_path.trim();
        if file_path.is_empty() {
            return Ok(false);
        }

        let existing_id = tx
            .query_row(
                "SELECT id FROM tracks WHERE file_path = ?1 LIMIT 1",
                params![file_path],
                |row| row.get::<_, String>(0),
            )
            .optional()?;

        let id = existing_id.unwrap_or_else(|| Uuid::now_v7().to_string());
        let file_size_bytes: Option<i64> = None;
        let fingerprint =
            build_track_match_fingerprint(&track.title, &track.artist, track.album.as_deref());
        let track_number = track.track_number.map(|v| v.max(1));
        let bpm = track.bpm.filter(|&v| v > 0.0);
        let key = track
            .key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string);

        tx.execute(
            r#"
            INSERT INTO tracks (
              id, title, artist, album, track_number, bpm, tonality, file_path, file_size_bytes,
              file_modified_at, artwork_path, waveform_peaks_path, duration_ms, match_fingerprint, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12, ?13, ?14, ?14)
            ON CONFLICT(file_path) DO UPDATE SET
              title = excluded.title,
              artist = excluded.artist,
              album = excluded.album,
              track_number = COALESCE(excluded.track_number, tracks.track_number),
              bpm = COALESCE(excluded.bpm, tracks.bpm),
              tonality = excluded.tonality,
              file_size_bytes = COALESCE(excluded.file_size_bytes, tracks.file_size_bytes),
              artwork_path = excluded.artwork_path,
              waveform_peaks_path = COALESCE(excluded.waveform_peaks_path, tracks.waveform_peaks_path),
              duration_ms = COALESCE(excluded.duration_ms, tracks.duration_ms),
              match_fingerprint = COALESCE(excluded.match_fingerprint, tracks.match_fingerprint),
              updated_at = excluded.updated_at
            "#,
            params![
                id,
                track.title,
                track.artist,
                track.album,
                track_number,
                bpm,
                key,
                file_path,
                file_size_bytes,
                track.artwork_path,
                track.waveform_peaks_path,
                track.duration_ms,
                fingerprint,
                now_ts
            ],
        )?;

        track.local_track_id = Some(id);
        Ok(true)
    }

    pub fn inspect_usb_track(
        &self,
        req: InspectUsbTrackRequest,
    ) -> BackendResult<InspectUsbTrackData> {
        let track_id = req.track_id.trim().parse::<u32>().map_err(|_| {
            BackendError::Validation("trackId must be a numeric USB track id".to_string())
        })?;
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut warnings = Vec::<String>::new();

        let pdb_path = vendor_pdb_path(&usb_root);
        let file_hint = req
            .file_path
            .as_deref()
            .map(normalize_text)
            .unwrap_or_default();
        let title_hint = req.title.as_deref().map(normalize_text).unwrap_or_default();
        let artist_hint = req
            .artist
            .as_deref()
            .map(normalize_text)
            .unwrap_or_default();

        if pdb_path.exists() {
            let parsed = parse_pdb(&pdb_path)?;
            warnings.extend(parsed.warnings);
            let mut best: Option<(&crate::pdb_reader::PdbTrackRow, i32)> = None;
            for t in parsed.tracks.iter().filter(|t| t.id == track_id) {
                let artist = parsed
                    .artists
                    .get(&t.artist_id)
                    .cloned()
                    .unwrap_or_else(|| "Unknown Artist".to_string());
                let resolved_file_path = resolve_usb_side_path(&usb_root, &t.track_file_path)
                    .unwrap_or_else(|| t.track_file_path.clone());
                let mut score = 0i32;
                if !file_hint.is_empty() {
                    let candidate = normalize_text(&resolved_file_path);
                    if candidate.contains(&file_hint) || file_hint.contains(&candidate) {
                        score += 8;
                    }
                }
                if !title_hint.is_empty() {
                    let candidate = normalize_text(&t.title);
                    if candidate.contains(&title_hint) || title_hint.contains(&candidate) {
                        score += 4;
                    }
                }
                if !artist_hint.is_empty() {
                    let candidate = normalize_text(&artist);
                    if candidate.contains(&artist_hint) || artist_hint.contains(&candidate) {
                        score += 3;
                    }
                }
                match best {
                    Some((_, best_score)) if best_score >= score => {}
                    _ => best = Some((t, score)),
                }
            }
            if let Some((t, score)) = best {
                let has_hints =
                    !file_hint.is_empty() || !title_hint.is_empty() || !artist_hint.is_empty();
                if has_hints && score <= 0 {
                    // Keep searching via DB fallback when ID collides and hints don't match PDB row.
                } else {
                    let artist = parsed
                        .artists
                        .get(&t.artist_id)
                        .cloned()
                        .unwrap_or_else(|| "Unknown Artist".to_string());
                    let album = parsed.albums.get(&t.album_id).cloned();
                    let key = parsed.keys.get(&t.key_id).cloned();
                    let artwork_path = parsed
                        .artworks
                        .get(&t.artwork_id)
                        .and_then(|p| resolve_usb_side_path(&usb_root, p));
                    let resolved_file_path = resolve_usb_side_path(&usb_root, &t.track_file_path)
                        .unwrap_or_else(|| t.track_file_path.clone());
                    let usb_analysis_path = resolve_usb_side_path(&usb_root, &t.anlz_path);
                    let waveform_preview = usb_analysis_path
                        .as_deref()
                        .and_then(load_waveform_preview_from_analysis_path);
                    return Ok(InspectUsbTrackData {
                        source: "pdb".to_string(),
                        track: UsbTrack {
                            id: track_id.to_string(),
                            local_track_id: None,
                            title: if t.title.is_empty() {
                                "Unknown Title".to_string()
                            } else {
                                t.title.clone()
                            },
                            artist,
                            album,
                            track_number: (t.track_number > 0).then_some(t.track_number),
                            bpm: if t.tempo_x100 > 0 {
                                Some(t.tempo_x100 as f64 / 100.0)
                            } else {
                                None
                            },
                            key,
                            file_path: resolved_file_path,
                            usb_media_path: Some(t.track_file_path.clone()),
                            artwork_data_url: artwork_path
                                .as_deref()
                                .and_then(artwork_path_to_data_url),
                            artwork_path,
                            waveform_peaks_path: usb_analysis_path.clone(),
                            usb_analysis_path,
                            usb_analysis_path_raw: Some(t.anlz_path.clone()),
                            waveform_preview,
                            duration_ms: t
                                .duration_seconds
                                .map(|seconds| u64::from(seconds) * 1000),
                        },
                        warnings,
                    });
                }
            }
        } else {
            warnings.push(format!(
                "PDB not found under {}; using DB fallback only",
                usb_root.display()
            ));
        }

        if let Some(index) = try_read_track_index_from_edb(&usb_root, &mut warnings)
            && let Some(mut track) = index.get(&track_id).cloned() {
                let mut file_hint_match = true;
                if !file_hint.is_empty() {
                    let candidate = normalize_text(&track.file_path);
                    file_hint_match =
                        candidate.contains(&file_hint) || file_hint.contains(&candidate);
                }
                if file_hint_match {
                    if track.waveform_preview.is_none() {
                        track.waveform_preview = track
                            .usb_analysis_path
                            .as_deref()
                            .and_then(load_waveform_preview_from_analysis_path);
                    }
                    return Ok(InspectUsbTrackData {
                        source: "eDB".to_string(),
                        track,
                        warnings,
                    });
                }
            }

        Err(BackendError::Validation(format!(
            "trackId {track_id} not found on USB metadata sources"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_history_dates_from_track_date_created, build_history_track_date_index,
        normalize_date_created, select_history_rows,
    };
    use crate::models::{UsbHistory, UsbTrack};
    use crate::pdb_reader::{PdbHistoryEntryRow, PdbHistoryPlaylistRow, PdbTrackRow};
    use std::collections::HashMap;

    fn make_track(id: &str, file_path: &str) -> UsbTrack {
        UsbTrack {
            id: id.to_string(),
            local_track_id: None,
            title: "T".to_string(),
            artist: "A".to_string(),
            album: None,
            track_number: None,
            bpm: None,
            key: None,
            file_path: file_path.to_string(),
            usb_media_path: None,
            artwork_path: None,
            artwork_data_url: None,
            waveform_peaks_path: None,
            usb_analysis_path: None,
            usb_analysis_path_raw: None,
            waveform_preview: None,
            duration_ms: None,
        }
    }

    fn make_pdb_track(id: u32, date_added: Option<&str>) -> PdbTrackRow {
        PdbTrackRow {
            content_link: None,
            sample_rate_hz: None,
            file_size_bytes: None,
            master_content_id: None,
            master_db_id: None,
            id,
            artist_id: 0,
            album_id: 0,
            artwork_id: 0,
            key_id: 0,
            genre_id: 0,
            bitrate_kbps: None,
            track_number: 0,
            tempo_x100: 0,
            release_year: None,
            bit_depth: None,
            duration_seconds: None,
            file_type: None,
            isrc: None,
            date_added: date_added.map(str::to_string),
            release_date: None,
            dj_comment: None,
            file_name: None,
            publish_track_info: None,
            autoload_hotcues: None,
            title: String::new(),
            anlz_path: String::new(),
            track_file_path: String::new(),
        }
    }

    #[test]
    fn normalize_date_created_accepts_iso_datetime_prefix() {
        assert_eq!(
            normalize_date_created("2024-10-15T09:10:11Z")
                .map(|d| d.format("%Y-%m-%d").to_string()),
            Some("2024-10-15".to_string())
        );
    }

    #[test]
    fn build_history_track_date_index_uses_pdb_track_ids() {
        let index = build_history_track_date_index(&[
            make_pdb_track(1674, Some("2020-09-18")),
            make_pdb_track(917, Some("2025-12-19")),
            make_pdb_track(42, Some("")),
            make_pdb_track(43, None),
        ]);

        assert_eq!(index.get(&1674).map(String::as_str), Some("2020-09-18"));
        assert_eq!(index.get(&917).map(String::as_str), Some("2025-12-19"));
        assert!(!index.contains_key(&42));
        assert!(!index.contains_key(&43));
    }

    #[test]
    fn apply_history_dates_carries_latest_track_date_forward_in_playlist_order() {
        let mut histories = vec![
            UsbHistory {
                id: "usb-h-1".to_string(),
                name: "HISTORY 001".to_string(),
                created_at: Some("2021-09".to_string()),
                tracks: vec![
                    make_track("10", "/USB/Contents/A/1.mp3"),
                    make_track("20", "/USB/Contents/A/2.mp3"),
                ],
            },
            UsbHistory {
                id: "usb-h-2".to_string(),
                name: "HISTORY 002".to_string(),
                created_at: Some("2021-09".to_string()),
                tracks: vec![make_track("30", "/USB/Contents/A/3.mp3")],
            },
            UsbHistory {
                id: "usb-h-3".to_string(),
                name: "HISTORY 003".to_string(),
                created_at: Some("2021-09".to_string()),
                tracks: vec![make_track("40", "/USB/Contents/A/4.mp3")],
            },
        ];
        let date_created_by_track_id = HashMap::from([
            (10u32, "2024-10-15".to_string()),
            (20u32, "2024-10-14".to_string()),
            (30u32, "2024-10-10".to_string()),
            (40u32, "2024-10-20".to_string()),
        ]);

        apply_history_dates_from_track_date_created(&mut histories, &date_created_by_track_id);

        assert_eq!(histories[0].created_at.as_deref(), Some("2024-10-15"));
        assert_eq!(histories[1].created_at.as_deref(), Some("2024-10-15"));
        assert_eq!(histories[2].created_at.as_deref(), Some("2024-10-20"));
    }

    #[test]
    fn apply_history_dates_from_track_date_created_preserves_existing_valid_date() {
        let mut histories = vec![UsbHistory {
            id: "usb-h-1".to_string(),
            name: "HISTORY 001".to_string(),
            created_at: Some("2026-04-03".to_string()),
            tracks: vec![make_track("10", "/USB/Contents/A/1.mp3")],
        }];
        let date_created_by_track_id = HashMap::from([(10u32, "2024-10-15".to_string())]);

        apply_history_dates_from_track_date_created(&mut histories, &date_created_by_track_id);

        assert_eq!(histories[0].created_at.as_deref(), Some("2026-04-03"));
    }

    #[test]
    fn apply_history_dates_from_track_date_created_carries_existing_valid_date_forward() {
        let mut histories = vec![
            UsbHistory {
                id: "usb-h-1".to_string(),
                name: "HISTORY 001".to_string(),
                created_at: Some("2026-04-03".to_string()),
                tracks: vec![make_track("10", "/USB/Contents/A/1.mp3")],
            },
            UsbHistory {
                id: "usb-h-2".to_string(),
                name: "HISTORY 002".to_string(),
                created_at: Some("2021-09".to_string()),
                tracks: vec![make_track("20", "/USB/Contents/A/2.mp3")],
            },
        ];
        let date_created_by_track_id = HashMap::from([(20u32, "2024-10-15".to_string())]);

        apply_history_dates_from_track_date_created(&mut histories, &date_created_by_track_id);

        assert_eq!(histories[0].created_at.as_deref(), Some("2026-04-03"));
        assert_eq!(histories[1].created_at.as_deref(), Some("2026-04-03"));
    }

    #[test]
    fn select_history_rows_prefers_more_populated_t11_t12_family() {
        let playlists = vec![
            PdbHistoryPlaylistRow {
                id: 1,
                name: "HISTORY 001".to_string(),
                source_table: 17,
            },
            PdbHistoryPlaylistRow {
                id: 2,
                name: "HISTORY 002".to_string(),
                source_table: 11,
            },
            PdbHistoryPlaylistRow {
                id: 3,
                name: "".to_string(),
                source_table: 11,
            },
        ];
        let entries = vec![
            PdbHistoryEntryRow {
                track_id: Some(101),
                playlist_id: 1,
                entry_index: 1,
                source_table: 18,
            },
            PdbHistoryEntryRow {
                track_id: Some(201),
                playlist_id: 2,
                entry_index: 1,
                source_table: 12,
            },
            PdbHistoryEntryRow {
                track_id: Some(202),
                playlist_id: 3,
                entry_index: 2,
                source_table: 12,
            },
        ];

        let (history_playlists, history_entries) = select_history_rows(&playlists, &entries);
        assert_eq!(history_playlists.len(), 2);
        assert!(history_playlists.iter().all(|row| row.source_table == 11));
        assert_eq!(history_entries.len(), 2);
        assert!(history_entries.iter().all(|row| row.source_table == 12));
    }

    #[test]
    fn select_history_rows_falls_back_to_t17_t18_when_t11_t12_absent() {
        let playlists = vec![PdbHistoryPlaylistRow {
            id: 7,
            name: "HISTORY 007".to_string(),
            source_table: 17,
        }];
        let entries = vec![
            PdbHistoryEntryRow {
                track_id: Some(101),
                playlist_id: 7,
                entry_index: 1,
                source_table: 18,
            },
            PdbHistoryEntryRow {
                track_id: Some(102),
                playlist_id: 7,
                entry_index: 2,
                source_table: 18,
            },
        ];

        let (history_playlists, history_entries) = select_history_rows(&playlists, &entries);
        assert_eq!(history_playlists.len(), 1);
        assert_eq!(history_playlists[0].source_table, 17);
        assert_eq!(history_entries.len(), 2);
        assert!(history_entries.iter().all(|row| row.source_table == 18));
    }

    #[test]
    fn select_history_rows_prefers_runtime_t11_t12_even_when_t17_t18_have_seed_volume() {
        let mut playlists = Vec::new();
        for id in 1..=27u32 {
            playlists.push(PdbHistoryPlaylistRow {
                id,
                name: format!("HISTORY {id:03}"),
                source_table: 17,
            });
        }
        playlists.push(PdbHistoryPlaylistRow {
            id: 100,
            name: "HISTORY 100".to_string(),
            source_table: 11,
        });

        let mut entries = Vec::new();
        for id in 1..=27u32 {
            entries.push(PdbHistoryEntryRow {
                track_id: Some(1000 + id),
                playlist_id: id,
                entry_index: 1,
                source_table: 18,
            });
        }
        entries.push(PdbHistoryEntryRow {
            track_id: Some(2000),
            playlist_id: 100,
            entry_index: 1,
            source_table: 12,
        });

        let (history_playlists, history_entries) = select_history_rows(&playlists, &entries);
        assert_eq!(history_playlists.len(), 1);
        assert_eq!(history_playlists[0].source_table, 11);
        assert_eq!(history_entries.len(), 1);
        assert_eq!(history_entries[0].source_table, 12);
    }

    #[test]
    fn select_history_rows_uses_t11_t12_when_t17_t18_are_empty() {
        let playlists = vec![PdbHistoryPlaylistRow {
            id: 7,
            name: "HISTORY 007".to_string(),
            source_table: 11,
        }];
        let entries = vec![PdbHistoryEntryRow {
            track_id: Some(101),
            playlist_id: 7,
            entry_index: 1,
            source_table: 12,
        }];

        let (history_playlists, history_entries) = select_history_rows(&playlists, &entries);
        assert_eq!(history_playlists.len(), 1);
        assert_eq!(history_playlists[0].source_table, 11);
        assert_eq!(history_entries.len(), 1);
        assert_eq!(history_entries[0].source_table, 12);
    }
}
