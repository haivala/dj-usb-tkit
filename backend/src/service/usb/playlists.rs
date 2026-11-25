//! USB playlists: fetching the resolved list, paging a playlist's tracks,
//! and removing a playlist from the stick.

use super::cache::CachedUsbPlaylists;
use super::root::resolve_usb_root;
use super::staging::{UsbDatabases, UsbDatabasesExt};
use super::text::canonicalize_playlist_name;
use super::tracks::{build_usb_track_index, hydrate_usb_page, paginate_usb_tracks};
use super::{push_usb_stage_timing, push_usb_stage_timing_with_threshold, slow_stage_threshold_ms};
use crate::error::{BackendError, BackendResult};
use crate::i18n::tr;
use crate::logging::{self, Level};
use crate::models::{
    FetchUsbPlaylistsData, FetchUsbPlaylistsRequest, FetchUsbTracksData, FetchUsbTracksRequest,
    RemoveUsbPlaylistData, RemoveUsbPlaylistRequest, UsbImportStats, UsbPlaylist, UsbTrack,
    WarningEntry,
};
use crate::pdb_reader::ParsedPdb;
use crate::service::BackendService;
use crate::service::export_helpers::{
    analysis_bundle_path_variants, edb_playlist_exclusive_tracks, prune_stale_export_owned_files,
    remove_playlist_and_tracks_from_pdb_in, remove_playlist_from_edb,
};
use crate::service::track_list::duration_totals;
use crate::utils::{counted, warning_text};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

/// A USB playlist with its click and remove texts, built when answering (see
/// `with_usb_history_texts`).
pub(super) fn with_usb_playlist_texts(playlist: &UsbPlaylist) -> UsbPlaylist {
    let mut playlist = playlist.clone();
    playlist.selected_text =
        tr::usb_playlists_selected(&playlist.name, playlist.track_count).render();
    playlist.remove_confirm_text = tr::usb_playlists_remove_confirm(&playlist.name).render();
    playlist
}

/// Recursively remove empty directories bottom-up.
pub(super) fn cleanup_empty_dirs_recursive(dir: &Path) {
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

/// An artwork file and its medium variant (`.jpg` → `_m.jpg`).
pub(super) fn artwork_size_variants(small: &str) -> Vec<String> {
    let medium = small.replace(".jpg", "_m.jpg").replace(".png", "_m.png");
    if medium == small {
        vec![medium]
    } else {
        vec![small.to_string(), medium]
    }
}

impl BackendService {
    /// Build the `FetchUsbPlaylistsData` response from a resolved parse: in
    /// the pending order while an order edit is on.
    pub(super) fn finish_usb_playlists(
        &self,
        usb_root: &Path,
        resolved: &CachedUsbPlaylists,
    ) -> BackendResult<FetchUsbPlaylistsData> {
        let playlist_count = resolved.items.len();
        let track_total: usize = resolved.items.iter().map(|p| p.track_count).sum();
        let summary_text = tr::usb_playlists_summary(playlist_count, track_total).render();
        let warning_count = logging::count_warnings(&resolved.warnings);
        let order_edit = self.usb_order_edit_state(usb_root);
        // The tree's order, each folder's items in the pending order.
        let pending = self.pending_usb_playlist_order(usb_root);
        let walked = super::playlist_tree::walk(&resolved.tree, &pending);
        let position: HashMap<&str, usize> = walked
            .iter()
            .enumerate()
            .map(|(index, (node, _))| (node.id.as_str(), index))
            .collect();
        let mut items: Vec<UsbPlaylist> =
            resolved.items.iter().map(with_usb_playlist_texts).collect();
        items.sort_by_key(|item| {
            position
                .get(item.id.as_str())
                .copied()
                .unwrap_or(usize::MAX)
        });
        for item in &mut items {
            item.order_edited = order_edit.edited_playlist_ids.contains(&item.id);
        }
        Ok(FetchUsbPlaylistsData {
            items,
            tree: super::playlist_tree::tree_rows(&walked, &pending),
            order_edit,
            order_edit_blocked_text: Self::usb_order_edit_blocked_text(usb_root),
            stats: resolved.stats.clone(),
            status_text: tr::usb_playlists_loaded(playlist_count).render(),
            done_text: tr::usb_playlists_done(&summary_text).render(),
            summary_text,
            warning_text: warning_text(warning_count),
            warnings: resolved.warnings.clone(),
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
        F: crate::progress::ProgressFn,
    {
        let mut warnings = Vec::<WarningEntry>::new();
        let mut stage_started = std::time::Instant::now();
        on_progress(2, 100, &tr::usb_progress_resolving_root().render());
        on_progress(10, 100, &tr::usb_progress_parsing_pdb().render());
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        push_usb_stage_timing("resolve usb root", &mut stage_started);

        let cache_key = self.usb_parse_cache_key(&usb_root);
        if let Some(resolved) = self.usb_playlists_cache_hit(&cache_key) {
            return self.finish_usb_playlists(&usb_root, &resolved);
        }

        let databases = UsbDatabases::present(&usb_root)?;
        let staged_parse = if let Some(pdb_path) = databases.pdb() {
            let staged = self.parsed_staged_pdb(pdb_path)?;
            for message in &staged.parsed.warnings {
                logging::debug("usb-import", message);
            }
            Some(staged)
        } else {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.import.pdb-not-found",
                &tr::usb_warning_pdb_not_found_edb_only(usb_root.display()),
            ));
            None
        };
        let parsed: Option<&ParsedPdb> = staged_parse.as_deref().map(|staged| &*staged.parsed);
        push_usb_stage_timing("parse PDB", &mut stage_started);

        on_progress(30, 100, &tr::usb_progress_reading_edb().render());
        let (edb_track_by_id, edb_playlists, edb_folders) = self
            .staged_edb(&usb_root, None, &mut warnings)
            .map(|library| {
                let library = library.resolved(&usb_root);
                let playlists = (!library.playlists.is_empty()).then_some(library.playlists);
                (library.tracks, playlists, library.folders)
            })
            .unwrap_or_default();
        let edb_playlist_tracks = edb_playlists.as_ref().map(|m| {
            m.iter()
                .map(|(name, playlist)| (name.clone(), playlist.tracks.clone()))
                .collect::<HashMap<_, _>>()
        });
        let eedb_playlist_tracks_canonical = edb_playlist_tracks.as_ref().map(|m| {
            m.iter()
                .map(|(name, tracks)| (canonicalize_playlist_name(name), tracks.clone()))
                .collect::<HashMap<_, _>>()
        });
        push_usb_stage_timing_with_threshold(
            "read eDB",
            &mut stage_started,
            slow_stage_threshold_ms(edb_track_by_id.len()),
        );

        let mut track_by_id = HashMap::<u32, UsbTrack>::new();
        let mut entries_by_playlist =
            HashMap::<u32, Vec<crate::pdb_reader::PdbPlaylistEntryRow>>::new();
        let mut playlist_candidates = Vec::<PlaylistCandidate>::new();

        if let Some(parsed) = parsed {
            track_by_id = build_usb_track_index(parsed, &usb_root);

            entries_by_playlist =
                parsed
                    .playlist_entries
                    .iter()
                    .fold(HashMap::<u32, Vec<_>>::new(), |mut acc, e| {
                        acc.entry(e.playlist_id).or_default().push(*e);
                        acc
                    });

            let mut leaves = parsed
                .playlist_tree
                .iter()
                .filter(|n| !n.is_folder)
                .cloned()
                .collect::<Vec<_>>();
            leaves.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then(a.name.cmp(&b.name)));
            let tree = crate::pdb_reader::PlaylistTree::new(&parsed.playlist_tree);
            for node in leaves {
                // "Folder / Sub / Name": the eDB playlists are keyed the same way.
                let path = tree.path(&node);
                playlist_candidates.push(PlaylistCandidate {
                    pdb_id: Some(node.id),
                    short_name: node.name,
                    display_name: path.display(),
                    sort_order: node.sort_order,
                    path,
                });
            }
        }

        on_progress(
            50,
            100,
            &tr::usb_progress_resolving_edb_candidates().render(),
        );
        let mut seen = playlist_candidates
            .iter()
            .map(|c| canonicalize_playlist_name(&c.display_name))
            .collect::<HashSet<_>>();
        if let Some(map) = &edb_playlists {
            for (name, playlist) in map {
                let key = canonicalize_playlist_name(name);
                if seen.insert(key) {
                    playlist_candidates.push(PlaylistCandidate {
                        pdb_id: None,
                        short_name: name.clone(),
                        display_name: name.clone(),
                        // A reorder gives both databases one numbering.
                        sort_order: u32::try_from(playlist.sort_order).unwrap_or(0),
                        path: playlist.path.clone(),
                    });
                }
            }
        }
        playlist_candidates.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then(a.display_name.cmp(&b.display_name))
        });

        on_progress(70, 100, &tr::usb_progress_resolving_playlists().render());
        let mut items = Vec::new();
        let mut referenced_track_ids = HashSet::<u32>::new();
        let mut playlist_entries_total = 0usize;
        let mut source_counts = HashMap::<&'static str, usize>::new();
        let mut empty_source_playlists = Vec::<String>::new();
        // Each item's path and place, for the tree.
        let mut placed = HashMap::<String, (crate::pdb_reader::PlaylistPath, u32)>::new();
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
            let (playlist_tracks, source) = pick_playlist_tracks(&pdb_tracks, &export_tracks);
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

            let (total_duration_ms, duration_unknown_count) = duration_totals(&playlist_tracks);
            let id = candidate
                .pdb_id
                .map(|id| format!("usb-pl-{id}"))
                .unwrap_or_else(|| {
                    format!(
                        "usb-pl-name-{}",
                        canonicalize_playlist_name(&candidate.display_name)
                    )
                });
            placed
                .entry(id.clone())
                .or_insert_with(|| (candidate.path.clone(), candidate.sort_order));
            items.push(UsbPlaylist {
                id,
                name: candidate.display_name.clone(),
                source: source.to_string(),
                track_count: playlist_tracks.len(),
                // Built when answering: see `with_usb_playlist_texts`.
                selected_text: String::new(),
                remove_confirm_text: String::new(),
                tracks: playlist_tracks,
                total_duration_ms,
                duration_unknown_count,
                order_edited: false,
            });
        }
        push_usb_stage_timing("resolve playlists", &mut stage_started);
        let (deduped_items, collapsed) = dedupe_usb_playlists_by_name(items);
        items = deduped_items;
        if collapsed > 0 {
            logging::debug(
                "usb-import",
                format!(
                    "collapsed {} from USB sources",
                    counted(
                        collapsed,
                        "duplicate playlist name",
                        "duplicate playlist names"
                    )
                ),
            );
        }

        on_progress(
            90,
            100,
            &tr::usb_progress_finalizing_playlist_import().render(),
        );
        let stats = UsbImportStats {
            indexed_tracks: parsed.map(|p| p.tracks.len()).unwrap_or(0),
            playlist_referenced_tracks: referenced_track_ids.len(),
            playlist_entries: playlist_entries_total,
        };
        // Per-track materialization (local `tracks` rows, `track_usb_links`,
        // ANLZ cue import) is deferred out of the import: the add-to-playlist
        // path materializes on demand (browsing pages never writes). Importing every
        // track of every playlist here meant thousands of scattered on-USB
        // ANLZ reads -- minutes on an HDD-backed stick -- for data no unviewed,
        // unadded row ever uses.

        logging::debug(
            "usb-import",
            format!("USB root in use: {}", usb_root.display()),
        );

        if !source_counts.is_empty() {
            let pdb_count = source_counts.get("PDB").copied().unwrap_or(0);
            let edb_count = source_counts.get("eDB").copied().unwrap_or(0);
            if edb_count > 0 {
                logging::debug(
                    "usb-import",
                    format!(
                        "used eDB as playlist source for {}",
                        counted(edb_count, "playlist", "playlists")
                    ),
                );
            }
            if pdb_count > 0 {
                logging::debug(
                    "usb-import",
                    format!(
                        "used PDB as playlist source for {}",
                        counted(pdb_count, "playlist", "playlists")
                    ),
                );
            }
        }
        if !empty_source_playlists.is_empty() {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.playlists.empty-source",
                &tr::usb_playlists_warning_empty_source(
                    empty_source_playlists.len(),
                    empty_source_playlists.join(", "),
                ),
            ));
        }

        // The PDB's folders first: its places win, as its playlists' do.
        let pdb_folders = parsed.into_iter().flat_map(|parsed| {
            let tree = crate::pdb_reader::PlaylistTree::new(&parsed.playlist_tree);
            parsed
                .playlist_tree
                .iter()
                .filter(|row| row.is_folder)
                .map(|row| (tree.path(row), row.sort_order))
                .collect::<Vec<_>>()
        });
        let edb_folders = edb_folders
            .into_iter()
            .map(|(path, sort_order)| (path, u32::try_from(sort_order).unwrap_or(0)));
        let tree = super::playlist_tree::build_usb_tree(
            pdb_folders.chain(edb_folders),
            items.iter().filter_map(|item| {
                placed
                    .remove(&item.id)
                    .map(|(path, sort_order)| (item.id.clone(), path, sort_order))
            }),
        );
        let resolved = Arc::new(CachedUsbPlaylists {
            items,
            tree,
            stats,
            warnings,
        });
        if let Ok(mut guard) = self.usb_playlists_cache.lock() {
            *guard = Some((cache_key, Arc::clone(&resolved)));
        }
        self.finish_usb_playlists(&usb_root, &resolved)
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
        F: crate::progress::ProgressFn,
    {
        let mut warnings = Vec::<WarningEntry>::new();
        let mut stage_started = std::time::Instant::now();
        let name = req.playlist_name.trim().to_string();
        if name.is_empty() {
            return Err(BackendError::Internal(
                "playlistName must not be empty".to_string(),
            ));
        }

        // Stage 1: Resolve USB root, name candidates (0-10%)
        on_progress(0, 100, &tr::usb_progress_resolving_usb_root().render());
        let _usb_write_guard = self.lock_usb_writes();
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        push_usb_stage_timing("resolve usb root", &mut stage_started);

        let mut session = self.usb_write_session(
            &_usb_write_guard,
            &usb_root,
            // Removal can change the count, so it's worth a fresh
            // (best-effort) parse rather than carrying the previous
            // backup's count forward.
            crate::service::usb::staging::SessionBackup::CountingPlaylists {
                reason: "Before playlist removal",
            },
            &mut warnings,
        );
        push_usb_stage_timing("backup usb databases", &mut stage_started);

        on_progress(
            5,
            100,
            &tr::usb_progress_resolving_playlist_identifiers().render(),
        );
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
        push_usb_stage_timing("resolve playlist identifiers", &mut stage_started);

        // Stage 2: Remove playlist + detect shared tracks in PDB (10-30%)
        on_progress(
            10,
            100,
            &tr::usb_progress_analyzing_shared_tracks().render(),
        );
        // Without a PDB, the eDB says which tracks only this playlist holds.
        let pdb_result = if session.databases()?.pdb().is_some() {
            remove_playlist_and_tracks_from_pdb_in(
                &mut session,
                req.playlist_id.as_deref(),
                &name_candidates,
                &mut warnings,
            )?
        } else {
            edb_playlist_exclusive_tracks(&mut session, &name_candidates)?
        };
        push_usb_stage_timing("remove playlist and tracks from PDB", &mut stage_started);

        let removed_pdb = pdb_result.removed_playlist_count;
        let tracks_removed = pdb_result.exclusive_tracks.len();
        let tracks_kept_shared = pdb_result.shared_track_count;

        if tracks_kept_shared > 0 {
            warnings.push(logging::log_text(
                Level::Info,
                "usb-import",
                "usb.remove.shared-tracks-preserved",
                &tr::usb_playlists_warning_shared_tracks_kept(tracks_kept_shared),
            ));
        }

        // Stage 3: Collect the files to delete after the database commit (30-65%)
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

        for art_path in &pdb_result.exclusive_artwork_paths {
            stale_paths.extend(artwork_size_variants(art_path));
        }
        // Artwork files for exclusive artwork_ids
        let exclusive_artwork_ids: std::collections::HashSet<u32> = pdb_result
            .exclusive_tracks
            .iter()
            .filter(|t| t.artwork_id != 0)
            .map(|t| t.artwork_id)
            .collect();
        // Read artwork paths from parsed PDB artworks map
        if !exclusive_artwork_ids.is_empty()
            && let Some(pdb_path) = session.databases()?.pdb()
            && let Ok(staged) = self.parsed_staged_pdb(pdb_path)
            && let parsed = &staged.parsed
        {
            for art_id in &exclusive_artwork_ids {
                if let Some(art_path) = parsed.artworks.get(art_id) {
                    stale_paths.extend(artwork_size_variants(art_path));
                }
            }
        }

        // Stage 4: Remove from eDB (65-80%)
        on_progress(65, 100, &tr::usb_progress_cleaning_edb().render());
        let removed_edb = remove_playlist_from_edb(
            &mut session,
            &name_candidates,
            &exclusive_track_file_paths,
            &mut warnings,
        )?;
        push_usb_stage_timing("remove playlist from eDB", &mut stage_started);

        if removed_edb == 0 && removed_pdb == 0 {
            return Err(BackendError::NotFound(tr::error_usb_playlist_not_found(
                &name,
            )));
        }
        session.commit()?;

        on_progress(
            75,
            100,
            &tr::usb_progress_deleting_exclusive_track_files().render(),
        );
        let mut files_deleted = 0usize;
        if !stale_paths.is_empty() {
            let prune_result = prune_stale_export_owned_files(&usb_root, &stale_paths)?;
            files_deleted = prune_result.removed;
            if prune_result.missing > 0 {
                warnings.push(logging::log_text(
                    Level::Warn,
                    "usb-import",
                    "usb.remove.file-missing",
                    &tr::usb_playlists_warning_files_missing(prune_result.missing),
                ));
            }
            for path in &stale_paths {
                warnings.push(logging::log_text(
                    Level::Info,
                    "usb-import",
                    "usb.remove.file-deleted",
                    &tr::usb_playlists_warning_file_deleted(path),
                ));
            }
        }
        push_usb_stage_timing("delete exclusive track files", &mut stage_started);

        // Stage 5: Clean up empty directories under Contents/ (80-95%)
        on_progress(
            80,
            100,
            &tr::usb_progress_cleaning_empty_directories().render(),
        );
        let contents_dir = usb_root.join("Contents");
        if contents_dir.is_dir() {
            cleanup_empty_dirs_recursive(&contents_dir);
        }
        push_usb_stage_timing("cleanup empty directories", &mut stage_started);

        // Stage 6: Finalize (95-100%)
        on_progress(95, 100, &tr::usb_progress_finalizing().render());
        warnings.extend(self.refresh_engine_library(&usb_root));
        push_usb_stage_timing("finalize playlist removal", &mut stage_started);
        on_progress(
            100,
            100,
            &tr::usb_progress_playlist_removal_completed().render(),
        );

        let warning_count = logging::count_warnings(&warnings);
        Ok(RemoveUsbPlaylistData {
            status_text: tr::usb_playlists_removed(&name, removed_edb, removed_pdb).render(),
            playlist_name: name,
            removed_from_edb: removed_edb,
            removed_from_pdb: removed_pdb,
            tracks_removed,
            files_deleted,
            tracks_kept_shared,
            warning_text: warning_text(warning_count),
            warnings,
        })
    }

    /// One paginated/searched/sorted page of a USB playlist's tracks, with the
    /// waveform-preview bytes + artwork data URLs hydrated server-side for that
    /// page. The playlist list is resolved from the parse cache (or once, on a
    /// cache miss); only the returned page pays the per-track hydration I/O.
    /// Read-only: it never writes to the local DB.
    pub fn fetch_usb_playlist_tracks(
        &self,
        req: FetchUsbTracksRequest,
    ) -> BackendResult<FetchUsbTracksData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        logging::debug(
            "usb-import",
            format!("fetch_usb_playlist_tracks: loading {}", req.id),
        );
        let generation = Arc::clone(&self.usb_playlist_page_generation);
        let this_load = generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let resolve_started = std::time::Instant::now();
        let resolved = self.usb_playlists_resolved(&usb_root)?;
        let resolve_ms = resolve_started.elapsed().as_millis();
        if resolve_ms >= 50 {
            logging::debug(
                "usb-import",
                format!("USB playlist list took {resolve_ms}ms to resolve"),
            );
        }
        let playlist = resolved
            .items
            .iter()
            .find(|playlist| playlist.id == req.id)
            .ok_or_else(|| BackendError::NotFound(tr::error_usb_playlist_not_found(&req.id)))?;
        let mut tracks = playlist.tracks.clone();
        if let Some(order) = self.pending_usb_track_order(&usb_root, &req.id) {
            crate::service::usb::order_edit::apply_pending_order(
                &mut tracks,
                &order,
                crate::service::usb::order_edit::track_order_key,
            );
        }
        let mut data = paginate_usb_tracks(
            "fetch_usb_playlist_tracks",
            &req.id,
            tracks,
            &req,
            resolved.warnings.clone(),
        )?;
        // Local ids first: a matched track's waveform + artwork come from the
        // library's own files instead of the stick.
        let files = self.resolve_usb_track_page_local_ids(&mut data.items, &usb_root)?;
        hydrate_usb_page(
            "fetch_usb_playlist_tracks",
            &mut data.items,
            self.key_notation()?,
            &files,
            || generation.load(std::sync::atomic::Ordering::SeqCst) != this_load,
        );
        Ok(data)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PlaylistCandidate {
    pub(crate) pdb_id: Option<u32>,
    pub(crate) short_name: String,
    pub(crate) display_name: String,
    pub(crate) sort_order: u32,
    /// Its folders and name.
    pub(crate) path: crate::pdb_reader::PlaylistPath,
}

pub(crate) fn lookup_playlist_tracks<'a>(
    map: &'a Option<HashMap<String, Vec<UsbTrack>>>,
    map_canonical: &'a Option<HashMap<String, Vec<UsbTrack>>>,
    short_name: &str,
    display_name: &str,
) -> Option<&'a Vec<UsbTrack>> {
    // The path first: a bare name may be another folder's playlist.
    map.as_ref()
        .and_then(|m| {
            m.get(display_name)
                .or_else(|| m.get(display_name.trim()))
                .or_else(|| m.get(short_name))
                .or_else(|| m.get(short_name.trim()))
        })
        .or_else(|| {
            let key = canonicalize_playlist_name(display_name);
            map_canonical.as_ref().and_then(|m| m.get(&key))
        })
        .or_else(|| {
            let key = canonicalize_playlist_name(short_name);
            map_canonical.as_ref().and_then(|m| m.get(&key))
        })
}

/// A USB playlist's tracks, from one database: the eDB's list when it has
/// any, else the PDB's. Returns the list with its source ("eDB", "PDB" or
/// "none"). A repeated record ID keeps its first entry.
pub(crate) fn pick_playlist_tracks(
    pdb_tracks: &[UsbTrack],
    edb_tracks: &[UsbTrack],
) -> (Vec<UsbTrack>, &'static str) {
    let (tracks, source) = if !edb_tracks.is_empty() {
        (edb_tracks, "eDB")
    } else if !pdb_tracks.is_empty() {
        (pdb_tracks, "PDB")
    } else {
        return (Vec::new(), "none");
    };
    let mut seen = HashSet::<String>::new();
    let picked = tracks
        .iter()
        .filter(|track| track_merge_key(track).is_none_or(|key| seen.insert(key)))
        .cloned()
        .collect();
    (picked, source)
}

pub(crate) fn playlist_source_rank(source: &str) -> usize {
    match source {
        "PDB" => 3,
        "eDB" => 2,
        _ => 1,
    }
}

pub(crate) fn dedupe_usb_playlists_by_name(items: Vec<UsbPlaylist>) -> (Vec<UsbPlaylist>, usize) {
    let mut out = Vec::<UsbPlaylist>::new();
    let mut by_key = HashMap::<String, usize>::new();
    let mut collapsed = 0usize;
    for item in items {
        let key = canonicalize_playlist_name(&item.name);
        if let Some(existing_idx) = by_key.get(&key).copied() {
            let existing = &out[existing_idx];
            let existing_score = (existing.track_count, playlist_source_rank(&existing.source));
            let incoming_score = (item.track_count, playlist_source_rank(&item.source));
            if incoming_score > existing_score {
                out[existing_idx] = item;
            }
            collapsed += 1;
        } else {
            by_key.insert(key, out.len());
            out.push(item);
        }
    }
    (out, collapsed)
}

/// The USB record ID identifies a playlist entry: a track is in a playlist at
/// most once. The file path does not: distinct tracks can have file names
/// that differ only by punctuation. Metadata alone does not either: tracks
/// with the same title and artist can be distinct encodings.
pub(crate) fn track_merge_key(track: &UsbTrack) -> Option<String> {
    let id = track.id.trim();
    (!id.is_empty()).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_track(id: &str, title: &str, artist: &str, file_path: &str) -> UsbTrack {
        UsbTrack {
            id: id.to_string(),
            local_track_id: None,
            title: title.to_string(),
            artist: artist.to_string(),
            album: None,
            year: None,
            genre: None,
            track_number: None,
            bpm: None,
            key: None,
            key_display: None,
            key_color: None,
            file_path: file_path.to_string(),
            format_ext: crate::audio_format::format_ext_from_path(file_path),
            needs_hydration: false,
            format_compat: Default::default(),
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

    fn make_playlist(name: &str, source: &str, track_count: usize) -> UsbPlaylist {
        UsbPlaylist {
            id: format!("pl-{name}"),
            name: name.to_string(),
            source: source.to_string(),
            track_count,
            selected_text: String::new(),
            remove_confirm_text: String::new(),
            tracks: Vec::new(),
            total_duration_ms: 0,
            duration_unknown_count: 0,
            order_edited: false,
        }
    }

    #[test]
    fn source_rank_ordering() {
        assert_eq!(playlist_source_rank("PDB"), 3);
        assert_eq!(playlist_source_rank("eDB"), 2);
        assert_eq!(playlist_source_rank("other"), 1);
        assert_eq!(playlist_source_rank(""), 1);
    }

    #[test]
    fn pick_prefers_export_over_pdb() {
        let pdb = vec![make_track("1", "A", "X", "/a.mp3")];
        let export = vec![make_track("1", "A Better", "X", "/a.mp3")];

        let (merged, source) = pick_playlist_tracks(&pdb, &export);
        assert_eq!(source, "eDB");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].title, "A Better");
    }

    #[test]
    fn pick_export_source_does_not_append_same_name_pdb_tracks() {
        let pdb = vec![make_track("1", "Old USB Track", "X", "/old.mp3")];
        let export = vec![make_track("2", "New Export Track", "Y", "/new.mp3")];

        let (merged, source) = pick_playlist_tracks(&pdb, &export);
        assert_eq!(source, "eDB");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].title, "New Export Track");
        assert_eq!(merged[0].file_path, "/new.mp3");
    }

    #[test]
    fn pick_export_source_returns_only_export_set() {
        let pdb = vec![
            make_track("1", "Track A", "X", "/a.mp3"),
            make_track("2", "Track B", "Y", "/b.mp3"),
        ];
        let export = vec![make_track("1", "Track A Export", "X", "/a.mp3")];

        let (merged, source) = pick_playlist_tracks(&pdb, &export);
        assert_eq!(source, "eDB");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].title, "Track A Export");
    }

    #[test]
    fn pick_deduplicates_same_path_when_ids_differ() {
        let pdb = vec![make_track("1001", "Track A", "X", "/a.mp3")];
        let export = vec![make_track("2002", "Track A Export", "X", "/a.mp3")];

        let (merged, source) = pick_playlist_tracks(&pdb, &export);
        assert_eq!(source, "eDB");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "2002");
    }

    #[test]
    fn pick_keeps_distinct_tracks_whose_paths_differ_only_by_punctuation() {
        let export = vec![
            make_track("1", "Live Vol. 1", "X", "/Contents/X/Live - Vol. 1.mp3"),
            make_track("2", "Live Vol-1", "X", "/Contents/X/Live - Vol-1.mp3"),
        ];

        let (merged, _) = pick_playlist_tracks(&[], &export);
        assert_eq!(
            merged.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["1", "2"]
        );
    }

    #[test]
    fn pick_collapses_repeated_record_id() {
        let export = vec![
            make_track("1", "Track A", "X", "/a.mp3"),
            make_track("1", "Track A", "X", "/a.mp3"),
        ];

        let (merged, _) = pick_playlist_tracks(&[], &export);
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn pick_pdb_only() {
        let pdb = vec![make_track("1", "Track A", "X", "/a.mp3")];
        let export: Vec<UsbTrack> = vec![];

        let (merged, source) = pick_playlist_tracks(&pdb, &export);
        assert_eq!(source, "PDB");
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn pick_without_path_preserves_distinct_record_ids_and_unknown_entries() {
        let pdb = vec![
            make_track("1", "Song", "Artist", ""),
            make_track("2", "Song", "Artist", ""),
            make_track("", "Song", "Artist", ""),
            make_track("", "Song", "Artist", ""),
            make_track("1", "Song", "Artist", ""),
        ];

        let (merged, source) = pick_playlist_tracks(&pdb, &[]);
        assert_eq!(source, "PDB");
        assert_eq!(merged.len(), 4);
        assert_eq!(
            merged.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["1", "2", "", ""]
        );
    }

    #[test]
    fn pick_all_empty() {
        let (merged, source) = pick_playlist_tracks(&[], &[]);
        assert_eq!(source, "none");
        assert!(merged.is_empty());
    }

    #[test]
    fn dedupe_keeps_higher_track_count() {
        let items = vec![
            make_playlist("My Set", "PDB", 5),
            make_playlist("My Set", "PDB", 10),
        ];
        let (deduped, collapsed) = dedupe_usb_playlists_by_name(items);
        assert_eq!(deduped.len(), 1);
        assert_eq!(collapsed, 1);
        assert_eq!(deduped[0].track_count, 10);
    }

    #[test]
    fn dedupe_keeps_higher_rank_on_same_count() {
        let items = vec![
            make_playlist("My Set", "none", 5),
            make_playlist("My Set", "PDB", 5),
        ];
        let (deduped, collapsed) = dedupe_usb_playlists_by_name(items);
        assert_eq!(deduped.len(), 1);
        assert_eq!(collapsed, 1);
        assert_eq!(deduped[0].source, "PDB");
    }

    #[test]
    fn dedupe_different_names_kept() {
        let items = vec![
            make_playlist("Set A", "PDB", 5),
            make_playlist("Set B", "PDB", 3),
        ];
        let (deduped, collapsed) = dedupe_usb_playlists_by_name(items);
        assert_eq!(deduped.len(), 2);
        assert_eq!(collapsed, 0);
    }

    #[test]
    fn lookup_by_short_name() {
        let tracks = vec![make_track("1", "T", "A", "/t.mp3")];
        let map = Some(HashMap::from([("my_set".to_string(), tracks.clone())]));
        let result = lookup_playlist_tracks(&map, &None, "my_set", "display");
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 1);
    }

    #[test]
    fn lookup_by_display_name() {
        let tracks = vec![make_track("1", "T", "A", "/t.mp3")];
        let map = Some(HashMap::from([("My Display".to_string(), tracks.clone())]));
        let result = lookup_playlist_tracks(&map, &None, "short", "My Display");
        assert!(result.is_some());
    }

    #[test]
    fn lookup_falls_back_to_canonical() {
        let tracks = vec![make_track("1", "T", "A", "/t.mp3")];
        let canonical = Some(HashMap::from([(
            canonicalize_playlist_name("my set"),
            tracks.clone(),
        )]));
        let result = lookup_playlist_tracks(&None, &canonical, "my set", "other");
        assert!(result.is_some());
    }

    #[test]
    fn lookup_returns_none_when_no_match() {
        let result = lookup_playlist_tracks(&None, &None, "none", "none");
        assert!(result.is_none());
    }
}
