//! Local playlists and imported-playlist bookkeeping.

use super::*;

pub(super) fn track_id_exists(conn: &rusqlite::Connection, track_id: &str) -> BackendResult<bool> {
    let found = conn
        .query_row(
            "SELECT id FROM tracks WHERE id = ?1 LIMIT 1",
            params![track_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(found.is_some())
}

pub(super) fn add_candidate_is_usb_origin(
    candidate: &AddTrackCandidate,
    request_usb_root: Option<&str>,
) -> bool {
    let usb_analysis_path = candidate.usb_analysis_path.as_deref().unwrap_or("").trim();
    if !usb_analysis_path.is_empty() {
        return true;
    }

    let file_path = candidate.file_path.as_deref().unwrap_or("").trim();
    let usb_root = candidate
        .usb_root
        .as_deref()
        .or(request_usb_root)
        .unwrap_or("")
        .trim();
    !file_path.is_empty() && !usb_root.is_empty() && browse_path_matches_root(file_path, usb_root)
}

/// Create a local playlist holding `track_ids` in order; returns its id.
pub(crate) fn insert_local_playlist(
    conn: &rusqlite::Connection,
    name: &str,
    track_ids: &[String],
) -> BackendResult<String> {
    let id = Uuid::now_v7().to_string();
    let now = now();
    conn.execute(
        "INSERT INTO playlists (id, name, source, last_exported_at, last_exported_usb_root, last_exported_track_count, created_at, updated_at) VALUES (?1, ?2, 'local', NULL, NULL, NULL, ?3, ?3)",
        params![id, name, now],
    )?;
    for (index, track_id) in track_ids.iter().enumerate() {
        conn.execute(
            "INSERT INTO playlist_tracks (id, playlist_id, track_id, position, added_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![Uuid::now_v7().to_string(), id, track_id, index as i64 + 1, now],
        )?;
    }
    Ok(id)
}

/// The `playlists.import_source` of a list imported from an external library,
/// e.g. `mixxx:crate:4` (Mixxx playlist and crate ids overlap).
pub(crate) fn external_playlist_key(library: &str, kind: ExternalPlaylistKind, id: &str) -> String {
    let kind = match kind {
        ExternalPlaylistKind::Playlist => "playlist",
        ExternalPlaylistKind::Crate => "crate",
        ExternalPlaylistKind::History => "history",
    };
    format!("{library}:{kind}:{}", id.trim())
}

/// The library name shown for a playlist's `import_source` (see
/// [`external_playlist_key`]).
pub(super) fn imported_from_label(import_source: &str) -> Option<&'static str> {
    match import_source.split(':').next()? {
        "mixxx" => Some("Mixxx"),
        "rekordbox" => Some("rekordbox"),
        _ => None,
    }
}

/// The local playlist an earlier import of `key` made, if it still exists.
pub(crate) fn find_imported_playlist(
    conn: &rusqlite::Connection,
    key: &str,
) -> BackendResult<Option<ImportedPlaylistRef>> {
    Ok(conn
        .query_row(
            "SELECT id, name FROM playlists WHERE import_source = ?1 ORDER BY created_at LIMIT 1",
            params![key],
            |row| {
                Ok(ImportedPlaylistRef {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            },
        )
        .optional()?)
}

/// Mark each listed external playlist that was imported before.
pub(crate) fn annotate_imported_playlists(
    conn: &rusqlite::Connection,
    library: &str,
    items: &mut [ExternalPlaylistSummary],
) -> BackendResult<()> {
    for item in items {
        item.existing_playlist =
            find_imported_playlist(conn, &external_playlist_key(library, item.kind, &item.id))?;
    }
    Ok(())
}

/// Save an imported list's tracks: into the playlist an earlier import of
/// `key` made (its tracks and order replaced, its name kept, its USB export
/// status cleared), or into a new playlist named `name`. Returns the
/// playlist, and whether it was an existing one.
pub(crate) fn save_imported_playlist(
    conn: &rusqlite::Connection,
    key: &str,
    name: &str,
    track_ids: &[String],
) -> BackendResult<(ImportedPlaylistRef, bool)> {
    if let Some(existing) = find_imported_playlist(conn, key)? {
        conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            params![existing.id],
        )?;
        let now = now();
        for (index, track_id) in track_ids.iter().enumerate() {
            conn.execute(
                "INSERT INTO playlist_tracks (id, playlist_id, track_id, position, added_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![Uuid::now_v7().to_string(), existing.id, track_id, index as i64 + 1, now],
            )?;
        }
        conn.execute(
            "UPDATE playlists SET updated_at = ?1, last_exported_at = NULL, last_exported_usb_root = NULL, last_exported_track_count = NULL WHERE id = ?2",
            params![now, existing.id],
        )?;
        return Ok((existing, true));
    }
    let id = insert_local_playlist(conn, name, track_ids)?;
    conn.execute(
        "UPDATE playlists SET import_source = ?1 WHERE id = ?2",
        params![key, id],
    )?;
    Ok((
        ImportedPlaylistRef {
            id,
            name: name.to_string(),
        },
        false,
    ))
}

/// An external playlist import that found none of its tracks: failing beats
/// creating an empty playlist.
pub(crate) fn no_importable_tracks_error(name: &str, listed: usize) -> BackendError {
    BackendError::Validation(format!(
        "none of the {listed} track(s) in {name:?} could be imported: files not found on this computer or not supported"
    ))
}

impl BackendService {
    pub fn create_playlist(&self, req: CreatePlaylistRequest) -> BackendResult<CreatePlaylistData> {
        let name = req.name.trim();
        if name.is_empty() {
            return Err(BackendError::Validation(
                "playlist name must not be empty".to_string(),
            ));
        }

        let conn = self.db.connect()?;
        let id = insert_local_playlist(&conn, name, &[])?;

        Ok(CreatePlaylistData {
            playlist_id: id,
            name: name.to_string(),
        })
    }

    pub fn rename_playlist(&self, req: RenamePlaylistRequest) -> BackendResult<RenamePlaylistData> {
        let name = req.name.trim();
        if name.is_empty() {
            return Err(BackendError::Validation(
                "playlist name must not be empty".to_string(),
            ));
        }

        ensure_playlist_exists(&self.db, &req.playlist_id)?;

        let now = now();
        let conn = self.db.connect()?;
        conn.execute(
            "UPDATE playlists SET name = ?1, last_exported_at = NULL, last_exported_usb_root = NULL, last_exported_track_count = NULL, updated_at = ?2 WHERE id = ?3",
            params![name, now, req.playlist_id],
        )?;

        Ok(RenamePlaylistData {
            playlist_id: req.playlist_id,
            name: name.to_string(),
        })
    }

    pub fn delete_playlist(&self, req: DeletePlaylistRequest) -> BackendResult<DeletePlaylistData> {
        ensure_playlist_exists(&self.db, &req.playlist_id)?;

        let conn = self.db.connect()?;
        let deleted = conn.execute(
            "DELETE FROM playlists WHERE id = ?1",
            params![req.playlist_id.clone()],
        )?;

        Ok(DeletePlaylistData {
            playlist_id: req.playlist_id,
            deleted: deleted > 0,
        })
    }

    pub fn list_playlists(&self) -> BackendResult<ListPlaylistsData> {
        let conn = self.db.connect()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, name, source, last_exported_at, last_exported_usb_root, last_exported_track_count, created_at, updated_at, import_source
            FROM playlists
            ORDER BY created_at ASC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(Playlist {
                id: row.get(0)?,
                name: row.get(1)?,
                source: row.get(2)?,
                last_exported_at: row.get(3)?,
                last_exported_usb_root: row.get(4)?,
                last_exported_track_count: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                imported_from: row
                    .get::<_, Option<String>>(8)?
                    .as_deref()
                    .and_then(imported_from_label)
                    .map(str::to_string),
                // Filled in below (needs a lookup per exported playlist).
                last_exported_usb_name: None,
            })
        })?;

        let mut items = rows.collect::<Result<Vec<_>, _>>()?;
        for playlist in &mut items {
            playlist.last_exported_usb_name = playlist
                .last_exported_usb_root
                .as_deref()
                .and_then(|root| usb_drive_name(&conn, Path::new(root.trim())));
        }
        Ok(ListPlaylistsData { items })
    }

    /// The playlist's tracks as local library rows, in playlist position order.
    /// Shared by `get_playlist_tracks` and the playlist-scoped analyze path so
    /// both see the exact same set (USB-only entries with no local row are
    /// dropped by the JOIN in both).
    pub(crate) fn load_playlist_tracks_ordered(
        &self,
        conn: &rusqlite::Connection,
        playlist_id: &str,
    ) -> BackendResult<Vec<Track>> {
        // Column order MUST stay in lockstep with `TRACK_COLS` / `row_to_track`
        // (which reads `rekordbox_db_source` positionally at index 19) -- this JOIN
        // needs `t.`-prefixed names so it can't reuse `TRACK_COLS` verbatim.
        let mut stmt = conn.prepare(
            r#"
            SELECT t.id, t.title, t.artist, t.album, t.track_number, t.bpm, t.tonality, t.file_path,
                   t.file_size_bytes, t.format_ext, t.sample_rate_hz, t.bit_depth, t.bitrate_kbps, t.duration_ms,
                   t.artwork_path, t.waveform_peaks_path, t.bpm_analyzer, t.created_at, t.updated_at,
                   COALESCE(t.rekordbox_db_source, 0) AS rekordbox_db_source, t.wav_extensible_kind,
                   COALESCE(t.mixxx_db_source, 0) AS mixxx_db_source
            FROM playlist_tracks pt
            JOIN tracks t ON t.id = pt.track_id
            WHERE pt.playlist_id = ?1
            ORDER BY pt.position ASC
            "#,
        )?;
        let rows = stmt.query_map(params![playlist_id], |row| row_to_track(row, true))?;
        let mut items = rows.collect::<Result<Vec<_>, _>>()?;
        apply_frontend_track_fields(conn, &mut items)?;
        Ok(items)
    }

    /// Local track ids in `playlist_id` that still need core analysis, honoring
    /// the same `query` filter `get_playlist_tracks` applies to its
    /// `unanalyzed_count`. Playlist position order.
    pub(crate) fn collect_playlist_unanalyzed_track_ids(
        &self,
        conn: &rusqlite::Connection,
        playlist_id: &str,
        query: &str,
    ) -> BackendResult<Vec<String>> {
        let items = self.load_playlist_tracks_ordered(conn, playlist_id)?;
        let query = query.trim().to_lowercase();
        Ok(items
            .into_iter()
            .filter(|track| query.is_empty() || track_matches_query(track, &query))
            .filter(|track| !track.analysis_ready)
            .map(|track| track.id)
            .collect())
    }

    pub fn get_playlist_tracks(
        &self,
        req: GetPlaylistTracksRequest,
    ) -> BackendResult<GetPlaylistTracksData> {
        ensure_playlist_exists(&self.db, &req.playlist_id)?;

        let conn = self.db.connect()?;
        let mut items = self.load_playlist_tracks_ordered(&conn, &req.playlist_id)?;

        let query = req.query.trim().to_lowercase();
        if !query.is_empty() {
            items.retain(|track| track_matches_query(track, &query));
        }

        // Computed once server-side (here) over the whole filtered playlist, not
        // just the returned page -- see `GetPlaylistTracksData::total_duration_ms`.
        let mut total_duration_ms: u64 = 0;
        let mut duration_known_count: usize = 0;
        let mut unanalyzed_count: usize = 0;
        for track in &items {
            if let Some(d) = track.duration_ms
                && d > 0
            {
                total_duration_ms += d;
                duration_known_count += 1;
            }
            if !track.analysis_ready {
                unanalyzed_count += 1;
            }
        }
        let total = items.len();

        // Default order is playlist position (the SQL `ORDER BY`); a client
        // column sort overrides it, applied server-side so it spans the whole
        // playlist.
        sort_tracks(&mut items, req.sort_by.as_deref(), req.sort_dir.as_deref());

        let sort_by = req.sort_by.as_deref().map(str::trim).unwrap_or_default();
        let sort_dir = req.sort_dir.as_deref().map(str::trim).unwrap_or_default();
        let signature = build_track_cursor_signature(&[
            TRACK_CURSOR_VERSION,
            "get_playlist_tracks",
            &req.playlist_id,
            &query,
            sort_by,
            sort_dir,
        ]);
        let cursor = decode_track_page_cursor(req.cursor.as_deref(), &signature)?;
        let start_idx = cursor_start_index(&items, cursor.as_ref());
        // `limit == 0` ⇒ return the whole (filtered) playlist, for callers that
        // predate pagination.
        let limit = if req.limit == 0 {
            items.len().saturating_sub(start_idx).max(1)
        } else {
            req.limit.clamp(1, TRACK_QUERY_LIMIT_MAX)
        };
        let mut page: Vec<Track> = items.into_iter().skip(start_idx).take(limit + 1).collect();
        let (has_more, next_cursor) = paginate_tracks(&mut page, limit, &signature);

        Ok(GetPlaylistTracksData {
            playlist_id: req.playlist_id,
            items: page,
            total,
            next_cursor,
            has_more,
            total_duration_ms,
            duration_known_count,
            unanalyzed_count,
        })
    }

    pub fn add_tracks_to_playlist(
        &self,
        req: AddTracksToPlaylistRequest,
    ) -> BackendResult<AddTracksToPlaylistData> {
        if req.track_ids.is_empty() {
            return Err(BackendError::Validation(
                "trackIds must contain at least one id".to_string(),
            ));
        }

        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        ensure_playlist_exists_conn(&tx, &req.playlist_id)?;

        let mut next_position: i64 = tx.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM playlist_tracks WHERE playlist_id = ?1",
            params![req.playlist_id],
            |row| row.get(0),
        )?;

        let mut added = 0usize;
        let mut skipped = 0usize;
        let now = now();

        // Pre-load existing track IDs to avoid N+1 queries
        let mut existing_track_ids = std::collections::HashSet::new();
        {
            let mut stmt = tx.prepare("SELECT id FROM tracks")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                existing_track_ids.insert(row?);
            }
        }

        // Pre-load playlist track IDs for dedupe check
        let mut playlist_track_ids = std::collections::HashSet::new();
        if matches!(req.dedupe, DedupeMode::Skip) {
            let mut stmt =
                tx.prepare("SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1")?;
            let rows = stmt.query_map(params![req.playlist_id], |row| row.get::<_, String>(0))?;
            for row in rows {
                playlist_track_ids.insert(row?);
            }
        }

        for track_id in &req.track_ids {
            if !existing_track_ids.contains(track_id.as_str()) {
                return Err(BackendError::NotFound(format!(
                    "track not found: {track_id}"
                )));
            }

            if matches!(req.dedupe, DedupeMode::Skip)
                && playlist_track_ids.contains(track_id.as_str())
            {
                skipped += 1;
                continue;
            }

            tx.execute(
                r#"
                INSERT INTO playlist_tracks (id, playlist_id, track_id, position, added_at)
                VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                params![
                    Uuid::now_v7().to_string(),
                    req.playlist_id,
                    track_id,
                    next_position,
                    now
                ],
            )?;

            if matches!(req.dedupe, DedupeMode::Skip) {
                playlist_track_ids.insert(track_id.clone());
            }

            added += 1;
            next_position += 1;
        }

        tx.execute(
            "UPDATE playlists SET updated_at = ?1, last_exported_at = NULL, last_exported_usb_root = NULL, last_exported_track_count = NULL WHERE id = ?2",
            params![now, req.playlist_id],
        )?;

        tx.commit()?;

        Ok(AddTracksToPlaylistData {
            playlist_id: req.playlist_id,
            added,
            skipped,
        })
    }

    pub fn list_matching_track_ids(
        &self,
        req: ListMatchingTrackIdsRequest,
    ) -> BackendResult<ListMatchingTrackIdsData> {
        let conn = self.db.connect()?;
        let visible = self.compute_visible_library_tracks(
            &conn,
            &req.source_roots,
            ExternalLibraries::from_flags(req.include_rekordbox_db, req.include_mixxx_db),
            &req.query,
        )?;
        let track_ids: Vec<String> = visible.into_iter().map(|track| track.id).collect();
        let total = track_ids.len();
        Ok(ListMatchingTrackIdsData { track_ids, total })
    }

    pub fn add_library_selection_to_playlist(
        &self,
        req: AddLibrarySelectionToPlaylistRequest,
    ) -> BackendResult<AddTracksToPlaylistData> {
        let conn = self.db.connect()?;
        ensure_playlist_exists_conn(&conn, &req.playlist_id)?;

        let visible = self.compute_visible_library_tracks(
            &conn,
            &req.source_roots,
            ExternalLibraries::from_flags(req.include_rekordbox_db, req.include_mixxx_db),
            &req.query,
        )?;

        let wanted: std::collections::HashSet<&str> =
            req.track_ids.iter().map(String::as_str).collect();
        let chosen: Vec<&Track> = visible
            .iter()
            .filter(|track| req.all_matching || wanted.contains(track.id.as_str()))
            .collect();

        // Every visible row is either a real `tracks` row or a browse-only file
        // that isn't indexed yet (`id == file_path`); materialize the latter so
        // the playlist references a stable local id, matching what the
        // single-row "+" does.
        let mut track_ids = Vec::<String>::with_capacity(chosen.len());
        for track in chosen {
            if track_id_exists(&conn, &track.id)? {
                track_ids.push(track.id.clone());
                continue;
            }
            let materialized = self.materialize_source_track(MaterializeSourceTrackRequest {
                file_path: track.file_path.clone(),
                title: track.title.clone(),
                artist: track.artist.clone(),
                album: track.album.clone(),
                track_number: track.track_number,
                key: track.key.clone(),
                file_size_bytes: track.file_size_bytes,
                format_ext: track.format_ext.clone(),
                sample_rate_hz: track.sample_rate_hz,
                bit_depth: track.bit_depth,
                bitrate_kbps: track.bitrate_kbps,
            })?;
            track_ids.push(materialized.track_id);
        }
        drop(conn);

        if track_ids.is_empty() {
            return Ok(AddTracksToPlaylistData {
                playlist_id: req.playlist_id,
                added: 0,
                skipped: 0,
            });
        }

        self.add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: req.playlist_id,
            track_ids,
            dedupe: req.dedupe,
        })
    }

    pub fn add_track_candidates_to_playlist(
        &self,
        req: AddTrackCandidatesToPlaylistRequest,
    ) -> BackendResult<AddTrackCandidatesToPlaylistData> {
        let conn = self.db.connect()?;
        let requested = req.tracks.len();
        let mut track_ids = Vec::<String>::new();
        let mut resolutions = Vec::<AddTrackCandidateResolution>::with_capacity(requested);

        for candidate in &req.tracks {
            let resolution = self.resolve_add_track_candidate(
                &conn,
                candidate,
                req.usb_root.as_deref(),
                req.usb_root_valid,
            )?;
            if let Some(track_id) = resolution.track_id.as_ref() {
                track_ids.push(track_id.clone());
            }
            resolutions.push(resolution);
        }
        drop(conn);

        let resolved = track_ids.len();
        let unresolved = requested.saturating_sub(resolved);
        if track_ids.is_empty() {
            return Ok(AddTrackCandidatesToPlaylistData {
                playlist_id: req.playlist_id,
                requested,
                resolved,
                unresolved,
                added: 0,
                skipped: 0,
                resolutions,
            });
        }

        let added = self.add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: req.playlist_id,
            track_ids,
            dedupe: req.dedupe,
        })?;

        Ok(AddTrackCandidatesToPlaylistData {
            playlist_id: added.playlist_id,
            requested,
            resolved,
            unresolved,
            added: added.added,
            skipped: added.skipped,
            resolutions,
        })
    }

    pub(super) fn resolve_add_track_candidate(
        &self,
        conn: &rusqlite::Connection,
        candidate: &AddTrackCandidate,
        request_usb_root: Option<&str>,
        request_usb_root_valid: bool,
    ) -> BackendResult<AddTrackCandidateResolution> {
        let previous_id = trimmed_string(candidate.track_id.as_deref());
        // A USB-origin candidate carries the absolute path to its on-USB ANLZ
        // bundle; its cues / beat-grid are imported lazily (not during the USB
        // import) so they must be pulled in now, before the track can be
        // exported from this local playlist.
        let usb_anlz = candidate
            .usb_analysis_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let usb_origin = add_candidate_is_usb_origin(candidate, request_usb_root);
        if usb_origin {
            // Browsing a USB is read-only, so adding is where a USB track's
            // local row is created (or matched) and linked to this device,
            // and its ANLZ analysis imported.
            if let Some(track_id) =
                self.materialize_usb_add_candidate(candidate, request_usb_root)?
            {
                return Ok(AddTrackCandidateResolution {
                    previous_id,
                    track_id: Some(track_id),
                    resolved_by: "usbMaterialized".to_string(),
                    materialized: true,
                });
            }
        }

        if let Some(local_track_id) = trimmed_string(candidate.local_track_id.as_deref()) {
            if let Some(path) = usb_anlz
                && track_id_exists(conn, &local_track_id)?
            {
                self.ensure_usb_analysis_imported(&local_track_id, path)?;
            }
            return Ok(AddTrackCandidateResolution {
                previous_id,
                track_id: Some(local_track_id),
                resolved_by: "localTrackId".to_string(),
                materialized: false,
            });
        }

        if let Some(track_id) = previous_id.as_deref()
            && track_id_exists(conn, track_id)?
        {
            if let Some(path) = usb_anlz {
                self.ensure_usb_analysis_imported(track_id, path)?;
            }
            return Ok(AddTrackCandidateResolution {
                previous_id: previous_id.clone(),
                track_id: Some(track_id.to_string()),
                resolved_by: "self".to_string(),
                materialized: false,
            });
        }

        if usb_origin {
            return Ok(AddTrackCandidateResolution {
                previous_id,
                track_id: None,
                resolved_by: "usbOrigin".to_string(),
                materialized: false,
            });
        }

        let identity = self.resolve_track_identity(ResolveTrackIdentityRequest {
            track_id: previous_id.clone(),
            title: candidate.title.clone(),
            artist: candidate.artist.clone(),
            album: candidate.album.clone(),
            bpm: candidate.bpm,
            file_path: candidate.file_path.clone(),
            file_size_bytes: candidate.file_size_bytes,
            track_number: candidate.track_number,
            key: candidate.key.clone(),
            format_ext: candidate.format_ext.clone(),
            sample_rate_hz: candidate.sample_rate_hz,
            bit_depth: candidate.bit_depth,
            bitrate_kbps: candidate.bitrate_kbps,
            usb_root: candidate
                .usb_root
                .clone()
                .or_else(|| trimmed_string(request_usb_root)),
            usb_root_valid: candidate.usb_root_valid || request_usb_root_valid,
            usb_analysis_path: candidate.usb_analysis_path.clone(),
        })?;

        Ok(AddTrackCandidateResolution {
            previous_id,
            track_id: identity.track_id,
            resolved_by: identity.resolved_by,
            materialized: identity.materialized,
        })
    }

    pub fn remove_tracks_from_playlist(
        &self,
        req: RemoveTracksFromPlaylistRequest,
    ) -> BackendResult<RemoveTracksFromPlaylistData> {
        if req.track_ids.is_empty() {
            return Err(BackendError::Validation(
                "trackIds must contain at least one id".to_string(),
            ));
        }

        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        ensure_playlist_exists_conn(&tx, &req.playlist_id)?;

        let mut removed = 0usize;
        let mut uniq_track_ids = req.track_ids.clone();
        uniq_track_ids.sort();
        uniq_track_ids.dedup();

        for track_id in &uniq_track_ids {
            removed += tx.execute(
                "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
                params![req.playlist_id, track_id],
            )?;
        }

        let mut row_ids = Vec::<String>::new();
        {
            let mut stmt = tx.prepare(
                "SELECT id FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position ASC, id ASC",
            )?;
            let rows = stmt.query_map(params![req.playlist_id], |row| row.get::<_, String>(0))?;
            for row in rows {
                row_ids.push(row?);
            }
        }
        for (idx, row_id) in row_ids.iter().enumerate() {
            tx.execute(
                "UPDATE playlist_tracks SET position = ?1 WHERE id = ?2",
                params![(idx as i64) + 1, row_id],
            )?;
        }

        tx.execute(
            "UPDATE playlists SET updated_at = ?1, last_exported_at = NULL, last_exported_usb_root = NULL, last_exported_track_count = NULL WHERE id = ?2",
            params![now(), req.playlist_id],
        )?;

        tx.commit()?;
        Ok(RemoveTracksFromPlaylistData {
            playlist_id: req.playlist_id,
            removed,
        })
    }

    pub fn reorder_playlist_tracks(
        &self,
        req: ReorderPlaylistTracksRequest,
    ) -> BackendResult<ReorderPlaylistTracksData> {
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        ensure_playlist_exists_conn(&tx, &req.playlist_id)?;

        // `id` (playlist_tracks row id) keyed by `track_id`, and the current
        // position order -- needed by every mode.
        let mut row_by_track_id = std::collections::HashMap::<String, String>::new();
        let mut current_order: Vec<String> = Vec::new();
        {
            let mut stmt = tx.prepare(
                "SELECT id, track_id FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position ASC",
            )?;
            let rows = stmt.query_map(params![req.playlist_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (id, track_id) = row?;
                current_order.push(track_id.clone());
                row_by_track_id.insert(track_id, id);
            }
        }

        // Resolve the complete new order from whichever mode the request uses.
        let ordered_track_ids: Vec<String> = if !req.ordered_track_ids.is_empty() {
            req.ordered_track_ids.clone()
        } else if let Some(move_id) = req.move_track_id.as_deref() {
            if !row_by_track_id.contains_key(move_id) {
                return Err(BackendError::Validation(
                    "moveTrackId is not in this playlist".to_string(),
                ));
            }
            let mut order: Vec<String> = current_order
                .iter()
                .filter(|id| id.as_str() != move_id)
                .cloned()
                .collect();
            let insert_at = match req.before_track_id.as_deref() {
                Some(before) => order
                    .iter()
                    .position(|id| id == before)
                    .unwrap_or(order.len()),
                None => order.len(),
            };
            order.insert(insert_at, move_id.to_string());
            order
        } else if req.sort_by.is_some() {
            let mut tracks = {
                let mut stmt = tx.prepare(
                    r#"
                    SELECT t.id, t.title, t.artist, t.album, t.track_number, t.bpm, t.tonality, t.file_path,
                           t.file_size_bytes, t.format_ext, t.sample_rate_hz, t.bit_depth, t.bitrate_kbps, t.duration_ms,
                           t.artwork_path, t.waveform_peaks_path, t.bpm_analyzer, t.created_at, t.updated_at,
                           COALESCE(t.rekordbox_db_source, 0) AS rekordbox_db_source, t.wav_extensible_kind,
                   COALESCE(t.mixxx_db_source, 0) AS mixxx_db_source
                    FROM playlist_tracks pt
                    JOIN tracks t ON t.id = pt.track_id
                    WHERE pt.playlist_id = ?1
                    ORDER BY pt.position ASC
                    "#,
                )?;
                let rows =
                    stmt.query_map(params![req.playlist_id], |row| row_to_track(row, true))?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            sort_tracks(&mut tracks, req.sort_by.as_deref(), req.sort_dir.as_deref());
            tracks.into_iter().map(|t| t.id).collect()
        } else {
            return Err(BackendError::Validation(
                "reorder_playlist_tracks needs one of orderedTrackIds, moveTrackId, or sortBy"
                    .to_string(),
            ));
        };

        if ordered_track_ids.is_empty() {
            return Err(BackendError::Validation(
                "orderedTrackIds must contain at least one id".to_string(),
            ));
        }

        let mut uniq_ordered_ids = ordered_track_ids.clone();
        uniq_ordered_ids.sort();
        uniq_ordered_ids.dedup();
        if uniq_ordered_ids.len() != ordered_track_ids.len() {
            return Err(BackendError::Validation(
                "orderedTrackIds must not contain duplicates".to_string(),
            ));
        }

        let existing_track_ids: std::collections::HashSet<&String> =
            row_by_track_id.keys().collect();
        let requested_track_ids: std::collections::HashSet<&String> =
            ordered_track_ids.iter().collect();
        if existing_track_ids != requested_track_ids {
            return Err(BackendError::Validation(
                "orderedTrackIds must match the playlist's current track set".to_string(),
            ));
        }

        // Two-phase update: `playlist_tracks` has UNIQUE(playlist_id, position),
        // so writing final positions directly can collide mid-transaction with
        // whatever a row currently holds. Shift every row to a disjoint negative
        // range first, then assign the real 1..N positions in a second pass.
        for (idx, track_id) in ordered_track_ids.iter().enumerate() {
            let row_id = &row_by_track_id[track_id];
            tx.execute(
                "UPDATE playlist_tracks SET position = ?1 WHERE id = ?2",
                params![-((idx as i64) + 1), row_id],
            )?;
        }
        for (idx, track_id) in ordered_track_ids.iter().enumerate() {
            let row_id = &row_by_track_id[track_id];
            tx.execute(
                "UPDATE playlist_tracks SET position = ?1 WHERE id = ?2",
                params![(idx as i64) + 1, row_id],
            )?;
        }

        tx.execute(
            "UPDATE playlists SET updated_at = ?1, last_exported_at = NULL, last_exported_usb_root = NULL, last_exported_track_count = NULL WHERE id = ?2",
            params![now(), req.playlist_id],
        )?;

        tx.commit()?;
        Ok(ReorderPlaylistTracksData {
            playlist_id: req.playlist_id,
            reordered: ordered_track_ids.len(),
        })
    }
}

pub(super) fn ensure_playlist_exists_conn(
    conn: &rusqlite::Connection,
    playlist_id: &str,
) -> BackendResult<()> {
    let exists: Option<String> = conn
        .query_row(
            "SELECT id FROM playlists WHERE id = ?1",
            params![playlist_id],
            |row| row.get(0),
        )
        .optional()?;

    if exists.is_none() {
        return Err(BackendError::NotFound(format!(
            "playlist not found: {playlist_id}"
        )));
    }

    Ok(())
}

pub(super) fn ensure_playlist_exists(db: &Db, playlist_id: &str) -> BackendResult<()> {
    let conn = db.connect()?;
    ensure_playlist_exists_conn(&conn, playlist_id)
}
