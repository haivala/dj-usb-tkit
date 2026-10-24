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
    let export_date_added = normalize_export_date(Some(&manifest.generated_at))
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
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

        write_edb_cues_for_content(&tx, content_id, &track.cues, &content_columns)?;

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
    if content_columns.contains("analysisDataFilePath") && track.waveform_path.is_some() {
        tx.execute(
            "UPDATE content SET analysisDataFilePath = ?1 WHERE content_id = ?2",
            params![track.waveform_path, content_id],
        )?;
    }
    if content_columns.contains("bpmx100")
        && let Some(bpm) = track.bpm
    {
        let bpmx100 = (bpm * 100.0).round() as i64;
        tx.execute(
            "UPDATE content SET bpmx100 = ?1 WHERE content_id = ?2",
            params![bpmx100, content_id],
        )?;
    }
    if content_columns.contains("length")
        && let Some(duration_ms) = track.duration_ms
    {
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
        && let Some(cl) = track.content_link
    {
        tx.execute(
            "UPDATE content SET contentLink = ?1 WHERE content_id = ?2",
            params![cl, content_id],
        )?;
    }
    if content_columns.contains("masterContentId")
        && let Some(mci) = track.master_content_id
    {
        tx.execute(
            "UPDATE content SET masterContentId = ?1 WHERE content_id = ?2",
            params![mci, content_id],
        )?;
    }
    if content_columns.contains("masterDbId")
        && let Some(mdb) = track.master_db_id
    {
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
    // Rating and colour only when the library has one: the app doesn't set
    // them, so a rekordbox row's own values stay (and match its PDB row).
    if content_columns.contains("rating")
        && let Some(rating) = track.rating
    {
        tx.execute(
            "UPDATE content SET rating = ?1 WHERE content_id = ?2",
            params![rating, content_id],
        )?;
    }
    if content_columns.contains("djPlayCount") {
        tx.execute(
            "UPDATE content SET djPlayCount = ?1 WHERE content_id = ?2",
            params![track.dj_play_count.unwrap_or(0), content_id],
        )?;
    }
    if content_columns.contains("color_id")
        && let Some(color_id) = track.color_id
    {
        tx.execute(
            "UPDATE content SET color_id = ?1 WHERE content_id = ?2",
            params![color_id, content_id],
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
    // By name, as the PDB row is: the library's `genre_id` is never set, so
    // writing it would clear the genre the PDB keeps.
    if content_columns.contains("genre_id")
        && let Some(genre_name) = track.genre.as_deref().filter(|v| !v.trim().is_empty())
        && let Some(genre_id) = crate::edb::find_or_insert_genre(tx, genre_name)?
    {
        tx.execute(
            "UPDATE content SET genre_id = ?1 WHERE content_id = ?2",
            params![genre_id, content_id],
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
