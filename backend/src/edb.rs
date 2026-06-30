//! Export database (eDB/SQLite) helpers and CRUD operations.

use std::collections::HashMap;
use std::path::Path;

use chrono::TimeZone;
use rusqlite::{OptionalExtension, params, types::Value};
use serde::Serialize;

use crate::error::{BackendError, BackendResult};
use crate::metadata::sanitize_metadata;
use crate::models::UsbTrack;
use crate::service::usb_utils::resolve_usb_side_path;
use crate::service::usb_vendor_compat::{
    DEFAULT_MASTER_DB_KEY, DEFAULT_USB_EDB_KEY, USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR,
};

// -----------------------------------------------------------------------------
// eDB data contracts
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ExportPlaylistData {
    pub id: String,
    pub name: String,
    pub tracks: Vec<ExportTrackData>,
}

#[derive(Debug, Clone)]
pub struct ExportTrackData {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub file_path: String,
    pub file_name: String,
    pub file_modified_at: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub sample_rate_hz: Option<u32>,
    pub bit_depth: Option<u8>,
    pub bitrate_kbps: Option<u32>,
    pub disc_number: Option<u32>,
    pub subtitle: Option<String>,
    pub comment: Option<String>,
    pub title_for_search: Option<String>,
    pub kuvo_delivery_comment: Option<String>,
    pub dj_play_count: Option<u32>,
    pub rating: Option<u32>,
    pub color_id: Option<u32>,
    pub artist_id_lyricist: Option<u32>,
    pub artist_id_original_artist: Option<u32>,
    pub artist_id_remixer: Option<u32>,
    pub artist_id_composer: Option<u32>,
    pub genre_id: Option<u32>,
    pub genre: Option<String>,
    pub label_id: Option<u32>,
    pub isrc: Option<String>,
    pub release_year: Option<u32>,
    pub release_date: Option<String>,
    pub recorded_date: Option<String>,
    pub file_type: Option<i64>,
    pub artwork_path: Option<String>,
    pub waveform_peaks_path: Option<String>,
    pub duration_ms: Option<u64>,
    pub first_beat_ms: Option<u32>,
    pub position: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportManifestTrack {
    pub id: String,
    pub master_db_id: Option<i64>,
    pub master_content_id: Option<i64>,
    pub content_link: Option<i64>,
    pub position: usize,
    pub track_number: Option<u32>,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub source_path: String,
    pub exported_path: String,
    pub file_modified_at: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub sample_rate_hz: Option<u32>,
    pub bit_depth: Option<u8>,
    pub bitrate_kbps: Option<u32>,
    pub disc_number: Option<u32>,
    pub subtitle: Option<String>,
    pub comment: Option<String>,
    pub title_for_search: Option<String>,
    pub kuvo_delivery_comment: Option<String>,
    pub dj_play_count: Option<u32>,
    pub rating: Option<u32>,
    pub color_id: Option<u32>,
    pub artist_id_lyricist: Option<u32>,
    pub artist_id_original_artist: Option<u32>,
    pub artist_id_remixer: Option<u32>,
    pub artist_id_composer: Option<u32>,
    pub genre_id: Option<u32>,
    pub genre: Option<String>,
    pub label_id: Option<u32>,
    pub isrc: Option<String>,
    pub release_year: Option<u32>,
    pub release_date: Option<String>,
    pub recorded_date: Option<String>,
    pub file_type: Option<i64>,
    pub owns_exported_media: bool,
    pub owns_artwork: bool,
    pub owns_waveform: bool,
    pub artwork_path: Option<String>,
    pub waveform_path: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ExportDbPlaylist {
    pub playlist_id: i64,
    pub sort_order: i64,
    pub tracks: Vec<UsbTrack>,
}

// -----------------------------------------------------------------------------
// eDB open/unlock helpers
// -----------------------------------------------------------------------------

/// Validate that a SQLCipher key contains only safe characters (alphanumeric).
pub fn is_safe_sqlcipher_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric())
}

pub fn edb_path_from_usb_root(usb_root: &Path) -> std::path::PathBuf {
    usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("exportLibrary.db")
}

fn has_schema(conn: &rusqlite::Connection) -> bool {
    conn.query_row(
        "SELECT COUNT(1) FROM sqlite_master WHERE type IN ('table','view')",
        [],
        |row| row.get::<_, i64>(0),
    )
    .ok()
    .unwrap_or(0)
        > 0
}

fn effective_sqlcipher_keys() -> Vec<String> {
    vec![
        DEFAULT_USB_EDB_KEY.to_string(),
        DEFAULT_MASTER_DB_KEY.to_string(),
    ]
}

pub fn open_edb(path: &Path, warnings: &mut Vec<String>) -> Option<rusqlite::Connection> {
    if !path.exists() {
        return None;
    }

    let open_ro =
        || rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY);
    if let Ok(plain) = open_ro() {
        if has_schema(&plain) {
            warnings.push("eDB opened without SQLCipher key".to_string());
            return Some(plain);
        }
    }

    let merged_keys = effective_sqlcipher_keys();
    let mut attempts = 0usize;
    for raw_key in &merged_keys {
        if !is_safe_sqlcipher_key(raw_key) {
            warnings.push("skipping SQLCipher key with unsafe characters".to_string());
            continue;
        }
        attempts += 1;
        let batch = format!("PRAGMA key='{raw_key}';");
        let Ok(candidate) = open_ro() else { continue };
        if candidate.execute_batch(&batch).is_err() {
            continue;
        }
        if has_schema(&candidate) {
            let key_label = if raw_key == DEFAULT_USB_EDB_KEY {
                "default USB export"
            } else {
                "default master"
            };
            warnings.push(format!("eDB unlocked via SQLCipher with {key_label} key"));
            return Some(candidate);
        }
    }
    warnings.push(format!(
        "eDB unreadable after trying {attempts} SQLCipher key(s)"
    ));
    None
}

pub fn open_edb_from_usb_root(
    usb_root: &Path,
    warnings: &mut Vec<String>,
) -> Option<rusqlite::Connection> {
    let path = edb_path_from_usb_root(usb_root);
    open_edb(&path, warnings)
}

pub fn open_edb_rw(usb_root: &Path, warnings: &mut Vec<String>) -> Option<rusqlite::Connection> {
    let db_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("exportLibrary.db");
    if !db_path.exists() {
        return None;
    }

    let open_rw = || {
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
    };
    let has_schema = |conn: &rusqlite::Connection| {
        conn.query_row(
            "SELECT COUNT(1)
             FROM sqlite_master
             WHERE type IN ('table','view')
               AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .ok()
        .unwrap_or(0)
            > 0
    };

    if let Ok(plain) = open_rw() {
        if has_schema(&plain) {
            warnings.push("eDB opened read-write without SQLCipher key".to_string());
            return Some(plain);
        }
    }

    let merged_keys = effective_sqlcipher_keys();
    for raw_key in &merged_keys {
        if !is_safe_sqlcipher_key(raw_key) {
            warnings.push("skipping SQLCipher key with unsafe characters".to_string());
            continue;
        }
        let batch = format!("PRAGMA key='{raw_key}';");
        let Ok(candidate) = open_rw() else { continue };
        if candidate.execute_batch(&batch).is_err() {
            continue;
        }
        if has_schema(&candidate) {
            warnings.push("eDB opened read-write with SQLCipher key".to_string());
            return Some(candidate);
        }
    }
    None
}

pub fn try_read_track_index_from_edb(
    usb_root: &Path,
    warnings: &mut Vec<String>,
) -> Option<HashMap<u32, UsbTrack>> {
    let conn = open_edb_from_usb_root(usb_root, warnings)?;
    let has_length_col = conn
        .query_row(
            "SELECT COUNT(1) FROM pragma_table_info('content') WHERE name = 'length'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .ok()
        .unwrap_or(0)
        > 0;
    let length_expr = if has_length_col { "c.length" } else { "NULL" };
    let image_fk_col = if conn
        .query_row(
            "SELECT COUNT(1) FROM pragma_table_info('content') WHERE name = 'imageFilePath_id'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .ok()
        .unwrap_or(0)
        > 0
    {
        "c.imageFilePath_id"
    } else {
        "c.image_id"
    };
    let sql = format!(
        r#"
        SELECT
          c.content_id,
          c.title,
          ar.name AS artist_name,
          al.name AS album_name,
          c.bpmx100,
          k.name AS key_name,
          c.path,
          img.path AS image_path,
          c.analysisDataFilePath,
          {length_expr} AS length_seconds
        FROM content c
        LEFT JOIN artist ar ON ar.artist_id = c.artist_id_artist
        LEFT JOIN album al ON al.album_id = c.album_id
        LEFT JOIN "key" k ON k.key_id = c.key_id
        LEFT JOIN image img ON img.image_id = {image_fk_col}
        "#
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(err) => {
            warnings.push(format!("eDB content query failed: {err}"));
            return None;
        }
    };

    let rows = match stmt.query_map([], |row| {
        let content_id: i64 = row.get(0)?;
        let title: Option<String> = row.get(1)?;
        let artist: Option<String> = row.get(2)?;
        let album: Option<String> = row.get(3)?;
        let bpmx100: Option<i64> = row.get(4)?;
        let key_name: Option<String> = row.get(5)?;
        let path: Option<String> = row.get(6)?;
        let image_path: Option<String> = row.get(7)?;
        let analysis_path: Option<String> = row.get(8)?;
        let length_seconds: Option<i64> = row.get(9)?;
        let resolved_file_path = path
            .as_deref()
            .and_then(|p| resolve_usb_side_path(usb_root, p))
            .unwrap_or_default();
        Ok((
            content_id,
            UsbTrack {
                id: content_id.to_string(),
                local_track_id: None,
                title: title.unwrap_or_else(|| "Unknown Title".to_string()),
                artist: artist.unwrap_or_else(|| "Unknown Artist".to_string()),
                album,
                track_number: None,
                bpm: bpmx100.map(|v| v as f64 / 100.0),
                key: key_name,
                file_path: resolved_file_path,
                usb_media_path: path,
                artwork_path: image_path
                    .as_deref()
                    .and_then(|p| resolve_usb_side_path(usb_root, p)),
                artwork_data_url: None,
                waveform_peaks_path: analysis_path
                    .as_deref()
                    .and_then(|p| resolve_usb_side_path(usb_root, p)),
                usb_analysis_path: analysis_path
                    .as_deref()
                    .and_then(|p| resolve_usb_side_path(usb_root, p)),
                usb_analysis_path_raw: analysis_path,
                waveform_preview: None,
                duration_ms: length_seconds
                    .filter(|v| *v > 0)
                    .map(|v| (v as u64).saturating_mul(1000)),
            },
        ))
    }) {
        Ok(r) => r,
        Err(err) => {
            warnings.push(format!("eDB content row mapping failed: {err}"));
            return None;
        }
    };

    let mut index = HashMap::<u32, UsbTrack>::new();
    for row in rows {
        let Ok((content_id, track)) = row else {
            continue;
        };
        if content_id <= 0 {
            continue;
        }
        index.insert(content_id as u32, track);
    }

    if !index.is_empty() {
        warnings.push(format!(
            "loaded {} track metadata rows from eDB content index",
            index.len()
        ));
        Some(index)
    } else {
        None
    }
}

// -----------------------------------------------------------------------------
// eDB read/index helpers
// -----------------------------------------------------------------------------

pub fn try_read_content_date_created_index_from_edb(
    usb_root: &Path,
    warnings: &mut Vec<String>,
) -> Option<HashMap<u32, String>> {
    let conn = open_edb_from_usb_root(usb_root, warnings)?;
    let has_date_created_col = conn
        .query_row(
            "SELECT COUNT(1) FROM pragma_table_info('content') WHERE name = 'dateCreated'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .ok()
        .unwrap_or(0)
        > 0;
    if !has_date_created_col {
        warnings.push("eDB content.dateCreated column not found".to_string());
        return None;
    }

    let mut stmt = match conn.prepare(
        r#"
        SELECT content_id, dateCreated
        FROM content
        WHERE dateCreated IS NOT NULL AND TRIM(dateCreated) != ''
        "#,
    ) {
        Ok(s) => s,
        Err(err) => {
            warnings.push(format!("eDB dateCreated query failed: {err}"));
            return None;
        }
    };

    let rows = match stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    }) {
        Ok(r) => r,
        Err(err) => {
            warnings.push(format!("eDB dateCreated row mapping failed: {err}"));
            return None;
        }
    };

    let mut out = HashMap::<u32, String>::new();
    for row in rows.flatten() {
        let (content_id, date_created) = row;
        if content_id <= 0 {
            continue;
        }
        let trimmed = date_created.trim();
        if trimmed.is_empty() {
            continue;
        }
        out.insert(content_id as u32, trimmed.to_string());
    }

    if !out.is_empty() {
        warnings.push(format!(
            "loaded {} dateCreated value(s) from eDB content index",
            out.len()
        ));
        Some(out)
    } else {
        None
    }
}

pub fn try_read_playlists_with_metadata_from_edb(
    usb_root: &Path,
    warnings: &mut Vec<String>,
) -> Option<HashMap<String, ExportDbPlaylist>> {
    try_read_playlists_with_metadata_from_edb_internal(usb_root, warnings, true)
}

pub fn try_read_playlists_with_metadata_from_edb_db_only(
    usb_root: &Path,
    warnings: &mut Vec<String>,
) -> Option<HashMap<String, ExportDbPlaylist>> {
    try_read_playlists_with_metadata_from_edb_internal(usb_root, warnings, false)
}

fn try_read_playlists_with_metadata_from_edb_internal(
    usb_root: &Path,
    warnings: &mut Vec<String>,
    resolve_paths: bool,
) -> Option<HashMap<String, ExportDbPlaylist>> {
    let conn = open_edb_from_usb_root(usb_root, warnings)?;
    let has_length_col = conn
        .query_row(
            "SELECT COUNT(1) FROM pragma_table_info('content') WHERE name = 'length'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .ok()
        .unwrap_or(0)
        > 0;
    let length_expr = if has_length_col { "c.length" } else { "NULL" };
    let has_track_no_col = conn
        .query_row(
            "SELECT COUNT(1) FROM pragma_table_info('content') WHERE name = 'trackNo'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .ok()
        .unwrap_or(0)
        > 0;
    let track_no_expr = if has_track_no_col {
        "c.trackNo"
    } else {
        "NULL"
    };
    let image_fk_col = if conn
        .query_row(
            "SELECT COUNT(1) FROM pragma_table_info('content') WHERE name = 'imageFilePath_id'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .ok()
        .unwrap_or(0)
        > 0
    {
        "c.imageFilePath_id"
    } else {
        "c.image_id"
    };

    let mut playlist_stmt = match conn.prepare(
        r#"
        SELECT playlist_id, name, COALESCE(sequenceNo, 0)
        FROM playlist
        WHERE attribute = 0
        ORDER BY sequenceNo ASC
        "#,
    ) {
        Ok(s) => s,
        Err(err) => {
            warnings.push(format!("eDB schema query failed: {err}"));
            return None;
        }
    };

    let playlist_rows = match playlist_stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    }) {
        Ok(r) => r,
        Err(err) => {
            warnings.push(format!("eDB playlist query failed: {err}"));
            return None;
        }
    };

    let mut out = HashMap::<String, ExportDbPlaylist>::new();
    for playlist_row in playlist_rows {
        let Ok((playlist_id, playlist_name, sort_order)) = playlist_row else {
            continue;
        };
        let mut tracks = Vec::<UsbTrack>::new();
        let track_sql = format!(
            r#"
            SELECT
              c.content_id,
              c.title,
              ar.name AS artist_name,
              al.name AS album_name,
              c.bpmx100,
              k.name AS key_name,
              c.path,
              img.path AS image_path,
              c.analysisDataFilePath,
              {length_expr} AS length_seconds,
              {track_no_expr} AS track_no
            FROM playlist_content pc
            JOIN content c ON c.content_id = pc.content_id
            LEFT JOIN artist ar ON ar.artist_id = c.artist_id_artist
            LEFT JOIN album al ON al.album_id = c.album_id
            LEFT JOIN "key" k ON k.key_id = c.key_id
            LEFT JOIN image img ON img.image_id = {image_fk_col}
            WHERE pc.playlist_id = ?1
            ORDER BY pc.sequenceNo ASC
            "#
        );
        let mut track_stmt = match conn.prepare(&track_sql) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let rows = match track_stmt.query_map([playlist_id], |row| {
            let content_id: i64 = row.get(0)?;
            let title: Option<String> = row.get(1)?;
            let artist: Option<String> = row.get(2)?;
            let album: Option<String> = row.get(3)?;
            let bpmx100: Option<i64> = row.get(4)?;
            let key_name: Option<String> = row.get(5)?;
            let path: Option<String> = row.get(6)?;
            let image_path: Option<String> = row.get(7)?;
            let analysis_path: Option<String> = row.get(8)?;
            let length_seconds: Option<i64> = row.get(9)?;
            let track_no: Option<i64> = row.get(10)?;
            let resolved_image_path = if resolve_paths {
                image_path
                    .as_deref()
                    .and_then(|p| resolve_usb_side_path(usb_root, p))
            } else {
                image_path.clone()
            };
            let resolved_analysis_path = if resolve_paths {
                analysis_path
                    .as_deref()
                    .and_then(|p| resolve_usb_side_path(usb_root, p))
            } else {
                analysis_path.clone()
            };
            let resolved_file_path = if resolve_paths {
                path.as_deref()
                    .and_then(|p| resolve_usb_side_path(usb_root, p))
                    .unwrap_or_default()
            } else {
                path.clone().unwrap_or_default()
            };
            Ok(UsbTrack {
                id: content_id.to_string(),
                local_track_id: None,
                title: title.unwrap_or_else(|| "Unknown Title".to_string()),
                artist: artist.unwrap_or_else(|| "Unknown Artist".to_string()),
                album,
                track_number: track_no.and_then(|value| u32::try_from(value).ok()),
                bpm: bpmx100.map(|v| v as f64 / 100.0),
                key: key_name,
                file_path: resolved_file_path,
                usb_media_path: path,
                artwork_data_url: None,
                artwork_path: resolved_image_path,
                waveform_peaks_path: resolved_analysis_path.clone(),
                usb_analysis_path: resolved_analysis_path,
                usb_analysis_path_raw: analysis_path,
                waveform_preview: None,
                duration_ms: length_seconds
                    .filter(|v| *v > 0)
                    .map(|v| (v as u64).saturating_mul(1000)),
            })
        }) {
            Ok(r) => r,
            Err(_) => continue,
        };

        for row in rows {
            if let Ok(track) = row {
                tracks.push(track);
            }
        }
        let existing = out
            .entry(playlist_name)
            .or_insert_with(|| ExportDbPlaylist {
                playlist_id,
                sort_order,
                tracks: Vec::new(),
            });
        if sort_order < existing.sort_order
            || (sort_order == existing.sort_order && playlist_id < existing.playlist_id)
        {
            existing.playlist_id = playlist_id;
            existing.sort_order = sort_order;
        }
        let mut seen = existing
            .tracks
            .iter()
            .map(|track| track.id.clone())
            .collect::<std::collections::HashSet<_>>();
        for track in tracks {
            if seen.insert(track.id.clone()) {
                existing.tracks.push(track);
            }
        }
    }

    if !out.is_empty() {
        warnings.push(format!("loaded {} playlist(s) from eDB", out.len()));
        Some(out)
    } else {
        None
    }
}

pub fn table_exists(conn: &rusqlite::Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(1) FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |row| row.get::<_, i64>(0),
    )
    .ok()
    .unwrap_or(0)
        > 0
}

// -----------------------------------------------------------------------------
// eDB generic SQL helpers
// -----------------------------------------------------------------------------

pub fn load_table_columns(conn: &rusqlite::Connection, table: &str) -> BackendResult<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut out = Vec::<String>::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn load_table_row_template(
    conn: &rusqlite::Connection,
    table: &str,
    columns: &[String],
) -> BackendResult<HashMap<String, Value>> {
    let select_cols = columns
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {select_cols} FROM {table} LIMIT 1");
    let mut stmt = conn.prepare(&sql)?;
    let template = stmt
        .query_row([], |row| {
            let mut values = HashMap::<String, Value>::new();
            for (idx, column) in columns.iter().enumerate() {
                let value = row.get::<usize, Value>(idx)?;
                values.insert(column.clone(), value);
            }
            Ok(values)
        })
        .optional()?;
    template.ok_or_else(|| {
        BackendError::Internal(format!(
            "cannot export to table '{table}' because it has no template rows"
        ))
    })
}

pub fn map_values_for_insert(columns: &[String], values: &HashMap<String, Value>) -> Vec<Value> {
    columns
        .iter()
        .map(|c| values.get(c).cloned().unwrap_or(Value::Null))
        .collect::<Vec<_>>()
}

pub fn next_numeric_id(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
) -> BackendResult<i64> {
    let sql = format!("SELECT COALESCE(MAX({column}), 0) + 1 FROM {table}");
    let next = conn.query_row(&sql, [], |row| row.get::<_, i64>(0))?;
    Ok(next.max(1))
}

pub fn preferred_export_playlist_row_id(
    conn: &rusqlite::Connection,
    playlist_name: &str,
) -> BackendResult<Option<i64>> {
    let playlist_name = sanitize_metadata(playlist_name);
    conn.query_row(
        r#"
        SELECT p.playlist_id
        FROM playlist p
        LEFT JOIN playlist_content pc ON pc.playlist_id = p.playlist_id
        WHERE p.name = ?1 AND COALESCE(p.attribute, 0) = 0
        GROUP BY p.playlist_id
        ORDER BY COUNT(pc.content_id) DESC, p.playlist_id ASC
        LIMIT 1
        "#,
        params![playlist_name.as_ref()],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map_err(BackendError::from)
}

fn move_export_playlist_row_to_front(
    conn: &rusqlite::Connection,
    playlist_id: i64,
    columns: &[String],
) -> BackendResult<()> {
    if !columns.iter().any(|c| c == "sequenceNo") {
        return Ok(());
    }

    if columns.iter().any(|c| c == "playlist_id_parent") {
        let parent_id = conn
            .query_row(
                "SELECT COALESCE(playlist_id_parent, 0) FROM playlist WHERE playlist_id = ?1",
                params![playlist_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        conn.execute(
            "UPDATE playlist
             SET sequenceNo = COALESCE(sequenceNo, 0) + 1
             WHERE attribute = 0
               AND COALESCE(playlist_id_parent, 0) = ?1
               AND playlist_id <> ?2",
            params![parent_id, playlist_id],
        )?;
    } else {
        conn.execute(
            "UPDATE playlist
             SET sequenceNo = COALESCE(sequenceNo, 0) + 1
             WHERE attribute = 0 AND playlist_id <> ?1",
            params![playlist_id],
        )?;
    }

    conn.execute(
        "UPDATE playlist SET sequenceNo = 0 WHERE playlist_id = ?1 AND attribute = 0",
        params![playlist_id],
    )?;
    Ok(())
}

// -----------------------------------------------------------------------------
// eDB export CRUD helpers
// -----------------------------------------------------------------------------

fn sanitized_metadata_text(value: &str) -> String {
    sanitize_metadata(value).into_owned()
}

fn sanitized_optional_metadata_text(value: Option<&str>) -> Option<String> {
    value.map(sanitized_metadata_text)
}

fn sanitized_optional_nonempty_metadata_text(value: Option<&str>) -> Option<String> {
    value
        .map(sanitized_metadata_text)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn upsert_export_playlist_row(
    conn: &rusqlite::Connection,
    playlist: &ExportPlaylistData,
) -> BackendResult<i64> {
    let columns = load_table_columns(conn, "playlist")?;
    let playlist_name = sanitized_metadata_text(&playlist.name);
    if let Some(id) = preferred_export_playlist_row_id(conn, &playlist_name)? {
        move_export_playlist_row_to_front(conn, id, &columns)?;
        return Ok(id);
    }

    let next_id = next_numeric_id(conn, "playlist", "playlist_id")?;
    match load_table_row_template(conn, "playlist", &columns) {
        Ok(mut values) => {
            values.insert("playlist_id".to_string(), Value::Integer(next_id));
            if values.contains_key("name") {
                values.insert("name".to_string(), Value::Text(playlist_name.clone()));
            }
            if values.contains_key("attribute") {
                values.insert("attribute".to_string(), Value::Integer(0));
            }
            if values.contains_key("playlist_id_parent") {
                values.insert("playlist_id_parent".to_string(), Value::Integer(0));
            }
            if values.contains_key("sequenceNo") {
                values.insert("sequenceNo".to_string(), Value::Integer(0));
            }
            if values.contains_key("image_id") {
                values.insert("image_id".to_string(), Value::Integer(0));
            }
            dynamic_insert(conn, "playlist", &columns, &values)?;
        }
        Err(_) => {
            let next_seq = 0i64;
            if columns.iter().any(|c| c == "playlist_id_parent") {
                conn.execute(
                    "INSERT INTO playlist (playlist_id, name, attribute, playlist_id_parent, sequenceNo) VALUES (?1, ?2, 0, 0, ?3)",
                    params![next_id, playlist_name.as_str(), next_seq],
                )?;
            } else {
                conn.execute(
                    "INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (?1, ?2, 0, ?3)",
                    params![next_id, playlist_name.as_str(), next_seq],
                )?;
            }
        }
    }
    move_export_playlist_row_to_front(conn, next_id, &columns)?;
    Ok(next_id)
}

pub fn replace_export_playlist_row_with_identity(
    conn: &rusqlite::Connection,
    playlist: &ExportPlaylistData,
    playlist_id: i64,
    sequence_no: i64,
) -> BackendResult<i64> {
    let columns = load_table_columns(conn, "playlist")?;
    let playlist_name = sanitized_metadata_text(&playlist.name);
    // Remove existing rows for the same playlist name (will be recreated).
    conn.execute(
        "DELETE FROM playlist_content WHERE playlist_id IN (SELECT playlist_id FROM playlist WHERE name = ?1 AND attribute = 0)",
        params![playlist_name.as_str()],
    )?;
    conn.execute(
        "DELETE FROM playlist WHERE name = ?1 AND attribute = 0",
        params![playlist_name.as_str()],
    )?;
    // If the target playlist_id collides with a different existing playlist,
    // reassign that playlist to a new free ID instead of deleting it.
    let collider_name: Option<String> = conn
        .query_row(
            "SELECT name FROM playlist WHERE playlist_id = ?1 AND attribute = 0",
            params![playlist_id],
            |row| row.get(0),
        )
        .ok();
    if let Some(ref cname) = collider_name {
        if *cname != playlist_name {
            let new_id: i64 = conn.query_row(
                "SELECT COALESCE(MAX(playlist_id), 0) + 1 FROM playlist",
                [],
                |row| row.get(0),
            )?;
            conn.execute(
                "UPDATE playlist_content SET playlist_id = ?1 WHERE playlist_id = ?2",
                params![new_id, playlist_id],
            )?;
            conn.execute(
                "UPDATE playlist SET playlist_id = ?1 WHERE playlist_id = ?2 AND attribute = 0",
                params![new_id, playlist_id],
            )?;
        }
    }

    match load_table_row_template(conn, "playlist", &columns) {
        Ok(mut values) => {
            values.insert("playlist_id".to_string(), Value::Integer(playlist_id));
            if values.contains_key("name") {
                values.insert("name".to_string(), Value::Text(playlist_name.clone()));
            }
            if values.contains_key("attribute") {
                values.insert("attribute".to_string(), Value::Integer(0));
            }
            if values.contains_key("playlist_id_parent") {
                values.insert("playlist_id_parent".to_string(), Value::Integer(0));
            }
            if values.contains_key("sequenceNo") {
                values.insert("sequenceNo".to_string(), Value::Integer(sequence_no));
            }
            if values.contains_key("image_id") {
                values.insert("image_id".to_string(), Value::Integer(0));
            }
            dynamic_insert(conn, "playlist", &columns, &values)?;
        }
        Err(_) => {
            if columns.iter().any(|c| c == "playlist_id_parent") {
                conn.execute(
                    "INSERT INTO playlist (playlist_id, name, attribute, playlist_id_parent, sequenceNo) VALUES (?1, ?2, 0, 0, ?3)",
                    params![playlist_id, playlist_name.as_str(), sequence_no],
                )?;
            } else {
                conn.execute(
                    "INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (?1, ?2, 0, ?3)",
                    params![playlist_id, playlist_name.as_str(), sequence_no],
                )?;
            }
        }
    }
    Ok(playlist_id)
}

pub fn find_content_id_by_path(
    conn: &rusqlite::Connection,
    path: &str,
) -> BackendResult<Option<i64>> {
    conn.query_row(
        "SELECT content_id FROM content WHERE path = ?1 LIMIT 1",
        params![path],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map_err(BackendError::from)
}

pub fn insert_content_from_template(
    conn: &rusqlite::Connection,
    track: &ExportManifestTrack,
    export_date_added: Option<&str>,
) -> BackendResult<i64> {
    let columns = load_table_columns(conn, "content")?;
    let next_id = next_numeric_id(conn, "content", "content_id")?;
    match load_table_row_template(conn, "content", &columns) {
        Ok(mut values) => {
            values.insert("content_id".to_string(), Value::Integer(next_id));
            populate_content_values_map(conn, &mut values, track, export_date_added)?;
            dynamic_insert(conn, "content", &columns, &values)?;
        }
        Err(_) => {
            let mut values = columns
                .iter()
                .map(|c| (c.clone(), Value::Null))
                .collect::<HashMap<_, _>>();
            values.insert("content_id".to_string(), Value::Integer(next_id));
            populate_content_values_map(conn, &mut values, track, export_date_added)?;
            dynamic_insert(conn, "content", &columns, &values)?;
        }
    }
    Ok(next_id)
}

pub fn populate_content_values_map(
    conn: &rusqlite::Connection,
    values: &mut HashMap<String, Value>,
    track: &ExportManifestTrack,
    export_date_added: Option<&str>,
) -> BackendResult<()> {
    if values.contains_key("title") {
        values.insert(
            "title".to_string(),
            Value::Text(sanitized_metadata_text(&track.title)),
        );
    }
    if values.contains_key("path") {
        values.insert("path".to_string(), Value::Text(track.exported_path.clone()));
    }
    if values.contains_key("analysisDataFilePath") {
        if let Some(path) = track.waveform_path.as_ref() {
            values.insert(
                "analysisDataFilePath".to_string(),
                Value::Text(path.clone()),
            );
        } else {
            values.insert("analysisDataFilePath".to_string(), Value::Null);
        }
    }
    if values.contains_key("bpmx100") {
        let bpmx100 = track.bpm.map(|v| (v * 100.0).round() as i64).unwrap_or(0);
        values.insert("bpmx100".to_string(), Value::Integer(bpmx100));
    }
    if values.contains_key("length") {
        let length = track.duration_ms.map(duration_ms_to_seconds).unwrap_or(0);
        values.insert("length".to_string(), Value::Integer(length));
    }
    if values.contains_key("artist_id_artist") {
        let artist_id = find_or_insert_artist(conn, &track.artist)?;
        values.insert(
            "artist_id_artist".to_string(),
            artist_id.map(Value::Integer).unwrap_or(Value::Null),
        );
    }
    if values.contains_key("album_id") {
        let album_id = find_or_insert_album(conn, track.album.as_deref(), Some(&track.artist))?;
        values.insert(
            "album_id".to_string(),
            album_id.map(Value::Integer).unwrap_or(Value::Null),
        );
    }
    if values.contains_key("image_id") || values.contains_key("imageFilePath_id") {
        let image_value = match track.artwork_path.as_deref() {
            Some(artwork_path) if !artwork_path.trim().is_empty() => {
                find_or_insert_image(conn, artwork_path)?
                    .map(Value::Integer)
                    .unwrap_or(Value::Null)
            }
            _ => Value::Null,
        };
        if values.contains_key("image_id") {
            values.insert("image_id".to_string(), image_value.clone());
        }
        if values.contains_key("imageFilePath_id") {
            values.insert("imageFilePath_id".to_string(), image_value);
        }
    }
    if values.contains_key("key_id") {
        if let Some(key_id) = find_key_id_by_name(conn, track.key.as_deref())? {
            values.insert("key_id".to_string(), Value::Integer(key_id));
        } else {
            values.insert("key_id".to_string(), Value::Null);
        }
    }
    if values.contains_key("fileName") {
        values.insert(
            "fileName".to_string(),
            Value::Text(content_file_name(&track.exported_path)),
        );
    }
    if values.contains_key("fileSize") {
        values.insert(
            "fileSize".to_string(),
            track
                .file_size_bytes
                .map(Value::Integer)
                .unwrap_or(Value::Null),
        );
    }
    if values.contains_key("fileType") {
        values.insert(
            "fileType".to_string(),
            track.file_type.map(Value::Integer).unwrap_or(Value::Null),
        );
    }
    if values.contains_key("trackNo") {
        values.insert(
            "trackNo".to_string(),
            track
                .track_number
                .map(|v| Value::Integer(v as i64))
                .unwrap_or(Value::Null),
        );
    }
    if values.contains_key("discNo") {
        values.insert(
            "discNo".to_string(),
            Value::Integer(track.disc_number.unwrap_or(0) as i64),
        );
    }
    if values.contains_key("bitrate") {
        values.insert(
            "bitrate".to_string(),
            track
                .bitrate_kbps
                .map(|v| Value::Integer(v as i64))
                .unwrap_or(Value::Null),
        );
    }
    if values.contains_key("samplingRate") {
        values.insert(
            "samplingRate".to_string(),
            track
                .sample_rate_hz
                .map(|v| Value::Integer(v as i64))
                .unwrap_or(Value::Null),
        );
    }
    if values.contains_key("bitDepth") {
        values.insert(
            "bitDepth".to_string(),
            Value::Integer(i64::from(track.bit_depth.unwrap_or(16))),
        );
    }
    if values.contains_key("subtitle") {
        values.insert(
            "subtitle".to_string(),
            Value::Text(
                sanitized_optional_metadata_text(track.subtitle.as_deref()).unwrap_or_default(),
            ),
        );
    }
    if values.contains_key("titleForSearch") {
        let title_for_search =
            sanitized_optional_nonempty_metadata_text(track.title_for_search.as_deref())
                .unwrap_or_default();
        values.insert("titleForSearch".to_string(), Value::Text(title_for_search));
    }
    if values.contains_key("isrc") {
        values.insert(
            "isrc".to_string(),
            Value::Text(track.isrc.clone().unwrap_or_default()),
        );
    }
    if values.contains_key("djComment") {
        values.insert(
            "djComment".to_string(),
            Value::Text(
                sanitized_optional_metadata_text(track.comment.as_deref()).unwrap_or_default(),
            ),
        );
    }
    if values.contains_key("releaseYear") {
        values.insert(
            "releaseYear".to_string(),
            Value::Integer(i64::from(track.release_year.unwrap_or(0))),
        );
    }
    if values.contains_key("releaseDate") {
        values.insert(
            "releaseDate".to_string(),
            resolve_track_release_date_for_export(track)
                .map(Value::Text)
                .unwrap_or_else(|| Value::Text(String::new())),
        );
    }
    if values.contains_key("dateAdded") {
        values.insert(
            "dateAdded".to_string(),
            normalize_export_date(export_date_added)
                .map(Value::Text)
                .unwrap_or(Value::Null),
        );
    }
    if values.contains_key("dateCreated") {
        values.insert(
            "dateCreated".to_string(),
            resolve_track_created_date_for_export(track)
                .map(Value::Text)
                .unwrap_or(Value::Null),
        );
    }
    if values.contains_key("artist_id_lyricist") {
        values.insert(
            "artist_id_lyricist".to_string(),
            Value::Integer(i64::from(track.artist_id_lyricist.unwrap_or(0))),
        );
    }
    if values.contains_key("artist_id_originalArtist") {
        values.insert(
            "artist_id_originalArtist".to_string(),
            Value::Integer(i64::from(track.artist_id_original_artist.unwrap_or(0))),
        );
    }
    if values.contains_key("artist_id_remixer") {
        values.insert(
            "artist_id_remixer".to_string(),
            Value::Integer(i64::from(track.artist_id_remixer.unwrap_or(0))),
        );
    }
    if values.contains_key("artist_id_composer") {
        values.insert(
            "artist_id_composer".to_string(),
            Value::Integer(i64::from(track.artist_id_composer.unwrap_or(0))),
        );
    }
    if values.contains_key("genre_id") {
        let resolved_genre_id = if let Some(ref genre_name) = track.genre {
            find_or_insert_genre(conn, genre_name)?
        } else {
            track.genre_id.map(i64::from)
        };
        values.insert(
            "genre_id".to_string(),
            resolved_genre_id.map(Value::Integer).unwrap_or(Value::Null),
        );
    }
    if values.contains_key("label_id") {
        values.insert(
            "label_id".to_string(),
            Value::Integer(i64::from(track.label_id.unwrap_or(0))),
        );
    }
    if values.contains_key("analysedBits") {
        values.insert(
            "analysedBits".to_string(),
            Value::Integer(if track.waveform_path.is_some() { 41 } else { 0 }),
        );
    }
    if values.contains_key("contentLink") {
        if let Some(cl) = track.content_link {
            values.insert("contentLink".to_string(), Value::Integer(cl));
        }
    }
    if values.contains_key("masterContentId") {
        if let Some(mci) = track.master_content_id {
            values.insert("masterContentId".to_string(), Value::Integer(mci));
        }
    }
    if values.contains_key("masterDbId") {
        if let Some(mdb) = track.master_db_id {
            values.insert("masterDbId".to_string(), Value::Integer(mdb));
        }
    }
    if values.contains_key("isHotCueAutoLoadOn") {
        values.insert("isHotCueAutoLoadOn".to_string(), Value::Integer(1));
    }
    if values.contains_key("isKuvoDeliverStatusOn") {
        values.insert("isKuvoDeliverStatusOn".to_string(), Value::Integer(0));
    }
    if values.contains_key("hasModified") {
        values.insert("hasModified".to_string(), Value::Integer(0));
    }
    if values.contains_key("cueUpdateCount") {
        values.insert("cueUpdateCount".to_string(), Value::Integer(0));
    }
    if values.contains_key("analysisDataUpdateCount") {
        values.insert("analysisDataUpdateCount".to_string(), Value::Null);
    }
    if values.contains_key("informationUpdateCount") {
        values.insert("informationUpdateCount".to_string(), Value::Null);
    }
    if values.contains_key("rating") {
        values.insert(
            "rating".to_string(),
            Value::Integer(i64::from(track.rating.unwrap_or(0))),
        );
    }
    if values.contains_key("djPlayCount") {
        values.insert(
            "djPlayCount".to_string(),
            Value::Integer(i64::from(track.dj_play_count.unwrap_or(0))),
        );
    }
    if values.contains_key("color_id") {
        values.insert(
            "color_id".to_string(),
            Value::Integer(i64::from(track.color_id.unwrap_or(0))),
        );
    }
    if values.contains_key("kuvoDeliveryComment") {
        values.insert(
            "kuvoDeliveryComment".to_string(),
            Value::Text(
                sanitized_optional_metadata_text(track.kuvo_delivery_comment.as_deref())
                    .unwrap_or_default(),
            ),
        );
    }

    Ok(())
}

pub fn duration_ms_to_seconds(duration_ms: u64) -> i64 {
    (duration_ms / 1000) as i64
}

pub fn content_file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

pub fn normalize_export_date(value: Option<&str>) -> Option<String> {
    let trimmed = value.map(str::trim).filter(|v| !v.is_empty())?;
    let out = trimmed.chars().take(10).collect::<String>();
    if out.len() == 10 { Some(out) } else { None }
}

pub fn resolve_track_created_date_for_export(track: &ExportManifestTrack) -> Option<String> {
    normalize_export_date(track.recorded_date.as_deref())
        .or_else(|| normalize_export_date(track.release_date.as_deref()))
        .or_else(|| {
            let secs = track
                .file_modified_at
                .as_deref()
                .and_then(|v| v.parse::<i64>().ok())?;
            let dt = chrono::Utc.timestamp_opt(secs, 0).single()?;
            Some(dt.format("%Y-%m-%d").to_string())
        })
}

pub fn resolve_track_release_date_for_export(track: &ExportManifestTrack) -> Option<String> {
    normalize_export_date(track.release_date.as_deref()).or_else(|| {
        let secs = track
            .file_modified_at
            .as_deref()
            .and_then(|v| v.parse::<i64>().ok())?;
        let dt = chrono::Utc.timestamp_opt(secs, 0).single()?;
        Some(dt.format("%Y-%m-%d").to_string())
    })
}

pub fn find_key_id_by_name(
    conn: &rusqlite::Connection,
    key_name: Option<&str>,
) -> BackendResult<Option<i64>> {
    if !table_exists(conn, "key") {
        return Ok(None);
    }
    let Some(key_name) = key_name.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if let Some(existing) = conn
        .query_row(
            r#"SELECT key_id FROM "key" WHERE lower(name) = lower(?1) LIMIT 1"#,
            params![key_name],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(Some(existing));
    }
    let next_id = next_numeric_id(conn, "key", "key_id")?;
    let columns = load_table_columns(conn, "key")?;
    match load_table_row_template(conn, "key", &columns) {
        Ok(mut values) => {
            values.insert("key_id".to_string(), Value::Integer(next_id));
            if values.contains_key("name") {
                values.insert("name".to_string(), Value::Text(key_name.to_string()));
            }
            dynamic_insert(conn, "key", &columns, &values)?;
        }
        Err(_) => {
            conn.execute(
                r#"INSERT INTO "key" (key_id, name) VALUES (?1, ?2)"#,
                params![next_id, key_name],
            )?;
        }
    }
    Ok(Some(next_id))
}

pub fn find_or_insert_genre(
    conn: &rusqlite::Connection,
    genre_name: &str,
) -> BackendResult<Option<i64>> {
    if !table_exists(conn, "genre") {
        return Ok(None);
    }
    let genre_name = genre_name.trim();
    let genre_name = sanitize_metadata(genre_name);
    let genre_name = genre_name.trim();
    if genre_name.is_empty() {
        return Ok(None);
    }
    if let Some(existing) = conn
        .query_row(
            "SELECT genre_id FROM genre WHERE lower(name) = lower(?1) LIMIT 1",
            params![genre_name],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(Some(existing));
    }
    let next_id = next_numeric_id(conn, "genre", "genre_id")?;
    conn.execute(
        "INSERT INTO genre (genre_id, name) VALUES (?1, ?2)",
        params![next_id, genre_name],
    )?;
    Ok(Some(next_id))
}

pub fn find_or_insert_artist(
    conn: &rusqlite::Connection,
    artist_name: &str,
) -> BackendResult<Option<i64>> {
    if !table_exists(conn, "artist") {
        return Ok(None);
    }
    let artist_name = artist_name.trim();
    let artist_name = sanitize_metadata(artist_name);
    let artist_name = artist_name.trim();
    if artist_name.is_empty() {
        return Ok(None);
    }
    if let Some(existing) = conn
        .query_row(
            "SELECT artist_id FROM artist WHERE lower(name) = lower(?1) LIMIT 1",
            params![artist_name],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(Some(existing));
    }
    let next_id = next_numeric_id(conn, "artist", "artist_id")?;
    let columns = load_table_columns(conn, "artist")?;
    match load_table_row_template(conn, "artist", &columns) {
        Ok(mut values) => {
            values.insert("artist_id".to_string(), Value::Integer(next_id));
            if values.contains_key("name") {
                values.insert("name".to_string(), Value::Text(artist_name.to_string()));
            }
            if values.contains_key("nameForSearch") {
                values.insert("nameForSearch".to_string(), Value::Text(String::new()));
            }
            dynamic_insert(conn, "artist", &columns, &values)?;
        }
        Err(_) => {
            conn.execute(
                "INSERT INTO artist (artist_id, name) VALUES (?1, ?2)",
                params![next_id, artist_name],
            )?;
        }
    }
    Ok(Some(next_id))
}

pub fn find_or_insert_album(
    conn: &rusqlite::Connection,
    album_name: Option<&str>,
    artist_name: Option<&str>,
) -> BackendResult<Option<i64>> {
    if !table_exists(conn, "album") {
        return Ok(None);
    }
    let Some(album_name) = album_name
        .map(sanitize_metadata)
        .map(|name| name.trim().to_string())
        .filter(|v| !v.is_empty())
    else {
        return Ok(None);
    };
    if let Some(existing) = conn
        .query_row(
            "SELECT album_id FROM album WHERE lower(name) = lower(?1) LIMIT 1",
            params![album_name],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        let columns = load_table_columns(conn, "album")?;
        if columns.iter().any(|c| c == "artist_id") {
            if let Some(artist_name) = artist_name {
                if let Some(artist_id) = find_or_insert_artist(conn, artist_name)? {
                    let _ = conn.execute(
                        "UPDATE album SET artist_id = COALESCE(artist_id, ?1) WHERE album_id = ?2",
                        params![artist_id, existing],
                    );
                }
            }
        }
        if columns.iter().any(|c| c == "isComplation") {
            let _ = conn.execute(
                "UPDATE album SET isComplation = COALESCE(isComplation, 0) WHERE album_id = ?1",
                params![existing],
            );
        }
        return Ok(Some(existing));
    }
    let next_id = next_numeric_id(conn, "album", "album_id")?;
    let columns = load_table_columns(conn, "album")?;
    match load_table_row_template(conn, "album", &columns) {
        Ok(mut values) => {
            values.insert("album_id".to_string(), Value::Integer(next_id));
            if values.contains_key("name") {
                values.insert("name".to_string(), Value::Text(album_name.to_string()));
            }
            if values.contains_key("artist_id") {
                let artist_id = match artist_name {
                    Some(name) => find_or_insert_artist(conn, name)?,
                    None => None,
                };
                values.insert(
                    "artist_id".to_string(),
                    artist_id.map(Value::Integer).unwrap_or(Value::Null),
                );
            }
            if values.contains_key("isComplation") {
                values.insert("isComplation".to_string(), Value::Integer(0));
            }
            if values.contains_key("nameForSearch") {
                values.insert("nameForSearch".to_string(), Value::Text(String::new()));
            }
            if values.contains_key("image_id") {
                values.insert("image_id".to_string(), Value::Integer(0));
            }
            dynamic_insert(conn, "album", &columns, &values)?;
        }
        Err(_) => {
            let artist_id = match artist_name {
                Some(name) => find_or_insert_artist(conn, name)?,
                None => None,
            };
            if let Some(artist_id) = artist_id {
                conn.execute(
                    "INSERT INTO album (album_id, name, artist_id, isComplation) VALUES (?1, ?2, ?3, 0)",
                    params![next_id, album_name, artist_id],
                )?;
            } else {
                conn.execute(
                    "INSERT INTO album (album_id, name, isComplation) VALUES (?1, ?2, 0)",
                    params![next_id, album_name],
                )?;
            }
        }
    }
    Ok(Some(next_id))
}

pub fn find_or_insert_image(conn: &rusqlite::Connection, path: &str) -> BackendResult<Option<i64>> {
    if !table_exists(conn, "image") {
        return Ok(None);
    }
    if let Some(id) = conn
        .query_row(
            "SELECT image_id FROM image WHERE path = ?1 LIMIT 1",
            params![path],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(Some(id));
    }

    let next_id = next_numeric_id(conn, "image", "image_id")?;
    let columns = load_table_columns(conn, "image")?;
    match load_table_row_template(conn, "image", &columns) {
        Ok(mut values) => {
            values.insert("image_id".to_string(), Value::Integer(next_id));
            if values.contains_key("path") {
                values.insert("path".to_string(), Value::Text(path.to_string()));
            }
            dynamic_insert(conn, "image", &columns, &values)?;
        }
        Err(_) => {
            conn.execute(
                "INSERT INTO image (image_id, path) VALUES (?1, ?2)",
                params![next_id, path],
            )?;
        }
    }
    Ok(Some(next_id))
}

pub fn link_playlist_content(
    conn: &rusqlite::Connection,
    playlist_id: i64,
    content_id: i64,
    sequence: i64,
) -> BackendResult<()> {
    let columns = load_table_columns(conn, "playlist_content")?;
    match load_table_row_template(conn, "playlist_content", &columns) {
        Ok(mut values) => {
            if values.contains_key("playlist_id") {
                values.insert("playlist_id".to_string(), Value::Integer(playlist_id));
            }
            if values.contains_key("content_id") {
                values.insert("content_id".to_string(), Value::Integer(content_id));
            }
            if values.contains_key("sequenceNo") {
                values.insert("sequenceNo".to_string(), Value::Integer(sequence));
            }
            dynamic_insert(conn, "playlist_content", &columns, &values)
        }
        Err(_) => {
            conn.execute(
                "INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (?1, ?2, ?3)",
                params![playlist_id, content_id, sequence],
            )?;
            Ok(())
        }
    }
}

pub fn dynamic_insert(
    conn: &rusqlite::Connection,
    table: &str,
    columns: &[String],
    values: &HashMap<String, Value>,
) -> BackendResult<()> {
    let placeholders = (0..columns.len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    let cols_sql = columns
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("INSERT INTO {table} ({cols_sql}) VALUES ({placeholders})");
    let ordered = map_values_for_insert(columns, values);
    let params = rusqlite::params_from_iter(ordered.iter());
    conn.execute(&sql, params)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ExportPlaylistData, open_edb, open_edb_from_usb_root, open_edb_rw,
        try_read_content_date_created_index_from_edb, try_read_playlists_with_metadata_from_edb,
        try_read_playlists_with_metadata_from_edb_db_only, try_read_track_index_from_edb,
        upsert_export_playlist_row,
    };
    use crate::service::usb_vendor_compat::{USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR};
    use tempfile::tempdir;

    fn export_db_path(root: &std::path::Path) -> std::path::PathBuf {
        let dir = root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR);
        std::fs::create_dir_all(&dir).expect("create vendor db dir");
        dir.join("exportLibrary.db")
    }

    #[test]
    fn open_edb_rw_prefers_plain_sqlite_before_trying_keys() {
        let temp = tempdir().expect("tempdir");
        let db_path = export_db_path(temp.path());
        let conn = rusqlite::Connection::open(&db_path).expect("create sqlite db");
        conn.execute(
            "CREATE TABLE playlist (playlist_id INTEGER PRIMARY KEY)",
            [],
        )
        .expect("create schema");
        drop(conn);

        let mut warnings = Vec::new();
        let conn = open_edb_rw(temp.path(), &mut warnings);

        assert!(conn.is_some(), "plain sqlite db should open");
        assert_eq!(
            warnings,
            vec!["eDB opened read-write without SQLCipher key".to_string()]
        );
    }

    #[test]
    fn open_edb_rw_reports_unreadable_when_schema_missing() {
        let temp = tempdir().expect("tempdir");
        let db_path = export_db_path(temp.path());
        let conn = rusqlite::Connection::open(&db_path).expect("create sqlite db");
        drop(conn);

        let mut warnings = Vec::new();
        let conn = open_edb_rw(temp.path(), &mut warnings);

        assert!(conn.is_none(), "schema-less db should not open");
        assert!(
            warnings.iter().all(|w| !w.contains("unsafe characters")),
            "unexpected unsafe-key warning: {warnings:?}"
        );
        assert!(warnings.is_empty(), "expected no warnings: {warnings:?}");
    }

    #[test]
    fn open_edb_reports_unreadable_db() {
        let temp = tempdir().expect("tempdir");
        let db_path = export_db_path(temp.path());
        let _conn = rusqlite::Connection::open(&db_path).expect("create sqlite db");

        let mut warnings = Vec::new();
        let conn = open_edb(&db_path, &mut warnings);
        assert!(conn.is_none(), "schema-less db should not open");
        assert!(
            warnings
                .iter()
                .any(|w| w.starts_with("eDB unreadable after trying")),
            "expected unreadable warning: {warnings:?}"
        );
    }

    #[test]
    fn upsert_export_playlist_row_moves_existing_playlist_to_front() {
        let temp = tempdir().expect("tempdir");
        let db_path = export_db_path(temp.path());
        let conn = rusqlite::Connection::open(&db_path).expect("create sqlite db");
        conn.execute_batch(
            r#"
            CREATE TABLE playlist (
              playlist_id INTEGER PRIMARY KEY,
              name TEXT,
              attribute INTEGER,
              playlist_id_parent INTEGER,
              sequenceNo INTEGER
            );
            CREATE TABLE playlist_content (playlist_id INTEGER, content_id INTEGER, sequenceNo INTEGER);
            INSERT INTO playlist (playlist_id, name, attribute, playlist_id_parent, sequenceNo) VALUES
              (1, 'Old First', 0, 0, 0),
              (2, 'Exported', 0, 0, 1),
              (3, 'Child', 0, 99, 0),
              (4, 'Folder', 1, 0, 2);
            "#,
        )
        .expect("seed playlists");

        let playlist = ExportPlaylistData {
            id: "pl-exported".to_string(),
            name: "Exported".to_string(),
            tracks: Vec::new(),
        };
        let playlist_id =
            upsert_export_playlist_row(&conn, &playlist).expect("upsert existing playlist");
        assert_eq!(playlist_id, 2);

        let rows = conn
            .prepare(
                "SELECT playlist_id, sequenceNo FROM playlist WHERE attribute = 0 ORDER BY playlist_id",
            )
            .expect("prepare query")
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            .expect("query rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect rows");
        assert_eq!(
            rows,
            vec![(1, 1), (2, 0), (3, 0)],
            "target root playlist should move first without shifting other parents"
        );
    }

    #[test]
    fn upsert_export_playlist_row_sanitizes_metadata_name() {
        let temp = tempdir().expect("tempdir");
        let db_path = export_db_path(temp.path());
        let conn = rusqlite::Connection::open(&db_path).expect("create sqlite db");
        conn.execute_batch(
            r#"
            CREATE TABLE playlist (
              playlist_id INTEGER PRIMARY KEY,
              name TEXT,
              attribute INTEGER,
              sequenceNo INTEGER
            );
            CREATE TABLE playlist_content (
              playlist_id INTEGER,
              content_id INTEGER,
              sequenceNo INTEGER
            );
            "#,
        )
        .expect("create playlist table");

        let playlist = ExportPlaylistData {
            id: "pl-bad-name".to_string(),
            name: "Bad\0Name".to_string(),
            tracks: Vec::new(),
        };
        let playlist_id = upsert_export_playlist_row(&conn, &playlist).expect("upsert playlist");
        assert_eq!(playlist_id, 1);

        let name = conn
            .query_row(
                "SELECT name FROM playlist WHERE playlist_id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("load playlist name");
        assert_eq!(name, "BadName");
    }

    #[test]
    fn try_read_track_index_from_edb_reads_basic_track_data() {
        let temp = tempdir().expect("tempdir");
        let db_path = export_db_path(temp.path());
        let conn = rusqlite::Connection::open(&db_path).expect("create sqlite db");
        conn.execute_batch(
            r#"
            CREATE TABLE artist (artist_id INTEGER PRIMARY KEY, name TEXT);
            CREATE TABLE album (album_id INTEGER PRIMARY KEY, name TEXT);
            CREATE TABLE "key" (key_id INTEGER PRIMARY KEY, name TEXT);
            CREATE TABLE image (image_id INTEGER PRIMARY KEY, path TEXT);
            CREATE TABLE content (
              content_id INTEGER PRIMARY KEY,
              title TEXT,
              artist_id_artist INTEGER,
              album_id INTEGER,
              key_id INTEGER,
              image_id INTEGER,
              path TEXT,
              analysisDataFilePath TEXT,
              bpmx100 INTEGER,
              length INTEGER
            );
            INSERT INTO artist (artist_id, name) VALUES (1, 'Artist');
            INSERT INTO album (album_id, name) VALUES (1, 'Album');
            INSERT INTO "key" (key_id, name) VALUES (1, '8A');
            INSERT INTO image (image_id, path) VALUES (1, '/Contents/art.jpg');
            INSERT INTO content (
              content_id, title, artist_id_artist, album_id, key_id, image_id,
              path, analysisDataFilePath, bpmx100, length
            ) VALUES (
              10, 'Track', 1, 1, 1, 1,
              '/Contents/track.mp3', '/PIONEER/USBANLZ/P001/T/ANLZ0000.DAT', 12800, 180
            );
            "#,
        )
        .expect("seed eDB");

        let mut warnings = Vec::new();
        let index = try_read_track_index_from_edb(temp.path(), &mut warnings).expect("track index");
        let track = index.get(&10).expect("content id 10");
        assert_eq!(track.title, "Track");
        assert_eq!(track.artist, "Artist");
        assert_eq!(track.album.as_deref(), Some("Album"));
        assert_eq!(track.key.as_deref(), Some("8A"));
        assert_eq!(track.bpm, Some(128.0));
        assert_eq!(track.duration_ms, Some(180_000));
    }

    #[test]
    fn try_read_playlists_and_date_created_from_edb_reads_expected_values() {
        let temp = tempdir().expect("tempdir");
        let db_path = export_db_path(temp.path());
        let conn = rusqlite::Connection::open(&db_path).expect("create sqlite db");
        conn.execute_batch(
            r#"
            CREATE TABLE artist (artist_id INTEGER PRIMARY KEY, name TEXT);
            CREATE TABLE album (album_id INTEGER PRIMARY KEY, name TEXT);
            CREATE TABLE "key" (key_id INTEGER PRIMARY KEY, name TEXT);
            CREATE TABLE image (image_id INTEGER PRIMARY KEY, path TEXT);
            CREATE TABLE playlist (
              playlist_id INTEGER PRIMARY KEY,
              name TEXT,
              attribute INTEGER,
              sequenceNo INTEGER
            );
            CREATE TABLE content (
              content_id INTEGER PRIMARY KEY,
              title TEXT,
              artist_id_artist INTEGER,
              album_id INTEGER,
              key_id INTEGER,
              image_id INTEGER,
              path TEXT,
              analysisDataFilePath TEXT,
              bpmx100 INTEGER,
              length INTEGER,
              trackNo INTEGER,
              dateCreated TEXT
            );
            CREATE TABLE playlist_content (playlist_id INTEGER, content_id INTEGER, sequenceNo INTEGER);
            INSERT INTO artist (artist_id, name) VALUES (1, 'Artist');
            INSERT INTO album (album_id, name) VALUES (1, 'Album');
            INSERT INTO "key" (key_id, name) VALUES (1, '8A');
            INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (2, 'Set A', 0, 1);
            INSERT INTO content (
              content_id, title, artist_id_artist, album_id, key_id,
              path, analysisDataFilePath, bpmx100, length, trackNo, dateCreated
            ) VALUES (
              42, 'Track A', 1, 1, 1,
              '/Contents/a.mp3', '/PIONEER/USBANLZ/P001/A/ANLZ0000.DAT', 12500, 200, 3, '2024-02-10'
            );
            INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (2, 42, 1);
            "#,
        )
        .expect("seed eDB");

        let mut warnings = Vec::new();
        let playlists = try_read_playlists_with_metadata_from_edb(temp.path(), &mut warnings)
            .expect("playlists");
        let set_a = playlists.get("Set A").expect("Set A playlist");
        assert_eq!(set_a.playlist_id, 2);
        assert_eq!(set_a.tracks.len(), 1);
        assert_eq!(set_a.tracks[0].track_number, Some(3));
        assert!(
            std::path::Path::new(&set_a.tracks[0].file_path).starts_with(temp.path()),
            "default reader should resolve USB-relative media paths: {:?}",
            set_a.tracks[0]
        );

        let mut db_only_warnings = Vec::new();
        let db_only_playlists =
            try_read_playlists_with_metadata_from_edb_db_only(temp.path(), &mut db_only_warnings)
                .expect("DB-only playlists");
        let db_only_set_a = db_only_playlists.get("Set A").expect("Set A playlist");
        assert_eq!(db_only_set_a.tracks.len(), 1);
        let db_only_track = &db_only_set_a.tracks[0];
        assert_eq!(db_only_track.file_path, "/Contents/a.mp3");
        assert_eq!(
            db_only_track.usb_analysis_path.as_deref(),
            Some("/PIONEER/USBANLZ/P001/A/ANLZ0000.DAT")
        );
        assert_eq!(
            db_only_track.waveform_peaks_path.as_deref(),
            Some("/PIONEER/USBANLZ/P001/A/ANLZ0000.DAT")
        );

        let mut warnings2 = Vec::new();
        let dates = try_read_content_date_created_index_from_edb(temp.path(), &mut warnings2)
            .expect("dateCreated index");
        assert_eq!(dates.get(&42).map(String::as_str), Some("2024-02-10"));

        let mut warnings3 = Vec::new();
        let conn = open_edb_from_usb_root(temp.path(), &mut warnings3);
        assert!(conn.is_some(), "open_edb_from_usb_root should resolve path");
    }
}
