use std::path::{Path, PathBuf};

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

use crate::error::{BackendError, BackendResult};

const CURRENT_SCHEMA_VERSION: i64 = 1;

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
        ensure_tracks_column(&conn, "genre", "TEXT")?;
        ensure_tracks_column(&conn, "master_db_source", "INTEGER NOT NULL DEFAULT 0")?;
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
    "genre",
    "master_db_source",
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
