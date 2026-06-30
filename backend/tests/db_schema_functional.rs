use std::path::Path;

use backend::commands::BackendCommands;
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn db_migration_is_idempotent_and_adds_expected_columns() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    let db_path = data_dir.join("backend.db");

    seed_legacy_backend_schema(&db_path);

    let backend = BackendCommands::new(&data_dir).expect("first init migrates");
    drop(backend);
    let backend_again = BackendCommands::new(&data_dir).expect("second init idempotent");
    drop(backend_again);

    let conn = Connection::open(&db_path).expect("open migrated db");
    assert!(has_column(&conn, "tracks", "track_number"));
    assert!(has_column(&conn, "tracks", "format_ext"));
    assert!(has_column(&conn, "tracks", "sample_rate_hz"));
    assert!(has_column(&conn, "tracks", "bit_depth"));
    assert!(has_column(&conn, "tracks", "bitrate_kbps"));
    assert!(has_column(&conn, "playlists", "last_exported_at"));
    assert!(has_column(&conn, "playlists", "last_exported_usb_root"));
    assert!(has_column(&conn, "playlists", "last_exported_track_count"));

    let track_number_count: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM pragma_table_info('tracks') WHERE lower(name) = 'track_number'",
            [],
            |row| row.get(0),
        )
        .expect("count track_number columns");
    assert_eq!(track_number_count, 1, "track_number should not duplicate");

    let schema_row_count: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM schema_version WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("schema_version row count");
    assert_eq!(
        schema_row_count, 1,
        "schema_version should have one id=1 row"
    );
    let schema_version: i64 = conn
        .query_row(
            "SELECT version FROM schema_version WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("schema_version value");
    assert_eq!(schema_version, 1);
}

fn seed_legacy_backend_schema(db_path: &Path) {
    let conn = Connection::open(db_path).expect("open legacy db");
    conn.execute_batch(
        r#"
        CREATE TABLE tracks (
          id TEXT PRIMARY KEY,
          title TEXT NOT NULL,
          artist TEXT NOT NULL,
          album TEXT,
          bpm REAL,
          tonality TEXT,
          file_path TEXT NOT NULL UNIQUE,
          file_size_bytes INTEGER,
          file_modified_at TEXT,
          artwork_path TEXT,
          waveform_peaks_path TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE playlists (
          id TEXT PRIMARY KEY,
          name TEXT NOT NULL,
          source TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE playlist_tracks (
          id TEXT PRIMARY KEY,
          playlist_id TEXT NOT NULL,
          track_id TEXT NOT NULL,
          position INTEGER NOT NULL,
          added_at TEXT NOT NULL
        );

        CREATE TABLE app_settings (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        "#,
    )
    .expect("seed legacy schema");
}

fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .expect("prepare table_info");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query table_info");
    for row in rows {
        if row.expect("column name").eq_ignore_ascii_case(column) {
            return true;
        }
    }
    false
}
