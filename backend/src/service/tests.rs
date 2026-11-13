use super::*;

// --- build_track_match_fingerprint ---

#[test]
fn fingerprint_deterministic() {
    let fp1 = build_track_match_fingerprint("Title", "Artist", Some("Album"));
    let fp2 = build_track_match_fingerprint("Title", "Artist", Some("Album"));
    assert_eq!(fp1, fp2);
}

#[test]
fn fingerprint_case_insensitive() {
    let lower = build_track_match_fingerprint("midnight", "dj lantern", Some("harbour tapes"));
    let upper = build_track_match_fingerprint("MIDNIGHT", "DJ LANTERN", Some("HARBOUR TAPES"));
    let mixed = build_track_match_fingerprint("Midnight", "DJ Lantern", Some("Harbour Tapes"));
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

pub(super) fn sample_track(id: &str, file_path: &str) -> Track {
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
        bpm_source_text: None,
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
        transcodes: Vec::new(),
        export_file: None,
        file_source: None,
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
    assert!(matches!(err, BackendError::Internal(_)));
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
fn playlist_tracks_with_their_analysis_files_gone_need_analysis() {
    let (_dir, service) = test_service();
    let anlz = tempfile::tempdir().expect("anlz dir");
    let present = anlz.path().join("ANLZ0000.DAT");
    for ext in ["DAT", "EXT"] {
        std::fs::write(present.with_extension(ext), b"x").expect("bundle");
    }
    let gone = anlz.path().join("ANLZ0001.DAT");
    let conn = service.db.connect().expect("connect");
    for (id, waveform) in [
        ("present", present.to_string_lossy().to_string()),
        ("gone", gone.to_string_lossy().to_string()),
        // Only a stick resolves this; the export gate checks it there.
        (
            "usb-side",
            "/PIONEER/USBANLZ/P000/0000/ANLZ0000.DAT".to_string(),
        ),
    ] {
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, duration_ms, bpm, waveform_peaks_path, match_fingerprint, created_at, updated_at)
             VALUES (?1, ?1, 'Artist', ?2, 200000, 120.0, ?3, ?4, datetime('now'), datetime('now'))",
            params![id, format!("/music/{id}.mp3"), waveform, build_track_match_fingerprint(id, "Artist", None)],
        )
        .expect("insert track");
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
            track_ids: vec!["present".into(), "gone".into(), "usb-side".into()],
        })
        .expect("add tracks");

    let tracks = service
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id.clone(),
            ..Default::default()
        })
        .expect("get playlist tracks");
    assert_eq!(tracks.unanalyzed_count, 1);
    let ready: Vec<_> = tracks
        .items
        .iter()
        .map(|t| (t.id.as_str(), t.analysis_ready))
        .collect();
    assert_eq!(
        ready,
        [("present", true), ("gone", false), ("usb-side", true)]
    );
    // "Analyze Missing Tracks" picks the same track.
    let conn = service.db.connect().expect("connect");
    assert_eq!(
        service
            .collect_playlist_unanalyzed_track_ids(&conn, &playlist.playlist_id, "")
            .expect("collect"),
        ["gone"]
    );
}

#[test]
fn library_sources_read_what_older_versions_stored() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    for (key, value) in [
        (
            SETTING_UI_SOURCE_ROOTS,
            r#"["/music/a"," /music/b ","/music/a",""]"#,
        ),
        (
            SETTING_UI_SOURCE_ROOT_ENABLED,
            r#"{"/music/b":false,"/stale":true}"#,
        ),
        (SETTING_UI_MIXXX_DB_ENABLED, "1"),
    ] {
        conn.execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
            params![key, value],
        )
        .expect("seed setting");
    }

    let sources = LibrarySources::load(&conn).expect("library sources");
    assert_eq!(sources.roots, ["/music/a", "/music/b"]);
    assert!(sources.is_enabled("/music/a"));
    assert!(!sources.is_enabled("/music/b"));
    assert!(sources.mixxx_db && !sources.rekordbox_db);
    assert!(sources.ever_configured());
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
fn playback_names_the_track_as_artist_and_title() {
    use super::playback::playback_track_name;
    assert_eq!(playback_track_name(" Artist ", "Title"), "Artist - Title");
    assert_eq!(playback_track_name("", "Title"), "Title");
    assert_eq!(playback_track_name("Artist", " "), "Artist - Unknown Title");
}

#[test]
fn recoverable_playback_error_detection_matches_backend_retry_policy() {
    assert!(is_recoverable_playback_error("Output device is busy"));
    assert!(is_recoverable_playback_error("stream is already playing"));
    assert!(is_recoverable_playback_error("sink in use"));
    assert!(!is_recoverable_playback_error("decoder error: bad frame"));
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

// The track tables show these as sent: the BPM pill's tooltip, and a title
// and artist for a file that has none.
#[test]
fn track_rows_carry_the_bpm_source_and_unknown_title_texts() {
    let (_dir, service) = test_service();
    let conn = service.db.connect().expect("connect");
    for (id, title, analyzer) in [
        ("user", "", Some("user")),
        ("mixxx", "Song", Some("mixxx")),
        ("rekordbox", "Song", Some("rekordbox")),
        ("analyzed", "Song", Some("stratum-v2")),
        ("none", "Song", None),
    ] {
        insert_full_track(
            &conn,
            id,
            title,
            "",
            &format!("/music/{id}.mp3"),
            None,
            None,
            false,
        );
        conn.execute(
            "UPDATE tracks SET bpm_analyzer = ?1 WHERE id = ?2",
            params![analyzer, id],
        )
        .expect("set bpm analyzer");
    }
    let ids: Vec<String> = ["user", "mixxx", "rekordbox", "analyzed", "none"]
        .map(String::from)
        .to_vec();
    let rows = super::library::track_rows(&conn, &ids).expect("track rows");
    let texts: Vec<(&str, Option<&str>)> = rows
        .iter()
        .map(|track| (track.id.as_str(), track.bpm_source_text.as_deref()))
        .collect();
    assert_eq!(
        texts,
        [
            ("analyzed", Some("Analyzed with: stratum-v2")),
            ("mixxx", Some("From Mixxx")),
            ("none", None),
            ("rekordbox", Some("From rekordbox")),
            ("user", Some("Manually set")),
        ]
    );
    let untitled = rows.iter().find(|track| track.id == "user").expect("row");
    assert_eq!(
        (untitled.title.as_str(), untitled.artist.as_str()),
        ("Unknown Title", "Unknown Artist")
    );
}
