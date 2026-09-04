use std::path::{Path, PathBuf};

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

use crate::error::{BackendError, BackendResult};

const CURRENT_SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone)]
pub struct Db {
    db_path: PathBuf,
    pool: Pool<SqliteConnectionManager>,
}

impl Db {
    pub fn new(data_dir: impl AsRef<Path>) -> BackendResult<Self> {
        let data_dir = data_dir.as_ref();
        std::fs::create_dir_all(data_dir)?;

        let db_path = data_dir.join("backend.db");
        let manager = SqliteConnectionManager::file(&db_path);
        let pool = Pool::builder().max_size(8).build(manager).map_err(|err| {
            BackendError::Internal(format!("failed to create sqlite pool: {err}"))
        })?;
        let db = Self { db_path, pool };
        db.migrate()?;
        Ok(db)
    }

    pub fn connect(&self) -> BackendResult<PooledConnection<SqliteConnectionManager>> {
        let conn = self.pool.get().map_err(|err| {
            BackendError::Internal(format!("failed to get sqlite connection: {err}"))
        })?;
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;
            PRAGMA journal_mode = WAL;
            PRAGMA busy_timeout = 5000;
            "#,
        )?;
        Ok(conn)
    }

    pub fn data_dir(&self) -> PathBuf {
        self.db_path
            .parent()
            .map_or_else(|| PathBuf::from("."), |p| p.to_path_buf())
    }

    fn migrate(&self) -> BackendResult<()> {
        let conn = self.connect()?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS tracks (
              id TEXT PRIMARY KEY,
              title TEXT NOT NULL,
              artist TEXT NOT NULL,
              album TEXT,
              track_number INTEGER,
              bpm REAL,
              tonality TEXT,
              file_path TEXT NOT NULL UNIQUE,
              file_size_bytes INTEGER,
              file_modified_at TEXT,
              format_ext TEXT,
              sample_rate_hz INTEGER,
              bit_depth INTEGER,
              bitrate_kbps INTEGER,
              disc_number INTEGER,
              subtitle TEXT,
              comment TEXT,
              isrc TEXT,
              release_year INTEGER,
              release_date TEXT,
              recorded_date TEXT,
              duration_ms INTEGER,
              artwork_path TEXT,
              waveform_peaks_path TEXT,
              title_for_search TEXT,
              kuvo_delivery_comment TEXT,
              dj_play_count INTEGER,
              rating INTEGER,
              color_id INTEGER,
              artist_id_lyricist INTEGER,
              artist_id_original_artist INTEGER,
              artist_id_remixer INTEGER,
              artist_id_composer INTEGER,
              genre_id INTEGER,
              label_id INTEGER,
              match_fingerprint TEXT,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS track_cues (
              id TEXT PRIMARY KEY,
              track_id TEXT NOT NULL,
              position_ms INTEGER NOT NULL,
              color_id INTEGER,
              name TEXT,
              sort_order INTEGER NOT NULL DEFAULT 0,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              FOREIGN KEY(track_id) REFERENCES tracks(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS playlists (
              id TEXT PRIMARY KEY,
              name TEXT NOT NULL,
              source TEXT NOT NULL,
              last_exported_at TEXT,
              last_exported_usb_root TEXT,
              last_exported_track_count INTEGER,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS playlist_tracks (
              id TEXT PRIMARY KEY,
              playlist_id TEXT NOT NULL,
              track_id TEXT NOT NULL,
              position INTEGER NOT NULL,
              added_at TEXT NOT NULL,
              FOREIGN KEY(playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
              FOREIGN KEY(track_id) REFERENCES tracks(id) ON DELETE CASCADE,
              UNIQUE(playlist_id, position)
            );

            CREATE TABLE IF NOT EXISTS usb_devices (
              id TEXT PRIMARY KEY,
              root_path TEXT NOT NULL,
              root_path_key TEXT NOT NULL UNIQUE,
              label TEXT,
              mounted INTEGER NOT NULL DEFAULT 0,
              first_seen_at TEXT NOT NULL,
              last_seen_at TEXT NOT NULL,
              deleted_at TEXT,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS track_usb_links (
              id TEXT PRIMARY KEY,
              track_id TEXT NOT NULL,
              usb_device_id TEXT NOT NULL,
              usb_file_path TEXT NOT NULL,
              first_seen_at TEXT NOT NULL,
              last_seen_at TEXT NOT NULL,
              FOREIGN KEY(track_id) REFERENCES tracks(id) ON DELETE CASCADE,
              FOREIGN KEY(usb_device_id) REFERENCES usb_devices(id) ON DELETE CASCADE,
              UNIQUE(usb_device_id, usb_file_path)
            );

            CREATE TABLE IF NOT EXISTS usb_device_exports (
              id TEXT PRIMARY KEY,
              usb_device_id TEXT NOT NULL,
              playlist_id TEXT,
              playlist_name TEXT NOT NULL,
              exported_at TEXT NOT NULL,
              track_count INTEGER NOT NULL,
              track_fingerprints TEXT NOT NULL,
              created_at TEXT NOT NULL,
              FOREIGN KEY(usb_device_id) REFERENCES usb_devices(id) ON DELETE CASCADE,
              FOREIGN KEY(playlist_id) REFERENCES playlists(id) ON DELETE SET NULL
            );

            CREATE TABLE IF NOT EXISTS app_settings (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS schema_version (
              id INTEGER PRIMARY KEY CHECK (id = 1),
              version INTEGER NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_tracks_title_artist_album
              ON tracks (title, artist, album);
            CREATE INDEX IF NOT EXISTS idx_playlist_tracks_playlist
              ON playlist_tracks (playlist_id, position);
            CREATE INDEX IF NOT EXISTS idx_playlist_tracks_track
              ON playlist_tracks (track_id);
            CREATE INDEX IF NOT EXISTS idx_usb_devices_last_seen
              ON usb_devices (last_seen_at DESC);
            CREATE INDEX IF NOT EXISTS idx_track_usb_links_track
              ON track_usb_links (track_id);
            CREATE INDEX IF NOT EXISTS idx_track_usb_links_device
              ON track_usb_links (usb_device_id);
            CREATE INDEX IF NOT EXISTS idx_usb_device_exports_device
              ON usb_device_exports (usb_device_id, exported_at DESC);
            CREATE INDEX IF NOT EXISTS idx_track_cues_track
              ON track_cues (track_id, sort_order);
            "#,
        )?;

        ensure_tracks_column(&conn, "track_number", "INTEGER")?;
        ensure_tracks_column(&conn, "waveform_peaks_path", "TEXT")?;
        ensure_tracks_column(&conn, "match_fingerprint", "TEXT")?;
        ensure_tracks_column(&conn, "format_ext", "TEXT")?;
        ensure_tracks_column(&conn, "sample_rate_hz", "INTEGER")?;
        ensure_tracks_column(&conn, "bit_depth", "INTEGER")?;
        ensure_tracks_column(&conn, "bitrate_kbps", "INTEGER")?;
        ensure_tracks_column(&conn, "duration_ms", "INTEGER")?;
        ensure_tracks_column(&conn, "title_for_search", "TEXT")?;
        ensure_tracks_column(&conn, "kuvo_delivery_comment", "TEXT")?;
        ensure_tracks_column(&conn, "dj_play_count", "INTEGER")?;
        ensure_tracks_column(&conn, "rating", "INTEGER")?;
        ensure_tracks_column(&conn, "color_id", "INTEGER")?;
        ensure_tracks_column(&conn, "artist_id_lyricist", "INTEGER")?;
        ensure_tracks_column(&conn, "artist_id_original_artist", "INTEGER")?;
        ensure_tracks_column(&conn, "artist_id_remixer", "INTEGER")?;
        ensure_tracks_column(&conn, "artist_id_composer", "INTEGER")?;
        ensure_tracks_column(&conn, "genre_id", "INTEGER")?;
        ensure_tracks_column(&conn, "label_id", "INTEGER")?;
        ensure_tracks_column(&conn, "bpm_analyzer", "TEXT")?;
        ensure_tracks_column(&conn, "first_beat_ms", "INTEGER")?;
        ensure_tracks_column(&conn, "first_beat_ms_source", "TEXT")?;
        ensure_tracks_column(&conn, "genre", "TEXT")?;
        ensure_tracks_column(&conn, "master_db_source", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_tracks_column(&conn, "wav_extensible_kind", "TEXT")?;
        ensure_playlists_column(&conn, "last_exported_at", "TEXT")?;
        ensure_playlists_column(&conn, "last_exported_usb_root", "TEXT")?;
        ensure_playlists_column(&conn, "last_exported_track_count", "INTEGER")?;
        conn.execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_tracks_match_fingerprint
              ON tracks (match_fingerprint);
            "#,
        )?;
        set_schema_version(&conn, CURRENT_SCHEMA_VERSION)?;

        Ok(())
    }
}

/// Allowed column names for dynamic ALTER TABLE. Prevents SQL injection if
/// callers ever pass user-controlled strings (currently all literal).
const ALLOWED_TRACK_COLUMNS: &[&str] = &[
    "track_number",
    "match_fingerprint",
    "waveform_peaks_path",
    "format_ext",
    "sample_rate_hz",
    "bit_depth",
    "bitrate_kbps",
    "duration_ms",
    "title_for_search",
    "kuvo_delivery_comment",
    "dj_play_count",
    "rating",
    "color_id",
    "artist_id_lyricist",
    "artist_id_original_artist",
    "artist_id_remixer",
    "artist_id_composer",
    "genre_id",
    "label_id",
    "bpm_analyzer",
    "first_beat_ms",
    "first_beat_ms_source",
    "genre",
    "master_db_source",
    "wav_extensible_kind",
];
const ALLOWED_PLAYLIST_COLUMNS: &[&str] = &[
    "last_exported_at",
    "last_exported_usb_root",
    "last_exported_track_count",
];

fn ensure_tracks_column(
    conn: &Connection,
    column_name: &str,
    definition: &str,
) -> BackendResult<()> {
    if !ALLOWED_TRACK_COLUMNS.contains(&column_name) {
        return Err(crate::error::BackendError::Internal(format!(
            "ensure_tracks_column: column '{column_name}' not in allowlist"
        )));
    }

    let mut stmt = conn.prepare("PRAGMA table_info(tracks)")?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for col in columns {
        if col?.eq_ignore_ascii_case(column_name) {
            return Ok(());
        }
    }

    let sql = format!("ALTER TABLE tracks ADD COLUMN {column_name} {definition}");
    conn.execute_batch(&sql)?;
    Ok(())
}

fn ensure_playlists_column(
    conn: &Connection,
    column_name: &str,
    definition: &str,
) -> BackendResult<()> {
    if !ALLOWED_PLAYLIST_COLUMNS.contains(&column_name) {
        return Err(crate::error::BackendError::Internal(format!(
            "ensure_playlists_column: column '{column_name}' not in allowlist"
        )));
    }

    let mut stmt = conn.prepare("PRAGMA table_info(playlists)")?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for col in columns {
        if col?.eq_ignore_ascii_case(column_name) {
            return Ok(());
        }
    }

    let sql = format!("ALTER TABLE playlists ADD COLUMN {column_name} {definition}");
    conn.execute_batch(&sql)?;
    Ok(())
}

fn set_schema_version(conn: &Connection, version: i64) -> BackendResult<()> {
    conn.execute(
        r#"
        INSERT INTO schema_version (id, version, updated_at)
        VALUES (1, ?1, datetime('now'))
        ON CONFLICT(id) DO UPDATE SET
          version = CASE
            WHEN excluded.version > schema_version.version THEN excluded.version
            ELSE schema_version.version
          END,
          updated_at = CASE
            WHEN excluded.version > schema_version.version THEN datetime('now')
            ELSE schema_version.updated_at
          END
        "#,
        rusqlite::params![version],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CURRENT_SCHEMA_VERSION, Db, ensure_tracks_column};
    use rusqlite::Connection;

    #[test]
    fn migrate_sets_schema_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::new(dir.path()).expect("db init");
        let conn = db.connect().expect("db connect");
        let version: i64 = conn
            .query_row(
                "SELECT version FROM schema_version WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("schema_version row");
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn migrate_creates_track_cues_table_and_first_beat_source_column() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::new(dir.path()).expect("db init");
        let conn = db.connect().expect("db connect");

        let cue_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(track_cues)")
            .expect("prepare")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect");
        for expected in ["id", "track_id", "position_ms", "color_id", "name", "sort_order"] {
            assert!(
                cue_cols.iter().any(|c| c == expected),
                "track_cues missing column {expected}; got {cue_cols:?}"
            );
        }
        for absent in ["kind", "hot_cue_index", "is_loop"] {
            assert!(
                !cue_cols.iter().any(|c| c == absent),
                "track_cues should not have column {absent}; got {cue_cols:?}"
            );
        }

        let track_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(tracks)")
            .expect("prepare")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect");
        assert!(
            track_cols.iter().any(|c| c == "first_beat_ms_source"),
            "tracks missing first_beat_ms_source; got {track_cols:?}"
        );

        // Cues cascade-delete with their track.
        conn.execute_batch(
            r#"
            INSERT INTO tracks (id, title, artist, file_path, created_at, updated_at)
              VALUES ('t1', 'a', 'b', '/x.wav', 'now', 'now');
            INSERT INTO track_cues (id, track_id, position_ms, color_id, name, sort_order, created_at, updated_at)
              VALUES ('c1', 't1', 1000, 5, 'Drop', 0, 'now', 'now');
            "#,
        )
        .expect("seed cue");
        conn.execute("DELETE FROM tracks WHERE id = 't1'", [])
            .expect("delete track");
        let remaining: i64 = conn
            .query_row("SELECT COUNT(1) FROM track_cues", [], |r| r.get(0))
            .expect("count");
        assert_eq!(remaining, 0, "track_cues should cascade-delete with the track");
    }

    #[test]
    fn ensure_tracks_column_rejects_unknown_column_name() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            r#"
            CREATE TABLE tracks (
              id TEXT PRIMARY KEY,
              title TEXT NOT NULL,
              artist TEXT NOT NULL,
              file_path TEXT NOT NULL UNIQUE,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            "#,
        )
        .expect("create tracks table");

        let err = ensure_tracks_column(&conn, "totally_unknown_column", "TEXT")
            .expect_err("unknown column should be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("not in allowlist"),
            "unexpected error for disallowed ensure_tracks_column: {msg}"
        );
    }
}
