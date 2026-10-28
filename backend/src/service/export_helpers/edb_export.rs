//! Writing the export eDB: playlists, content rows and cues.

use super::*;

pub fn write_edb_playlist(
    usb_root: &Path,
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
    mirror_playlist_entries: bool,
) -> BackendResult<WriteExportLibraryDbResult> {
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

    let mut unlock_warnings = Vec::<WarningEntry>::new();
    let Some(mut conn) = open_edb_rw(usb_root, &mut unlock_warnings) else {
        return Err(BackendError::Internal(format!(
            "unable to open eDB in read-write mode ({})",
            unlock_warnings
                .iter()
                .map(|w| w.message.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        )));
    };

    write_edb_playlist_with_conn(&mut conn, playlist, manifest, mirror_playlist_entries)
}

pub fn write_edb_playlist_with_conn(
    conn: &mut rusqlite::Connection,
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
    mirror_playlist_entries: bool,
) -> BackendResult<WriteExportLibraryDbResult> {
    let tx = conn.transaction()?;
    let mut results = write_edb_playlists_in_tx(
        &tx,
        &[EdbPlaylistWrite {
            playlist,
            manifest,
            mirror_playlist_entries,
            identity: None,
        }],
    )?;
    tx.commit()?;
    // Keep WAL mode after writes.
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
    results
        .pop()
        .ok_or_else(|| BackendError::Internal("eDB write produced no result".to_string()))
}

/// One playlist for [`write_edb_playlists_in_tx`].
pub struct EdbPlaylistWrite<'a> {
    pub playlist: &'a ExportPlaylistData,
    pub manifest: &'a ExportManifest,
    /// The playlist's members become exactly the manifest's tracks, in its
    /// order; otherwise new tracks are appended.
    pub mirror_playlist_entries: bool,
    /// The playlist row's id and `sequenceNo`, as the repair takes them from
    /// the PDB. Without one the row is found by name and moved first, as an
    /// export does.
    pub identity: Option<(i64, i64)>,
}

/// Write every one of `writes` into the eDB inside `tx`: playlist rows,
/// content rows (inserted, or updated where they changed), membership and
/// `property`. Nothing is committed; the caller commits or rolls back, so
/// several playlists (and the PDB written beside them) succeed or fail
/// together.
pub fn write_edb_playlists_in_tx(
    tx: &rusqlite::Transaction<'_>,
    writes: &[EdbPlaylistWrite<'_>],
) -> BackendResult<Vec<WriteExportLibraryDbResult>> {
    if !table_exists(tx, "playlist")
        || !table_exists(tx, "content")
        || !table_exists(tx, "playlist_content")
    {
        return Err(BackendError::Internal(
            "eDB is missing required playlist/content tables".to_string(),
        ));
    }
    let mut results = Vec::with_capacity(writes.len());
    for write in writes {
        let EdbPlaylistWrite {
            playlist,
            manifest,
            mirror_playlist_entries,
            identity,
        } = *write;
        let export_date_added = normalize_export_date(Some(&manifest.generated_at))
            .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
        let playlist_id = match identity {
            Some((id, sequence_no)) => {
                replace_export_playlist_row_with_identity(tx, playlist, id, sequence_no)?
            }
            None => upsert_export_playlist_row(tx, playlist)?,
        };
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
                    let key = canonicalize_track_path_identity(&path);
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
        let mut updated_content = 0usize;
        let mut linked_playlist_entries = 0usize;
        let playlist_content_columns = load_table_columns_tx(tx, "playlist_content")?;
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
        let content_columns = load_table_columns_tx(tx, "content")?;

        for track in &manifest.tracks {
            let track_path_key = canonicalize_track_path_identity(&track.exported_path);
            let preferred_existing = prev_content_id_by_path.get(&track_path_key).copied();
            let hinted_existing = match track.edb_content_id {
                Some(content_id) => tx
                    .query_row(
                        "SELECT content_id FROM content WHERE content_id = ?1",
                        params![content_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?,
                None => None,
            };
            let resolved_existing = match hinted_existing.or(preferred_existing) {
                Some(content_id) => Some(content_id),
                None => find_content_id_by_path(tx, &track.exported_path)?,
            };
            let content_id = match resolved_existing {
                Some(existing) => {
                    let was_previously_linked = previously_linked_ids.contains(&existing);
                    if update_existing_content_row(
                        tx,
                        existing,
                        track,
                        &content_columns,
                        Some(export_date_added.as_str()),
                    )? {
                        updated_content += 1;
                    }
                    // The update counters are the app's export bookkeeping; a
                    // track the repair carries over from the USB keeps its own.
                    if track.from_library {
                        // Vendor-authored exports apply +8 only to the appended
                        // export member while other existing rows receive +2.
                        let is_appended_export_member = track.position + 1 >= manifest.tracks.len();
                        let bump = if was_previously_linked || !is_appended_export_member {
                            2
                        } else {
                            8
                        };
                        for column in ["analysisDataUpdateCount", "informationUpdateCount"] {
                            if content_columns.contains(column) {
                                tx.execute(
                                    &format!(
                                        "UPDATE content SET {column} = (COALESCE(CAST({column} AS INTEGER), 0) + ?2) WHERE content_id = ?1"
                                    ),
                                    params![existing, bump],
                                )?;
                            }
                        }
                        if content_columns.contains("cueUpdateCount") {
                            tx.execute(
                                "UPDATE content SET cueUpdateCount = '' WHERE content_id = ?1",
                                params![existing],
                            )?;
                        }
                    }
                    existing
                }
                None => {
                    let id =
                        insert_content_from_template(tx, track, Some(export_date_added.as_str()))?;
                    inserted_content += 1;
                    id
                }
            };

            // The app owns the cues of tracks it exports from its library; a
            // track carried over from the USB keeps the cue rows it has.
            if track.from_library {
                write_edb_cues_for_content(tx, content_id, &track.cues, &content_columns)?;
            }

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
            link_playlist_content(tx, playlist_id, content_id, seq)?;
            linked_playlist_entries += 1;
        }
        results.push(WriteExportLibraryDbResult {
            inserted_content,
            updated_content,
            linked_playlist_entries,
            playlist_id,
            sort_order,
        });

    }

    let Some(manifest) = writes.first().map(|w| w.manifest) else {
        return Ok(results);
    };
    // Keep property summary in sync with actual content cardinality.
    if table_exists(tx, "property") {
        let property_columns = load_table_columns_tx(tx, "property")?;
        if property_columns.contains("numberOfContents") {
            tx.execute(
                "UPDATE property SET numberOfContents = (SELECT COUNT(1) FROM content)",
                [],
            )?;
        }
        // rekordbox shows the drive under this name. Without a name of
        // this drive's own (none set, or one carried over from another
        // drive) whatever is there stays, e.g. a name set in rekordbox.
        let usb_root = Path::new(&manifest.usb_root);
        if property_columns.contains("deviceName")
            && !crate::service::usb_identity::has_name_conflict(usb_root)
            && let Some(name) = crate::service::usb_identity::read_drive_name(usb_root)
        {
            tx.execute("UPDATE property SET deviceName = ?1", params![name])?;
        }
    }

    Ok(results)
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

/// Rewrite the eDB `cue` rows for a `content_id` from the app's cue list.
///
/// The app owns cue rows for tracks it exports: existing rows for this content
/// are dropped and replaced. Hot cues carry `colorTableIndex`; a hot cue's
/// memory point and the playback-start cue use `-1`; a memory cue carries its
/// colour or `-1`, and its name like a hot cue. Precise decoder seek anchors (`inMpegFrameNumber`, block offsets,
/// …) are left at 0 — the CDJ recomputes them from `inUsec` + the analysis
/// file. `content.cueUpdateCount` is bumped when any cue is written.
pub fn write_edb_cues_for_content(
    tx: &rusqlite::Transaction<'_>,
    content_id: i64,
    cues: &[TrackCue],
    content_columns: &HashSet<String>,
) -> BackendResult<()> {
    if !table_exists(tx, "cue") {
        return Ok(());
    }
    tx.execute("DELETE FROM cue WHERE content_id = ?1", params![content_id])?;
    if cues.is_empty() {
        return Ok(());
    }

    let cue_columns = load_table_columns_tx(tx, "cue")?;
    let mut cue_id = next_numeric_id(tx, "cue", "cue_id")?;

    let split = split_cues(cues);

    // Each hot cue is written twice: a memory point (kind 0) and, for the
    // first 8 by position, a hot-cue pad (kind 1) carrying the colour. Hot-slot
    // ordering is implicit in insert order. The playback-start cue is a lone
    // memory point, written first; each memory cue is a lone memory point
    // with its own colour, in position order among the hot cues.
    const MEMORY: (i64, i64) = (0, -1);
    let color_index = |cue: &TrackCue| cue.color_id.map_or(-1, i64::from);
    let mut points: Vec<(&TrackCue, Vec<(i64, i64)>)> = split
        .memory
        .into_iter()
        .map(|cue| (cue, vec![(0, color_index(cue))]))
        .chain(
            split
                .hot
                .into_iter()
                .map(|cue| (cue, vec![MEMORY, (1, i64::from(cue.color_id.unwrap_or(0)))])),
        )
        .collect();
    points.sort_by_key(|(cue, _)| cue.position_ms);
    let rows = split
        .start
        .map(|cue| (cue, vec![MEMORY]))
        .into_iter()
        .chain(points);
    for (cue, kinds) in rows {
        let in_usec = i64::from(cue.position_ms) * 1000;
        let in_frames_150 = ((f64::from(cue.position_ms) * 150.0) / 1000.0).round() as i64;
        let comment = cue.name.clone().unwrap_or_default();

        for (kind, color_table_index) in kinds {
            let mut fields: Vec<(&str, rusqlite::types::Value)> = vec![
                ("cue_id", cue_id.into()),
                ("content_id", content_id.into()),
                ("kind", kind.into()),
                ("colorTableIndex", color_table_index.into()),
                ("cueComment", comment.clone().into()),
                ("isActiveLoop", 0i64.into()),
                ("beatLoopNumerator", 0i64.into()),
                ("beatLoopDenominator", 0i64.into()),
                ("inUsec", in_usec.into()),
                ("outUsec", (-1i64).into()),
                ("in150FramePerSec", in_frames_150.into()),
                ("out150FramePerSec", (-1i64).into()),
            ];
            // Only keep columns that actually exist in this eDB's `cue` table.
            fields.retain(|(name, _)| cue_columns.contains(*name));

            let col_list = fields
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ");
            let placeholders = (1..=fields.len())
                .map(|n| format!("?{n}"))
                .collect::<Vec<_>>()
                .join(", ");
            let values: Vec<rusqlite::types::Value> = fields.into_iter().map(|(_, v)| v).collect();
            tx.execute(
                &format!("INSERT INTO cue ({col_list}) VALUES ({placeholders})"),
                rusqlite::params_from_iter(values.iter()),
            )?;
            cue_id += 1;
        }
    }

    if content_columns.contains("cueUpdateCount") {
        tx.execute(
            "UPDATE content SET cueUpdateCount = (COALESCE(CAST(cueUpdateCount AS INTEGER), 0) + 1) WHERE content_id = ?1",
            params![content_id],
        )?;
    }
    if content_columns.contains("isHotCueAutoLoadOn") {
        tx.execute(
            "UPDATE content SET isHotCueAutoLoadOn = 1 WHERE content_id = ?1",
            params![content_id],
        )?;
    }
    Ok(())
}

/// Two SQL values equal for a content column: integers, reals and numeric
/// text compare by value (rekordbox stores some numbers as text).
fn content_values_equal(a: &rusqlite::types::Value, b: &rusqlite::types::Value) -> bool {
    use rusqlite::types::Value;
    let text = |v: &Value| match v {
        Value::Null => None,
        Value::Integer(i) => Some(i.to_string()),
        Value::Real(f) if f.fract() == 0.0 && f.abs() < 1e15 => Some((*f as i64).to_string()),
        Value::Real(f) => Some(f.to_string()),
        Value::Text(t) => Some(t.trim().to_string()),
        Value::Blob(b) => Some(String::from_utf8_lossy(b).into_owned()),
    };
    text(a) == text(b)
}

/// Update an existing `content` row from `track`, writing only what changed:
/// each column the track gives a value for takes it when the row's differs.
/// A value the track doesn't carry leaves the row's own, never NULL or a
/// default over it. The app's own export touches (flags, `analysedBits`,
/// the export date as `dateAdded` on a row that has none, derived dates)
/// only apply to a track built from the library (`from_library`). Returns
/// whether the row changed.
pub fn update_existing_content_row(
    tx: &rusqlite::Transaction<'_>,
    content_id: i64,
    track: &ExportManifestTrack,
    content_columns: &HashSet<String>,
    export_date_added: Option<&str>,
) -> BackendResult<bool> {
    use rusqlite::types::Value;
    let text = |value: Option<&str>| {
        value
            .map(|v| sanitize_metadata(v).trim().to_string())
            .filter(|v| !v.is_empty())
            .map(Value::Text)
    };
    let int = |value: Option<i64>| value.map(Value::Integer);
    let from_library = track.from_library;

    // (column, value) the row should have; columns the track has no value
    // for are left out. `if_empty` columns are only filled, never replaced.
    let mut desired: Vec<(&'static str, Value)> = Vec::new();
    let mut if_empty: Vec<(&'static str, Value)> = Vec::new();
    let mut want = |column: &'static str, value: Option<Value>| {
        if let Some(value) = value
            && content_columns.contains(column)
        {
            desired.push((column, value));
        }
    };

    want("title", text(Some(&track.title)));
    want(
        "path",
        Some(track.exported_path.as_str())
            .filter(|p| !p.trim().is_empty())
            .map(|p| Value::Text(p.to_string())),
    );
    want(
        "fileName",
        Some(track.exported_path.as_str())
            .filter(|p| !p.trim().is_empty())
            .map(|p| Value::Text(content_file_name(p))),
    );
    want(
        "analysisDataFilePath",
        track
            .waveform_path
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .map(|p| Value::Text(p.to_string())),
    );
    want("bpmx100", int(track.bpm.map(|bpm| (bpm * 100.0).round() as i64)));
    want("length", int(track.duration_ms.map(duration_ms_to_seconds)));
    want(
        "artist_id_artist",
        match track.artist.trim() {
            "" => None,
            artist => int(find_or_insert_artist(tx, artist)?),
        },
    );
    want(
        "key_id",
        int(find_key_id_by_name(tx, track.key.as_deref())?),
    );
    if let Some(path) = track
        .artwork_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        && (content_columns.contains("image_id") || content_columns.contains("imageFilePath_id"))
    {
        let image_id = find_or_insert_image(tx, path)?;
        want("image_id", int(image_id));
        want("imageFilePath_id", int(image_id));
    }
    want("fileSize", int(track.file_size_bytes));
    want("fileType", int(track.file_type));
    want("trackNo", int(track.track_number.map(i64::from)));
    want("discNo", int(track.disc_number.map(i64::from)));
    want("bitrate", int(track.bitrate_kbps.map(i64::from)));
    want("samplingRate", int(track.sample_rate_hz.map(i64::from)));
    want("bitDepth", int(track.bit_depth.map(i64::from)));
    want("subtitle", text(track.subtitle.as_deref()));
    want("titleForSearch", text(track.title_for_search.as_deref()));
    want("isrc", text(track.isrc.as_deref()));
    want("djComment", text(track.comment.as_deref()));
    want("kuvoDeliveryComment", text(track.kuvo_delivery_comment.as_deref()));
    if from_library {
        want(
            "releaseYear",
            int(resolve_track_release_year_for_export(track).map(i64::from)),
        );
        want(
            "releaseDate",
            text(resolve_track_release_date_for_export(track).as_deref()),
        );
    } else {
        want("releaseYear", int(track.release_year.map(i64::from)));
        want("releaseDate", text(track.release_date.as_deref()));
    }
    want(
        "dateCreated",
        text(
            track
                .date_created
                .clone()
                .or_else(|| {
                    from_library
                        .then(|| resolve_track_created_date_for_export(track))
                        .flatten()
                })
                .as_deref(),
        ),
    );
    match normalize_export_date(track.date_added.as_deref()) {
        Some(date) => want("dateAdded", Some(Value::Text(date))),
        None if from_library && content_columns.contains("dateAdded") => {
            if let Some(date) = normalize_export_date(export_date_added) {
                if_empty.push(("dateAdded", Value::Text(date)));
            }
        }
        None => {}
    }
    want("contentLink", int(track.content_link));
    want("masterContentId", int(track.master_content_id));
    want("masterDbId", int(track.master_db_id));
    // Rating and colour only when the library has one: the app doesn't set
    // them, so a rekordbox row's own values stay (and match its PDB row).
    want("rating", int(track.rating.map(i64::from)));
    want("color_id", int(track.color_id.map(i64::from)));
    want("djPlayCount", int(track.dj_play_count.map(i64::from)));
    want(
        "artist_id_lyricist",
        int(track.artist_id_lyricist.map(i64::from)),
    );
    want(
        "artist_id_originalArtist",
        int(track.artist_id_original_artist.map(i64::from)),
    );
    let named_artist = |name: Option<&str>| -> BackendResult<Option<i64>> {
        match name.map(str::trim).filter(|v| !v.is_empty()) {
            Some(name) => find_or_insert_artist(tx, name),
            None => Ok(None),
        }
    };
    want(
        "artist_id_remixer",
        int(named_artist(track.remixer.as_deref())?
            .or(track.artist_id_remixer.map(i64::from))),
    );
    want(
        "artist_id_composer",
        int(named_artist(track.composer.as_deref())?
            .or(track.artist_id_composer.map(i64::from))),
    );
    // By name, as the PDB row is: the library's `genre_id` is never set, so
    // writing it would clear the genre the PDB keeps.
    want(
        "genre_id",
        match track.genre.as_deref().filter(|v| !v.trim().is_empty()) {
            Some(genre) => int(crate::edb::find_or_insert_genre(tx, genre)?),
            None => None,
        },
    );
    want(
        "label_id",
        match track.label.as_deref().filter(|v| !v.trim().is_empty()) {
            Some(label) => int(crate::edb::find_or_insert_label(tx, label)?),
            None => int(track.label_id.map(i64::from)),
        },
    );
    if from_library {
        // Mark exported tracks as having the newer analysis generation so
        // Vendor software can enable the modern waveform path for generated ANLZ.
        want(
            "analysedBits",
            Some(Value::Integer(if track.waveform_path.is_some() { 41 } else { 0 })),
        );
        want("isHotCueAutoLoadOn", Some(Value::Integer(1)));
        want("isKuvoDeliverStatusOn", Some(Value::Integer(1)));
        want("hasModified", Some(Value::Integer(0)));
    }
    // The album is only filled in: a row's own album stays.
    if content_columns.contains("album_id")
        && let Some(album) = track.album.as_deref().filter(|v| !v.trim().is_empty())
    {
        let existing_album_id: Option<i64> = tx
            .query_row(
                "SELECT album_id FROM content WHERE content_id = ?1",
                params![content_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        if existing_album_id.is_none()
            && let Some(album_id) = find_or_insert_album(tx, Some(album), Some(&track.artist))?
        {
            if_empty.push(("album_id", Value::Integer(album_id)));
        }
    }

    let columns: Vec<&'static str> = desired
        .iter()
        .chain(&if_empty)
        .map(|(column, _)| *column)
        .collect();
    if columns.is_empty() {
        return Ok(false);
    }
    let current: Vec<Value> = tx.query_row(
        &format!(
            "SELECT {} FROM content WHERE content_id = ?1",
            columns
                .iter()
                .map(|c| format!("\"{c}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        params![content_id],
        |row| (0..columns.len()).map(|i| row.get::<_, Value>(i)).collect(),
    )?;
    let is_empty = |v: &Value| match v {
        Value::Null => true,
        Value::Text(t) => t.trim().is_empty(),
        _ => false,
    };
    let changes: Vec<(&'static str, Value)> = desired
        .into_iter()
        .map(|change| (change, false))
        .chain(if_empty.into_iter().map(|change| (change, true)))
        .zip(&current)
        .filter(|(((_, value), only_if_empty), now)| {
            if *only_if_empty {
                is_empty(now)
            } else {
                !content_values_equal(value, now)
            }
        })
        .map(|((change, _), _)| change)
        .collect();
    if changes.is_empty() {
        return Ok(false);
    }
    let assignments = changes
        .iter()
        .enumerate()
        .map(|(i, (column, _))| format!("\"{column}\" = ?{}", i + 2))
        .collect::<Vec<_>>()
        .join(", ");
    let mut values: Vec<Value> = vec![Value::Integer(content_id)];
    values.extend(changes.into_iter().map(|(_, value)| value));
    tx.execute(
        &format!("UPDATE content SET {assignments} WHERE content_id = ?1"),
        rusqlite::params_from_iter(values),
    )?;
    Ok(true)
}
