use super::*;
use crate::models::ScanRekordboxDbRequest;

// --- build_track_match_fingerprint ---

#[test]
fn fingerprint_deterministic() {
    let fp1 = build_track_match_fingerprint("Title", "Artist", Some("Album"));
    let fp2 = build_track_match_fingerprint("Title", "Artist", Some("Album"));
    assert_eq!(fp1, fp2);
}

#[test]
fn fingerprint_case_insensitive() {
    let lower = build_track_match_fingerprint("midnight", "dj shadow", Some("entroducing"));
    let upper = build_track_match_fingerprint("MIDNIGHT", "DJ SHADOW", Some("ENTRODUCING"));
    let mixed = build_track_match_fingerprint("Midnight", "DJ Shadow", Some("Entroducing"));
    assert_eq!(lower, upper);
    assert_eq!(lower, mixed);
}

#[test]
fn fingerprint_whitespace_normalized() {
    let single = build_track_match_fingerprint("My Track", "An Artist", None);
    let multi = build_track_match_fingerprint("My   Track", "An  Artist", None);
    let tabs = build_track_match_fingerprint("My\tTrack", "An\tArtist", None);
    assert_eq!(single, multi);
    assert_eq!(single, tabs);
}

#[test]
fn fingerprint_leading_trailing_whitespace() {
    let clean = build_track_match_fingerprint("Title", "Artist", None);
    let padded = build_track_match_fingerprint("  Title  ", "  Artist  ", None);
    assert_eq!(clean, padded);
}

#[test]
fn fingerprint_different_tracks_differ() {
    let a = build_track_match_fingerprint("Track A", "Artist X", None);
    let b = build_track_match_fingerprint("Track B", "Artist X", None);
    assert_ne!(a, b);
}

#[test]
fn fingerprint_album_matters() {
    let with_album = build_track_match_fingerprint("Title", "Artist", Some("Album"));
    let without_album = build_track_match_fingerprint("Title", "Artist", None);
    let diff_album = build_track_match_fingerprint("Title", "Artist", Some("Other"));
    assert_ne!(with_album, without_album);
    assert_ne!(with_album, diff_album);
}

#[test]
fn fingerprint_empty_fields() {
    // Should not panic on empty inputs
    let fp = build_track_match_fingerprint("", "", None);
    assert!(!fp.is_empty(), "fingerprint should still produce output");
    assert_eq!(fp.len(), 16, "fingerprint should be 16 hex chars");
}

#[test]
fn fingerprint_is_hex_string() {
    let fp = build_track_match_fingerprint("Test", "Artist", Some("Album"));
    assert_eq!(fp.len(), 16);
    assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn fingerprint_unicode_preserved() {
    // Unicode chars are lowercased but not stripped
    let a = build_track_match_fingerprint("Café", "Müsik", None);
    let b = build_track_match_fingerprint("café", "müsik", None);
    assert_eq!(a, b);
}

#[test]
fn fingerprint_very_long_strings() {
    let long = "a".repeat(10_000);
    let fp = build_track_match_fingerprint(&long, &long, Some(&long));
    assert_eq!(fp.len(), 16);
    assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
}

// --- normalize_hash_part ---

#[test]
fn normalize_hash_part_collapses_whitespace() {
    assert_eq!(normalize_hash_part("a  b   c"), "a b c");
}

#[test]
fn normalize_hash_part_lowercases() {
    assert_eq!(normalize_hash_part("HELLO"), "hello");
}

#[test]
fn normalize_hash_part_trims() {
    assert_eq!(normalize_hash_part("  hello  "), "hello");
}

#[test]
fn normalize_hash_part_tabs_to_spaces() {
    assert_eq!(normalize_hash_part("a\tb"), "a b");
}

#[test]
fn normalize_hash_part_empty() {
    assert_eq!(normalize_hash_part(""), "");
}

#[test]
fn rekordbox_db_resource_candidates_prefer_windows_share_for_pioneer_paths() {
    let master_path = Path::new("/tmp/AppData/Roaming/Pioneer/rekordbox/master.db");
    let candidates = rekordbox_db_resource_candidates(
        master_path,
        "/PIONEER/USBANLZ/b13/f6121-04a7-4e91-9d08-0b039109a193/ANLZ0000.DAT",
    );

    assert_eq!(
        candidates[0],
        PathBuf::from("/tmp/AppData/Roaming/Pioneer/rekordbox")
            .join("share")
            .join("PIONEER/USBANLZ/b13/f6121-04a7-4e91-9d08-0b039109a193/ANLZ0000.DAT")
    );
    assert_eq!(
        candidates[1],
        PathBuf::from("/tmp/AppData/Roaming/Pioneer/rekordbox")
            .join("PIONEER/USBANLZ/b13/f6121-04a7-4e91-9d08-0b039109a193/ANLZ0000.DAT")
    );
}

#[test]
fn rekordbox_db_resource_candidates_ignore_blank_values() {
    let candidates = rekordbox_db_resource_candidates(Path::new("/tmp/rekordbox/master.db"), " ");
    assert!(candidates.is_empty());
}

#[test]
fn rekordbox_db_analysis_file_candidates_prefer_ext_for_pwv4() {
    let master_path = Path::new("/tmp/AppData/Roaming/Pioneer/rekordbox/master.db");
    let candidates = rekordbox_db_analysis_file_candidates(
        master_path,
        "/PIONEER/USBANLZ/b13/f6121-04a7-4e91-9d08-0b039109a193/ANLZ0000.DAT",
    );

    assert_eq!(
        candidates[0],
        PathBuf::from("/tmp/AppData/Roaming/Pioneer/rekordbox")
            .join("share")
            .join("PIONEER/USBANLZ/b13/f6121-04a7-4e91-9d08-0b039109a193/ANLZ0000.EXT")
    );
    assert_eq!(
        candidates[1],
        PathBuf::from("/tmp/AppData/Roaming/Pioneer/rekordbox")
            .join("share")
            .join("PIONEER/USBANLZ/b13/f6121-04a7-4e91-9d08-0b039109a193/ANLZ0000.2EX")
    );
}

fn create_rekordbox_db_fixture(
    root: &Path,
    rows: &[(&str, &str, Option<&str>, Option<&str>)],
) -> PathBuf {
    let master_path = root.join("rekordbox").join("master.db");
    std::fs::create_dir_all(master_path.parent().unwrap()).expect("create master db dir");
    let conn = rusqlite::Connection::open(&master_path).expect("create master db");
    conn.execute_batch(&format!(
        "PRAGMA key='{}';",
        usb_vendor_compat::DEFAULT_MASTER_DB_KEY
    ))
    .expect("set master db key");
    conn.execute_batch(
        r#"
            CREATE TABLE djmdArtist (ID INTEGER PRIMARY KEY, Name TEXT);
            CREATE TABLE djmdAlbum (ID INTEGER PRIMARY KEY, Name TEXT);
            CREATE TABLE djmdKey (ID INTEGER PRIMARY KEY, ScaleName TEXT);
            CREATE TABLE djmdContent (
              ID INTEGER PRIMARY KEY,
              FolderPath TEXT,
              Title TEXT,
              SrcArtistName TEXT,
              ArtistID INTEGER,
              AlbumID INTEGER,
              BPM INTEGER,
              KeyID INTEGER,
              Length INTEGER,
              AnalysisDataPath TEXT,
              ImagePath TEXT,
              rb_local_deleted INTEGER DEFAULT 0
            );
            INSERT INTO djmdArtist (ID, Name) VALUES (1, 'Fixture Artist');
            INSERT INTO djmdAlbum (ID, Name) VALUES (1, 'Fixture Album');
            INSERT INTO djmdKey (ID, ScaleName) VALUES (1, '8A');
            "#,
    )
    .expect("create master db schema");

    for (idx, (file_path, title, anlz_path, image_path)) in rows.iter().enumerate() {
        conn.execute(
            r#"
                INSERT INTO djmdContent
                  (ID, FolderPath, Title, SrcArtistName, ArtistID, AlbumID, BPM, KeyID, Length,
                   AnalysisDataPath, ImagePath, rb_local_deleted)
                VALUES (?1, ?2, ?3, 'Fallback Artist', 1, 1, 12600, 1, 245, ?4, ?5, 0)
                "#,
            params![idx as i64 + 1, file_path, title, anlz_path, image_path],
        )
        .expect("insert master db row");
    }

    conn.execute(
        "INSERT INTO djmdContent (ID, FolderPath, Title, rb_local_deleted)
         VALUES (999, '/deleted/track.mp3', 'Deleted', 1)",
        [],
    )
    .expect("insert deleted row");
    drop(conn);
    master_path
}

#[test]
fn scan_rekordbox_db_maps_wine_drive_paths_to_linux_paths() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let prefix = tmp.path().join("prefix");
    std::fs::create_dir_all(prefix.join("dosdevices")).expect("dosdevices");
    let music = prefix.join("drive_c/users/dj/Music");
    std::fs::create_dir_all(&music).expect("music dir");
    std::os::unix::fs::symlink("../drive_c", prefix.join("dosdevices/c:")).expect("c:");
    std::fs::write(music.join("a.mp3"), b"audio").expect("write track");
    let (_service_dir, service) = test_service();

    let master_path = create_rekordbox_db_fixture(
        &prefix.join("drive_c/users/dj/AppData/Roaming/Pioneer"),
        &[("C:/users/dj/Music/a.mp3", "Under Wine", None, None)],
    );
    let result = service
        .scan_rekordbox_db(ScanRekordboxDbRequest {
            path: Some(master_path.to_string_lossy().to_string()),
        })
        .expect("scan rekordbox db");
    assert_eq!(result.indexed, 1);
    assert!(result.not_found.is_empty(), "{:?}", result.not_found);

    let conn = service.db.connect().expect("service db");
    let file_path: String = conn
        .query_row(
            "SELECT file_path FROM tracks WHERE title = 'Under Wine'",
            [],
            |row| row.get(0),
        )
        .expect("imported track");
    let expected = music.canonicalize().expect("music").join("a.mp3");
    assert_eq!(PathBuf::from(file_path), expected);
}

#[test]
fn scan_rekordbox_db_imports_updates_keeps_missing_and_reports_resource_warnings() {
    let master_root = tempfile::tempdir().expect("master root");
    let media_root = tempfile::tempdir().expect("media root");
    let (_service_dir, service) = test_service();

    let existing_path = media_root.path().join("existing.mp3");
    let new_nulls_path = media_root.path().join("new-nulls.aif");
    let new_missing_assets_path = media_root.path().join("new-missing.flac");
    std::fs::write(&existing_path, b"existing").expect("write existing file");
    std::fs::write(&new_nulls_path, b"new").expect("write new file");
    std::fs::write(&new_missing_assets_path, b"missing").expect("write missing-assets file");

    let share_dir = master_root.path().join("rekordbox").join("share");
    let anlz_path = share_dir.join("PIONEER/USBANLZ/a/ANLZ0000.EXT");
    let image_path = share_dir.join("PIONEER/Artwork/a/COVER.JPG");
    std::fs::create_dir_all(anlz_path.parent().unwrap()).expect("create anlz dir");
    std::fs::create_dir_all(image_path.parent().unwrap()).expect("create image dir");
    std::fs::write(&anlz_path, b"anlz").expect("write anlz");
    std::fs::write(&image_path, b"jpg").expect("write artwork");

    let removed_path = media_root.path().join("removed.mp3");
    let master_path = create_rekordbox_db_fixture(
        master_root.path(),
        &[
            (
                existing_path.to_str().unwrap(),
                "Updated Existing",
                Some("/PIONEER/USBANLZ/a/ANLZ0000.DAT"),
                Some("/PIONEER/Artwork/a/COVER.JPG"),
            ),
            (
                new_nulls_path.to_str().unwrap(),
                "Inserted Nulls",
                None,
                None,
            ),
            (
                new_missing_assets_path.to_str().unwrap(),
                "Inserted Missing Assets",
                Some("/PIONEER/USBANLZ/missing/ANLZ0000.DAT"),
                Some("/PIONEER/Artwork/missing/COVER.JPG"),
            ),
            (removed_path.to_str().unwrap(), "Removed", None, None),
        ],
    );

    let conn = service.db.connect().expect("service db");
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, match_fingerprint, created_at, updated_at)
         VALUES ('existing-track', 'Old', 'Old Artist', ?1, 'old-fp', 'old', 'old')",
        params![existing_path.to_str().unwrap()],
    )
    .expect("seed existing track");
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, match_fingerprint, created_at, updated_at)
         VALUES ('removed-track', 'Gone', 'Old Artist', ?1, 'old-fp', 'old', 'old')",
        params![removed_path.to_str().unwrap()],
    )
    .expect("seed removed track");
    drop(conn);

    let imported = || {
        service
            .detect_external_rekordbox_db()
            .expect("detect")
            .imported
    };
    assert!(!imported(), "nothing imported yet");
    let result = service
        .scan_rekordbox_db(ScanRekordboxDbRequest {
            path: Some(master_path.to_string_lossy().to_string()),
        })
        .expect("scan master db");
    assert!(imported());

    assert_eq!(result.indexed, 2);
    assert_eq!(result.updated, 1);
    // A missing file is reported, never deleted (its drive may just be
    // unplugged).
    assert_eq!(result.removed, 0);
    assert_eq!(
        result.not_found,
        vec![removed_path.to_string_lossy().to_string()]
    );
    let warning_codes = result
        .warnings
        .iter()
        .map(|warning| warning.code.as_str())
        .collect::<Vec<_>>();
    for code in [
        "scan.rekordbox-db.anlz-sample",
        "scan.rekordbox-db.image-sample",
        "scan.rekordbox-db.anlz-null",
        "scan.rekordbox-db.anlz-miss-summary",
        "scan.rekordbox-db.anlz-ok",
        "scan.rekordbox-db.artwork-null",
        "scan.rekordbox-db.artwork-miss-summary",
        "scan.rekordbox-db.artwork-ok",
    ] {
        assert!(
            warning_codes.contains(&code),
            "expected warning code {code}, got {warning_codes:?}"
        );
    }

    let conn = service.db.connect().expect("service db");
    let updated: (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT title, artist, tonality, waveform_peaks_path, artwork_path
               FROM tracks WHERE id = 'existing-track'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("updated row");
    assert_eq!(updated.0, "Updated Existing");
    assert_eq!(updated.1, "Fixture Artist");
    assert_eq!(updated.2.as_deref(), Some("8A"));
    assert_eq!(updated.3.as_deref(), Some(anlz_path.to_str().unwrap()));
    assert!(
        updated
            .4
            .as_deref()
            .is_some_and(|path| path.ends_with(".JPG"))
    );

    let missing_kept: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tracks WHERE id = 'removed-track'",
            [],
            |row| row.get(0),
        )
        .expect("missing row count");
    assert_eq!(missing_kept, 1);

    let imported_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tracks WHERE rekordbox_db_source = 1",
            [],
            |row| row.get(0),
        )
        .expect("imported count");
    assert_eq!(imported_count, 3);
}

#[test]
fn decode_track_page_cursor_rejects_invalid_token() {
    let signature = build_track_cursor_signature(&[TRACK_CURSOR_VERSION, "list_tracks"]);
    let result = decode_track_page_cursor(Some("not-base64"), &signature);
    assert!(result.is_err());
}

#[test]
fn decode_track_page_cursor_rejects_signature_mismatch() {
    let sig_a = build_track_cursor_signature(&[TRACK_CURSOR_VERSION, "list_tracks"]);
    let sig_b = build_track_cursor_signature(&[TRACK_CURSOR_VERSION, "search_tracks", "abc"]);
    let token = encode_track_page_cursor(&sig_a, "/music/a.mp3", "track-a").expect("token");
    let result = decode_track_page_cursor(Some(&token), &sig_b);
    assert!(result.is_err());
}

#[test]
fn decode_track_page_cursor_accepts_matching_signature() {
    let sig = build_track_cursor_signature(&[TRACK_CURSOR_VERSION, "search_tracks", "q"]);
    let token = encode_track_page_cursor(&sig, "/music/a.mp3", "track-a").expect("token");
    let decoded = decode_track_page_cursor(Some(&token), &sig)
        .expect("decode ok")
        .expect("cursor present");
    assert_eq!(decoded.file_path, "/music/a.mp3");
    assert_eq!(decoded.id, "track-a");
    assert_eq!(decoded.signature, sig);
}

// --- find_confident_fingerprint_match / untainted_usb_root_paths ---

fn test_db() -> (tempfile::TempDir, crate::db::Db) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = crate::db::Db::new(dir.path()).expect("db init");
    (dir, db)
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

#[test]
fn find_confident_fingerprint_match_requires_duration_and_size_agreement() {
    let (_dir, db) = test_db();
    let conn = db.connect().expect("connect");
    insert_track(
        &conn,
        "local-1",
        "/music/a.mp3",
        "fp1",
        Some(200_000),
        Some(1_000),
    );

    // Both agree -> match.
    let matched = find_confident_fingerprint_match(
        &conn,
        "fp1",
        "/usb/a.mp3",
        Some(201_000),
        Some(1_000),
        &[],
    )
    .expect("query");
    assert_eq!(matched, Some("local-1".to_string()));

    // Duration diverges beyond tolerance -> no match.
    let no_match = find_confident_fingerprint_match(
        &conn,
        "fp1",
        "/usb/a.mp3",
        Some(210_000),
        Some(1_000),
        &[],
    )
    .expect("query");
    assert_eq!(no_match, None);

    // Duration matches but size diverges -> no match (Clean/Explicit collision case).
    let no_match_size =
        find_confident_fingerprint_match(&conn, "fp1", "/usb/a.mp3", Some(200_500), Some(999), &[])
            .expect("query");
    assert_eq!(no_match_size, None);
}

#[test]
fn find_confident_fingerprint_match_recovers_missing_size_from_file() {
    let (_dir, db) = test_db();
    let conn = db.connect().expect("connect");
    let files = tempfile::tempdir().expect("files");
    let local = files.path().join("local.mp3");
    let usb = files.path().join("usb.mp3");
    std::fs::write(&local, [1u8; 4]).expect("local file");
    std::fs::write(&usb, [2u8; 4]).expect("USB file");
    insert_track(
        &conn,
        "local-1",
        local.to_str().unwrap(),
        "fp1",
        Some(200_000),
        None,
    );

    let matched = find_confident_fingerprint_match(
        &conn,
        "fp1",
        usb.to_str().unwrap(),
        Some(200_100),
        None,
        &[],
    )
    .expect("query");
    assert_eq!(matched, Some("local-1".to_string()));

    std::fs::write(&usb, [2u8; 5]).expect("different size");
    let mismatched = find_confident_fingerprint_match(
        &conn,
        "fp1",
        usb.to_str().unwrap(),
        Some(200_100),
        None,
        &[],
    )
    .expect("query");
    assert_eq!(mismatched, None);

    std::fs::remove_file(&local).expect("remove local file");
    let unavailable = find_confident_fingerprint_match(
        &conn,
        "fp1",
        usb.to_str().unwrap(),
        Some(200_100),
        None,
        &[],
    )
    .expect("query");
    assert_eq!(unavailable, None);
}

#[test]
fn find_confident_fingerprint_match_keeps_different_encodings_separate() {
    let (_dir, db) = test_db();
    let conn = db.connect().expect("connect");
    insert_track(
        &conn,
        "local-flac",
        "/music/a.flac",
        "fp1",
        Some(200_000),
        None,
    );
    let result =
        find_confident_fingerprint_match(&conn, "fp1", "/usb/a.mp3", Some(200_000), None, &[])
            .expect("query");
    assert_eq!(result, None);
}

#[test]
fn find_confident_fingerprint_match_refuses_to_pick_between_two_candidates() {
    let (_dir, db) = test_db();
    let conn = db.connect().expect("connect");
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
        "local-2",
        "/music/b.mp3",
        "fp1",
        Some(200_000),
        Some(1_000),
    );

    let result = find_confident_fingerprint_match(
        &conn,
        "fp1",
        "/usb/a.mp3",
        Some(200_000),
        Some(1_000),
        &[],
    )
    .expect("query");
    assert_eq!(result, None, "ambiguous ties must not auto-merge");
}

#[test]
fn find_confident_fingerprint_match_skips_candidates_under_a_known_usb_root() {
    let (_dir, db) = test_db();
    let conn = db.connect().expect("connect");
    insert_track(
        &conn,
        "placeholder-1",
        "/mnt/usb1/Contents/a.mp3",
        "fp1",
        Some(200_000),
        Some(1_000),
    );

    let result = find_confident_fingerprint_match(
        &conn,
        "fp1",
        "/usb/a.mp3",
        Some(200_000),
        Some(1_000),
        &["/mnt/usb1".to_string()],
    )
    .expect("query");
    assert_eq!(
        result, None,
        "placeholder-vs-placeholder must never be treated as a match"
    );
}

#[test]
fn untainted_usb_root_paths_excludes_configured_source_roots() {
    let (_dir, db) = test_db();
    let conn = db.connect().expect("connect");
    usb_utils::upsert_usb_device(&conn, Path::new("/mnt/data"), false, &now())
        .expect("seed usb device");
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
        params![SETTING_UI_SOURCE_ROOTS, r#"["/mnt/data"]"#],
    )
    .expect("seed source roots setting");

    let paths = untainted_usb_root_paths(&conn).expect("untainted paths");
    assert!(
        !paths.iter().any(|p| p == "/mnt/data"),
        "a path reused as a configured source root should no longer be treated as USB-tainted"
    );
}

// --- small pure helpers ---

#[test]
fn sanitize_source_roots_trims_dedupes_and_drops_empty() {
    let roots = sanitize_source_roots(vec![
        "  /music  ".to_string(),
        "/music".to_string(),
        "   ".to_string(),
        "/other".to_string(),
    ]);
    assert_eq!(roots, vec!["/music".to_string(), "/other".to_string()]);
}

#[test]
fn source_root_is_usable_true_for_dir_false_for_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(source_root_is_usable(dir.path()));
    assert!(!source_root_is_usable(Path::new(
        "/definitely/does/not/exist/anywhere"
    )));
}

#[test]
fn source_root_status_reports_exists_and_is_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let status = source_root_status(dir.path().to_str().unwrap());
    assert!(status.exists);
    assert!(status.is_dir);

    let missing = source_root_status("/definitely/does/not/exist/anywhere");
    assert!(!missing.exists);
    assert!(!missing.is_dir);
}

#[test]
fn relative_path_under_source_root_matches_case_insensitively() {
    assert_eq!(
        relative_path_under_source_root("/Music/Artist/song.mp3", "/music"),
        Some("Artist/song.mp3".to_string())
    );
    assert_eq!(
        relative_path_under_source_root("/music", "/music"),
        Some(String::new())
    );
    assert_eq!(
        relative_path_under_source_root("/other/song.mp3", "/music"),
        None
    );
    assert_eq!(relative_path_under_source_root("/music/song.mp3", ""), None);
}

#[test]
fn relocated_source_path_joins_segments_and_handles_root_only() {
    let root = Path::new("/new/root");
    assert_eq!(
        relocated_source_path(root, "Artist/song.mp3"),
        root.join("Artist").join("song.mp3")
    );
    assert_eq!(relocated_source_path(root, ""), root.to_path_buf());
    assert_eq!(relocated_source_path(root, "///"), root.to_path_buf());
}

#[test]
fn track_has_core_analysis_for_source_status_requires_all_three_fields() {
    let mut track = sample_track("t1", "/music/a.mp3");
    assert!(!track_has_core_analysis_for_source_status(&track));

    track.waveform_peaks_path = Some("/data/a.dat".to_string());
    track.bpm = Some(120.0);
    track.duration_ms = Some(200_000);
    assert!(track_has_core_analysis_for_source_status(&track));

    track.bpm = Some(0.0);
    assert!(!track_has_core_analysis_for_source_status(&track));
}

#[test]
fn has_core_analysis_fields_requires_waveform_bpm_and_positive_duration() {
    assert!(has_core_analysis_fields(
        Some("/data/a.dat"),
        Some(120.0),
        Some(200_000)
    ));
    // missing / blank waveform path
    assert!(!has_core_analysis_fields(None, Some(120.0), Some(200_000)));
    assert!(!has_core_analysis_fields(
        Some("   "),
        Some(120.0),
        Some(200_000)
    ));
    // non-positive bpm
    assert!(!has_core_analysis_fields(
        Some("/data/a.dat"),
        Some(0.0),
        Some(200_000)
    ));
    assert!(!has_core_analysis_fields(
        Some("/data/a.dat"),
        None,
        Some(200_000)
    ));
    // non-positive / missing duration
    assert!(!has_core_analysis_fields(
        Some("/data/a.dat"),
        Some(120.0),
        Some(0)
    ));
    assert!(!has_core_analysis_fields(
        Some("/data/a.dat"),
        Some(120.0),
        None
    ));
}

#[test]
fn non_empty_db_value_filters_blank_and_trims() {
    assert_eq!(non_empty_db_value("  hello  "), Some("hello"));
    assert_eq!(non_empty_db_value("   "), None);
    assert_eq!(non_empty_db_value(""), None);
}

#[test]
fn looks_like_windows_absolute_path_detects_drive_letter() {
    assert!(looks_like_windows_absolute_path("C:/Users/dj"));
    assert!(looks_like_windows_absolute_path("D:\\Music"));
    assert!(!looks_like_windows_absolute_path("/mnt/data"));
    assert!(!looks_like_windows_absolute_path("relative/path"));
}

#[test]
fn is_pioneer_virtual_path_case_insensitive() {
    assert!(is_pioneer_virtual_path("/PIONEER/USBANLZ/x"));
    assert!(is_pioneer_virtual_path("pioneer/usbanlz/x"));
    assert!(!is_pioneer_virtual_path("/Contents/x"));
}

#[test]
fn push_unique_path_dedupes_equal_paths() {
    let mut paths = Vec::new();
    push_unique_path(&mut paths, PathBuf::from("/a"));
    push_unique_path(&mut paths, PathBuf::from("/a"));
    push_unique_path(&mut paths, PathBuf::from("/b"));
    assert_eq!(paths, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
}

#[test]
fn resolve_rekordbox_db_resource_path_returns_first_existing_candidate() {
    let master_path = Path::new("/tmp/rekordbox/master.db");
    let db_path = "/PIONEER/USBANLZ/x/ANLZ0000.DAT";
    let expected = rekordbox_db_resource_candidates(master_path, db_path)
        .into_iter()
        .nth(1)
        .expect("at least two candidates");
    let expected_for_closure = expected.clone();
    let resolved =
        resolve_rekordbox_db_resource_path(master_path, db_path, |p| p == expected_for_closure);
    assert_eq!(resolved, Some(expected));
}

#[test]
fn resolve_rekordbox_db_resource_path_none_when_nothing_exists() {
    let master_path = Path::new("/tmp/rekordbox/master.db");
    let resolved =
        resolve_rekordbox_db_resource_path(master_path, "/PIONEER/x/ANLZ0000.DAT", |_| false);
    assert!(resolved.is_none());
}

#[test]
fn frontend_ui_setting_keys_contains_theme_and_help_seen() {
    let keys = frontend_ui_setting_keys();
    assert!(keys.contains(&SETTING_UI_THEME));
    assert!(keys.contains(&SETTING_UI_HELP_SEEN));
}

/// Every `FRONTEND_DB_KEY_*` the frontend persists must be accepted by
/// `set_frontend_setting`, or saving it fails with "unsupported frontend
/// setting key" and the value only survives in the webview's localStorage.
#[test]
fn frontend_ui_setting_keys_cover_every_frontend_db_key() {
    let source = include_str!("../../../vanilla-ui/settings_keys.mjs");
    let frontend_keys: Vec<&str> = source
        .lines()
        .filter(|line| {
            line.trim_start()
                .starts_with("export const FRONTEND_DB_KEY_")
        })
        .filter_map(|line| line.split('"').nth(1))
        .collect();
    assert!(frontend_keys.len() > 10, "parsed {frontend_keys:?}");
    let allowed = frontend_ui_setting_keys();
    let missing: Vec<_> = frontend_keys
        .iter()
        .filter(|key| !allowed.contains(key))
        .collect();
    assert!(
        missing.is_empty(),
        "backend rejects frontend setting keys: {missing:?}"
    );
}

#[test]
fn source_root_keys_equal_normalizes_case_and_trailing_slash() {
    assert!(source_root_keys_equal("/Music/", "/music"));
    assert!(!source_root_keys_equal("/music", "/other"));
}

#[test]
fn path_file_name_and_stem_lower_handle_backslashes() {
    assert_eq!(matching_file_name(r"C:\Music\Song.MP3"), "song.mp3");
    assert_eq!(matching_file_stem(r"C:\Music\Song.MP3"), "song");
    assert_eq!(matching_file_stem("no-extension"), "no-extension");
}

#[test]
fn build_track_cursor_signature_differs_by_input() {
    let sig_a = build_track_cursor_signature(&["v1", "list_tracks"]);
    let sig_b = build_track_cursor_signature(&["v1", "search_tracks"]);
    assert_ne!(sig_a, sig_b);
    assert_eq!(sig_a, build_track_cursor_signature(&["v1", "list_tracks"]));
}

#[test]
fn check_essentia_installed_false_when_files_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(!check_essentia_installed(dir.path()));
}

// --- BackendService CRUD/service-level tests ---

fn test_service() -> (tempfile::TempDir, BackendService) {
    let dir = tempfile::tempdir().expect("service data dir");
    let service = BackendService::new(dir.path()).expect("backend service");
    (dir, service)
}

fn sample_track(id: &str, file_path: &str) -> Track {
    Track {
        id: id.to_string(),
        title: "Title".to_string(),
        artist: "Artist".to_string(),
        album: None,
        year: None,
        genre: None,
        track_number: None,
        bpm: None,
        bpm_analyzer: None,
        key: None,
        key_display: None,
        key_color: None,
        key_source: None,
        file_path: file_path.to_string(),
        file_size_bytes: None,
        format_ext: None,
        sample_rate_hz: None,
        bit_depth: None,
        bitrate_kbps: None,
        wav_extensible_kind: None,
        duration_ms: None,
        artwork_path: None,
        artwork_data_url: None,
        waveform_peaks_path: None,
        waveform_preview: None,
        waveform_color_data: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        rekordbox_db_source: false,
        mixxx_db_source: false,
        is_usb_path: false,
        analysis_ready: false,
        format_compat: Default::default(),
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_full_track(
    conn: &rusqlite::Connection,
    id: &str,
    title: &str,
    artist: &str,
    file_path: &str,
    duration_ms: Option<i64>,
    file_size_bytes: Option<i64>,
    rekordbox_db_source: bool,
) {
    let fp = build_track_match_fingerprint(title, artist, None);
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, file_size_bytes, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'), datetime('now'))",
        params![id, title, artist, file_path, duration_ms, file_size_bytes, fp, rekordbox_db_source as i64],
    )
    .expect("insert track");
}

#[test]
fn list_playlists_names_the_drive_as_it_was_called_at_export() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("service db");
    // Another stick, "Booth Stick", now uses the mount path both playlists
    // were exported to.
    conn.execute_batch(&format!(
        "INSERT INTO usb_devices (id, root_path, root_path_key, label, first_seen_at, last_seen_at, created_at, updated_at)
           VALUES ('d1', '/run/media/dj/STICK', '{}', 'Booth Stick', 'x', 'x', 'x', 'x');
         INSERT INTO playlists (id, name, source, last_exported_at, last_exported_usb_root, last_exported_usb_name, created_at, updated_at)
           VALUES ('p1', 'Named', 'local', 'x', '/run/media/dj/STICK', 'Club Stick', '1', '1'),
                  ('p2', 'Before names were kept', 'local', 'x', '/run/media/dj/STICK', NULL, '2', '2'),
                  ('p3', 'Never', 'local', NULL, NULL, NULL, '3', '3');",
        normalize_source_root_for_matching("/run/media/dj/STICK")
    ))
    .expect("seed");
    drop(conn);
    let names: Vec<Option<String>> = service
        .list_playlists()
        .expect("list")
        .items
        .into_iter()
        .map(|p| p.last_exported_usb_name)
        .collect();
    // Never reordered, so newest first: p3, p2, p1. The path's current
    // drive names neither export.
    assert_eq!(names, vec![None, None, Some("Club Stick".to_string())]);
}

#[test]
fn reorder_playlists_saves_sidebar_order_and_new_playlists_go_on_top() {
    let (_dir, service) = test_service();
    let create = |name: &str| {
        service
            .create_playlist(CreatePlaylistRequest {
                name: name.to_string(),
            })
            .expect("create playlist")
            .playlist_id
    };
    let listed_names = || -> Vec<String> {
        service
            .list_playlists()
            .expect("list")
            .items
            .into_iter()
            .map(|p| p.name)
            .collect()
    };
    let a = create("A");
    let b = create("B");
    let c = create("C");
    assert_eq!(listed_names(), ["C", "B", "A"]);

    let saved = service
        .reorder_playlists(ReorderPlaylistsRequest {
            ordered_playlist_ids: vec![a.clone(), c.clone(), b.clone()],
        })
        .expect("reorder");
    assert_eq!(saved.reordered, 3);
    assert_eq!(listed_names(), ["A", "C", "B"]);

    create("D");
    assert_eq!(listed_names(), ["D", "A", "C", "B"]);

    // Missing, duplicate or unknown ids change nothing.
    for ids in [
        vec![a.clone(), c.clone(), b.clone()],
        vec![a.clone(), a.clone(), c.clone(), b.clone()],
        vec![a.clone(), c.clone(), b.clone(), "nope".to_string()],
    ] {
        let err = service
            .reorder_playlists(ReorderPlaylistsRequest {
                ordered_playlist_ids: ids,
            })
            .unwrap_err();
        assert!(matches!(err, BackendError::Validation(_)));
    }
    assert_eq!(listed_names(), ["D", "A", "C", "B"]);
}

#[test]
fn create_playlist_rejects_empty_name() {
    let (_dir, service) = test_service();
    let err = service
        .create_playlist(CreatePlaylistRequest {
            name: "   ".to_string(),
        })
        .unwrap_err();
    assert!(matches!(err, BackendError::Validation(_)));
}

#[test]
fn playlist_create_rename_delete_roundtrip() {
    let (_dir, service) = test_service();
    let created = service
        .create_playlist(CreatePlaylistRequest {
            name: "  My Playlist  ".to_string(),
        })
        .expect("create playlist");
    assert_eq!(created.name, "My Playlist");

    let listed = service.list_playlists().expect("list playlists");
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].id, created.playlist_id);

    let renamed = service
        .rename_playlist(RenamePlaylistRequest {
            playlist_id: created.playlist_id.clone(),
            name: "Renamed".to_string(),
        })
        .expect("rename playlist");
    assert_eq!(renamed.name, "Renamed");

    let deleted = service
        .delete_playlist(DeletePlaylistRequest {
            playlist_id: created.playlist_id.clone(),
        })
        .expect("delete playlist");
    assert!(deleted.deleted);

    let err = service
        .rename_playlist(RenamePlaylistRequest {
            playlist_id: created.playlist_id,
            name: "Nope".to_string(),
        })
        .unwrap_err();
    assert!(matches!(err, BackendError::NotFound(_)));
}

#[test]
fn add_tracks_to_playlist_rejects_empty_ids() {
    let (_dir, service) = test_service();
    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    let err = service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id,
            track_ids: Vec::new(),
        })
        .unwrap_err();
    assert!(matches!(err, BackendError::Validation(_)));
}

#[test]
fn add_tracks_to_playlist_rejects_unknown_track() {
    let (_dir, service) = test_service();
    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    let err = service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id,
            track_ids: vec!["missing-track".to_string()],
        })
        .unwrap_err();
    assert!(matches!(err, BackendError::NotFound(_)));
}

#[test]
fn add_track_candidates_to_playlist_materializes_local_source_rows() {
    let (_dir, service) = test_service();
    let source_dir = tempfile::tempdir().expect("source dir");
    let file_path = source_dir.path().join("track.mp3");
    std::fs::write(&file_path, b"audio").expect("write source file");
    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");

    let result = service
        .add_track_candidates_to_playlist(AddTrackCandidatesToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            tracks: vec![AddTrackCandidate {
                track_id: Some(file_path.to_string_lossy().to_string()),
                title: "Track".to_string(),
                artist: "Artist".to_string(),
                file_path: Some(file_path.to_string_lossy().to_string()),
                ..Default::default()
            }],
            usb_root: None,
            usb_root_valid: false,
        })
        .expect("add candidate");

    assert_eq!(result.requested, 1);
    assert_eq!(result.resolved, 1);
    assert_eq!(result.unresolved, 0);
    assert_eq!(result.added, 1);
    assert_eq!(result.skipped, 0);
    assert!(result.resolutions[0].materialized);
    assert_eq!(
        result.resolutions[0].resolved_by,
        crate::models::ResolvedBy::Materialized
    );

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(tracks.items.len(), 1);
    assert_eq!(tracks.items[0].title, "Track");
}

#[test]
fn add_track_candidates_to_playlist_does_not_fuzzy_resolve_usb_origin_rows() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "local-1",
        "Same Song",
        "Same Artist",
        "/music/same.mp3",
        None,
        Some(1000),
        false,
    );
    drop(conn);
    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");

    let result = service
        .add_track_candidates_to_playlist(AddTrackCandidatesToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            tracks: vec![AddTrackCandidate {
                track_id: Some("usb-1".to_string()),
                title: "Same Song".to_string(),
                artist: "Same Artist".to_string(),
                file_path: Some("/usb/Contents/same.mp3".to_string()),
                file_size_bytes: Some(1000),
                usb_root: Some("/usb".to_string()),
                usb_analysis_path: Some("/usb/PIONEER/USBANLZ/P001/A/ANLZ0000.DAT".to_string()),
                ..Default::default()
            }],
            usb_root: None,
            usb_root_valid: false,
        })
        .expect("add candidate");

    assert_eq!(result.requested, 1);
    assert_eq!(result.resolved, 0);
    assert_eq!(result.unresolved, 1);
    assert_eq!(result.added, 0);
    assert_eq!(result.skipped, 0);
    assert_eq!(
        result.resolutions[0].resolved_by,
        crate::models::ResolvedBy::UsbOrigin
    );

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert!(tracks.items.is_empty());
}

#[test]
fn add_tracks_to_playlist_dedupe_skip_counts_skipped_and_get_playlist_tracks_orders_by_position() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song A",
        "Artist",
        "/music/a.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t2",
        "Song B",
        "Artist",
        "/music/b.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");

    let added = service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string()],
        })
        .expect("add tracks");
    assert_eq!(added.added, 2);
    assert_eq!(added.skipped, 0);

    let skip_result = service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string()],
        })
        .expect("add duplicate with skip mode");
    assert_eq!(skip_result.added, 0);
    assert_eq!(skip_result.skipped, 1);

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(
        tracks
            .items
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        vec!["t1".to_string(), "t2".to_string()]
    );
}

#[test]
fn get_playlist_tracks_list_totals_ignore_search() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song A",
        "Artist",
        "/music/a.mp3",
        Some(200_000),
        Some(8_000_000),
        false,
    );
    insert_full_track(
        &conn,
        "t2",
        "Song B",
        "Artist",
        "/music/b.mp3",
        Some(100_000),
        Some(2_000_000),
        false,
    );
    // Unknown size: counted as a track, adds nothing to the size.
    insert_full_track(
        &conn,
        "t3",
        "Song C",
        "Artist",
        "/music/c.mp3",
        Some(100_000),
        None,
        false,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        })
        .expect("add tracks");

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            query: "song b".to_string(),
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(tracks.total, 1);
    assert_eq!(tracks.total_duration_ms, 100_000);
    assert_eq!(tracks.list_track_count, 3);
    assert_eq!(tracks.list_size_bytes, 10_000_000);
}

#[test]
fn get_playlist_tracks_computes_total_duration_server_side() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song A",
        "Artist",
        "/music/a.mp3",
        Some(200_000),
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t2",
        "Song B",
        "Artist",
        "/music/b.mp3",
        None,
        None,
        false,
    );
    // master.db-sourced -- must survive get_playlist_tracks' hand-written
    // SELECT (regression: its column list was misaligned vs `row_to_track`,
    // which reads rekordbox_db_source positionally, so every playlist track
    // came back rekordbox_db_source=false).
    insert_full_track(
        &conn,
        "t3",
        "Song C",
        "Artist",
        "/music/c.mp3",
        Some(100_000),
        None,
        true,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        })
        .expect("add tracks");

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(tracks.total_duration_ms, 300_000);
    assert_eq!(tracks.duration_known_count, 2);

    let by_id = |id: &str| {
        tracks
            .items
            .iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("missing track {id}"))
    };
    assert!(
        by_id("t3").rekordbox_db_source,
        "rekordbox_db_source must survive get_playlist_tracks"
    );
    assert!(!by_id("t1").rekordbox_db_source);
}

#[test]
fn get_selection_duration_sums_only_the_ticked_playlist_tracks() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    for (id, path, duration) in [
        ("t1", "/music/a.mp3", Some(200_000)),
        ("t2", "/music/b.mp3", None),
        ("t3", "/music/c.mp3", Some(100_000)),
    ] {
        insert_full_track(&conn, id, "Song", "Artist", path, duration, None, false);
    }
    drop(conn);
    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        })
        .expect("add tracks");

    // t3 unticked; "gone" is no longer in the playlist and adds nothing.
    let data = service
        .get_selection_duration(GetSelectionDurationRequest::Playlist {
            id: playlist.playlist_id,
            track_ids: vec!["t1".to_string(), "t2".to_string(), "gone".to_string()],
        })
        .expect("selection duration");
    assert_eq!(data.total_duration_ms, 200_000);
    assert_eq!(data.duration_known_count, 1);
    assert_eq!(data.track_count, 2);
}

#[test]
fn get_playlist_tracks_paginates_filters_and_sorts_server_side() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    for (id, title) in [("t1", "Charlie"), ("t2", "Alpha"), ("t3", "Bravo")] {
        insert_full_track(
            &conn,
            id,
            title,
            "Artist",
            &format!("/m/{id}.mp3"),
            None,
            None,
            false,
        );
    }
    drop(conn);
    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    let pid = playlist.playlist_id.clone();
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: pid.clone(),
            track_ids: vec!["t1".into(), "t2".into(), "t3".into()],
        })
        .expect("add tracks");

    // Default order is playlist position; totals span the whole playlist.
    let page1 = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: pid.clone(),
            limit: 2,
            ..Default::default()
        })
        .expect("page 1");
    assert_eq!(page1.total, 3);
    assert_eq!(
        page1.items.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
        ["t1", "t2"]
    );
    assert!(page1.has_more);
    let page2 = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: pid.clone(),
            limit: 2,
            cursor: page1.next_cursor.clone(),
            ..Default::default()
        })
        .expect("page 2");
    assert_eq!(
        page2.items.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
        ["t3"]
    );
    assert!(!page2.has_more);

    // An offset jumps straight to a position, with no cursor needed.
    let jumped = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: pid.clone(),
            limit: 1,
            offset: Some(1),
            ..Default::default()
        })
        .expect("offset page");
    assert_eq!(
        jumped
            .items
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        ["t2"]
    );
    assert!(jumped.has_more);

    // Server-side title sort spans the whole playlist.
    let sorted = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: pid.clone(),
            sort_by: Some("title".into()),
            ..Default::default()
        })
        .expect("sorted");
    assert_eq!(
        sorted
            .items
            .iter()
            .map(|t| t.title.clone())
            .collect::<Vec<_>>(),
        ["Alpha", "Bravo", "Charlie"]
    );

    // A cursor from one sort/query is rejected under another.
    let err = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: pid.clone(),
            cursor: page1.next_cursor,
            sort_by: Some("title".into()),
            ..Default::default()
        })
        .unwrap_err();
    assert!(matches!(err, BackendError::Validation(_)));

    // Query filters and still reports whole-playlist... no -- total is the
    // filtered count, duration spans the filtered set.
    let filtered = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: pid,
            query: "alph".into(),
            ..Default::default()
        })
        .expect("filtered");
    assert_eq!(filtered.total, 1);
    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].id, "t2");
}

#[test]
fn remove_tracks_from_playlist_recompacts_positions() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song A",
        "Artist",
        "/music/a.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t2",
        "Song B",
        "Artist",
        "/music/b.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t3",
        "Song C",
        "Artist",
        "/music/c.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        })
        .expect("add tracks");

    let removed = service
        .remove_tracks_from_playlist(RemoveTracksFromPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t2".to_string()],
        })
        .expect("remove track");
    assert_eq!(removed.removed, 1);

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(
        tracks
            .items
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        vec!["t1".to_string(), "t3".to_string()]
    );
}

#[test]
fn reorder_playlist_tracks_applies_arbitrary_new_order() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song A",
        "Artist",
        "/music/a.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t2",
        "Song B",
        "Artist",
        "/music/b.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t3",
        "Song C",
        "Artist",
        "/music/c.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        })
        .expect("add tracks");

    let reordered = service
        .reorder_playlist_tracks(ReorderPlaylistTracksRequest {
            playlist_id: playlist.playlist_id.clone(),
            ordered_track_ids: vec!["t3".to_string(), "t1".to_string(), "t2".to_string()],
            ..Default::default()
        })
        .expect("reorder tracks");
    assert_eq!(reordered.reordered, 3);

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(
        tracks
            .items
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        vec!["t3".to_string(), "t1".to_string(), "t2".to_string()]
    );
}

#[test]
fn reorder_playlist_tracks_sort_commit_mode_orders_the_whole_playlist() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn, "t1", "Charlie", "Artist", "/m/c.mp3", None, None, false,
    );
    insert_full_track(
        &conn, "t2", "Alpha", "Artist", "/m/a.mp3", None, None, false,
    );
    insert_full_track(
        &conn, "t3", "Bravo", "Artist", "/m/b.mp3", None, None, false,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        })
        .expect("add tracks");

    let reordered = service
        .reorder_playlist_tracks(ReorderPlaylistTracksRequest {
            playlist_id: playlist.playlist_id.clone(),
            sort_by: Some("title".to_string()),
            sort_dir: Some("asc".to_string()),
            ..Default::default()
        })
        .expect("sort-commit reorder");
    assert_eq!(reordered.reordered, 3);

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(
        tracks
            .items
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        vec!["t2".to_string(), "t3".to_string(), "t1".to_string()]
    );
}

#[test]
fn reorder_playlist_tracks_single_move_mode_repositions_one_track() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(&conn, "t1", "One", "Artist", "/m/1.mp3", None, None, false);
    insert_full_track(&conn, "t2", "Two", "Artist", "/m/2.mp3", None, None, false);
    insert_full_track(
        &conn, "t3", "Three", "Artist", "/m/3.mp3", None, None, false,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        })
        .expect("add tracks");

    // Move t3 to before t2 -> [t1, t3, t2].
    service
        .reorder_playlist_tracks(ReorderPlaylistTracksRequest {
            playlist_id: playlist.playlist_id.clone(),
            move_track_id: Some("t3".to_string()),
            before_track_id: Some("t2".to_string()),
            ..Default::default()
        })
        .expect("single move");
    let order = |svc: &BackendService| {
        svc.get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id.clone(),
            ..Default::default()
        })
        .expect("get playlist tracks")
        .items
        .iter()
        .map(|t| t.id.clone())
        .collect::<Vec<_>>()
    };
    assert_eq!(order(&service), vec!["t1", "t3", "t2"]);

    // before_track_id absent -> move t1 to the end -> [t3, t2, t1].
    service
        .reorder_playlist_tracks(ReorderPlaylistTracksRequest {
            playlist_id: playlist.playlist_id.clone(),
            move_track_id: Some("t1".to_string()),
            ..Default::default()
        })
        .expect("move to end");
    assert_eq!(order(&service), vec!["t3", "t2", "t1"]);

    // Unknown move_track_id is rejected.
    assert!(
        service
            .reorder_playlist_tracks(ReorderPlaylistTracksRequest {
                playlist_id: playlist.playlist_id.clone(),
                move_track_id: Some("nope".to_string()),
                ..Default::default()
            })
            .is_err()
    );
}

#[test]
fn reorder_playlist_tracks_rejects_mismatched_track_set() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song A",
        "Artist",
        "/music/a.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t2",
        "Song B",
        "Artist",
        "/music/b.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["t1".to_string(), "t2".to_string()],
        })
        .expect("add tracks");

    let missing_track = service.reorder_playlist_tracks(ReorderPlaylistTracksRequest {
        playlist_id: playlist.playlist_id.clone(),
        ordered_track_ids: vec!["t1".to_string()],
        ..Default::default()
    });
    assert!(missing_track.is_err());

    let unknown_track = service.reorder_playlist_tracks(ReorderPlaylistTracksRequest {
        playlist_id: playlist.playlist_id.clone(),
        ordered_track_ids: vec!["t1".to_string(), "t2".to_string(), "t3".to_string()],
        ..Default::default()
    });
    assert!(unknown_track.is_err());

    let duplicate_track = service.reorder_playlist_tracks(ReorderPlaylistTracksRequest {
        playlist_id: playlist.playlist_id.clone(),
        ordered_track_ids: vec!["t1".to_string(), "t1".to_string()],
        ..Default::default()
    });
    assert!(duplicate_track.is_err());

    // No-mode request is rejected.
    let no_mode = service.reorder_playlist_tracks(ReorderPlaylistTracksRequest {
        playlist_id: playlist.playlist_id,
        ..Default::default()
    });
    assert!(no_mode.is_err());
}

#[test]
fn frontend_settings_roundtrip_and_reject_unsupported_key() {
    let (_dir, service) = test_service();
    let defaults = service.get_frontend_settings().expect("get settings");
    assert!(defaults.values.is_empty());

    service
        .set_frontend_setting(SetFrontendSettingRequest {
            key: SETTING_UI_THEME.to_string(),
            value: Some("dark".to_string()),
        })
        .expect("set theme");
    let after_set = service.get_frontend_settings().expect("get settings");
    assert_eq!(
        after_set.values.get(SETTING_UI_THEME).map(String::as_str),
        Some("dark")
    );

    service
        .set_frontend_setting(SetFrontendSettingRequest {
            key: SETTING_UI_THEME.to_string(),
            value: None,
        })
        .expect("clear theme");
    let after_clear = service.get_frontend_settings().expect("get settings");
    assert!(!after_clear.values.contains_key(SETTING_UI_THEME));

    let err = service
        .set_frontend_setting(SetFrontendSettingRequest {
            key: "not_a_real_setting".to_string(),
            value: Some("x".to_string()),
        })
        .unwrap_err();
    assert!(matches!(err, BackendError::Validation(_)));
}

#[test]
fn remove_essentia_removes_dir_and_clears_setting() {
    let (_dir, service) = test_service();
    let essentia_dir = service.db.data_dir().join("essentia");
    std::fs::create_dir_all(&essentia_dir).expect("create essentia dir");
    std::fs::write(essentia_dir.join("marker.txt"), b"x").expect("write marker");
    service
        .set_frontend_setting(SetFrontendSettingRequest {
            key: SETTING_UI_ANALYSIS_ENGINE.to_string(),
            value: Some("essentia".to_string()),
        })
        .expect("set analysis engine");

    service.remove_essentia().expect("remove essentia");

    assert!(!essentia_dir.exists());
    let settings = service.get_frontend_settings().expect("get settings");
    assert!(!settings.values.contains_key(SETTING_UI_ANALYSIS_ENGINE));
}

#[test]
fn browse_source_files_empty_without_roots_or_rekordbox_db() {
    let (_dir, service) = test_service();
    let result = service
        .browse_source_files(BrowseSourceFilesRequest {
            source_roots: Vec::new(),
            include_rekordbox_db: false,
            include_mixxx_db: false,
            query: String::new(),
            limit: 100,
            cursor: None,
            ..Default::default()
        })
        .expect("browse source files");
    assert_eq!(result.total, 0);
    assert!(result.items.is_empty());
}

#[test]
fn browse_source_files_includes_rekordbox_db_tracks_when_requested() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Master Song",
        "Artist",
        "/master/a.mp3",
        None,
        None,
        true,
    );
    drop(conn);

    let result = service
        .browse_source_files(BrowseSourceFilesRequest {
            source_roots: Vec::new(),
            include_rekordbox_db: true,
            include_mixxx_db: false,
            query: String::new(),
            limit: 100,
            cursor: None,
            ..Default::default()
        })
        .expect("browse source files");
    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].id, "t1");
    assert!(result.items[0].rekordbox_db_source);
}

#[test]
fn browse_source_files_computes_duration_total_over_countable_tracks_only() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    // Fully core-analyzed (bpm + waveform + duration): countable via the
    // primary rule.
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, waveform_peaks_path, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES ('t1', 'Analyzed', 'Artist', '/master/a.mp3', 200000, 120.0, '/data/a.dat', ?1, 1, datetime('now'), datetime('now'))",
        params![build_track_match_fingerprint("Analyzed", "Artist", None)],
    )
    .expect("insert t1");
    // master.db track with only a duration: countable via the
    // rekordbox-db-specific OR-branch, not the primary rule.
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES ('t2', 'Master Only', 'Artist', '/master/b.mp3', 100000, ?1, 1, datetime('now'), datetime('now'))",
        params![build_track_match_fingerprint("Master Only", "Artist", None)],
    )
    .expect("insert t2");
    // No analysis at all: not countable, excluded from both the sum and
    // the known count, but still present in `total`.
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES ('t3', 'Unanalyzed', 'Artist', '/master/c.mp3', ?1, 1, datetime('now'), datetime('now'))",
        params![build_track_match_fingerprint("Unanalyzed", "Artist", None)],
    )
    .expect("insert t3");
    drop(conn);

    let result = service
        .browse_source_files(BrowseSourceFilesRequest {
            source_roots: Vec::new(),
            include_rekordbox_db: true,
            include_mixxx_db: false,
            query: String::new(),
            limit: 100,
            cursor: None,
            ..Default::default()
        })
        .expect("browse source files");
    assert_eq!(result.total, 3);
    assert_eq!(result.duration_known_count, 2);
    assert_eq!(result.total_duration_ms, 300_000);
}

#[test]
fn browse_source_files_sorts_server_side_across_pages() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    for (id, title) in [("m1", "Zeta"), ("m2", "Alpha"), ("m3", "Mu")] {
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, match_fingerprint, rekordbox_db_source, created_at, updated_at)
             VALUES (?1, ?2, 'Artist', ?3, ?4, 1, datetime('now'), datetime('now'))",
            params![id, title, format!("/master/{id}.mp3"), build_track_match_fingerprint(title, "Artist", None)],
        )
        .expect("insert");
    }
    drop(conn);

    let req = |cursor: Option<String>| BrowseSourceFilesRequest {
        include_rekordbox_db: true,
        include_mixxx_db: false,
        limit: 2,
        cursor,
        sort_by: Some("title".to_string()),
        sort_dir: Some("desc".to_string()),
        ..Default::default()
    };
    let page1 = service.browse_source_files(req(None)).expect("page 1");
    assert_eq!(
        page1
            .items
            .iter()
            .map(|t| t.title.clone())
            .collect::<Vec<_>>(),
        ["Zeta", "Mu"]
    );
    assert!(page1.has_more);
    let page2 = service
        .browse_source_files(req(page1.next_cursor))
        .expect("page 2");
    assert_eq!(
        page2
            .items
            .iter()
            .map(|t| t.title.clone())
            .collect::<Vec<_>>(),
        ["Alpha"]
    );

    // An offset jumps straight to a position in the sorted list.
    let jumped = service
        .browse_source_files(BrowseSourceFilesRequest {
            offset: Some(1),
            limit: 1,
            ..req(None)
        })
        .expect("offset page");
    assert_eq!(
        jumped
            .items
            .iter()
            .map(|t| t.title.clone())
            .collect::<Vec<_>>(),
        ["Mu"]
    );
    assert!(jumped.has_more);
    assert_eq!(jumped.total, 3);
}

#[test]
fn track_rows_carry_analysis_ready_computed_from_db_fields() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    // Fully core-analyzed: bpm + waveform path + duration.
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, waveform_peaks_path, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES ('ready', 'Ready', 'Artist', '/master/ready.mp3', 200000, 120.0, '/data/ready.dat', ?1, 1, datetime('now'), datetime('now'))",
        params![build_track_match_fingerprint("Ready", "Artist", None)],
    )
    .expect("insert ready");
    // Has duration + bpm but no waveform path -> still needs analysis.
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES ('missing', 'Missing', 'Artist', '/master/missing.mp3', 200000, 120.0, ?1, 1, datetime('now'), datetime('now'))",
        params![build_track_match_fingerprint("Missing", "Artist", None)],
    )
    .expect("insert missing");
    drop(conn);

    let result = service
        .browse_source_files(BrowseSourceFilesRequest {
            source_roots: Vec::new(),
            include_rekordbox_db: true,
            include_mixxx_db: false,
            query: String::new(),
            limit: 100,
            cursor: None,
            ..Default::default()
        })
        .expect("browse source files");
    let by_id = |id: &str| {
        result
            .items
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .analysis_ready
    };
    assert!(by_id("ready"));
    assert!(!by_id("missing"));
}

#[test]
fn track_rows_derive_format_ext_from_path_when_the_column_is_null() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    // master.db import path leaves format_ext NULL.
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES ('m1', 'Song', 'Artist', '/master/Artist/Song.FLAC', ?1, 1, datetime('now'), datetime('now'))",
        params![build_track_match_fingerprint("Song", "Artist", None)],
    )
    .expect("insert null-format row");
    drop(conn);

    let browsed = service
        .browse_source_files(BrowseSourceFilesRequest {
            source_roots: Vec::new(),
            include_rekordbox_db: true,
            include_mixxx_db: false,
            query: String::new(),
            limit: 100,
            cursor: None,
            ..Default::default()
        })
        .expect("browse source files");
    assert_eq!(
        browsed
            .items
            .iter()
            .find(|t| t.id == "m1")
            .unwrap()
            .format_ext
            .as_deref(),
        Some("flac"),
        "row_to_track must fall back to the file-path extension"
    );

    let playlist = service
        .create_playlist(CreatePlaylistRequest {
            name: "PL".to_string(),
        })
        .expect("create playlist");
    service
        .add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist.playlist_id.clone(),
            track_ids: vec!["m1".to_string()],
        })
        .expect("add track");
    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(tracks.items[0].format_ext.as_deref(), Some("flac"));
}

#[test]
fn browse_source_files_source_root_analysis_ignores_the_search_query() {
    let (_dir, service) = test_service();
    let root_a = tempfile::tempdir().expect("root a");
    let root_b = tempfile::tempdir().expect("root b");
    let path_a = root_a.path().join("alpha.mp3");
    let path_b = root_b.path().join("bravo.mp3");
    std::fs::write(&path_a, b"data").expect("write alpha");
    std::fs::write(&path_b, b"data").expect("write bravo");
    let path_a_str = path_a.to_string_lossy().to_string();
    let path_b_str = path_b.to_string_lossy().to_string();

    let conn = service.db.connect().expect("connect");
    // Both tracks are fully analyzed (bpm + waveform + duration). Their
    // titles deliberately share no substring, so a query matching one
    // matches zero tracks in the other's source root.
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, waveform_peaks_path, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES ('t1', 'Findable Alpha', 'Artist', ?1, 200000, 120.0, '/data/a.dat', ?2, 0, datetime('now'), datetime('now'))",
        params![path_a_str, build_track_match_fingerprint("Findable Alpha", "Artist", None)],
    )
    .expect("insert t1");
    conn.execute(
        "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, waveform_peaks_path, match_fingerprint, rekordbox_db_source, created_at, updated_at)
         VALUES ('t2', 'Unrelated Bravo', 'Artist', ?1, 200000, 120.0, '/data/b.dat', ?2, 0, datetime('now'), datetime('now'))",
        params![path_b_str, build_track_match_fingerprint("Unrelated Bravo", "Artist", None)],
    )
    .expect("insert t2");
    drop(conn);

    let result = service
        .browse_source_files(BrowseSourceFilesRequest {
            source_roots: vec![
                root_a.path().to_string_lossy().to_string(),
                root_b.path().to_string_lossy().to_string(),
            ],
            include_rekordbox_db: false,
            include_mixxx_db: false,
            // Matches only the track in root_a -- root_b's track matches
            // nothing, which used to collapse its `total` to 0 and flip
            // `fully_analyzed` to false even though root_b is 100%
            // analyzed on its own.
            query: "Findable".to_string(),
            limit: 100,
            cursor: None,
            ..Default::default()
        })
        .expect("browse source files");

    // The search still narrows what's returned/shown...
    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].id, "t1");

    // ...but both source roots' analyzed status reflects their full,
    // query-independent contents.
    assert_eq!(result.source_root_analysis.len(), 2);
    for status in &result.source_root_analysis {
        assert_eq!(status.total, 1, "source root {}", status.source_root);
        assert_eq!(status.analyzed, 1, "source root {}", status.source_root);
        assert!(status.fully_analyzed, "source root {}", status.source_root);
    }
}

#[test]
fn get_source_root_analysis_counts_each_root_on_its_own() {
    let (_dir, service) = test_service();
    let root_a = tempfile::tempdir().expect("root a");
    let root_b = tempfile::tempdir().expect("root b");
    let path_a = root_a.path().join("alpha.mp3");
    let path_b1 = root_b.path().join("bravo.mp3");
    let path_b2 = root_b.path().join("charlie.mp3");
    for path in [&path_a, &path_b1, &path_b2] {
        std::fs::write(path, b"data").expect("write track");
    }

    let conn = service.db.connect().expect("connect");
    // root_a: one analyzed track. root_b: one analyzed, one not (no bpm/waveform).
    for (id, title, path, analyzed) in [
        ("t1", "Alpha", &path_a, true),
        ("t2", "Bravo", &path_b1, true),
        ("t3", "Charlie", &path_b2, false),
    ] {
        let path = path.to_string_lossy().to_string();
        let fingerprint = build_track_match_fingerprint(title, "Artist", None);
        if analyzed {
            conn.execute(
                "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, waveform_peaks_path, match_fingerprint, rekordbox_db_source, created_at, updated_at)
                 VALUES (?1, ?2, 'Artist', ?3, 200000, 120.0, '/data/w.dat', ?4, 0, datetime('now'), datetime('now'))",
                params![id, title, path, fingerprint],
            )
            .expect("insert analyzed track");
        } else {
            conn.execute(
                "INSERT INTO tracks (id, title, artist, file_path, match_fingerprint, rekordbox_db_source, created_at, updated_at)
                 VALUES (?1, ?2, 'Artist', ?3, ?4, 0, datetime('now'), datetime('now'))",
                params![id, title, path, fingerprint],
            )
            .expect("insert unanalyzed track");
        }
    }
    drop(conn);

    let root_a_str = root_a.path().to_string_lossy().to_string();
    let root_b_str = root_b.path().to_string_lossy().to_string();
    let result = service
        .get_source_root_analysis(GetSourceRootAnalysisRequest {
            source_roots: vec![root_a_str.clone(), root_b_str.clone()],
        })
        .expect("source root analysis");

    let status = |root: &str| {
        result
            .items
            .iter()
            .find(|item| item.source_root == root)
            .unwrap_or_else(|| panic!("no status for {root}"))
    };
    assert_eq!(result.items.len(), 2);
    assert_eq!(
        (status(&root_a_str).total, status(&root_a_str).analyzed),
        (1, 1)
    );
    assert!(status(&root_a_str).fully_analyzed);
    assert_eq!(
        (status(&root_b_str).total, status(&root_b_str).analyzed),
        (2, 1)
    );
    assert!(!status(&root_b_str).fully_analyzed);
}

#[test]
fn check_source_roots_reports_missing_and_existing() {
    let (_dir, service) = test_service();
    let existing = tempfile::tempdir().expect("tempdir");
    let result = service
        .check_source_roots(CheckSourceRootsRequest {
            source_roots: vec![
                existing.path().to_string_lossy().to_string(),
                "/definitely/does/not/exist/anywhere".to_string(),
            ],
        })
        .expect("check source roots");
    assert_eq!(result.items.len(), 2);
    assert_eq!(
        result.missing,
        vec!["/definitely/does/not/exist/anywhere".to_string()]
    );
}

#[test]
fn export_blocking_missing_source_roots_flags_only_roots_that_hold_a_track() {
    let (_dir, service) = test_service();
    let present = tempfile::tempdir().expect("present root");
    let present_root = present.path().to_string_lossy().to_string();
    let missing_root = "/definitely/does/not/exist/anywhere".to_string();
    let missing_but_unused = "/also/not/here".to_string();

    let conn = service.db.connect().expect("connect");
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
        params![
            SETTING_UI_SOURCE_ROOTS,
            serde_json::to_string(&[
                present_root.clone(),
                missing_root.clone(),
                missing_but_unused,
            ])
            .unwrap()
        ],
    )
    .expect("seed source roots");

    // A track under the missing root blocks; one under the present root and
    // one under no configured root do not. The missing-but-unused root is
    // missing yet holds no track, so it is not reported.
    let blocking = export_blocking_missing_source_roots(
        &conn,
        [
            format!("{missing_root}/set/track.mp3"),
            format!("{present_root}/track.mp3"),
            "/somewhere/else/track.mp3".to_string(),
        ],
    )
    .expect("guard");
    assert_eq!(blocking, vec![missing_root]);

    // Every track lives under a present root -> nothing blocks.
    let clear =
        export_blocking_missing_source_roots(&conn, [format!("{present_root}/only/here.mp3")])
            .expect("guard");
    assert!(clear.is_empty());
}

#[test]
fn relocate_source_root_updates_matching_track_paths() {
    let (_dir, service) = test_service();
    let old_root = tempfile::tempdir().expect("old root");
    let new_root = tempfile::tempdir().expect("new root");
    std::fs::write(new_root.path().join("song.mp3"), b"data").expect("write new file");

    let old_path = old_root.path().join("song.mp3");
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song",
        "Artist",
        old_path.to_str().unwrap(),
        None,
        None,
        false,
    );
    drop(conn);

    let result = service
        .relocate_source_root(RelocateSourceRootRequest {
            old_root: old_root.path().to_string_lossy().to_string(),
            new_root: new_root.path().to_string_lossy().to_string(),
        })
        .expect("relocate source root");
    assert_eq!(result.matched, 1);
    assert_eq!(result.updated, 1);

    let conn = service.db.connect().expect("connect");
    let new_path: String = conn
        .query_row("SELECT file_path FROM tracks WHERE id = 't1'", [], |r| {
            r.get(0)
        })
        .expect("read updated path");
    assert_eq!(new_path, new_root.path().join("song.mp3").to_string_lossy());
}

#[test]
fn relocate_source_root_rejects_same_root() {
    let (_dir, service) = test_service();
    let err = service
        .relocate_source_root(RelocateSourceRootRequest {
            old_root: "/music".to_string(),
            new_root: "/music/".to_string(),
        })
        .unwrap_err();
    assert!(matches!(err, BackendError::Validation(_)));
}

#[test]
fn relocate_source_root_accepts_a_case_only_rename() {
    let (_dir, service) = test_service();
    let base = tempfile::tempdir().expect("base");
    let old_root = base.path().join("music").join("Sets");
    let new_root = base.path().join("Music").join("Sets");
    std::fs::create_dir_all(&new_root).expect("create new root");
    std::fs::write(new_root.join("song.mp3"), b"data").expect("write new file");

    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song",
        "Artist",
        old_root.join("song.mp3").to_str().unwrap(),
        None,
        None,
        false,
    );
    drop(conn);

    let result = service
        .relocate_source_root(RelocateSourceRootRequest {
            old_root: old_root.to_string_lossy().to_string(),
            new_root: new_root.to_string_lossy().to_string(),
        })
        .expect("a case-only rename is a real move on Linux");
    assert_eq!(result.updated, 1);

    let conn = service.db.connect().expect("connect");
    let new_path: String = conn
        .query_row("SELECT file_path FROM tracks WHERE id = 't1'", [], |r| {
            r.get(0)
        })
        .expect("read updated path");
    assert_eq!(new_path, new_root.join("song.mp3").to_string_lossy());
}

#[test]
fn materialize_source_track_inserts_new_then_updates_existing() {
    let (_dir, service) = test_service();
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("song.mp3");
    std::fs::write(&file_path, b"data").expect("write file");

    let inserted = service
        .materialize_source_track(MaterializeSourceTrackRequest {
            file_path: file_path.to_string_lossy().to_string(),
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            track_number: None,
            key: None,
            file_size_bytes: None,
            format_ext: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
        })
        .expect("materialize new track");

    let updated = service
        .materialize_source_track(MaterializeSourceTrackRequest {
            file_path: file_path.to_string_lossy().to_string(),
            title: "Song Updated".to_string(),
            artist: "Artist".to_string(),
            album: None,
            track_number: None,
            key: None,
            file_size_bytes: None,
            format_ext: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
        })
        .expect("materialize existing track");
    assert_eq!(inserted.track_id, updated.track_id);

    let conn = service.db.connect().expect("connect");
    let title: String = conn
        .query_row(
            "SELECT title FROM tracks WHERE id = ?1",
            params![updated.track_id],
            |r| r.get(0),
        )
        .expect("read title");
    assert_eq!(title, "Song Updated");
}

#[test]
fn materialize_source_track_rejects_missing_file() {
    let (_dir, service) = test_service();
    let err = service
        .materialize_source_track(MaterializeSourceTrackRequest {
            file_path: "/definitely/does/not/exist.mp3".to_string(),
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            track_number: None,
            key: None,
            file_size_bytes: None,
            format_ext: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
        })
        .unwrap_err();
    assert!(matches!(err, BackendError::NotFound(_)));
}

#[test]
fn resolve_track_identity_materializes_safe_local_source_path() {
    let (_dir, service) = test_service();
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("song.mp3");
    std::fs::write(&file_path, b"data").expect("write file");

    let result = service
        .resolve_track_identity(ResolveTrackIdentityRequest {
            track_id: Some(file_path.to_string_lossy().to_string()),
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            file_path: Some(file_path.to_string_lossy().to_string()),
            file_size_bytes: None,
            track_number: None,
            key: Some("8A".to_string()),
            format_ext: Some("mp3".to_string()),
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            usb_root: Some("/usb".to_string()),
            usb_root_valid: true,
            usb_analysis_path: None,
        })
        .expect("resolve identity");

    assert!(result.materialized);
    assert_eq!(result.resolved_by, crate::models::ResolvedBy::Materialized);
    assert!(result.track_id.is_some());
}

#[test]
fn resolve_track_identity_skips_usb_materialization_and_resolves_local_match() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "local-1",
        "Song",
        "Artist",
        "/music/song.mp3",
        None,
        Some(1234),
        false,
    );
    drop(conn);

    let result = service
        .resolve_track_identity(ResolveTrackIdentityRequest {
            track_id: Some("usb-placeholder".to_string()),
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            file_path: Some("/usb/Contents/song.mp3".to_string()),
            file_size_bytes: Some(1234),
            track_number: None,
            key: None,
            format_ext: Some("mp3".to_string()),
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            usb_root: Some("/usb".to_string()),
            usb_root_valid: true,
            usb_analysis_path: Some("/usb/PIONEER/USBANLZ/ANLZ0000.DAT".to_string()),
        })
        .expect("resolve identity");

    assert!(!result.materialized);
    assert_eq!(result.resolved_by, crate::models::ResolvedBy::Hash);
    assert_eq!(result.track_id.as_deref(), Some("local-1"));
}

#[test]
fn remove_tracks_by_source_roots_deletes_matching_prefix() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "A",
        "Artist",
        "/music/keep/a.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t2",
        "B",
        "Artist",
        "/music/drop/b.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let result = service
        .remove_tracks_by_source_roots(RemoveTracksBySourceRootsRequest {
            source_roots: vec!["/music/drop".to_string()],
        })
        .expect("remove tracks by source roots");
    assert_eq!(result.removed, 1);

    let conn = service.db.connect().expect("connect");
    let remaining: Vec<String> = conn
        .prepare("SELECT id FROM tracks")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("ids");
    assert_eq!(remaining, ["t1"]);
}

#[test]
fn remove_tracks_by_source_roots_keeps_tracks_another_root_covers() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
        params![SETTING_UI_SOURCE_ROOTS, r#"["/music","/music/sub"]"#],
    )
    .expect("seed source roots");
    for (id, path) in [
        ("top", "/music/a.mp3"),
        ("sub", "/music/sub/b.mp3"),
        ("other", "/elsewhere/c.mp3"),
    ] {
        insert_full_track(&conn, id, id, "Artist", path, None, None, false);
    }
    drop(conn);
    let remaining = || -> Vec<String> {
        let conn = service.db.connect().expect("connect");
        conn.prepare("SELECT id FROM tracks ORDER BY id")
            .expect("prepare")
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("ids")
    };
    let remove = |root: &str| {
        service
            .remove_tracks_by_source_roots(RemoveTracksBySourceRootsRequest {
                source_roots: vec![root.to_string()],
            })
            .expect("remove tracks by source roots")
            .removed
    };

    // The parent still covers the subfolder's track.
    assert_eq!(remove("/music/sub"), 0);
    assert_eq!(remaining(), ["other", "sub", "top"]);

    // The still-configured subfolder keeps its track when the parent goes.
    assert_eq!(remove("/music"), 1);
    assert_eq!(remaining(), ["other", "sub"]);
}

#[test]
fn get_tracks_by_ids_with_previews_dedupes_and_sorts() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t2",
        "B",
        "Artist",
        "/music/b.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn,
        "t1",
        "A",
        "Artist",
        "/music/a.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let result = service
        .get_tracks_by_ids_with_previews(GetTracksByIdsRequest {
            track_ids: vec!["t2".to_string(), "t1".to_string(), "t1".to_string()],
        })
        .expect("get tracks by ids");
    assert_eq!(
        result
            .items
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        vec!["t1".to_string(), "t2".to_string()]
    );
}

#[test]
fn get_tracks_by_ids_with_previews_empty_ids_returns_empty() {
    let (_dir, service) = test_service();
    let result = service
        .get_tracks_by_ids_with_previews(GetTracksByIdsRequest {
            track_ids: Vec::new(),
        })
        .expect("get tracks by ids");
    assert!(result.items.is_empty());
}

#[test]
fn playback_source_label_states_only_where_playback_comes_from() {
    assert_eq!(playback_source_label(true), "Library");
    assert_eq!(playback_source_label(false), "USB");
}

#[test]
fn recoverable_playback_error_detection_matches_backend_retry_policy() {
    assert!(is_recoverable_playback_error("Output device is busy"));
    assert!(is_recoverable_playback_error("stream is already playing"));
    assert!(is_recoverable_playback_error("sink in use"));
    assert!(!is_recoverable_playback_error("decoder error: bad frame"));
}

#[test]
fn resolve_playback_source_fast_path_by_track_id() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song",
        "Artist",
        "/music/a.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let result = service
        .resolve_playback_source(ResolvePlaybackSourceRequest {
            title: String::new(),
            artist: String::new(),
            album: None,
            bpm: None,
            file_path: None,
            file_size_bytes: None,
            track_id: Some("t1".to_string()),
        })
        .expect("resolve playback source");
    assert_eq!(result.matched_by, crate::models::TrackMatch::SelfId);
    assert_eq!(result.resolved_path.as_deref(), Some("/music/a.mp3"));
    assert_eq!(result.track_id.as_deref(), Some("t1"));
}

#[test]
fn resolve_playback_source_falls_back_to_fingerprint_hash_match() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "t1",
        "Song",
        "Artist",
        "/music/a.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let result = service
        .resolve_playback_source(ResolvePlaybackSourceRequest {
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            file_path: None,
            file_size_bytes: None,
            track_id: None,
        })
        .expect("resolve playback source");
    assert_eq!(result.matched_by, crate::models::TrackMatch::Hash);
    assert_eq!(result.track_id.as_deref(), Some("t1"));
}

#[test]
fn resolve_playback_source_fast_path_skips_row_whose_file_is_missing() {
    let (dir, service) = test_service();
    let live_path = dir.path().join("live.mp3");
    std::fs::write(&live_path, b"x").expect("write live file");
    let live_path = live_path.to_string_lossy().to_string();

    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "stale",
        "Song",
        "Artist",
        "/music/gone.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn, "live", "Song", "Artist", &live_path, None, None, false,
    );
    drop(conn);

    let result = service
        .resolve_playback_source(ResolvePlaybackSourceRequest {
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            file_path: None,
            file_size_bytes: None,
            track_id: Some("stale".to_string()),
        })
        .expect("resolve playback source");
    assert_eq!(result.resolved_path.as_deref(), Some(live_path.as_str()));
    assert_eq!(result.track_id.as_deref(), Some("live"));
}

#[test]
fn resolve_playback_source_best_candidate_prefers_file_on_disk() {
    let (dir, service) = test_service();
    let live_path = dir.path().join("live.mp3");
    std::fs::write(&live_path, b"x").expect("write live file");
    let live_path = live_path.to_string_lossy().to_string();

    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "stale",
        "Song",
        "Artist",
        "/music/gone.mp3",
        None,
        None,
        false,
    );
    insert_full_track(
        &conn, "live", "Song", "Artist", &live_path, None, None, false,
    );
    drop(conn);

    let result = service
        .resolve_playback_source(ResolvePlaybackSourceRequest {
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            file_path: None,
            file_size_bytes: None,
            track_id: None,
        })
        .expect("resolve playback source");
    assert_eq!(result.resolved_path.as_deref(), Some(live_path.as_str()));
    assert_eq!(result.track_id.as_deref(), Some("live"));
}

fn song_request(file_path: Option<&str>) -> ResolvePlaybackSourceRequest {
    ResolvePlaybackSourceRequest {
        title: "Song".to_string(),
        artist: "Artist".to_string(),
        album: None,
        bpm: None,
        file_path: file_path.map(str::to_string),
        file_size_bytes: None,
        track_id: None,
    }
}

#[test]
fn resolve_playback_source_plays_own_local_file_over_stale_export_row() {
    // A library file not indexed yet (e.g. its row was removed) must play
    // itself, not an old export row that shares its title and artist.
    let (dir, service) = test_service();
    let own = dir.path().join("Song.mp3");
    std::fs::write(&own, b"x").expect("write own file");
    let own = own.to_string_lossy().to_string();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "stale-export",
        "Song",
        "Artist",
        "/gone/USB_OLD/Contents/Artist/Song.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let result = service
        .resolve_playback_source(song_request(Some(&own)))
        .expect("resolve playback source");
    assert_eq!(result.matched_by, crate::models::TrackMatch::OwnFile);
    assert_eq!(result.resolved_path.as_deref(), Some(own.as_str()));
    assert_eq!(result.track_id, None);

    let conn = service.db.connect().expect("connect");
    insert_full_track(&conn, "own", "Song", "Artist", &own, None, None, false);
    drop(conn);
    let result = service
        .resolve_playback_source(song_request(Some(&own)))
        .expect("resolve playback source");
    assert_eq!(result.matched_by, crate::models::TrackMatch::SelfId);
    assert_eq!(result.track_id.as_deref(), Some("own"));
}

#[test]
fn unregistered_export_stick_counts_as_usb() {
    let (dir, service) = test_service();
    let stick = dir.path().join("STICK");
    std::fs::create_dir_all(stick.join("PIONEER")).expect("vendor dir");
    std::fs::create_dir_all(stick.join("Contents")).expect("contents dir");
    let on_stick = stick.join("Contents").join("Song.mp3");
    std::fs::write(&on_stick, b"x").expect("write stick file");
    let on_stick = on_stick.to_string_lossy().to_string();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn, "stick", "Song", "Artist", &on_stick, None, None, false,
    );
    drop(conn);

    let result = service
        .resolve_playback_source(song_request(None))
        .expect("resolve playback source");
    assert_eq!(result.matched_by, crate::models::TrackMatch::NoMatch);

    let tracks = service
        .get_tracks_by_ids_with_previews(GetTracksByIdsRequest {
            track_ids: vec!["stick".to_string()],
        })
        .expect("get tracks by ids");
    assert!(tracks.items[0].is_usb_path);
}

#[test]
fn resolve_playback_source_falls_back_to_missing_file_row_when_nothing_on_disk() {
    // The USB fallback in play_resolved_track relies on still getting a
    // library path here even when its file is gone -- don't regress that.
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "stale",
        "Song",
        "Artist",
        "/music/gone.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    let result = service
        .resolve_playback_source(ResolvePlaybackSourceRequest {
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            file_path: None,
            file_size_bytes: None,
            track_id: None,
        })
        .expect("resolve playback source");
    assert_eq!(result.matched_by, crate::models::TrackMatch::Hash);
    assert_eq!(result.resolved_path.as_deref(), Some("/music/gone.mp3"));
}

#[test]
fn resolve_playback_source_keeps_missing_self_row_as_last_resort() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    insert_full_track(
        &conn,
        "stale",
        "Song",
        "Artist",
        "/music/gone.mp3",
        None,
        None,
        false,
    );
    drop(conn);

    // Metadata that matches nothing by fingerprint or title, so the search
    // passes come up empty and the missing-file self row is all we have.
    let result = service
        .resolve_playback_source(ResolvePlaybackSourceRequest {
            title: "Totally Different".to_string(),
            artist: "Nobody".to_string(),
            album: None,
            bpm: None,
            file_path: None,
            file_size_bytes: None,
            track_id: Some("stale".to_string()),
        })
        .expect("resolve playback source");
    assert_eq!(result.matched_by, crate::models::TrackMatch::SelfId);
    assert_eq!(result.resolved_path.as_deref(), Some("/music/gone.mp3"));
}

#[test]
fn resolve_playback_source_returns_none_for_blank_title_or_artist() {
    let (_dir, service) = test_service();
    let result = service
        .resolve_playback_source(ResolvePlaybackSourceRequest {
            title: "   ".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            file_path: None,
            file_size_bytes: None,
            track_id: None,
        })
        .expect("resolve playback source");
    assert_eq!(result.matched_by, crate::models::TrackMatch::NoMatch);
    assert!(result.resolved_path.is_none());
}
