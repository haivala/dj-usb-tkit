//! Export database (eDB/SQLite) helpers and CRUD operations.

use crate::utils::counted;
use std::collections::HashMap;
use std::path::Path;

use chrono::TimeZone;
use rusqlite::{OptionalExtension, params, types::Value};
use serde::Serialize;

use crate::error::{BackendError, BackendResult};
use crate::i18n::tr;
use crate::logging::{self, Level};
use crate::metadata::sanitize_metadata;
use crate::models::{UsbTrack, WarningEntry};
use crate::service::usb_utils::resolve_usb_side_path_lexical_text;
use crate::service::usb_vendor_compat::{
    DEFAULT_USB_EDB_KEY, USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR,
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

#[derive(Debug, Clone, Default)]
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
    /// Unix seconds; the earlier of the source file's creation and
    /// modification time (see `ScannedTrack::file_timestamp`).
    pub file_timestamp_at: Option<i64>,
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
    /// The analysis bundle came from rekordbox and its tempo and first beat
    /// haven't been changed here since, so export keeps rekordbox's own
    /// (possibly variable-tempo) beat grid instead of rebuilding it.
    pub beatgrid_from_rekordbox: bool,
    pub cues: Vec<crate::models::TrackCue>,
    pub position: usize,
    /// The file exported is one of the track's transcodes, in this format
    /// (`"mp3"`): it is a USB track of its own, with its own
    /// masterContentId (see `export_identity_key`).
    pub transcode_format: Option<&'static str>,
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
    /// Unix seconds; the earlier of the source file's creation and
    /// modification time (see `ScannedTrack::file_timestamp`).
    pub file_timestamp_at: Option<i64>,
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
    pub first_beat_ms: Option<u32>,
    pub cues: Vec<crate::models::TrackCue>,
    /// Date the track was added to the USB (`content.dateAdded`, the PDB's
    /// `date_added`). `None`: a new row gets the export date and an existing
    /// one keeps its own.
    pub date_added: Option<String>,
    /// `content.dateCreated`. `None` derives it from the recorded date,
    /// release date or file timestamp, as before.
    pub date_created: Option<String>,
    /// Label, remixer and composer names, written to both databases'
    /// dictionaries. `None` leaves what a row has.
    pub label: Option<String>,
    pub remixer: Option<String>,
    pub composer: Option<String>,
    /// Built from the app's library by an export. `false` for a track the
    /// repair builds from the USB's own databases: the writers then write
    /// only its data and keep the USB's cues, update counters and flags, and
    /// derive no dates.
    pub from_library: bool,
    /// The PDB track row and eDB content row this track already has on the
    /// USB, when the caller knows them (the repair does). The writers update
    /// those rows instead of matching by path, so a path that differs
    /// between the two databases is corrected rather than duplicated.
    pub pdb_track_id: Option<u32>,
    pub edb_content_id: Option<i64>,
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

pub fn edb_path_from_usb_root(usb_root: &Path) -> std::path::PathBuf {
    usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("exportLibrary.db")
}

/// Whether `conn` sees a schema: at least one table or view of its own. A
/// wrong key reads the file as garbage and sees none.
fn has_schema(conn: &rusqlite::Connection) -> bool {
    conn.query_row(
        "SELECT COUNT(1) FROM sqlite_master
         WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .ok()
    .unwrap_or(0)
        > 0
}

/// SQLCipher 4's defaults, which the eDB uses: PBKDF2-HMAC-SHA512 over
/// 256 000 iterations, a 32-byte key and the file's first 16 bytes as salt.
const SQLCIPHER_KDF_ITERATIONS: i32 = 256_000;
const SQLCIPHER_SALT_LEN: usize = 16;
const SQLITE_PLAIN_HEADER: &[u8; SQLCIPHER_SALT_LEN] = b"SQLite format 3\0";

/// eDB keys derived from `DEFAULT_USB_EDB_KEY`, by the file's salt. Deriving
/// one is what makes opening the eDB slow (tens of ms), and commands that run
/// together each open it; a rewritten eDB keeps its salt, so a key derived
/// once opens it from then on. Same salt, same key: a memo, not state.
static EDB_KEY_BY_SALT: std::sync::OnceLock<
    std::sync::Mutex<HashMap<[u8; SQLCIPHER_SALT_LEN], [u8; 32]>>,
> = std::sync::OnceLock::new();

fn edb_salt(path: &Path) -> Option<[u8; SQLCIPHER_SALT_LEN]> {
    use std::io::Read;
    let mut salt = [0u8; SQLCIPHER_SALT_LEN];
    std::fs::File::open(path).ok()?.read_exact(&mut salt).ok()?;
    (&salt != SQLITE_PLAIN_HEADER).then_some(salt)
}

fn derive_edb_key(salt: &[u8; SQLCIPHER_SALT_LEN]) -> Option<[u8; 32]> {
    let pass = DEFAULT_USB_EDB_KEY.as_bytes();
    let mut key = [0u8; 32];
    // SAFETY: every pointer is to a live buffer of the length passed with it,
    // and `EVP_sha512` returns a static digest description.
    let ok = unsafe {
        openssl_sys::PKCS5_PBKDF2_HMAC(
            pass.as_ptr().cast(),
            i32::try_from(pass.len()).ok()?,
            salt.as_ptr(),
            SQLCIPHER_SALT_LEN as i32,
            SQLCIPHER_KDF_ITERATIONS,
            openssl_sys::EVP_sha512(),
            key.len() as i32,
            key.as_mut_ptr(),
        )
    };
    (ok == 1).then_some(key)
}

fn edb_key_for_salt(salt: &[u8; SQLCIPHER_SALT_LEN]) -> Option<[u8; 32]> {
    let memo = EDB_KEY_BY_SALT.get_or_init(Default::default);
    if let Some(key) = memo.lock().ok().and_then(|m| m.get(salt).copied()) {
        return Some(key);
    }
    derive_edb_key(salt)
}

/// Open the eDB at `path` with `flags`: as plain SQLite, else with its key.
/// The key comes derived from the memo (`EDB_KEY_BY_SALT`); an eDB it
/// doesn't open (other SQLCipher settings) is opened with the passphrase.
fn open_edb_with_flags(path: &Path, flags: rusqlite::OpenFlags) -> Option<rusqlite::Connection> {
    let open = || rusqlite::Connection::open_with_flags(path, flags).ok();
    let read_write = flags.contains(rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE);
    let unlocked = |how: &str| {
        logging::debug(
            "edb",
            if read_write {
                format!("eDB opened read-write with SQLCipher key ({how})")
            } else {
                format!("eDB unlocked via SQLCipher with {how}")
            },
        );
    };
    if let Some(plain) = open()
        && has_schema(&plain)
    {
        logging::debug(
            "edb",
            if read_write {
                "eDB opened read-write without SQLCipher key"
            } else {
                "eDB opened without SQLCipher key"
            },
        );
        return Some(plain);
    }

    let salt = edb_salt(path)?;
    if let Some(key) = edb_key_for_salt(&salt) {
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        if let Some(conn) = open()
            && conn
                .execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))
                .is_ok()
            && has_schema(&conn)
        {
            let memo = EDB_KEY_BY_SALT.get_or_init(Default::default);
            if let Ok(mut memo) = memo.lock() {
                memo.insert(salt, key);
            }
            unlocked("its derived key");
            return Some(conn);
        }
    }

    let conn = open()?;
    conn.execute_batch(&format!("PRAGMA key='{DEFAULT_USB_EDB_KEY}';"))
        .ok()?;
    if has_schema(&conn) {
        unlocked("the USB export key");
        return Some(conn);
    }
    None
}

pub fn open_edb(path: &Path, warnings: &mut Vec<WarningEntry>) -> Option<rusqlite::Connection> {
    if !path.exists() {
        return None;
    }
    let conn = open_edb_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY);
    if conn.is_none() {
        warnings.push(logging::log_text(
            Level::Warn,
            "edb",
            "edb.open.failed",
            &tr::usb_warning_edb_unreadable(),
        ));
    }
    conn
}

/// A read-only connection to `usb_root`'s staged eDB (see
/// `usb_staging::UsbDatabases`); `None` without one, or when it can't be
/// opened (that adds a warning).
pub fn open_edb_from_usb_root(
    usb_root: &Path,
    warnings: &mut Vec<WarningEntry>,
) -> Option<rusqlite::Connection> {
    use crate::service::usb_staging::{UsbDatabases, UsbDatabasesExt};
    let databases = match UsbDatabases::present(usb_root) {
        Ok(databases) => databases,
        Err(err) => {
            logging::debug(
                "edb",
                format!(
                    "failed to stage eDB locally, falling back to USB path: {err}",
                    err = err.english()
                ),
            );
            return open_edb(&edb_path_from_usb_root(usb_root), warnings);
        }
    };
    open_edb(databases.edb()?, warnings)
}

pub use crate::service::usb_staging::open_edb_rw;

/// A read-write connection to the eDB at `path`; `None` when it isn't there
/// or can't be opened.
pub(crate) fn open_edb_rw_at(path: &Path) -> Option<rusqlite::Connection> {
    if !path.exists() {
        return None;
    }
    open_edb_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
}

/// The eDB in the parsed PDB's shape, for the readers that work on a parsed
/// PDB when the stick has only the eDB: one track row per `content` row
/// (its `content_id` as the track id), the keys and artwork, the playlists
/// with their entries, and the histories as t11/t12 rows.
pub(crate) fn edb_as_parsed_pdb(
    conn: &rusqlite::Connection,
) -> BackendResult<crate::pdb_reader::ParsedPdb> {
    use crate::pdb_reader::{
        ParsedPdb, PdbHistoryEntryRow, PdbHistoryPlaylistRow, PdbPlaylistEntryRow,
        PdbPlaylistTreeRow, PdbTrackRow,
    };
    let id = |value: Option<i64>| value.and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    let mut parsed = ParsedPdb::default();
    if table_exists(conn, "content") {
        let image_fk = if content_column_or_null(conn, "imageFilePath_id") == "NULL" {
            content_column_or_null(conn, "image_id")
        } else {
            "c.imageFilePath_id".to_string()
        };
        let sql = format!(
            "SELECT c.content_id, c.path, {anlz}, {bpm}, {key}, {mdb}, {len}, {size}, {title}, {image_fk} \
             FROM content c ORDER BY c.content_id",
            anlz = content_column_or_null(conn, "analysisDataFilePath"),
            bpm = content_column_or_null(conn, "bpmx100"),
            key = content_column_or_null(conn, "key_id"),
            mdb = content_column_or_null(conn, "masterDbId"),
            len = content_column_or_null(conn, "length"),
            size = content_column_or_null(conn, "fileSize"),
            title = content_column_or_null(conn, "title"),
        );
        let mut stmt = conn.prepare(&sql)?;
        parsed.tracks = stmt
            .query_map([], |row| {
                Ok(PdbTrackRow {
                    id: id(row.get(0)?),
                    track_file_path: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    anlz_path: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    tempo_x100: id(row.get(3)?),
                    key_id: id(row.get(4)?),
                    master_db_id: row
                        .get::<_, Option<i64>>(5)?
                        .and_then(|v| u32::try_from(v).ok()),
                    duration_seconds: row
                        .get::<_, Option<i64>>(6)?
                        .and_then(|v| u32::try_from(v).ok()),
                    file_size_bytes: row
                        .get::<_, Option<i64>>(7)?
                        .and_then(|v| u32::try_from(v).ok()),
                    title: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
                    artwork_id: id(row.get(9)?),
                    ..PdbTrackRow::default()
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    let id_names = |sql: &str| -> BackendResult<HashMap<u32, String>> {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| {
            Ok((
                id(row.get(0)?),
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    };
    if table_exists(conn, "key") {
        parsed.keys = id_names("SELECT key_id, name FROM \"key\"")?;
    }
    if table_exists(conn, "image") {
        parsed.artworks = id_names("SELECT image_id, path FROM image")?;
    }
    if table_exists(conn, "playlist") {
        let mut stmt = conn.prepare(
            "SELECT playlist_id, COALESCE(playlist_id_parent, 0), COALESCE(sequenceNo, 0), \
             COALESCE(attribute, 0), COALESCE(name, '') FROM playlist",
        )?;
        parsed.playlist_tree = stmt
            .query_map([], |row| {
                Ok(PdbPlaylistTreeRow {
                    id: id(row.get(0)?),
                    parent_id: id(row.get(1)?),
                    sort_order: id(row.get(2)?),
                    row_is_folder: row.get::<_, i64>(3)? == 1,
                    name: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    if table_exists(conn, "playlist_content") {
        let mut stmt = conn.prepare(
            "SELECT playlist_id, content_id, COALESCE(sequenceNo, 0) FROM playlist_content",
        )?;
        parsed.playlist_entries = stmt
            .query_map([], |row| {
                Ok(PdbPlaylistEntryRow {
                    playlist_id: id(row.get(0)?),
                    track_id: id(row.get(1)?),
                    entry_index: id(row.get(2)?),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    if table_exists(conn, "history") {
        let mut stmt = conn.prepare(
            "SELECT history_id, COALESCE(name, '') FROM history \
             ORDER BY COALESCE(sequenceNo, 0), history_id",
        )?;
        parsed.history_playlists = stmt
            .query_map([], |row| {
                Ok(PdbHistoryPlaylistRow {
                    id: id(row.get(0)?),
                    name: row.get(1)?,
                    source_table: 11,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    if table_exists(conn, "history_content") {
        let mut stmt = conn.prepare(
            "SELECT history_id, content_id, COALESCE(sequenceNo, 0) FROM history_content",
        )?;
        parsed.history_entries = stmt
            .query_map([], |row| {
                Ok(PdbHistoryEntryRow {
                    track_id: row
                        .get::<_, Option<i64>>(1)?
                        .and_then(|v| u32::try_from(v).ok()),
                    playlist_id: id(row.get(0)?),
                    entry_index: id(row.get(2)?),
                    source_table: 12,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(parsed)
}

/// `c.<column>` when the eDB's `content` table has `column`, else `NULL`, so
/// a read works across eDB schema versions.
pub(crate) fn content_column_or_null(conn: &rusqlite::Connection, column: &str) -> String {
    let found = conn
        .query_row(
            "SELECT COUNT(1) FROM pragma_table_info('content') WHERE name = ?1",
            [column],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if found {
        format!("c.{column}")
    } else {
        "NULL".to_string()
    }
}

/// A `releaseYear` value as a track's year: none for 0 (no year).
fn edb_release_year(value: Option<i64>) -> Option<u32> {
    value
        .and_then(|year| u32::try_from(year).ok())
        .filter(|year| *year > 0)
}

// -----------------------------------------------------------------------------
// eDB read
// -----------------------------------------------------------------------------

/// What [`read_edb`] reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdbRead {
    /// Every track (`EdbLibrary::tracks`).
    pub tracks: bool,
    /// The playlists with their tracks (`EdbLibrary::playlists`).
    pub playlists: bool,
    /// Each track's `dateCreated` (`EdbLibrary::date_created`).
    pub date_created: bool,
    /// Track file, artwork and analysis paths resolved against the USB root
    /// (which looks each one up on disk), instead of as the eDB stores them.
    pub resolve_paths: bool,
}

impl EdbRead {
    /// Everything, paths as stored: what the service caches for the staged eDB.
    pub const ALL: Self = Self {
        tracks: true,
        playlists: true,
        date_created: true,
        resolve_paths: false,
    };
}

/// An eDB's tracks and playlists, as [`read_edb`] read them; what it wasn't
/// asked for is empty.
#[derive(Debug, Clone, Default)]
pub struct EdbLibrary {
    /// Every track, by `content_id`.
    pub tracks: HashMap<u32, UsbTrack>,
    /// The playlists by name, each with its tracks in order. Playlists that
    /// share a name are one, under the first one's id, without repeats.
    pub playlists: HashMap<String, ExportDbPlaylist>,
    /// `content_id` -> `dateCreated`, for the tracks that have one.
    pub date_created: HashMap<u32, String>,
}

impl EdbLibrary {
    /// The same library with its paths resolved against `usb_root`, as a
    /// `resolve_paths` read gives them.
    pub fn resolved(&self, usb_root: &Path) -> Self {
        Self {
            tracks: self.resolved_tracks(usb_root),
            playlists: self.resolved_playlists(usb_root),
            date_created: self.date_created.clone(),
        }
    }

    /// The tracks with their paths resolved against `usb_root`.
    pub fn resolved_tracks(&self, usb_root: &Path) -> HashMap<u32, UsbTrack> {
        self.tracks
            .iter()
            .map(|(id, track)| (*id, resolve_track_paths(track.clone(), usb_root)))
            .collect()
    }

    /// The playlists with their tracks' paths resolved against `usb_root`.
    pub fn resolved_playlists(&self, usb_root: &Path) -> HashMap<String, ExportDbPlaylist> {
        self.playlists
            .iter()
            .map(|(name, playlist)| {
                (
                    name.clone(),
                    ExportDbPlaylist {
                        playlist_id: playlist.playlist_id,
                        sort_order: playlist.sort_order,
                        tracks: playlist
                            .tracks
                            .iter()
                            .map(|track| resolve_track_paths(track.clone(), usb_root))
                            .collect(),
                    },
                )
            })
            .collect()
    }

    /// The playlists, or `None` when there are none (or they weren't read).
    pub fn playlists_or_none(&self) -> Option<&HashMap<String, ExportDbPlaylist>> {
        (!self.playlists.is_empty()).then_some(&self.playlists)
    }
}

/// `track` (read with its paths as stored) with them resolved against `usb_root`.
fn resolve_track_paths(mut track: UsbTrack, usb_root: &Path) -> UsbTrack {
    let resolve =
        |path: Option<&str>| path.and_then(|p| resolve_usb_side_path_lexical_text(usb_root, p));
    track.file_path = resolve(track.usb_media_path.as_deref()).unwrap_or_default();
    track.artwork_path = resolve(track.artwork_path.as_deref());
    let analysis = resolve(track.usb_analysis_path_raw.as_deref());
    track.waveform_peaks_path = analysis.clone();
    track.usb_analysis_path = analysis;
    track
}

/// Reads what `what` asks for from an open eDB: the one reader of its tracks
/// and playlists. `None` when the eDB has no readable content or playlist
/// table.
pub fn read_edb(conn: &rusqlite::Connection, usb_root: &Path, what: EdbRead) -> Option<EdbLibrary> {
    let mut library = EdbLibrary::default();
    if !(what.tracks || what.playlists || what.date_created) {
        return Some(library);
    }
    for (content_id, track, date_created) in read_edb_content(conn)? {
        if what.date_created
            && let Some(date) = date_created
        {
            library.date_created.insert(content_id, date);
        }
        if what.tracks || what.playlists {
            library.tracks.insert(content_id, track);
        }
    }
    if what.playlists {
        library.playlists = read_edb_playlists(conn, &library.tracks)?;
        logging::debug(
            "edb",
            format!(
                "loaded {} from eDB",
                counted(library.playlists.len(), "playlist", "playlists")
            ),
        );
    }
    if !what.tracks {
        library.tracks.clear();
    }
    // Resolving looks each path up on disk: only for the tracks returned.
    if what.resolve_paths {
        library = library.resolved(usb_root);
    }
    Some(library)
}

/// Every `content` row as a track (paths as stored), with its `dateCreated`.
fn read_edb_content(conn: &rusqlite::Connection) -> Option<Vec<(u32, UsbTrack, Option<String>)>> {
    let length_expr = content_column_or_null(conn, "length");
    let track_no_expr = content_column_or_null(conn, "trackNo");
    let release_year_expr = content_column_or_null(conn, "releaseYear");
    let file_size_expr = content_column_or_null(conn, "fileSize");
    let date_created_expr = content_column_or_null(conn, "dateCreated");
    let image_fk_col = if content_column_or_null(conn, "imageFilePath_id") == "NULL" {
        "c.image_id"
    } else {
        "c.imageFilePath_id"
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
          {length_expr} AS length_seconds,
          {track_no_expr} AS track_no,
          {release_year_expr} AS release_year,
          CAST({file_size_expr} AS INTEGER) AS file_size,
          {date_created_expr} AS date_created
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
            logging::debug("edb", format!("eDB content query failed: {err}"));
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
        let track_no: Option<i64> = row.get(10)?;
        let release_year: Option<i64> = row.get(11)?;
        let file_size: Option<i64> = row.get(12)?;
        let date_created: Option<String> = row.get(13)?;
        let format_ext = path
            .as_deref()
            .and_then(crate::audio_format::format_ext_from_path);
        let track = UsbTrack {
            id: content_id.to_string(),
            local_track_id: None,
            title: title.unwrap_or_else(|| "Unknown Title".to_string()),
            artist: artist.unwrap_or_else(|| "Unknown Artist".to_string()),
            album,
            year: edb_release_year(release_year),
            genre: None,
            track_number: track_no.and_then(|value| u32::try_from(value).ok()),
            bpm: bpmx100.map(|v| v as f64 / 100.0),
            key: key_name,
            key_display: None,
            key_color: None,
            file_path: path.clone().unwrap_or_default(),
            needs_hydration: false,
            format_compat: crate::service::format_compat::compute_format_compat(
                format_ext.as_deref(),
                None,
                None,
                None,
                None,
            ),
            format_ext,
            usb_media_path: path,
            artwork_data_url: None,
            artwork_path: image_path,
            waveform_peaks_path: analysis_path.clone(),
            usb_analysis_path: analysis_path.clone(),
            usb_analysis_path_raw: analysis_path,
            waveform_preview: None,
            duration_ms: length_seconds
                .filter(|v| *v > 0)
                .map(|v| (v as u64).saturating_mul(1000)),
            file_size_bytes: file_size.filter(|size| *size > 0),
        };
        let date_created = date_created
            .map(|date| date.trim().to_string())
            .filter(|date| !date.is_empty());
        Ok((content_id, track, date_created))
    }) {
        Ok(r) => r,
        Err(err) => {
            logging::debug("edb", format!("eDB content row mapping failed: {err}"));
            return None;
        }
    };
    let tracks = rows
        .flatten()
        .filter_map(|(content_id, track, date_created)| {
            u32::try_from(content_id)
                .ok()
                .filter(|id| *id > 0)
                .map(|id| (id, track, date_created))
        })
        .collect::<Vec<_>>();
    logging::debug(
        "edb",
        format!("loaded {} track rows from eDB content", tracks.len()),
    );
    Some(tracks)
}

/// The playlists (not folders), each with its tracks from `tracks` in order.
fn read_edb_playlists(
    conn: &rusqlite::Connection,
    tracks: &HashMap<u32, UsbTrack>,
) -> Option<HashMap<String, ExportDbPlaylist>> {
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
            logging::debug("edb", format!("eDB schema query failed: {err}"));
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
        Ok(rows) => rows.flatten().collect::<Vec<_>>(),
        Err(err) => {
            logging::debug("edb", format!("eDB playlist query failed: {err}"));
            return None;
        }
    };

    // Every playlist's content ids in order, in one query.
    let mut content_by_playlist = HashMap::<i64, Vec<i64>>::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT playlist_id, content_id FROM playlist_content ORDER BY playlist_id, sequenceNo ASC",
    ) && let Ok(rows) =
        stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
    {
        for (playlist_id, content_id) in rows.flatten() {
            content_by_playlist
                .entry(playlist_id)
                .or_default()
                .push(content_id);
        }
    }

    let mut out = HashMap::<String, ExportDbPlaylist>::new();
    for (playlist_id, playlist_name, sort_order) in playlist_rows {
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
        let content_ids = content_by_playlist
            .get(&playlist_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for content_id in content_ids {
            let Some(track) = u32::try_from(*content_id)
                .ok()
                .and_then(|id| tracks.get(&id))
            else {
                continue;
            };
            if seen.insert(track.id.clone()) {
                existing.tracks.push(track.clone());
            }
        }
    }
    Some(out)
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

/// One eDB track's star rating and colour, as the PDB track row carries them
/// (bytes 89 and 88): rating 0–5, colour 0 (none) or a `color` row id 1–8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdbRatingColour {
    pub rating: u8,
    pub color_id: u8,
}

/// `content.path` → rating and colour for every eDB track with a path. A
/// schema without either column reads as 0 for it.
pub fn read_rating_colour_by_path(conn: &rusqlite::Connection) -> HashMap<String, EdbRatingColour> {
    if !table_exists(conn, "content") {
        return HashMap::new();
    }
    let columns = load_table_columns(conn, "content").unwrap_or_default();
    let column_or_zero = |name: &str| {
        if columns.iter().any(|c| c == name) {
            format!("COALESCE({name}, 0)")
        } else {
            "0".to_string()
        }
    };
    let sql = format!(
        "SELECT path, {}, {} FROM content WHERE path IS NOT NULL AND trim(path) <> ''",
        column_or_zero("rating"),
        column_or_zero("color_id"),
    );
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return HashMap::new();
    };
    let byte = |v: i64| u8::try_from(v).unwrap_or(0);
    stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            EdbRatingColour {
                rating: byte(row.get::<_, i64>(1)?),
                color_id: byte(row.get::<_, i64>(2)?),
            },
        ))
    })
    .map(|rows| rows.flatten().collect())
    .unwrap_or_default()
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

    let has_parent_col = columns.iter().any(|c| c == "playlist_id_parent");

    // Renumber siblings to a fresh contiguous sequence (target = 0, the rest
    // keep their existing relative order at 1..N) instead of incrementing
    // every other sibling's sequenceNo unconditionally. An unconditional +1
    // diverges from the PDB side (`move_playlist_tree_row_to_front`, which
    // always recomputes a tight contiguous sort_order) after the very first
    // run whenever a sibling already sorts after the target, and the two
    // surfaces drift further apart on every additional additive export.
    let mut sibling_ids: Vec<i64> = if has_parent_col {
        let parent_id = conn
            .query_row(
                "SELECT COALESCE(playlist_id_parent, 0) FROM playlist WHERE playlist_id = ?1",
                params![playlist_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        let mut stmt = conn.prepare(
            "SELECT playlist_id FROM playlist
             WHERE attribute = 0
               AND COALESCE(playlist_id_parent, 0) = ?1
               AND playlist_id <> ?2
             ORDER BY COALESCE(sequenceNo, 0) ASC, playlist_id ASC",
        )?;
        let rows = stmt.query_map(params![parent_id, playlist_id], |row| row.get::<_, i64>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    } else {
        let mut stmt = conn.prepare(
            "SELECT playlist_id FROM playlist
             WHERE attribute = 0 AND playlist_id <> ?1
             ORDER BY COALESCE(sequenceNo, 0) ASC, playlist_id ASC",
        )?;
        let rows = stmt.query_map(params![playlist_id], |row| row.get::<_, i64>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    conn.execute(
        "UPDATE playlist SET sequenceNo = 0 WHERE playlist_id = ?1 AND attribute = 0",
        params![playlist_id],
    )?;
    for (idx, sibling_id) in sibling_ids.drain(..).enumerate() {
        conn.execute(
            "UPDATE playlist SET sequenceNo = ?1 WHERE playlist_id = ?2 AND attribute = 0",
            params![(idx as i64) + 1, sibling_id],
        )?;
    }
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
    // If the target playlist_id collides with a different existing row,
    // reassign that row to a new free ID instead of deleting it. The
    // `playlist_id` UNIQUE constraint applies across all rows regardless of
    // `attribute` (folders share the same id space as playlists), so the
    // collision check and the reassignment must not be scoped to
    // `attribute = 0` — doing so let a same-id folder row go undetected (and
    // even when detected, left unmoved), causing the INSERT below to fail
    // with "UNIQUE constraint failed: playlist.playlist_id".
    let collider_name: Option<String> = conn
        .query_row(
            "SELECT name FROM playlist WHERE playlist_id = ?1",
            params![playlist_id],
            |row| row.get(0),
        )
        .ok();
    if let Some(ref cname) = collider_name
        && *cname != playlist_name
    {
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
            "UPDATE playlist SET playlist_id = ?1 WHERE playlist_id = ?2",
            params![new_id, playlist_id],
        )?;
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
    // A track from the library gets the export's derived dates; one the
    // repair carries over from the USB keeps exactly the dates it has.
    let (release_year, release_date, date_created) = if track.from_library {
        (
            resolve_track_release_year_for_export(track).map(i64::from),
            resolve_track_release_date_for_export(track),
            track
                .date_created
                .clone()
                .or_else(|| resolve_track_created_date_for_export(track)),
        )
    } else {
        (
            track.release_year.map(i64::from),
            track.release_date.clone(),
            track.date_created.clone(),
        )
    };
    if values.contains_key("releaseYear") {
        values.insert(
            "releaseYear".to_string(),
            Value::Integer(release_year.unwrap_or(0)),
        );
    }
    if values.contains_key("releaseDate") {
        values.insert(
            "releaseDate".to_string(),
            Value::Text(release_date.unwrap_or_default()),
        );
    }
    if values.contains_key("dateAdded") {
        values.insert(
            "dateAdded".to_string(),
            normalize_export_date(track.date_added.as_deref())
                .or_else(|| normalize_export_date(export_date_added))
                .map(Value::Text)
                .unwrap_or(Value::Null),
        );
    }
    if values.contains_key("dateCreated") {
        values.insert(
            "dateCreated".to_string(),
            date_created.map(Value::Text).unwrap_or(Value::Null),
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
    let named_artist = |name: Option<&str>| -> BackendResult<Option<i64>> {
        match name.map(str::trim).filter(|v| !v.is_empty()) {
            Some(name) => find_or_insert_artist(conn, name),
            None => Ok(None),
        }
    };
    if values.contains_key("artist_id_remixer") {
        let id = named_artist(track.remixer.as_deref())?
            .or(track.artist_id_remixer.map(i64::from))
            .unwrap_or(0);
        values.insert("artist_id_remixer".to_string(), Value::Integer(id));
    }
    if values.contains_key("artist_id_composer") {
        let id = named_artist(track.composer.as_deref())?
            .or(track.artist_id_composer.map(i64::from))
            .unwrap_or(0);
        values.insert("artist_id_composer".to_string(), Value::Integer(id));
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
        let id = match track.label.as_deref().filter(|v| !v.trim().is_empty()) {
            Some(label) => find_or_insert_label(conn, label)?,
            None => None,
        }
        .or(track.label_id.map(i64::from))
        .unwrap_or(0);
        values.insert("label_id".to_string(), Value::Integer(id));
    }
    if values.contains_key("analysedBits") {
        values.insert(
            "analysedBits".to_string(),
            Value::Integer(if track.waveform_path.is_some() { 41 } else { 0 }),
        );
    }
    if values.contains_key("contentLink")
        && let Some(cl) = track.content_link
    {
        values.insert("contentLink".to_string(), Value::Integer(cl));
    }
    if values.contains_key("masterContentId")
        && let Some(mci) = track.master_content_id
    {
        values.insert("masterContentId".to_string(), Value::Integer(mci));
    }
    if values.contains_key("masterDbId")
        && let Some(mdb) = track.master_db_id
    {
        values.insert("masterDbId".to_string(), Value::Integer(mdb));
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
        values.insert("cueUpdateCount".to_string(), Value::Null);
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

fn timestamp_date(secs: Option<i64>) -> Option<String> {
    let dt = chrono::Utc.timestamp_opt(secs?, 0).single()?;
    Some(dt.format("%Y-%m-%d").to_string())
}

/// The part of the release date the tags do pin down when there's no full
/// date: `YYYY-MM` or `YYYY` from a partial date, else `YYYY` from the year.
fn partial_release_date(release_date: Option<&str>, release_year: Option<u32>) -> Option<String> {
    let tag = release_date.map(str::trim).unwrap_or("");
    fn digits(s: Option<&str>) -> Option<&str> {
        s.filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
    }
    if let (Some(year), Some("-"), Some(month)) =
        (digits(tag.get(..4)), tag.get(4..5), digits(tag.get(5..7)))
    {
        return Some(format!("{year}-{month}"));
    }
    if let Some(year) = digits(tag.get(..4)) {
        return Some(year.to_string());
    }
    release_year
        .filter(|year| (1..=9999).contains(year))
        .map(|year| format!("{year:04}"))
}

/// A partial date (`YYYY` or `YYYY-MM`) made a full one: `candidate` when it
/// falls inside it, else the start of that year or month.
pub fn complete_partial_date(partial: &str, candidate: Option<&str>) -> String {
    match candidate {
        Some(date) if date.len() == 10 && date.starts_with(partial) => date.to_string(),
        _ if partial.len() == 4 => format!("{partial}-01-01"),
        _ => format!("{partial}-01"),
    }
}

pub fn resolve_track_created_date_for_export(track: &ExportManifestTrack) -> Option<String> {
    normalize_export_date(track.recorded_date.as_deref())
        .or_else(|| normalize_export_date(track.release_date.as_deref()))
        .or_else(|| timestamp_date(track.file_timestamp_at))
}

/// Release date written to both the PDB and the eDB: the full date, else the
/// file timestamp (Unix seconds). A file timestamp that contradicts a year
/// (or month) the date or the year do carry is replaced by the start of that
/// year (or month).
pub fn resolve_release_date(
    release_date: Option<&str>,
    release_year: Option<u32>,
    file_timestamp_at: Option<i64>,
) -> Option<String> {
    normalize_export_date(release_date).or_else(|| {
        let file_date = timestamp_date(file_timestamp_at);
        match partial_release_date(release_date, release_year) {
            Some(partial) => Some(complete_partial_date(&partial, file_date.as_deref())),
            None => file_date,
        }
    })
}

/// [`resolve_release_date`] for an export manifest track.
pub fn resolve_track_release_date_for_export(track: &ExportManifestTrack) -> Option<String> {
    resolve_release_date(
        track.release_date.as_deref(),
        track.release_year,
        track.file_timestamp_at,
    )
}

/// Release year written to both the PDB and the eDB: the year tag, else the
/// year of the release date, so the two fields never disagree.
pub fn resolve_track_release_year_for_export(track: &ExportManifestTrack) -> Option<u16> {
    track
        .release_year
        .and_then(|year| u16::try_from(year).ok())
        .filter(|year| *year > 0)
        .or_else(|| {
            resolve_track_release_date_for_export(track)?
                .get(..4)?
                .parse::<u16>()
                .ok()
                .filter(|year| *year > 0)
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

pub fn find_or_insert_label(
    conn: &rusqlite::Connection,
    label_name: &str,
) -> BackendResult<Option<i64>> {
    if !table_exists(conn, "label") {
        return Ok(None);
    }
    let label_name = sanitize_metadata(label_name.trim());
    let label_name = label_name.trim();
    if label_name.is_empty() {
        return Ok(None);
    }
    if let Some(existing) = conn
        .query_row(
            "SELECT label_id FROM label WHERE lower(name) = lower(?1) LIMIT 1",
            params![label_name],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(Some(existing));
    }
    let next_id = next_numeric_id(conn, "label", "label_id")?;
    conn.execute(
        "INSERT INTO label (label_id, name) VALUES (?1, ?2)",
        params![next_id, label_name],
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
        if columns.iter().any(|c| c == "artist_id")
            && let Some(artist_name) = artist_name
            && let Some(artist_id) = find_or_insert_artist(conn, artist_name)?
        {
            let _ = conn.execute(
                "UPDATE album SET artist_id = COALESCE(artist_id, ?1) WHERE album_id = ?2",
                params![artist_id, existing],
            );
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
        EdbRead, ExportPlaylistData, open_edb, open_edb_from_usb_root, open_edb_rw, read_edb,
        replace_export_playlist_row_with_identity, upsert_export_playlist_row,
    };
    use crate::service::usb_vendor_compat::{USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR};
    use tempfile::tempdir;

    fn export_db_path(root: &std::path::Path) -> std::path::PathBuf {
        let dir = root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR);
        std::fs::create_dir_all(&dir).expect("create vendor db dir");
        dir.join("exportLibrary.db")
    }

    #[test]
    fn open_edb_rw_opens_plain_sqlite() {
        let temp = tempdir().expect("tempdir");
        let db_path = export_db_path(temp.path());
        let conn = rusqlite::Connection::open(&db_path).expect("create sqlite db");
        conn.execute(
            "CREATE TABLE playlist (playlist_id INTEGER PRIMARY KEY)",
            [],
        )
        .expect("create schema");
        drop(conn);

        let conn = open_edb_rw(temp.path());

        assert!(conn.is_some(), "plain sqlite db should open");
    }

    #[test]
    fn open_edb_rw_refuses_db_without_schema() {
        let temp = tempdir().expect("tempdir");
        let db_path = export_db_path(temp.path());
        let conn = rusqlite::Connection::open(&db_path).expect("create sqlite db");
        drop(conn);

        let conn = open_edb_rw(temp.path());

        assert!(conn.is_none(), "schema-less db should not open");
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
                .any(|w| w.message.starts_with("eDB unreadable")),
            "expected unreadable warning: {warnings:?}"
        );
    }

    #[test]
    fn encrypted_edb_opens_with_its_derived_key_read_only_and_read_write() {
        let temp = tempdir().expect("tempdir");
        let db_path = export_db_path(temp.path());
        crate::service::usb_utils::initialize_fullshape_edb(&db_path, "2026-01-01")
            .expect("create eDB");
        let salt = super::edb_salt(&db_path).expect("an encrypted eDB has a salt");

        let mut warnings = Vec::new();
        let ro = open_edb(&db_path, &mut warnings).expect("read-only open");
        assert!(warnings.is_empty(), "{warnings:?}");
        // Only an open with the derived key adds it.
        let memo = super::EDB_KEY_BY_SALT.get().expect("memo");
        assert!(memo.lock().unwrap().contains_key(&salt));
        let tables = |conn: &rusqlite::Connection| -> i64 {
            conn.query_row("SELECT COUNT(1) FROM sqlite_master", [], |r| r.get(0))
                .unwrap()
        };
        let rw = super::open_edb_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
            .expect("read-write open");
        assert_eq!(tables(&ro), tables(&rw));
        rw.execute("UPDATE property SET deviceName = 'STICK'", [])
            .expect("write through the derived key");
        drop(rw);
        drop(ro);
        let mut warnings = Vec::new();
        let again = open_edb(&db_path, &mut warnings).expect("reopen after a write");
        let name: String = again
            .query_row("SELECT deviceName FROM property", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "STICK");
    }

    fn export_db_with_schema(root: &std::path::Path) -> std::path::PathBuf {
        let db_path = export_db_path(root);
        let conn = rusqlite::Connection::open(&db_path).expect("create sqlite db");
        conn.execute(
            "CREATE TABLE playlist (playlist_id INTEGER PRIMARY KEY)",
            [],
        )
        .expect("create schema");
        db_path
    }

    // These three tests specifically verify that `open_edb_rw`/
    // `open_edb_from_usb_root` route through `usb_staging::stage_edb` (the
    // global-reading wrapper production code relies on) rather than through
    // `usb_staging`'s own `_with_root` test entry points (which take an
    // explicit root and are covered by `usb_staging`'s own test module).
    // `set_cache_root_for_test` only overrides `cache_root()` for the
    // *calling thread* (see its doc comment in usb_staging.rs) -- since
    // `cargo test` never runs two tests concurrently on the same thread,
    // this needs no lock and can't race with unrelated tests running on
    // other threads. The returned guard resets the override on drop
    // (including on panic), so nothing here leaks to a later test the
    // harness schedules on the same reused thread. The cache dir is still
    // leaked (`.keep()`, never deleted) as harmless extra insurance.
    #[test]
    fn open_edb_rw_with_staging_enabled_creates_local_cache_copy() {
        let cache_dir = tempdir().expect("cache dir").keep();
        let _guard = crate::service::usb_staging::set_cache_root_for_test(Some(cache_dir));

        let temp = tempdir().expect("tempdir");
        let db_path = export_db_with_schema(temp.path());

        let conn = open_edb_rw(temp.path());
        assert!(conn.is_some(), "staged db should still open");
        drop(conn);

        let staged_path = crate::service::usb_staging::stage_edb(temp.path()).expect("stage");
        assert!(
            staged_path.is_file(),
            "expected local cache copy at {staged_path:?}"
        );
        assert_ne!(
            staged_path, db_path,
            "staged path must differ from the real USB path"
        );
    }

    #[test]
    fn open_edb_from_usb_root_with_staging_skips_recopy_when_usb_source_unchanged() {
        let cache_dir = tempdir().expect("cache dir").keep();
        let _guard = crate::service::usb_staging::set_cache_root_for_test(Some(cache_dir));

        let temp = tempdir().expect("tempdir");
        export_db_with_schema(temp.path());

        let mut warnings = Vec::new();
        let _conn1 = open_edb_from_usb_root(temp.path(), &mut warnings);
        let staged_path = crate::service::usb_staging::stage_edb(temp.path()).expect("stage");
        let mtime_before = std::fs::metadata(&staged_path)
            .expect("stat staged copy")
            .modified()
            .expect("mtime");

        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut warnings2 = Vec::new();
        let _conn2 = open_edb_from_usb_root(temp.path(), &mut warnings2);
        let mtime_after = std::fs::metadata(&staged_path)
            .expect("stat staged copy")
            .modified()
            .expect("mtime");
        assert_eq!(
            mtime_before, mtime_after,
            "unchanged USB source should not trigger a re-copy on repeated opens"
        );
    }

    #[test]
    fn open_edb_from_usb_root_with_staging_recopies_after_usb_side_change() {
        let cache_dir = tempdir().expect("cache dir").keep();
        let _guard = crate::service::usb_staging::set_cache_root_for_test(Some(cache_dir));

        let temp = tempdir().expect("tempdir");
        let db_path = export_db_with_schema(temp.path());

        let mut warnings = Vec::new();
        let _conn1 = open_edb_from_usb_root(temp.path(), &mut warnings);

        // Simulate an external change landing directly on the USB file
        // (e.g. Rekordbox itself writing to the drive).
        std::thread::sleep(std::time::Duration::from_millis(20));
        let conn = rusqlite::Connection::open(&db_path).expect("reopen usb db");
        conn.execute("CREATE TABLE extra (id INTEGER PRIMARY KEY)", [])
            .expect("add table directly on usb");
        drop(conn);

        let mut warnings2 = Vec::new();
        let _conn2 = open_edb_from_usb_root(temp.path(), &mut warnings2);
        let staged_path = crate::service::usb_staging::stage_edb(temp.path()).expect("stage");
        let staged_conn = rusqlite::Connection::open(&staged_path).expect("open staged copy");
        let has_extra: i64 = staged_conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master WHERE type='table' AND name='extra'",
                [],
                |row| row.get(0),
            )
            .expect("query sqlite_master");
        assert_eq!(
            has_extra, 1,
            "local cache should reflect the USB-side change after reopening"
        );
    }

    #[test]
    fn open_edb_rw_with_staging_disabled_preserves_direct_usb_behavior() {
        let _guard = crate::service::usb_staging::set_cache_root_for_test(None);

        let temp = tempdir().expect("tempdir");
        export_db_with_schema(temp.path());

        let conn = open_edb_rw(temp.path());
        assert!(
            conn.is_some(),
            "with staging disabled, connection should still open directly against the USB path"
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
    fn replace_export_playlist_row_with_identity_relocates_folder_collider() {
        // Regression test: a target playlist_id that happens to already be
        // occupied by a FOLDER row (attribute != 0) must still be detected
        // and relocated. The old query/update pair scoped both the collision
        // check and the reassignment to `attribute = 0`, so a folder sitting
        // on the target id went completely undetected — the subsequent
        // INSERT then hit "UNIQUE constraint failed: playlist.playlist_id".
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
              (5, 'Some Folder', 1, 0, 0);
            "#,
        )
        .expect("seed a folder occupying the target id");

        let playlist = ExportPlaylistData {
            id: "pl-new".to_string(),
            name: "Top40".to_string(),
            tracks: Vec::new(),
        };
        let playlist_id = replace_export_playlist_row_with_identity(&conn, &playlist, 5, 0)
            .expect("must relocate the folder collider instead of failing the UNIQUE constraint");
        assert_eq!(playlist_id, 5);

        let new_row: (String, i64) = conn
            .query_row(
                "SELECT name, attribute FROM playlist WHERE playlist_id = 5",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("target id now holds the new playlist row");
        assert_eq!(new_row, ("Top40".to_string(), 0));

        let folder_row: (String, i64) = conn
            .query_row(
                "SELECT name, attribute FROM playlist WHERE name = 'Some Folder'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("folder must have been relocated, not lost");
        assert_eq!(
            folder_row.1, 1,
            "relocated row must keep its folder attribute"
        );
    }

    #[test]
    fn upsert_export_playlist_row_reruns_produce_stable_contiguous_sequence() {
        // Regression test: a naive "increment every other sibling by 1" front-move
        // (the old implementation) stays correct for one sibling, but diverges
        // from the PDB side's fresh contiguous renumbering as soon as a sibling
        // sorts after the target, and drifts further on every repeated run. The
        // PDB writer (`move_playlist_tree_row_to_front`) always recomputes a
        // tight 0..N-1 sequence from current relative order, so eDB must match
        // that exactly (same values, not just same relative order) or strict
        // parity's "Playlist ordering parity" check fails for every sibling.
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
              (1, 'A0', 0, 0, 0),
              (2, 'A1', 0, 0, 1),
              (3, 'Target', 0, 0, 2),
              (4, 'A2', 0, 0, 3),
              (5, 'A3', 0, 0, 4);
            "#,
        )
        .expect("seed playlists");

        let playlist = ExportPlaylistData {
            id: "pl-target".to_string(),
            name: "Target".to_string(),
            tracks: Vec::new(),
        };

        let expected_after_one_run = vec![(1, 1), (2, 2), (3, 0), (4, 3), (5, 4)];
        for _ in 0..2 {
            let playlist_id =
                upsert_export_playlist_row(&conn, &playlist).expect("upsert existing playlist");
            assert_eq!(playlist_id, 3);
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
                rows, expected_after_one_run,
                "sequenceNo must be a fresh contiguous 0..N-1 sequence (matching PDB's \
                 recomputed sort_order) on every run, not an unconditional +1 to every \
                 other sibling"
            );
        }
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
    fn read_edb_reads_basic_track_data() {
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
              length INTEGER,
              fileSize INTEGER
            );
            INSERT INTO artist (artist_id, name) VALUES (1, 'Artist');
            INSERT INTO album (album_id, name) VALUES (1, 'Album');
            INSERT INTO "key" (key_id, name) VALUES (1, '8A');
            INSERT INTO image (image_id, path) VALUES (1, '/Contents/art.jpg');
            INSERT INTO content (
              content_id, title, artist_id_artist, album_id, key_id, image_id,
              path, analysisDataFilePath, bpmx100, length, fileSize
            ) VALUES (
              10, 'Track', 1, 1, 1, 1,
              '/Contents/track.mp3', '/PIONEER/USBANLZ/P001/T/ANLZ0000.DAT', 12800, 180, 4096
            );
            "#,
        )
        .expect("seed eDB");

        let mut warnings = Vec::new();
        let conn = open_edb_from_usb_root(temp.path(), &mut warnings).expect("open eDB");
        let tracks = EdbRead {
            tracks: true,
            resolve_paths: true,
            ..EdbRead::default()
        };
        let library = read_edb(&conn, temp.path(), tracks).expect("eDB read");
        assert!(library.playlists.is_empty(), "only what was asked for");
        let index = library.tracks;
        let track = index.get(&10).expect("content id 10");
        assert_eq!(track.title, "Track");
        assert_eq!(track.artist, "Artist");
        assert_eq!(track.album.as_deref(), Some("Album"));
        assert_eq!(track.key.as_deref(), Some("8A"));
        assert_eq!(track.bpm, Some(128.0));
        assert_eq!(track.duration_ms, Some(180_000));
        assert_eq!(track.file_size_bytes, Some(4096), "the recorded size");
    }

    #[test]
    fn read_edb_reads_playlists_and_date_created() {
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
              dateCreated TEXT,
              fileSize INTEGER
            );
            CREATE TABLE playlist_content (playlist_id INTEGER, content_id INTEGER, sequenceNo INTEGER);
            INSERT INTO artist (artist_id, name) VALUES (1, 'Artist');
            INSERT INTO album (album_id, name) VALUES (1, 'Album');
            INSERT INTO "key" (key_id, name) VALUES (1, '8A');
            INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (2, 'Set A', 0, 1);
            INSERT INTO content (
              content_id, title, artist_id_artist, album_id, key_id,
              path, analysisDataFilePath, bpmx100, length, trackNo, dateCreated, fileSize
            ) VALUES (
              42, 'Track A', 1, 1, 1,
              '/Contents/a.mp3', '/PIONEER/USBANLZ/P001/A/ANLZ0000.DAT', 12500, 200, 3, '2024-02-10',
              5120
            );
            INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (2, 42, 1);
            "#,
        )
        .expect("seed eDB");

        let mut warnings = Vec::new();
        let conn = open_edb_from_usb_root(temp.path(), &mut warnings)
            .expect("open_edb_from_usb_root should resolve path");
        let resolved = EdbRead {
            playlists: true,
            resolve_paths: true,
            ..EdbRead::default()
        };
        let library = read_edb(&conn, temp.path(), resolved).expect("eDB read");
        assert!(library.tracks.is_empty() && library.date_created.is_empty());
        let playlists = library.playlists;
        let set_a = playlists.get("Set A").expect("Set A playlist");
        assert_eq!(set_a.playlist_id, 2);
        assert_eq!(set_a.tracks.len(), 1);
        assert_eq!(set_a.tracks[0].track_number, Some(3));
        assert_eq!(
            set_a.tracks[0].file_size_bytes,
            Some(5120),
            "the recorded size"
        );
        assert!(
            std::path::Path::new(&set_a.tracks[0].file_path).starts_with(temp.path()),
            "default reader should resolve USB-relative media paths: {:?}",
            set_a.tracks[0]
        );

        // Everything, paths as stored; resolving them afterwards gives what a
        // resolving read gives.
        let all = read_edb(&conn, temp.path(), EdbRead::ALL).expect("eDB read");
        assert_eq!(
            all.resolved_playlists(temp.path())["Set A"].tracks[0].file_path,
            set_a.tracks[0].file_path
        );
        let db_only_playlists = &all.playlists;
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

        let dates = &all.date_created;
        assert_eq!(dates.get(&42).map(String::as_str), Some("2024-02-10"));
    }
}
