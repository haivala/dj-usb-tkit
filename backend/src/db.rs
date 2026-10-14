use std::path::{Path, PathBuf};

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, OptionalExtension};

use crate::error::{BackendError, BackendResult};

const CURRENT_SCHEMA_VERSION: i64 = 4;

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
              file_modified_at INTEGER,
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
              is_playback_start INTEGER NOT NULL DEFAULT 0,
              is_memory INTEGER NOT NULL DEFAULT 0,
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

            CREATE TABLE IF NOT EXISTS data_migrations (
              id TEXT PRIMARY KEY,
              applied_at TEXT NOT NULL
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

        rename_legacy_rekordbox_names(&conn)?;
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
        ensure_tracks_column(&conn, "rekordbox_db_source", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_tracks_column(&conn, "mixxx_db_source", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_tracks_column(&conn, "wav_extensible_kind", "TEXT")?;
        ensure_tracks_column(&conn, "tonality_source", "TEXT")?;
        ensure_tracks_column(&conn, "genre_source", "TEXT")?;
        ensure_playlists_column(&conn, "last_exported_at", "TEXT")?;
        ensure_playlists_column(&conn, "last_exported_usb_root", "TEXT")?;
        ensure_playlists_column(&conn, "last_exported_track_count", "INTEGER")?;
        ensure_playlists_column(&conn, "import_source", "TEXT")?;
        ensure_playlists_column(&conn, "sort_order", "INTEGER")?;
        ensure_track_cues_column(&conn, "is_playback_start", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_track_cues_column(&conn, "is_memory", "INTEGER NOT NULL DEFAULT 0")?;
        conn.execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_tracks_match_fingerprint
              ON tracks (match_fingerprint);
            "#,
        )?;
        store_file_modified_at_as_integer(&conn)?;
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
    "rekordbox_db_source",
    "mixxx_db_source",
    "wav_extensible_kind",
    "tonality_source",
    "genre_source",
];
const ALLOWED_PLAYLIST_COLUMNS: &[&str] = &[
    "last_exported_at",
    "last_exported_usb_root",
    "last_exported_track_count",
    "import_source",
    "sort_order",
];

const ALLOWED_TRACK_CUES_COLUMNS: &[&str] = &["is_playback_start", "is_memory"];

fn ensure_tracks_column(
    conn: &Connection,
    column_name: &str,
    definition: &str,
) -> BackendResult<()> {
    ensure_column(
        conn,
        "tracks",
        ALLOWED_TRACK_COLUMNS,
        column_name,
        definition,
    )
}

fn ensure_playlists_column(
    conn: &Connection,
    column_name: &str,
    definition: &str,
) -> BackendResult<()> {
    ensure_column(
        conn,
        "playlists",
        ALLOWED_PLAYLIST_COLUMNS,
        column_name,
        definition,
    )
}

fn ensure_track_cues_column(
    conn: &Connection,
    column_name: &str,
    definition: &str,
) -> BackendResult<()> {
    ensure_column(
        conn,
        "track_cues",
        ALLOWED_TRACK_CUES_COLUMNS,
        column_name,
        definition,
    )
}

/// Idempotent `ALTER TABLE {table} ADD COLUMN`, gated on `allowed`.
fn ensure_column(
    conn: &Connection,
    table: &str,
    allowed: &[&str],
    column_name: &str,
    definition: &str,
) -> BackendResult<()> {
    if !allowed.contains(&column_name) {
        return Err(crate::error::BackendError::Internal(format!(
            "ensure_{table}_column: column '{column_name}' not in allowlist"
        )));
    }

    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for col in columns {
        if col?.eq_ignore_ascii_case(column_name) {
            return Ok(());
        }
    }

    let sql = format!("ALTER TABLE {table} ADD COLUMN {column_name} {definition}");
    conn.execute_batch(&sql)?;
    Ok(())
}

/// Schema v3 renamed the rekordbox library's "master DB" names: the
/// `tracks.master_db_source` column and the chip's stored on/off setting.
/// Both steps are no-ops once done, and on a fresh database.
fn rename_legacy_rekordbox_names(conn: &Connection) -> BackendResult<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(tracks)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    let has = |name: &str| columns.iter().any(|c| c.eq_ignore_ascii_case(name));
    if has("master_db_source") && !has("rekordbox_db_source") {
        conn.execute_batch(
            "ALTER TABLE tracks RENAME COLUMN master_db_source TO rekordbox_db_source",
        )?;
    }
    conn.execute(
        "UPDATE OR IGNORE app_settings SET key = 'ui_rekordbox_db_enabled_v1'
         WHERE key = 'ui_master_db_enabled_v1'",
        [],
    )?;
    conn.execute(
        "DELETE FROM app_settings WHERE key = 'ui_master_db_enabled_v1'",
        [],
    )?;
    Ok(())
}

/// Schema v4: `tracks.file_modified_at` (the file's Unix seconds) is an
/// `INTEGER` column. Before, it was `TEXT`, and SQLite keeps whatever goes
/// into a `TEXT` column as text, so the column's declared type has to
/// change. SQLite can't change a column's type in place, so this rebuilds
/// `tracks` the way SQLite's docs describe ("Making Other Kinds Of Table
/// Schema Changes"): a copy made from the table's own current definition with
/// only that column's type changed, so every other column, index and row
/// stays exactly as it was. A no-op once done, and on a fresh database.
fn store_file_modified_at_as_integer(conn: &Connection) -> BackendResult<()> {
    let declared_type: Option<String> = conn
        .prepare("SELECT type FROM pragma_table_info('tracks') WHERE name = 'file_modified_at'")?
        .query_row([], |row| row.get(0))
        .optional()?;
    if !declared_type.is_some_and(|t| t.eq_ignore_ascii_case("TEXT")) {
        return Ok(());
    }

    let table_sql: String = conn.query_row(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'tracks'",
        [],
        |row| row.get(0),
    )?;
    let new_table_sql = rebuilt_tracks_sql(&table_sql).ok_or_else(|| {
        BackendError::Internal(format!(
            "schema v4: unexpected tracks definition, not rebuilding: {table_sql}"
        ))
    })?;
    let index_sql = conn
        .prepare(
            "SELECT sql FROM sqlite_schema
             WHERE type = 'index' AND tbl_name = 'tracks' AND sql IS NOT NULL",
        )?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;

    // Dropping the old table would otherwise cascade-delete every cue,
    // playlist entry and USB link pointing at it. This pragma is ignored
    // inside a transaction, so it is set before and restored after.
    conn.execute_batch("PRAGMA foreign_keys = OFF")?;
    let result = (|| -> BackendResult<()> {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(&new_table_sql)?;
        tx.execute_batch(
            "INSERT INTO tracks_v4 SELECT * FROM tracks;
             UPDATE tracks_v4 SET file_modified_at = NULL
               WHERE typeof(file_modified_at) NOT IN ('integer', 'null');
             DROP TABLE tracks;
             ALTER TABLE tracks_v4 RENAME TO tracks;",
        )?;
        for sql in &index_sql {
            tx.execute_batch(sql)?;
        }
        let broken_links: i64 =
            tx.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })?;
        if broken_links > 0 {
            return Err(BackendError::Internal(format!(
                "schema v4: tracks rebuild would break {broken_links} foreign key(s)"
            )));
        }
        tx.commit()?;
        Ok(())
    })();
    conn.execute_batch("PRAGMA foreign_keys = ON")?;
    result
}

/// `tracks`' `CREATE TABLE` text as `tracks_v4` with `file_modified_at` as
/// `INTEGER`, or `None` if that column isn't declared exactly once as
/// `file_modified_at TEXT`. Everything from the column list on is kept as is;
/// the name before it may be quoted (SQLite quotes it after a rename).
fn rebuilt_tracks_sql(table_sql: &str) -> Option<String> {
    let body = &table_sql[table_sql.find('(')?..];
    let old_column = "file_modified_at TEXT";
    if body.matches(old_column).count() != 1 {
        return None;
    }
    Some(format!(
        "CREATE TABLE tracks_v4 {}",
        body.replacen(old_column, "file_modified_at INTEGER", 1)
    ))
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
    fn migrate_renames_legacy_master_db_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        {
            let db = Db::new(dir.path()).expect("db init");
            let conn = db.connect().expect("db connect");
            // Put the database back in its pre-v3 shape.
            conn.execute_batch(
                r#"
                ALTER TABLE tracks RENAME COLUMN rekordbox_db_source TO master_db_source;
                INSERT INTO tracks (id, title, artist, file_path, created_at, updated_at,
                                    master_db_source)
                  VALUES ('t1', 'T', 'A', '/m/a.mp3', 'now', 'now', 1);
                INSERT INTO app_settings (key, value, updated_at)
                  VALUES ('ui_master_db_enabled_v1', 'false', 'now');
                "#,
            )
            .expect("downgrade schema");
        }

        let db = Db::new(dir.path()).expect("db reopen");
        let conn = db.connect().expect("db connect");
        let source: i64 = conn
            .query_row(
                "SELECT rekordbox_db_source FROM tracks WHERE id = 't1'",
                [],
                |row| row.get(0),
            )
            .expect("renamed column keeps its value");
        assert_eq!(source, 1);
        let setting: String = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = 'ui_rekordbox_db_enabled_v1'",
                [],
                |row| row.get(0),
            )
            .expect("setting moved to the new key");
        assert_eq!(setting, "false");
        let legacy: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM app_settings WHERE key = 'ui_master_db_enabled_v1'",
                [],
                |row| row.get(0),
            )
            .expect("count legacy setting");
        assert_eq!(legacy, 0);
    }

    fn column_type(conn: &Connection, table: &str, column: &str) -> String {
        conn.query_row(
            "SELECT type FROM pragma_table_info(?1) WHERE name = ?2",
            [table, column],
            |row| row.get(0),
        )
        .expect("column type")
    }

    #[test]
    fn migrate_stores_file_modified_at_as_integer_and_keeps_linked_rows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let index_count = |conn: &Connection| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'index' AND tbl_name = 'tracks'",
                [],
                |row| row.get(0),
            )
            .expect("count indexes")
        };
        let indexes_before;
        let columns_before: Vec<String>;
        {
            let db = Db::new(dir.path()).expect("db init");
            let conn = db.connect().expect("db connect");
            indexes_before = index_count(&conn);
            // Put `tracks` back in its pre-v4 shape: the column declared TEXT.
            let sql: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'tracks'",
                    [],
                    |row| row.get(0),
                )
                .expect("tracks sql");
            let old_sql = sql
                .replacen("CREATE TABLE tracks", "CREATE TABLE tracks_old", 1)
                .replacen("file_modified_at INTEGER", "file_modified_at TEXT", 1);
            conn.execute_batch(&format!(
                "PRAGMA foreign_keys = OFF;
                 {old_sql};
                 DROP TABLE tracks;
                 ALTER TABLE tracks_old RENAME TO tracks;
                 CREATE INDEX idx_tracks_title_artist_album ON tracks (title, artist, album);
                 CREATE INDEX idx_tracks_match_fingerprint ON tracks (match_fingerprint);
                 PRAGMA foreign_keys = ON;
                 UPDATE schema_version SET version = 3;"
            ))
            .expect("downgrade tracks");
            assert_eq!(column_type(&conn, "tracks", "file_modified_at"), "TEXT");
            columns_before = conn
                .prepare("SELECT name FROM pragma_table_info('tracks')")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            conn.execute_batch(
                r#"
                INSERT INTO tracks (id, title, artist, file_path, file_modified_at,
                                    created_at, updated_at)
                  VALUES ('t1', 'T1', 'A', '/m/a.mp3', 1714521600, 'now', 'now'),
                         ('t2', 'T2', 'A', '/m/b.mp3', 'not a number', 'now', 'now'),
                         ('t3', 'T3', 'A', '/m/c.mp3', NULL, 'now', 'now');
                INSERT INTO track_cues (id, track_id, position_ms, created_at, updated_at)
                  VALUES ('c1', 't1', 1000, 'now', 'now');
                INSERT INTO playlists (id, name, source, created_at, updated_at)
                  VALUES ('p1', 'Set', 'app', 'now', 'now');
                INSERT INTO playlist_tracks (id, playlist_id, track_id, position, added_at)
                  VALUES ('pt1', 'p1', 't1', 0, 'now');
                INSERT INTO usb_devices (id, root_path, root_path_key, first_seen_at,
                                         last_seen_at, created_at, updated_at)
                  VALUES ('u1', '/media/stick', '/media/stick', 'now', 'now', 'now', 'now');
                INSERT INTO track_usb_links (id, track_id, usb_device_id, usb_file_path,
                                             first_seen_at, last_seen_at)
                  VALUES ('l1', 't1', 'u1', '/Contents/a.mp3', 'now', 'now');
                "#,
            )
            .expect("seed rows");
            let stored: String = conn
                .query_row(
                    "SELECT typeof(file_modified_at) FROM tracks WHERE id = 't1'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(stored, "text", "a TEXT column stores the number as text");
        }

        let db = Db::new(dir.path()).expect("db reopen");
        let conn = db.connect().expect("db connect");
        assert_eq!(column_type(&conn, "tracks", "file_modified_at"), "INTEGER");
        let columns_after: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('tracks')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(columns_after, columns_before, "same columns, same order");
        assert_eq!(index_count(&conn), indexes_before);

        let modified = |id: &str| -> Option<i64> {
            conn.query_row(
                "SELECT file_modified_at FROM tracks WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .expect("read file_modified_at")
        };
        assert_eq!(modified("t1"), Some(1_714_521_600));
        assert_eq!(
            modified("t2"),
            None,
            "a value that isn't a number is dropped"
        );
        assert_eq!(modified("t3"), None);

        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |row| row.get(0)).unwrap() };
        assert_eq!(count("SELECT COUNT(*) FROM tracks"), 3);
        assert_eq!(count("SELECT COUNT(*) FROM track_cues"), 1);
        assert_eq!(count("SELECT COUNT(*) FROM playlist_tracks"), 1);
        assert_eq!(count("SELECT COUNT(*) FROM track_usb_links"), 1);
        assert_eq!(count("PRAGMA foreign_keys"), 1, "foreign keys are back on");
        assert_eq!(
            count("SELECT version FROM schema_version"),
            CURRENT_SCHEMA_VERSION
        );

        // The links still cascade: they point at the rebuilt table.
        conn.execute("DELETE FROM tracks WHERE id = 't1'", [])
            .expect("delete track");
        assert_eq!(count("SELECT COUNT(*) FROM track_cues"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM playlist_tracks"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM track_usb_links"), 0);
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
        for expected in [
            "id",
            "track_id",
            "position_ms",
            "color_id",
            "name",
            "sort_order",
            "is_playback_start",
            "is_memory",
        ] {
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
        assert_eq!(
            remaining, 0,
            "track_cues should cascade-delete with the track"
        );
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
