//! Direct-service-level tests for the USB-vs-local track dedup/merge and
//! playback-resolution fixes (see TODO.md "Fix USB-vs-local track/playback
//! source-of-truth bugs"). These seed `tracks`/`usb_devices` rows directly
//! via raw SQL rather than going through a full USB export/browse roundtrip
//! (covered separately in `lib_integration.rs`), so they can exercise
//! specific confidence-gate and merge scenarios cheaply.

use backend::models::ResolvePlaybackSourceRequest;
use backend::service::BackendService;
use rusqlite::params;
use tempfile::tempdir;
use uuid::Uuid;

fn new_service() -> (tempfile::TempDir, BackendService) {
    let dir = tempdir().expect("tempdir");
    let svc = BackendService::new(dir.path()).expect("service init");
    (dir, svc)
}

fn insert_track(
    conn: &rusqlite::Connection,
    id: &str,
    file_path: &str,
    fingerprint: &str,
    duration_ms: Option<i64>,
    file_size_bytes: Option<i64>,
) {
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, file_size_bytes, match_fingerprint, created_at, updated_at)
         VALUES (?1, 'T', 'A', ?2, ?3, ?4, ?5, datetime('now'), datetime('now'))",
        params![id, file_path, duration_ms, file_size_bytes, fingerprint],
    )
    .expect("insert track");
}

fn insert_usb_device(conn: &rusqlite::Connection, id: &str, root_path: &str) {
    conn.execute(
        "INSERT INTO usb_devices (id, root_path, root_path_key, mounted, first_seen_at, last_seen_at, created_at, updated_at)
         VALUES (?1, ?2, ?2, 0, datetime('now'), datetime('now'), datetime('now'), datetime('now'))",
        params![id, root_path],
    )
    .expect("insert usb device");
}

fn insert_track_usb_link(
    conn: &rusqlite::Connection,
    track_id: &str,
    device_id: &str,
    usb_file_path: &str,
) {
    conn.execute(
        "INSERT INTO track_usb_links (id, track_id, usb_device_id, usb_file_path, first_seen_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))",
        params![Uuid::now_v7().to_string(), track_id, device_id, usb_file_path],
    )
    .expect("insert track_usb_link");
}

fn insert_playlist_with_track(conn: &rusqlite::Connection, playlist_id: &str, track_id: &str) {
    conn.execute(
        "INSERT INTO playlists (id, name, source, created_at, updated_at) VALUES (?1, 'P', 'app', datetime('now'), datetime('now'))",
        params![playlist_id],
    )
    .expect("insert playlist");
    conn.execute(
        "INSERT INTO playlist_tracks (id, playlist_id, track_id, position, added_at) VALUES (?1, ?2, ?3, 0, datetime('now'))",
        params![Uuid::now_v7().to_string(), playlist_id, track_id],
    )
    .expect("insert playlist_track");
}

#[test]
fn merge_orphaned_usb_placeholder_tracks_reassigns_links_and_deletes_placeholder() {
    let (dir, svc) = new_service();
    let conn = svc.db.connect().expect("connect");
    insert_usb_device(&conn, "dev-1", "/mnt/usb1");
    insert_track(
        &conn,
        "local-1",
        "/music/a.mp3",
        "fp1",
        Some(200_000),
        Some(1_000),
    );
    insert_track(
        &conn,
        "placeholder-1",
        "/mnt/usb1/Contents/a.mp3",
        "fp1",
        Some(200_500),
        Some(1_000),
    );
    insert_track_usb_link(&conn, "placeholder-1", "dev-1", "/mnt/usb1/Contents/a.mp3");
    insert_playlist_with_track(&conn, "pl-1", "placeholder-1");
    drop(conn);

    let report = svc
        .merge_orphaned_usb_placeholder_tracks()
        .expect("merge should succeed");
    assert_eq!(report.merged, 1);

    let conn = svc.db.connect().expect("connect");
    let placeholder_gone: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM tracks WHERE id = 'placeholder-1'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(placeholder_gone, 0);

    let link_track_id: String = conn
        .query_row(
            "SELECT track_id FROM track_usb_links WHERE usb_device_id = 'dev-1'",
            [],
            |row| row.get(0),
        )
        .expect("link track id");
    assert_eq!(link_track_id, "local-1");

    let playlist_track_id: String = conn
        .query_row(
            "SELECT track_id FROM playlist_tracks WHERE playlist_id = 'pl-1'",
            [],
            |row| row.get(0),
        )
        .expect("playlist track id");
    assert_eq!(playlist_track_id, "local-1");
    let _ = dir;
}

#[test]
fn merge_orphaned_usb_placeholder_tracks_skips_diverging_duration_or_size() {
    let (_dir, svc) = new_service();
    let conn = svc.db.connect().expect("connect");
    insert_usb_device(&conn, "dev-1", "/mnt/usb1");

    // Diverging duration.
    insert_track(
        &conn,
        "local-a",
        "/music/a.mp3",
        "fp-a",
        Some(200_000),
        Some(1_000),
    );
    insert_track(
        &conn,
        "placeholder-a",
        "/mnt/usb1/Contents/a.mp3",
        "fp-a",
        Some(210_000),
        Some(1_000),
    );

    // Matching duration, diverging size (Clean/Explicit collision case).
    insert_track(
        &conn,
        "local-b",
        "/music/b.mp3",
        "fp-b",
        Some(200_000),
        Some(1_000),
    );
    insert_track(
        &conn,
        "placeholder-b",
        "/mnt/usb1/Contents/b.mp3",
        "fp-b",
        Some(200_000),
        Some(999),
    );

    // Two local candidates tie -- ambiguous, must not auto-merge.
    insert_track(
        &conn,
        "local-c1",
        "/music/c1.mp3",
        "fp-c",
        Some(200_000),
        Some(1_000),
    );
    insert_track(
        &conn,
        "local-c2",
        "/music/c2.mp3",
        "fp-c",
        Some(200_000),
        Some(1_000),
    );
    insert_track(
        &conn,
        "placeholder-c",
        "/mnt/usb1/Contents/c.mp3",
        "fp-c",
        Some(200_000),
        Some(1_000),
    );
    drop(conn);

    let report = svc.merge_orphaned_usb_placeholder_tracks().expect("merge");
    assert_eq!(report.merged, 0, "no group should merge");

    let conn = svc.db.connect().expect("connect");
    let remaining: i64 = conn
        .query_row("SELECT COUNT(1) FROM tracks", [], |row| row.get(0))
        .expect("count");
    assert_eq!(remaining, 7, "no rows should have been deleted");
}

#[test]
fn merge_orphaned_usb_placeholder_tracks_skips_placeholder_with_missing_file_size() {
    let (_dir, svc) = new_service();
    let conn = svc.db.connect().expect("connect");
    insert_usb_device(&conn, "dev-1", "/mnt/usb1");
    insert_track(
        &conn,
        "local-1",
        "/music/a.mp3",
        "fp1",
        Some(200_000),
        Some(1_000),
    );
    insert_track(
        &conn,
        "placeholder-1",
        "/mnt/usb1/Contents/a.mp3",
        "fp1",
        Some(200_000),
        None,
    );
    drop(conn);

    let report = svc.merge_orphaned_usb_placeholder_tracks().expect("merge");
    assert_eq!(
        report.merged, 0,
        "missing file_size_bytes on the placeholder must block the batch merge (unlike the live per-track path)"
    );
}

#[test]
fn merge_orphaned_usb_placeholder_tracks_is_idempotent() {
    let (_dir, svc) = new_service();
    let conn = svc.db.connect().expect("connect");
    insert_usb_device(&conn, "dev-1", "/mnt/usb1");
    insert_track(
        &conn,
        "local-1",
        "/music/a.mp3",
        "fp1",
        Some(200_000),
        Some(1_000),
    );
    insert_track(
        &conn,
        "placeholder-1",
        "/mnt/usb1/Contents/a.mp3",
        "fp1",
        Some(200_000),
        Some(1_000),
    );
    drop(conn);

    let first = svc
        .merge_orphaned_usb_placeholder_tracks()
        .expect("first merge");
    assert_eq!(first.merged, 1);
    let second = svc
        .merge_orphaned_usb_placeholder_tracks()
        .expect("second merge");
    assert_eq!(second.merged, 0, "re-running the merge should be a no-op");
}

fn resolve_req(title: &str, artist: &str, track_id: Option<&str>) -> ResolvePlaybackSourceRequest {
    ResolvePlaybackSourceRequest {
        title: title.to_string(),
        artist: artist.to_string(),
        album: None,
        bpm: None,
        file_path: None,
        file_size_bytes: None,
        track_id: track_id.map(str::to_string),
    }
}

#[test]
fn resolve_playback_source_excludes_usb_placeholder_rows_even_when_no_local_copy_exists() {
    let (_dir, svc) = new_service();
    let conn = svc.db.connect().expect("connect");
    insert_usb_device(&conn, "dev-1", "/mnt/usb1");
    insert_track(
        &conn,
        "placeholder-1",
        "/mnt/usb1/Contents/a.mp3",
        "",
        Some(200_000),
        Some(1_000),
    );
    conn.execute(
        "UPDATE tracks SET title = 'Only On Usb', artist = 'Artist', match_fingerprint = ?1 WHERE id = 'placeholder-1'",
        params![backend::service::build_track_match_fingerprint("Only On Usb", "Artist", None)],
    )
    .expect("set fingerprint");
    drop(conn);

    let mut req = resolve_req("Only On Usb", "Artist", None);
    req.file_path = Some("/mnt/usb1/Contents/a.mp3".to_string());
    let resolved = svc.resolve_playback_source(req).expect("resolve");
    assert_eq!(resolved.resolved_path, None);
    assert_eq!(resolved.matched_by, "none");
}

#[test]
fn resolve_playback_source_treats_path_as_local_once_it_is_a_configured_source_root() {
    let (_dir, svc) = new_service();
    let conn = svc.db.connect().expect("connect");
    insert_usb_device(&conn, "dev-1", "/mnt/data");
    insert_track(
        &conn,
        "track-1",
        "/mnt/data/a.mp3",
        "",
        Some(200_000),
        Some(1_000),
    );
    let fp = backend::service::build_track_match_fingerprint("Reused Path", "Artist", None);
    conn.execute(
        "UPDATE tracks SET title = 'Reused Path', artist = 'Artist', match_fingerprint = ?1 WHERE id = 'track-1'",
        params![fp],
    )
    .expect("set fingerprint");
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES ('ui_source_roots_v1', '[\"/mnt/data\"]', datetime('now'))",
        [],
    )
    .expect("configure source root");
    drop(conn);

    let resolved = svc
        .resolve_playback_source(resolve_req("Reused Path", "Artist", None))
        .expect("resolve");
    assert_eq!(resolved.resolved_path.as_deref(), Some("/mnt/data/a.mp3"));
    assert_eq!(resolved.matched_by, "hash");
}

#[test]
fn resolve_playback_source_excludes_candidate_with_mismatched_file_size() {
    let (_dir, svc) = new_service();
    let conn = svc.db.connect().expect("connect");
    let fp = backend::service::build_track_match_fingerprint("Sized Track", "Artist", None);
    insert_track(
        &conn,
        "wrong-size",
        "/music/wrong.mp3",
        &fp,
        Some(200_000),
        Some(999),
    );
    insert_track(
        &conn,
        "right-size",
        "/music/right.mp3",
        &fp,
        Some(200_000),
        Some(1_000),
    );
    conn.execute(
        "UPDATE tracks SET title = 'Sized Track', artist = 'Artist' WHERE match_fingerprint = ?1",
        params![fp],
    )
    .expect("set titles");
    drop(conn);

    let mut req = resolve_req("Sized Track", "Artist", None);
    req.file_size_bytes = Some(1_000);
    let resolved = svc.resolve_playback_source(req).expect("resolve");
    assert_eq!(resolved.track_id.as_deref(), Some("right-size"));
}

#[test]
fn resolve_playback_source_ignores_file_size_veto_when_either_side_unknown() {
    let (_dir, svc) = new_service();
    let conn = svc.db.connect().expect("connect");
    let fp = backend::service::build_track_match_fingerprint("Unknown Size", "Artist", None);
    insert_track(
        &conn,
        "only-candidate",
        "/music/only.mp3",
        &fp,
        Some(200_000),
        None,
    );
    conn.execute(
        "UPDATE tracks SET title = 'Unknown Size', artist = 'Artist' WHERE match_fingerprint = ?1",
        params![fp],
    )
    .expect("set titles");
    drop(conn);

    let mut req = resolve_req("Unknown Size", "Artist", None);
    req.file_size_bytes = Some(12345);
    let resolved = svc.resolve_playback_source(req).expect("resolve");
    assert_eq!(
        resolved.track_id.as_deref(),
        Some("only-candidate"),
        "a missing file size on either side must not block an otherwise-fine match"
    );
}

#[test]
fn resolve_playback_source_fast_path_returns_own_path_for_non_usb_track_id() {
    let (_dir, svc) = new_service();
    let conn = svc.db.connect().expect("connect");
    insert_track(&conn, "local-1", "/music/a.mp3", "", None, None);
    drop(conn);

    // Deliberately mismatched title/artist -- if the fast path weren't
    // taken, the fingerprint/title search below would never find this row.
    let resolved = svc
        .resolve_playback_source(resolve_req("Totally Different", "Nobody", Some("local-1")))
        .expect("resolve");
    assert_eq!(resolved.matched_by, "self");
    assert_eq!(resolved.resolved_path.as_deref(), Some("/music/a.mp3"));
}

#[test]
fn resolve_playback_source_falls_through_when_track_id_is_usb_rooted() {
    let (_dir, svc) = new_service();
    let conn = svc.db.connect().expect("connect");
    insert_usb_device(&conn, "dev-1", "/mnt/usb1");
    let fp =
        backend::service::build_track_match_fingerprint("Stale Playlist Entry", "Artist", None);
    insert_track(&conn, "local-1", "/music/a.mp3", &fp, None, None);
    insert_track(
        &conn,
        "placeholder-1",
        "/mnt/usb1/Contents/a.mp3",
        &fp,
        None,
        None,
    );
    conn.execute(
        "UPDATE tracks SET title = 'Stale Playlist Entry', artist = 'Artist' WHERE match_fingerprint = ?1",
        params![fp],
    )
    .expect("set titles");
    drop(conn);

    let resolved = svc
        .resolve_playback_source(resolve_req(
            "Stale Playlist Entry",
            "Artist",
            Some("placeholder-1"),
        ))
        .expect("resolve");
    assert_ne!(
        resolved.resolved_path.as_deref(),
        Some("/mnt/usb1/Contents/a.mp3"),
        "a stale placeholder track_id must not be trusted as-is"
    );
    assert_eq!(resolved.resolved_path.as_deref(), Some("/music/a.mp3"));
}

#[test]
fn resolve_playback_source_fast_path_handles_missing_track_id() {
    let (_dir, svc) = new_service();
    let conn = svc.db.connect().expect("connect");
    let fp = backend::service::build_track_match_fingerprint("Findable Track", "Artist", None);
    insert_track(&conn, "local-1", "/music/a.mp3", &fp, None, None);
    conn.execute(
        "UPDATE tracks SET title = 'Findable Track', artist = 'Artist' WHERE match_fingerprint = ?1",
        params![fp],
    )
    .expect("set titles");
    drop(conn);

    let resolved = svc
        .resolve_playback_source(resolve_req(
            "Findable Track",
            "Artist",
            Some("does-not-exist"),
        ))
        .expect("resolve");
    assert_eq!(resolved.matched_by, "hash");
    assert_eq!(resolved.resolved_path.as_deref(), Some("/music/a.mp3"));
}

#[test]
fn prune_usb_device_soft_deletes_without_breaking_existing_links() {
    let (_dir, svc) = new_service();
    let conn = svc.db.connect().expect("connect");
    insert_usb_device(&conn, "dev-1", "/mnt/usb1");
    insert_track(&conn, "local-1", "/music/a.mp3", "fp1", None, None);
    insert_track_usb_link(&conn, "local-1", "dev-1", "/mnt/usb1/Contents/a.mp3");
    conn.execute(
        "INSERT INTO usb_device_exports (id, usb_device_id, playlist_id, playlist_name, exported_at, track_count, track_fingerprints, created_at)
         VALUES ('exp-1', 'dev-1', NULL, 'P', datetime('now'), 1, '[]', datetime('now'))",
        [],
    )
    .expect("seed export history");
    drop(conn);

    let pruned = svc
        .prune_usb_device(backend::models::PruneUsbDeviceRequest {
            id: "dev-1".to_string(),
        })
        .expect("prune");
    assert!(pruned.pruned);

    let conn = svc.db.connect().expect("connect");
    let deleted_at: Option<String> = conn
        .query_row(
            "SELECT deleted_at FROM usb_devices WHERE id = 'dev-1'",
            [],
            |row| row.get(0),
        )
        .expect("deleted_at");
    assert!(deleted_at.is_some());

    let listed = svc.list_usb_devices().expect("list");
    assert!(
        listed.items.is_empty(),
        "pruned device must not appear in list_usb_devices"
    );

    let link_count: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM track_usb_links WHERE usb_device_id = 'dev-1'",
            [],
            |row| row.get(0),
        )
        .expect("link count");
    assert_eq!(
        link_count, 1,
        "pruning must not cascade-delete track_usb_links"
    );
    let export_count: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM usb_device_exports WHERE usb_device_id = 'dev-1'",
            [],
            |row| row.get(0),
        )
        .expect("export count");
    assert_eq!(
        export_count, 1,
        "pruning must not cascade-delete usb_device_exports"
    );

    // `all_usb_device_root_paths`/`untainted_usb_root_paths` (exercised
    // directly in service::tests and diag_tests) deliberately never filter
    // on deleted_at -- confirm the underlying row (which those functions
    // read from) is still present, just soft-deleted, not gone.
    let root_still_present: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM usb_devices WHERE root_path = '/mnt/usb1'",
            [],
            |row| row.get(0),
        )
        .expect("root still present");
    assert_eq!(root_still_present, 1);

    // Re-picking/validating the same root should un-prune it (exercised at
    // the service level in Step 2/4's own tests via upsert_usb_device;
    // here we just confirm list_usb_devices reflects deleted_at again once
    // it's cleared).
    conn.execute(
        "UPDATE usb_devices SET deleted_at = NULL WHERE id = 'dev-1'",
        [],
    )
    .expect("clear deleted_at");
    let listed_again = svc.list_usb_devices().expect("list again");
    assert_eq!(listed_again.items.len(), 1);
}
