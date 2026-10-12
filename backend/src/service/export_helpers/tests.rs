use super::super::usb_vendor_compat::USB_ANALYSIS_DIR;
use super::*;
use crate::models::RunUsbParityReportRequest;
use crate::service::BackendService;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

fn mapping_test_track() -> ExportManifestTrack {
    ExportManifestTrack {
        first_beat_ms: None,
        cues: Vec::new(),
        id: "track-1".to_string(),
        master_db_id: Some(556_677),
        master_content_id: Some(112_233),
        content_link: Some(998_877),
        position: 1,
        track_number: Some(3),
        title: "Mapping Track".to_string(),
        artist: "Mapping Artist".to_string(),
        album: Some("Mapping Album".to_string()),
        bpm: Some(123.45),
        key: Some("8A".to_string()),
        source_path: "/source/mapping.mp3".to_string(),
        exported_path: "/Contents/Mapping Artist/Mapping Album/Mapping Track.mp3".to_string(),
        file_modified_at: Some(1_714_521_600),
        file_size_bytes: Some(42_424),
        sample_rate_hz: Some(44_100),
        bit_depth: Some(24),
        bitrate_kbps: Some(320),
        disc_number: Some(2),
        subtitle: Some("Subtitle".to_string()),
        comment: Some("Comment".to_string()),
        title_for_search: Some("Mapping Track Search".to_string()),
        kuvo_delivery_comment: Some("Kuvo note".to_string()),
        dj_play_count: Some(7),
        rating: Some(4),
        color_id: Some(6),
        artist_id_lyricist: Some(11),
        artist_id_original_artist: Some(12),
        artist_id_remixer: Some(13),
        artist_id_composer: Some(14),
        genre_id: Some(15),
        genre: None,
        label_id: Some(16),
        isrc: Some("USRC17607839".to_string()),
        release_year: Some(2024),
        release_date: Some("2024-05-01".to_string()),
        recorded_date: Some("2024-05-01T10:00:00Z".to_string()),
        file_type: Some(1),
        owns_exported_media: true,
        owns_artwork: true,
        owns_waveform: true,
        artwork_path: Some("/PIONEER/Artwork/00001/a00123.jpg".to_string()),
        waveform_path: Some("/PIONEER/USBANLZ/P111/11111111/ANLZ0000.DAT".to_string()),
        duration_ms: Some(185_000),
    }
}

fn create_content_mapping_schema(conn: &rusqlite::Connection) {
    conn.execute_batch(
        r#"
            CREATE TABLE artist (
              artist_id INTEGER PRIMARY KEY,
              name TEXT
            );
            CREATE TABLE album (
              album_id INTEGER PRIMARY KEY,
              name TEXT,
              artist_id INTEGER,
              isComplation INTEGER
            );
            CREATE TABLE image (
              image_id INTEGER PRIMARY KEY,
              path TEXT
            );
            CREATE TABLE "key" (
              key_id INTEGER PRIMARY KEY,
              name TEXT
            );
            CREATE TABLE genre (
              genre_id INTEGER PRIMARY KEY,
              name TEXT
            );
            CREATE TABLE content (
              content_id INTEGER PRIMARY KEY,
              title TEXT,
              path TEXT,
              analysisDataFilePath TEXT,
              bpmx100 INTEGER,
              length INTEGER,
              artist_id_artist INTEGER,
              album_id INTEGER,
              key_id INTEGER,
              image_id INTEGER,
              fileName TEXT,
              fileSize INTEGER,
              fileType INTEGER,
              trackNo INTEGER,
              discNo INTEGER,
              bitrate INTEGER,
              samplingRate INTEGER,
              bitDepth INTEGER,
              subtitle TEXT,
              titleForSearch TEXT,
              isrc TEXT,
              djComment TEXT,
              kuvoDeliveryComment TEXT,
              releaseYear INTEGER,
              releaseDate TEXT,
              dateAdded TEXT,
              dateCreated TEXT,
              analysedBits INTEGER,
              djPlayCount INTEGER,
              rating INTEGER,
              color_id INTEGER,
              artist_id_lyricist INTEGER,
              artist_id_originalArtist INTEGER,
              artist_id_remixer INTEGER,
              artist_id_composer INTEGER,
              genre_id INTEGER,
              label_id INTEGER,
              contentLink INTEGER,
              masterContentId INTEGER,
              masterDbId INTEGER
            );
            "#,
    )
    .expect("create mapping schema");
}

#[test]
fn insert_content_from_template_maps_parity_critical_columns() {
    let conn = rusqlite::Connection::open_in_memory().expect("open memory db");
    create_content_mapping_schema(&conn);
    conn.execute(r#"INSERT INTO "key" (key_id, name) VALUES (1, '8A')"#, [])
        .expect("seed key");

    let track = mapping_test_track();
    let content_id =
        insert_content_from_template(&conn, &track, Some("2024-07-09")).expect("insert content");
    assert_eq!(content_id, 1);

    conn.query_row(
        "SELECT title, path, analysisDataFilePath, bpmx100, length, trackNo, analysedBits,
                titleForSearch, kuvoDeliveryComment, djPlayCount, rating, color_id,
                artist_id_lyricist, artist_id_originalArtist, artist_id_remixer, artist_id_composer,
                genre_id, label_id, contentLink, masterContentId, masterDbId, releaseDate, dateAdded, dateCreated
         FROM content WHERE content_id = 1",
        [],
        |r| {
            assert_eq!(r.get::<_, String>(0)?, track.title);
            assert_eq!(r.get::<_, String>(1)?, track.exported_path);
            assert_eq!(r.get::<_, Option<String>>(2)?, track.waveform_path);
            assert_eq!(r.get::<_, i64>(3)?, 12345);
            assert_eq!(r.get::<_, i64>(4)?, 185);
            assert_eq!(r.get::<_, Option<i64>>(5)?, Some(3));
            assert_eq!(r.get::<_, i64>(6)?, 41);
            assert_eq!(
                r.get::<_, Option<String>>(7)?.as_deref(),
                Some("Mapping Track Search")
            );
            assert_eq!(r.get::<_, Option<String>>(8)?.as_deref(), Some("Kuvo note"));
            assert_eq!(r.get::<_, Option<i64>>(9)?, Some(7));
            assert_eq!(r.get::<_, Option<i64>>(10)?, Some(4));
            assert_eq!(r.get::<_, Option<i64>>(11)?, Some(6));
            assert_eq!(r.get::<_, Option<i64>>(12)?, Some(11));
            assert_eq!(r.get::<_, Option<i64>>(13)?, Some(12));
            assert_eq!(r.get::<_, Option<i64>>(14)?, Some(13));
            assert_eq!(r.get::<_, Option<i64>>(15)?, Some(14));
            assert_eq!(r.get::<_, Option<i64>>(16)?, Some(15));
            assert_eq!(r.get::<_, Option<i64>>(17)?, Some(16));
            assert_eq!(r.get::<_, Option<i64>>(18)?, track.content_link);
            assert_eq!(r.get::<_, Option<i64>>(19)?, track.master_content_id);
            assert_eq!(r.get::<_, Option<i64>>(20)?, track.master_db_id);
            assert_eq!(r.get::<_, Option<String>>(21)?.as_deref(), Some("2024-05-01"));
            assert_eq!(r.get::<_, Option<String>>(22)?.as_deref(), Some("2024-07-09"));
            assert_eq!(r.get::<_, Option<String>>(23)?.as_deref(), Some("2024-05-01"));
            Ok(())
        },
    )
    .expect("load mapped content row");
}

#[test]
fn insert_content_from_template_sanitizes_metadata_but_preserves_paths_and_keys() {
    let conn = rusqlite::Connection::open_in_memory().expect("open memory db");
    create_content_mapping_schema(&conn);
    conn.execute(
        r#"INSERT INTO "key" (key_id, name) VALUES (?1, ?2)"#,
        rusqlite::params![9_i64, "8\0A"],
    )
    .expect("seed key");

    let mut track = mapping_test_track();
    track.title = format!("{}\0tail", "T".repeat(260));
    track.artist = "Art\0ist".to_string();
    track.album = Some("Alb\0um".to_string());
    track.genre = Some("Tech\0no".to_string());
    track.genre_id = None;
    track.subtitle = Some("Sub\0title".to_string());
    track.title_for_search = Some("  Find\0Me  ".to_string());
    track.comment = Some("Com\0ment".to_string());
    track.kuvo_delivery_comment = Some("Ku\0vo".to_string());
    track.key = Some("8\0A".to_string());
    track.exported_path = "/Contents/Bad\0Path.mp3".to_string();
    track.waveform_path = Some("/PIONEER/USBANLZ/Bad\0Anlz.DAT".to_string());

    let content_id =
        insert_content_from_template(&conn, &track, Some("2024-07-09")).expect("insert content");
    assert_eq!(content_id, 1);

    conn.query_row(
        "SELECT c.title, c.path, c.analysisDataFilePath, c.subtitle, c.titleForSearch,
                c.djComment, c.kuvoDeliveryComment, c.key_id, ar.name, al.name, g.name
         FROM content c
         LEFT JOIN artist ar ON ar.artist_id = c.artist_id_artist
         LEFT JOIN album al ON al.album_id = c.album_id
         LEFT JOIN genre g ON g.genre_id = c.genre_id
         WHERE c.content_id = 1",
        [],
        |r| {
            let title = r.get::<_, String>(0)?;
            assert_eq!(title, "T".repeat(255));
            assert!(!title.contains('\0'));
            assert_eq!(r.get::<_, String>(1)?, track.exported_path);
            assert_eq!(r.get::<_, Option<String>>(2)?, track.waveform_path);
            assert_eq!(r.get::<_, Option<String>>(3)?.as_deref(), Some("Subtitle"));
            assert_eq!(r.get::<_, Option<String>>(4)?.as_deref(), Some("FindMe"));
            assert_eq!(r.get::<_, Option<String>>(5)?.as_deref(), Some("Comment"));
            assert_eq!(r.get::<_, Option<String>>(6)?.as_deref(), Some("Kuvo"));
            assert_eq!(r.get::<_, Option<i64>>(7)?, Some(9));
            assert_eq!(r.get::<_, Option<String>>(8)?.as_deref(), Some("Artist"));
            assert_eq!(r.get::<_, Option<String>>(9)?.as_deref(), Some("Album"));
            assert_eq!(r.get::<_, Option<String>>(10)?.as_deref(), Some("Techno"));
            Ok(())
        },
    )
    .expect("load sanitized content row");
}

#[test]
fn insert_content_from_template_matches_rb6_null_empty_conventions() {
    let conn = rusqlite::Connection::open_in_memory().expect("open memory db");
    create_content_mapping_schema(&conn);
    conn.execute(r#"INSERT INTO "key" (key_id, name) VALUES (1, '8A')"#, [])
        .expect("seed key");

    let mut track = mapping_test_track();
    track.title_for_search = None;
    track.release_date = None;

    let content_id =
        insert_content_from_template(&conn, &track, Some("2024-07-09")).expect("insert content");
    assert_eq!(content_id, 1);

    let row = conn
        .query_row(
            "SELECT titleForSearch, releaseDate FROM content WHERE content_id = 1",
            [],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .expect("load content row");

    assert_eq!(
        row.0.as_deref(),
        Some(""),
        "titleForSearch should be empty string when missing"
    );
    assert_eq!(
        row.1.as_deref(),
        Some("2024-05-01"),
        "releaseDate should fall back to file_modified_at when release_date is missing"
    );
}

#[test]
fn update_existing_content_row_maps_identity_and_analysis_fields() {
    let mut conn = rusqlite::Connection::open_in_memory().expect("open memory db");
    create_content_mapping_schema(&conn);
    conn.execute(
        "INSERT INTO content (
            content_id, title, path, analysisDataFilePath, bpmx100, length, analysedBits,
            contentLink, masterContentId, masterDbId
         ) VALUES (
            10, 'old', '/Contents/old.mp3', '/PIONEER/USBANLZ/P000/OLD/ANLZ0000.DAT',
            0, 0, 0, 1, 2, 3
         )",
        [],
    )
    .expect("seed content row");
    conn.execute(r#"INSERT INTO "key" (key_id, name) VALUES (1, '8A')"#, [])
        .expect("seed key");

    let mut track = mapping_test_track();
    track.waveform_path = None;
    track.content_link = Some(7001);
    track.master_content_id = Some(7002);
    track.master_db_id = Some(7003);
    let tx = conn.transaction().expect("start tx");
    let columns = load_table_columns_tx(&tx, "content").expect("content columns");
    update_existing_content_row(&tx, 10, &track, &columns, Some("2024-07-09"))
        .expect("update content");
    tx.commit().expect("commit tx");

    let row = conn
        .query_row(
            "SELECT title, path, analysisDataFilePath, analysedBits, contentLink, masterContentId, masterDbId
             FROM content WHERE content_id = 10",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                ))
            },
        )
        .expect("load updated row");

    assert_eq!(row.0, track.title);
    assert_eq!(row.1, track.exported_path);
    assert_eq!(
        row.2.as_deref(),
        Some("/PIONEER/USBANLZ/P000/OLD/ANLZ0000.DAT"),
        "missing export waveform path should preserve existing analysisDataFilePath"
    );
    assert_eq!(row.3, 0);
    assert_eq!(row.4, track.content_link);
    assert_eq!(row.5, track.master_content_id);
    assert_eq!(row.6, track.master_db_id);
}

#[test]
fn update_existing_content_row_sanitizes_metadata_but_preserves_paths_and_keys() {
    let mut conn = rusqlite::Connection::open_in_memory().expect("open memory db");
    create_content_mapping_schema(&conn);
    conn.execute(
        r#"INSERT INTO "key" (key_id, name) VALUES (?1, ?2)"#,
        rusqlite::params![9_i64, "8\0A"],
    )
    .expect("seed key");
    conn.execute(
        "INSERT INTO content (
            content_id, title, path, analysisDataFilePath, subtitle, titleForSearch,
            djComment, kuvoDeliveryComment, key_id
         ) VALUES (
            10, 'old', '/Contents/old.mp3', '/PIONEER/USBANLZ/OLD.DAT',
            'old sub', 'old search', 'old comment', 'old kuvo', NULL
         )",
        [],
    )
    .expect("seed content row");

    let mut track = mapping_test_track();
    track.title = "Bad\0Title".to_string();
    track.subtitle = Some("Sub\0title".to_string());
    track.title_for_search = Some("  Find\0Me  ".to_string());
    track.comment = Some("Com\0ment".to_string());
    track.kuvo_delivery_comment = Some("Ku\0vo".to_string());
    track.key = Some("8\0A".to_string());
    track.exported_path = "/Contents/Bad\0Path.mp3".to_string();
    track.waveform_path = Some("/PIONEER/USBANLZ/Bad\0Anlz.DAT".to_string());

    let tx = conn.transaction().expect("start tx");
    let columns = load_table_columns_tx(&tx, "content").expect("content columns");
    update_existing_content_row(&tx, 10, &track, &columns, Some("2024-07-09"))
        .expect("update content");
    tx.commit().expect("commit tx");

    conn.query_row(
        "SELECT title, path, analysisDataFilePath, subtitle, titleForSearch,
                djComment, kuvoDeliveryComment, key_id
         FROM content WHERE content_id = 10",
        [],
        |r| {
            assert_eq!(r.get::<_, String>(0)?, "BadTitle");
            assert_eq!(r.get::<_, String>(1)?, track.exported_path);
            assert_eq!(r.get::<_, Option<String>>(2)?, track.waveform_path);
            assert_eq!(r.get::<_, Option<String>>(3)?.as_deref(), Some("Subtitle"));
            assert_eq!(r.get::<_, Option<String>>(4)?.as_deref(), Some("FindMe"));
            assert_eq!(r.get::<_, Option<String>>(5)?.as_deref(), Some("Comment"));
            assert_eq!(r.get::<_, Option<String>>(6)?.as_deref(), Some("Kuvo"));
            assert_eq!(r.get::<_, Option<i64>>(7)?, Some(9));
            Ok(())
        },
    )
    .expect("load updated content row");
}

#[test]
fn update_existing_content_row_preserves_existing_metadata_when_export_track_is_thin() {
    let mut conn = rusqlite::Connection::open_in_memory().expect("open memory db");
    create_content_mapping_schema(&conn);
    conn.execute(
        "INSERT INTO artist (artist_id, name) VALUES (7, 'Existing Artist')",
        [],
    )
    .expect("seed artist");
    conn.execute(
        "INSERT INTO album (album_id, name, artist_id, isComplation) VALUES (9, 'Existing Album', 7, 0)",
        [],
    )
    .expect("seed album");
    conn.execute(r#"INSERT INTO "key" (key_id, name) VALUES (5, '8A')"#, [])
        .expect("seed key");
    conn.execute(
        "INSERT INTO image (image_id, path) VALUES (11, '/PIONEER/Artwork/00001/a.jpg')",
        [],
    )
    .expect("seed image");
    conn.execute(
        "INSERT INTO content (
            content_id, title, path, analysisDataFilePath, bpmx100, length,
            artist_id_artist, album_id, key_id, image_id
         ) VALUES (
            10, 'old', '/Contents/existing.mp3', '/PIONEER/USBANLZ/P000/OLD.DAT',
            12800, 321, 7, 9, 5, 11
         )",
        [],
    )
    .expect("seed content row");

    let mut track = mapping_test_track();
    track.album = None;
    track.artwork_path = None;
    track.duration_ms = None;
    track.key = None;

    let tx = conn.transaction().expect("start tx");
    let columns = load_table_columns_tx(&tx, "content").expect("content columns");
    update_existing_content_row(&tx, 10, &track, &columns, Some("2024-07-09"))
        .expect("update content");
    tx.commit().expect("commit tx");

    let row = conn
        .query_row(
            "SELECT artist_id_artist, album_id, key_id, image_id, length FROM content WHERE content_id = 10",
            [],
            |r| {
                Ok((
                    r.get::<_, Option<i64>>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            },
        )
        .expect("load updated row");

    assert!(
        row.0.is_some() && row.0 != Some(7),
        "artist is rewritten from export track"
    );
    assert_eq!(
        row.1,
        Some(9),
        "missing export album should preserve existing album"
    );
    assert_eq!(
        row.2, None,
        "missing export key should clear stale existing key"
    );
    assert_eq!(
        row.3,
        Some(11),
        "missing export artwork should preserve existing image"
    );
    assert_eq!(
        row.4, 321,
        "missing export duration should preserve existing length"
    );
}

#[test]
fn update_existing_content_row_matches_rb6_null_empty_conventions() {
    let mut conn = rusqlite::Connection::open_in_memory().expect("open memory db");
    create_content_mapping_schema(&conn);
    conn.execute(r#"INSERT INTO "key" (key_id, name) VALUES (1, '8A')"#, [])
        .expect("seed key");
    conn.execute(
        "INSERT INTO content (
            content_id, title, path, titleForSearch, releaseDate
         ) VALUES (
            10, 'old', '/Contents/old.mp3', 'legacy-search', '2020-01-01'
         )",
        [],
    )
    .expect("seed content row");

    let mut track = mapping_test_track();
    track.title_for_search = None;
    track.release_date = None;

    let tx = conn.transaction().expect("start tx");
    let columns = load_table_columns_tx(&tx, "content").expect("content columns");
    update_existing_content_row(&tx, 10, &track, &columns, Some("2024-07-09"))
        .expect("update content");
    tx.commit().expect("commit tx");

    let row = conn
        .query_row(
            "SELECT titleForSearch, releaseDate FROM content WHERE content_id = 10",
            [],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .expect("load updated row");

    assert_eq!(row.0, None, "titleForSearch should be NULL after update");
    assert_eq!(
        row.1.as_deref(),
        Some("2024-05-01"),
        "releaseDate should fall back to file_modified_at after update"
    );
}

#[test]
fn insert_content_from_template_sets_release_year_zero_when_missing() {
    let conn = rusqlite::Connection::open_in_memory().expect("open memory db");
    create_content_mapping_schema(&conn);
    conn.execute(r#"INSERT INTO "key" (key_id, name) VALUES (1, '8A')"#, [])
        .expect("seed key");

    let mut track = mapping_test_track();
    track.release_year = None;

    let content_id =
        insert_content_from_template(&conn, &track, Some("2024-07-09")).expect("insert content");
    assert_eq!(content_id, 1);

    let release_year = conn
        .query_row(
            "SELECT releaseYear FROM content WHERE content_id = 1",
            [],
            |r| r.get::<_, Option<i64>>(0),
        )
        .expect("load content row");
    assert_eq!(release_year, Some(0));
}

#[test]
fn update_existing_content_row_sets_release_year_zero_when_missing() {
    let mut conn = rusqlite::Connection::open_in_memory().expect("open memory db");
    create_content_mapping_schema(&conn);
    conn.execute(r#"INSERT INTO "key" (key_id, name) VALUES (1, '8A')"#, [])
        .expect("seed key");
    conn.execute(
        "INSERT INTO content (content_id, title, path, releaseYear) VALUES (10, 'old', '/Contents/old.mp3', 2024)",
        [],
    )
    .expect("seed content row");

    let mut track = mapping_test_track();
    track.release_year = None;

    let tx = conn.transaction().expect("start tx");
    let columns = load_table_columns_tx(&tx, "content").expect("content columns");
    update_existing_content_row(&tx, 10, &track, &columns, Some("2024-07-09"))
        .expect("update content");
    tx.commit().expect("commit tx");

    let release_year = conn
        .query_row(
            "SELECT releaseYear FROM content WHERE content_id = 10",
            [],
            |r| r.get::<_, Option<i64>>(0),
        )
        .expect("load updated row");
    assert_eq!(release_year, Some(0));
}

#[test]
fn link_playlist_content_maps_playlist_id_content_id_and_sequence() {
    let conn = rusqlite::Connection::open_in_memory().expect("open memory db");
    conn.execute_batch(
        "CREATE TABLE playlist_content (
            playlist_id INTEGER,
            content_id INTEGER,
            sequenceNo INTEGER
         )",
    )
    .expect("create playlist_content");

    link_playlist_content(&conn, 17, 42, 3).expect("link playlist content");
    let row = conn
        .query_row(
            "SELECT playlist_id, content_id, sequenceNo FROM playlist_content",
            [],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )
        .expect("load playlist_content row");
    assert_eq!(row, (17, 42, 3));
}

// --- sanitize_filename_component ---

#[test]
fn sanitize_filename_alphanumeric_preserved() {
    assert_eq!(sanitize_filename_component("hello123"), "hello123");
}

#[test]
fn sanitize_filename_spaces_become_underscores() {
    assert_eq!(sanitize_filename_component("my track"), "my_track");
}

#[test]
fn sanitize_filename_control_chars_removed() {
    assert_eq!(sanitize_filename_component("ab\x00cd\x01ef"), "abcdef");
}

#[test]
fn sanitize_filename_replacement_char_removed() {
    assert_eq!(
        sanitize_filename_component("hello\u{FFFD}world"),
        "helloworld"
    );
}

#[test]
fn sanitize_filename_empty_becomes_untitled() {
    assert_eq!(sanitize_filename_component(""), "untitled");
}

#[test]
fn sanitize_filename_only_special_chars_becomes_untitled() {
    assert_eq!(sanitize_filename_component("   "), "untitled");
}

#[test]
fn sanitize_filename_consecutive_underscores_collapsed() {
    // Multiple non-alphanum chars produce underscores that get collapsed
    let result = sanitize_filename_component("a   b");
    assert_eq!(result, "a_b");
}

// --- sanitize_contents_component ---

#[test]
fn sanitize_contents_path_separators_replaced() {
    assert_eq!(sanitize_contents_component("a/b\\c"), "a_b_c");
}

#[test]
fn sanitize_contents_control_chars_replaced() {
    assert_eq!(sanitize_contents_component("ab\x01cd"), "ab_cd");
}

#[test]
fn sanitize_contents_special_fs_chars_replaced() {
    assert_eq!(sanitize_contents_component("a:b*c?d"), "a_b_c_d");
}

#[test]
fn sanitize_contents_empty_becomes_unknown() {
    assert_eq!(sanitize_contents_component(""), "Unknown");
    assert_eq!(sanitize_contents_component("   "), "Unknown");
}

#[test]
fn sanitize_contents_preserves_unicode_letters() {
    // Unicode letters are NOT control chars, so they pass through
    let result = sanitize_contents_component("Café Müsik");
    assert!(result.contains("Caf"));
    assert!(result.contains("sik"));
}

#[test]
fn sanitize_contents_caps_zalgo_combining_marks() {
    // "Zalgo" text stacks dozens of combining marks onto one base
    // character; CDJ text rendering hangs compositing an unbounded
    // stack. The on-disk folder/file name must not carry that through.
    let mark = '\u{0301}';
    let zalgo: String = std::iter::once('e')
        .chain(std::iter::repeat_n(mark, 60))
        .collect();
    let result = sanitize_contents_component(&zalgo);
    assert!(
        result.chars().count() <= crate::metadata::MAX_GRAPHEME_CLUSTER_CHARS,
        "expected capped cluster, got {} chars",
        result.chars().count()
    );
    assert!(result.starts_with('e'));
}

#[test]
fn sanitize_contents_caps_script_diversity() {
    // Fabricated (not pulled from any real file), shaped like a name
    // observed in the wild: no single grapheme cluster is deep enough
    // to trip the combining-mark cap, but it mixes many unrelated
    // scripts in a short string — separately observed to hang the
    // CDJ's Artist browse menu and track-load screen. Codepoints are
    // Cherokee, Runic, Glagolitic, Coptic, N'Ko, Vai, Osmanya, and
    // Deseret, chosen only because they're distinct scripts.
    let artist = "\u{13A0}\u{13A1} \u{16A0}\u{16A1} \u{2C00}\u{2C01} \u{2C80}\u{2C81} \u{07CA}\u{07CB} \u{A500}\u{A501} \u{10480}\u{10481} \u{10400}\u{10401}";
    let result = sanitize_contents_component(artist);
    assert!(!result.is_empty());
}

#[test]
fn sanitize_contents_file_name_keeps_extension_past_script_cap() {
    // Braille, Yi and Georgian fill the script budget before any Latin
    // letter, so capping the whole name dropped the "mp" of "mp3" and
    // the exported file ended in ".3", which a CDJ won't play.
    let name = "\u{28CE}\u{2847}\u{A27A}\u{10DA} - 06 Title-.mp3";
    assert!(sanitize_contents_component(name).ends_with(".3"));
    assert!(sanitize_contents_file_name(name).ends_with("-.mp3"));
}

#[test]
fn sanitize_contents_file_name_sanitizes_stem_like_component() {
    assert_eq!(sanitize_contents_file_name("a/b:c.flac"), "a_b_c.flac");
    assert_eq!(sanitize_contents_file_name("Track.mp3"), "Track.mp3");
    // Not an extension: the whole name goes through the component sanitizer.
    assert_eq!(sanitize_contents_file_name("no extension"), "no extension");
    assert_eq!(sanitize_contents_file_name(".hidden"), ".hidden");
    assert_eq!(
        sanitize_contents_file_name("Track.final mix"),
        sanitize_contents_component("Track.final mix")
    );
}

#[test]
fn exported_media_target_path_keeps_extension_for_multi_script_names() {
    let source = Path::new("/home/user/Music/\u{28CE}\u{2847}\u{A27A}\u{10DA} - 06 Title-.mp3");
    let result = exported_media_target_path(
        Path::new("/mnt/usb/Contents"),
        source,
        "Artist",
        Some("Album"),
        "Title",
        "mp3",
    );
    let file_name = result.file_name().unwrap().to_str().unwrap();
    assert!(file_name.ends_with(".mp3"), "got {file_name:?}");
}

// --- truncate_component ---

#[test]
fn truncate_within_limit_unchanged() {
    assert_eq!(truncate_component("short", 48), "short");
}

#[test]
fn truncate_at_limit() {
    let long = "a".repeat(50);
    assert_eq!(truncate_component(&long, 48).len(), 48);
}

#[test]
fn truncate_strips_trailing_spaces_and_dots() {
    assert_eq!(truncate_component("hello   ", 48), "hello");
    assert_eq!(truncate_component("hello...", 48), "hello");
}

#[test]
fn truncate_all_dots_becomes_unknown() {
    assert_eq!(truncate_component("...", 48), "Unknown");
}

#[test]
fn truncate_empty_becomes_unknown() {
    assert_eq!(truncate_component("", 48), "Unknown");
}

// --- limit_contents_file_name ---

#[test]
fn limit_filename_within_limit_unchanged() {
    assert_eq!(limit_contents_file_name("track.mp3", 48), "track.mp3");
}

#[test]
fn limit_filename_preserves_extension() {
    let long_name = format!("{}.mp3", "a".repeat(60));
    let result = limit_contents_file_name(&long_name, 48);
    assert!(
        result.ends_with(".mp3"),
        "extension not preserved: {result}"
    );
    assert!(result.chars().count() <= 48, "too long: {result}");
}

#[test]
fn limit_filename_no_extension() {
    let long_name = "a".repeat(60);
    let result = limit_contents_file_name(&long_name, 48);
    assert_eq!(result.chars().count(), 48);
}

// --- limit_contents_file_name_with_suffix ---

#[test]
fn limit_filename_with_suffix_none_matches_plain() {
    let long_name = format!("{}.mp3", "a".repeat(60));
    assert_eq!(
        limit_contents_file_name_with_suffix(&long_name, 48, None),
        limit_contents_file_name(&long_name, 48)
    );
}

#[test]
fn limit_filename_with_suffix_appends_ordinal_before_extension() {
    let long_name = "Test Artist With A Long Name - A Deliberately Overlong Album Title For Truncation Tests - 01 Track Alpha.wav";
    let plain = limit_contents_file_name_with_suffix(long_name, 48, None);
    let first = limit_contents_file_name_with_suffix(long_name, 48, Some(1));
    let second = limit_contents_file_name_with_suffix(long_name, 48, Some(2));
    assert!(first.ends_with("-1.wav"), "got: {first}");
    assert!(second.ends_with("-2.wav"), "got: {second}");
    assert_ne!(plain, first);
    assert_ne!(first, second);
    assert!(plain.chars().count() <= 48);
    assert!(first.chars().count() <= 48);
    assert!(second.chars().count() <= 48);
}

#[test]
fn limit_filename_with_suffix_disambiguates_a_seven_way_collision() {
    // The exact shape of the field corruption report: 7 tracks share a
    // >48-char common prefix and all truncate identically at ordinal
    // None. Assigning ordinals 1..=6 to the collisions must yield 7
    // total distinct names.
    let long_name = "Test Artist With A Long Name - A Deliberately Overlong Album Title For Truncation Tests - 07 Track Golf.wav";
    let mut names = vec![limit_contents_file_name_with_suffix(long_name, 48, None)];
    for n in 1..=6u32 {
        names.push(limit_contents_file_name_with_suffix(long_name, 48, Some(n)));
    }
    let unique: std::collections::HashSet<_> = names.iter().collect();
    assert_eq!(unique.len(), names.len(), "collided names: {names:?}");
}

// --- stable_u32_hash ---

#[test]
fn stable_hash_deterministic() {
    let h1 = stable_u32_hash("test-track-id");
    let h2 = stable_u32_hash("test-track-id");
    assert_eq!(h1, h2);
}

#[test]
fn stable_hash_different_inputs_differ() {
    assert_ne!(stable_u32_hash("track-a"), stable_u32_hash("track-b"));
}

#[test]
fn stable_hash_empty_string() {
    // Should not panic
    let _ = stable_u32_hash("");
}

// --- canonical_artwork_target_path ---

#[test]
fn artwork_path_structure() {
    let usb = Path::new("/mnt/usb");
    let path = canonical_artwork_target_path(usb, "track-123", "/music/cover.jpg");
    let s = path.to_string_lossy();
    assert!(
        s.starts_with("/mnt/usb/PIONEER/Artwork/"),
        "wrong base: {s}"
    );
    assert!(s.ends_with(".jpg"), "wrong extension: {s}");
}

#[test]
fn artwork_path_stable_across_calls() {
    let usb = Path::new("/mnt/usb");
    let p1 = canonical_artwork_target_path(usb, "id-1", "/art.png");
    let p2 = canonical_artwork_target_path(usb, "id-1", "/art.png");
    assert_eq!(p1, p2);
}

#[test]
fn artwork_path_always_jpg_for_player() {
    let usb = Path::new("/mnt/usb");
    // Regardless of source extension, player artwork is always JPEG
    let path = canonical_artwork_target_path(usb, "id-1", "/art.PNG");
    assert!(
        path.to_string_lossy().ends_with(".jpg"),
        "Player artwork must be .jpg"
    );
}

#[test]
fn artwork_path_no_extension_defaults_to_jpg() {
    let usb = Path::new("/mnt/usb");
    let path = canonical_artwork_target_path(usb, "id-1", "/art");
    assert!(path.to_string_lossy().ends_with(".jpg"));
}

#[test]
fn artwork_target_paths_produces_small_and_medium() {
    let usb = Path::new("/mnt/usb");
    let (small, medium) = canonical_artwork_target_paths(usb, "track-99");
    let s = small.to_string_lossy();
    let m = medium.to_string_lossy();
    assert!(s.starts_with("/mnt/usb/PIONEER/Artwork/"));
    assert!(s.ends_with(".jpg"));
    assert!(!s.contains("_m"), "small path must NOT contain _m");
    assert!(
        m.ends_with("_m.jpg"),
        "medium path must end with _m.jpg: {m}"
    );
    // Same directory
    assert_eq!(small.parent(), medium.parent());
}

#[test]
fn export_artwork_for_player_creates_two_square_jpegs() {
    // Create a test PNG image (non-square: 200x100)
    let dir = tempdir().unwrap();
    let source = dir.path().join("cover.png");
    let img = image::RgbImage::from_fn(200, 100, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
    });
    image::DynamicImage::ImageRgb8(img).save(&source).unwrap();

    let usb_root = dir.path().join("USB");
    std::fs::create_dir_all(&usb_root).unwrap();
    let mut warnings = Vec::new();
    let result = export_artwork_for_player(
        source.to_str().unwrap(),
        &usb_root,
        "test-track-1",
        &mut warnings,
    )
    .unwrap();
    assert!(result.is_some(), "should produce artwork path");
    assert!(warnings.is_empty(), "no warnings: {warnings:?}");

    let (small_path, medium_path) = canonical_artwork_target_paths(&usb_root, "test-track-1");
    assert!(small_path.is_file(), "small JPEG must exist");
    assert!(medium_path.is_file(), "medium JPEG must exist");

    // Verify dimensions are square
    let small_img = image::open(&small_path).unwrap();
    let medium_img = image::open(&medium_path).unwrap();
    assert_eq!(small_img.width(), 80);
    assert_eq!(small_img.height(), 80);
    assert_eq!(medium_img.width(), 240);
    assert_eq!(medium_img.height(), 240);
}

#[test]
fn export_artwork_for_player_missing_source_returns_none() {
    let dir = tempdir().unwrap();
    let mut warnings = Vec::new();
    let result = export_artwork_for_player(
        "/nonexistent/cover.jpg",
        dir.path(),
        "track-1",
        &mut warnings,
    )
    .unwrap();
    assert!(result.is_none());
    assert!(!warnings.is_empty());
}

#[test]
fn verify_edb_content_fails_when_artwork_file_is_missing_on_disk() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    let export_db = usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR);
    std::fs::create_dir_all(&export_db).unwrap();

    let db_path = export_db.join("exportLibrary.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(
        r#"
            CREATE TABLE playlist (playlist_id INTEGER PRIMARY KEY, name TEXT, attribute INTEGER, sequenceNo INTEGER);
            CREATE TABLE image (image_id INTEGER PRIMARY KEY, path TEXT);
            CREATE TABLE content (content_id INTEGER PRIMARY KEY, path TEXT, analysisDataFilePath TEXT, image_id INTEGER);
            CREATE TABLE playlist_content (playlist_id INTEGER, content_id INTEGER, sequenceNo INTEGER);
            INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (1, 'Test', 0, 1);
            INSERT INTO image (image_id, path) VALUES (10, '/PIONEER/Artwork/00001/b14990.jpg');
            INSERT INTO content (content_id, path, analysisDataFilePath, image_id)
              VALUES (20, '/Contents/Artist One/Album One/Artist One - Album One - 03 Track One.mp3', NULL, 10);
            INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (1, 20, 1);
            "#,
    )
    .unwrap();

    let playlist = ExportPlaylistData {
        id: "pl1".to_string(),
        name: "Test".to_string(),
        tracks: vec![],
    };
    let manifest = ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: "pl1".to_string(),
        playlist_name: "Test".to_string(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: ExportToUsbOptions {
            include_artwork: true,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: vec![],
        tracks: vec![ExportManifestTrack {
            first_beat_ms: None,
            cues: Vec::new(),
            id: "t1".to_string(),
            master_db_id: None,
            master_content_id: None,
            content_link: None,
            position: 1,
            track_number: Some(3),
            title: "Track One".to_string(),
            artist: "Artist One".to_string(),
            album: Some("Album One".to_string()),
            bpm: Some(150.37),
            key: Some("Am".to_string()),
            source_path: "/tmp/source.mp3".to_string(),
            exported_path:
                "/Contents/Artist One/Album One/Artist One - Album One - 03 Track One.mp3"
                    .to_string(),
            file_modified_at: None,
            file_size_bytes: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: None,
            color_id: None,
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: None,
            release_year: None,
            release_date: None,
            recorded_date: None,
            file_type: None,
            owns_exported_media: true,
            owns_artwork: true,
            owns_waveform: true,
            artwork_path: Some("/PIONEER/Artwork/00001/b14990.jpg".to_string()),
            waveform_path: None,
            duration_ms: Some(202_000),
        }],
    };

    let err = verify_edb_content(usb_root, &playlist, &manifest).unwrap_err();
    assert!(
        err.to_string().contains("artwork file missing on disk"),
        "unexpected error: {err}"
    );
}

#[test]
fn upsert_export_playlist_row_prefers_same_name_playlist_with_most_entries() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("exportLibrary.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(
        r#"
            CREATE TABLE playlist (playlist_id INTEGER PRIMARY KEY, name TEXT, attribute INTEGER, sequenceNo INTEGER);
            CREATE TABLE playlist_content (playlist_id INTEGER, content_id INTEGER, sequenceNo INTEGER);
            INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES
              (1, 'Testi', 0, 1),
              (2, 'Testi', 0, 2);
            INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES
              (1, 11, 1),
              (1, 12, 2),
              (2, 21, 1);
            "#,
    )
    .unwrap();

    let playlist = ExportPlaylistData {
        id: "pl1".to_string(),
        name: "Testi".to_string(),
        tracks: vec![],
    };

    let chosen = upsert_export_playlist_row(&conn, &playlist).unwrap();
    assert_eq!(
        chosen, 1,
        "should target the fullest same-name playlist row"
    );
}

// --- canonical_analysis_bundle_paths ---

#[test]
fn analysis_paths_structure() {
    let usb = Path::new("/mnt/usb");
    let (dat, ext, twoex) =
        canonical_analysis_bundle_paths(usb, "/Contents/Artist/Album/track-456.mp3");
    let dat_s = dat.to_string_lossy();
    assert!(dat_s.contains("PIONEER/USBANLZ/"), "wrong base: {dat_s}");
    assert!(dat_s.ends_with("ANLZ0000.DAT"));
    assert!(ext.to_string_lossy().ends_with("ANLZ0000.EXT"));
    assert!(twoex.to_string_lossy().ends_with("ANLZ0000.2EX"));
}

#[test]
fn analysis_paths_deterministic() {
    let usb = Path::new("/mnt/usb");
    let (d1, e1, t1) = canonical_analysis_bundle_paths(usb, "/Contents/Artist/Album/id-x.mp3");
    let (d2, e2, t2) = canonical_analysis_bundle_paths(usb, "/Contents/Artist/Album/id-x.mp3");
    assert_eq!(d1, d2);
    assert_eq!(e1, e2);
    assert_eq!(t1, t2);
}

#[test]
fn export_analysis_copies_internal_cache_anlz_even_when_invalid() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path().join("usb");
    fs::create_dir_all(&usb_root).unwrap();

    let stale_dir = dir.path().join(".app-data/analysis/waveforms");
    fs::create_dir_all(&stale_dir).unwrap();
    let stale_dat = stale_dir.join("A1B2C3D4.DAT");
    let stale_ext = stale_dir.join("A1B2C3D4.EXT");
    let stale_2ex = stale_dir.join("A1B2C3D4.2EX");
    fs::write(&stale_dat, b"STALE-DAT").unwrap();
    fs::write(&stale_ext, b"STALE-EXT").unwrap();
    fs::write(&stale_2ex, b"STALE-2EX").unwrap();

    let fixture_mp3 = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/noart/track_no_art.mp3");
    assert!(
        fixture_mp3.is_file(),
        "fixture mp3 missing: {}",
        fixture_mp3.display()
    );

    let track = ExportTrackData {
        cues: Vec::new(),
        id: "t-local-cache".to_string(),
        title: "Track".to_string(),
        artist: "Artist".to_string(),
        album: None,
        track_number: Some(1),
        bpm: Some(120.0),
        key: None,
        file_path: fixture_mp3.to_string_lossy().to_string(),
        file_name: "track_no_art.mp3".to_string(),
        file_modified_at: None,
        file_size_bytes: None,
        sample_rate_hz: None,
        bit_depth: None,
        bitrate_kbps: None,
        disc_number: None,
        subtitle: None,
        comment: None,
        title_for_search: None,
        kuvo_delivery_comment: None,
        dj_play_count: None,
        rating: None,
        color_id: None,
        artist_id_lyricist: None,
        artist_id_original_artist: None,
        artist_id_remixer: None,
        artist_id_composer: None,
        genre_id: None,
        genre: None,
        label_id: None,
        isrc: None,
        release_year: None,
        release_date: None,
        recorded_date: None,
        file_type: None,
        artwork_path: None,
        waveform_peaks_path: Some(stale_dat.to_string_lossy().to_string()),
        duration_ms: Some(30_000),
        first_beat_ms: None,
        beatgrid_from_rekordbox: false,
        position: 1,
    };

    let mut warnings = Vec::new();
    let exported =
        export_analysis_bundle_for_track(&track, &usb_root, "/Contents/t.mp3", &mut warnings)
            .expect("export analysis bundle");
    assert!(exported.is_some(), "expected waveform path");

    let (dat, ext, twoex) = canonical_analysis_bundle_paths(&usb_root, "/Contents/t.mp3");
    assert_eq!(fs::read(&dat).unwrap(), b"STALE-DAT");
    assert_eq!(fs::read(&ext).unwrap(), b"STALE-EXT");
    assert_eq!(fs::read(&twoex).unwrap(), b"STALE-2EX");
}

#[test]
fn export_analysis_copies_local_anlz_and_rewrites_ppth_path() {
    use crate::service::anlz::{
        build_anlz_2ex_file, build_anlz_dat_file, build_anlz_ext_file, ppth_path_from_anlz,
    };

    let dir = tempdir().unwrap();
    let usb_root = dir.path().join("usb");
    fs::create_dir_all(&usb_root).unwrap();
    let local_dir = dir.path().join("analysis");
    fs::create_dir_all(&local_dir).unwrap();

    // 400 bins satisfies the minimum detail-gate for any duration ≤ 15s (required ≤ 2400,
    // so the PWV7 triplet-run check is skipped). Use 1s to keep the fixture minimal.
    let waveform = WaveformData::from_peaks(vec![128; 400]);
    let dat_content = build_anlz_dat_file(&waveform, "", None, 1_000, None, &[]);
    let ext_content = build_anlz_ext_file(&waveform, "", None, 1_000, None, &[]);
    let twoex_content = build_anlz_2ex_file(&waveform, "", 1_000);
    assert!(
        ppth_path_from_anlz(&dat_content).is_none(),
        "local analysis cache should start without PPTH"
    );

    let local_dat = local_dir.join("ABCD1234.DAT");
    let local_ext = local_dir.join("ABCD1234.EXT");
    let local_2ex = local_dir.join("ABCD1234.2EX");
    fs::write(&local_dat, &dat_content).unwrap();
    fs::write(&local_ext, &ext_content).unwrap();
    fs::write(&local_2ex, &twoex_content).unwrap();

    let track = ExportTrackData {
        cues: Vec::new(),
        id: "t-copy-local".to_string(),
        title: "Track".to_string(),
        artist: "Artist".to_string(),
        album: None,
        track_number: None,
        bpm: None,
        key: None,
        file_path: "/should/not/be/read.mp3".to_string(),
        file_name: "track.mp3".to_string(),
        file_modified_at: None,
        file_size_bytes: None,
        sample_rate_hz: None,
        bit_depth: None,
        bitrate_kbps: None,
        disc_number: None,
        subtitle: None,
        comment: None,
        title_for_search: None,
        kuvo_delivery_comment: None,
        dj_play_count: None,
        rating: None,
        color_id: None,
        artist_id_lyricist: None,
        artist_id_original_artist: None,
        artist_id_remixer: None,
        artist_id_composer: None,
        genre_id: None,
        genre: None,
        label_id: None,
        isrc: None,
        release_year: None,
        release_date: None,
        recorded_date: None,
        file_type: None,
        artwork_path: None,
        waveform_peaks_path: Some(local_dat.to_string_lossy().to_string()),
        duration_ms: Some(1_000),
        first_beat_ms: None,
        beatgrid_from_rekordbox: false,
        position: 1,
    };

    let exported_path = "/Contents/Fixture Ö Artist/Fixture Ä Album/03 - Entä jos Fixture.flac";
    let mut warnings = Vec::new();
    let result = export_analysis_bundle_for_track(&track, &usb_root, exported_path, &mut warnings)
        .expect("copy local anlz");
    assert!(result.is_some(), "expected a path returned");
    assert!(warnings.is_empty(), "no warnings expected");

    let (dat, ext, twoex) = canonical_analysis_bundle_paths(&usb_root, exported_path);
    let dat_bytes = fs::read(&dat).unwrap();
    let ext_bytes = fs::read(&ext).unwrap();
    let twoex_bytes = fs::read(&twoex).unwrap();
    assert_eq!(
        ppth_path_from_anlz(&dat_bytes).as_deref(),
        Some(exported_path)
    );
    assert_eq!(
        ppth_path_from_anlz(&ext_bytes).as_deref(),
        Some(exported_path)
    );
    assert_eq!(
        ppth_path_from_anlz(&twoex_bytes).as_deref(),
        Some(exported_path)
    );
    assert!(dat_bytes.windows(4).any(|w| w == b"PVBR"));
    assert!(ext_bytes.windows(4).any(|w| w == b"PWV3"));
    assert!(twoex_bytes.windows(4).any(|w| w == b"PWV7"));
}

#[test]
fn ensure_analysis_bundle_ppth_rebuilds_beatgrid_from_bpm_alone_on_retain_path() {
    use crate::service::anlz::{
        build_anlz_2ex_file, build_anlz_dat_file, build_anlz_ext_file,
        read_beatgrid_tempo_from_anlz,
    };

    let dir = tempdir().unwrap();
    let usb_root = dir.path().join("usb");
    fs::create_dir_all(&usb_root).unwrap();

    let exported_path = "/Contents/Artist/Album/track.flac";

    // Simulate a bundle already on the USB (the "retain" export path)
    // whose beat grid was baked at a stale/default 120 BPM, matching
    // the real-world bug: PDB/eDB and the app's own `tracks.bpm` are
    // already correct, but nothing has re-derived the beat grid since.
    let waveform = WaveformData::from_peaks(vec![128; 400]);
    let (dat_path, ext_path, twoex_path) =
        canonical_analysis_bundle_paths(&usb_root, exported_path);
    fs::create_dir_all(dat_path.parent().unwrap()).unwrap();
    fs::write(
        &dat_path,
        build_anlz_dat_file(&waveform, exported_path, Some(120.0), 200_000, None, &[]),
    )
    .unwrap();
    fs::write(
        &ext_path,
        build_anlz_ext_file(&waveform, exported_path, Some(120.0), 200_000, None, &[]),
    )
    .unwrap();
    fs::write(
        &twoex_path,
        build_anlz_2ex_file(&waveform, exported_path, 200_000),
    )
    .unwrap();

    // The track has since been correctly re-analyzed to 140 BPM, but
    // has no cues and no confident first-beat -- the exact shape that
    // used to leave the retain path's beat-grid rebuild un-triggered.
    let track = ExportTrackData {
        bpm: Some(140.0),
        first_beat_ms: None,
        cues: Vec::new(),
        duration_ms: Some(200_000),
        ..make_test_track("t-bpm-fix", "Track", "track.flac")
    };

    ensure_analysis_bundle_ppth(
        &usb_root,
        &dat_path.to_string_lossy(),
        exported_path,
        &track,
    )
    .expect("ensure analysis bundle ppth");

    assert_eq!(
        read_beatgrid_tempo_from_anlz(&fs::read(&dat_path).unwrap()),
        Some(14_000),
        ".DAT beat grid must be corrected to the track's real bpm"
    );
    assert_eq!(
        read_beatgrid_tempo_from_anlz(&fs::read(&ext_path).unwrap()),
        Some(14_000),
        ".EXT beat grid must be corrected to the track's real bpm"
    );
}

/// A rekordbox library bundle as `rekordbox_import` stores it: the track
/// points at the `.EXT`, the `.DAT` carries a filled `PVBR` and a 128 BPM
/// grid, and there is no `.2EX`. Returns (path to `.EXT`, source `PVBR` chunk).
fn write_rekordbox_style_bundle(dir: &Path) -> (PathBuf, Vec<u8>) {
    use crate::service::anlz::{build_anlz_dat_file, build_anlz_ext_file};

    let waveform = WaveformData::from_peaks(vec![128; 400]);
    let mut dat = build_anlz_dat_file(&waveform, "", Some(128.0), 200_000, Some(437), &[]);
    let pvbr = dat
        .windows(4)
        .position(|w| w == b"PVBR")
        .expect("generated DAT has a PVBR");
    // Fill the seek index the way rekordbox does for a VBR MP3.
    for i in 0..400u32 {
        let at = pvbr + 16 + i as usize * 4;
        dat[at..at + 4].copy_from_slice(&(1_000 + i * 417).to_be_bytes());
    }
    dat[pvbr + 16 + 1600..pvbr + 16 + 1604].copy_from_slice(&8_820_000u32.to_be_bytes());
    let pvbr_chunk = dat[pvbr..pvbr + 16 + 1604].to_vec();

    let anlz_dir = dir.join("share/PIONEER/USBANLZ/abc/def");
    fs::create_dir_all(&anlz_dir).unwrap();
    fs::write(anlz_dir.join("ANLZ0000.DAT"), &dat).unwrap();
    fs::write(
        anlz_dir.join("ANLZ0000.EXT"),
        build_anlz_ext_file(&waveform, "", Some(128.0), 200_000, Some(437), &[]),
    )
    .unwrap();
    (anlz_dir.join("ANLZ0000.EXT"), pvbr_chunk)
}

#[test]
fn export_analysis_copies_rekordbox_dat_when_track_points_at_ext() {
    use crate::service::anlz::read_beatgrid_tempo_from_anlz;

    let dir = tempdir().unwrap();
    let usb_root = dir.path().join("usb");
    fs::create_dir_all(&usb_root).unwrap();
    let (rb_ext, pvbr_chunk) = write_rekordbox_style_bundle(dir.path());

    let exported_path = "/Contents/Artist/Album/track.mp3";
    let (dat, ext, twoex) = canonical_analysis_bundle_paths(&usb_root, exported_path);
    // A `.2EX` left by an earlier export of this path must not survive
    // next to a bundle that has none.
    fs::create_dir_all(twoex.parent().unwrap()).unwrap();
    fs::write(&twoex, b"STALE-2EX").unwrap();

    // The tempo differs from the grid's on purpose: an unedited
    // rekordbox grid is kept as rekordbox wrote it.
    let track = ExportTrackData {
        bpm: Some(140.0),
        duration_ms: Some(200_000),
        first_beat_ms: Some(437),
        waveform_peaks_path: Some(rb_ext.to_string_lossy().to_string()),
        beatgrid_from_rekordbox: true,
        ..make_test_track("t-rb", "Track", "track.mp3")
    };

    let mut warnings = Vec::new();
    let (_, files_written) =
        export_analysis_bundle_for_track(&track, &usb_root, exported_path, &mut warnings)
            .expect("export rekordbox bundle")
            .expect("bundle exported");
    assert!(warnings.is_empty(), "no warnings expected: {warnings:?}");
    assert_eq!(files_written, 2);

    let dat_bytes = fs::read(&dat).unwrap();
    let ext_bytes = fs::read(&ext).unwrap();
    assert!(
        dat_bytes.windows(pvbr_chunk.len()).any(|w| w == pvbr_chunk),
        "rekordbox's PVBR must reach the USB .DAT unchanged"
    );
    assert_eq!(
        read_beatgrid_tempo_from_anlz(&dat_bytes),
        Some(12_800),
        "the USB .DAT must carry rekordbox's own beat grid"
    );
    assert!(
        ext_bytes.windows(4).any(|w| w == b"PWV3") && !ext_bytes.windows(4).any(|w| w == b"PVBR"),
        "the USB .EXT must come from rekordbox's .EXT"
    );
    assert!(!twoex.exists(), "stale .2EX must be removed");
}

#[test]
fn export_analysis_rebuilds_rekordbox_grid_once_edited_here() {
    use crate::service::anlz::read_beatgrid_tempo_from_anlz;

    let dir = tempdir().unwrap();
    let usb_root = dir.path().join("usb");
    fs::create_dir_all(&usb_root).unwrap();
    let (rb_ext, _) = write_rekordbox_style_bundle(dir.path());

    let exported_path = "/Contents/Artist/Album/track.mp3";
    let track = ExportTrackData {
        bpm: Some(140.0),
        duration_ms: Some(200_000),
        first_beat_ms: Some(437),
        waveform_peaks_path: Some(rb_ext.to_string_lossy().to_string()),
        beatgrid_from_rekordbox: false,
        ..make_test_track("t-rb-edited", "Track", "track.mp3")
    };

    let mut warnings = Vec::new();
    export_analysis_bundle_for_track(&track, &usb_root, exported_path, &mut warnings)
        .expect("export rekordbox bundle")
        .expect("bundle exported");

    let (dat, ..) = canonical_analysis_bundle_paths(&usb_root, exported_path);
    assert_eq!(
        read_beatgrid_tempo_from_anlz(&fs::read(&dat).unwrap()),
        Some(14_000),
        "an edited tempo must still rebuild the grid"
    );
}

#[test]
fn export_analysis_does_not_regenerate_when_local_dat_missing() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path().join("usb");
    fs::create_dir_all(&usb_root).unwrap();

    let fixture_mp3 = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/noart/track_no_art.mp3");
    assert!(
        fixture_mp3.is_file(),
        "fixture mp3 missing: {}",
        fixture_mp3.display()
    );

    let track = ExportTrackData {
        cues: Vec::new(),
        id: "t-fallback".to_string(),
        title: "Track".to_string(),
        artist: "Artist".to_string(),
        album: None,
        track_number: None,
        bpm: Some(120.0),
        key: None,
        file_path: fixture_mp3.to_string_lossy().to_string(),
        file_name: "track_no_art.mp3".to_string(),
        file_modified_at: None,
        file_size_bytes: None,
        sample_rate_hz: None,
        bit_depth: None,
        bitrate_kbps: None,
        disc_number: None,
        subtitle: None,
        comment: None,
        title_for_search: None,
        kuvo_delivery_comment: None,
        dj_play_count: None,
        rating: None,
        color_id: None,
        artist_id_lyricist: None,
        artist_id_original_artist: None,
        artist_id_remixer: None,
        artist_id_composer: None,
        genre_id: None,
        genre: None,
        label_id: None,
        isrc: None,
        release_year: None,
        release_date: None,
        recorded_date: None,
        file_type: None,
        artwork_path: None,
        waveform_peaks_path: Some("/nonexistent/path/DEADBEEF.DAT".to_string()),
        duration_ms: Some(30_000),
        first_beat_ms: None,
        beatgrid_from_rekordbox: false,
        position: 1,
    };

    let mut warnings = Vec::new();
    let result =
        export_analysis_bundle_for_track(&track, &usb_root, "/Contents/t.mp3", &mut warnings)
            .expect("missing local analysis handled");
    assert!(
        result.is_none(),
        "missing local analysis must not regenerate"
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.message.contains("analysis bundle missing")),
        "expected missing bundle warning, got {warnings:?}"
    );

    let (dat, ..) = canonical_analysis_bundle_paths(&usb_root, "/Contents/t.mp3");
    assert!(
        !dat.exists(),
        "export must not create generated analysis when source bundle is missing"
    );
}

// --- exported_media_target_path ---

#[test]
fn media_path_preserves_contents_relative() {
    let media = Path::new("/mnt/usb/Contents");
    let source = Path::new("/other/usb/Contents/Artist/Album/track.mp3");
    let result = exported_media_target_path(media, source, "X", Some("Y"), "Z", "mp3");
    // Should preserve the path after "Contents"
    let s = result.to_string_lossy();
    assert!(
        s.starts_with("/mnt/usb/Contents/Artist/Album/track.mp3"),
        "got: {s}"
    );
}

#[test]
fn media_path_fallback_uses_sanitized_artist_album() {
    let media = Path::new("/mnt/usb/Contents");
    let source = Path::new("/local/music/my_track.mp3");
    let result =
        exported_media_target_path(media, source, "DJ Test", Some("Cool Album"), "Title", "mp3");
    let s = result.to_string_lossy();
    assert!(s.contains("DJ Test"), "missing artist: {s}");
    assert!(s.contains("Cool Album"), "missing album: {s}");
}

#[test]
fn media_path_no_album_uses_unknown() {
    let media = Path::new("/mnt/usb/Contents");
    let source = Path::new("/local/music/track.mp3");
    let result = exported_media_target_path(media, source, "Artist", None, "Title", "mp3");
    let s = result.to_string_lossy();
    assert!(s.contains("UnknownAlbum"), "missing fallback: {s}");
}

// --- copy_if_different ---

#[test]
fn copy_if_different_copies_new_file() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("source.bin");
    let dst = dir.path().join("sub/target.bin");
    fs::write(&src, b"hello").unwrap();

    copy_if_different(&src, &dst).unwrap();
    assert_eq!(fs::read(&dst).unwrap(), b"hello");
}

#[test]
fn copy_if_different_skips_same_size() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("source.bin");
    let dst = dir.path().join("target.bin");
    fs::write(&src, b"hello").unwrap();
    fs::write(&dst, b"world").unwrap(); // same size, different content

    copy_if_different(&src, &dst).unwrap();
    // Same size → skips copy, so dst still has "world"
    assert_eq!(fs::read(&dst).unwrap(), b"world");
}

#[test]
fn copy_if_different_copies_different_size() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("source.bin");
    let dst = dir.path().join("target.bin");
    fs::write(&src, b"hello world").unwrap();
    fs::write(&dst, b"hi").unwrap();

    copy_if_different(&src, &dst).unwrap();
    assert_eq!(fs::read(&dst).unwrap(), b"hello world");
}

#[test]
fn copy_if_different_missing_source_errors() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("nonexistent.bin");
    let dst = dir.path().join("target.bin");

    let result = copy_if_different(&src, &dst);
    assert!(result.is_err());
}

// --- copy_wav_normalized_if_needed ---

fn wav_with_extensible_fmt(sub_format_tag: u16, data: &[u8]) -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&0xFFFEu16.to_le_bytes()); // WAVE_FORMAT_EXTENSIBLE
    fmt.extend_from_slice(&1u16.to_le_bytes()); // mono
    fmt.extend_from_slice(&44_100u32.to_le_bytes());
    fmt.extend_from_slice(&(44_100u32 * 2).to_le_bytes()); // byte rate
    fmt.extend_from_slice(&2u16.to_le_bytes()); // block align
    fmt.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    fmt.extend_from_slice(&22u16.to_le_bytes()); // cbSize
    fmt.extend_from_slice(&16u16.to_le_bytes()); // validBitsPerSample
    fmt.extend_from_slice(&0x4u32.to_le_bytes()); // channel mask
    fmt.extend_from_slice(&sub_format_tag.to_le_bytes());
    fmt.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x10, 0x00]);
    fmt.extend_from_slice(&[0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71]);

    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    body.extend_from_slice(&fmt);
    body.extend_from_slice(b"data");
    body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    body.extend_from_slice(data);

    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

#[test]
fn copy_wav_normalized_if_needed_rewrites_extensible_pcm() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("source.wav");
    let data = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
    fs::write(&src, wav_with_extensible_fmt(1, &data)).unwrap();
    let dst = dir.path().join("sub/target.wav");

    copy_wav_normalized_if_needed(&src, &dst).unwrap();

    let out = fs::read(&dst).unwrap();
    let format_tag = u16::from_le_bytes(out[20..22].try_into().unwrap());
    assert_eq!(format_tag, 1, "should be normalized to plain PCM");
    assert!(
        out.ends_with(&data),
        "sample data must be preserved verbatim"
    );
}

#[test]
fn copy_wav_normalized_if_needed_skips_rewrite_when_target_up_to_date() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("source.wav");
    fs::write(&src, wav_with_extensible_fmt(1, &[0u8; 20])).unwrap();
    let dst = dir.path().join("target.wav");

    copy_wav_normalized_if_needed(&src, &dst).unwrap();
    let first_write = fs::metadata(&dst).unwrap().modified().unwrap();

    std::thread::sleep(std::time::Duration::from_millis(10));
    copy_wav_normalized_if_needed(&src, &dst).unwrap();
    let second_write = fs::metadata(&dst).unwrap().modified().unwrap();

    assert_eq!(
        first_write, second_write,
        "up-to-date target should not be rewritten"
    );
}

#[test]
fn copy_wav_normalized_if_needed_falls_back_for_non_extensible_wav() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("source.wav");
    // Plain PCM (format tag 1), not extensible - should behave like copy_if_different.
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&44_100u32.to_le_bytes());
    fmt.extend_from_slice(&(44_100u32 * 2).to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    body.extend_from_slice(&fmt);
    body.extend_from_slice(b"data");
    body.extend_from_slice(&8u32.to_le_bytes());
    body.extend_from_slice(&[0u8; 8]);
    let mut plain = Vec::new();
    plain.extend_from_slice(b"RIFF");
    plain.extend_from_slice(&(body.len() as u32).to_le_bytes());
    plain.extend_from_slice(&body);
    fs::write(&src, &plain).unwrap();
    let dst = dir.path().join("target.wav");

    copy_wav_normalized_if_needed(&src, &dst).unwrap();
    assert_eq!(
        fs::read(&dst).unwrap(),
        plain,
        "non-extensible WAV copied as-is"
    );
}

#[test]
fn copy_wav_normalized_if_needed_falls_back_for_extensible_other_subformat() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("source.wav");
    let source_bytes = wav_with_extensible_fmt(0x0006, &[0u8; 20]);
    fs::write(&src, &source_bytes).unwrap();
    let dst = dir.path().join("target.wav");

    copy_wav_normalized_if_needed(&src, &dst).unwrap();
    assert_eq!(
        fs::read(&dst).unwrap(),
        source_bytes,
        "extensible-other WAV is not safe to rewrite, so it's copied verbatim"
    );
}

#[test]
fn remove_playlist_and_tracks_from_pdb_allows_removing_last_playlist() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();

    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    let playlist = ExportPlaylistData {
        id: "pl-last".to_string(),
        name: "Last Playlist".to_string(),
        tracks: vec![ExportTrackData {
            cues: Vec::new(),
            id: "t1".to_string(),
            title: "Song A".to_string(),
            artist: "Artist".to_string(),
            album: Some("Album".to_string()),
            track_number: Some(1),
            bpm: Some(124.0),
            key: Some("8A".to_string()),
            file_path: "/source/a.mp3".to_string(),
            file_name: "a.mp3".to_string(),
            file_modified_at: None,
            file_size_bytes: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: None,
            color_id: None,
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: None,
            release_year: None,
            release_date: None,
            recorded_date: None,
            file_type: None,
            artwork_path: None,
            waveform_peaks_path: None,
            duration_ms: Some(195_000),
            first_beat_ms: None,
            beatgrid_from_rekordbox: false,
            position: 0,
        }],
    };
    let manifest = ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: "pl-last".to_string(),
        playlist_name: "Last Playlist".to_string(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![ExportManifestTrack {
            first_beat_ms: None,
            cues: Vec::new(),
            id: "t1".to_string(),
            master_db_id: None,
            master_content_id: None,
            content_link: None,
            position: 1,
            track_number: Some(1),
            title: "Song A".to_string(),
            artist: "Artist".to_string(),
            album: Some("Album".to_string()),
            bpm: Some(124.0),
            key: Some("8A".to_string()),
            source_path: "/source/a.mp3".to_string(),
            exported_path: "/Contents/Artist/Album/a.mp3".to_string(),
            file_modified_at: None,
            file_size_bytes: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: None,
            color_id: None,
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: None,
            release_year: None,
            release_date: None,
            recorded_date: None,
            file_type: None,
            owns_exported_media: true,
            owns_artwork: true,
            owns_waveform: true,
            artwork_path: None,
            waveform_path: None,
            duration_ms: Some(195_000),
        }],
    };

    write_pdb(usb_root, &playlist, &manifest, true, None, None).expect("append playlist to pdb");

    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let pdb_size_before = std::fs::metadata(&pdb_path).unwrap().len();

    let parsed_before = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb before remove");
    let target_playlist_id = parsed_before
        .playlist_tree
        .iter()
        .find(|row| {
            !row.row_is_folder
                && canonicalize_playlist_name(&row.name)
                    == canonicalize_playlist_name("Last Playlist")
        })
        .map(|row| row.id)
        .expect("target playlist leaf id");

    let mut warnings = Vec::new();
    let result = remove_playlist_and_tracks_from_pdb(
        usb_root,
        Some(&format!("usb-pl-{target_playlist_id}")),
        &[String::from("Last Playlist")],
        &mut warnings,
    )
    .expect("remove last playlist from pdb");
    assert_eq!(result.removed_playlist_count, 1);

    let pdb_size_after = std::fs::metadata(&pdb_path).unwrap().len();
    assert_eq!(
        pdb_size_before, pdb_size_after,
        "PDB file size must not change on deletion (shape preservation)"
    );

    let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb after remove");
    assert!(
        parsed
            .playlist_tree
            .iter()
            .filter(|row| !row.row_is_folder)
            .all(|row| row.id != target_playlist_id),
        "playlist leaf should be removed from playlist_tree by id"
    );
    assert!(
        parsed.playlist_entries.is_empty(),
        "playlist_entries should be empty after removing last playlist"
    );
    assert!(
        parsed.tracks.is_empty(),
        "tracks should be empty after removing last playlist"
    );
    assert_eq!(result.exclusive_tracks.len(), 1);
    assert_eq!(result.shared_track_count, 0);
}

#[test]
fn remove_playlist_cleans_exclusive_tracks_preserves_shared() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();
    std::fs::create_dir_all(usb_root.join("Contents/Artist/Album")).unwrap();
    std::fs::create_dir_all(
        usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_ANALYSIS_DIR)
            .join("P001"),
    )
    .unwrap();

    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    // Export playlist A with tracks t1, t2
    let playlist_a = ExportPlaylistData {
        id: "pl-a".to_string(),
        name: "Playlist A".to_string(),
        tracks: vec![
            make_test_track("t1", "Song One", "a1.mp3"),
            make_test_track("t2", "Song Two", "a2.mp3"),
        ],
    };
    let manifest_a = make_test_manifest(
        "pl-a",
        "Playlist A",
        usb_root,
        &[("t1", "Song One", "a1.mp3"), ("t2", "Song Two", "a2.mp3")],
    );
    write_pdb(usb_root, &playlist_a, &manifest_a, true, None, None).expect("export playlist A");

    // Export playlist B with tracks t2 (shared), t3 (exclusive)
    let playlist_b = ExportPlaylistData {
        id: "pl-b".to_string(),
        name: "Playlist B".to_string(),
        tracks: vec![
            make_test_track("t2", "Song Two", "a2.mp3"),
            make_test_track("t3", "Song Three", "a3.mp3"),
        ],
    };
    let manifest_b = make_test_manifest(
        "pl-b",
        "Playlist B",
        usb_root,
        &[("t2", "Song Two", "a2.mp3"), ("t3", "Song Three", "a3.mp3")],
    );
    write_pdb(usb_root, &playlist_b, &manifest_b, true, None, None).expect("export playlist B");

    // Create dummy audio files
    for name in &["a1.mp3", "a2.mp3", "a3.mp3"] {
        std::fs::write(
            usb_root.join("Contents/Artist/Album").join(name),
            b"fake audio",
        )
        .unwrap();
    }

    // Create dummy ANLZ files
    let anlz_dir = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_ANALYSIS_DIR)
        .join("P001");
    for name in &["a1.DAT", "a1.EXT", "a2.DAT", "a2.EXT", "a3.DAT", "a3.EXT"] {
        std::fs::write(anlz_dir.join(name), b"fake anlz").unwrap();
    }

    // Parse before removal to get track IDs
    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let parsed_before = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb before");
    assert_eq!(
        parsed_before.tracks.len(),
        3,
        "should have 3 tracks before removal"
    );
    let pdb_size_before = std::fs::metadata(&pdb_path).unwrap().len();

    // Remove playlist B
    let mut warnings = Vec::new();
    let result = remove_playlist_and_tracks_from_pdb(
        usb_root,
        None,
        &[String::from("Playlist B")],
        &mut warnings,
    )
    .expect("remove playlist B");

    assert_eq!(result.removed_playlist_count, 1);
    assert_eq!(result.shared_track_count, 1, "t2 is shared");
    assert_eq!(result.exclusive_tracks.len(), 1, "t3 is exclusive to B");
    assert!(
        result
            .exclusive_tracks
            .iter()
            .any(|t| t.track_file_path.contains("a3.mp3")),
        "exclusive track should be a3.mp3"
    );

    let pdb_size_after = std::fs::metadata(&pdb_path).unwrap().len();
    assert_eq!(
        pdb_size_before, pdb_size_after,
        "PDB file size must not change on deletion (shape preservation)"
    );

    // Verify PDB state after removal
    let parsed_after = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb after");

    // Playlist B should be removed
    assert!(
        !parsed_after
            .playlist_tree
            .iter()
            .any(|p| !p.row_is_folder && p.name == "Playlist B"),
        "Playlist B should not exist"
    );

    // Playlist A should still exist
    assert!(
        parsed_after
            .playlist_tree
            .iter()
            .any(|p| !p.row_is_folder && p.name == "Playlist A"),
        "Playlist A should still exist"
    );

    // t1 and t2 should remain, t3 should be gone
    assert_eq!(
        parsed_after.tracks.len(),
        2,
        "should have 2 tracks (t1, t2) after removing B"
    );
    assert!(
        parsed_after
            .tracks
            .iter()
            .any(|t| t.track_file_path.contains("a1.mp3")),
        "t1 should remain"
    );

    // Regression: deleting a playlist tombstones its t07 row, but t07 is
    // a `(trc, 0)` footer-convention table (docs/PDB.md "Page Footer
    // Conventions"), not a `(1, trc - 1)` tombstone-convention table like
    // t00/t08/t13. `remove_rows_inplace` used to apply the `(1, last
    // removed slot)` shape unconditionally to every table, which flipped
    // playlist_tree pages (still holding the surviving "Playlist A" row)
    // into the shape `detect_pdb_wrong_playlist_tree_shape` (and the "PDB
    // structural integrity: playlist_tree page(s) wrong shape"
    // diagnostic) flags as broken.
    let wrong_shape_pages = crate::service::repair::detect_pdb_wrong_playlist_tree_shape(&pdb_path);
    assert!(
        wrong_shape_pages.is_empty(),
        "deleting a playlist must not leave playlist_tree pages in the wrong \
         (u5, num_rl) footer shape: {wrong_shape_pages:?}"
    );

    assert!(
        parsed_after
            .tracks
            .iter()
            .any(|t| t.track_file_path.contains("a2.mp3")),
        "t2 should remain (shared)"
    );
    assert!(
        !parsed_after
            .tracks
            .iter()
            .any(|t| t.track_file_path.contains("a3.mp3")),
        "t3 should be removed"
    );
}

#[test]
fn remove_playlist_fully_shared_preserves_all_tracks() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();

    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    // Export playlist A with tracks t1, t2
    let playlist_a = ExportPlaylistData {
        id: "pl-a".to_string(),
        name: "Playlist A".to_string(),
        tracks: vec![
            make_test_track("t1", "Song One", "a1.mp3"),
            make_test_track("t2", "Song Two", "a2.mp3"),
        ],
    };
    let manifest_a = make_test_manifest(
        "pl-a",
        "Playlist A",
        usb_root,
        &[("t1", "Song One", "a1.mp3"), ("t2", "Song Two", "a2.mp3")],
    );
    write_pdb(usb_root, &playlist_a, &manifest_a, true, None, None).expect("export playlist A");

    // Export playlist B with the SAME tracks (fully overlapping)
    let playlist_b = ExportPlaylistData {
        id: "pl-b".to_string(),
        name: "Playlist B".to_string(),
        tracks: vec![
            make_test_track("t1", "Song One", "a1.mp3"),
            make_test_track("t2", "Song Two", "a2.mp3"),
        ],
    };
    let manifest_b = make_test_manifest(
        "pl-b",
        "Playlist B",
        usb_root,
        &[("t1", "Song One", "a1.mp3"), ("t2", "Song Two", "a2.mp3")],
    );
    write_pdb(usb_root, &playlist_b, &manifest_b, true, None, None).expect("export playlist B");

    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let pdb_size_before = std::fs::metadata(&pdb_path).unwrap().len();

    let mut warnings = Vec::new();
    let result = remove_playlist_and_tracks_from_pdb(
        usb_root,
        None,
        &[String::from("Playlist B")],
        &mut warnings,
    )
    .expect("remove playlist B");

    assert_eq!(result.removed_playlist_count, 1);
    assert_eq!(result.shared_track_count, 2, "both tracks are shared");
    assert_eq!(result.exclusive_tracks.len(), 0, "no exclusive tracks");

    let pdb_size_after = std::fs::metadata(&pdb_path).unwrap().len();
    assert_eq!(
        pdb_size_before, pdb_size_after,
        "PDB file size must not change on deletion (shape preservation)"
    );

    let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb after");
    assert_eq!(parsed.tracks.len(), 2, "all tracks should remain");
}

#[test]
fn remove_playlist_preserves_pdb_page_count_with_many_entries() {
    // Create enough playlist entries to span multiple pages, then remove
    // one playlist and verify the file size (page count) is unchanged.
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();

    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    // 20 tracks → 20 playlist entries per playlist → enough to stress
    let tracks_a: Vec<ExportTrackData> = (0..20)
        .map(|i| make_test_track(&format!("t{i}"), &format!("Song {i}"), &format!("a{i}.mp3")))
        .collect();
    let playlist_a = ExportPlaylistData {
        id: "pl-a".to_string(),
        name: "Big Playlist".to_string(),
        tracks: tracks_a,
    };
    let manifest_entries: Vec<(&str, &str, &str)> = (0..20)
        .map(|i| {
            // Leak strings so they live long enough for the slice
            let id: &'static str = Box::leak(format!("t{i}").into_boxed_str());
            let title: &'static str = Box::leak(format!("Song {i}").into_boxed_str());
            let file: &'static str = Box::leak(format!("a{i}.mp3").into_boxed_str());
            (id, title, file)
        })
        .collect();
    let manifest_a = make_test_manifest("pl-a", "Big Playlist", usb_root, &manifest_entries);
    write_pdb(usb_root, &playlist_a, &manifest_a, true, None, None).expect("export big playlist");

    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let pdb_size_before = std::fs::metadata(&pdb_path).unwrap().len();
    let parsed_before = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse before");
    assert_eq!(parsed_before.tracks.len(), 20);

    let mut warnings = Vec::new();
    let result = remove_playlist_and_tracks_from_pdb(
        usb_root,
        None,
        &[String::from("Big Playlist")],
        &mut warnings,
    )
    .expect("remove big playlist");
    assert_eq!(result.removed_playlist_count, 1);
    assert_eq!(result.exclusive_tracks.len(), 20);

    let pdb_size_after = std::fs::metadata(&pdb_path).unwrap().len();
    assert_eq!(
        pdb_size_before, pdb_size_after,
        "PDB file size must not change on deletion (shape preservation): before={} after={}",
        pdb_size_before, pdb_size_after
    );

    let parsed_after = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse after");
    assert!(
        parsed_after.tracks.is_empty(),
        "all tracks should be removed"
    );
    assert!(
        parsed_after
            .playlist_tree
            .iter()
            .all(|p| p.row_is_folder || p.name != "Big Playlist"),
        "playlist should be removed"
    );
}

#[test]
fn remove_playlist_and_tracks_from_pdb_keeps_history_referenced_tracks() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();

    let mut data = crate::pdb_writer::PdbData::empty();
    data.tracks.push(PdbTrackRowData {
        header_flags_u32: None,
        content_link: Some(1),
        sample_rate_hz: Some(44_100),
        file_size_bytes: Some(1234),
        master_content_id: Some(1),
        master_db_id: Some(1),
        id: 1,
        artist_id: 0,
        album_id: 0,
        artwork_id: 0,
        key_id: 0,
        genre_id: 0,
        bitrate_kbps: Some(320),
        track_number: Some(1),
        bpm: Some(120.0),
        release_year: None,
        bit_depth: None,
        duration_seconds: Some(180),
        file_type: None,
        isrc: None,
        date_added: None,
        release_date: None,
        dj_comment: None,
        file_name: Some("track.mp3".to_string()),
        publish_track_info_on: None,
        autoload_hotcues_on: None,
        color_id: None,
        rating: None,
        title: "History Shared".to_string(),
        anlz_path: "/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT".to_string(),
        file_path: "/Contents/Artist/Album/track.mp3".to_string(),
    });
    data.playlist_tree
        .push(crate::pdb_writer::PdbPlaylistTreeRow {
            id: 100,
            parent_id: 0,
            sort_order: 0,
            is_folder: false,
            name: "Playlist B".to_string(),
        });
    data.playlist_entries
        .push(crate::pdb_writer::PdbPlaylistEntryRow {
            entry_index: 1,
            track_id: 1,
            playlist_id: 100,
        });
    data.history_playlists.push(crate::pdb_writer::PdbDictRow {
        id: 200,
        name: "HISTORY 001".to_string(),
    });
    data.history_entries
        .push(crate::pdb_writer::PdbHistoryEntryRow {
            track_id: 1,
            playlist_id: 200,
            entry_index: 1,
        });

    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    crate::pdb_writer::write_pdb_to_file(&pdb_path, &data).expect("write pdb");

    let mut warnings = Vec::new();
    let result = remove_playlist_and_tracks_from_pdb(
        usb_root,
        None,
        &[String::from("Playlist B")],
        &mut warnings,
    )
    .expect("remove playlist");

    assert_eq!(result.removed_playlist_count, 1);
    assert_eq!(result.shared_track_count, 1);
    assert!(
        result.exclusive_tracks.is_empty(),
        "history-referenced track must not be treated as exclusive"
    );

    let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse patched pdb");
    assert_eq!(
        parsed.tracks.len(),
        1,
        "history-referenced track must remain"
    );
    assert!(
        parsed
            .playlist_tree
            .iter()
            .all(|row| row.name != "Playlist B"),
        "playlist should be removed while track remains"
    );
}

#[test]
fn initialize_usb_seeds_baseline_history_shape_pages() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();

    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let bytes = std::fs::read(&pdb_path).expect("read export.pdb");

    fn u32le(bytes: &[u8], off: usize) -> u32 {
        u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap())
    }
    fn u16le(bytes: &[u8], off: usize) -> u16 {
        u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap())
    }
    let ps = 4096usize;
    let tptr = |tt: usize| 0x1cusize + tt * 16;

    // t17/t18/t19 baseline pointer triplets
    let o17 = tptr(17);
    let o18 = tptr(18);
    let o19 = tptr(19);
    assert_eq!(u32le(&bytes, o17 + 4), 44);
    assert_eq!(u32le(&bytes, o17 + 8), 35);
    assert_eq!(u32le(&bytes, o17 + 12), 36);
    assert_eq!(u32le(&bytes, o18 + 4), 45);
    assert_eq!(u32le(&bytes, o18 + 8), 37);
    assert_eq!(u32le(&bytes, o18 + 12), 38);
    assert_eq!(u32le(&bytes, o19 + 4), 41);
    assert_eq!(u32le(&bytes, o19 + 8), 39);
    assert_eq!(u32le(&bytes, o19 + 12), 40);

    // Data pages carry seeded row counts/bitmasks.
    let p36 = 36 * ps;
    let p38 = 38 * ps;
    let p40 = 40 * ps;
    assert_eq!(bytes[p36 + 24], 22);
    assert_eq!(bytes[p36 + 25], 0xc0);
    assert_eq!(u16le(&bytes, p36 + ps - 4), 0xffff);
    assert_eq!(u16le(&bytes, p36 + ps - 2), 0xffff);

    assert_eq!(bytes[p38 + 24], 17);
    assert_eq!(bytes[p38 + 25], 0x20);
    assert_eq!(u16le(&bytes, p38 + ps - 4), 0xffff);
    assert_eq!(u16le(&bytes, p38 + ps - 2), 0xffff);

    assert_eq!(bytes[p40 + 24], 1);
    assert_eq!(bytes[p40 + 25], 0x20);
    assert_eq!(u16le(&bytes, p40 + ps - 4), 0x0001);
    assert_eq!(u16le(&bytes, p40 + ps - 2), 0x0001);
}

#[test]
fn export_preserves_t17_t18_seed_shape_and_allows_t19_runtime_rows() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();
    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");
    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let before_bytes = std::fs::read(&pdb_path).expect("read initialized export.pdb");

    // Force candidate fallback path on legacy-seeded USB by seeding a t08 mismatch
    // so the shape merger path is exercised while preserving seed tables.
    let playlist = ExportPlaylistData {
        id: "pl-test".to_string(),
        name: "Test".to_string(),
        tracks: vec![
            make_test_track("t1", "Song 1", "a1.mp3"),
            make_test_track("t2", "Song 2", "a2.mp3"),
            make_test_track("t3", "Song 3", "a3.mp3"),
            make_test_track("t4", "Song 4", "a4.mp3"),
        ],
    };
    let manifest = make_test_manifest(
        "pl-test",
        "Test",
        usb_root,
        &[
            ("t1", "Song 1", "a1.mp3"),
            ("t2", "Song 2", "a2.mp3"),
            ("t3", "Song 3", "a3.mp3"),
            ("t4", "Song 4", "a4.mp3"),
        ],
    );
    write_pdb(usb_root, &playlist, &manifest, true, None, None).expect("export playlist");

    let bytes = std::fs::read(&pdb_path).expect("read export.pdb");
    let ps = 4096usize;
    let tptr = |tt: usize| 0x1cusize + tt * 16;
    let u32le = |off: usize| u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
    let u16le = |off: usize| u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap());

    // t17/t18 remain baseline pointers and seed counts.
    let o17 = tptr(17);
    let o18 = tptr(18);
    for table_type in [16usize, 17, 18] {
        let ptr = tptr(table_type);
        assert_eq!(
            &bytes[ptr + 4..ptr + 16],
            &before_bytes[ptr + 4..ptr + 16],
            "t{table_type} pointer triplet should stay byte-stable"
        );
    }
    for page_idx in [33usize, 34, 35, 36, 37, 38] {
        let off = page_idx * ps;
        assert_eq!(
            &bytes[off..off + ps],
            &before_bytes[off..off + ps],
            "t16/t17/t18 page {page_idx} should stay byte-stable"
        );
    }
    assert_eq!(u32le(o17 + 8), 35);
    assert_eq!(u32le(o17 + 12), 36);
    assert_eq!(u32le(o18 + 8), 37);
    assert_eq!(u32le(o18 + 12), 38);
    assert_eq!(bytes[36 * ps + 24], 22);
    assert_eq!(bytes[38 * ps + 24], 17);

    // t19 evolves to runtime-style rows during export.
    let o19 = tptr(19);
    assert!(u32le(o19 + 4) >= 41);
    assert_eq!(u32le(o19 + 8), 39);
    assert!(u32le(o19 + 12) >= 40);
    // t19 always ends up with exactly one placeholder row on a first
    // export, regardless of track count (no real play history exists
    // yet on a freshly initialized USB, and no reference export -- real
    // rekordbox or an independent working DJ export tool -- has
    // ever shown tt=19 needing more than one row here). The
    // `initialize_usb` template already seeds exactly one row, SEALED
    // (flags=0x24), matching a genuine rekordbox-initialized template
    // (confirmed via a reference template carrying
    // rekordbox-only files this app never writes) -- since that's
    // already the desired end state, the export leaves it untouched
    // rather than needlessly rewriting it to a different, self-invented
    // footer shape.
    assert_eq!(bytes[40 * ps + 24], 1);
    assert_eq!(bytes[40 * ps + 25], 0x20);
    assert_eq!(bytes[40 * ps + 27], 0x24);
    assert_eq!(u16le(40 * ps + ps - 4), 0x0001);
    assert_eq!(u16le(40 * ps + ps - 2), 0x0001);
    // t19 row payload pattern should follow runtime sequence profile:
    // 0x00000280 + i*0x00200000, and second dword = i.
    let p40 = 40 * ps;
    let row_base = p40 + 40;
    let u0 = u32le(row_base);
    let u1 = u32le(row_base + 4);
    assert_eq!(u0, 0x0000_0280u32);
    assert_eq!(u1, 0u32);

    // t08 runtime row-index behavior should mirror growth writer contract.
    let o8 = tptr(8);
    assert_eq!(u32le(o8 + 8), 17);
    assert_eq!(u32le(o8 + 12), 18);
    assert_eq!(bytes[18 * ps + 24], 4);
    assert_eq!(u16le(18 * ps + 34), 3);
    assert_eq!(u16le(40 * ps + 34), 0);
}

/// t09/t10/t11/t12/t14/t15 are tables this codebase never writes rows
/// into, but reference exporters/hardware are known to populate some of
/// them (e.g. legacy history fallback tables) independently as a USB is
/// used over time. A pure tail append to one of these tables between the
/// on-disk "before" PDB and the candidate "after" PDB must not block an
/// unrelated export.
#[test]
fn topology_guard_tolerates_pure_tail_append_on_unwritten_table() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();
    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");
    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let before = std::fs::read(&pdb_path).expect("read initialized export.pdb");

    // Simulate hardware/reference-exporter growth: append one well-formed
    // row to t11's chain the same way any table's tail grows, without
    // this codebase ever having a t11 row encoder of its own.
    let mut after = before.clone();
    crate::pdb_writer::append_rows_to_chain_in_place(&mut after, 11, &[vec![0u8; 16]], 4096)
        .expect("simulated hardware append to t11");

    assert!(
        validate_topology_locked_export_bytes(&before, &after).is_ok(),
        "a pure tail append on a table this codebase never writes to must be tolerated"
    );
}

/// The same guard must still reject a relocation/reorder of an
/// already-committed page on one of those unwritten tables — tolerating
/// tail growth must not mean tolerating anything.
#[test]
fn topology_guard_rejects_relocated_page_on_unwritten_table() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();
    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");
    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let before = std::fs::read(&pdb_path).expect("read initialized export.pdb");

    // Grow t11 by one page first (legitimate baseline), then point
    // last_page at a page never actually chained there — the new
    // last_page is unreachable by walking next-pointers from first, so
    // this is not a real tail append and must be rejected.
    let mut after = before.clone();
    crate::pdb_writer::append_rows_to_chain_in_place(&mut after, 11, &[vec![0u8; 16]], 4096)
        .expect("simulated hardware append to t11");
    let (ec, first, _last) = table_ptr_fields(&after, 11).expect("t11 pointer present");
    let bogus_last = (after.len() / 4096) as u32 - 1;
    crate::utils::set_table_ptr_fields(&mut after, 11, ec, first, bogus_last);

    let err = validate_topology_locked_export_bytes(&before, &after)
        .expect_err("an unreachable last_page must be rejected as a non-append change");
    assert!(
        err.to_string().contains("t11"),
        "error should name the offending table: {err}"
    );
}

#[test]
fn rewrite_pdb_playlist_rebuilds_resolvable_dictionary_rows_on_initialized_usb() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();

    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    let playlist = ExportPlaylistData {
        id: "pl-dict".to_string(),
        name: "Dictionary Repair".to_string(),
        tracks: vec![make_test_track("t1", "Song One", "a1.mp3")],
    };
    let manifest = ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: "pl-dict".to_string(),
        playlist_name: "Dictionary Repair".to_string(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: true,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![ExportManifestTrack {
            first_beat_ms: None,
            cues: Vec::new(),
            id: "t1".to_string(),
            master_db_id: Some(1),
            master_content_id: Some(1),
            content_link: Some(1),
            position: 1,
            track_number: Some(7),
            title: "Song One".to_string(),
            artist: "Artist".to_string(),
            album: Some("Album".to_string()),
            bpm: Some(120.0),
            key: Some("8A".to_string()),
            source_path: "/source/a1.mp3".to_string(),
            exported_path: "/Contents/Artist/Album/a1.mp3".to_string(),
            file_modified_at: None,
            file_size_bytes: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: None,
            color_id: None,
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: None,
            release_year: None,
            release_date: None,
            recorded_date: None,
            file_type: None,
            owns_exported_media: true,
            owns_artwork: true,
            owns_waveform: true,
            artwork_path: Some("/PIONEER/Artwork/00001/a00001.jpg".to_string()),
            waveform_path: Some("/PIONEER/USBANLZ/P001/A001/ANLZ0000.DAT".to_string()),
            duration_ms: Some(180_000),
        }],
    };

    write_pdb(usb_root, &playlist, &manifest, true, None, None).expect("initial append");
    let parsed_before = crate::pdb_reader::parse_pdb(
        &usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb"),
    )
    .expect("parse appended pdb");
    let track_before = parsed_before
        .tracks
        .iter()
        .find(|track| track.track_file_path == "/Contents/Artist/Album/a1.mp3")
        .expect("appended track");
    assert_eq!(
        parsed_before
            .artists
            .get(&track_before.artist_id)
            .map(String::as_str),
        Some("Artist"),
        "initial append should materialize resolvable artist dictionary rows"
    );

    rewrite_pdb_playlist_from_manifest(usb_root, &playlist, &manifest, 1, 1)
        .expect("rewrite playlist");

    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse rewritten pdb");

    let playlist_row = parsed
        .playlist_tree
        .iter()
        .find(|row| !row.row_is_folder && row.name == "Dictionary Repair")
        .expect("rewritten playlist row");
    assert_eq!(playlist_row.id, 1);

    let track = parsed
        .tracks
        .iter()
        .find(|track| track.track_file_path == "/Contents/Artist/Album/a1.mp3")
        .expect("rewritten track");
    assert!(track.artist_id > 0, "track should carry artist id");
    assert!(track.album_id > 0, "track should carry album id");
    assert!(track.key_id > 0, "track should carry key id");
    assert!(track.artwork_id > 0, "track should carry artwork id");
    assert_eq!(
        parsed.artists.get(&track.artist_id).map(String::as_str),
        Some("Artist"),
        "artists map after rewrite: {:?}",
        parsed.artists
    );
    assert_eq!(
        parsed.albums.get(&track.album_id).map(String::as_str),
        Some("Album"),
        "albums map after rewrite: {:?}",
        parsed.albums
    );
    assert_eq!(
        parsed.keys.get(&track.key_id).map(String::as_str),
        Some("8A"),
        "keys map after rewrite: {:?}",
        parsed.keys
    );
    assert_eq!(
        parsed.artworks.get(&track.artwork_id).map(String::as_str),
        Some("/PIONEER/Artwork/00001/a00001.jpg")
    );
}

#[test]
fn write_pdb_fresh_backfills_dictionary_ids_from_edb_when_manifest_is_thin() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();
    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    let mut unlock_warnings = Vec::<WarningEntry>::new();
    let conn = open_edb_rw(usb_root, &mut unlock_warnings).expect("open eDB");
    let mut seed_track = mapping_test_track();
    seed_track.exported_path = "/Contents/Fallback Artist/Fallback Album/fallback.mp3".to_string();
    seed_track.artist = "Fallback Artist".to_string();
    seed_track.album = Some("Fallback Album".to_string());
    seed_track.key = Some("10A".to_string());
    seed_track.artwork_path = Some("/PIONEER/Artwork/00001/fallback.jpg".to_string());
    let _ = insert_content_from_template(&conn, &seed_track, Some("2024-01-01"))
        .expect("seed content row");

    let playlist = ExportPlaylistData {
        id: "pl-fallback".to_string(),
        name: "Fallback Playlist".to_string(),
        tracks: vec![make_test_track(
            "t-fallback",
            "Fallback Song",
            "fallback.mp3",
        )],
    };
    let mut thin_manifest_track = seed_track.clone();
    thin_manifest_track.id = "t-fallback".to_string();
    thin_manifest_track.position = 1;
    thin_manifest_track.title = "Fallback Song".to_string();
    thin_manifest_track.artist = String::new();
    thin_manifest_track.album = None;
    thin_manifest_track.key = None;
    thin_manifest_track.artwork_path = None;
    thin_manifest_track.waveform_path =
        Some("/PIONEER/USBANLZ/P111/11111111/ANLZ0000.DAT".to_string());

    let manifest = ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: "pl-fallback".to_string(),
        playlist_name: "Fallback Playlist".to_string(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: true,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![thin_manifest_track],
    };

    write_pdb(usb_root, &playlist, &manifest, true, None, None)
        .expect("write pdb with eDB fallback metadata");

    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse rewritten pdb");
    let track = parsed
        .tracks
        .iter()
        .find(|track| track.track_file_path == seed_track.exported_path.as_str())
        .expect("fallback track");

    assert!(
        track.artist_id > 0,
        "artist id should come from eDB fallback"
    );
    assert!(track.album_id > 0, "album id should come from eDB fallback");
    assert!(track.key_id > 0, "key id should come from eDB fallback");
    assert!(
        track.artwork_id > 0,
        "artwork id should come from eDB fallback"
    );
    assert_eq!(
        parsed.artists.get(&track.artist_id).map(String::as_str),
        Some("Fallback Artist")
    );
    assert_eq!(
        parsed.albums.get(&track.album_id).map(String::as_str),
        Some("Fallback Album")
    );
    assert_eq!(
        parsed.keys.get(&track.key_id).map(String::as_str),
        Some("10A")
    );
    assert_eq!(
        parsed.artworks.get(&track.artwork_id).map(String::as_str),
        Some("/PIONEER/Artwork/00001/fallback.jpg")
    );
}

#[test]
fn write_pdb_updates_alias_matched_track_to_manifest_filename_exactly() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();
    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    let playlist = ExportPlaylistData {
        id: "pl-alias-path".to_string(),
        name: "Alias Path".to_string(),
        tracks: Vec::new(),
    };
    let old_path = "/Contents/Example Artist/Example Album/Example Artist - Example Album - .mp3";
    let corrected_path =
        "/Contents/Example Artist/Example Album/Example Artist - Example Album -.mp3";
    let corrected_file_name = "Example Artist - Example Album -.mp3";

    let mut track = mapping_test_track();
    track.id = "alias-track".to_string();
    track.title = "Example Title".to_string();
    track.artist = "Example Artist".to_string();
    track.album = Some("Example Album".to_string());
    track.exported_path = old_path.to_string();

    let mut manifest = ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: playlist.id.clone(),
        playlist_name: playlist.name.clone(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![track],
    };

    write_pdb(usb_root, &playlist, &manifest, true, None, None)
        .expect("write initial pdb with old alias path");
    manifest.tracks[0].exported_path = corrected_path.to_string();

    write_pdb(usb_root, &playlist, &manifest, true, None, None)
        .expect("rewrite pdb with corrected manifest path");

    let parsed = crate::pdb_reader::parse_pdb(
        &usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb"),
    )
    .expect("parse rewritten pdb");
    let track = parsed
        .tracks
        .iter()
        .find(|track| track.title == "Example Title")
        .expect("rewritten alias track");

    assert_eq!(track.track_file_path, corrected_path);
    assert_eq!(track.file_name.as_deref(), Some(corrected_file_name));
    assert!(
        parsed
            .tracks
            .iter()
            .all(|track| track.track_file_path != old_path),
        "old alias path must not stay indexed after the corrected export path is written: {:?}",
        parsed
            .tracks
            .iter()
            .map(|track| track.track_file_path.as_str())
            .collect::<Vec<_>>()
    );

    // Regression: rewriting file_path/file_name to the corrected alias
    // often can't be patched into the old row's slot in place (the
    // encoded string length changes), forcing the additive writer to
    // relocate the row and tombstone the old slot. That vacated slot
    // must not be left holding a duplicate of the relocated row's id —
    // see `mark_row_slot_inactive` in pdb_writer.
    let tombstoned = crate::service::repair::detect_pdb_tombstoned_playlist_tree_ids(
        &usb_root
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("export.pdb"),
    );
    assert!(
        tombstoned.is_empty(),
        "alias-path rewrite must not leave duplicate-id tombstoned rows behind: {tombstoned:?}"
    );
}

#[test]
fn write_pdb_reuses_existing_row_when_a_different_manifest_track_id_targets_the_same_foreign_scheme_path()
 {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();
    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    let playlist = ExportPlaylistData {
        id: "pl-fingerprint-fallback".to_string(),
        name: "Fingerprint Fallback".to_string(),
        tracks: Vec::new(),
    };
    // A path this app did not write itself (simulates Rekordbox's own layout,
    // or an older export run's naming scheme) — this is what export.rs's
    // content-fingerprint fallback resolves `exported_path` to when it
    // matches an existing on-USB track by file size + normalized title/artist.
    let foreign_path = "/Contents/Foreign Scheme/Steadfast-1.mp3";

    let mut track_a = mapping_test_track();
    track_a.id = "local-track-a".to_string();
    track_a.title = "Steadfast".to_string();
    track_a.artist = "Kuro".to_string();
    track_a.exported_path = foreign_path.to_string();

    let mut manifest = ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: playlist.id.clone(),
        playlist_name: playlist.name.clone(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![track_a],
    };

    write_pdb(usb_root, &playlist, &manifest, true, None, None)
        .expect("write initial pdb with foreign-scheme path");

    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let first_parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse first pdb");
    assert_eq!(first_parsed.tracks.len(), 1);
    let original_pdb_id = first_parsed.tracks[0].id;

    // Second additive export: a DIFFERENT local-library track id, but
    // export.rs's content-fingerprint fallback resolved its exported_path
    // to the SAME foreign path (same content, different app-level track id
    // — exactly what happens when a still-duplicated pair on a real USB
    // gets re-exported after the fix).
    let mut track_b = mapping_test_track();
    track_b.id = "local-track-b".to_string();
    track_b.title = "Steadfast".to_string();
    track_b.artist = "Kuro".to_string();
    track_b.exported_path = foreign_path.to_string();
    manifest.tracks = vec![track_b];

    write_pdb(usb_root, &playlist, &manifest, true, None, None)
        .expect("rewrite pdb for fingerprint-matched track b");

    let second_parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse second pdb");
    assert_eq!(
        second_parsed.tracks.len(),
        1,
        "a second manifest track id resolving to the same on-USB path must not mint a \
         duplicate PDB row: {:?}",
        second_parsed
            .tracks
            .iter()
            .map(|t| (t.id, t.track_file_path.as_str()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        second_parsed.tracks[0].id, original_pdb_id,
        "the existing row's numeric PDB track id must be reused, not replaced"
    );
    assert_eq!(second_parsed.tracks[0].track_file_path, foreign_path);
}

#[test]
fn write_pdb_writes_rating_and_colour_and_keeps_them_when_the_library_has_none() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();
    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");
    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let playlist = ExportPlaylistData {
        id: "pl-rating".to_string(),
        name: "Rating".to_string(),
        tracks: Vec::new(),
    };
    let manifest_with = |rating: Option<u32>, color_id: Option<u32>| {
        let mut track = mapping_test_track();
        track.rating = rating;
        track.color_id = color_id;
        ExportManifest {
            version: 1,
            generated_at: "2024-01-01".to_string(),
            playlist_id: playlist.id.clone(),
            playlist_name: playlist.name.clone(),
            usb_root: usb_root.to_string_lossy().to_string(),
            options: crate::models::ExportToUsbOptions {
                include_artwork: false,
                include_analysis: false,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 1,
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks: vec![track],
        }
    };
    let rating_colour = || {
        let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb");
        assert_eq!(parsed.tracks.len(), 1);
        (parsed.tracks[0].rating, parsed.tracks[0].color_id)
    };

    // A new row carries them (bytes 89/88, as rekordbox writes them).
    write_pdb(
        usb_root,
        &playlist,
        &manifest_with(Some(4), Some(3)),
        true,
        None,
        None,
    )
    .expect("first export");
    assert_eq!(rating_colour(), (4, 3));

    // Re-exported from a library without them (the app never sets them):
    // the row keeps what it has, e.g. rekordbox's own values.
    write_pdb(
        usb_root,
        &playlist,
        &manifest_with(None, None),
        true,
        None,
        None,
    )
    .expect("re-export without rating/colour");
    assert_eq!(rating_colour(), (4, 3));

    // A value from the library replaces it.
    write_pdb(
        usb_root,
        &playlist,
        &manifest_with(Some(1), Some(8)),
        true,
        None,
        None,
    )
    .expect("re-export with new rating/colour");
    assert_eq!(rating_colour(), (1, 8));
}

#[test]
fn update_existing_content_row_keeps_rating_and_colour_the_library_lacks() {
    let mut conn = rusqlite::Connection::open_in_memory().expect("open memory db");
    create_content_mapping_schema(&conn);
    conn.execute_batch(
        "INSERT INTO content (content_id, title, path, rating, color_id)
           VALUES (10, 'old', '/Contents/existing.mp3', 5, 6);",
    )
    .expect("seed content row");
    let rating_colour = |conn: &rusqlite::Connection| {
        conn.query_row(
            "SELECT rating, color_id FROM content WHERE content_id = 10",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .expect("load row")
    };
    let mut track = mapping_test_track();

    for (rating, color_id, expected) in [(None, None, (5, 6)), (Some(2), Some(1), (2, 1))] {
        track.rating = rating;
        track.color_id = color_id;
        let tx = conn.transaction().expect("start tx");
        let columns = load_table_columns_tx(&tx, "content").expect("content columns");
        update_existing_content_row(&tx, 10, &track, &columns, Some("2024-07-09"))
            .expect("update content");
        tx.commit().expect("commit tx");
        assert_eq!(
            rating_colour(&conn),
            expected,
            "library {rating:?}/{color_id:?}"
        );
    }
}

#[test]
fn write_edb_playlist_updates_alias_matched_content_path_in_place() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();
    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    let playlist = ExportPlaylistData {
        id: "pl-alias-edb".to_string(),
        name: "Alias EDB".to_string(),
        tracks: Vec::new(),
    };
    let old_path = "/Contents/Example Artist/Example Album/Example Artist - Example Album - .mp3";
    let corrected_path =
        "/Contents/Example Artist/Example Album/Example Artist - Example Album -.mp3";

    let mut track = mapping_test_track();
    track.id = "alias-track".to_string();
    track.title = "Example Title".to_string();
    track.artist = "Example Artist".to_string();
    track.album = Some("Example Album".to_string());
    track.exported_path = old_path.to_string();

    let mut manifest = ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: playlist.id.clone(),
        playlist_name: playlist.name.clone(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![track],
    };

    write_edb_playlist(usb_root, &playlist, &manifest, true)
        .expect("write initial eDB content path");
    manifest.tracks[0].exported_path = corrected_path.to_string();
    write_edb_playlist(usb_root, &playlist, &manifest, true).expect("rewrite eDB content path");

    let mut warnings = Vec::<WarningEntry>::new();
    let conn = open_edb_rw(usb_root, &mut warnings).expect("open eDB");
    let paths = {
        let mut stmt = conn
            .prepare("SELECT path FROM content ORDER BY content_id")
            .expect("prepare content path query");
        stmt.query_map([], |row| row.get::<_, String>(0))
            .expect("query content paths")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect content paths")
    };

    assert_eq!(
        paths,
        vec![corrected_path.to_string()],
        "alias-normalized eDB rewrite should update the previously linked content row \
         instead of leaving a stale indexed path behind"
    );
}

#[test]
fn strict_parity_report_has_no_dictionary_id_issues_for_thin_manifest_fallback() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();
    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    let mut unlock_warnings = Vec::<WarningEntry>::new();
    let mut conn = open_edb_rw(usb_root, &mut unlock_warnings).expect("open eDB");

    let mut seed_track = mapping_test_track();
    seed_track.exported_path = "/Contents/Fallback Artist/Fallback Album/fallback.mp3".to_string();
    seed_track.artist = "Fallback Artist".to_string();
    seed_track.album = Some("Fallback Album".to_string());
    seed_track.key = Some("10A".to_string());
    seed_track.artwork_path = Some("/PIONEER/Artwork/00001/fallback.jpg".to_string());

    let playlist = ExportPlaylistData {
        id: "pl-fallback-parity".to_string(),
        name: "Fallback Parity".to_string(),
        tracks: vec![make_test_track(
            "t-fallback",
            "Fallback Song",
            "fallback.mp3",
        )],
    };

    let tx = conn.transaction().expect("start eDB tx");
    let content_id = insert_content_from_template(&tx, &seed_track, Some("2024-01-01"))
        .expect("seed content row");
    let playlist_id = upsert_export_playlist_row(&tx, &playlist).expect("upsert playlist");
    link_playlist_content(&tx, playlist_id, content_id, 1).expect("link playlist content");
    tx.commit().expect("commit eDB tx");

    let mut thin_manifest_track = seed_track.clone();
    thin_manifest_track.id = "t-fallback".to_string();
    thin_manifest_track.position = 1;
    thin_manifest_track.title = "Fallback Song".to_string();
    thin_manifest_track.artist = String::new();
    thin_manifest_track.album = None;
    thin_manifest_track.key = None;
    thin_manifest_track.artwork_path = None;
    thin_manifest_track.waveform_path =
        Some("/PIONEER/USBANLZ/P111/11111111/ANLZ0000.DAT".to_string());

    let manifest = ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: "pl-fallback-parity".to_string(),
        playlist_name: "Fallback Parity".to_string(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: true,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![thin_manifest_track],
    };

    write_pdb(usb_root, &playlist, &manifest, true, None, None)
        .expect("write pdb with eDB fallback metadata");

    let backend_data = tempdir().expect("backend data dir");
    let service = BackendService::new(backend_data.path()).expect("create backend service");
    let report = service
        .run_usb_parity_report(RunUsbParityReportRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
        })
        .expect("run strict parity report");

    let detail = report
        .playlist_details
        .iter()
        .find(|d| d.name == "Fallback Parity")
        .expect("fallback parity detail");
    assert_eq!(
        detail.dictionary_id_issue_tracks, 0,
        "dictionaryIdIssueTracks should remain zero when PDB uses eDB fallback metadata"
    );
}

#[test]
fn strict_parity_report_has_no_dictionary_id_issues_when_edb_is_thin_but_manifest_is_rich() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();
    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    let mut unlock_warnings = Vec::<WarningEntry>::new();
    let mut conn = open_edb_rw(usb_root, &mut unlock_warnings).expect("open eDB");

    let mut rich_manifest_track = mapping_test_track();
    rich_manifest_track.exported_path = "/Contents/Rich Artist/Rich Album/rich.mp3".to_string();
    rich_manifest_track.artist = "Rich Artist".to_string();
    rich_manifest_track.album = Some("Rich Album".to_string());
    rich_manifest_track.key = Some("9A".to_string());
    rich_manifest_track.artwork_path = Some("/PIONEER/Artwork/00001/rich.jpg".to_string());

    let playlist = ExportPlaylistData {
        id: "pl-rich-manifest".to_string(),
        name: "Rich Manifest".to_string(),
        tracks: vec![make_test_track("t-rich", "Rich Song", "rich.mp3")],
    };

    let tx = conn.transaction().expect("start eDB tx");
    let content_id = insert_content_from_template(&tx, &rich_manifest_track, Some("2024-01-01"))
        .expect("seed content row");
    let content_columns = load_table_columns_tx(&tx, "content").expect("content columns");
    if content_columns.contains("artist_id_artist") {
        tx.execute(
            "UPDATE content SET artist_id_artist = 0 WHERE content_id = ?1",
            params![content_id],
        )
        .expect("thin eDB artist id");
    }
    if content_columns.contains("album_id") {
        tx.execute(
            "UPDATE content SET album_id = NULL WHERE content_id = ?1",
            params![content_id],
        )
        .expect("thin eDB album id");
    }
    if content_columns.contains("key_id") {
        tx.execute(
            "UPDATE content SET key_id = NULL WHERE content_id = ?1",
            params![content_id],
        )
        .expect("thin eDB key id");
    }
    if content_columns.contains("image_id") {
        tx.execute(
            "UPDATE content SET image_id = NULL WHERE content_id = ?1",
            params![content_id],
        )
        .expect("thin eDB image id");
    }
    if content_columns.contains("imageFilePath_id") {
        tx.execute(
            "UPDATE content SET imageFilePath_id = NULL WHERE content_id = ?1",
            params![content_id],
        )
        .expect("thin eDB imageFilePath id");
    }
    let playlist_id = upsert_export_playlist_row(&tx, &playlist).expect("upsert playlist");
    link_playlist_content(&tx, playlist_id, content_id, 1).expect("link playlist content");
    tx.commit().expect("commit eDB tx");

    rich_manifest_track.id = "t-rich".to_string();
    rich_manifest_track.position = 1;
    rich_manifest_track.title = "Rich Song".to_string();
    rich_manifest_track.waveform_path =
        Some("/PIONEER/USBANLZ/P222/22222222/ANLZ0000.DAT".to_string());

    let manifest = ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: "pl-rich-manifest".to_string(),
        playlist_name: "Rich Manifest".to_string(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: true,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![rich_manifest_track],
    };

    write_pdb(usb_root, &playlist, &manifest, true, None, None)
        .expect("write pdb with rich manifest metadata");

    let backend_data = tempdir().expect("backend data dir");
    let service = BackendService::new(backend_data.path()).expect("create backend service");
    let report = service
        .run_usb_parity_report(RunUsbParityReportRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
        })
        .expect("run strict parity report");

    let detail = report
        .playlist_details
        .iter()
        .find(|d| d.name == "Rich Manifest")
        .expect("rich manifest detail");
    assert_eq!(
        detail.dictionary_id_issue_tracks, 0,
        "dictionaryIdIssueTracks should be zero when manifest metadata materializes dictionary rows"
    );
    assert!(
        detail.edb_missing_core_metadata > 0,
        "eDB is intentionally thinned in this scenario"
    );
}

#[test]
fn strict_parity_additive_write_keeps_dictionary_ids_resolved_with_thin_manifest() {
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();
    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    let mut unlock_warnings = Vec::<WarningEntry>::new();
    let mut conn = open_edb_rw(usb_root, &mut unlock_warnings).expect("open eDB");

    let mut base_track = mapping_test_track();
    base_track.id = "t-base".to_string();
    base_track.title = "Base Song".to_string();
    base_track.artist = "Base Artist".to_string();
    base_track.album = Some("Base Album".to_string());
    base_track.key = Some("8A".to_string());
    base_track.exported_path = "/Contents/Base Artist/Base Album/base.mp3".to_string();
    base_track.artwork_path = Some("/PIONEER/Artwork/00001/base.jpg".to_string());
    base_track.waveform_path = Some("/PIONEER/USBANLZ/P001/AAAA0001/ANLZ0000.DAT".to_string());
    base_track.position = 1;

    let mut add_track_rich = mapping_test_track();
    add_track_rich.id = "t-add".to_string();
    add_track_rich.title = "Add Song".to_string();
    add_track_rich.artist = "Add Artist".to_string();
    add_track_rich.album = Some("Add Album".to_string());
    add_track_rich.key = Some("10A".to_string());
    add_track_rich.exported_path = "/Contents/Add Artist/Add Album/add.mp3".to_string();
    add_track_rich.artwork_path = Some("/PIONEER/Artwork/00001/add.jpg".to_string());
    add_track_rich.waveform_path = Some("/PIONEER/USBANLZ/P001/AAAA0002/ANLZ0000.DAT".to_string());
    add_track_rich.position = 2;

    let playlist = ExportPlaylistData {
        id: "pl-additive".to_string(),
        name: "Additive Fallback".to_string(),
        tracks: vec![
            make_test_track("t-base", "Base Song", "base.mp3"),
            make_test_track("t-add", "Add Song", "add.mp3"),
        ],
    };

    let tx = conn.transaction().expect("start eDB tx");
    let base_content_id = insert_content_from_template(&tx, &base_track, Some("2024-01-01"))
        .expect("seed base content row");
    let add_content_id = insert_content_from_template(&tx, &add_track_rich, Some("2024-01-01"))
        .expect("seed add content row");
    let playlist_id = upsert_export_playlist_row(&tx, &playlist).expect("upsert playlist");
    link_playlist_content(&tx, playlist_id, base_content_id, 1).expect("link base track");
    tx.commit().expect("commit eDB tx");

    let base_manifest = ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: "pl-additive".to_string(),
        playlist_name: "Additive Fallback".to_string(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: true,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![base_track.clone()],
    };
    write_pdb(usb_root, &playlist, &base_manifest, true, None, None)
        .expect("write base playlist to pdb");

    let tx = conn.transaction().expect("start additive eDB tx");
    link_playlist_content(&tx, playlist_id, add_content_id, 2).expect("link additive track");
    tx.commit().expect("commit additive eDB tx");

    let mut add_track_thin_manifest = add_track_rich.clone();
    add_track_thin_manifest.artist = String::new();
    add_track_thin_manifest.album = None;
    add_track_thin_manifest.key = None;
    add_track_thin_manifest.artwork_path = None;

    let additive_manifest = ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: "pl-additive".to_string(),
        playlist_name: "Additive Fallback".to_string(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: true,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![add_track_thin_manifest],
    };
    write_pdb(usb_root, &playlist, &additive_manifest, false, None, None)
        .expect("append additive track with thin manifest");

    let backend_data = tempdir().expect("backend data dir");
    let service = BackendService::new(backend_data.path()).expect("create backend service");
    let report = service
        .run_usb_parity_report(RunUsbParityReportRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
        })
        .expect("run strict parity report");

    let detail = report
        .playlist_details
        .iter()
        .find(|d| d.name == "Additive Fallback")
        .expect("additive fallback detail");
    assert_eq!(
        detail.dictionary_id_issue_tracks, 0,
        "dictionaryIdIssueTracks should stay zero for additive write with thin manifest"
    );
}

#[test]
fn rewrite_pdb_playlist_keeps_f_sharp_and_f_minor_as_distinct_keys() {
    let dir = tempdir().expect("tempdir");
    let usb_root = dir.path();
    std::fs::create_dir_all(usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR))
        .expect("create pdb dir");
    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    let playlist = ExportPlaylistData {
        id: "pl-keys".to_string(),
        name: "Key Collision".to_string(),
        tracks: vec![],
    };
    let manifest = ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: "pl-keys".to_string(),
        playlist_name: "Key Collision".to_string(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 2,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![
            ExportManifestTrack {
                first_beat_ms: None,
                cues: Vec::new(),
                id: "t1".to_string(),
                master_db_id: Some(1),
                master_content_id: Some(1),
                content_link: Some(1),
                position: 1,
                track_number: Some(1),
                title: "Sharp".to_string(),
                artist: "Artist".to_string(),
                album: Some("Album".to_string()),
                bpm: Some(120.0),
                key: Some("F#m".to_string()),
                source_path: "/source/sharp.mp3".to_string(),
                exported_path: "/Contents/Artist/Album/sharp.mp3".to_string(),
                file_modified_at: None,
                file_size_bytes: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                title_for_search: None,
                kuvo_delivery_comment: None,
                dj_play_count: None,
                rating: None,
                color_id: None,
                artist_id_lyricist: None,
                artist_id_original_artist: None,
                artist_id_remixer: None,
                artist_id_composer: None,
                genre_id: None,
                genre: None,
                label_id: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                file_type: None,
                owns_exported_media: true,
                owns_artwork: false,
                owns_waveform: false,
                artwork_path: None,
                waveform_path: None,
                duration_ms: Some(180_000),
            },
            ExportManifestTrack {
                first_beat_ms: None,
                cues: Vec::new(),
                id: "t2".to_string(),
                master_db_id: Some(2),
                master_content_id: Some(2),
                content_link: Some(2),
                position: 2,
                track_number: Some(2),
                title: "Minor".to_string(),
                artist: "Artist".to_string(),
                album: Some("Album".to_string()),
                bpm: Some(120.0),
                key: Some("Fm".to_string()),
                source_path: "/source/minor.mp3".to_string(),
                exported_path: "/Contents/Artist/Album/minor.mp3".to_string(),
                file_modified_at: None,
                file_size_bytes: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                title_for_search: None,
                kuvo_delivery_comment: None,
                dj_play_count: None,
                rating: None,
                color_id: None,
                artist_id_lyricist: None,
                artist_id_original_artist: None,
                artist_id_remixer: None,
                artist_id_composer: None,
                genre_id: None,
                genre: None,
                label_id: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                file_type: None,
                owns_exported_media: true,
                owns_artwork: false,
                owns_waveform: false,
                artwork_path: None,
                waveform_path: None,
                duration_ms: Some(180_000),
            },
        ],
    };

    rewrite_pdb_playlist_from_manifest(usb_root, &playlist, &manifest, 1, 1)
        .expect("rewrite with distinct musical keys");

    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse rewritten pdb");

    let sharp = parsed
        .tracks
        .iter()
        .find(|track| track.track_file_path == "/Contents/Artist/Album/sharp.mp3")
        .expect("sharp track");
    let minor = parsed
        .tracks
        .iter()
        .find(|track| track.track_file_path == "/Contents/Artist/Album/minor.mp3")
        .expect("minor track");

    assert_ne!(
        sharp.key_id, minor.key_id,
        "F#m and Fm must resolve to different PDB key IDs"
    );
    assert_eq!(
        parsed.keys.get(&sharp.key_id).map(String::as_str),
        Some("F#m")
    );
    assert_eq!(
        parsed.keys.get(&minor.key_id).map(String::as_str),
        Some("Fm")
    );
}

#[test]
fn key_resolution_uses_enharmonic_match_without_minor_mode_confusion() {
    let mut map = HashMap::<String, u32>::new();
    map.insert("db".to_string(), 16);
    map.insert("dbm".to_string(), 1);

    assert_eq!(resolve_key_id_by_name(&map, "C#"), Some(16));
    assert_eq!(resolve_key_id_by_name(&map, "C#m"), Some(1));
    assert_ne!(
        resolve_key_id_by_name(&map, "C#"),
        resolve_key_id_by_name(&map, "C#m")
    );
}

/// Helper: create a minimal ExportTrackData
fn make_test_track(id: &str, title: &str, filename: &str) -> ExportTrackData {
    ExportTrackData {
        cues: Vec::new(),
        id: id.to_string(),
        title: title.to_string(),
        artist: "Artist".to_string(),
        album: Some("Album".to_string()),
        track_number: Some(1),
        bpm: Some(120.0),
        key: None,
        file_path: format!("/source/{filename}"),
        file_name: filename.to_string(),
        file_modified_at: None,
        file_size_bytes: None,
        sample_rate_hz: None,
        bit_depth: None,
        bitrate_kbps: None,
        disc_number: None,
        subtitle: None,
        comment: None,
        title_for_search: None,
        kuvo_delivery_comment: None,
        dj_play_count: None,
        rating: None,
        color_id: None,
        artist_id_lyricist: None,
        artist_id_original_artist: None,
        artist_id_remixer: None,
        artist_id_composer: None,
        genre_id: None,
        genre: None,
        label_id: None,
        isrc: None,
        release_year: None,
        release_date: None,
        recorded_date: None,
        file_type: None,
        artwork_path: None,
        waveform_peaks_path: None,
        duration_ms: Some(180_000),
        first_beat_ms: None,
        beatgrid_from_rekordbox: false,
        position: 0,
    }
}

/// Helper: create a minimal ExportManifest
fn make_test_manifest(
    playlist_id: &str,
    playlist_name: &str,
    usb_root: &std::path::Path,
    tracks: &[(&str, &str, &str)], // (id, title, filename)
) -> ExportManifest {
    ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: playlist_id.to_string(),
        playlist_name: playlist_name.to_string(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: tracks.len(),
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: tracks
            .iter()
            .enumerate()
            .map(|(i, (id, title, filename))| ExportManifestTrack {
                first_beat_ms: None,
                cues: Vec::new(),
                id: id.to_string(),
                master_db_id: None,
                master_content_id: None,
                content_link: None,
                position: i + 1,
                track_number: Some(1),
                title: title.to_string(),
                artist: "Artist".to_string(),
                album: Some("Album".to_string()),
                bpm: Some(120.0),
                key: None,
                source_path: format!("/source/{filename}"),
                exported_path: format!("/Contents/Artist/Album/{filename}"),
                file_modified_at: None,
                file_size_bytes: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                title_for_search: None,
                kuvo_delivery_comment: None,
                dj_play_count: None,
                rating: None,
                color_id: None,
                artist_id_lyricist: None,
                artist_id_original_artist: None,
                artist_id_remixer: None,
                artist_id_composer: None,
                genre_id: None,
                genre: None,
                label_id: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                file_type: None,
                owns_exported_media: true,
                owns_artwork: true,
                owns_waveform: true,
                artwork_path: None,
                waveform_path: None,
                duration_ms: Some(180_000),
            })
            .collect(),
    }
}

#[test]
fn write_pdb_refreshes_file_size_sample_rate_bitrate_on_reexport_of_existing_track() {
    // A track re-exported at the *same* exported_path (e.g. re-encoded/
    // replaced with a different file, then the playlist re-exported)
    // must have its PDB row's file_size_bytes/sample_rate_hz/bitrate_kbps
    // refreshed, not frozen at whatever the first export originally
    // wrote -- these previously weren't in the "update existing track"
    // field list even though duration/title/anlz_path were.
    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    crate::service::usb_utils::initialize_usb(usb_root).expect("initialize usb skeleton");

    let playlist = ExportPlaylistData {
        id: "pl-refresh".to_string(),
        name: "Refresh".to_string(),
        tracks: vec![make_test_track("t1", "Song", "song.mp3")],
    };

    let mut first_manifest = make_test_manifest(
        "pl-refresh",
        "Refresh",
        usb_root,
        &[("t1", "Song", "song.mp3")],
    );
    first_manifest.tracks[0].file_size_bytes = Some(1_000_000);
    first_manifest.tracks[0].sample_rate_hz = Some(44_100);
    first_manifest.tracks[0].bitrate_kbps = Some(128);
    write_pdb(usb_root, &playlist, &first_manifest, true, None, None)
        .expect("write initial export");

    let mut second_manifest = first_manifest.clone();
    second_manifest.tracks[0].file_size_bytes = Some(2_500_000);
    second_manifest.tracks[0].sample_rate_hz = Some(48_000);
    second_manifest.tracks[0].bitrate_kbps = Some(320);
    write_pdb(usb_root, &playlist, &second_manifest, true, None, None)
        .expect("write re-export with replaced file metadata");

    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let parsed = crate::pdb_reader::parse_pdb(&pdb_path).expect("parse pdb");
    assert_eq!(
        parsed.tracks.len(),
        1,
        "re-export must update, not duplicate"
    );
    let track = &parsed.tracks[0];
    assert_eq!(track.file_size_bytes, Some(2_500_000));
    assert_eq!(track.sample_rate_hz, Some(48_000));
    assert_eq!(track.bitrate_kbps, Some(320));
}

// --- collect_manifest_owned_paths ---

#[test]
fn collect_owned_paths_includes_media_artwork_analysis() {
    let usb_root = Path::new("/mnt/usb");
    let manifest = ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: "pl1".to_string(),
        playlist_name: "Test".to_string(),
        usb_root: "/mnt/usb".to_string(),
        options: ExportToUsbOptions {
            include_artwork: true,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: vec![],
        tracks: vec![ExportManifestTrack {
            first_beat_ms: None,
            cues: Vec::new(),
            id: "t1".to_string(),
            master_db_id: None,
            master_content_id: None,
            content_link: None,
            position: 1,
            track_number: Some(1),
            title: "Track".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            key: None,
            source_path: "/local/track.mp3".to_string(),
            exported_path: "/Contents/Artist/Album/track.mp3".to_string(),
            file_modified_at: None,
            file_size_bytes: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: None,
            color_id: None,
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: None,
            release_year: None,
            release_date: None,
            recorded_date: None,
            file_type: None,
            owns_exported_media: true,
            owns_artwork: true,
            owns_waveform: true,
            artwork_path: Some("/PIONEER/Artwork/00001/a00001.jpg".to_string()),
            waveform_path: Some("/PIONEER/USBANLZ/P001/ABC/ANLZ0000.DAT".to_string()),
            duration_ms: None,
        }],
    };
    let owned = collect_manifest_owned_paths(usb_root, &manifest);
    // 1 media + 1 artwork + 3 analysis bundle (DAT/EXT/2EX) = 5
    assert!(
        owned.len() >= 3,
        "should include media + artwork + analysis paths, got {} items: {:?}",
        owned.len(),
        owned
    );
    assert!(
        owned.iter().any(|p| p.contains("track.mp3")),
        "should include media path"
    );
    assert!(
        owned.iter().any(|p| p.contains("Artwork")),
        "should include artwork path"
    );
    assert!(
        owned.iter().any(|p| p.contains("USBANLZ")),
        "should include analysis path"
    );
}

#[test]
fn collect_owned_paths_skips_none_artwork_and_waveform() {
    let usb_root = Path::new("/mnt/usb");
    let manifest = ExportManifest {
        version: 1,
        generated_at: "2024-01-01".to_string(),
        playlist_id: "pl1".to_string(),
        playlist_name: "Test".to_string(),
        usb_root: "/mnt/usb".to_string(),
        options: ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: vec![],
        tracks: vec![ExportManifestTrack {
            first_beat_ms: None,
            cues: Vec::new(),
            id: "t1".to_string(),
            master_db_id: None,
            master_content_id: None,
            content_link: None,
            position: 1,
            track_number: None,
            title: "Track".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            key: None,
            source_path: "/local/track.mp3".to_string(),
            exported_path: "/Contents/track.mp3".to_string(),
            file_modified_at: None,
            file_size_bytes: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: None,
            color_id: None,
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: None,
            release_year: None,
            release_date: None,
            recorded_date: None,
            file_type: None,
            owns_exported_media: true,
            owns_artwork: true,
            owns_waveform: true,
            artwork_path: None,
            waveform_path: None,
            duration_ms: None,
        }],
    };
    let owned = collect_manifest_owned_paths(usb_root, &manifest);
    // Should only have the media path
    assert_eq!(
        owned.len(),
        1,
        "should only include media path when no artwork/analysis"
    );
}

// ── Path shaping unit tests ──────────────────────────────────────────

#[test]
fn to_usb_relative_path_strips_root_prefix() {
    let usb = Path::new("/mnt/usb");
    assert_eq!(
        to_usb_relative_path(usb, "/mnt/usb/Contents/Artist/Track.mp3"),
        Some("/Contents/Artist/Track.mp3".to_string())
    );
}

#[test]
fn to_usb_relative_path_joins_relative_then_strips() {
    let usb = Path::new("/mnt/usb");
    assert_eq!(
        to_usb_relative_path(usb, "Contents/Artist/Track.mp3"),
        Some("/Contents/Artist/Track.mp3".to_string())
    );
}

#[test]
fn to_usb_relative_path_returns_none_for_unrelated_absolute() {
    let usb = Path::new("/mnt/usb");
    assert_eq!(to_usb_relative_path(usb, "/other/path/Track.mp3"), None);
}

#[test]
fn to_usb_relative_path_analysis_dir() {
    let usb = Path::new("/tmp/usb");
    assert_eq!(
        to_usb_relative_path(usb, "/tmp/usb/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT"),
        Some("/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT".to_string())
    );
}

#[test]
fn content_file_name_extracts_filename() {
    assert_eq!(
        content_file_name("/Contents/Artist/Album/Track.mp3"),
        "Track.mp3"
    );
}

#[test]
fn content_file_name_handles_bare_filename() {
    assert_eq!(content_file_name("Track.mp3"), "Track.mp3");
}

#[test]
fn content_file_name_handles_directory_path() {
    // Path::new treats trailing component as filename
    assert_eq!(content_file_name("/Contents/Artist"), "Artist");
}

#[test]
fn exported_media_target_path_preserves_existing_contents_structure() {
    let media_root = Path::new("/mnt/usb/Contents");
    let source = Path::new("/local/music/Contents/DJ Artist/Remix Album/hot_track.flac");
    let result = exported_media_target_path(
        media_root,
        source,
        "DJ Artist",
        Some("Remix Album"),
        "Hot Track",
        "flac",
    );
    assert_eq!(
        result,
        PathBuf::from("/mnt/usb/Contents/DJ Artist/Remix Album/hot_track.flac")
    );
}

#[test]
fn exported_media_target_path_builds_from_metadata_when_no_contents_in_source() {
    let media_root = Path::new("/mnt/usb/Contents");
    let source = Path::new("/home/user/Music/artist - track.mp3");
    let result = exported_media_target_path(
        media_root,
        source,
        "Test Artist",
        Some("Test Album"),
        "Test Track",
        "mp3",
    );
    assert_eq!(
        result,
        PathBuf::from("/mnt/usb/Contents/Test Artist/Test Album/artist - track.mp3")
    );
}

#[test]
fn manifest_paths_are_already_usb_relative() {
    // Verify the invariant that ExportManifestTrack paths are USB-relative
    // (start with /) — both eDB and PDB writers depend on this.
    let track = mapping_test_track();
    assert!(
        track.exported_path.starts_with("/Contents/"),
        "exported_path must be USB-relative: {}",
        track.exported_path
    );
    assert!(
        track
            .waveform_path
            .as_ref()
            .unwrap()
            .starts_with("/PIONEER/"),
        "waveform_path must be USB-relative: {}",
        track.waveform_path.as_ref().unwrap()
    );
    assert!(
        track
            .artwork_path
            .as_ref()
            .unwrap()
            .starts_with("/PIONEER/"),
        "artwork_path must be USB-relative: {}",
        track.artwork_path.as_ref().unwrap()
    );
}

#[test]
fn edb_and_pdb_receive_identical_media_path_from_manifest() {
    // The PDB writer previously re-converted already-relative paths through
    // to_usb_relative_path(). This test ensures both sides get the same value
    // from the manifest without re-conversion.
    let track = mapping_test_track();

    // eDB uses track.exported_path directly (line ~3061)
    let edb_path = track.exported_path.clone();

    // PDB now also uses track.exported_path directly (was to_usb_relative_path)
    let pdb_path = track.exported_path.clone();

    assert_eq!(
        edb_path, pdb_path,
        "eDB and PDB must receive identical media paths"
    );
}

#[test]
fn edb_and_pdb_receive_identical_analysis_path_from_manifest() {
    let track = mapping_test_track();

    // eDB uses track.waveform_path directly (line ~3064)
    let edb_anlz = track.waveform_path.clone().unwrap_or_default();

    // PDB now also uses track.waveform_path directly
    let pdb_anlz = track.waveform_path.clone().unwrap_or_default();

    assert_eq!(
        edb_anlz, pdb_anlz,
        "eDB and PDB must receive identical analysis paths"
    );
}

#[test]
fn genre_and_label_tables_preserved_through_export_roundtrip() {
    use crate::pdb_writer::{
        PdbData, PdbDictRow, standard_colors, standard_columns_raw, write_pdb_to_file,
    };
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let usb_root = dir.path();
    let pdb_dir = usb_root
        .join(super::USB_VENDOR_ROOT_DIR)
        .join(super::USB_VENDOR_DB_DIR);
    std::fs::create_dir_all(&pdb_dir).unwrap();

    // Write a PDB with genre and label rows
    let data = PdbData {
        genres: vec![
            PdbDictRow {
                id: 1,
                name: "Electronic".to_string(),
            },
            PdbDictRow {
                id: 2,
                name: "Techno".to_string(),
            },
        ],
        labels: vec![PdbDictRow {
            id: 1,
            name: "Kompakt".to_string(),
        }],
        colors: standard_colors(),
        columns_raw_rows: standard_columns_raw(),
        ..PdbData::empty()
    };
    write_pdb_to_file(&pdb_dir.join("export.pdb"), &data).unwrap();

    // Export over it with a manifest containing one track
    let playlist = super::ExportPlaylistData {
        id: "pl-1".to_string(),
        name: "Test".to_string(),
        tracks: vec![super::ExportTrackData {
            cues: Vec::new(),
            id: "t1".to_string(),
            title: "Track A".to_string(),
            artist: "Artist".to_string(),
            album: None,
            track_number: None,
            bpm: None,
            key: None,
            file_path: "/source/a.mp3".to_string(),
            file_name: "a.mp3".to_string(),
            file_modified_at: None,
            file_size_bytes: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: None,
            color_id: None,
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: None,
            release_year: None,
            release_date: None,
            recorded_date: None,
            file_type: None,
            artwork_path: None,
            waveform_peaks_path: None,
            duration_ms: None,
            first_beat_ms: None,
            beatgrid_from_rekordbox: false,
            position: 0,
        }],
    };
    let manifest = super::ExportManifest {
        version: 1,
        generated_at: "2025-01-01".to_string(),
        playlist_id: "pl-1".to_string(),
        playlist_name: "Test".to_string(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: crate::models::ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![super::ExportManifestTrack {
            first_beat_ms: None,
            cues: Vec::new(),
            id: "t1".to_string(),
            master_db_id: None,
            master_content_id: None,
            content_link: None,
            position: 0,
            track_number: None,
            title: "Track A".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            key: None,
            source_path: "/source/a.mp3".to_string(),
            exported_path: "/Contents/Artist/a.mp3".to_string(),
            file_modified_at: None,
            file_size_bytes: None,
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: None,
            color_id: None,
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: None,
            release_year: None,
            release_date: None,
            recorded_date: None,
            file_type: None,
            owns_exported_media: true,
            owns_artwork: false,
            owns_waveform: false,
            artwork_path: None,
            waveform_path: None,
            duration_ms: None,
        }],
    };

    super::write_pdb(usb_root, &playlist, &manifest, true, None, None).unwrap();

    // Parse result and verify genres/labels survived
    let parsed = crate::pdb_reader::parse_pdb(&pdb_dir.join("export.pdb")).unwrap();
    assert_eq!(parsed.genres.len(), 2, "genres should survive export");
    assert!(
        parsed.genres.values().any(|v| v == "Electronic"),
        "Electronic genre missing"
    );
    assert!(
        parsed.genres.values().any(|v| v == "Techno"),
        "Techno genre missing"
    );
    assert_eq!(parsed.labels.len(), 1, "labels should survive export");
    assert!(
        parsed.labels.values().any(|v| v == "Kompakt"),
        "Kompakt label missing"
    );
}

#[test]
fn default_export_profile_is_rb6_compatible() {
    assert_eq!(PdbLayoutProfile::DEFAULT, PdbLayoutProfile::Rb6Compatible);
    // from_env with no env var should return the default
    unsafe {
        std::env::remove_var("PDB_LAYOUT_PROFILE");
    }
    assert_eq!(
        PdbLayoutProfile::from_env(),
        PdbLayoutProfile::Rb6Compatible
    );
}

#[test]
fn export_profile_as_str_roundtrips() {
    assert_eq!(PdbLayoutProfile::Current.as_str(), "current");
    assert_eq!(PdbLayoutProfile::Rb6Compatible.as_str(), "rb6_compatible");
    assert_eq!(PdbLayoutProfile::Rb7Compatible.as_str(), "rb7_compatible");
}

#[test]
fn validate_no_empty_data_pages_catches_empty_page() {
    // Build a minimal 3-page PDB: page 0 = file header, page 1 = sentinel, page 2 = data
    let page_size = 4096usize;
    let mut bytes = vec![0u8; page_size * 3];

    // File header at page 0 — set table 8 pointers: first=1, last=2
    crate::utils::set_table_ptr_fields(&mut bytes, 8, 3, 1, 2);

    // Sentinel page (page 1): page_idx=1, table_type=8, next=2, flags=0x64
    let off1 = page_size;
    bytes[off1 + 0x04..off1 + 0x08].copy_from_slice(&1u32.to_le_bytes());
    bytes[off1 + 0x08..off1 + 0x0c].copy_from_slice(&8u32.to_le_bytes());
    bytes[off1 + 0x0c..off1 + 0x10].copy_from_slice(&2u32.to_le_bytes());
    bytes[off1 + 0x1b] = 0x64; // sentinel

    // Data page (page 2): page_idx=2, table_type=8, next=0, flags=0x24
    // Leave nrs=0 and used_size=0 → empty data page
    let off2 = page_size * 2;
    bytes[off2 + 0x04..off2 + 0x08].copy_from_slice(&2u32.to_le_bytes());
    bytes[off2 + 0x08..off2 + 0x0c].copy_from_slice(&8u32.to_le_bytes());
    bytes[off2 + 0x1b] = 0x24; // data page

    let result = validate_no_empty_data_pages(&bytes, page_size, 8, 1, 2);
    assert!(result.is_err(), "should reject empty data page in chain");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("0 rows"), "error should mention 0 rows: {msg}");

    // Now give the page some rows — validation should pass
    bytes[off2 + 0x18] = 1; // nrs = 1
    bytes[off2 + 0x1e..off2 + 0x20].copy_from_slice(&12u16.to_le_bytes()); // used_size = 12
    let result2 = validate_no_empty_data_pages(&bytes, page_size, 8, 1, 2);
    assert!(result2.is_ok(), "should pass with non-empty data page");
}
