//! Track library: scanning, search, listing, browsing, track identity and paging.

use super::*;

pub(super) fn browse_path_key(path: &str) -> String {
    path.trim().replace('\\', "/").to_ascii_lowercase()
}

/// The file a stored `tracks.file_path` names on this machine. Used by
/// playback resolution to prefer a library row whose file is actually
/// present over a stale/duplicate row pointing at a moved-or-deleted file.
pub(super) fn stored_file_path(path: &str) -> &Path {
    Path::new(path.trim())
}

pub(crate) fn browse_path_matches_root(file_path: &str, root: &str) -> bool {
    let file_key = browse_path_key(file_path);
    let root_key = browse_path_key(root).trim_end_matches('/').to_string();
    !root_key.is_empty() && (file_key == root_key || file_key.starts_with(&format!("{root_key}/")))
}

impl BackendService {
    pub fn scan_library(&self, req: ScanLibraryRequest) -> BackendResult<ScanLibraryData> {
        let source_roots = sanitize_source_roots(req.source_roots);
        if source_roots.is_empty() {
            return Err(BackendError::Validation(
                "sourceRoots must contain at least one path".to_string(),
            ));
        }

        let mut existing_roots = Vec::<String>::new();
        let mut not_found = Vec::<String>::new();
        for root in source_roots {
            if source_root_is_usable(Path::new(&root)) {
                existing_roots.push(root);
            } else {
                not_found.push(root);
            }
        }
        let mut warnings = Vec::<WarningEntry>::new();
        if !not_found.is_empty() {
            warnings.push(logging::log(
                Level::Warn,
                "library-scan",
                "scan.source-folders-missing",
                format!(
                    "{} source folder(s) missing or not directories: {}",
                    not_found.len(),
                    not_found.join(", ")
                ),
            ));
        }

        if existing_roots.is_empty() {
            return Ok(ScanLibraryData {
                job_id: Uuid::now_v7().to_string(),
                indexed: 0,
                updated: 0,
                removed: 0,
                not_found,
                scoped_track_count: 0,
                album_count: 0,
                unanalyzed_count: 0,
                warnings,
            });
        }

        let scanned = scan_audio_files(&existing_roots)?;
        let now = now();
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;

        // Pre-load all existing tracks into a HashMap to avoid N+1 queries
        let mut existing_tracks = ExistingTracksByPath::new();
        {
            let mut stmt = tx.prepare(
                "SELECT id, file_modified_at, title, artist, album, track_number, tonality, format_ext, sample_rate_hz, bit_depth, bitrate_kbps, disc_number, subtitle, comment, isrc, release_year, release_date, recorded_date, genre, file_path, wav_extensible_kind, COALESCE(genre_source = 'user', 0), file_created_at, COALESCE(release_date_source = 'user', 0) FROM tracks",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(19)?, // file_path as key
                    (
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<u32>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<u32>>(8)?,
                        row.get::<_, Option<u8>>(9)?,
                        row.get::<_, Option<u32>>(10)?,
                        row.get::<_, Option<u32>>(11)?,
                        row.get::<_, Option<String>>(12)?,
                        row.get::<_, Option<String>>(13)?,
                        row.get::<_, Option<String>>(14)?,
                        row.get::<_, Option<u32>>(15)?,
                        row.get::<_, Option<String>>(16)?,
                        row.get::<_, Option<String>>(17)?,
                        row.get::<_, Option<String>>(18)?, // genre
                        row.get::<_, Option<String>>(20)?, // wav_extensible_kind
                        row.get::<_, bool>(21)?,           // genre edited in the app
                        row.get::<_, Option<i64>>(22)?,    // file_created_at
                        row.get::<_, bool>(23)?,           // release date edited in the app
                    ),
                ))
            })?;
            for row in rows {
                let (path, data) = row?;
                existing_tracks.insert(path, data);
            }
        }

        let mut indexed = 0usize;
        let mut updated = 0usize;

        for item in &scanned {
            let existing = existing_tracks.get(&item.path).cloned();

            match existing {
                None => {
                    let id = Uuid::now_v7().to_string();
                    let fingerprint = build_track_match_fingerprint(
                        &item.title,
                        &item.artist,
                        item.album.as_deref(),
                    );
                    let wav_extensible_kind =
                        item.wav_extensible_kind.map(WavFormatIssue::as_db_str);
                    tx.execute(
                        r#"
                        INSERT INTO tracks (
                          id, title, artist, album, track_number, bpm, tonality, file_path,
                          file_size_bytes, file_modified_at, format_ext, sample_rate_hz, bit_depth, bitrate_kbps,
                          disc_number, subtitle, comment, isrc, release_year, release_date, recorded_date,
                          genre, duration_ms, artwork_path, waveform_peaks_path, match_fingerprint,
                          created_at, updated_at, wav_extensible_kind, file_created_at
                        ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, NULL, NULL, NULL, ?22, ?23, ?23, ?24, ?25)
                        "#,
                        params![
                            id,
                            item.title,
                            item.artist,
                            item.album,
                            item.track_number,
                            item.tonality,
                            item.path,
                            item.file_size_bytes,
                            item.file_modified_at,
                            item.format_ext,
                            item.sample_rate_hz,
                            item.bit_depth,
                            item.bitrate_kbps,
                            item.disc_number,
                            item.subtitle,
                            item.comment,
                            item.isrc,
                            item.release_year,
                            item.release_date,
                            item.recorded_date,
                            item.genre,
                            fingerprint,
                            now,
                            wav_extensible_kind,
                            item.file_created_at
                        ],
                    )?;
                    indexed += 1;
                }
                Some((
                    id,
                    old_modified,
                    old_title,
                    old_artist,
                    old_album,
                    old_track_number,
                    old_tonality,
                    old_format_ext,
                    old_sample_rate_hz,
                    old_bit_depth,
                    old_bitrate_kbps,
                    old_disc_number,
                    old_subtitle,
                    old_comment,
                    old_isrc,
                    old_release_year,
                    old_release_date,
                    old_recorded_date,
                    old_genre,
                    old_wav_extensible_kind,
                    genre_edited,
                    old_created,
                    date_edited,
                )) => {
                    let wav_extensible_kind =
                        item.wav_extensible_kind.map(WavFormatIssue::as_db_str);
                    let tonality_changed = item.tonality.is_some() && old_tonality != item.tonality;
                    let metadata_changed = old_title != item.title
                        || old_artist != item.artist
                        || old_album != item.album
                        || old_track_number != item.track_number
                        || tonality_changed
                        || old_format_ext != item.format_ext
                        || old_sample_rate_hz != item.sample_rate_hz
                        || old_bit_depth != item.bit_depth
                        || old_bitrate_kbps != item.bitrate_kbps
                        || old_disc_number != item.disc_number
                        || old_subtitle != item.subtitle
                        || old_comment != item.comment
                        || old_isrc != item.isrc
                        // So does a release date (and its year) edited in the app.
                        || (!date_edited
                            && (old_release_year != item.release_year
                                || old_release_date != item.release_date))
                        || old_recorded_date != item.recorded_date
                        // A genre edited in the app wins over the file's tag.
                        || (!genre_edited && old_genre != item.genre)
                        || old_wav_extensible_kind.as_deref() != wav_extensible_kind;
                    let file_changed = old_modified != item.file_modified_at || metadata_changed;
                    // A creation time alone changing (first rescan after the
                    // column was added) is stored but doesn't count as an update.
                    if file_changed || old_created != item.file_created_at {
                        let fingerprint = build_track_match_fingerprint(
                            &item.title,
                            &item.artist,
                            item.album.as_deref(),
                        );
                        tx.execute(
                            r#"
                            UPDATE tracks
                            SET title = ?1,
                                artist = ?2,
                                album = ?3,
                                track_number = ?4,
                                tonality = COALESCE(?5, tonality),
                                file_size_bytes = ?6,
                                file_modified_at = ?7,
                                format_ext = ?8,
                                sample_rate_hz = ?9,
                                bit_depth = ?10,
                                bitrate_kbps = ?11,
                                disc_number = ?12,
                                subtitle = ?13,
                                comment = ?14,
                                isrc = ?15,
                                release_year = CASE WHEN release_date_source = 'user' THEN release_year ELSE ?16 END,
                                release_date = CASE WHEN release_date_source = 'user' THEN release_date ELSE ?17 END,
                                recorded_date = ?18,
                                genre = CASE WHEN genre_source = 'user' THEN genre ELSE ?19 END,
                                match_fingerprint = ?20,
                                updated_at = ?21,
                                wav_extensible_kind = ?23,
                                file_created_at = ?24
                            WHERE id = ?22
                            "#,
                            params![
                                item.title,
                                item.artist,
                                item.album,
                                item.track_number,
                                item.tonality,
                                item.file_size_bytes,
                                item.file_modified_at,
                                item.format_ext,
                                item.sample_rate_hz,
                                item.bit_depth,
                                item.bitrate_kbps,
                                item.disc_number,
                                item.subtitle,
                                item.comment,
                                item.isrc,
                                item.release_year,
                                item.release_date,
                                item.recorded_date,
                                item.genre,
                                fingerprint,
                                now,
                                id,
                                wav_extensible_kind,
                                item.file_created_at
                            ],
                        )?;
                        if file_changed {
                            updated += 1;
                        }
                    }
                }
            }
        }

        let scanned_paths = unique_paths(&scanned);
        let mut removed = 0usize;
        let mut orphaned_cache_bundles = Vec::<PathBuf>::new();
        let analysis_dir = self.db.data_dir().join("analysis");
        let waveform_dir = analysis_dir.join("waveforms");
        let artwork_dir = analysis_dir.join("artwork");
        for root in &existing_roots {
            let escaped_root = root
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let slash_like = format!("{escaped_root}/%");
            let backslash_like = format!("{escaped_root}\\\\%");
            let mut stmt = tx.prepare(
                "SELECT id, file_path, waveform_peaks_path FROM tracks WHERE file_path = ?1 OR file_path LIKE ?2 ESCAPE '\\' OR file_path LIKE ?3 ESCAPE '\\'",
            )?;
            let rows = stmt.query_map(params![root, slash_like, backslash_like], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;

            for row in rows {
                let (id, path, waveform_peaks_path) = row?;
                if !scanned_paths.contains(path.as_str()) {
                    tx.execute("DELETE FROM tracks WHERE id = ?1", params![id])?;
                    if let Some(dat) = waveform_peaks_path
                        .as_deref()
                        .and_then(|p| local_cache_bundle_dat(&waveform_dir, Path::new(p.trim())))
                    {
                        orphaned_cache_bundles.push(dat);
                    }
                    let _ = std::fs::remove_file(artwork_dir.join(format!("{id}.jpg")));
                    removed += 1;
                }
            }
        }

        // Clean up the removed tracks' local ANLZ cache, unless a remaining
        // track shares the bundle (the cache name hashes the lower-cased
        // source path, so case-variant paths map to one bundle).
        for dat in &orphaned_cache_bundles {
            let still_referenced: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM tracks WHERE waveform_peaks_path = ?1)",
                params![dat.to_string_lossy()],
                |row| row.get(0),
            )?;
            if !still_referenced {
                remove_bundle_files(dat);
            }
        }

        // Detect stale local ANLZ cache (missing PWV6 in .2EX) and mark for
        // re-analysis. Only the app's own cache is checked: a bundle on a USB
        // stick is never cleared or deleted here.
        {
            let mut stmt = tx.prepare(
                "SELECT id, waveform_peaks_path FROM tracks WHERE waveform_peaks_path IS NOT NULL",
            )?;
            let rows: Vec<(String, String)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            for (id, waveform_peaks_path) in &rows {
                let Some(dat) =
                    local_cache_bundle_dat(&waveform_dir, Path::new(waveform_peaks_path.trim()))
                else {
                    continue;
                };
                if let Ok(bytes) = std::fs::read(dat.with_extension("2EX"))
                    && bytes.starts_with(b"PMAI")
                    && !bytes.windows(4).any(|w| w == b"PWV6")
                {
                    tx.execute(
                        "UPDATE tracks SET waveform_peaks_path = NULL WHERE id = ?1",
                        params![id],
                    )?;
                    remove_bundle_files(&dat);
                }
            }
        }

        tx.commit()?;

        // Post-scan library facts over the whole scanned set (not the page the
        // frontend reloads next): every on-disk file now has an indexed row and
        // removed files are gone, so a plain DB read is accurate.
        let (scoped_track_count, album_count, unanalyzed_count) = {
            let mut stmt = conn.prepare(&format!("SELECT {TRACK_COLS} FROM tracks"))?;
            let rows = stmt.query_map([], |row| row_to_track(row, false))?;
            let scoped: Vec<Track> = rows
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|track| {
                    existing_roots
                        .iter()
                        .any(|root| browse_path_matches_root(&track.file_path, root))
                })
                .collect();
            let mut albums = std::collections::HashSet::<String>::new();
            let mut unanalyzed = 0usize;
            for track in &scoped {
                let album = track
                    .album
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .to_lowercase();
                if !album.is_empty() {
                    albums.insert(album);
                }
                if !track_has_core_analysis_for_source_status(track) {
                    unanalyzed += 1;
                }
            }
            (scoped.len(), albums.len(), unanalyzed)
        };

        Ok(ScanLibraryData {
            job_id: Uuid::now_v7().to_string(),
            indexed,
            updated,
            removed,
            not_found,
            scoped_track_count,
            album_count,
            unanalyzed_count,
            warnings,
        })
    }

    /// Builds the complete library track list for a set of source roots
    /// (plus optional master.db tracks) -- NOT filtered by search query.
    /// This is the expensive, I/O-bound part (filesystem scan + DB read),
    /// shared by `compute_visible_library_tracks` below and by
    /// `browse_source_files` directly (which needs the *unfiltered* set to
    /// compute `source_root_analysis` -- a folder's analyzed status must
    /// not depend on the current search query -- without scanning twice).
    /// Sanitizes/sorts/dedups `source_roots` itself, so callers may pass
    /// raw values.
    pub(super) fn compute_all_library_tracks(
        &self,
        conn: &rusqlite::Connection,
        source_roots: &[String],
        libraries: ExternalLibraries,
    ) -> BackendResult<Vec<Track>> {
        let mut source_roots = source_roots
            .iter()
            .map(|root| root.trim().to_string())
            .filter(|root| !root.is_empty())
            .collect::<Vec<_>>();
        source_roots.sort();
        source_roots.dedup();
        if source_roots.is_empty() && !libraries.any() {
            return Ok(Vec::new());
        }

        let scanned = if source_roots.is_empty() {
            Vec::new()
        } else {
            scan_audio_files_cached(&source_roots, &self.library_scan_cache)?
        };
        let mut stmt = conn.prepare(&format!("SELECT {TRACK_COLS} FROM tracks"))?;
        let indexed_rows = stmt.query_map([], |row| row_to_track(row, false))?;
        let indexed_tracks = indexed_rows.collect::<Result<Vec<_>, _>>()?;
        let indexed_by_path = indexed_tracks
            .iter()
            .cloned()
            .map(|track| (browse_path_key(&track.file_path), track))
            .collect::<std::collections::HashMap<_, _>>();

        let mut seen_paths = std::collections::HashSet::<String>::new();
        let mut items = scanned
            .into_iter()
            .map(|scanned| {
                let scanned_key = browse_path_key(&scanned.path);
                seen_paths.insert(scanned_key.clone());
                if let Some(existing) = indexed_by_path.get(&scanned_key) {
                    existing.clone()
                } else {
                    let now = now();
                    let wav_extensible_kind = scanned
                        .wav_extensible_kind
                        .map(|kind| kind.as_db_str().to_string());
                    let format_compat = format_compat::compute_format_compat(
                        scanned.format_ext.as_deref(),
                        scanned.sample_rate_hz,
                        scanned.bit_depth,
                        scanned.bitrate_kbps,
                        wav_extensible_kind.as_deref(),
                    );
                    Track {
                        id: scanned.path.clone(),
                        title: scanned.title,
                        artist: scanned.artist,
                        album: scanned.album,
                        year: scanned.release_year,
                        genre: scanned.genre,
                        track_number: scanned.track_number,
                        bpm: None,
                        bpm_analyzer: None,
                        key: scanned.tonality,
                        key_display: None,
                        key_color: None,
                        key_source: None,
                        file_path: scanned.path,
                        file_size_bytes: scanned.file_size_bytes,
                        format_ext: scanned.format_ext,
                        sample_rate_hz: scanned.sample_rate_hz,
                        bit_depth: scanned.bit_depth,
                        bitrate_kbps: scanned.bitrate_kbps,
                        wav_extensible_kind,
                        duration_ms: None,
                        artwork_path: None,
                        artwork_data_url: None,
                        waveform_peaks_path: None,
                        waveform_preview: None,
                        waveform_color_data: None,
                        created_at: now.clone(),
                        updated_at: now,
                        rekordbox_db_source: false,
                        mixxx_db_source: false,
                        is_usb_path: false,
                        // Freshly scanned, not yet indexed -- no analysis.
                        analysis_ready: false,
                        format_compat,
                    }
                }
            })
            .collect::<Vec<_>>();

        if libraries.any() {
            for track in indexed_tracks
                .into_iter()
                .filter(|track| libraries.includes(track))
            {
                let path_key = browse_path_key(&track.file_path);
                if seen_paths.insert(path_key) {
                    items.push(track);
                }
            }
        }

        items.sort_by(|a, b| {
            browse_path_key(&a.file_path)
                .cmp(&browse_path_key(&b.file_path))
                .then_with(|| a.id.cmp(&b.id))
        });

        Ok(items)
    }

    /// Builds the complete filtered track list for a set of source roots
    /// (plus optional master.db tracks) matching a search query -- the same
    /// filtering `browse_source_files` returns to the frontend before it's
    /// paginated. Shared by `browse_source_files` and the analysis pipeline
    /// (which uses it to compute the live library-duration-total baseline),
    /// so both stay consistent by construction. Sanitizes/sorts/dedups
    /// `source_roots` itself, so callers may pass raw values.
    pub(super) fn compute_visible_library_tracks(
        &self,
        conn: &rusqlite::Connection,
        source_roots: &[String],
        libraries: ExternalLibraries,
        query: &str,
    ) -> BackendResult<Vec<Track>> {
        let mut items = self.compute_all_library_tracks(conn, source_roots, libraries)?;
        TrackQuery::new(query).retain(&mut items);
        Ok(items)
    }

    pub fn browse_source_files(
        &self,
        req: BrowseSourceFilesRequest,
    ) -> BackendResult<BrowseSourceFilesData> {
        let mut source_roots = req
            .source_roots
            .into_iter()
            .map(|root| root.trim().to_string())
            .filter(|root| !root.is_empty())
            .collect::<Vec<_>>();
        source_roots.sort();
        source_roots.dedup();
        let libraries = ExternalLibraries {
            rekordbox_db: req.include_rekordbox_db,
            mixxx_db: req.include_mixxx_db,
        };
        if source_roots.is_empty() && !libraries.any() {
            return Ok(BrowseSourceFilesData {
                total: 0,
                items: Vec::new(),
                next_cursor: None,
                has_more: false,
                source_root_analysis: Vec::new(),
                total_duration_ms: 0,
                duration_known_count: 0,
            });
        }

        let limit = req.limit.clamp(1, 5000);
        let query = TrackQuery::new(&req.query);
        let sort = TrackSort::new(req.sort_by.as_deref(), req.sort_dir.as_deref());
        let roots_signature = source_roots.join("\u{1F}");
        let signature = build_track_cursor_signature(&[
            TRACK_CURSOR_VERSION,
            "browse_source_files",
            query.as_str(),
            &roots_signature,
            if libraries.rekordbox_db {
                "rekordbox_db"
            } else {
                "no_rekordbox_db"
            },
            if libraries.mixxx_db {
                "mixxx_db"
            } else {
                "no_mixxx_db"
            },
            sort.by,
            sort.dir,
        ]);
        let cursor = decode_track_page_cursor(req.cursor.as_deref(), &signature)?;

        let conn = self.db.connect()?;
        let all_items = self.compute_all_library_tracks(&conn, &source_roots, libraries)?;

        // source_root_analysis describes each folder's own analyzed state
        // and must not depend on the current search query -- otherwise a
        // query that doesn't match (all of) a folder's tracks makes an
        // otherwise fully-analyzed folder's chip appear unanalyzed (in the
        // worst case, a query matching zero tracks in a folder collapses
        // `total` to 0, and `fully_analyzed` requires `total > 0`). Computed
        // from the full, unfiltered set -- before the query filter below.
        let source_root_analysis = source_root_analysis(&source_roots, &all_items);

        let mut items = all_items;
        query.retain(&mut items);
        // Client-driven column sort, applied server-side so it spans the whole
        // library, not just the page already loaded. Stable, so equal keys keep
        // the natural file-path order from compute_all_library_tracks.
        sort.apply(&mut items);

        let mut total_duration_ms: u64 = 0;
        let mut duration_known_count: usize = 0;
        for track in &items {
            let has_duration = track.duration_ms.map(|d| d > 0).unwrap_or(false);
            let countable = track_has_core_analysis_for_source_status(track)
                || (track.rekordbox_db_source && has_duration);
            if countable && let Some(d) = track.duration_ms {
                total_duration_ms += d;
                duration_known_count += 1;
            }
        }

        let total = items.len();
        let start_idx = if let Some(offset) = req.offset {
            offset.min(total)
        } else if let Some(cursor) = cursor.as_ref() {
            items
                .iter()
                .position(|track| {
                    browse_path_key(&track.file_path) == browse_path_key(&cursor.file_path)
                        && track.id == cursor.id
                })
                .map(|idx| idx + 1)
                .unwrap_or(0)
        } else {
            0
        };
        let mut page_items = items
            .into_iter()
            .skip(start_idx)
            .take(limit + 1)
            .collect::<Vec<_>>();
        let (has_more, next_cursor) = paginate_tracks(&mut page_items, limit, &signature);
        apply_frontend_track_fields(&conn, &mut page_items)?;
        Ok(BrowseSourceFilesData {
            total,
            items: page_items,
            next_cursor,
            has_more,
            source_root_analysis,
            total_duration_ms,
            duration_known_count,
        })
    }

    pub fn materialize_source_track(
        &self,
        req: MaterializeSourceTrackRequest,
    ) -> BackendResult<MaterializeSourceTrackData> {
        let file_path = req.file_path.trim();
        if file_path.is_empty() {
            return Err(BackendError::Validation(
                "filePath must not be empty".to_string(),
            ));
        }
        let path = std::path::Path::new(file_path);
        if !path.is_file() {
            return Err(BackendError::NotFound(format!(
                "source file does not exist: {file_path}"
            )));
        }

        let metadata = std::fs::metadata(path)?;
        let file_modified_at = crate::scanner::file_time_secs(metadata.modified());
        let file_created_at = crate::scanner::file_time_secs(metadata.created());
        let title = req.title.trim();
        let artist = req.artist.trim();
        let album = req.album.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
        let format_ext = req
            .format_ext
            .and_then(|value| {
                let trimmed = value.trim().to_string();
                (!trimmed.is_empty()).then_some(trimmed)
            })
            .or_else(|| crate::audio_format::format_ext_from_path(file_path));
        let key = req.key.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
        let fingerprint = build_track_match_fingerprint(title, artist, album.as_deref());
        let file_size_bytes = req
            .file_size_bytes
            .or_else(|| i64::try_from(metadata.len()).ok());
        let now = now();

        let conn = self.db.connect()?;
        let mut stmt = conn.prepare("SELECT id FROM tracks WHERE file_path = ?1 LIMIT 1")?;
        let existing_id = stmt
            .query_row(params![file_path], |row| row.get::<_, String>(0))
            .optional()?;

        let track_id = if let Some(id) = existing_id {
            conn.execute(
                r#"
                UPDATE tracks
                SET title = ?1,
                    artist = ?2,
                    album = ?3,
                    track_number = ?4,
                    tonality = COALESCE(?5, tonality),
                    file_size_bytes = ?6,
                    file_modified_at = ?7,
                    format_ext = ?8,
                    sample_rate_hz = ?9,
                    bit_depth = ?10,
                    bitrate_kbps = ?11,
                    match_fingerprint = ?12,
                    updated_at = ?13,
                    file_created_at = ?15
                WHERE id = ?14
                "#,
                params![
                    title,
                    artist,
                    album,
                    req.track_number,
                    key,
                    file_size_bytes,
                    file_modified_at,
                    format_ext,
                    req.sample_rate_hz,
                    req.bit_depth,
                    req.bitrate_kbps,
                    fingerprint,
                    now,
                    id,
                    file_created_at
                ],
            )?;
            id
        } else {
            let id = Uuid::now_v7().to_string();
            conn.execute(
                r#"
                INSERT INTO tracks (
                  id, title, artist, album, track_number, bpm, tonality, file_path,
                  file_size_bytes, file_modified_at, format_ext, sample_rate_hz, bit_depth, bitrate_kbps,
                  disc_number, subtitle, comment, isrc, release_year, release_date, recorded_date,
                  duration_ms, artwork_path, waveform_peaks_path, match_fingerprint,
                  created_at, updated_at, file_created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, ?14, ?15, ?15, ?16)
                "#,
                params![
                    id,
                    title,
                    artist,
                    album,
                    req.track_number,
                    key,
                    file_path,
                    file_size_bytes,
                    file_modified_at,
                    format_ext,
                    req.sample_rate_hz,
                    req.bit_depth,
                    req.bitrate_kbps,
                    fingerprint,
                    now,
                    file_created_at
                ],
            )?;
            id
        };

        Ok(MaterializeSourceTrackData { track_id })
    }

    pub fn resolve_track_identity(
        &self,
        req: ResolveTrackIdentityRequest,
    ) -> BackendResult<ResolveTrackIdentityData> {
        let file_path = req.file_path.as_deref().unwrap_or("").trim();
        let usb_root = req.usb_root.as_deref().unwrap_or("").trim();
        let usb_analysis_path = req.usb_analysis_path.as_deref().unwrap_or("").trim();
        let path_is_selected_usb = !file_path.is_empty()
            && !usb_root.is_empty()
            && browse_path_matches_root(file_path, usb_root);
        let path_has_usb_marker = !usb_analysis_path.is_empty() || path_is_selected_usb;

        let resolve_request = || ResolvePlaybackSourceRequest {
            title: req.title.clone(),
            artist: req.artist.clone(),
            album: req.album.clone(),
            bpm: req.bpm,
            file_path: req.file_path.clone(),
            file_size_bytes: req.file_size_bytes,
            track_id: req.track_id.clone(),
        };

        let resolved = self.resolve_playback_source(resolve_request())?;
        if resolved.matched_by == TrackMatch::SelfId {
            return Ok(ResolveTrackIdentityData {
                track_id: resolved.track_id,
                resolved_by: resolved.matched_by.into(),
                materialized: false,
            });
        }

        if !file_path.is_empty()
            && !path_has_usb_marker
            && let Ok(data) = self.materialize_source_track(MaterializeSourceTrackRequest {
                file_path: file_path.to_string(),
                title: req.title.clone(),
                artist: req.artist.clone(),
                album: req.album.clone(),
                track_number: req.track_number,
                key: req.key.clone(),
                file_size_bytes: req.file_size_bytes,
                format_ext: req.format_ext.clone(),
                sample_rate_hz: req.sample_rate_hz,
                bit_depth: req.bit_depth,
                bitrate_kbps: req.bitrate_kbps,
            })
        {
            return Ok(ResolveTrackIdentityData {
                track_id: Some(data.track_id),
                resolved_by: ResolvedBy::Materialized,
                materialized: true,
            });
        }

        Ok(ResolveTrackIdentityData {
            track_id: resolved.track_id,
            resolved_by: resolved.matched_by.into(),
            materialized: false,
        })
    }

    pub fn get_tracks_by_ids_with_previews(
        &self,
        req: GetTracksByIdsRequest,
    ) -> BackendResult<GetTracksByIdsData> {
        let mut ids = req
            .track_ids
            .into_iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return Ok(GetTracksByIdsData { items: Vec::new() });
        }

        let conn = self.db.connect()?;
        let placeholders = vec!["?"; ids.len()].join(", ");
        let sql = format!("SELECT {TRACK_COLS} FROM tracks WHERE id IN ({placeholders})");

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(ids.iter()), |row| row_to_track(row, true))?;
        let mut found = rows.collect::<Result<Vec<_>, _>>()?;
        found.sort_by(|a, b| a.id.cmp(&b.id));
        apply_frontend_track_fields(&conn, &mut found)?;

        Ok(GetTracksByIdsData { items: found })
    }
}

/// Finds a single high-confidence local-track match for a USB-sourced
/// fingerprint. Duration must agree when available, and an exact file size
/// must be known on both sides. Missing sizes are read from file metadata
/// when possible; fingerprinting alone is text-only (title/artist/album).
/// Different known file formats are rejected even if the text and file facts
/// match: alternate file formats serve different players and remain separate.
/// Returns `None` (no match) whenever zero or more than one candidate
/// survives -- a spurious extra placeholder row is a strictly safer failure
/// mode than silently linking the wrong audio file. Candidates whose own
/// `file_path` falls under a known (untainted) USB root are skipped
/// entirely: a placeholder row matching another placeholder is never a
/// genuine match.
pub(crate) fn find_confident_fingerprint_match(
    conn: &rusqlite::Connection,
    fingerprint: &str,
    incoming_path: &str,
    incoming_duration_ms: Option<i64>,
    incoming_file_size_bytes: Option<i64>,
    usb_root_paths: &[String],
) -> BackendResult<Option<String>> {
    if fingerprint.trim().is_empty() {
        return Ok(None);
    }
    let mut stmt = conn.prepare(
        "SELECT id, file_path, duration_ms, file_size_bytes FROM tracks WHERE match_fingerprint = ?1",
    )?;
    let rows: Vec<(String, String, Option<i64>, Option<i64>)> = stmt
        .query_map(params![fingerprint], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<_, _>>()?;

    let incoming_size =
        super::track_matching::file_size_for_match(incoming_path, incoming_file_size_bytes);
    let Some(incoming_size) = incoming_size else {
        return Ok(None);
    };
    let mut passing = Vec::new();
    for (id, path, cand_duration, cand_size) in rows {
        if usb_root_paths
            .iter()
            .any(|root| browse_path_matches_root(&path, root))
        {
            continue;
        }
        if library_metadata_candidate_compatible(
            incoming_path,
            &path,
            incoming_duration_ms,
            cand_duration,
            Some(incoming_size),
            cand_size,
        ) {
            passing.push(id);
        }
    }

    if passing.len() == 1 {
        Ok(passing.into_iter().next())
    } else {
        Ok(None)
    }
}

pub(crate) fn row_to_track(
    row: &rusqlite::Row<'_>,
    include_previews: bool,
) -> rusqlite::Result<Track> {
    let file_path: String = row.get(7)?;
    let artwork_path: Option<String> = row.get(14)?;
    let waveform_peaks_path: Option<String> = row.get(15)?;
    let bpm_analyzer: Option<String> = row.get(16)?;
    // Load waveform preview (small, ~400 bytes) when requested.
    // Skip artwork base64 data URL for local tracks - the frontend uses
    // artworkPath via Tauri's asset protocol (convertFileSrc) instead.
    // Embedding full images as data URLs in JSON causes IPC/memory crashes
    // when hydrating many tracks at once.
    let is_rekordbox_db = row.get::<_, i64>(19).unwrap_or(0) != 0;
    // master.db tracks: try PWV4 color data from .EXT first; fall back to greyscale
    // PWAV/PWV2 from .DAT if extended analysis hasn't been run.
    let (waveform_preview, waveform_color_data) = if include_previews {
        if is_rekordbox_db {
            let color = waveform_peaks_path
                .as_deref()
                .map(Path::new)
                .and_then(read_pwv4_from_anlz);
            if color.is_some() {
                (None, color)
            } else {
                let preview = waveform_peaks_path
                    .as_deref()
                    .map(Path::new)
                    .and_then(load_waveform_preview_from_analysis_path);
                (preview, None)
            }
        } else {
            let preview = waveform_peaks_path
                .as_deref()
                .map(Path::new)
                .and_then(load_waveform_preview_from_analysis_path);
            (preview, None)
        }
    } else {
        (None, None)
    };
    let artwork_data_url: Option<String> = None;

    let bpm: Option<f64> = row.get(5)?;
    let duration_ms: Option<u64> = row.get(13)?;
    // Pure DB-column math, correct for every caller (no separate pass).
    let analysis_ready = has_core_analysis_fields(waveform_peaks_path.as_deref(), bpm, duration_ms);

    let format_ext: Option<String> = row
        .get::<_, Option<String>>(9)?
        .or_else(|| crate::audio_format::format_ext_from_path(&file_path));
    let sample_rate_hz: Option<u32> = row.get(10)?;
    let bit_depth: Option<u8> = row.get(11)?;
    let bitrate_kbps: Option<u32> = row.get(12)?;
    let wav_extensible_kind: Option<String> = row.get("wav_extensible_kind").unwrap_or(None);
    let format_compat = format_compat::compute_format_compat(
        format_ext.as_deref(),
        sample_rate_hz,
        bit_depth,
        bitrate_kbps,
        wav_extensible_kind.as_deref(),
    );

    Ok(Track {
        id: row.get(0)?,
        title: row.get(1)?,
        artist: row.get(2)?,
        album: row.get(3)?,
        year: row
            .get::<_, Option<u32>>("release_year")
            .unwrap_or(None)
            .filter(|year| *year > 0),
        genre: row
            .get::<_, Option<String>>("genre")
            .unwrap_or(None)
            .map(|genre| genre.trim().to_string())
            .filter(|genre| !genre.is_empty()),
        track_number: row.get(4)?,
        bpm,
        bpm_analyzer,
        key: row.get(6)?,
        key_source: row.get("tonality_source").unwrap_or(None),
        file_size_bytes: row.get(8)?,
        // Backend-owned: every track-returning command guarantees a format on
        // the wire. Fall back to the file-path extension for legacy NULL rows
        // and import paths that don't set the column (master.db, USB merge).
        format_ext,
        file_path,
        sample_rate_hz,
        bit_depth,
        bitrate_kbps,
        wav_extensible_kind,
        duration_ms,
        artwork_path,
        artwork_data_url,
        waveform_peaks_path,
        waveform_preview,
        waveform_color_data,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
        rekordbox_db_source: is_rekordbox_db,
        mixxx_db_source: row.get::<_, i64>("mixxx_db_source").unwrap_or(0) != 0,
        // Filled in by callers that expose Track to the frontend (see
        // apply_frontend_track_fields); internal-only callers leave this false.
        is_usb_path: false,
        key_display: None,
        key_color: None,
        analysis_ready,
        format_compat,
    })
}

/// Resolve a decoded page cursor to the index the next page starts at, over an
/// already-sorted+filtered `items` slice. `id` is unique, so it alone
/// identifies the boundary row regardless of the active sort.
pub(super) fn cursor_start_index(items: &[Track], cursor: Option<&TrackPageCursor>) -> usize {
    match cursor {
        Some(cursor) => items
            .iter()
            .position(|track| track.id == cursor.id)
            .map(|idx| idx + 1)
            .unwrap_or(0),
        None => 0,
    }
}

pub(super) fn paginate_tracks(
    items: &mut Vec<Track>,
    limit: usize,
    signature: &str,
) -> (bool, Option<String>) {
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next_cursor = items
        .last()
        .and_then(|last| encode_track_page_cursor(signature, &last.file_path, &last.id));
    (has_more, next_cursor)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TrackPageCursor {
    pub(super) version: String,
    pub(super) signature: String,
    pub(super) file_path: String,
    pub(super) id: String,
}

pub(crate) fn build_track_cursor_signature(parts: &[&str]) -> String {
    let joined = parts.join("|");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    joined.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Offset-based page cursor for the fully-in-memory list producers (the USB
/// per-list track commands): the source list is rebuilt deterministically per
/// request, so a plain offset is a stable boundary. Bound to its query/sort by
/// `signature` -- a stale cursor is rejected, and the frontend restarts from
/// page 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OffsetPageCursor {
    pub(super) version: String,
    pub(super) signature: String,
    pub(super) offset: usize,
}

pub(crate) fn encode_offset_cursor(signature: &str, offset: usize) -> String {
    let payload = OffsetPageCursor {
        version: TRACK_CURSOR_VERSION.to_string(),
        signature: signature.to_string(),
        offset,
    };
    let json = serde_json::to_vec(&payload).unwrap_or_default();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
}

pub(crate) fn decode_offset_cursor(
    raw_cursor: Option<&str>,
    expected_signature: &str,
) -> BackendResult<Option<usize>> {
    let raw = match raw_cursor {
        Some(value) if !value.trim().is_empty() => value.trim(),
        _ => return Ok(None),
    };
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| BackendError::Validation("invalid cursor token".to_string()))?;
    let cursor: OffsetPageCursor = serde_json::from_slice(&decoded)
        .map_err(|_| BackendError::Validation("invalid cursor payload".to_string()))?;
    if cursor.version != TRACK_CURSOR_VERSION {
        return Err(BackendError::Validation(
            "unsupported cursor version".to_string(),
        ));
    }
    if cursor.signature != expected_signature {
        return Err(BackendError::Validation(
            "cursor does not match current query".to_string(),
        ));
    }
    Ok(Some(cursor.offset))
}

pub(super) fn encode_track_page_cursor(
    signature: &str,
    file_path: &str,
    id: &str,
) -> Option<String> {
    if file_path.trim().is_empty() || id.trim().is_empty() {
        return None;
    }
    let payload = TrackPageCursor {
        version: TRACK_CURSOR_VERSION.to_string(),
        signature: signature.to_string(),
        file_path: file_path.to_string(),
        id: id.to_string(),
    };
    let json = serde_json::to_vec(&payload).ok()?;
    Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
}

pub(super) fn decode_track_page_cursor(
    raw_cursor: Option<&str>,
    expected_signature: &str,
) -> BackendResult<Option<TrackPageCursor>> {
    let raw = match raw_cursor {
        Some(value) if !value.trim().is_empty() => value.trim(),
        _ => return Ok(None),
    };
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| BackendError::Validation("invalid cursor token".to_string()))?;
    let cursor: TrackPageCursor = serde_json::from_slice(&decoded)
        .map_err(|_| BackendError::Validation("invalid cursor payload".to_string()))?;
    if cursor.version != TRACK_CURSOR_VERSION {
        return Err(BackendError::Validation(
            "unsupported cursor version".to_string(),
        ));
    }
    if cursor.signature != expected_signature {
        return Err(BackendError::Validation(
            "cursor does not match current query".to_string(),
        ));
    }
    if cursor.file_path.trim().is_empty() || cursor.id.trim().is_empty() {
        return Err(BackendError::Validation(
            "invalid cursor payload".to_string(),
        ));
    }
    Ok(Some(cursor))
}

impl BackendService {
    pub(super) fn backfill_track_fingerprints(&self) -> BackendResult<()> {
        let conn = self.db.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, artist, album FROM tracks WHERE match_fingerprint IS NULL OR match_fingerprint = ''",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        let pending = rows.collect::<Result<Vec<_>, _>>()?;
        if pending.is_empty() {
            return Ok(());
        }
        for (id, title, artist, album) in pending {
            let fp = build_track_match_fingerprint(&title, &artist, album.as_deref());
            conn.execute(
                "UPDATE tracks SET match_fingerprint = ?1 WHERE id = ?2",
                params![fp, id],
            )?;
        }
        Ok(())
    }
}
