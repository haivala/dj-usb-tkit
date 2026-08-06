use std::path::Path;

use backend::commands::BackendCommands;
use backend::models::ValidateUsbRootRequest;
use rusqlite::{Connection, OptionalExtension};
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

    {
        let conn = Connection::open(&db_path).expect("open once-migrated db");
        assert_eq!(usb_table_count(&conn), 3, "usb tables should exist after first migration");
    }

    let backend_again = BackendCommands::new(&data_dir).expect("second init idempotent");
    drop(backend_again);

    let conn = Connection::open(&db_path).expect("open migrated db");
    assert_eq!(usb_table_count(&conn), 3, "usb tables should still exist after second migration");
    assert!(has_column(&conn, "tracks", "track_number"));
    assert!(has_column(&conn, "tracks", "format_ext"));
    assert!(has_column(&conn, "tracks", "sample_rate_hz"));
    assert!(has_column(&conn, "tracks", "bit_depth"));
    assert!(has_column(&conn, "tracks", "bitrate_kbps"));
    assert!(has_column(&conn, "tracks", "wav_extensible_kind"));
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

fn usb_table_count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(1) FROM sqlite_master WHERE type='table' AND name IN ('usb_devices','track_usb_links','usb_device_exports')",
        [],
        |row| row.get(0),
    )
    .expect("count usb tables")
}

#[test]
fn validate_usb_root_marks_device_mounted() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    let usb_dir = root.path().join("usb-a");
    std::fs::create_dir_all(&usb_dir).expect("create usb dir");

    let backend = BackendCommands::new(&data_dir).expect("init");
    let resp = backend.validate_usb_root(ValidateUsbRootRequest {
        path: usb_dir.to_string_lossy().to_string(),
    });
    assert!(resp.ok, "validate_usb_root should succeed: {:?}", resp.error);
    drop(backend);

    let conn = Connection::open(data_dir.join("backend.db")).expect("open db");
    let mounted: i64 = conn
        .query_row("SELECT mounted FROM usb_devices WHERE root_path_key = ?1", rusqlite::params![
            normalize_path_for_test(&usb_dir)
        ], |row| row.get(0))
        .expect("mounted row");
    assert_eq!(mounted, 1);
}

#[test]
fn validate_usb_root_unmounts_previously_mounted_device() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    let usb_a = root.path().join("usb-a");
    let usb_b = root.path().join("usb-b");
    std::fs::create_dir_all(&usb_a).expect("create usb-a");
    std::fs::create_dir_all(&usb_b).expect("create usb-b");

    let backend = BackendCommands::new(&data_dir).expect("init");
    let resp_a = backend.validate_usb_root(ValidateUsbRootRequest {
        path: usb_a.to_string_lossy().to_string(),
    });
    assert!(resp_a.ok);
    let resp_b = backend.validate_usb_root(ValidateUsbRootRequest {
        path: usb_b.to_string_lossy().to_string(),
    });
    assert!(resp_b.ok);
    drop(backend);

    let conn = Connection::open(data_dir.join("backend.db")).expect("open db");
    let mounted_a: i64 = conn
        .query_row(
            "SELECT mounted FROM usb_devices WHERE root_path_key = ?1",
            rusqlite::params![normalize_path_for_test(&usb_a)],
            |row| row.get(0),
        )
        .expect("mounted a");
    let mounted_b: i64 = conn
        .query_row(
            "SELECT mounted FROM usb_devices WHERE root_path_key = ?1",
            rusqlite::params![normalize_path_for_test(&usb_b)],
            |row| row.get(0),
        )
        .expect("mounted b");
    assert_eq!(mounted_a, 0, "validating B should unmount A");
    assert_eq!(mounted_b, 1);
}

fn normalize_path_for_test(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
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

fn seed_app_setting(db_path: &Path, key: &str, value: &str) {
    let conn = Connection::open(db_path).expect("open db for setting seed");
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .expect("seed app_setting");
}

fn app_setting_value(db_path: &Path, key: &str) -> Option<String> {
    let conn = Connection::open(db_path).expect("open db for setting read");
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        rusqlite::params![key],
        |row| row.get(0),
    )
    .optional()
    .expect("query app_setting")
}

#[test]
fn usb_devices_backfilled_from_legacy_root_settings() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    let db_path = data_dir.join("backend.db");
    seed_legacy_backend_schema(&db_path);
    seed_app_setting(&db_path, "ui_usb_root_v1", "/mnt/usbA");
    seed_app_setting(
        &db_path,
        "ui_usb_recent_roots_v1",
        r#"["/mnt/usbA","/mnt/usbB"]"#,
    );

    let backend = BackendCommands::new(&data_dir).expect("init migrates and backfills");
    drop(backend);

    let conn = Connection::open(&db_path).expect("open migrated db");
    let mut stmt = conn
        .prepare("SELECT root_path, mounted FROM usb_devices ORDER BY root_path")
        .expect("prepare");
    let rows: Vec<(String, i64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");
    assert_eq!(rows.len(), 2, "both distinct legacy roots should be backfilled");
    for (_, mounted) in &rows {
        assert_eq!(*mounted, 0, "backfilled devices must not be marked mounted");
    }
    let paths: Vec<&str> = rows.iter().map(|(p, _)| p.as_str()).collect();
    assert!(paths.contains(&"/mnt/usbA"));
    assert!(paths.contains(&"/mnt/usbB"));
}

#[test]
fn usb_devices_backfill_tolerates_malformed_recent_roots_json() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    let db_path = data_dir.join("backend.db");
    seed_legacy_backend_schema(&db_path);
    seed_app_setting(&db_path, "ui_usb_root_v1", "/mnt/usbA");
    seed_app_setting(&db_path, "ui_usb_recent_roots_v1", "not-json");

    let backend = BackendCommands::new(&data_dir).expect("startup should not fail on bad JSON");
    drop(backend);

    let conn = Connection::open(&db_path).expect("open migrated db");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM usb_devices WHERE root_path = '/mnt/usbA'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(count, 1, "ui_usb_root_v1 should still backfill");
}

#[test]
fn usb_recent_roots_setting_deleted_after_backfill() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    let db_path = data_dir.join("backend.db");
    seed_legacy_backend_schema(&db_path);
    seed_app_setting(&db_path, "ui_usb_root_v1", "/mnt/usbA");
    seed_app_setting(&db_path, "ui_usb_recent_roots_v1", r#"["/mnt/usbA"]"#);

    let backend = BackendCommands::new(&data_dir).expect("init");
    drop(backend);

    assert_eq!(app_setting_value(&db_path, "ui_usb_recent_roots_v1"), None);
    assert_eq!(
        app_setting_value(&db_path, "ui_usb_root_v1"),
        Some("/mnt/usbA".to_string())
    );
}

#[test]
fn startup_resets_previously_mounted_usb_devices() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    std::fs::create_dir_all(&data_dir).expect("create data dir");

    let backend = BackendCommands::new(&data_dir).expect("first launch");
    drop(backend);

    let db_path = data_dir.join("backend.db");
    {
        let conn = Connection::open(&db_path).expect("open db");
        conn.execute(
            "INSERT INTO usb_devices (id, root_path, root_path_key, mounted, first_seen_at, last_seen_at, created_at, updated_at)
             VALUES ('dev-1', '/mnt/usbA', '/mnt/usba', 1, datetime('now'), datetime('now'), datetime('now'), datetime('now'))",
            [],
        )
        .expect("seed mounted device");
    }

    let backend_again = BackendCommands::new(&data_dir).expect("second launch");
    drop(backend_again);

    let conn = Connection::open(&db_path).expect("open db again");
    let mounted: i64 = conn
        .query_row(
            "SELECT mounted FROM usb_devices WHERE id = 'dev-1'",
            [],
            |row| row.get(0),
        )
        .expect("mounted");
    assert_eq!(mounted, 0, "mount state from a prior session must not survive restart");
}
