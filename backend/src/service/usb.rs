//! USB validation, playlist/history fetching, track inspection.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::edb::{
    open_edb_from_usb_root, table_exists, try_read_content_date_created_index_from_edb_with_conn,
    try_read_playlists_with_metadata_from_edb_with_conn, try_read_track_index_from_edb,
    try_read_track_index_from_edb_with_conn,
};
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{
    FetchUsbHistoriesData, FetchUsbHistoriesRequest, FetchUsbPlaylistsData,
    FetchUsbPlaylistsRequest, InspectUsbTrackData, InspectUsbTrackRequest, InspectUsbTrackResult,
    InspectUsbTracksData, InspectUsbTracksRequest, RemoveUsbPlaylistData, RemoveUsbPlaylistRequest,
    ReorderUsbPlaylistsData, ReorderUsbPlaylistsRequest, UsbHistory, UsbHistoryCounts,
    UsbImportStats, UsbPlaylist, UsbTrack, ValidateUsbRootData, ValidateUsbRootRequest,
    WarningEntry,
};
use crate::pdb_reader::{ParsedPdb, PdbHistoryEntryRow, PdbHistoryPlaylistRow, parse_pdb};

use super::analysis::normalize_text;
use super::export_helpers::{
    analysis_bundle_path_variants, prune_stale_export_owned_files,
    remove_playlist_and_tracks_from_pdb, remove_playlist_from_edb,
};
use super::usb_helpers::{
    PlaylistCandidate, build_usb_track_id_index, decode_history_playlist_id,
    decode_history_track_id, dedupe_usb_playlists_by_name, history_entry_sort_key,
    is_named_history_playlist, lookup_playlist_tracks, merge_playlist_tracks, normalize_packed_id,
    parse_history_name_numeric_id, parse_history_slot_id, sanitize_history_name, sanitize_text,
};
use super::usb_utils::{
    self, artwork_path_to_data_url, canonicalize_or_self, canonicalize_playlist_name,
    has_write_access, load_waveform_preview_from_analysis_path, normalize_usb_root_path,
    parse_history_numeric_id, resolve_usb_root, resolve_usb_side_path,
};
use super::usb_vendor_compat::{
    USB_CONTENTS_DIR, USB_VENDOR_ROOT_DIR, vendor_edb_path, vendor_pdb_path,
};
use super::{
    BackendService, FINGERPRINT_MATCH_DURATION_TOLERANCE_MS, browse_path_matches_root,
    build_track_match_fingerprint, export_log, find_confident_fingerprint_match, now,
    untainted_usb_root_paths,
};

const SLOW_USB_STAGE_MS: u128 = 8_000;

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
                    file_size_bytes: t.file_size_bytes.map(i64::from),
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
    conn: &rusqlite::Connection,
    usb_root: &std::path::Path,
    track_by_id: &mut HashMap<u32, UsbTrack>,
    warnings: &mut Vec<WarningEntry>,
) {
    if let Some(all_edb_tracks) = try_read_track_index_from_edb_with_conn(conn, usb_root, warnings)
    {
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

/// Drop blank/template history playlist rows. Fresh or never-played exports
/// ship a fixed block of these alongside t17/t18 (empty name, no track
/// link) — real rekordbox-recorded sessions are always named "HISTORY NNN".
fn filter_named_history_playlists(
    playlists: Vec<PdbHistoryPlaylistRow>,
) -> Vec<PdbHistoryPlaylistRow> {
    playlists
        .into_iter()
        .filter(|row| is_named_history_playlist(&row.name))
        .collect()
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
    warnings: &mut Vec<WarningEntry>,
    stage: &str,
    started: &mut std::time::Instant,
) {
    push_usb_stage_timing_with_threshold(warnings, stage, started, SLOW_USB_STAGE_MS);
}

/// Extra allowance for eDB-heavy stages whose wall-clock cost scales with
/// library size (SQL joins over `content`/`playlist`/etc. that grow with
/// track count), not media speed -- see `push_usb_stage_timing_with_threshold`
/// for why a flat threshold alone misdiagnoses "slow media" on large-but-fast
/// libraries.
const SLOW_USB_STAGE_PER_TRACK_MS: u128 = 1;

fn slow_stage_threshold_ms(item_count: usize) -> u128 {
    SLOW_USB_STAGE_MS + (item_count as u128) * SLOW_USB_STAGE_PER_TRACK_MS
}

/// Same as `push_usb_stage_timing`, but with an explicit threshold instead
/// of the flat `SLOW_USB_STAGE_MS` default. Stages whose cost is genuinely
/// proportional to library size (not media speed) should pass a threshold
/// computed via `slow_stage_threshold_ms` instead of assuming every stage
/// costs the same regardless of how much data it reads.
fn push_usb_stage_timing_with_threshold(
    warnings: &mut Vec<WarningEntry>,
    stage: &str,
    started: &mut std::time::Instant,
    threshold_ms: u128,
) {
    let elapsed = started.elapsed().as_millis();
    warnings.push(logging::log(
        Level::Info,
        "usb-import",
        "usb.import.stage-timing",
        format!("stage timing: {stage}: {elapsed}ms"),
    ));
    if elapsed >= threshold_ms {
        warnings.push(logging::log(
            Level::Warn,
            "usb-import",
            "usb.import.slow-media",
            format!("slow-media suspected: stage '{stage}' took {elapsed}ms"),
        ));
    }
    *started = std::time::Instant::now();
}

impl BackendService {
    pub fn validate_usb_root(
        &self,
        req: ValidateUsbRootRequest,
    ) -> BackendResult<ValidateUsbRootData> {
        let mut warnings = Vec::<WarningEntry>::new();
        let trimmed = req.path.trim();
        if trimmed.is_empty() {
            warnings.push(logging::log(
                Level::Warn,
                "usb-import",
                "usb.import.path-empty",
                "USB path is empty",
            ));
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
            warnings.push(logging::log(
                Level::Warn,
                "usb-import",
                "usb.import.path-not-found",
                format!("Path does not exist: {}", candidate.display()),
            ));
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
        let mount_conn = self.db.connect()?;
        usb_utils::upsert_usb_device(&mount_conn, &normalized, true, &now())?;
        drop(mount_conn);
        let has_vendor_root = normalized.join(USB_VENDOR_ROOT_DIR).is_dir();
        let has_contents = normalized.join(USB_CONTENTS_DIR).is_dir();
        let has_pdb = vendor_pdb_path(&normalized).is_file();
        let has_edb = vendor_edb_path(&normalized).is_file();

        if !has_vendor_root {
            warnings.push(logging::log(
                Level::Warn,
                "usb-import",
                "usb.import.missing-vendor-root",
                "Missing vendor root directory",
            ));
        }
        if !has_contents {
            warnings.push(logging::log(
                Level::Warn,
                "usb-import",
                "usb.import.missing-contents",
                "Missing contents directory",
            ));
        }
        if !has_pdb {
            warnings.push(logging::log(
                Level::Warn,
                "usb-import",
                "usb.import.missing-pdb",
                "Missing PDB in vendor db directory",
            ));
        }

        let has_write_access = has_write_access(&normalized);
        if !has_write_access {
            warnings.push(logging::log(
                Level::Warn,
                "usb-import",
                "usb.import.read-only",
                "USB appears read-only: imports may work but export will fail",
            ));
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

    pub fn list_usb_devices(&self) -> BackendResult<crate::models::ListUsbDevicesData> {
        let conn = self.db.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, root_path, label, mounted, first_seen_at, last_seen_at
             FROM usb_devices
             WHERE deleted_at IS NULL
             ORDER BY last_seen_at DESC",
        )?;
        let items = stmt
            .query_map([], |row| {
                Ok(crate::models::UsbDeviceSummary {
                    id: row.get(0)?,
                    root_path: row.get(1)?,
                    label: row.get(2)?,
                    mounted: row.get::<_, i64>(3)? != 0,
                    first_seen_at: row.get(4)?,
                    last_seen_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(crate::models::ListUsbDevicesData { items })
    }

    pub fn prune_usb_device(
        &self,
        req: crate::models::PruneUsbDeviceRequest,
    ) -> BackendResult<crate::models::PruneUsbDeviceData> {
        let conn = self.db.connect()?;
        let now_ts = now();
        let changed = conn.execute(
            "UPDATE usb_devices SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            params![now_ts, req.id],
        )?;
        Ok(crate::models::PruneUsbDeviceData {
            pruned: changed > 0,
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
        let mut warnings = Vec::<WarningEntry>::new();
        let mut stage_started = std::time::Instant::now();
        on_progress(2, 100, "USB: Resolving root");
        on_progress(10, 100, "USB: Parsing PDB");
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        push_usb_stage_timing(&mut warnings, "resolve usb root", &mut stage_started);
        let pdb_path = vendor_pdb_path(&usb_root);
        let parsed = if pdb_path.exists() {
            let parsed = parse_pdb(&pdb_path)?;
            warnings.extend(parsed.warnings.iter().map(|message| {
                logging::log(
                    Level::Warn,
                    "usb-import",
                    "usb.import.pdb-parse",
                    message.clone(),
                )
            }));
            Some(parsed)
        } else {
            warnings.push(logging::log(
                Level::Warn,
                "usb-import",
                "usb.import.pdb-not-found",
                format!(
                    "PDB not found under {}; continuing with eDB-only mode",
                    usb_root.display()
                ),
            ));
            None
        };
        push_usb_stage_timing(&mut warnings, "parse PDB", &mut stage_started);

        on_progress(30, 100, "USB: Reading eDB");
        // Open the eDB once and reuse it for every read below -- opening it
        // per-call was redundantly re-running SQLCipher key negotiation
        // (see try_read_track_index_from_edb_with_conn's doc comment).
        let edb_conn = open_edb_from_usb_root(&usb_root, &mut warnings);
        let edb_playlists = edb_conn.as_ref().and_then(|conn| {
            try_read_playlists_with_metadata_from_edb_with_conn(conn, &usb_root, &mut warnings)
        });
        let edb_playlist_tracks = edb_playlists.as_ref().map(|m| {
            m.iter()
                .map(|(name, playlist)| (name.clone(), playlist.tracks.clone()))
                .collect::<HashMap<_, _>>()
        });
        let mut edb_track_by_id =
            edb_track_index_from_playlist_tracks(edb_playlist_tracks.as_ref());
        if let Some(conn) = edb_conn.as_ref() {
            merge_full_edb_track_index(conn, &usb_root, &mut edb_track_by_id, &mut warnings);
        }
        let eedb_playlist_tracks_canonical = edb_playlist_tracks.as_ref().map(|m| {
            m.iter()
                .map(|(name, tracks)| (canonicalize_playlist_name(name), tracks.clone()))
                .collect::<HashMap<_, _>>()
        });
        push_usb_stage_timing_with_threshold(
            &mut warnings,
            "read eDB",
            &mut stage_started,
            slow_stage_threshold_ms(edb_track_by_id.len()),
        );

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
            warnings.push(logging::log(
                Level::Info,
                "usb-import",
                "usb.playlists.collapsed-duplicates",
                format!("collapsed {collapsed} duplicate playlist name(s) from USB sources"),
            ));
        }

        on_progress(90, 100, "USB: Finalizing playlist import");
        let stats = UsbImportStats {
            indexed_tracks: parsed.as_ref().map(|p| p.tracks.len()).unwrap_or(0),
            playlist_referenced_tracks: referenced_track_ids.len(),
            playlist_entries: playlist_entries_total,
        };
        let materialized_tracks = self.materialize_usb_playlist_tracks(&mut items, &usb_root)?;
        push_usb_stage_timing(
            &mut warnings,
            "finalize playlist import",
            &mut stage_started,
        );

        warnings.insert(
            0,
            logging::log(
                Level::Info,
                "usb-import",
                "usb.import.root-in-use",
                format!("USB root in use: {}", usb_root.display()),
            ),
        );

        if !source_counts.is_empty() {
            let pdb_count = source_counts.get("pdb").copied().unwrap_or(0);
            let edb_count = source_counts.get("eDB").copied().unwrap_or(0);
            if edb_count > 0 {
                warnings.push(logging::log(
                    Level::Info,
                    "usb-import",
                    "usb.playlists.source-edb",
                    format!("used eDB as playlist source for {edb_count} playlist(s)"),
                ));
            }
            if pdb_count > 0 {
                warnings.push(logging::log(
                    Level::Info,
                    "usb-import",
                    "usb.playlists.source-pdb",
                    format!("used PDB as playlist source for {pdb_count} playlist(s)"),
                ));
            }
        }
        if !empty_source_playlists.is_empty() {
            warnings.push(logging::log(
                Level::Warn,
                "usb-import",
                "usb.playlists.empty-source",
                format!(
                    "{} playlist(s) had zero static track entries in export data: {}",
                    empty_source_playlists.len(),
                    empty_source_playlists.join(", ")
                ),
            ));
        }
        if materialized_tracks > 0 {
            warnings.push(logging::log(
                Level::Info,
                "usb-import",
                "usb.playlists.materialized",
                format!("materialized {materialized_tracks} USB track row(s) into local library"),
            ));
        }

        Ok(FetchUsbPlaylistsData {
            items,
            stats,
            warnings,
        })
    }

    pub fn fetch_usb_histories(
        &self,
        req: FetchUsbHistoriesRequest,
    ) -> BackendResult<FetchUsbHistoriesData> {
        self.fetch_usb_histories_with_progress(req, |_, _, _| {})
    }

    pub fn reorder_usb_playlists(
        &self,
        req: ReorderUsbPlaylistsRequest,
    ) -> BackendResult<ReorderUsbPlaylistsData> {
        self.reorder_usb_playlists_with_progress(req, |_, _, _| {})
    }

    pub fn reorder_usb_playlists_with_progress<F>(
        &self,
        req: ReorderUsbPlaylistsRequest,
        mut on_progress: F,
    ) -> BackendResult<ReorderUsbPlaylistsData>
    where
        F: FnMut(usize, usize, &str),
    {
        let mut warnings = Vec::<WarningEntry>::new();
        on_progress(0, 100, "USB: Resolving USB root");
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;

        on_progress(10, 100, "USB: Computing new playlist order");
        let mut desired_sort_by_id = HashMap::<u32, u32>::new();
        let mut next_sort_order = 0u32;
        for id in &req.ordered_playlist_ids {
            let Some(pdb_id_str) = id.strip_prefix("usb-pl-") else {
                continue;
            };
            // eDB-only playlists use "usb-pl-name-{canonical}" ids and have no
            // PDB row to patch, so they fail to parse as a bare u32 and are skipped.
            let Ok(pdb_id) = pdb_id_str.parse::<u32>() else {
                continue;
            };
            desired_sort_by_id.insert(pdb_id, next_sort_order);
            next_sort_order += 1;
        }

        if desired_sort_by_id.is_empty() {
            return Err(BackendError::Validation(
                "reorder_usb_playlists: no PDB-backed playlist ids in orderedPlaylistIds"
                    .to_string(),
            ));
        }

        on_progress(30, 100, "USB: Patching PDB playlist order");
        let patched = crate::service::repair::restore_pdb_playlist_sort_orders(
            &usb_root,
            &desired_sort_by_id,
        )?;

        on_progress(70, 100, "USB: Syncing eDB playlist order");
        let edb_updated = crate::service::repair::sync_edb_playlist_sort_orders_from_pdb(
            &usb_root,
            &mut warnings,
        )?;
        if edb_updated == 0 {
            warnings.push(logging::log(
                Level::Warn,
                "usb-import",
                "usb.reorder.zero-rows-synced",
                "reorder: eDB playlist order sync updated 0 rows",
            ));
        }

        on_progress(100, 100, "USB: Playlist order saved");

        Ok(ReorderUsbPlaylistsData {
            reordered: patched,
            warnings,
        })
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
        let mut warnings = Vec::<WarningEntry>::new();
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
            warnings.push(logging::log(
                Level::Info,
                "usb-import",
                "usb.remove.shared-tracks-preserved",
                format!("{tracks_kept_shared} shared tracks preserved (used by other playlists)"),
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
                && let Ok(parsed) = parse_pdb(&pdb_path)
            {
                for art_id in &exclusive_artwork_ids {
                    if let Some(art_path) = parsed.artworks.get(art_id) {
                        // Small artwork
                        stale_paths.push(art_path.clone());
                        // Medium variant: replace .jpg with _m.jpg
                        let medium = art_path.replace(".jpg", "_m.jpg").replace(".png", "_m.png");
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
                warnings.push(logging::log(
                    Level::Warn,
                    "usb-import",
                    "usb.remove.file-missing",
                    format!("{} files already missing from USB", prune_result.missing),
                ));
            }
            for path in &stale_paths {
                warnings.push(logging::log(
                    Level::Info,
                    "usb-import",
                    "usb.remove.file-deleted",
                    format!("deleted: {path}"),
                ));
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
            warnings,
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
        let mut stage_warnings = Vec::<WarningEntry>::new();
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
                    logging::log(
                        Level::Info,
                        "usb-import",
                        "usb.import.root-in-use",
                        format!("USB root in use: {}", usb_root.display()),
                    ),
                    logging::log(
                        Level::Warn,
                        "usb-import",
                        "usb.histories.pdb-not-found",
                        format!(
                            "PDB not found under {}; history import requires PDB",
                            usb_root.display()
                        ),
                    ),
                ],
            });
        }

        let parsed = parse_pdb(&pdb_path)?;
        push_usb_stage_timing(&mut stage_warnings, "parse PDB", &mut stage_started);
        on_progress(30, 100, "USB: Reading supplemental databases");
        let mut supplemental_warnings = Vec::<WarningEntry>::new();
        // Open the eDB once and reuse it for every read below -- opening it
        // per-call was redundantly re-running SQLCipher key negotiation up
        // to 4x in this one stage (see try_read_track_index_from_edb_with_conn's
        // doc comment), which could dominate wall time on a large eDB even
        // on fast media and misdiagnose as "slow media suspected".
        let supplemental_edb_conn = open_edb_from_usb_root(&usb_root, &mut supplemental_warnings);
        let supplemental_edb_playlist_tracks = supplemental_edb_conn
            .as_ref()
            .and_then(|conn| {
                try_read_playlists_with_metadata_from_edb_with_conn(
                    conn,
                    &usb_root,
                    &mut supplemental_warnings,
                )
            })
            .map(|m| {
                m.into_iter()
                    .map(|(name, playlist)| (name, playlist.tracks))
                    .collect::<HashMap<_, _>>()
            });
        let mut supplemental_track_by_id =
            edb_track_index_from_playlist_tracks(supplemental_edb_playlist_tracks.as_ref());
        if let Some(conn) = supplemental_edb_conn.as_ref() {
            merge_full_edb_track_index(
                conn,
                &usb_root,
                &mut supplemental_track_by_id,
                &mut supplemental_warnings,
            );
        }
        let mut date_created_by_track_id = build_history_track_date_index(&parsed.tracks);
        if date_created_by_track_id.is_empty() {
            date_created_by_track_id = supplemental_edb_conn
                .as_ref()
                .and_then(|conn| {
                    try_read_content_date_created_index_from_edb_with_conn(
                        conn,
                        &mut supplemental_warnings,
                    )
                })
                .unwrap_or_default();
        }
        let export_log = match export_log::load_export_log(&usb_root) {
            Ok(log) => log,
            Err(err) => {
                supplemental_warnings.push(logging::log(
                    Level::Warn,
                    "usb-import",
                    "usb.histories.export-log-ignored",
                    format!("USB export log ignored: {err}"),
                ));
                None
            }
        };
        let (edb_history_rows, edb_history_content_rows) =
            if let Some(conn) = supplemental_edb_conn.as_ref() {
                let history_rows = if table_exists(conn, "history") {
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
                let history_content_rows = if table_exists(conn, "history_content") {
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
        push_usb_stage_timing_with_threshold(
            &mut stage_warnings,
            "read supplemental databases",
            &mut stage_started,
            slow_stage_threshold_ms(supplemental_track_by_id.len()),
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
        let history_playlists = filter_named_history_playlists(history_playlists);

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
                                        file_size_bytes: None,
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
        let mut warnings: Vec<WarningEntry> = parsed
            .warnings
            .iter()
            .map(|message| {
                logging::log(
                    Level::Warn,
                    "usb-import",
                    "usb.import.pdb-parse",
                    message.clone(),
                )
            })
            .collect();
        warnings.push(logging::log(
            Level::Info,
            "usb-import",
            "usb.histories.selected-table-family",
            format!(
                "history import: selected populated history table family from t11/t12 vs t17/t18: playlists={}, entries={}",
                history_playlists.len(),
                selected_history_entries.len()
            ),
        ));
        warnings.insert(
            0,
            logging::log(
                Level::Info,
                "usb-import",
                "usb.import.root-in-use",
                format!("USB root in use: {}", usb_root.display()),
            ),
        );
        warnings.extend(supplemental_warnings);
        warnings.extend(stage_warnings);
        let materialized_tracks = self.materialize_usb_history_tracks(&mut items, &usb_root)?;
        push_usb_stage_timing(&mut warnings, "finalize history import", &mut stage_started);
        if materialized_tracks > 0 {
            warnings.push(logging::log(
                Level::Info,
                "usb-import",
                "usb.histories.materialized",
                format!(
                    "materialized {materialized_tracks} USB history track row(s) into local library"
                ),
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
            warnings,
        })
    }

    fn materialize_usb_playlist_tracks(
        &self,
        playlists: &mut [UsbPlaylist],
        usb_root: &std::path::Path,
    ) -> BackendResult<usize> {
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        let now_ts = now();
        let usb_device_id = usb_utils::upsert_usb_device(&tx, usb_root, false, &now_ts)?;
        let usb_root_paths = untainted_usb_root_paths(&tx)?;
        let mut materialized = 0usize;

        for playlist in playlists {
            for track in &mut playlist.tracks {
                if self.materialize_usb_track_row(
                    &tx,
                    track,
                    &now_ts,
                    &usb_device_id,
                    &usb_root_paths,
                )? {
                    materialized += 1;
                }
            }
        }

        tx.commit()?;
        Ok(materialized)
    }

    fn materialize_usb_history_tracks(
        &self,
        histories: &mut [UsbHistory],
        usb_root: &std::path::Path,
    ) -> BackendResult<usize> {
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        let now_ts = now();
        let usb_device_id = usb_utils::upsert_usb_device(&tx, usb_root, false, &now_ts)?;
        let usb_root_paths = untainted_usb_root_paths(&tx)?;
        let mut materialized = 0usize;

        for history in histories {
            for track in &mut history.tracks {
                if self.materialize_usb_track_row(
                    &tx,
                    track,
                    &now_ts,
                    &usb_device_id,
                    &usb_root_paths,
                )? {
                    materialized += 1;
                }
            }
        }

        tx.commit()?;
        Ok(materialized)
    }

    /// Matches an incoming USB-sourced track against existing `tracks` rows
    /// before ever creating a placeholder row, so a USB copy of a song the
    /// user already has locally links to that genuine row instead of
    /// spawning a second, disconnected one (which also structurally
    /// prevents the genuine row's `waveform_peaks_path` from ever being
    /// clobbered by USB-sourced data, since a matched row is never written
    /// to at all). See the "Confidence gate" note below for why a fingerprint
    /// match alone isn't sufficient to merge.
    fn materialize_usb_track_row(
        &self,
        tx: &rusqlite::Transaction<'_>,
        track: &mut UsbTrack,
        now_ts: &str,
        usb_device_id: &str,
        usb_root_paths: &[String],
    ) -> BackendResult<bool> {
        let file_path = track.file_path.trim();
        if file_path.is_empty() {
            return Ok(false);
        }

        let fingerprint =
            build_track_match_fingerprint(&track.title, &track.artist, track.album.as_deref());

        let matched_id = find_confident_fingerprint_match(
            tx,
            &fingerprint,
            track.duration_ms.map(|v| v as i64),
            track.file_size_bytes,
            usb_root_paths,
        )?;

        let id = if let Some(id) = matched_id {
            // Genuine local row: don't touch title/artist/album/file_path/
            // waveform_peaks_path -- just record that this device also has
            // a copy of it.
            id
        } else {
            let existing_id = tx
                .query_row(
                    "SELECT id FROM tracks WHERE file_path = ?1 LIMIT 1",
                    params![file_path],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;

            let id = existing_id.unwrap_or_else(|| Uuid::now_v7().to_string());
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
                  waveform_peaks_path = COALESCE(tracks.waveform_peaks_path, excluded.waveform_peaks_path),
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
                    track.file_size_bytes,
                    track.artwork_path,
                    track.waveform_peaks_path,
                    track.duration_ms,
                    fingerprint,
                    now_ts
                ],
            )?;
            id
        };

        tx.execute(
            r#"
            INSERT INTO track_usb_links (id, track_id, usb_device_id, usb_file_path, first_seen_at, last_seen_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?5)
            ON CONFLICT(usb_device_id, usb_file_path) DO UPDATE SET
              track_id = excluded.track_id,
              last_seen_at = excluded.last_seen_at
            "#,
            params![Uuid::now_v7().to_string(), id, usb_device_id, file_path, now_ts],
        )?;

        track.local_track_id = Some(id);
        Ok(true)
    }

    /// One-time (but safely re-runnable) cleanup of duplicate `tracks` rows
    /// created by the *pre-fix* `materialize_usb_track_row`, which matched
    /// purely by `file_path` and so always spawned a second, disconnected
    /// row for a USB copy of a song the user already had locally. This does
    /// not touch playback correctness going forward (Step 6/6c's exclusion
    /// and `track_id` fast path already handle that) -- it only removes the
    /// now-redundant duplicate rows so Library search/browse stops showing
    /// the same song twice.
    ///
    /// Deliberately more conservative than the live per-track path in
    /// `materialize_usb_track_row`: a missing duration or file size on the
    /// placeholder row blocks the merge here rather than relaxing that gate,
    /// since a batch pass has no per-track human moment to notice a bad
    /// merge the way a fresh browse does.
    pub fn merge_orphaned_usb_placeholder_tracks(
        &self,
    ) -> BackendResult<crate::models::MergeUsbPlaceholderTracksData> {
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        let usb_root_paths = untainted_usb_root_paths(&tx)?;

        let mut stmt = tx.prepare(
            "SELECT id, file_path, match_fingerprint, duration_ms, file_size_bytes FROM tracks
             WHERE match_fingerprint IS NOT NULL AND match_fingerprint != ''",
        )?;
        type Row = (String, String, Option<i64>, Option<i64>);
        let rows: Vec<(String, Row)> = stmt
            .query_map([], |row| {
                let fingerprint: String = row.get(2)?;
                Ok((
                    fingerprint,
                    (row.get(0)?, row.get(1)?, row.get(3)?, row.get(4)?),
                ))
            })?
            .collect::<Result<_, _>>()?;
        drop(stmt);

        let mut by_fingerprint = HashMap::<String, Vec<Row>>::new();
        for (fingerprint, row) in rows {
            by_fingerprint.entry(fingerprint).or_default().push(row);
        }

        let mut merged = 0usize;
        for group in by_fingerprint.into_values() {
            let (locals, placeholders): (Vec<Row>, Vec<Row>) =
                group.into_iter().partition(|(_, path, _, _)| {
                    !usb_root_paths
                        .iter()
                        .any(|root| browse_path_matches_root(path, root))
                });
            if locals.len() != 1 {
                // Zero or ambiguous local candidates for this fingerprint --
                // leave every row in the group alone.
                continue;
            }
            let (local_id, _local_path, local_duration, local_size) = &locals[0];

            for (placeholder_id, _placeholder_path, placeholder_duration, placeholder_size) in
                &placeholders
            {
                let duration_ok = matches!(
                    (local_duration, placeholder_duration),
                    (Some(a), Some(b)) if (a - b).abs() <= FINGERPRINT_MATCH_DURATION_TOLERANCE_MS
                );
                let size_ok =
                    matches!((local_size, placeholder_size), (Some(a), Some(b)) if a == b);
                if !duration_ok || !size_ok {
                    continue;
                }

                // Avoid landing the same track twice in one playlist: drop
                // the placeholder's row wherever the local track is already
                // present in that playlist, then reassign whatever's left.
                tx.execute(
                    "DELETE FROM playlist_tracks
                     WHERE track_id = ?1
                       AND playlist_id IN (SELECT playlist_id FROM playlist_tracks WHERE track_id = ?2)",
                    params![placeholder_id, local_id],
                )?;
                tx.execute(
                    "UPDATE playlist_tracks SET track_id = ?1 WHERE track_id = ?2",
                    params![local_id, placeholder_id],
                )?;
                // track_usb_links' UNIQUE(usb_device_id, usb_file_path) means
                // a given (device, path) pair can only ever belong to one
                // row regardless of track_id, so this reassignment can never
                // collide with an existing link on the local row.
                tx.execute(
                    "UPDATE track_usb_links SET track_id = ?1 WHERE track_id = ?2",
                    params![local_id, placeholder_id],
                )?;
                tx.execute("DELETE FROM tracks WHERE id = ?1", params![placeholder_id])?;
                merged += 1;
            }
        }

        tx.commit()?;
        Ok(crate::models::MergeUsbPlaceholderTracksData { merged })
    }

    pub fn inspect_usb_track(
        &self,
        req: InspectUsbTrackRequest,
    ) -> BackendResult<InspectUsbTrackData> {
        let track_id = req.track_id.trim().parse::<u32>().map_err(|_| {
            BackendError::Validation("trackId must be a numeric USB track id".to_string())
        })?;
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut warnings = Vec::<WarningEntry>::new();

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

        let parsed = if pdb_path.exists() {
            let parsed = parse_pdb(&pdb_path)?;
            warnings.extend(parsed.warnings.iter().map(|message| {
                logging::log(
                    Level::Warn,
                    "usb-import",
                    "usb.import.pdb-parse",
                    message.clone(),
                )
            }));
            Some(parsed)
        } else {
            warnings.push(logging::log(
                Level::Warn,
                "usb-import",
                "usb.import.pdb-not-found",
                format!(
                    "PDB not found under {}; using DB fallback only",
                    usb_root.display()
                ),
            ));
            None
        };

        let edb_index = try_read_track_index_from_edb(&usb_root, &mut warnings);

        match resolve_usb_track_from_sources(
            track_id,
            &file_hint,
            &title_hint,
            &artist_hint,
            &usb_root,
            parsed.as_ref(),
            edb_index.as_ref(),
        ) {
            Some((source, track)) => Ok(InspectUsbTrackData {
                source,
                track,
                warnings,
            }),
            None => Err(BackendError::Validation(format!(
                "trackId {track_id} not found on USB metadata sources"
            ))),
        }
    }

    /// Batch counterpart to `inspect_usb_track`. Parses the PDB and opens/
    /// queries the eDB connection once for the whole batch instead of once
    /// per track -- selecting a USB playlist used to call `inspect_usb_track`
    /// once per track, which re-opened and re-keyed the SQLCipher eDB
    /// connection (see `try_read_track_index_from_edb_with_conn`'s doc
    /// comment) and re-parsed the PDB on every single call.
    pub fn inspect_usb_tracks(
        &self,
        req: InspectUsbTracksRequest,
    ) -> BackendResult<InspectUsbTracksData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut warnings = Vec::<WarningEntry>::new();

        if req.items.is_empty() {
            return Ok(InspectUsbTracksData {
                items: Vec::new(),
                warnings,
            });
        }

        let pdb_path = vendor_pdb_path(&usb_root);
        let parsed = if pdb_path.exists() {
            let parsed = parse_pdb(&pdb_path)?;
            warnings.extend(parsed.warnings.iter().map(|message| {
                logging::log(
                    Level::Warn,
                    "usb-import",
                    "usb.import.pdb-parse",
                    message.clone(),
                )
            }));
            Some(parsed)
        } else {
            warnings.push(logging::log(
                Level::Warn,
                "usb-import",
                "usb.import.pdb-not-found",
                format!(
                    "PDB not found under {}; using DB fallback only",
                    usb_root.display()
                ),
            ));
            None
        };

        // Open the eDB once and reuse it for every item below -- see
        // `try_read_track_index_from_edb_with_conn`'s doc comment.
        let edb_conn = open_edb_from_usb_root(&usb_root, &mut warnings);
        let edb_index = edb_conn.as_ref().and_then(|conn| {
            try_read_track_index_from_edb_with_conn(conn, &usb_root, &mut warnings)
        });

        let items = req
            .items
            .into_iter()
            .map(|item| {
                let track_id = match item.track_id.trim().parse::<u32>() {
                    Ok(id) => id,
                    Err(_) => {
                        return InspectUsbTrackResult {
                            track_id: item.track_id,
                            source: None,
                            track: None,
                        };
                    }
                };
                let file_hint = item
                    .file_path
                    .as_deref()
                    .map(normalize_text)
                    .unwrap_or_default();
                let title_hint = item
                    .title
                    .as_deref()
                    .map(normalize_text)
                    .unwrap_or_default();
                let artist_hint = item
                    .artist
                    .as_deref()
                    .map(normalize_text)
                    .unwrap_or_default();
                match resolve_usb_track_from_sources(
                    track_id,
                    &file_hint,
                    &title_hint,
                    &artist_hint,
                    &usb_root,
                    parsed.as_ref(),
                    edb_index.as_ref(),
                ) {
                    Some((source, track)) => InspectUsbTrackResult {
                        track_id: item.track_id,
                        source: Some(source),
                        track: Some(track),
                    },
                    None => InspectUsbTrackResult {
                        track_id: item.track_id,
                        source: None,
                        track: None,
                    },
                }
            })
            .collect();

        Ok(InspectUsbTracksData { items, warnings })
    }
}

/// Matches a single USB track id against an already-parsed PDB and/or an
/// already-built eDB track index, preferring the PDB best-match scored
/// against the supplied hints and falling back to the eDB index. Callers are
/// responsible for parsing the PDB and opening/querying the eDB exactly
/// once and passing the results in here, so this can be called once per
/// track in a batch without repeating that work.
fn resolve_usb_track_from_sources(
    track_id: u32,
    file_hint: &str,
    title_hint: &str,
    artist_hint: &str,
    usb_root: &std::path::Path,
    parsed: Option<&ParsedPdb>,
    edb_index: Option<&HashMap<u32, UsbTrack>>,
) -> Option<(String, UsbTrack)> {
    if let Some(parsed) = parsed {
        let mut best: Option<(&crate::pdb_reader::PdbTrackRow, i32)> = None;
        for t in parsed.tracks.iter().filter(|t| t.id == track_id) {
            let artist = parsed
                .artists
                .get(&t.artist_id)
                .cloned()
                .unwrap_or_else(|| "Unknown Artist".to_string());
            let resolved_file_path = resolve_usb_side_path(usb_root, &t.track_file_path)
                .unwrap_or_else(|| t.track_file_path.clone());
            let mut score = 0i32;
            if !file_hint.is_empty() {
                let candidate = normalize_text(&resolved_file_path);
                if candidate.contains(file_hint) || file_hint.contains(&candidate) {
                    score += 8;
                }
            }
            if !title_hint.is_empty() {
                let candidate = normalize_text(&t.title);
                if candidate.contains(title_hint) || title_hint.contains(&candidate) {
                    score += 4;
                }
            }
            if !artist_hint.is_empty() {
                let candidate = normalize_text(&artist);
                if candidate.contains(artist_hint) || artist_hint.contains(&candidate) {
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
                    .and_then(|p| resolve_usb_side_path(usb_root, p));
                let resolved_file_path = resolve_usb_side_path(usb_root, &t.track_file_path)
                    .unwrap_or_else(|| t.track_file_path.clone());
                let usb_analysis_path = resolve_usb_side_path(usb_root, &t.anlz_path);
                let waveform_preview = usb_analysis_path
                    .as_deref()
                    .and_then(load_waveform_preview_from_analysis_path);
                return Some((
                    "pdb".to_string(),
                    UsbTrack {
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
                        duration_ms: t.duration_seconds.map(|seconds| u64::from(seconds) * 1000),
                        file_size_bytes: t.file_size_bytes.map(i64::from),
                    },
                ));
            }
        }
    }

    if let Some(index) = edb_index
        && let Some(mut track) = index.get(&track_id).cloned()
    {
        let mut file_hint_match = true;
        if !file_hint.is_empty() {
            let candidate = normalize_text(&track.file_path);
            file_hint_match = candidate.contains(file_hint) || file_hint.contains(&candidate);
        }
        if file_hint_match {
            if track.waveform_preview.is_none() {
                track.waveform_preview = track
                    .usb_analysis_path
                    .as_deref()
                    .and_then(load_waveform_preview_from_analysis_path);
            }
            return Some(("eDB".to_string(), track));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{
        SLOW_USB_STAGE_MS, apply_history_dates_from_track_date_created,
        build_history_track_date_index, build_track_match_fingerprint, build_usb_track_index,
        cleanup_empty_dirs_recursive, edb_track_index_from_playlist_tracks,
        filter_named_history_playlists, normalize_date_created, now, push_usb_stage_timing,
        push_usb_stage_timing_with_threshold, select_history_rows, slow_stage_threshold_ms,
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
            file_size_bytes: None,
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

    // --- filter_named_history_playlists ---

    #[test]
    fn filter_named_history_playlists_drops_blank_seed_rows() {
        // Shaped like the real byte-level data confirmed on several unrelated
        // fresh/never-played rekordbox exports: a block of blank-name
        // template rows shipped alongside one real session.
        let playlists = vec![
            PdbHistoryPlaylistRow {
                id: 65537,
                name: String::new(),
                source_table: 17,
            },
            PdbHistoryPlaylistRow {
                id: 393221,
                name: "\u{1}".to_string(),
                source_table: 17,
            },
            PdbHistoryPlaylistRow {
                id: 10,
                name: "HISTORY 001".to_string(),
                source_table: 17,
            },
        ];

        let filtered = filter_named_history_playlists(playlists);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, 10);
    }

    #[test]
    fn filter_named_history_playlists_drops_all_seed_only_export() {
        // A fresh/never-played export: every t17 row is a blank template
        // slot, no real session recorded yet.
        let playlists = (0..22)
            .map(|id| PdbHistoryPlaylistRow {
                id,
                name: String::new(),
                source_table: 17,
            })
            .collect::<Vec<_>>();

        assert!(filter_named_history_playlists(playlists).is_empty());
    }

    #[test]
    fn filter_named_history_playlists_keeps_all_real_sessions() {
        let playlists = vec![
            PdbHistoryPlaylistRow {
                id: 1,
                name: "HISTORY 001".to_string(),
                source_table: 17,
            },
            PdbHistoryPlaylistRow {
                id: 2,
                name: "HISTORY 002".to_string(),
                source_table: 17,
            },
        ];

        assert_eq!(filter_named_history_playlists(playlists).len(), 2);
    }

    #[test]
    fn slow_stage_threshold_ms_equals_baseline_at_zero_items() {
        assert_eq!(slow_stage_threshold_ms(0), SLOW_USB_STAGE_MS);
    }

    #[test]
    fn slow_stage_threshold_ms_scales_linearly_above_baseline() {
        let baseline = slow_stage_threshold_ms(0);
        let at_1000 = slow_stage_threshold_ms(1_000);
        let at_2000 = slow_stage_threshold_ms(2_000);
        assert!(at_1000 > baseline, "threshold should grow with item count");
        // Linear: doubling the item count should double the allowance added on top of baseline.
        assert_eq!((at_2000 - baseline), (at_1000 - baseline) * 2);
    }

    // --- build_usb_track_index / edb_track_index_from_playlist_tracks ---

    #[allow(clippy::too_many_arguments)]
    fn make_indexed_pdb_track(
        id: u32,
        title: &str,
        artist_id: u32,
        album_id: u32,
        key_id: u32,
        artwork_id: u32,
        track_number: u32,
        tempo_x100: u32,
        track_file_path: &str,
        anlz_path: &str,
    ) -> PdbTrackRow {
        PdbTrackRow {
            content_link: None,
            sample_rate_hz: None,
            file_size_bytes: Some(4096),
            master_content_id: None,
            master_db_id: None,
            id,
            artist_id,
            album_id,
            artwork_id,
            key_id,
            genre_id: 0,
            bitrate_kbps: None,
            track_number,
            tempo_x100,
            release_year: None,
            bit_depth: None,
            duration_seconds: Some(200),
            file_type: None,
            isrc: None,
            date_added: None,
            release_date: None,
            dj_comment: None,
            file_name: None,
            publish_track_info: None,
            autoload_hotcues: None,
            title: title.to_string(),
            anlz_path: anlz_path.to_string(),
            track_file_path: track_file_path.to_string(),
        }
    }

    #[test]
    fn build_usb_track_index_resolves_metadata_and_paths() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let usb_root = tmp.path();
        let parsed = crate::pdb_reader::ParsedPdb {
            tracks: vec![make_indexed_pdb_track(
                1,
                "Song One",
                10,
                20,
                30,
                40,
                5,
                12500,
                "/Contents/Artist/Song One.mp3",
                "/PIONEER/USBANLZ/P001/ANLZ0000.DAT",
            )],
            artists: HashMap::from([(10u32, "DJ Test".to_string())]),
            albums: HashMap::from([(20u32, "Test Album".to_string())]),
            keys: HashMap::from([(30u32, "8A".to_string())]),
            artworks: HashMap::from([(40u32, "/PIONEER/Artwork/a.jpg".to_string())]),
            ..Default::default()
        };

        let index = build_usb_track_index(&parsed, usb_root);
        let track = index.get(&1).expect("track present");
        assert_eq!(track.title, "Song One");
        assert_eq!(track.artist, "DJ Test");
        assert_eq!(track.album.as_deref(), Some("Test Album"));
        assert_eq!(track.key.as_deref(), Some("8A"));
        assert_eq!(track.track_number, Some(5));
        assert_eq!(track.bpm, Some(125.0));
        assert!(track.artwork_path.is_some());
        assert!(track.file_path.ends_with("Song One.mp3"));
        assert!(track.usb_analysis_path.is_some());
        assert_eq!(
            track.usb_analysis_path_raw.as_deref(),
            Some("/PIONEER/USBANLZ/P001/ANLZ0000.DAT")
        );
        assert_eq!(track.duration_ms, Some(200_000));
    }

    #[test]
    fn build_usb_track_index_falls_back_when_title_artist_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let usb_root = tmp.path();
        let parsed = crate::pdb_reader::ParsedPdb {
            tracks: vec![make_indexed_pdb_track(
                2,
                "",
                999,
                0,
                0,
                0,
                0,
                0,
                "relative/song.mp3",
                "",
            )],
            ..Default::default()
        };

        let index = build_usb_track_index(&parsed, usb_root);
        let track = index.get(&2).expect("track present");
        assert_eq!(track.title, "Unknown Title");
        assert_eq!(track.artist, "Unknown Artist");
        assert_eq!(track.album, None);
        assert_eq!(track.track_number, None);
        assert_eq!(track.bpm, None);
        assert!(track.usb_analysis_path.is_none());
    }

    #[test]
    fn edb_track_index_from_playlist_tracks_none_returns_empty() {
        assert!(edb_track_index_from_playlist_tracks(None).is_empty());
    }

    #[test]
    fn edb_track_index_from_playlist_tracks_dedupes_by_parsed_id() {
        let map = HashMap::from([
            (
                "Playlist A".to_string(),
                vec![make_track("7", "/a.mp3"), make_track("7", "/dup.mp3")],
            ),
            ("Playlist B".to_string(), vec![make_track("9", "/b.mp3")]),
        ]);
        let index = edb_track_index_from_playlist_tracks(Some(&map));
        assert_eq!(index.len(), 2);
        assert!(index.contains_key(&7));
        assert!(index.contains_key(&9));
    }

    // --- cleanup_empty_dirs_recursive ---

    #[test]
    fn cleanup_empty_dirs_recursive_removes_empty_but_keeps_non_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("empty/nested")).unwrap();
        std::fs::create_dir_all(root.join("kept")).unwrap();
        std::fs::write(root.join("kept/file.txt"), b"data").unwrap();

        cleanup_empty_dirs_recursive(root);

        assert!(!root.join("empty").exists());
        assert!(root.join("kept").exists());
        assert!(root.join("kept/file.txt").exists());
    }

    // --- push_usb_stage_timing / push_usb_stage_timing_with_threshold ---

    #[test]
    fn push_usb_stage_timing_with_threshold_flags_slow_stage_at_zero_threshold() {
        let mut warnings = Vec::new();
        let mut started = std::time::Instant::now();
        push_usb_stage_timing_with_threshold(&mut warnings, "test-stage", &mut started, 0);
        assert!(
            warnings.iter().any(|w| w.code == "usb.import.slow-media"),
            "elapsed time always exceeds a zero threshold"
        );
        assert!(warnings.iter().any(|w| w.code == "usb.import.stage-timing"));
    }

    #[test]
    fn push_usb_stage_timing_with_threshold_no_warning_below_threshold() {
        let mut warnings = Vec::new();
        let mut started = std::time::Instant::now();
        push_usb_stage_timing_with_threshold(&mut warnings, "test-stage", &mut started, 60_000);
        assert!(!warnings.iter().any(|w| w.code == "usb.import.slow-media"));
        assert!(warnings.iter().any(|w| w.code == "usb.import.stage-timing"));
    }

    #[test]
    fn push_usb_stage_timing_uses_default_threshold() {
        let mut warnings = Vec::new();
        let mut started = std::time::Instant::now();
        push_usb_stage_timing(&mut warnings, "quick-stage", &mut started);
        assert!(!warnings.iter().any(|w| w.code == "usb.import.slow-media"));
    }

    // --- BackendService USB integration tests ---

    fn test_service() -> (tempfile::TempDir, crate::service::BackendService) {
        let dir = tempfile::tempdir().expect("service data dir");
        let service = crate::service::BackendService::new(dir.path()).expect("backend service");
        (dir, service)
    }

    fn test_usb_root() -> (tempfile::TempDir, std::path::PathBuf) {
        let td = tempfile::tempdir().expect("tempdir");
        let usb_root = td.path().join("USB_TEST");
        std::fs::create_dir_all(&usb_root).expect("create usb root");
        crate::service::initialize_usb(usb_root.to_str().expect("utf-8 usb root path"))
            .expect("initialize usb");
        (td, usb_root)
    }

    fn make_export_track(id: &str, title: &str, filename: &str) -> crate::edb::ExportTrackData {
        crate::edb::ExportTrackData {
            id: id.to_string(),
            title: title.to_string(),
            artist: "Artist".to_string(),
            album: Some("Album".to_string()),
            track_number: Some(1),
            bpm: Some(120.0),
            key: Some("8A".to_string()),
            file_path: format!("/source/{filename}"),
            file_name: filename.to_string(),
            file_modified_at: None,
            file_size_bytes: Some(4_000_000),
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
            duration_ms: Some(200_000),
            first_beat_ms: None,
            position: 0,
        }
    }

    fn make_export_manifest(
        playlist_id: &str,
        playlist_name: &str,
        usb_root: &std::path::Path,
        tracks: &[(&str, &str, &str)],
    ) -> crate::service::export_helpers::ExportManifest {
        crate::service::export_helpers::ExportManifest {
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
                .map(
                    |(i, (id, title, filename))| crate::edb::ExportManifestTrack {
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
                        key: Some("8A".to_string()),
                        source_path: format!("/source/{filename}"),
                        exported_path: format!("/Contents/Artist/Album/{filename}"),
                        file_modified_at: None,
                        file_size_bytes: Some(4_000_000),
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
                        duration_ms: Some(200_000),
                    },
                )
                .collect(),
        }
    }

    fn seeded_playlist_usb_with_tracks(
        tracks: &[(&str, &str, &str)],
    ) -> (tempfile::TempDir, std::path::PathBuf) {
        let (dir, usb_root) = test_usb_root();
        let playlist = crate::edb::ExportPlaylistData {
            id: "pl-1".to_string(),
            name: "My Playlist".to_string(),
            tracks: tracks
                .iter()
                .map(|(id, title, filename)| make_export_track(id, title, filename))
                .collect(),
        };
        let manifest = make_export_manifest("pl-1", "My Playlist", &usb_root, tracks);
        crate::service::export_helpers::write_pdb(
            &usb_root, &playlist, &manifest, true, None, None,
        )
        .expect("write pdb");
        (dir, usb_root)
    }

    fn seeded_playlist_usb() -> (tempfile::TempDir, std::path::PathBuf) {
        seeded_playlist_usb_with_tracks(&[("t1", "Song A", "a.mp3")])
    }

    #[test]
    fn validate_usb_root_empty_path_is_invalid() {
        let (_dir, service) = test_service();
        let result = service
            .validate_usb_root(crate::models::ValidateUsbRootRequest {
                path: "   ".to_string(),
            })
            .expect("validate");
        assert!(!result.valid);
        assert!(!result.has_write_access);
        assert!(result.normalized_root.is_none());
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.code == "usb.import.path-empty")
        );
    }

    #[test]
    fn validate_usb_root_missing_path_is_invalid() {
        let (_dir, service) = test_service();
        let missing = std::env::temp_dir().join(format!("does-not-exist-{}", uuid::Uuid::now_v7()));
        let result = service
            .validate_usb_root(crate::models::ValidateUsbRootRequest {
                path: missing.to_string_lossy().to_string(),
            })
            .expect("validate");
        assert!(!result.valid);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.code == "usb.import.path-not-found")
        );
    }

    #[test]
    fn validate_usb_root_bare_directory_reports_missing_pieces() {
        let (_dir, service) = test_service();
        let usb_dir = tempfile::tempdir().expect("usb dir");
        let result = service
            .validate_usb_root(crate::models::ValidateUsbRootRequest {
                path: usb_dir.path().to_string_lossy().to_string(),
            })
            .expect("validate");
        assert!(!result.valid);
        assert!(!result.has_vendor_root);
        assert!(!result.has_contents);
        assert!(!result.has_pdb);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.code == "usb.import.missing-vendor-root")
        );
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.code == "usb.import.missing-contents")
        );
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.code == "usb.import.missing-pdb")
        );
    }

    #[test]
    fn validate_usb_root_initialized_usb_is_valid_and_registers_device() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let result = service
            .validate_usb_root(crate::models::ValidateUsbRootRequest {
                path: usb_root.to_string_lossy().to_string(),
            })
            .expect("validate");
        assert!(result.valid);
        assert!(result.has_vendor_root);
        assert!(result.has_contents);
        assert!(result.has_pdb);
        assert!(result.has_edb);
        assert!(result.normalized_root.is_some());

        let devices = service.list_usb_devices().expect("list devices");
        assert_eq!(devices.items.len(), 1);
        assert!(devices.items[0].mounted);
    }

    #[test]
    fn prune_usb_device_soft_deletes_and_removes_from_listing() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        service
            .validate_usb_root(crate::models::ValidateUsbRootRequest {
                path: usb_root.to_string_lossy().to_string(),
            })
            .expect("validate");
        let devices = service.list_usb_devices().expect("list devices");
        let id = devices.items[0].id.clone();

        let pruned = service
            .prune_usb_device(crate::models::PruneUsbDeviceRequest { id: id.clone() })
            .expect("prune");
        assert!(pruned.pruned);

        let after = service
            .list_usb_devices()
            .expect("list devices after prune");
        assert!(after.items.is_empty());

        let pruned_again = service
            .prune_usb_device(crate::models::PruneUsbDeviceRequest { id })
            .expect("prune again");
        assert!(
            !pruned_again.pruned,
            "already-pruned device should be a no-op"
        );
    }

    #[test]
    fn fetch_usb_playlists_returns_playlist_with_track() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = seeded_playlist_usb();

        let result = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            })
            .expect("fetch usb playlists");

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].name, "My Playlist");
        assert_eq!(result.items[0].track_count, 1);
        assert_eq!(result.items[0].tracks[0].title, "Song A");
        assert_eq!(result.stats.indexed_tracks, 1);
    }

    #[test]
    fn reorder_usb_playlists_updates_pdb_sort_order() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let playlist_a = crate::edb::ExportPlaylistData {
            id: "pl-a".to_string(),
            name: "Playlist A".to_string(),
            tracks: vec![make_export_track("t1", "Song A", "a.mp3")],
        };
        let manifest_a = make_export_manifest(
            "pl-a",
            "Playlist A",
            &usb_root,
            &[("t1", "Song A", "a.mp3")],
        );
        crate::service::export_helpers::write_pdb(
            &usb_root,
            &playlist_a,
            &manifest_a,
            true,
            None,
            None,
        )
        .expect("write playlist a");

        let playlist_b = crate::edb::ExportPlaylistData {
            id: "pl-b".to_string(),
            name: "Playlist B".to_string(),
            tracks: vec![make_export_track("t2", "Song B", "b.mp3")],
        };
        let manifest_b = make_export_manifest(
            "pl-b",
            "Playlist B",
            &usb_root,
            &[("t2", "Song B", "b.mp3")],
        );
        crate::service::export_helpers::write_pdb(
            &usb_root,
            &playlist_b,
            &manifest_b,
            true,
            None,
            None,
        )
        .expect("write playlist b");

        let fetched = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            })
            .expect("fetch before reorder");
        let mut ids: Vec<String> = fetched.items.iter().map(|p| p.id.clone()).collect();
        ids.reverse();

        let result = service
            .reorder_usb_playlists(crate::models::ReorderUsbPlaylistsRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                ordered_playlist_ids: ids,
            })
            .expect("reorder playlists");
        assert!(result.reordered > 0);
    }

    #[test]
    fn reorder_usb_playlists_rejects_ids_with_no_pdb_backed_entries() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let err = service
            .reorder_usb_playlists(crate::models::ReorderUsbPlaylistsRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                ordered_playlist_ids: vec!["usb-pl-name-only".to_string()],
            })
            .unwrap_err();
        assert!(matches!(err, crate::error::BackendError::Validation(_)));
    }

    #[test]
    fn remove_usb_playlist_removes_from_pdb_and_edb() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = seeded_playlist_usb();

        let result = service
            .remove_usb_playlist(crate::models::RemoveUsbPlaylistRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                playlist_id: None,
                playlist_name: "My Playlist".to_string(),
            })
            .expect("remove usb playlist");

        assert_eq!(result.removed_from_pdb, 1);

        let after = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            })
            .expect("fetch after remove");
        assert!(after.items.iter().all(|p| p.name != "My Playlist"));
    }

    #[test]
    fn remove_usb_playlist_rejects_empty_name() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let err = service
            .remove_usb_playlist(crate::models::RemoveUsbPlaylistRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                playlist_id: None,
                playlist_name: "   ".to_string(),
            })
            .unwrap_err();
        assert!(matches!(err, crate::error::BackendError::Validation(_)));
    }

    #[test]
    fn remove_usb_playlist_not_found_returns_error() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let err = service
            .remove_usb_playlist(crate::models::RemoveUsbPlaylistRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                playlist_id: None,
                playlist_name: "Does Not Exist".to_string(),
            })
            .unwrap_err();
        assert!(matches!(err, crate::error::BackendError::NotFound(_)));
    }

    #[test]
    fn inspect_usb_track_finds_by_id_from_pdb() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = seeded_playlist_usb();

        let fetched = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            })
            .expect("fetch playlists");
        let track_id = fetched.items[0].tracks[0].id.clone();

        let inspected = service
            .inspect_usb_track(crate::models::InspectUsbTrackRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                track_id,
                file_path: None,
                title: None,
                artist: None,
            })
            .expect("inspect usb track");
        assert_eq!(inspected.track.title, "Song A");
    }

    #[test]
    fn inspect_usb_track_errors_for_unknown_id() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = seeded_playlist_usb();

        let err = service
            .inspect_usb_track(crate::models::InspectUsbTrackRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                track_id: "999999".to_string(),
                file_path: None,
                title: None,
                artist: None,
            })
            .unwrap_err();
        assert!(matches!(err, crate::error::BackendError::Validation(_)));
    }

    #[test]
    fn inspect_usb_track_rejects_non_numeric_id() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let err = service
            .inspect_usb_track(crate::models::InspectUsbTrackRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                track_id: "not-a-number".to_string(),
                file_path: None,
                title: None,
                artist: None,
            })
            .unwrap_err();
        assert!(matches!(err, crate::error::BackendError::Validation(_)));
    }

    #[test]
    fn inspect_usb_tracks_resolves_multiple_ids_matching_single_calls() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = seeded_playlist_usb_with_tracks(&[
            ("t1", "Song A", "a.mp3"),
            ("t2", "Song B", "b.mp3"),
        ]);
        let usb_root_str = usb_root.to_string_lossy().to_string();

        let fetched = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(usb_root_str.clone()),
            })
            .expect("fetch playlists");
        let track_ids: Vec<String> = fetched.items[0]
            .tracks
            .iter()
            .map(|t| t.id.clone())
            .collect();
        assert_eq!(track_ids.len(), 2);

        let batch = service
            .inspect_usb_tracks(crate::models::InspectUsbTracksRequest {
                usb_root: Some(usb_root_str.clone()),
                items: track_ids
                    .iter()
                    .map(|id| crate::models::InspectUsbTrackItem {
                        track_id: id.clone(),
                        file_path: None,
                        title: None,
                        artist: None,
                    })
                    .collect(),
            })
            .expect("inspect usb tracks");
        assert_eq!(batch.items.len(), 2);

        // Every batched result must agree with what a per-track
        // `inspect_usb_track` call would have returned -- the batch path is
        // a refactor of the same matching logic, not a different one.
        for id in &track_ids {
            let single = service
                .inspect_usb_track(crate::models::InspectUsbTrackRequest {
                    usb_root: Some(usb_root_str.clone()),
                    track_id: id.clone(),
                    file_path: None,
                    title: None,
                    artist: None,
                })
                .expect("inspect single usb track");
            let batched = batch
                .items
                .iter()
                .find(|item| &item.track_id == id)
                .expect("expected batch result for id");
            assert_eq!(batched.source.as_deref(), Some(single.source.as_str()));
            let batched_track = batched.track.as_ref().expect("expected resolved track");
            assert_eq!(batched_track.title, single.track.title);
            assert_eq!(batched_track.artist, single.track.artist);
        }
    }

    #[test]
    fn inspect_usb_tracks_returns_none_for_unknown_id_without_failing_batch() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = seeded_playlist_usb();
        let usb_root_str = usb_root.to_string_lossy().to_string();

        let fetched = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(usb_root_str.clone()),
            })
            .expect("fetch playlists");
        let valid_id = fetched.items[0].tracks[0].id.clone();

        let batch = service
            .inspect_usb_tracks(crate::models::InspectUsbTracksRequest {
                usb_root: Some(usb_root_str),
                items: vec![
                    crate::models::InspectUsbTrackItem {
                        track_id: valid_id.clone(),
                        file_path: None,
                        title: None,
                        artist: None,
                    },
                    crate::models::InspectUsbTrackItem {
                        track_id: "999999".to_string(),
                        file_path: None,
                        title: None,
                        artist: None,
                    },
                ],
            })
            .expect("inspect usb tracks");
        // Unlike `inspect_usb_track`, one unresolved id must not fail the
        // whole batch -- the caller still gets every other result back.
        assert_eq!(batch.items.len(), 2);
        let valid = batch
            .items
            .iter()
            .find(|i| i.track_id == valid_id)
            .expect("valid result");
        assert!(valid.track.is_some());
        assert_eq!(valid.source.as_deref(), Some("pdb"));
        let unknown = batch
            .items
            .iter()
            .find(|i| i.track_id == "999999")
            .expect("unknown result");
        assert!(unknown.track.is_none());
        assert!(unknown.source.is_none());
    }

    #[test]
    fn inspect_usb_tracks_treats_non_numeric_id_as_unresolved_item() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = seeded_playlist_usb();
        let usb_root_str = usb_root.to_string_lossy().to_string();

        let fetched = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(usb_root_str.clone()),
            })
            .expect("fetch playlists");
        let valid_id = fetched.items[0].tracks[0].id.clone();

        let batch = service
            .inspect_usb_tracks(crate::models::InspectUsbTracksRequest {
                usb_root: Some(usb_root_str),
                items: vec![
                    crate::models::InspectUsbTrackItem {
                        track_id: "not-a-number".to_string(),
                        file_path: None,
                        title: None,
                        artist: None,
                    },
                    crate::models::InspectUsbTrackItem {
                        track_id: valid_id.clone(),
                        file_path: None,
                        title: None,
                        artist: None,
                    },
                ],
            })
            .expect("inspect usb tracks");
        assert_eq!(batch.items.len(), 2);
        let bad = batch
            .items
            .iter()
            .find(|i| i.track_id == "not-a-number")
            .expect("bad id result");
        assert!(bad.track.is_none());
        assert!(bad.source.is_none());
        let good = batch
            .items
            .iter()
            .find(|i| i.track_id == valid_id)
            .expect("valid result");
        assert!(good.track.is_some());
    }

    #[test]
    fn inspect_usb_tracks_returns_empty_items_for_empty_batch_without_reading_disk() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();

        let result = service
            .inspect_usb_tracks(crate::models::InspectUsbTracksRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                items: Vec::new(),
            })
            .expect("inspect usb tracks with empty items");
        assert!(result.items.is_empty());
        // An empty batch must short-circuit before parsing the PDB or
        // opening the eDB -- if it didn't, the freshly-initialized fixture
        // root (which has a full-shape PDB+eDB) would still produce zero
        // warnings here anyway, so this only proves the *shape* of the
        // response, not the short-circuit; the short-circuit itself is
        // covered by `inspect_usb_tracks_opens_edb_connection_once_for_whole_batch`
        // asserting exactly one eDB open for a non-empty batch.
        assert!(
            result.warnings.is_empty(),
            "expected no warnings for empty batch: {:?}",
            result.warnings
        );
    }

    #[test]
    fn inspect_usb_tracks_opens_edb_connection_once_for_whole_batch() {
        let (_dir, service) = test_service();
        let usb_root_dir = tempfile::tempdir().expect("usb root dir");
        let usb_root = usb_root_dir.path().to_path_buf();
        let edb_path = super::vendor_edb_path(&usb_root);
        std::fs::create_dir_all(edb_path.parent().expect("edb parent dir"))
            .expect("create vendor db dir");
        let conn = rusqlite::Connection::open(&edb_path).expect("create eDB fixture");
        conn.execute_batch(
            r#"
            CREATE TABLE content (
              content_id INTEGER PRIMARY KEY,
              title TEXT,
              artist_id_artist INTEGER,
              album_id INTEGER,
              bpmx100 INTEGER,
              key_id INTEGER,
              path TEXT,
              image_id INTEGER,
              analysisDataFilePath TEXT
            );
            CREATE TABLE artist (artist_id INTEGER PRIMARY KEY, name TEXT);
            CREATE TABLE album (album_id INTEGER PRIMARY KEY, name TEXT);
            CREATE TABLE "key" (key_id INTEGER PRIMARY KEY, name TEXT);
            CREATE TABLE image (image_id INTEGER PRIMARY KEY, path TEXT);
            INSERT INTO artist (artist_id, name) VALUES (1, 'eDB Artist');
            INSERT INTO content (content_id, title, artist_id_artist, path) VALUES
              (101, 'eDB Song A', 1, '/Contents/a.mp3'),
              (102, 'eDB Song B', 1, '/Contents/b.mp3'),
              (103, 'eDB Song C', 1, '/Contents/c.mp3');
            "#,
        )
        .expect("seed eDB fixture schema");
        drop(conn);

        // No PDB on disk at all -- forces every item through the eDB
        // fallback path, exercising `edb_index` reuse across the batch.
        let batch = service
            .inspect_usb_tracks(crate::models::InspectUsbTracksRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                items: ["101", "102", "103"]
                    .into_iter()
                    .map(|id| crate::models::InspectUsbTrackItem {
                        track_id: id.to_string(),
                        file_path: None,
                        title: None,
                        artist: None,
                    })
                    .collect(),
            })
            .expect("inspect usb tracks (edb-only)");

        assert_eq!(batch.items.len(), 3);
        for item in &batch.items {
            assert_eq!(item.source.as_deref(), Some("eDB"));
            assert_eq!(
                item.track.as_ref().expect("resolved track").artist,
                "eDB Artist"
            );
        }
        // The actual regression this batch command exists to prevent: one
        // `edb.open.no-key` warning per open. Before batching, selecting a
        // playlist opened the eDB once per track; this must stay at 1 for
        // the whole batch regardless of how many items it contains.
        let edb_opens = batch
            .warnings
            .iter()
            .filter(|w| w.code == "edb.open.no-key")
            .count();
        assert_eq!(
            edb_opens, 1,
            "expected exactly one eDB open for the whole batch, got {edb_opens}: {:?}",
            batch.warnings
        );
    }

    #[test]
    fn fetch_usb_histories_returns_empty_when_pdb_missing() {
        let (_dir, service) = test_service();
        let usb_dir = tempfile::tempdir().expect("usb dir");

        let result = service
            .fetch_usb_histories(crate::models::FetchUsbHistoriesRequest {
                usb_root: Some(usb_dir.path().to_string_lossy().to_string()),
            })
            .expect("fetch usb histories");

        assert!(result.items.is_empty());
        assert_eq!(result.counts.imported_playlists, 0);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.code == "usb.histories.pdb-not-found")
        );
    }

    #[test]
    fn merge_orphaned_usb_placeholder_tracks_reassigns_playlist_and_links() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        let usb_root_path = "/mnt/usb-device";
        crate::service::usb_utils::upsert_usb_device(
            &conn,
            std::path::Path::new(usb_root_path),
            false,
            &now(),
        )
        .expect("seed usb device");

        let fingerprint = build_track_match_fingerprint("Song", "Artist", None);
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, duration_ms, file_size_bytes, match_fingerprint, created_at, updated_at)
             VALUES ('local-1', 'Song', 'Artist', '/library/song.mp3', 200000, 5000, ?1, datetime('now'), datetime('now'))",
            rusqlite::params![fingerprint],
        )
        .expect("insert local track");
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, duration_ms, file_size_bytes, match_fingerprint, created_at, updated_at)
             VALUES ('placeholder-1', 'Song', 'Artist', '/mnt/usb-device/Contents/song.mp3', 200000, 5000, ?1, datetime('now'), datetime('now'))",
            rusqlite::params![fingerprint],
        )
        .expect("insert placeholder track");
        drop(conn);

        let playlist = service
            .create_playlist(crate::models::CreatePlaylistRequest {
                name: "PL".to_string(),
            })
            .expect("create playlist");
        let conn = service.db.connect().expect("connect");
        conn.execute(
            "INSERT INTO playlist_tracks (id, playlist_id, track_id, position, added_at)
             VALUES ('pt-1', ?1, 'placeholder-1', 0, datetime('now'))",
            rusqlite::params![playlist.playlist_id],
        )
        .expect("link playlist track");
        drop(conn);

        let result = service
            .merge_orphaned_usb_placeholder_tracks()
            .expect("merge placeholders");
        assert_eq!(result.merged, 1);

        let conn = service.db.connect().expect("connect");
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tracks WHERE id = 'placeholder-1'",
                [],
                |r| r.get(0),
            )
            .expect("count placeholder");
        assert_eq!(remaining, 0);
        let playlist_track_id: String = conn
            .query_row(
                "SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1",
                rusqlite::params![playlist.playlist_id],
                |r| r.get(0),
            )
            .expect("playlist track id");
        assert_eq!(playlist_track_id, "local-1");
    }
}
