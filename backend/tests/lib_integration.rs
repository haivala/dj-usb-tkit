use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Deserialize;
use std::fs;
use std::hash::{Hash, Hasher};

use tempfile::tempdir;
use walkdir::WalkDir;

use backend::commands::BackendCommands;
use backend::models::{
    AddTracksToPlaylistRequest, AnalyzeNewTracksRequest, CreatePlaylistRequest, DedupeMode,
    ExportToUsbOptions, ExportToUsbRequest, FetchUsbHistoriesRequest, FetchUsbPlaylistsRequest,
    GetPlaylistTracksRequest, GetTracksByIdsRequest, InitializeUsbRequest,
    MaterializeSourceTrackRequest, RemoveTracksBySourceRootsRequest,
    RemoveTracksFromPlaylistRequest, RemoveUsbPlaylistRequest, RepairUsbDiagnosticsRequest,
    ResolvePlaybackSourceRequest, RunUsbDiagnosticsRequest, RunUsbParityReportRequest,
    ScanLibraryRequest, SearchTracksRequest, SetFrontendSettingRequest, WarningEntry,
};
use backend::pdb_reader::parse_pdb;
use backend::service::BackendService;
use backend::service::anlz::canonical_analysis_bundle_paths;
use backend::service::export_helpers::{
    ExportManifest, ExportManifestTrack, ExportPlaylistData, analysis_bundle_path_variants,
    exported_media_target_path, write_pdb,
};
use backend::service::usb_vendor_compat::{
    USB_ANALYSIS_DIR, USB_ARTWORK_DIR, USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR,
};

fn test_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn vendor_db_dir(root: &Path) -> PathBuf {
    root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)
}

use backend::service::usb_vendor_compat::DEFAULT_USB_EDB_KEY;

/// Open an eDB, trying plain SQLite first, then SQLCipher.
fn open_export_db(path: &Path) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open(path).expect("open eDB");
    let has_schema = conn
        .query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type IN ('table','view')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0);
    if has_schema == 0 {
        conn.execute_batch(&format!("PRAGMA key='{DEFAULT_USB_EDB_KEY}';"))
            .expect("apply SQLCipher key");
        let unlocked = conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master WHERE type IN ('table','view')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);
        assert!(
            unlocked > 0,
            "failed to unlock SQLCipher DB at {}",
            path.display()
        );
    }
    conn
}

fn seed_usb_unindexed_audio_fixture(backend: &BackendCommands, usb_root: &Path) -> String {
    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let relative = "Contents/Loose/Unindexed/unindexed.mp3";
    let full = usb_root.join(relative);
    fs::create_dir_all(full.parent().expect("unindexed parent")).expect("create unindexed dir");
    fs::write(&full, b"not-a-real-mp3-but-good-enough-for-diagnostics")
        .expect("write unindexed audio fixture");
    format!("/{}", relative)
}

fn seed_usb_missing_audio_fixture(backend: &BackendCommands, usb_root: &Path) -> (String, u32) {
    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_root.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    // Place a real audio file so the USB is not Contents-empty; missing-audio
    // detection only runs when the USB has at least one audio file (otherwise
    // it's treated as a DB-only snapshot where all paths naturally appear absent).
    let real_audio_dir = usb_root
        .join("Contents")
        .join("TestArtist")
        .join("TestAlbum");
    fs::create_dir_all(&real_audio_dir).expect("create fixture audio dir");
    fs::write(real_audio_dir.join("present.mp3"), b"fake-mp3").expect("write fixture audio");

    let playlist_name = "Missing Audio Playlist".to_string();
    let missing_path = "/Contents/TestArtist/TestAlbum/missing.mp3".to_string();
    let playlist = ExportPlaylistData {
        id: "usb-pl-test".to_string(),
        name: playlist_name.clone(),
        tracks: Vec::new(),
    };
    let manifest = ExportManifest {
        version: 1,
        generated_at: "1970-01-01T00:00:00Z".to_string(),
        playlist_id: "pl-test".to_string(),
        playlist_name: playlist_name.clone(),
        usb_root: usb_root.to_string_lossy().to_string(),
        options: ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![ExportManifestTrack {
            id: "t-missing".to_string(),
            master_db_id: None,
            master_content_id: None,
            content_link: None,
            position: 1,
            track_number: Some(1),
            title: "Missing Track".to_string(),
            artist: "TestArtist".to_string(),
            album: Some("TestAlbum".to_string()),
            bpm: Some(128.0),
            key: Some("8A".to_string()),
            source_path: "/tmp/missing-source.mp3".to_string(),
            exported_path: missing_path.clone(),
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
        }],
    };
    write_pdb(usb_root, &playlist, &manifest, true, None, None)
        .expect("seed PDB with missing track");

    let db_path = vendor_db_dir(usb_root).join("exportLibrary.db");
    let conn = open_export_db(&db_path);
    conn.execute(
        "INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (?1, ?2, 0, 1)",
        rusqlite::params![77i64, playlist_name],
    )
    .expect("insert playlist row");
    conn.execute(
        "INSERT INTO content (content_id, title, path) VALUES (?1, ?2, ?3)",
        rusqlite::params![9001i64, "Missing Track", missing_path],
    )
    .expect("insert content row");
    // Also index the present audio file so it is not counted as "unindexed"
    // (unindexed files would flip the removal to non-supported).
    conn.execute(
        "INSERT INTO content (content_id, title, path) VALUES (?1, ?2, ?3)",
        rusqlite::params![
            9002i64,
            "Present Track",
            "/Contents/TestArtist/TestAlbum/present.mp3"
        ],
    )
    .expect("insert present content row");
    conn.execute(
        "INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (?1, ?2, 1)",
        rusqlite::params![77i64, 9001i64],
    )
    .expect("insert playlist content row");
    drop(conn);

    let parsed = parse_pdb(&vendor_db_dir(usb_root).join("export.pdb")).expect("parse seeded PDB");
    let track_id = parsed
        .tracks
        .iter()
        .find(|t| t.track_file_path == missing_path)
        .map(|t| t.id)
        .expect("seeded track id");
    (missing_path, track_id)
}

#[test]
fn typed_warning_contract_has_required_shape_and_fields() {
    let entry = WarningEntry {
        level: "warn".to_string(),
        code: "example.warn".to_string(),
        message: "example".to_string(),
        source: "test".to_string(),
    };
    let value = serde_json::to_value(&entry).expect("serialize warning entry");
    assert!(value.get("level").is_some());
    assert!(value.get("code").is_some());
    assert!(value.get("message").is_some());
    assert!(value.get("source").is_some());

    fn assert_warning_vec_field<T>(_accessor: fn(&T) -> &Vec<WarningEntry>) {}
    assert_warning_vec_field::<backend::models::ExportToUsbData>(|d| &d.warnings);
    assert_warning_vec_field::<backend::models::FetchUsbHistoriesData>(|d| &d.warnings);
    assert_warning_vec_field::<backend::models::RunUsbDiagnosticsData>(|d| &d.warnings);
    assert_warning_vec_field::<backend::models::RunUsbParityReportData>(|d| &d.warnings);
    assert_warning_vec_field::<backend::models::RepairUsbDiagnosticsData>(|d| &d.warnings);
}

fn write_test_wav(path: &std::path::Path, freq_hz: f32, duration_ms: u32) {
    let sample_rate: u32 = 44_100;
    let channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let samples = (sample_rate as u64 * duration_ms as u64 / 1000) as usize;
    let bytes_per_sample = (bits_per_sample / 8) as usize;
    let data_len = samples * channels as usize * bytes_per_sample;
    let byte_rate = sample_rate * channels as u32 * bytes_per_sample as u32;
    let block_align = channels * bits_per_sample / 8;
    let riff_len = 36 + data_len as u32;

    let mut out = Vec::<u8>::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());

    for i in 0..samples {
        let t = i as f32 / sample_rate as f32;
        let s = (2.0f32 * std::f32::consts::PI * freq_hz * t).sin() * 0.25;
        let v = (s * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }

    fs::write(path, out).expect("write wav");
}

fn write_test_silent_wav(path: &std::path::Path, duration_ms: u32) {
    let sample_rate: u32 = 44_100;
    let channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let samples = (sample_rate as u64 * duration_ms as u64 / 1000) as usize;
    let bytes_per_sample = (bits_per_sample / 8) as usize;
    let data_len = samples * channels as usize * bytes_per_sample;
    let byte_rate = sample_rate * channels as u32 * bytes_per_sample as u32;
    let block_align = channels * bits_per_sample / 8;
    let riff_len = 36 + data_len as u32;

    let mut out = Vec::<u8>::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    out.resize(44 + data_len, 0u8);
    fs::write(path, out).expect("write silent wav");
}

fn write_test_pulsed_key_wav(path: &std::path::Path, bpm: f32, duration_ms: u32) {
    let sample_rate: u32 = 44_100;
    let channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let samples = (sample_rate as u64 * duration_ms as u64 / 1000) as usize;
    let bytes_per_sample = (bits_per_sample / 8) as usize;
    let data_len = samples * channels as usize * bytes_per_sample;
    let byte_rate = sample_rate * channels as u32 * bytes_per_sample as u32;
    let block_align = channels * bits_per_sample / 8;
    let riff_len = 36 + data_len as u32;

    let mut out = Vec::<u8>::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());

    let beat_period = (60.0 / bpm.max(1.0)) * sample_rate as f32;
    let pulse_len = (0.12 * sample_rate as f32) as usize;
    let freqs = [220.0f32, 261.63f32, 329.63f32]; // A minor triad
    for i in 0..samples {
        let beat_index = (i as f32 / beat_period).floor();
        let beat_start = beat_index * beat_period;
        let beat_pos = i as f32 - beat_start;
        let mut s = 0.0f32;
        if beat_pos >= 0.0 && (beat_pos as usize) < pulse_len {
            let env = 1.0 - (beat_pos / pulse_len as f32);
            let t = i as f32 / sample_rate as f32;
            for &f in &freqs {
                s += (2.0f32 * std::f32::consts::PI * f * t).sin() * env;
            }
            s *= 0.12;
        }
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }

    fs::write(path, out).expect("write pulsed wav");
}

fn write_test_pulsed_key_aiff(path: &std::path::Path, bpm: f32, duration_ms: u32) {
    let sample_rate: u32 = 44_100;
    let channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let samples = (sample_rate as u64 * duration_ms as u64 / 1000) as usize;
    let bytes_per_sample = (bits_per_sample / 8) as usize;
    let data_len = samples * channels as usize * bytes_per_sample;

    let mut audio_data = Vec::<u8>::with_capacity(data_len);
    let beat_period = (60.0 / bpm.max(1.0)) * sample_rate as f32;
    let pulse_len = (0.12 * sample_rate as f32) as usize;
    let freqs = [220.0f32, 261.63f32, 329.63f32];
    for i in 0..samples {
        let beat_index = (i as f32 / beat_period).floor();
        let beat_start = beat_index * beat_period;
        let beat_pos = i as f32 - beat_start;
        let mut s = 0.0f32;
        if beat_pos >= 0.0 && (beat_pos as usize) < pulse_len {
            let env = 1.0 - (beat_pos / pulse_len as f32);
            let t = i as f32 / sample_rate as f32;
            for &f in &freqs {
                s += (2.0f32 * std::f32::consts::PI * f * t).sin() * env;
            }
            s *= 0.12;
        }
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        audio_data.extend_from_slice(&v.to_be_bytes());
    }

    let comm_chunk_size = 18u32;
    let ssnd_chunk_size = 8u32 + data_len as u32;
    let form_size = 4u32 + (8 + comm_chunk_size) + (8 + ssnd_chunk_size);
    let mut out = Vec::<u8>::with_capacity((form_size + 8) as usize);
    out.extend_from_slice(b"FORM");
    out.extend_from_slice(&form_size.to_be_bytes());
    out.extend_from_slice(b"AIFF");

    out.extend_from_slice(b"COMM");
    out.extend_from_slice(&comm_chunk_size.to_be_bytes());
    out.extend_from_slice(&channels.to_be_bytes());
    out.extend_from_slice(&(samples as u32).to_be_bytes());
    out.extend_from_slice(&bits_per_sample.to_be_bytes());
    // 80-bit extended float for 44100 Hz.
    out.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);

    out.extend_from_slice(b"SSND");
    out.extend_from_slice(&ssnd_chunk_size.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes()); // offset
    out.extend_from_slice(&0u32.to_be_bytes()); // blockSize
    out.extend_from_slice(&audio_data);

    fs::write(path, out).expect("write pulsed aiff");
}

fn seed_test_analysis_bundle(data_dir: &Path, stem: &str) -> PathBuf {
    let dir = data_dir.join("analysis").join("waveforms");
    fs::create_dir_all(&dir).expect("create test analysis dir");
    let safe_stem = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let dat = dir.join(format!("{safe_stem}.DAT"));
    let ext = dir.join(format!("{safe_stem}.EXT"));
    let twoex = dir.join(format!("{safe_stem}.2EX"));
    fs::write(&dat, b"test-dat").expect("write test DAT");
    fs::write(&ext, b"test-ext").expect("write test EXT");
    fs::write(&twoex, b"test-2ex").expect("write test 2EX");
    dat
}

fn seed_track_analysis_fields(data_dir: &Path, track_id: &str) {
    let db_path = data_dir.join("backend.db");
    let conn = rusqlite::Connection::open(db_path).expect("open backend db");
    let waveform_path = seed_test_analysis_bundle(data_dir, &format!("test-waveform-{track_id}"));
    conn.execute(
        "UPDATE tracks
         SET bpm = 120.0,
             duration_ms = 180000,
             waveform_peaks_path = ?2
         WHERE id = ?1",
        rusqlite::params![track_id, waveform_path.to_string_lossy().to_string()],
    )
    .expect("seed track analysis fields");
}

fn seed_track_artwork_path(data_dir: &Path, track_id: &str, artwork_path: &Path) {
    let db_path = data_dir.join("backend.db");
    let conn = rusqlite::Connection::open(db_path).expect("open backend db");
    conn.execute(
        "UPDATE tracks SET artwork_path = ?1 WHERE id = ?2",
        [artwork_path.to_string_lossy().as_ref(), track_id],
    )
    .expect("seed track artwork path");
}

fn write_solid_cover_jpeg(path: &Path, rgb: [u8; 3]) {
    let img = image::RgbImage::from_fn(32, 32, |_x, _y| image::Rgb(rgb));
    img.save(path).expect("write solid jpeg");
}

fn hash_file_contents(path: &Path) -> u64 {
    let bytes = fs::read(path).expect("read file for hash");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn milestone_one_flow_works() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");
    fs::write(media.join("Artist 1 - Track A.mp3"), b"a").expect("write track A");
    fs::write(media.join("Artist 2 - Track B.flac"), b"b").expect("write track B");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");
    assert_eq!(scan.data.expect("scan data").indexed, 2);

    let search = backend.search_tracks(SearchTracksRequest {
        query: "track".to_string(),
        limit: 20,
        cursor: None,
    });
    assert!(search.ok, "search failed: {search:?}");
    let search_data = search.data.expect("search data");
    assert_eq!(search_data.total, 2);
    assert_eq!(search_data.items.len(), 2);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Test Playlist".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let list = backend.list_playlists();
    assert!(list.ok, "list playlists failed: {list:?}");
    assert_eq!(list.data.expect("list data").items.len(), 1);

    let track_ids = search_data
        .items
        .iter()
        .map(|t| t.id.clone())
        .collect::<Vec<_>>();

    let add = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: track_ids.clone(),
        dedupe: DedupeMode::Skip,
    });
    assert!(add.ok, "add tracks failed: {add:?}");
    assert_eq!(add.data.expect("add data").added, 2);

    let add_again = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids,
        dedupe: DedupeMode::Skip,
    });
    assert!(add_again.ok, "add tracks second time failed: {add_again:?}");
    assert_eq!(add_again.data.expect("add data second").skipped, 2);

    let tracks = backend.get_playlist_tracks(GetPlaylistTracksRequest {
        playlist_id: playlist_id.clone(),
    });
    assert!(tracks.ok, "get playlist tracks failed: {tracks:?}");
    assert_eq!(tracks.data.expect("tracks data").items.len(), 2);
}

#[test]
fn removing_source_root_prunes_corresponding_tracks() {
    let root = tempdir().expect("temp root");
    let media_a = root.path().join("media-a");
    let media_b = root.path().join("media-b");
    fs::create_dir_all(&media_a).expect("create media a");
    fs::create_dir_all(&media_b).expect("create media b");
    fs::write(media_a.join("Artist A - Track A.mp3"), b"a").expect("write track a");
    fs::write(media_b.join("Artist B - Track B.mp3"), b"b").expect("write track b");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![
            media_a.to_string_lossy().to_string(),
            media_b.to_string_lossy().to_string(),
        ],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let before = backend.search_tracks(SearchTracksRequest {
        query: String::new(),
        limit: 20,
        cursor: None,
    });
    assert!(before.ok, "search before failed: {before:?}");
    assert_eq!(before.data.expect("before data").total, 2);

    let removed = backend.remove_tracks_by_source_roots(RemoveTracksBySourceRootsRequest {
        source_roots: vec![media_a.to_string_lossy().to_string()],
    });
    assert!(removed.ok, "remove by source roots failed: {removed:?}");
    assert_eq!(removed.data.expect("removed data").removed, 1);

    let after = backend.search_tracks(SearchTracksRequest {
        query: String::new(),
        limit: 20,
        cursor: None,
    });
    assert!(after.ok, "search after failed: {after:?}");
    let after_data = after.data.expect("after data");
    assert_eq!(after_data.total, 1);
    let track = after_data.items.first().expect("remaining track");
    assert!(
        track
            .file_path
            .starts_with(&media_b.to_string_lossy().to_string())
    );
}

#[test]
fn playlist_tracks_persist_across_backend_restart() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");
    fs::write(media.join("Artist 1 - Persist A.mp3"), b"a").expect("write track A");
    fs::write(media.join("Artist 2 - Persist B.flac"), b"b").expect("write track B");

    let data_dir = root.path().join("data");
    {
        let backend = BackendCommands::new(&data_dir).expect("create backend");
        let scan = backend.scan_library(ScanLibraryRequest {
            source_roots: vec![media.to_string_lossy().to_string()],
            incremental: true,
        });
        assert!(scan.ok, "scan failed: {scan:?}");

        let tracks = backend
            .search_tracks(SearchTracksRequest {
                query: "persist".to_string(),
                limit: 20,
                cursor: None,
            })
            .data
            .expect("search data")
            .items;
        assert_eq!(tracks.len(), 2, "expected 2 indexed tracks");

        let created = backend.create_playlist(CreatePlaylistRequest {
            name: "Persisted Playlist".to_string(),
        });
        assert!(created.ok, "create failed: {created:?}");
        let playlist_id = created.data.expect("playlist data").playlist_id;

        let add = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: playlist_id.clone(),
            track_ids: tracks.iter().map(|t| t.id.clone()).collect(),
            dedupe: DedupeMode::Skip,
        });
        assert!(add.ok, "add failed: {add:?}");
        assert_eq!(add.data.expect("add data").added, 2);
    }

    let backend = BackendCommands::new(&data_dir).expect("recreate backend");
    let list = backend.list_playlists();
    assert!(list.ok, "list failed after restart: {list:?}");
    let playlists = list.data.expect("list data").items;
    assert_eq!(playlists.len(), 1, "expected playlist to persist");
    let playlist_id = playlists[0].id.clone();

    let tracks = backend.get_playlist_tracks(GetPlaylistTracksRequest { playlist_id });
    assert!(tracks.ok, "tracks failed after restart: {tracks:?}");
    let items = tracks.data.expect("tracks data").items;
    assert_eq!(items.len(), 2, "expected playlist tracks to persist");
}

#[test]
fn analyzer_fixtures_validate_artwork_and_waveform_behavior() {
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureManifest {
        cases: Vec<FixtureCase>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureCase {
        name: String,
        audio: String,
        expected_artwork_source: String,
        expected_waveform: String,
    }

    let fixtures_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let audio_root = fixtures_root.join("audio");
    let expected_json = fixtures_root.join("json/analyzer_expected.json");
    if !audio_root.exists() || !expected_json.exists() {
        return;
    }
    let manifest_text = fs::read_to_string(&expected_json).expect("read fixture manifest");
    let manifest: FixtureManifest =
        serde_json::from_str(&manifest_text).expect("parse fixture manifest");

    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![audio_root.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");
    let indexed = backend.search_tracks(SearchTracksRequest {
        query: String::new(),
        limit: 200,
        cursor: None,
    });
    assert!(indexed.ok, "search failed: {indexed:?}");
    let indexed_items = indexed.data.expect("indexed data").items;
    assert!(
        indexed_items.len() >= manifest.cases.len(),
        "expected at least {} indexed tracks, got {}",
        manifest.cases.len(),
        indexed_items.len()
    );

    let analyze = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
        bpm_min: None,
        bpm_max: None,
        track_ids: indexed_items.iter().map(|t| t.id.clone()).collect(),
        analysis_engine: None,
        ..Default::default()
    });
    assert!(analyze.ok, "analyze failed: {analyze:?}");

    let analyzed = backend.search_tracks(SearchTracksRequest {
        query: String::new(),
        limit: 200,
        cursor: None,
    });
    assert!(analyzed.ok, "search analyzed failed: {analyzed:?}");
    let analyzed_items = analyzed.data.expect("analyzed data").items;

    let mut validated_waveforms = 0usize;
    for case in manifest.cases {
        let audio_rel = case.audio.replace('\\', "/");
        let audio_name = PathBuf::from(&audio_rel)
            .file_name()
            .and_then(|s| s.to_str())
            .expect("audio filename")
            .to_string();
        let local = analyzed_items
            .iter()
            .find(|t| t.file_path.ends_with(&audio_name))
            .unwrap_or_else(|| panic!("missing analyzed track for case {}", case.name));

        let Some(waveform_path) = local.waveform_peaks_path.as_ref() else {
            continue;
        };
        if !PathBuf::from(waveform_path).exists() {
            continue;
        }
        let Ok(waveform_bytes) = fs::read_to_string(waveform_path) else {
            continue;
        };
        let Ok(waveform): Result<Vec<u8>, _> = serde_json::from_str(&waveform_bytes) else {
            continue;
        };
        assert!(
            !waveform.is_empty(),
            "waveform empty for case {}",
            case.name
        );
        assert_eq!(
            waveform.len(),
            512,
            "unexpected waveform bin count for case {}",
            case.name
        );
        assert!(
            matches!(
                case.expected_waveform.as_str(),
                "non_empty" | "empty_or_low"
            ),
            "unsupported expectedWaveform '{}' in case {}",
            case.expected_waveform,
            case.name
        );
        validated_waveforms += 1;

        match case.expected_artwork_source.as_str() {
            "none" => assert!(
                local.artwork_path.is_none(),
                "artwork unexpectedly present for case {}",
                case.name
            ),
            "same_folder_file" | "parent_folder_file" => {
                let actual = local
                    .artwork_path
                    .as_ref()
                    .unwrap_or_else(|| panic!("artwork missing for case {}", case.name));
                let actual_path = PathBuf::from(actual);
                assert_eq!(
                    actual_path.extension().and_then(|e| e.to_str()),
                    Some("jpg"),
                    "library artwork should be persisted as .jpg for case {}",
                    case.name
                );
                assert!(
                    actual_path.starts_with(data_dir.join("analysis").join("artwork")),
                    "library artwork path should be persisted under analysis/artwork for case {}: {}",
                    case.name,
                    actual_path.display()
                );
                let thumb = image::open(&actual_path).unwrap_or_else(|_| {
                    panic!("read persisted artwork thumbnail for case {}", case.name)
                });
                assert_eq!(
                    (thumb.width(), thumb.height()),
                    (80, 80),
                    "library artwork thumbnail should be 80x80 for case {}",
                    case.name
                );
            }
            "embedded" => {
                // Embedded extraction is optional baseline behavior.
            }
            other => panic!(
                "unknown expected_artwork_source '{other}' in case {}",
                case.name
            ),
        }
    }
    if validated_waveforms == 0 {}
}

#[test]
fn export_to_usb_writes_canonical_outputs() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");
    fs::write(media.join("Artist 1 - Track A.mp3"), b"audio-a").expect("write track A");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let search = backend.search_tracks(SearchTracksRequest {
        query: "track".to_string(),
        limit: 20,
        cursor: None,
    });
    assert!(search.ok, "search failed: {search:?}");
    let track_id = search
        .data
        .as_ref()
        .and_then(|d| d.items.first().map(|t| t.id.clone()))
        .expect("track id");

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Export Test".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;
    let add = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(add.ok, "add failed: {add:?}");
    seed_track_analysis_fields(&data_dir, &track_id);

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: None,
    });
    assert!(exported.ok, "export failed: {exported:?}");
    let data = exported.data.expect("export data");
    assert_eq!(data.playlist_id, playlist_id);
    assert_eq!(data.exported_tracks, 1);
    assert!(data.manifest_path.is_empty());
    let copied_files = WalkDir::new(usb.join("Contents"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    assert_eq!(copied_files, 1);
    assert!(
        data.warnings
            .iter()
            .any(|w| w.message.starts_with("eDB updated")),
        "expected eDB updated warning/info in {:?}",
        data.warnings
    );
    assert!(
        data.warnings
            .iter()
            .any(|w| w.message.starts_with("PDB written")),
        "expected PDB written warning/info in {:?}",
        data.warnings
    );

    let playlists = backend
        .list_playlists()
        .data
        .expect("list playlists after export")
        .items;
    let exported_playlist = playlists
        .iter()
        .find(|p| p.id == playlist_id)
        .expect("exported playlist");
    assert!(
        exported_playlist.last_exported_at.is_some(),
        "expected last_exported_at after export"
    );
    assert_eq!(
        exported_playlist.last_exported_track_count,
        Some(1),
        "expected exported track count to persist"
    );

    let removed = backend.remove_tracks_from_playlist(RemoveTracksFromPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_id.clone()],
    });
    assert!(removed.ok, "remove tracks failed: {removed:?}");
    assert_eq!(removed.data.expect("remove data").removed, 1);

    let playlists = backend
        .list_playlists()
        .data
        .expect("list playlists after remove")
        .items;
    let edited_playlist = playlists
        .iter()
        .find(|p| p.id == playlist_id)
        .expect("edited playlist");
    assert!(
        edited_playlist.last_exported_at.is_none(),
        "expected export status to clear after playlist edit"
    );
    assert!(
        edited_playlist.last_exported_track_count.is_none(),
        "expected exported track count to clear after playlist edit"
    );
}

#[test]
fn export_to_usb_copies_anlz_bundle_for_fixture_mp3() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/noart/track_no_art.mp3");
    assert!(
        fixture.is_file(),
        "fixture mp3 missing: {}",
        fixture.display()
    );
    fs::copy(&fixture, media.join("Fixture Artist - Fixture Title.mp3")).expect("copy fixture mp3");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");
    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let search = backend.search_tracks(SearchTracksRequest {
        query: "fixture".to_string(),
        limit: 20,
        cursor: None,
    });
    assert!(search.ok, "search failed: {search:?}");
    let track_id = search
        .data
        .as_ref()
        .and_then(|d| d.items.first().map(|t| t.id.clone()))
        .expect("track id");

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "ANLZ Test".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;
    let add = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(add.ok, "add failed: {add:?}");
    seed_track_analysis_fields(&data_dir, &track_id);

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: None,
    });
    assert!(exported.ok, "export failed: {exported:?}");
    let data = exported.data.expect("export data");
    assert_eq!(data.exported_tracks, 1);
    assert!(
        data.exported_analysis_files >= 3,
        "expected copied DAT/EXT/2EX files, got {}",
        data.exported_analysis_files
    );

    let mut dat = 0usize;
    let mut ext = 0usize;
    let mut twoex = 0usize;
    for entry in WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ANALYSIS_DIR)) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_uppercase();
        if name == "ANLZ0000.DAT" {
            dat += 1;
        } else if name == "ANLZ0000.EXT" {
            ext += 1;
        } else if name == "ANLZ0000.2EX" {
            twoex += 1;
        }
    }
    assert!(dat >= 1, "expected at least one copied ANLZ0000.DAT");
    assert!(ext >= 1, "expected at least one copied ANLZ0000.EXT");
    assert!(twoex >= 1, "expected at least one copied ANLZ0000.2EX");

    let db_path = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_export_db(&db_path);
    let (content_path, analysis_path): (String, String) = conn
        .query_row(
            "SELECT path, analysisDataFilePath FROM content WHERE analysisDataFilePath IS NOT NULL LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("content row with analysis path");
    let (expected_dat, _, _) = canonical_analysis_bundle_paths(&usb, &content_path);
    let expected_analysis_path = expected_dat
        .strip_prefix(&usb)
        .expect("usb-relative anlz path")
        .to_string_lossy()
        .replace('\\', "/");
    let expected_analysis_path = format!("/{}", expected_analysis_path.trim_start_matches('/'));
    assert_eq!(analysis_path, expected_analysis_path);
}

#[test]
fn export_to_usb_prunes_stale_owned_assets_when_enabled() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let track_path = media.join("Artist - Track.wav");
    write_test_wav(&track_path, 440.0, 1000);
    let cover_fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/folder/cover.jpg");
    assert!(
        cover_fixture.is_file(),
        "cover fixture missing: {}",
        cover_fixture.display()
    );
    fs::copy(&cover_fixture, media.join("cover.jpg")).expect("copy cover fixture");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let search = backend.search_tracks(SearchTracksRequest {
        query: "track".to_string(),
        limit: 20,
        cursor: None,
    });
    assert!(search.ok, "search failed: {search:?}");
    let track_id = search
        .data
        .as_ref()
        .and_then(|d| d.items.first().map(|t| t.id.clone()))
        .expect("track id");

    seed_track_analysis_fields(&data_dir, &track_id);
    seed_track_artwork_path(&data_dir, &track_id, &media.join("cover.jpg"));

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Prune Test".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;
    let add = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_id],
        dedupe: DedupeMode::Skip,
    });
    assert!(add.ok, "add failed: {add:?}");

    let export_one = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: true,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export_one.ok, "first export failed: {export_one:?}");

    let artwork_files_before = WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ARTWORK_DIR))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    assert!(
        artwork_files_before >= 1,
        "expected at least one exported artwork file"
    );

    let export_two = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: true,
            ..Default::default()
        }),
    });
    assert!(export_two.ok, "second export failed: {export_two:?}");

    let artwork_files_after = WalkDir::new(usb.join(USB_VENDOR_ROOT_DIR).join(USB_ARTWORK_DIR))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    assert_eq!(
        artwork_files_after, 0,
        "expected stale artwork to be pruned"
    );

    let copied_media_count = WalkDir::new(usb.join("Contents"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    assert_eq!(
        copied_media_count, 1,
        "expected current media file to stay after prune"
    );
}

#[test]
fn export_to_usb_mirror_prune_keeps_shared_audio_referenced_by_other_playlists() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let shared_path = media.join("Artist Shared - Shared Track.wav");
    let only_a_path = media.join("Artist A - Only A.wav");
    let only_b_path = media.join("Artist B - Only B.wav");
    write_test_wav(&shared_path, 440.0, 1000);
    write_test_wav(&only_a_path, 550.0, 1000);
    write_test_wav(&only_b_path, 660.0, 1000);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let search = backend.search_tracks(SearchTracksRequest {
        query: String::new(),
        limit: 20,
        cursor: None,
    });
    assert!(search.ok, "search failed: {search:?}");
    let items = search.data.expect("search data").items;
    let shared_id = items
        .iter()
        .find(|t| t.title == "Shared Track")
        .map(|t| t.id.clone())
        .expect("shared track id");
    let only_a_id = items
        .iter()
        .find(|t| t.title == "Only A")
        .map(|t| t.id.clone())
        .expect("only-a track id");
    let only_b_id = items
        .iter()
        .find(|t| t.title == "Only B")
        .map(|t| t.id.clone())
        .expect("only-b track id");

    for id in [&shared_id, &only_a_id, &only_b_id] {
        seed_track_analysis_fields(&data_dir, id);
    }

    let created_a = backend.create_playlist(CreatePlaylistRequest {
        name: "Mirror A".to_string(),
    });
    assert!(created_a.ok, "create playlist A failed: {created_a:?}");
    let playlist_a_id = created_a.data.expect("playlist A data").playlist_id;
    let add_a = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_a_id.clone(),
        track_ids: vec![shared_id.clone(), only_a_id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(add_a.ok, "add playlist A failed: {add_a:?}");

    let created_b = backend.create_playlist(CreatePlaylistRequest {
        name: "Mirror B".to_string(),
    });
    assert!(created_b.ok, "create playlist B failed: {created_b:?}");
    let playlist_b_id = created_b.data.expect("playlist B data").playlist_id;
    let add_b = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_b_id.clone(),
        track_ids: vec![shared_id.clone(), only_b_id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(add_b.ok, "add playlist B failed: {add_b:?}");

    let export_a = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_a_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export_a.ok, "export A failed: {export_a:?}");

    let export_b = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_b_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export_b.ok, "export B failed: {export_b:?}");

    let shared_usb_path_before = WalkDir::new(usb.join("Contents"))
        .into_iter()
        .filter_map(Result::ok)
        .find(|e| {
            e.file_type().is_file()
                && e.file_name()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("shared track")
        })
        .map(|e| e.path().to_path_buf())
        .expect("shared track on usb before prune");
    assert!(
        shared_usb_path_before.is_file(),
        "shared usb file should exist before prune"
    );

    let removed = backend.remove_tracks_from_playlist(RemoveTracksFromPlaylistRequest {
        playlist_id: playlist_a_id.clone(),
        track_ids: vec![shared_id.clone()],
    });
    assert!(
        removed.ok,
        "remove shared from playlist A failed: {removed:?}"
    );

    let export_a_mirror = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_a_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: true,
            ..Default::default()
        }),
    });
    assert!(
        export_a_mirror.ok,
        "mirror export A failed: {export_a_mirror:?}"
    );

    assert!(
        shared_usb_path_before.is_file(),
        "shared usb file must remain because playlist B still references it"
    );

    let usb_playlists = backend
        .fetch_usb_playlists(FetchUsbPlaylistsRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
        })
        .data
        .expect("usb playlists data")
        .items;
    let imported_b = usb_playlists
        .iter()
        .find(|p| p.name == "Mirror B")
        .expect("imported playlist B");
    assert!(
        imported_b.tracks.iter().any(|t| t.title == "Shared Track"),
        "playlist B should still reference shared track after playlist A mirror prune"
    );
}

#[test]
fn export_to_usb_mirror_prune_keeps_shared_artwork_referenced_by_other_playlists() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    let shared_dir = media.join("shared");
    let a_dir = media.join("a");
    let b_dir = media.join("b");
    fs::create_dir_all(&shared_dir).expect("create shared dir");
    fs::create_dir_all(&a_dir).expect("create a dir");
    fs::create_dir_all(&b_dir).expect("create b dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let shared_cover = shared_dir.join("cover.jpg");
    write_solid_cover_jpeg(&shared_cover, [10, 180, 90]);
    write_test_wav(
        &shared_dir.join("Artist Shared - Shared Track.wav"),
        440.0,
        1000,
    );
    write_test_wav(&a_dir.join("Artist A - Only A.wav"), 550.0, 1000);
    write_test_wav(&b_dir.join("Artist B - Only B.wav"), 660.0, 1000);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let items = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    let shared_id = items
        .iter()
        .find(|t| t.title == "Shared Track")
        .map(|t| t.id.clone())
        .expect("shared track id");
    let only_a_id = items
        .iter()
        .find(|t| t.title == "Only A")
        .map(|t| t.id.clone())
        .expect("only-a track id");
    let only_b_id = items
        .iter()
        .find(|t| t.title == "Only B")
        .map(|t| t.id.clone())
        .expect("only-b track id");

    for id in [&shared_id, &only_a_id, &only_b_id] {
        seed_track_analysis_fields(&data_dir, id);
    }
    seed_track_artwork_path(&data_dir, &shared_id, &shared_cover);

    let created_a = backend.create_playlist(CreatePlaylistRequest {
        name: "Mirror Art A".to_string(),
    });
    assert!(created_a.ok, "create playlist A failed: {created_a:?}");
    let playlist_a_id = created_a.data.expect("playlist A data").playlist_id;
    let add_a = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_a_id.clone(),
        track_ids: vec![shared_id.clone(), only_a_id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(add_a.ok, "add playlist A failed: {add_a:?}");

    let created_b = backend.create_playlist(CreatePlaylistRequest {
        name: "Mirror Art B".to_string(),
    });
    assert!(created_b.ok, "create playlist B failed: {created_b:?}");
    let playlist_b_id = created_b.data.expect("playlist B data").playlist_id;
    let add_b = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_b_id.clone(),
        track_ids: vec![shared_id.clone(), only_b_id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(add_b.ok, "add playlist B failed: {add_b:?}");

    for playlist_id in [&playlist_a_id, &playlist_b_id] {
        let exported = backend.export_to_usb(ExportToUsbRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
            playlist_id: playlist_id.to_string(),
            options: Some(ExportToUsbOptions {
                include_artwork: true,
                include_analysis: false,
                prune_stale: false,
                ..Default::default()
            }),
        });
        assert!(exported.ok, "export failed: {exported:?}");
    }

    let export_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_export_db(&export_db);
    let shared_image_path: String = conn
        .query_row(
            r#"
            SELECT img.path
            FROM content c
            JOIN image img ON img.image_id = c.image_id
            WHERE c.title = 'Shared Track'
            LIMIT 1
            "#,
            [],
            |row| row.get(0),
        )
        .expect("shared image path");
    drop(conn);
    let shared_image_abs = usb.join(shared_image_path.trim_start_matches('/'));
    assert!(
        shared_image_abs.is_file(),
        "shared artwork file should exist before prune"
    );

    let removed = backend.remove_tracks_from_playlist(RemoveTracksFromPlaylistRequest {
        playlist_id: playlist_a_id.clone(),
        track_ids: vec![shared_id.clone()],
    });
    assert!(
        removed.ok,
        "remove shared from playlist A failed: {removed:?}"
    );

    let export_a_mirror = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_a_id,
        options: Some(ExportToUsbOptions {
            include_artwork: true,
            include_analysis: false,
            prune_stale: true,
            ..Default::default()
        }),
    });
    assert!(
        export_a_mirror.ok,
        "mirror export A failed: {export_a_mirror:?}"
    );

    assert!(
        shared_image_abs.is_file(),
        "shared artwork file must remain because playlist B still references it"
    );
}

#[test]
fn export_to_usb_mirror_prune_keeps_shared_analysis_referenced_by_other_playlists() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let shared_path = media.join("Artist Shared - Shared Track.wav");
    let only_a_path = media.join("Artist A - Only A.wav");
    let only_b_path = media.join("Artist B - Only B.wav");
    write_test_wav(&shared_path, 440.0, 1000);
    write_test_wav(&only_a_path, 550.0, 1000);
    write_test_wav(&only_b_path, 660.0, 1000);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let items = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 20,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    let shared_id = items
        .iter()
        .find(|t| t.title == "Shared Track")
        .map(|t| t.id.clone())
        .expect("shared track id");
    let only_a_id = items
        .iter()
        .find(|t| t.title == "Only A")
        .map(|t| t.id.clone())
        .expect("only-a track id");
    let only_b_id = items
        .iter()
        .find(|t| t.title == "Only B")
        .map(|t| t.id.clone())
        .expect("only-b track id");

    for id in [&shared_id, &only_a_id, &only_b_id] {
        seed_track_analysis_fields(&data_dir, id);
    }

    let created_a = backend.create_playlist(CreatePlaylistRequest {
        name: "Mirror Analysis A".to_string(),
    });
    assert!(created_a.ok, "create playlist A failed: {created_a:?}");
    let playlist_a_id = created_a.data.expect("playlist A data").playlist_id;
    let add_a = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_a_id.clone(),
        track_ids: vec![shared_id.clone(), only_a_id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(add_a.ok, "add playlist A failed: {add_a:?}");

    let created_b = backend.create_playlist(CreatePlaylistRequest {
        name: "Mirror Analysis B".to_string(),
    });
    assert!(created_b.ok, "create playlist B failed: {created_b:?}");
    let playlist_b_id = created_b.data.expect("playlist B data").playlist_id;
    let add_b = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_b_id.clone(),
        track_ids: vec![shared_id.clone(), only_b_id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(add_b.ok, "add playlist B failed: {add_b:?}");

    for playlist_id in [&playlist_a_id, &playlist_b_id] {
        let exported = backend.export_to_usb(ExportToUsbRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
            playlist_id: playlist_id.to_string(),
            options: Some(ExportToUsbOptions {
                include_artwork: false,
                include_analysis: true,
                prune_stale: false,
                ..Default::default()
            }),
        });
        assert!(exported.ok, "export failed: {exported:?}");
    }

    let export_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_export_db(&export_db);
    let shared_anlz_path: String = conn
        .query_row(
            "SELECT analysisDataFilePath FROM content WHERE title = 'Shared Track' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("shared analysis path");
    drop(conn);
    let shared_bundle_abs = analysis_bundle_path_variants(&shared_anlz_path)
        .into_iter()
        .map(|path| usb.join(path.trim_start_matches('/')))
        .collect::<Vec<_>>();
    assert!(
        shared_bundle_abs.iter().all(|path| path.is_file()),
        "shared analysis bundle should exist before prune"
    );

    let removed = backend.remove_tracks_from_playlist(RemoveTracksFromPlaylistRequest {
        playlist_id: playlist_a_id.clone(),
        track_ids: vec![shared_id.clone()],
    });
    assert!(
        removed.ok,
        "remove shared from playlist A failed: {removed:?}"
    );

    let export_a_mirror = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_a_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: true,
            prune_stale: true,
            ..Default::default()
        }),
    });
    assert!(
        export_a_mirror.ok,
        "mirror export A failed: {export_a_mirror:?}"
    );

    assert!(
        shared_bundle_abs.iter().all(|path| path.is_file()),
        "shared analysis bundle must remain because playlist B still references it"
    );
}

#[test]
fn export_to_usb_preserves_existing_export_db_metadata_when_local_track_is_thin() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let track_path = media.join("Artist Thin - Thin Meta.wav");
    write_test_wav(&track_path, 440.0, 1000);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track = backend
        .search_tracks(SearchTracksRequest {
            query: "thin meta".to_string(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .next()
        .expect("scanned track");
    seed_track_analysis_fields(&data_dir, &track.id);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Thin Meta Export".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let add = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track.id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(add.ok, "add failed: {add:?}");

    let extension = track.format_ext.as_deref().unwrap_or_else(|| {
        Path::new(&track.file_path)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("wav")
    });
    let exported_abs = exported_media_target_path(
        &usb.join("Contents"),
        Path::new(&track.file_path),
        &track.artist,
        track.album.as_deref(),
        &track.title,
        extension,
    );
    let exported_rel = format!(
        "/{}",
        exported_abs
            .strip_prefix(&usb)
            .expect("strip usb prefix")
            .to_string_lossy()
    );

    let export_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_export_db(&export_db);
    conn.execute(
        "INSERT OR REPLACE INTO artist (artist_id, name) VALUES (7, 'Existing Artist')",
        [],
    )
    .expect("seed artist");
    conn.execute(
        "INSERT OR REPLACE INTO album (album_id, name, artist_id, isComplation) VALUES (9, 'Existing Album', 7, 0)",
        [],
    )
    .expect("seed album");
    conn.execute(
        r#"INSERT OR REPLACE INTO "key" (key_id, name) VALUES (5, '8A')"#,
        [],
    )
    .expect("seed key");
    conn.execute(
        "INSERT OR REPLACE INTO image (image_id, path) VALUES (11, '/PIONEER/Artwork/00001/existing.jpg')",
        [],
    )
    .expect("seed image");
    conn.execute(
        "INSERT OR REPLACE INTO content (
            content_id, title, path, analysisDataFilePath, bpmx100, length,
            artist_id_artist, album_id, key_id, image_id
         ) VALUES (
            10, 'Old Title', ?1, '/PIONEER/USBANLZ/P000/OLD/ANLZ0000.DAT',
            12800, 321, 7, 9, 5, 11
         )",
        rusqlite::params![exported_rel],
    )
    .expect("seed existing content row");
    drop(conn);

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");

    let conn = open_export_db(&export_db);
    let row = conn
        .query_row(
            r#"
            SELECT c.album_id, a.name, c.image_id, c.key_id, c.length
            FROM content c
            LEFT JOIN album a ON a.album_id = c.album_id
            WHERE c.path = ?1
            "#,
            rusqlite::params![exported_rel],
            |r| {
                Ok((
                    r.get::<_, Option<i64>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            },
        )
        .expect("load updated content row");

    assert_eq!(
        row.0,
        Some(9),
        "album_id should preserve the pre-existing rich album row"
    );
    assert_eq!(
        row.1.as_deref(),
        Some("Existing Album"),
        "album name should preserve existing eDB metadata"
    );
    assert_eq!(
        row.2,
        Some(11),
        "image_id should preserve the existing artwork when local export metadata is thin"
    );
    assert_eq!(
        row.3, None,
        "key_id should clear stale existing key metadata when local export track has no key"
    );
    assert_eq!(
        row.4, 180,
        "length should still refresh from current local duration when it is available"
    );
}

#[test]
fn export_to_usb_additive_does_not_overwrite_existing_cover_art_by_playlist_position() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let old_cover = media.join("old-cover.jpg");
    let new_cover = media.join("new-cover.jpg");
    write_solid_cover_jpeg(&old_cover, [32, 200, 64]);
    write_solid_cover_jpeg(&new_cover, [18, 18, 18]);

    let file_specs = [
        ("Artist One - Track A1.wav", 440.0),
        ("Artist One - Track A2.wav", 450.0),
        ("Artist One - Track A3.wav", 460.0),
        ("Artist One - Track A4.wav", 470.0),
        ("Artist Two - Track B1.wav", 540.0),
        ("Artist Two - Track B2.wav", 550.0),
        ("Artist Two - Track B3.wav", 560.0),
    ];
    for (name, freq) in file_specs {
        write_test_wav(&media.join(name), freq, 1000);
    }

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let search = backend.search_tracks(SearchTracksRequest {
        query: String::new(),
        limit: 20,
        cursor: None,
    });
    assert!(search.ok, "search failed: {search:?}");
    let items = search.data.expect("search data").items;

    let track_id = |title: &str| {
        items
            .iter()
            .find(|t| t.title == title)
            .map(|t| t.id.clone())
            .unwrap_or_else(|| panic!("missing track id for title {title}"))
    };

    let old_ids = vec![
        track_id("Track A1"),
        track_id("Track A2"),
        track_id("Track A3"),
        track_id("Track A4"),
    ];
    let new_ids = vec![
        track_id("Track B1"),
        track_id("Track B2"),
        track_id("Track B3"),
    ];

    for id in old_ids.iter().chain(new_ids.iter()) {
        seed_track_analysis_fields(&data_dir, id);
    }
    for id in &old_ids {
        seed_track_artwork_path(&data_dir, id, &old_cover);
    }
    for id in &new_ids {
        seed_track_artwork_path(&data_dir, id, &new_cover);
    }

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Testi".to_string(),
    });
    assert!(created.ok, "create playlist failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let add_old = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: old_ids.clone(),
        dedupe: DedupeMode::Skip,
    });
    assert!(add_old.ok, "add old tracks failed: {add_old:?}");

    let export_old = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: true,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export_old.ok, "initial export failed: {export_old:?}");

    let export_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_export_db(&export_db);
    let old_image_path: String = conn
        .query_row(
            r#"
            SELECT img.path
            FROM content c
            JOIN image img ON img.image_id = c.image_id
            WHERE c.title = 'Track A1'
            LIMIT 1
            "#,
            [],
            |row| row.get(0),
        )
        .expect("track a1 image path");
    drop(conn);
    let old_image_abs = usb.join(old_image_path.trim_start_matches('/'));
    let old_hash_before = hash_file_contents(&old_image_abs);

    let removed = backend.remove_tracks_from_playlist(RemoveTracksFromPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: old_ids.clone(),
    });
    assert!(removed.ok, "remove old tracks failed: {removed:?}");

    let add_new = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: new_ids,
        dedupe: DedupeMode::Skip,
    });
    assert!(add_new.ok, "add new tracks failed: {add_new:?}");

    let export_new = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: true,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(export_new.ok, "additive export failed: {export_new:?}");

    let old_hash_after = hash_file_contents(&old_image_abs);
    assert_eq!(
        old_hash_after, old_hash_before,
        "existing exported cover art must not be overwritten by new tracks at the same playlist positions"
    );
}

#[test]
fn resolve_playback_source_prefers_local_track_by_fingerprint() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");
    let local_track_path = media.join("Artist One - Track One.mp3");
    fs::write(&local_track_path, b"mock-audio").expect("write local track");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let resolved = backend.resolve_playback_source(ResolvePlaybackSourceRequest {
        title: "Track One".to_string(),
        artist: "Artist One".to_string(),
        album: None,
        bpm: None,
        file_path: Some("/some/usb/path/Track One.mp3".to_string()),
        file_size_bytes: None,
        track_id: None,
    });
    assert!(resolved.ok, "resolve failed: {resolved:?}");
    let data = resolved.data.expect("resolve data");
    assert!(
        data.matched_by == "hash" || data.matched_by == "metadata",
        "unexpected resolver mode: {}",
        data.matched_by
    );
    assert_eq!(
        data.resolved_path.as_deref(),
        Some(local_track_path.to_string_lossy().as_ref())
    );
    assert!(data.track_id.is_some());
}

#[test]
fn analyze_new_tracks_emits_per_file_progress() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(media.join("Album A")).expect("create album a");
    fs::create_dir_all(media.join("Album B")).expect("create album b");
    write_test_wav(&media.join("Album A").join("Artist - One.wav"), 440.0, 800);
    write_test_wav(&media.join("Album B").join("Artist - Two.wav"), 523.25, 800);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let tracks = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 100,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    assert_eq!(tracks.len(), 2, "expected 2 scanned tracks");

    #[derive(Debug, Clone)]
    struct ProgressEvent {
        current: usize,
        total: usize,
        file_path: String,
        track_ready: bool,
        has_waveform: bool,
        has_duration: bool,
        has_bpm_or_key: bool,
    }

    let progress = Arc::new(Mutex::new(Vec::<ProgressEvent>::new()));
    let progress_ref = Arc::clone(&progress);
    let analyzed_response = backend.analyze_new_tracks_with_progress(
        AnalyzeNewTracksRequest {
            bpm_min: None,
            bpm_max: None,
            track_ids: tracks.iter().map(|t| t.id.clone()).collect(),
            analysis_engine: None,
            ..Default::default()
        },
        move |progress| {
            progress_ref
                .lock()
                .expect("progress lock")
                .push(ProgressEvent {
                    current: progress.current,
                    total: progress.total,
                    file_path: progress.file_path.clone(),
                    track_ready: progress.track_ready,
                    has_waveform: progress
                        .waveform_preview
                        .as_ref()
                        .map(|p| !p.is_empty())
                        .unwrap_or(false)
                        || progress.waveform_peaks_path.is_some(),
                    has_duration: progress.duration_ms.is_some(),
                    has_bpm_or_key: progress.bpm.is_some() || progress.key.is_some(),
                });
        },
    );
    assert!(
        analyzed_response.ok,
        "analyze internal failed: {analyzed_response:?}"
    );
    let analyzed = analyzed_response.data.expect("analyze data");

    assert_eq!(
        analyzed.analyzed + analyzed.failed,
        2,
        "analysis count mismatch"
    );

    let calls = progress.lock().expect("progress lock final");
    assert!(
        calls.len() >= 2,
        "expected progress events, got {}",
        calls.len()
    );
    let ready_events: Vec<_> = calls.iter().filter(|c| c.track_ready).collect();
    assert_eq!(
        ready_events.len(),
        2,
        "expected one track_ready event per file"
    );
    assert!(
        calls
            .iter()
            .any(|c| !c.track_ready && (c.has_waveform || c.has_duration || c.has_bpm_or_key)),
        "expected at least one partial piece-level event before track_ready"
    );
    assert!(
        ready_events
            .iter()
            .all(|c| c.current >= 1 && c.current <= c.total && c.total == 2),
        "ready events should have valid current/total counters"
    );
    assert!(
        calls.iter().all(|c| c.file_path.ends_with(".wav")),
        "progress payload should include file paths"
    );
}

#[test]
fn analyze_new_tracks_progress_reports_library_duration_total_scoped_to_visible_roots() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let root_a = media.join("RootA");
    let root_b = media.join("RootB");
    fs::create_dir_all(&root_a).expect("create root a");
    fs::create_dir_all(&root_b).expect("create root b");
    write_test_wav(&root_a.join("Artist - Visible.wav"), 440.0, 800);
    write_test_wav(&root_b.join("Artist - Hidden.wav"), 523.25, 800);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![
            root_a.to_string_lossy().to_string(),
            root_b.to_string_lossy().to_string(),
        ],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let tracks = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 100,
            cursor: None,
        })
        .data
        .expect("search data")
        .items;
    assert_eq!(tracks.len(), 2, "expected 2 scanned tracks");
    let visible_track = tracks
        .iter()
        .find(|t| t.file_path.contains("Visible"))
        .expect("visible track");
    let hidden_track = tracks
        .iter()
        .find(|t| t.file_path.contains("Hidden"))
        .expect("hidden track");

    #[derive(Debug, Clone)]
    struct ProgressEvent {
        track_id: String,
        track_ready: bool,
        library_total_duration_ms: Option<u64>,
        library_duration_unknown_count: Option<usize>,
    }

    let progress = Arc::new(Mutex::new(Vec::<ProgressEvent>::new()));
    let progress_ref = Arc::clone(&progress);
    let analyzed_response = backend.analyze_new_tracks_with_progress(
        AnalyzeNewTracksRequest {
            bpm_min: None,
            bpm_max: None,
            // Analyze both tracks, but only RootA is "currently visible"
            // under the frontend's filter -- RootB's track should never
            // contribute to the reported library duration total, even
            // though it's part of this same analysis batch.
            track_ids: vec![visible_track.id.clone(), hidden_track.id.clone()],
            analysis_engine: None,
            source_roots: vec![root_a.to_string_lossy().to_string()],
            ..Default::default()
        },
        move |progress| {
            progress_ref
                .lock()
                .expect("progress lock")
                .push(ProgressEvent {
                    track_id: progress.track_id.clone(),
                    track_ready: progress.track_ready,
                    library_total_duration_ms: progress.library_total_duration_ms,
                    library_duration_unknown_count: progress.library_duration_unknown_count,
                });
        },
    );
    assert!(
        analyzed_response.ok,
        "analyze internal failed: {analyzed_response:?}"
    );
    let analyzed = analyzed_response.data.expect("analyze data");
    assert_eq!(
        analyzed.analyzed + analyzed.failed,
        2,
        "analysis count mismatch"
    );

    let calls = progress.lock().expect("progress lock final");
    let visible_ready = calls
        .iter()
        .find(|c| c.track_id == visible_track.id && c.track_ready)
        .expect("visible track ready event");
    let visible_duration_ms = visible_ready
        .library_total_duration_ms
        .expect("visible track should report a library duration total");
    assert!(
        visible_duration_ms > 0,
        "expected the visible track's own duration to be reflected in the total"
    );

    // The running total only ever grows within a batch, so the *last*
    // reported value is the settled one -- if RootB's track had incorrectly
    // contributed, this would exceed the visible track's own duration.
    let final_total = calls
        .iter()
        .filter_map(|c| c.library_total_duration_ms)
        .max()
        .expect("at least one library duration total reported");
    assert_eq!(
        final_total, visible_duration_ms,
        "hidden (out-of-filter) track must not contribute to the library duration total"
    );

    // Worker completion order between the two tracks isn't deterministic,
    // so the hidden track's own event reflects the total *at whatever
    // point it happened to finish* -- either before the visible track (0)
    // or after it (visible_duration_ms). What it must never do is include
    // its own duration on top of either of those.
    let hidden_ready = calls
        .iter()
        .find(|c| c.track_id == hidden_track.id && c.track_ready)
        .expect("hidden track ready event");
    assert!(
        matches!(hidden_ready.library_total_duration_ms, Some(0) | None)
            || hidden_ready.library_total_duration_ms == Some(visible_duration_ms),
        "hidden track's own completion must not add its own duration to the total, got {:?} (visible alone contributes {})",
        hidden_ready.library_total_duration_ms,
        visible_duration_ms
    );

    // Only the visible track is in the filtered set, and it's now fully
    // known -- unknown count settles at 0, not counting the hidden track.
    let final_unknown_count = calls
        .iter()
        .rev()
        .find_map(|c| c.library_duration_unknown_count)
        .expect("at least one unknown count reported");
    assert_eq!(final_unknown_count, 0);
}

#[test]
fn analyze_new_tracks_uses_audio_content_for_bpm_key_not_filename_tokens() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media");
    let misleading = media.join("Artist - 174_1B_misleading.wav");
    write_test_pulsed_key_wav(&misleading, 120.0, 20_000);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let before = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("before search")
        .items;
    let track = before.first().expect("scanned track");
    assert!(track.bpm.is_none(), "scan should not prefill bpm");
    assert!(track.key.is_none(), "scan should not prefill key");

    let analyze = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
        bpm_min: None,
        bpm_max: None,
        track_ids: vec![track.id.clone()],
        analysis_engine: None,
        ..Default::default()
    });
    assert!(analyze.ok, "analyze failed: {analyze:?}");

    let after = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("after search")
        .items;
    let analyzed = after.first().expect("analyzed track");
    let bpm = analyzed.bpm.expect("audio-derived bpm");
    assert!(
        (110.0..=130.0).contains(&bpm),
        "expected bpm near 120 from audio pulses, got {bpm}"
    );
    let key = analyzed.key.clone().expect("audio-derived key");
    assert_ne!(
        key, "1B",
        "key should not come from misleading filename token"
    );
    assert_ne!(
        key, "174",
        "key should not come from misleading numeric token"
    );
}

#[test]
fn analyze_new_tracks_does_not_guess_bpm_key_from_filename_on_silence() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media");
    let misleading = media.join("Artist - 128_8A_silence.wav");
    write_test_silent_wav(&misleading, 10_000);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .first()
        .expect("scanned track")
        .clone();

    let analyze = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
        bpm_min: None,
        bpm_max: None,
        track_ids: vec![track.id.clone()],
        analysis_engine: None,
        ..Default::default()
    });
    assert!(analyze.ok, "analyze failed: {analyze:?}");

    let analyzed = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search analyzed")
        .items
        .first()
        .expect("analyzed track")
        .clone();
    assert!(
        analyzed.bpm.is_none(),
        "bpm should remain unset for silence even with 128 filename token"
    );
    if let Some(key) = analyzed.key.as_deref() {
        assert_ne!(key, "8A", "key should not come from filename token");
        assert_ne!(
            key, "128",
            "key should not come from numeric filename token"
        );
    }
}

#[test]
fn analyze_new_tracks_with_stratum_default_produces_bpm_and_key() {
    // With stratum-dsp as default engine, BPM/key should be detected even
    // without essentia.js / Node.js available.
    let _guard = test_env_lock().lock().expect("env lock");
    let prev_enabled = std::env::var("DJTKIT_ENABLE_ESSENTIA_JS").ok();
    let prev_runner = std::env::var("DJTKIT_ESSENTIA_RUNNER").ok();
    // SAFETY: tests serialize env access through a global mutex.
    unsafe {
        std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", "0");
        std::env::remove_var("DJTKIT_ESSENTIA_RUNNER");
    }

    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media");
    let source = media.join("Artist - stratum_test.wav");
    write_test_pulsed_key_wav(&source, 120.0, 20_000);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .first()
        .expect("scanned track")
        .clone();

    let analyze = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
        bpm_min: None,
        bpm_max: None,
        track_ids: vec![track.id.clone()],
        analysis_engine: None,
        ..Default::default()
    });
    assert!(analyze.ok, "analyze failed: {analyze:?}");

    let analyzed = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search analyzed")
        .items
        .first()
        .expect("analyzed track")
        .clone();
    // Stratum is the default engine — BPM/key should be detected from the test WAV.
    assert!(
        analyzed.bpm.is_some(),
        "bpm should be set with stratum default engine"
    );

    match prev_enabled {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ENABLE_ESSENTIA_JS") }
        }
    }
    match prev_runner {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ESSENTIA_RUNNER", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ESSENTIA_RUNNER") }
        }
    }
}

#[test]
fn analyze_new_tracks_respects_requested_bpm_range() {
    // The batch endpoint used to hardcode resolve_analysis_bpm_range(None, None)
    // (default 70-180) regardless of the request's bpm_min/bpm_max. This proves
    // a caller-supplied range is actually threaded through: analyzing a 120bpm
    // fixture against a range that excludes 120 must not detect ~120bpm.
    let _guard = test_env_lock().lock().expect("env lock");
    let prev_enabled = std::env::var("DJTKIT_ENABLE_ESSENTIA_JS").ok();
    let prev_runner = std::env::var("DJTKIT_ESSENTIA_RUNNER").ok();
    // SAFETY: tests serialize env access through a global mutex.
    unsafe {
        std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", "0");
        std::env::remove_var("DJTKIT_ESSENTIA_RUNNER");
    }

    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media");
    let source = media.join("Artist - stratum_range_test.wav");
    write_test_pulsed_key_wav(&source, 120.0, 20_000);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .first()
        .expect("scanned track")
        .clone();

    let analyze = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
        bpm_min: Some(200),
        bpm_max: Some(220),
        track_ids: vec![track.id.clone()],
        analysis_engine: None,
        ..Default::default()
    });
    assert!(analyze.ok, "analyze failed: {analyze:?}");

    let analyzed = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search analyzed")
        .items
        .first()
        .expect("analyzed track")
        .clone();
    if let Some(bpm) = analyzed.bpm {
        assert!(
            (bpm - 120.0).abs() > 5.0,
            "expected the requested 200-220 bpm range to steer detection away from the \
             fixture's true 120bpm, got {bpm}"
        );
    }

    match prev_enabled {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ENABLE_ESSENTIA_JS") }
        }
    }
    match prev_runner {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ESSENTIA_RUNNER", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ESSENTIA_RUNNER") }
        }
    }
}

#[test]
fn analyze_new_tracks_commits_periodically_not_only_at_the_end() {
    // analyze_new_tracks_with_progress used to hold one write transaction
    // open for the entire batch, committing only once at the very end. For a
    // long-running batch this blocked every other write in the app (e.g.
    // settings persistence) until the whole batch finished, surfacing as
    // SQLITE_BUSY errors once the busy_timeout elapsed. It now commits after
    // every single track instead of periodically/by batch size -- a fixed
    // track-count interval doesn't bound wall-clock lock-hold time if
    // individual tracks are slow (confirmed in practice: a small fixed
    // interval still held the lock for minutes in a slow/dev build). This
    // proves commits happen incrementally mid-batch: a separate
    // BackendService/connection can see already-analyzed tracks before the
    // analyze_new_tracks_with_progress call itself has returned.
    let _guard = test_env_lock().lock().expect("env lock");
    let prev_enabled = std::env::var("DJTKIT_ENABLE_ESSENTIA_JS").ok();
    let prev_runner = std::env::var("DJTKIT_ESSENTIA_RUNNER").ok();
    let prev_max_workers = std::env::var("DJTKIT_ANALYSIS_MAX_WORKERS").ok();
    // SAFETY: tests serialize env access through a global mutex.
    unsafe {
        std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", "0");
        std::env::remove_var("DJTKIT_ESSENTIA_RUNNER");
        std::env::set_var("DJTKIT_ANALYSIS_MAX_WORKERS", "3");
    }

    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media");
    // Multiple tracks so the run crosses multiple commit boundaries before
    // the whole call finishes.
    for i in 0..20 {
        let source = media.join(format!("Artist - commit_test_{i:02}.wav"));
        write_test_pulsed_key_wav(&source, 120.0, 3_000);
    }

    let data_dir = root.path().join("data");
    let service = BackendService::new(&data_dir).expect("create service");
    let scan = service
        .scan_library(ScanLibraryRequest {
            source_roots: vec![media.to_string_lossy().to_string()],
            incremental: true,
        })
        .expect("scan succeeds");
    assert_eq!(scan.indexed, 20);

    let track_ids: Vec<String> = service
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 50,
            cursor: None,
        })
        .expect("search succeeds")
        .items
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(track_ids.len(), 20);

    // A separate service/connection from the one running the batch -- this
    // is the stand-in for "some other backend operation trying to write or
    // read while a big analysis batch is still in flight".
    let observer = BackendService::new(&data_dir).expect("create observer service");
    let mut saw_mid_batch_commit = false;
    let mut max_committed_before_done = 0usize;
    let result = service.analyze_new_tracks_with_progress(
        AnalyzeNewTracksRequest {
            track_ids,
            bpm_min: None,
            bpm_max: None,
            analysis_engine: None,
            ..Default::default()
        },
        |progress| {
            if !progress.track_ready || progress.current >= progress.total {
                return;
            }
            let committed = observer
                .search_tracks(SearchTracksRequest {
                    query: String::new(),
                    limit: 50,
                    cursor: None,
                })
                .expect("observer search succeeds")
                .items
                .iter()
                .filter(|t| t.bpm.is_some())
                .count();
            max_committed_before_done = max_committed_before_done.max(committed);
            if committed > 0 {
                saw_mid_batch_commit = true;
            }
        },
    );
    assert!(result.is_ok(), "analyze failed: {result:?}");
    assert!(
        saw_mid_batch_commit,
        "expected a separate connection to see at least one already-committed track \
         before the batch finished (max observed: {max_committed_before_done}); \
         commits are not happening incrementally"
    );

    match prev_enabled {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ENABLE_ESSENTIA_JS") }
        }
    }
    match prev_runner {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ESSENTIA_RUNNER", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ESSENTIA_RUNNER") }
        }
    }
    match prev_max_workers {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ANALYSIS_MAX_WORKERS", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ANALYSIS_MAX_WORKERS") }
        }
    }
}

#[test]
fn analyze_new_tracks_commits_periodically_with_real_worker_count_on_moderate_batch() {
    // Regression test for the real bug this whole area went through: any
    // commit interval tied to a track *count* (whether fixed, or scaled by
    // worker count and capped) can still leave the write lock held for an
    // unbounded amount of *wall-clock* time if individual tracks are slow.
    // The batch now commits after every single track instead, which is the
    // only interval that's actually independent of hardware, worker count,
    // and per-track duration. This test deliberately does NOT force
    // DJTKIT_ANALYSIS_MAX_WORKERS, so it exercises the real
    // available_parallelism()-derived worker count on a moderate (39-track)
    // batch, matching the exact scenario that originally exposed the bug.
    let _guard = test_env_lock().lock().expect("env lock");
    let prev_enabled = std::env::var("DJTKIT_ENABLE_ESSENTIA_JS").ok();
    let prev_runner = std::env::var("DJTKIT_ESSENTIA_RUNNER").ok();
    let prev_max_workers = std::env::var("DJTKIT_ANALYSIS_MAX_WORKERS").ok();
    // SAFETY: tests serialize env access through a global mutex.
    unsafe {
        std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", "0");
        std::env::remove_var("DJTKIT_ESSENTIA_RUNNER");
        std::env::remove_var("DJTKIT_ANALYSIS_MAX_WORKERS");
    }

    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media");
    for i in 0..39 {
        let source = media.join(format!("Artist - moderate_batch_{i:02}.wav"));
        write_test_pulsed_key_wav(&source, 120.0, 2_000);
    }

    let data_dir = root.path().join("data");
    let service = BackendService::new(&data_dir).expect("create service");
    let scan = service
        .scan_library(ScanLibraryRequest {
            source_roots: vec![media.to_string_lossy().to_string()],
            incremental: true,
        })
        .expect("scan succeeds");
    assert_eq!(scan.indexed, 39);

    let track_ids: Vec<String> = service
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 50,
            cursor: None,
        })
        .expect("search succeeds")
        .items
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(track_ids.len(), 39);

    let observer = BackendService::new(&data_dir).expect("create observer service");
    let mut saw_mid_batch_commit = false;
    let result = service.analyze_new_tracks_with_progress(
        AnalyzeNewTracksRequest {
            track_ids,
            bpm_min: None,
            bpm_max: None,
            analysis_engine: None,
            ..Default::default()
        },
        |progress| {
            if !progress.track_ready || progress.current >= progress.total {
                return;
            }
            let committed = observer
                .search_tracks(SearchTracksRequest {
                    query: String::new(),
                    limit: 50,
                    cursor: None,
                })
                .expect("observer search succeeds")
                .items
                .iter()
                .filter(|t| t.bpm.is_some())
                .count();
            if committed > 0 {
                saw_mid_batch_commit = true;
            }
        },
    );
    assert!(result.is_ok(), "analyze failed: {result:?}");
    assert!(
        saw_mid_batch_commit,
        "expected at least one mid-batch commit visible to a separate connection \
         with the real (uncapped) worker count on a 39-track batch"
    );

    match prev_enabled {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ENABLE_ESSENTIA_JS") }
        }
    }
    match prev_runner {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ESSENTIA_RUNNER", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ESSENTIA_RUNNER") }
        }
    }
    match prev_max_workers {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ANALYSIS_MAX_WORKERS", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ANALYSIS_MAX_WORKERS") }
        }
    }
}

#[test]
fn analyze_new_tracks_pauses_and_resumes_between_tracks() {
    // Pausing must let the track currently being analyzed finish normally,
    // but no worker may start a *new* track until resumed. Forcing a single
    // worker makes the ordering deterministic: track 1 finishes, we pause
    // and kick off a background resume after a short real delay, and assert
    // track 2 didn't start being reported ready until at least roughly that
    // delay had elapsed.
    let _guard = test_env_lock().lock().expect("env lock");
    let prev_enabled = std::env::var("DJTKIT_ENABLE_ESSENTIA_JS").ok();
    let prev_runner = std::env::var("DJTKIT_ESSENTIA_RUNNER").ok();
    let prev_max_workers = std::env::var("DJTKIT_ANALYSIS_MAX_WORKERS").ok();
    // SAFETY: tests serialize env access through a global mutex.
    unsafe {
        std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", "0");
        std::env::remove_var("DJTKIT_ESSENTIA_RUNNER");
        std::env::set_var("DJTKIT_ANALYSIS_MAX_WORKERS", "1");
    }

    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media");
    for i in 0..6 {
        let source = media.join(format!("Artist - pause_test_{i:02}.wav"));
        write_test_pulsed_key_wav(&source, 120.0, 2_000);
    }

    let data_dir = root.path().join("data");
    let service = BackendService::new(&data_dir).expect("create service");
    let scan = service
        .scan_library(ScanLibraryRequest {
            source_roots: vec![media.to_string_lossy().to_string()],
            incremental: true,
        })
        .expect("scan succeeds");
    assert_eq!(scan.indexed, 6);

    let track_ids: Vec<String> = service
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 50,
            cursor: None,
        })
        .expect("search succeeds")
        .items
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(track_ids.len(), 6);

    const RESUME_DELAY: std::time::Duration = std::time::Duration::from_millis(250);
    let mut ready_count = 0usize;
    let mut paused_at: Option<std::time::Instant> = None;
    let mut resumed_gap: Option<std::time::Duration> = None;
    let mut resume_handle: Option<std::thread::JoinHandle<()>> = None;
    let result = service.analyze_new_tracks_with_progress(
        AnalyzeNewTracksRequest {
            track_ids,
            bpm_min: None,
            bpm_max: None,
            analysis_engine: None,
            ..Default::default()
        },
        |progress| {
            if !progress.track_ready {
                return;
            }
            ready_count += 1;
            if ready_count == 1 {
                service.set_analysis_paused(true).expect("pause succeeds");
                paused_at = Some(std::time::Instant::now());
                let resume_service = service.clone();
                resume_handle = Some(std::thread::spawn(move || {
                    std::thread::sleep(RESUME_DELAY);
                    resume_service
                        .set_analysis_paused(false)
                        .expect("resume succeeds");
                }));
            } else if ready_count == 2 {
                resumed_gap = paused_at.map(|t| t.elapsed());
            }
        },
    );
    assert!(result.is_ok(), "analyze failed: {result:?}");
    let data = result.expect("already checked");
    assert_eq!(data.analyzed, 6, "expected the whole batch to complete");
    let gap = resumed_gap.expect("expected a second track to become ready after resuming");
    assert!(
        gap >= RESUME_DELAY.mul_f32(0.6),
        "expected track 2 to only become ready roughly after the resume delay \
         ({RESUME_DELAY:?}), but it was ready after only {gap:?}"
    );
    if let Some(handle) = resume_handle {
        handle.join().expect("resume thread should not panic");
    }

    match prev_enabled {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ENABLE_ESSENTIA_JS") }
        }
    }
    match prev_runner {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ESSENTIA_RUNNER", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ESSENTIA_RUNNER") }
        }
    }
    match prev_max_workers {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ANALYSIS_MAX_WORKERS", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ANALYSIS_MAX_WORKERS") }
        }
    }
}

#[test]
fn analyze_new_tracks_cancel_stops_early_without_hanging() {
    // Cancelling must stop workers from picking up new tracks (the
    // in-flight one still finishes), and the coordinator must return
    // gracefully once every worker has exited instead of hanging forever on
    // a channel that will never receive another event.
    let _guard = test_env_lock().lock().expect("env lock");
    let prev_enabled = std::env::var("DJTKIT_ENABLE_ESSENTIA_JS").ok();
    let prev_runner = std::env::var("DJTKIT_ESSENTIA_RUNNER").ok();
    let prev_max_workers = std::env::var("DJTKIT_ANALYSIS_MAX_WORKERS").ok();
    // SAFETY: tests serialize env access through a global mutex.
    unsafe {
        std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", "0");
        std::env::remove_var("DJTKIT_ESSENTIA_RUNNER");
        std::env::set_var("DJTKIT_ANALYSIS_MAX_WORKERS", "1");
    }

    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media");
    for i in 0..6 {
        let source = media.join(format!("Artist - cancel_test_{i:02}.wav"));
        write_test_pulsed_key_wav(&source, 120.0, 2_000);
    }

    let data_dir = root.path().join("data");
    let service = BackendService::new(&data_dir).expect("create service");
    let scan = service
        .scan_library(ScanLibraryRequest {
            source_roots: vec![media.to_string_lossy().to_string()],
            incremental: true,
        })
        .expect("scan succeeds");
    assert_eq!(scan.indexed, 6);

    let track_ids: Vec<String> = service
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 50,
            cursor: None,
        })
        .expect("search succeeds")
        .items
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(track_ids.len(), 6);

    let mut ready_count = 0usize;
    let result = service.analyze_new_tracks_with_progress(
        AnalyzeNewTracksRequest {
            track_ids,
            bpm_min: None,
            bpm_max: None,
            analysis_engine: None,
            ..Default::default()
        },
        |progress| {
            if !progress.track_ready {
                return;
            }
            ready_count += 1;
            if ready_count == 1 {
                service.cancel_analysis().expect("cancel succeeds");
            }
        },
    );
    assert!(
        result.is_ok(),
        "cancelling should return gracefully, not error/hang: {result:?}"
    );
    let data = result.expect("already checked");
    // With a single worker, cancelling as soon as track 1 is ready still
    // lets track 2 slip in: the worker sends track 1's "done" event and
    // immediately loops back to pop the next track, which can win the race
    // against the coordinator thread processing that event and running our
    // cancel callback. Either way the batch must stop well short of all 6.
    assert!(
        (1..6).contains(&data.analyzed),
        "expected cancellation to stop the batch early (1 to 5 of 6 tracks), got {}",
        data.analyzed
    );
    let expected_warning = format!("Analysis cancelled: {} of 6 tracks analyzed", data.analyzed);
    assert!(
        data.warnings.iter().any(|w| w.message == expected_warning),
        "expected warning {expected_warning:?}, got: {:?}",
        data.warnings
    );

    match prev_enabled {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ENABLE_ESSENTIA_JS") }
        }
    }
    match prev_runner {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ESSENTIA_RUNNER", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ESSENTIA_RUNNER") }
        }
    }
    match prev_max_workers {
        Some(v) => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::set_var("DJTKIT_ANALYSIS_MAX_WORKERS", v) }
        }
        None => {
            // SAFETY: tests serialize env access through a global mutex.
            unsafe { std::env::remove_var("DJTKIT_ANALYSIS_MAX_WORKERS") }
        }
    }
}

#[test]
fn scan_library_rescan_preserves_existing_key_when_scanner_has_no_tonality() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media");
    let source = media.join("Artist - preserve_key.wav");
    write_test_wav(&source, 440.0, 800);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan1 = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan1.ok, "initial scan failed: {scan1:?}");

    let track = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .first()
        .expect("scanned track")
        .clone();

    let conn =
        rusqlite::Connection::open(backend.data_dir().join("backend.db")).expect("db connect");
    conn.execute(
        "UPDATE tracks SET tonality = ?1 WHERE id = ?2",
        rusqlite::params!["8A", track.id],
    )
    .expect("set tonality");
    drop(conn);

    let scan2 = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan2.ok, "rescan failed: {scan2:?}");

    let rescanned = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search rescanned")
        .items
        .first()
        .expect("rescanned track")
        .clone();

    assert_eq!(
        rescanned.key.as_deref(),
        Some("8A"),
        "rescan should preserve existing key when scanner has no tonality"
    );
}

#[test]
fn fetch_usb_playlists_materialization_clears_stale_local_key_when_usb_key_is_missing() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let usb_file = usb
        .join("Contents")
        .join("Artist")
        .join("usb_key_clear")
        .join("Artist - usb_key_clear.wav");
    fs::create_dir_all(usb_file.parent().expect("usb file parent"))
        .expect("create usb contents parent");
    write_test_wav(&usb_file, 440.0, 1000);

    let stale_local_id = "usb-local-existing".to_string();
    let conn =
        rusqlite::Connection::open(backend.data_dir().join("backend.db")).expect("db connect");
    conn.execute(
        r#"INSERT INTO tracks (
            id, title, artist, album, bpm, tonality, file_path, file_size_bytes,
            artwork_path, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)"#,
        rusqlite::params![
            stale_local_id,
            "usb_key_clear",
            "Artist",
            "usb_key_clear",
            128.0_f64,
            "Am",
            usb_file.to_string_lossy().to_string(),
            std::fs::metadata(&usb_file)
                .ok()
                .and_then(|m| i64::try_from(m.len()).ok()),
            "/tmp/stale-cover.jpg",
            "2026-03-11T00:00:00Z"
        ],
    )
    .expect("seed stale local usb track");
    drop(conn);

    let playlist = ExportPlaylistData {
        id: "usb-playlist".to_string(),
        name: "USB Key Clear".to_string(),
        tracks: Vec::new(),
    };
    let manifest_track = ExportManifestTrack {
        id: "manifest-track".to_string(),
        master_db_id: None,
        master_content_id: None,
        content_link: None,
        position: 1,
        track_number: Some(1),
        title: "usb_key_clear".to_string(),
        artist: "Artist".to_string(),
        album: Some("usb_key_clear".to_string()),
        bpm: Some(128.0),
        key: None,
        source_path: usb_file.to_string_lossy().to_string(),
        exported_path: format!(
            "/Contents/Artist/usb_key_clear/{}",
            usb_file.file_name().expect("usb file").to_string_lossy()
        ),
        file_modified_at: Some("1714521600".to_string()),
        file_size_bytes: std::fs::metadata(&usb_file)
            .ok()
            .and_then(|m| i64::try_from(m.len()).ok()),
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
        file_type: Some(1),
        owns_exported_media: true,
        owns_artwork: true,
        owns_waveform: true,
        artwork_path: None,
        waveform_path: Some("/PIONEER/USBANLZ/P001/TEST/ANLZ0000.DAT".to_string()),
        duration_ms: Some(180_000),
    };
    let manifest = ExportManifest {
        version: 1,
        generated_at: "1970-01-01T00:00:00Z".to_string(),
        playlist_id: playlist.id.clone(),
        playlist_name: playlist.name.clone(),
        usb_root: usb.to_string_lossy().to_string(),
        options: ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![manifest_track],
    };
    write_pdb(&usb, &playlist, &manifest, false, None, None).expect("write export pdb");

    let export_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_export_db(&export_db);
    conn.execute(
        r#"INSERT OR REPLACE INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (1, 'USB Key Clear', 0, 1)"#,
        [],
    )
    .expect("insert playlist");
    conn.execute(
        r#"INSERT OR REPLACE INTO artist (artist_id, name) VALUES (1, 'Artist')"#,
        [],
    )
    .expect("insert artist");
    conn.execute(
        r#"INSERT OR REPLACE INTO content (content_id, title, artist_id_artist, bpmx100, path, analysisDataFilePath, length, key_id)
           VALUES (1, 'Artist - usb_key_clear', 1, 12800, ?1, '/PIONEER/USBANLZ/P001/TEST/ANLZ0000.DAT', 180, NULL)"#,
        rusqlite::params![format!(
            "/Contents/Artist/usb_key_clear/{}",
            usb_file.file_name().expect("usb file").to_string_lossy()
        )],
    )
    .expect("insert content");
    conn.execute(
        r#"INSERT OR REPLACE INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (1, 1, 1)"#,
        [],
    )
    .expect("insert playlist content");
    drop(conn);

    let playlists = backend.fetch_usb_playlists(FetchUsbPlaylistsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(playlists.ok, "fetch usb playlists failed: {playlists:?}");

    let reloaded = backend
        .get_tracks_by_ids_with_previews(GetTracksByIdsRequest {
            track_ids: vec![stale_local_id.clone()],
        })
        .data
        .expect("reloaded local track")
        .items
        .into_iter()
        .find(|t| t.id == stale_local_id)
        .expect("materialized local track");

    assert!(
        reloaded
            .key
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty(),
        "USB materialization should clear stale local key when USB row has no key: {:?}",
        reloaded
    );
    assert!(
        reloaded
            .artwork_path
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty(),
        "USB materialization should clear stale local artwork when USB row has no artwork: {:?}",
        reloaded
    );
}

/// Regression test for a false-positive "slow-media suspected" warning:
/// `fetch_usb_histories`'s "read supplemental databases" stage used to open
/// and SQLCipher-decrypt the same eDB file from scratch up to 4 times in a
/// single call (once per read helper, plus once more for the history
/// COUNT(*) queries), which on a large eDB could take long enough to trip
/// the timing-based slow-media warning purely from redundant CPU/SQL work,
/// not actual media speed. It's now opened once and the connection is
/// reused. Asserting on the exact open-success warning count (rather than a
/// failure count, which has no cheap-to-build fixture in this codebase) is
/// the direct, observable proof the dedup landed.
#[test]
fn fetch_usb_histories_opens_edb_exactly_once_not_once_per_read() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let usb_file = usb
        .join("Contents")
        .join("Artist")
        .join("edb_reuse")
        .join("Artist - edb_reuse.wav");
    fs::create_dir_all(usb_file.parent().expect("usb file parent"))
        .expect("create usb contents parent");
    write_test_wav(&usb_file, 440.0, 1000);

    let playlist = ExportPlaylistData {
        id: "usb-playlist".to_string(),
        name: "EDB Reuse".to_string(),
        tracks: Vec::new(),
    };
    let manifest_track = ExportManifestTrack {
        id: "manifest-track".to_string(),
        master_db_id: None,
        master_content_id: None,
        content_link: None,
        position: 1,
        track_number: Some(1),
        title: "edb_reuse".to_string(),
        artist: "Artist".to_string(),
        album: Some("edb_reuse".to_string()),
        bpm: Some(128.0),
        key: None,
        source_path: usb_file.to_string_lossy().to_string(),
        exported_path: format!(
            "/Contents/Artist/edb_reuse/{}",
            usb_file.file_name().expect("usb file").to_string_lossy()
        ),
        file_modified_at: Some("1714521600".to_string()),
        file_size_bytes: std::fs::metadata(&usb_file)
            .ok()
            .and_then(|m| i64::try_from(m.len()).ok()),
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
        file_type: Some(1),
        owns_exported_media: true,
        owns_artwork: true,
        owns_waveform: true,
        artwork_path: None,
        waveform_path: Some("/PIONEER/USBANLZ/P001/TEST/ANLZ0000.DAT".to_string()),
        duration_ms: Some(180_000),
    };
    let manifest = ExportManifest {
        version: 1,
        generated_at: "1970-01-01T00:00:00Z".to_string(),
        playlist_id: playlist.id.clone(),
        playlist_name: playlist.name.clone(),
        usb_root: usb.to_string_lossy().to_string(),
        options: ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![manifest_track],
    };
    write_pdb(&usb, &playlist, &manifest, false, None, None).expect("write export pdb");

    let export_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_export_db(&export_db);
    conn.execute(
        r#"INSERT OR REPLACE INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (1, 'EDB Reuse', 0, 1)"#,
        [],
    )
    .expect("insert playlist");
    conn.execute(
        r#"INSERT OR REPLACE INTO artist (artist_id, name) VALUES (1, 'Artist')"#,
        [],
    )
    .expect("insert artist");
    conn.execute(
        r#"INSERT OR REPLACE INTO content (content_id, title, artist_id_artist, bpmx100, path, analysisDataFilePath, length, key_id, dateCreated)
           VALUES (1, 'Artist - edb_reuse', 1, 12800, ?1, '/PIONEER/USBANLZ/P001/TEST/ANLZ0000.DAT', 180, NULL, '2026-01-01')"#,
        rusqlite::params![format!(
            "/Contents/Artist/edb_reuse/{}",
            usb_file.file_name().expect("usb file").to_string_lossy()
        )],
    )
    .expect("insert content");
    conn.execute(
        r#"INSERT OR REPLACE INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (1, 1, 1)"#,
        [],
    )
    .expect("insert playlist content");
    conn.execute(
        r#"INSERT OR REPLACE INTO history (history_id, sequenceNo, name, attribute, history_id_parent) VALUES (1, 1, 'HISTORY 001', 0, 0)"#,
        [],
    )
    .expect("insert history");
    conn.execute(
        r#"INSERT OR REPLACE INTO history_content (history_id, content_id, sequenceNo) VALUES (1, 1, 1)"#,
        [],
    )
    .expect("insert history content");
    drop(conn);

    let histories = backend.fetch_usb_histories(FetchUsbHistoriesRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(histories.ok, "fetch usb histories failed: {histories:?}");
    let warnings = histories.data.expect("histories data").warnings;

    let open_success_count = warnings
        .iter()
        .filter(|w| w.code == "edb.open.no-key" || w.code == "edb.open.unlocked")
        .count();
    assert_eq!(
        open_success_count, 1,
        "expected the eDB to be opened exactly once (reused thereafter), got warnings: {warnings:?}"
    );
}

#[test]
fn analyze_new_tracks_extracts_bpm_key_from_aiff() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media");
    let source = media.join("Artist - aiff_analysis.aiff");
    write_test_pulsed_key_aiff(&source, 120.0, 20_000);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .first()
        .expect("scanned track")
        .clone();

    let analyze = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
        bpm_min: None,
        bpm_max: None,
        track_ids: vec![track.id.clone()],
        analysis_engine: None,
        ..Default::default()
    });
    assert!(analyze.ok, "analyze failed: {analyze:?}");

    let analyzed = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search analyzed")
        .items
        .first()
        .expect("analyzed track")
        .clone();
    let bpm = analyzed.bpm.expect("aiff bpm");
    assert!(
        (110.0..=130.0).contains(&bpm),
        "expected bpm near 120 from AIFF pulses, got {bpm}"
    );
    let key = analyzed.key.expect("aiff key");
    assert!(!key.trim().is_empty(), "expected non-empty key");
}

#[test]
fn usb_diagnostics_emits_playlist_name_progress_messages() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let _seed = seed_usb_missing_audio_fixture(&backend, &usb);

    let progress = Arc::new(Mutex::new(Vec::<String>::new()));
    let progress_ref = Arc::clone(&progress);
    let result = backend.run_usb_diagnostics_with_progress(
        RunUsbDiagnosticsRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
        },
        move |_current, _total, message| {
            progress_ref
                .lock()
                .expect("progress lock")
                .push(message.to_string());
        },
    );
    assert!(result.ok, "diagnostics failed: {result:?}");

    let messages = progress.lock().expect("progress final lock");
    assert!(
        messages.iter().any(|m| m == "USB: Checking PDB integrity"),
        "expected structured PDB progress stage"
    );
    assert!(
        messages
            .iter()
            .any(|m| m == "USB: Checking playlist resolution"),
        "expected structured playlist resolution stage"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.starts_with("Resolving playlist ")),
        "expected per-playlist progress with playlist name"
    );
    let report = result.data.expect("diagnostics report");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.message.starts_with("stage timing:")),
        "expected stage timing entries in diagnostics warnings"
    );
}

#[test]
fn repair_usb_diagnostics_preview_lists_missing_audio_references_fix() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let _seed = seed_usb_missing_audio_fixture(&backend, &usb);

    let preview_response = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: false,
        selected_fix_ids: Vec::new(),
    });
    assert!(
        preview_response.ok,
        "repair preview failed: {preview_response:?}"
    );
    let preview = preview_response.data.expect("repair preview");

    assert!(
        preview
            .proposed_fixes
            .iter()
            .any(|f| f.id == "remove_missing_audio_references" && f.supported),
        "missing-audio fix should be proposed in preview"
    );
    assert!(
        preview
            .warnings
            .iter()
            .any(|w| w.message.starts_with("missing-audio reference: ")),
        "missing-audio paths should be emitted to warnings/event log"
    );
    assert!(
        preview
            .warnings
            .iter()
            .any(|w| w.code == "usb.diagnostics.missing-audio" && w.level == "warn"),
        "missing-audio path entries should be warn-level with a dedicated code"
    );
}

#[test]
fn repair_usb_diagnostics_preview_lists_unindexed_audio_paths_in_warnings() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let unindexed_path = seed_usb_unindexed_audio_fixture(&backend, &usb);

    let preview_response = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: false,
        selected_fix_ids: Vec::new(),
    });
    assert!(
        preview_response.ok,
        "repair preview failed: {preview_response:?}"
    );
    let preview = preview_response.data.expect("repair preview");

    assert!(
        preview
            .warnings
            .iter()
            .any(|w| w.message == format!("unindexed audio file: {unindexed_path}")),
        "unindexed audio path should be emitted to warnings/event log"
    );
    assert!(
        preview
            .warnings
            .iter()
            .any(|w| w.code == "usb.diagnostics.unindexed-audio" && w.level == "warn"),
        "unindexed audio path entries should be warn-level with a dedicated code"
    );
}

#[test]
fn repair_usb_diagnostics_strict_upgrade_rewrites_pdb_from_edb() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let playlist = ExportPlaylistData {
        id: "usb-pl-testi".to_string(),
        name: "Testi".to_string(),
        tracks: Vec::new(),
    };
    let manifest = ExportManifest {
        version: 1,
        generated_at: "1970-01-01T00:00:00Z".to_string(),
        playlist_id: "pl-testi".to_string(),
        playlist_name: "Testi".to_string(),
        usb_root: usb.to_string_lossy().to_string(),
        options: ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 3,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![
            ExportManifestTrack {
                id: "t-a".to_string(),
                master_db_id: None,
                master_content_id: None,
                content_link: None,
                position: 1,
                track_number: Some(1),
                title: "Track A".to_string(),
                artist: "Artist".to_string(),
                album: Some("Album".to_string()),
                bpm: Some(128.0),
                key: Some("8A".to_string()),
                source_path: "/tmp/a.mp3".to_string(),
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
                duration_ms: Some(180_000),
            },
            ExportManifestTrack {
                id: "t-b".to_string(),
                master_db_id: None,
                master_content_id: None,
                content_link: None,
                position: 2,
                track_number: Some(2),
                title: "Track B".to_string(),
                artist: "Artist".to_string(),
                album: Some("Album".to_string()),
                bpm: Some(129.0),
                key: Some("9A".to_string()),
                source_path: "/tmp/b.mp3".to_string(),
                exported_path: "/Contents/Artist/Album/b.mp3".to_string(),
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
                duration_ms: Some(181_000),
            },
            ExportManifestTrack {
                id: "t-c".to_string(),
                master_db_id: None,
                master_content_id: None,
                content_link: None,
                position: 3,
                track_number: Some(3),
                title: "Track C".to_string(),
                artist: "Artist".to_string(),
                album: Some("Album".to_string()),
                bpm: Some(130.0),
                key: Some("10A".to_string()),
                source_path: "/tmp/c.mp3".to_string(),
                exported_path: "/Contents/Artist/Album/c.mp3".to_string(),
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
                duration_ms: Some(182_000),
            },
        ],
    };

    write_pdb(&usb, &playlist, &manifest, true, None, None).expect("write pdb");
    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_export_db(&vendor_db);
    conn.execute(
        "INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (100, 'Testi', 0, 100)",
        [],
    )
    .expect("insert first testi playlist row");
    conn.execute(
        "INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (101, 'Testi', 0, 101)",
        [],
    )
    .expect("insert second testi playlist row");
    conn.execute(
        "INSERT OR REPLACE INTO artist (artist_id, name) VALUES (7, 'Artist')",
        [],
    )
    .expect("insert artist");
    conn.execute(
        "INSERT OR REPLACE INTO album (album_id, name, artist_id, isComplation) VALUES (9, 'Album', 7, 0)",
        [],
    )
    .expect("insert album");
    conn.execute(
        r#"INSERT OR REPLACE INTO "key" (key_id, name) VALUES (5, '8A')"#,
        [],
    )
    .expect("insert key");
    conn.execute(
        "INSERT OR REPLACE INTO image (image_id, path) VALUES (3, '/PIONEER/Artwork/00001/a00003.jpg')",
        [],
    )
    .expect("insert image");
    conn.execute(
        "INSERT INTO content (content_id, title, path, artist_id_artist, album_id, key_id, image_id, analysisDataFilePath, bpmx100, length, trackNo) VALUES (200, 'Track A', '/Contents/Artist/Album/a.mp3', 7, 9, 5, 3, '/PIONEER/USBANLZ/P001/A0000001/ANLZ0000.DAT', 12800, 180, 1)",
        [],
    )
    .expect("insert content a");
    conn.execute(
        "INSERT INTO content (content_id, title, path, artist_id_artist, album_id, key_id, image_id, analysisDataFilePath, bpmx100, length, trackNo) VALUES (201, 'Track B', '/Contents/Artist/Album/b.mp3', 7, 9, 5, 3, '/PIONEER/USBANLZ/P001/A0000002/ANLZ0000.DAT', 12900, 181, 2)",
        [],
    )
    .expect("insert content b");
    conn.execute(
        "INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (100, 200, 1)",
        [],
    )
    .expect("link first row");
    conn.execute(
        "INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (101, 201, 1)",
        [],
    )
    .expect("link duplicate row");
    drop(conn);

    let preview_response = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: false,
        selected_fix_ids: Vec::new(),
    });
    assert!(
        preview_response.ok,
        "repair preview failed: {preview_response:?}"
    );
    let preview = preview_response.data.expect("repair preview");
    assert!(
        preview
            .proposed_fixes
            .iter()
            .any(|f| f.id == "upgrade_export_data_to_strict_parity"
                && f.supported
                && !f.destructive),
        "strict upgrade fix should be proposed as supported and safe"
    );

    let applied_response = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["upgrade_export_data_to_strict_parity".to_string()],
    });
    assert!(
        applied_response.ok,
        "repair apply failed: {applied_response:?}"
    );
    let applied = applied_response.data.expect("repair apply");
    assert!(
        applied
            .applied_fixes
            .iter()
            .any(|line| line.starts_with("Upgrade Export Data To Strict Parity:")),
        "strict parity upgrade should apply"
    );

    let parity_response = backend.run_usb_parity_report(RunUsbParityReportRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(
        parity_response.ok,
        "parity after repair failed: {parity_response:?}"
    );
    let parity = parity_response.data.expect("parity after repair");
    let playlist = parity
        .playlist_details
        .into_iter()
        .find(|detail| detail.name == "Testi")
        .expect("playlist detail");
    // Strict repair preserves existing playlist members and converges linkage
    // without silently dropping PDB-only members. Here eDB had 2 tracks
    // (A, B) and PDB had an extra member (C), so post-repair both sides
    // should include all 3 members.
    assert_eq!(
        playlist.pdb_tracks, 3,
        "strict repair should preserve existing playlist membership"
    );
    assert_eq!(playlist.edb_tracks, 3);
    assert_eq!(playlist.only_in_pdb, 0);
    assert_eq!(playlist.only_in_edb, 0);
}

#[test]
fn repair_usb_diagnostics_strict_upgrade_is_not_proposed_when_neither_side_is_rich_enough() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize usb failed: {init:?}");

    let playlist = ExportPlaylistData {
        id: "usb-pl-thin".to_string(),
        name: "Thin Repair".to_string(),
        tracks: Vec::new(),
    };
    let manifest = ExportManifest {
        version: 1,
        generated_at: "1970-01-01T00:00:00Z".to_string(),
        playlist_id: "pl-thin".to_string(),
        playlist_name: "Thin Repair".to_string(),
        usb_root: usb.to_string_lossy().to_string(),
        options: ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![ExportManifestTrack {
            id: "t-thin".to_string(),
            master_db_id: None,
            master_content_id: None,
            content_link: None,
            position: 1,
            track_number: None,
            title: "Thin Track".to_string(),
            artist: "Artist".to_string(),
            album: None,
            bpm: None,
            key: None,
            source_path: "/tmp/thin.mp3".to_string(),
            exported_path: "/Contents/Artist/Thin/thin.mp3".to_string(),
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
            owns_exported_media: false,
            owns_artwork: false,
            owns_waveform: false,
            artwork_path: None,
            waveform_path: None,
            duration_ms: None,
        }],
    };
    write_pdb(&usb, &playlist, &manifest, true, None, None).expect("write thin pdb");

    let vendor_db = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_export_db(&vendor_db);
    conn.execute(
        "INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (1, 'Thin Repair', 0, 1)",
        [],
    )
    .expect("insert playlist");
    conn.execute(
        "INSERT INTO content (content_id, title, path) VALUES (1, 'Thin Track', '/Contents/Artist/Thin/thin.mp3')",
        [],
    )
    .expect("insert thin content");
    conn.execute(
        "INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (1, 1, 1)",
        [],
    )
    .expect("insert thin playlist content");
    drop(conn);

    let preview_response = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: false,
        selected_fix_ids: Vec::new(),
    });
    assert!(
        preview_response.ok,
        "repair preview failed: {preview_response:?}"
    );
    let preview = preview_response.data.expect("repair preview");

    // With collect-merge-write, repair is always proposed when parity fails —
    // even when both sides have thin metadata.  The merge ensures both sides
    // end up with the same data (parity), regardless of richness.
    assert!(
        preview
            .proposed_fixes
            .iter()
            .any(|fix| fix.id == "upgrade_export_data_to_strict_parity" && fix.supported),
        "strict repair should be proposed even when both sides are thin (merge ensures parity)"
    );
}

#[test]
fn repair_usb_diagnostics_apply_removes_missing_audio_references_from_db_and_pdb_entries() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let (missing_path, missing_track_id) = seed_usb_missing_audio_fixture(&backend, &usb);

    let applied_response = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: true,
        selected_fix_ids: vec!["remove_missing_audio_references".to_string()],
    });
    assert!(
        applied_response.ok,
        "repair apply failed: {applied_response:?}"
    );

    let db_path = vendor_db_dir(&usb).join("exportLibrary.db");
    let conn = open_export_db(&db_path);
    let remaining_content: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM content WHERE path = ?1",
            rusqlite::params![missing_path],
            |row| row.get(0),
        )
        .expect("count content rows post");
    assert_eq!(
        remaining_content, 0,
        "missing-audio content row should be removed from eDB"
    );
    let remaining_links: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM playlist_content WHERE content_id = ?1",
            rusqlite::params![9001i64],
            |row| row.get(0),
        )
        .expect("count playlist_content rows post");
    assert_eq!(
        remaining_links, 0,
        "playlist_content links for missing audio should be removed"
    );

    let parsed = parse_pdb(&vendor_db_dir(&usb).join("export.pdb")).expect("parse PDB post");
    assert!(
        parsed
            .playlist_entries
            .iter()
            .all(|e| e.track_id != missing_track_id),
        "playlist entries should no longer reference missing-audio track id"
    );
}

#[test]
fn remove_usb_playlist_deletes_playlist_from_edb() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(vendor_db_dir(&usb)).expect("create usb dirs");
    let db_path = vendor_db_dir(&usb).join("exportLibrary.db");
    {
        let conn = rusqlite::Connection::open(&db_path).expect("create export db");
        conn.execute_batch(&format!("PRAGMA key='{DEFAULT_USB_EDB_KEY}';"))
            .expect("set SQLCipher key");
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
            INSERT INTO playlist (playlist_id, name, attribute, sequenceNo)
              VALUES (1, 'My Playlist', 0, 1);
            INSERT INTO playlist_content (playlist_id, content_id, sequenceNo)
              VALUES (1, 10, 1);
            "#,
        )
        .expect("seed export db");
    }

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let removed = backend.remove_usb_playlist(RemoveUsbPlaylistRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: None,
        playlist_name: "My Playlist".to_string(),
    });
    assert!(removed.ok, "remove failed: {removed:?}");
    let removed_data = removed.data.expect("remove data");
    assert_eq!(removed_data.removed_from_edb, 1);

    let conn = open_export_db(&db_path);
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM playlist WHERE name = 'My Playlist'",
            [],
            |row| row.get(0),
        )
        .expect("count rows");
    assert_eq!(count, 0, "playlist row should be deleted");
}

#[test]
#[ignore = "real-device parity check — set USB_PARITY_LIBRARY_FOLDER and USB_PARITY_REFERENCE_ROOT env vars"]
fn export_to_usb_test_matches_expected_usb_content_rows_for_exported_tracks() {
    let library_folder = match std::env::var("USB_PARITY_LIBRARY_FOLDER") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("USB_PARITY_LIBRARY_FOLDER not set, skipping");
            return;
        }
    };
    let reference_root = match std::env::var("USB_PARITY_REFERENCE_ROOT") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("USB_PARITY_REFERENCE_ROOT not set, skipping");
            return;
        }
    };

    let library = PathBuf::from(&library_folder);
    let usb_expected_root = PathBuf::from(&reference_root);
    let usb_test_root = std::env::current_dir()
        .expect("current dir")
        .join("USB_TEST");
    if !library.is_dir() || !usb_expected_root.is_dir() {
        eprintln!("library or reference root does not exist, skipping");
        return;
    }
    std::fs::remove_dir_all(&usb_test_root).ok();
    std::fs::create_dir_all(&usb_test_root).expect("recreate USB_TEST root");

    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let init = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb_test_root.to_string_lossy().to_string(),
    });
    assert!(init.ok, "initialize USB_TEST failed: {init:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![library.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let search = backend.search_tracks(SearchTracksRequest {
        query: String::new(),
        limit: 10_000,
        cursor: None,
    });
    assert!(search.ok, "search failed: {search:?}");
    let tracks = search.data.expect("search data").items;
    assert!(
        !tracks.is_empty(),
        "no scanned tracks found in {library_folder}"
    );
    let track_ids = tracks.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
    let track_titles = tracks.iter().map(|t| t.title.clone()).collect::<Vec<_>>();

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "USB_TEST_1to1".to_string(),
    });
    assert!(created.ok, "create playlist failed: {created:?}");
    let playlist_id = created.data.expect("create playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids,
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add tracks failed: {added:?}");

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb_test_root.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: true,
            include_analysis: true,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");

    let expected_db = usb_expected_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("exportLibrary.db");
    let test_db = usb_test_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("exportLibrary.db");
    assert!(
        expected_db.is_file(),
        "expected USB DB missing: {}",
        expected_db.display()
    );
    assert!(
        test_db.is_file(),
        "USB_TEST DB missing: {}",
        test_db.display()
    );

    let expected_conn = open_export_db(&expected_db);
    let test_conn = open_export_db(&test_db);

    let load_columns = |conn: &rusqlite::Connection| -> BTreeSet<String> {
        let mut out = BTreeSet::<String>::new();
        let mut stmt = conn
            .prepare("PRAGMA table_info(content)")
            .expect("prepare pragma table_info(content)");
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query table_info(content)");
        for row in rows {
            out.insert(row.expect("column name"));
        }
        out
    };

    let expected_cols = load_columns(&expected_conn);
    let test_cols = load_columns(&test_conn);
    let ignored = [
        "content_id",
        "created_at",
        "updated_at",
        "rb_data_status",
        "rb_local_created",
        "rb_local_updated",
        "rb_local_deleted",
        "UUID",
        "ID",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect::<BTreeSet<_>>();
    let allowed_to_differ = [
        // App-owned identity values are intentionally local-export specific.
        "analysisDataFilePath",
        "contentLink",
        "masterContentId",
        "masterDbId",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect::<BTreeSet<_>>();
    let common_cols = expected_cols
        .intersection(&test_cols)
        .filter(|c| !ignored.contains(*c))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        !common_cols.is_empty(),
        "no comparable content columns between expected/test DBs"
    );
    let must_match_cols = common_cols
        .iter()
        .filter(|c| !allowed_to_differ.contains(*c))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        !must_match_cols.is_empty(),
        "no must-match content columns between expected/test DBs"
    );
    assert!(
        must_match_cols.iter().any(|c| c == "length"),
        "content.length must exist for parity check"
    );

    let load_row = |conn: &rusqlite::Connection,
                    title: &str,
                    selected_cols: &[String]|
     -> Option<HashMap<String, String>> {
        let select_cols = selected_cols
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {select_cols} FROM content
             WHERE lower(title) = lower(?1)
             ORDER BY content_id ASC
             LIMIT 1"
        );
        let mut stmt = conn.prepare(&sql).ok()?;
        stmt.query_row([title], |row| {
            let mut out = HashMap::<String, String>::new();
            for (idx, col) in selected_cols.iter().enumerate() {
                let v: rusqlite::types::Value = row.get(idx)?;
                out.insert(col.clone(), format!("{v:?}"));
            }
            Ok(out)
        })
        .ok()
    };

    for title in &track_titles {
        let expected = load_row(&expected_conn, title, &must_match_cols)
            .unwrap_or_else(|| panic!("expected DB missing content row for title '{title}'"));
        let actual = load_row(&test_conn, title, &must_match_cols)
            .unwrap_or_else(|| panic!("USB_TEST DB missing content row for title '{title}'"));
        assert_eq!(
            expected, actual,
            "eDB content row mismatch for title '{title}'"
        );
    }

    // ── PDB track row comparison ────────────────────────────────────
    let expected_pdb = usb_expected_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let test_pdb = usb_test_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    if expected_pdb.is_file() && test_pdb.is_file() {
        let expected_parsed =
            backend::pdb_reader::parse_pdb(&expected_pdb).expect("parse reference PDB");
        let test_parsed = backend::pdb_reader::parse_pdb(&test_pdb).expect("parse test PDB");

        // Build title→track maps for comparison
        let expected_by_title: HashMap<String, &backend::pdb_reader::PdbTrackRow> = expected_parsed
            .tracks
            .iter()
            .map(|t| (t.title.to_lowercase(), t))
            .collect();
        let test_by_title: HashMap<String, &backend::pdb_reader::PdbTrackRow> = test_parsed
            .tracks
            .iter()
            .map(|t| (t.title.to_lowercase(), t))
            .collect();

        for title in &track_titles {
            let key = title.to_lowercase();
            let expected_track = expected_by_title.get(&key);
            let test_track = test_by_title.get(&key);
            if let (Some(exp), Some(act)) = (expected_track, test_track) {
                assert_eq!(exp.title, act.title, "PDB title mismatch for '{title}'");
                // Resolve dictionary names for comparison
                let exp_artist = expected_parsed.artists.get(&exp.artist_id);
                let act_artist = test_parsed.artists.get(&act.artist_id);
                assert_eq!(exp_artist, act_artist, "PDB artist mismatch for '{title}'");
                let exp_album = expected_parsed.albums.get(&exp.album_id);
                let act_album = test_parsed.albums.get(&act.album_id);
                assert_eq!(exp_album, act_album, "PDB album mismatch for '{title}'");
                let exp_key = expected_parsed.keys.get(&exp.key_id);
                let act_key = test_parsed.keys.get(&act.key_id);
                assert_eq!(exp_key, act_key, "PDB key mismatch for '{title}'");
                assert_eq!(
                    exp.duration_seconds, act.duration_seconds,
                    "PDB duration mismatch for '{title}'"
                );
                assert_eq!(
                    exp.tempo_x100, act.tempo_x100,
                    "PDB tempo mismatch for '{title}'"
                );
                assert_eq!(
                    exp.track_number, act.track_number,
                    "PDB track_number mismatch for '{title}'"
                );
            } else {
                if expected_track.is_none() {
                    eprintln!("PDB: reference missing track '{title}' — skipping PDB comparison");
                }
                if test_track.is_none() {
                    panic!("PDB: test export missing track '{title}'");
                }
            }
        }
    }
}

/// Exports the same library to two independent USB roots and asserts the
/// eDB content rows and PDB track rows are byte-for-byte identical between
/// them. This is not a substitute for
/// `export_to_usb_test_matches_expected_usb_content_rows_for_exported_tracks`
/// above (which validates against genuine Rekordbox-produced reference
/// data and requires real hardware) -- two app-generated exports of the
/// same input can never catch a systematic bug where our exporter is
/// self-consistently wrong. What it does catch, and what nothing else in
/// this suite checks at the row-value level, is non-determinism in the
/// export pipeline itself: dictionary ID assignment, artist/album/key
/// resolution, or content-column values differing between two runs over
/// identical input.
#[test]
fn export_to_usb_produces_identical_content_and_pdb_rows_across_two_runs() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb_a = root.path().join("usb_a");
    let usb_b = root.path().join("usb_b");
    fs::create_dir_all(&media).expect("create media dir");

    let fixture_track = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("audio")
        .join("formats")
        .join("track_format_wav.wav");
    assert!(
        fixture_track.is_file(),
        "fixture audio track missing: {}",
        fixture_track.display()
    );
    fs::copy(&fixture_track, media.join("Determinism Fixture Track.wav"))
        .expect("copy fixture audio");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let search = backend.search_tracks(SearchTracksRequest {
        query: String::new(),
        limit: 10,
        cursor: None,
    });
    assert!(search.ok, "search failed: {search:?}");
    let tracks = search.data.expect("search data").items;
    assert!(!tracks.is_empty(), "no tracks found after fixture scan");
    let track_titles = tracks.iter().map(|t| t.title.clone()).collect::<Vec<_>>();

    for track in &tracks {
        seed_track_analysis_fields(&data_dir, &track.id);
    }

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "DeterminismCheck".to_string(),
    });
    assert!(created.ok, "create playlist failed: {created:?}");
    let playlist_id = created.data.expect("create playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: tracks.iter().map(|t| t.id.clone()).collect(),
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add tracks failed: {added:?}");

    for usb_root in [&usb_a, &usb_b] {
        fs::create_dir_all(usb_root).expect("create usb root");
        let init = backend.initialize_usb(InitializeUsbRequest {
            usb_root: usb_root.to_string_lossy().to_string(),
        });
        assert!(init.ok, "initialize usb failed: {init:?}");

        let exported = backend.export_to_usb(ExportToUsbRequest {
            usb_root: Some(usb_root.to_string_lossy().to_string()),
            playlist_id: playlist_id.clone(),
            options: Some(ExportToUsbOptions {
                include_artwork: false,
                include_analysis: true,
                prune_stale: false,
                ..Default::default()
            }),
        });
        assert!(exported.ok, "export to {usb_root:?} failed: {exported:?}");
    }

    // ── eDB content row comparison ──────────────────────────────────
    let conn_a = open_export_db(&vendor_db_dir(&usb_a).join("exportLibrary.db"));
    let conn_b = open_export_db(&vendor_db_dir(&usb_b).join("exportLibrary.db"));

    let load_columns = |conn: &rusqlite::Connection| -> BTreeSet<String> {
        let mut out = BTreeSet::<String>::new();
        let mut stmt = conn
            .prepare("PRAGMA table_info(content)")
            .expect("prepare pragma table_info(content)");
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query table_info(content)");
        for row in rows {
            out.insert(row.expect("column name"));
        }
        out
    };
    // App-owned identity values are intentionally per-export specific
    // (row ids, timestamps, per-run file paths) and must not be compared.
    let ignored_cols = [
        "content_id",
        "created_at",
        "updated_at",
        "rb_data_status",
        "rb_local_created",
        "rb_local_updated",
        "rb_local_deleted",
        "UUID",
        "ID",
        "analysisDataFilePath",
        "contentLink",
        "masterContentId",
        "masterDbId",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect::<BTreeSet<_>>();
    let compare_cols = load_columns(&conn_a)
        .intersection(&load_columns(&conn_b))
        .filter(|c| !ignored_cols.contains(*c))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        !compare_cols.is_empty(),
        "no comparable content columns between the two exports"
    );
    assert!(
        compare_cols.iter().any(|c| c == "length"),
        "content.length must exist for determinism check"
    );

    let load_row = |conn: &rusqlite::Connection,
                    title: &str,
                    selected_cols: &[String]|
     -> HashMap<String, String> {
        let select_cols = selected_cols
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {select_cols} FROM content
             WHERE lower(title) = lower(?1)
             ORDER BY content_id ASC
             LIMIT 1"
        );
        let mut stmt = conn.prepare(&sql).expect("prepare content select");
        stmt.query_row([title], |row| {
            let mut out = HashMap::<String, String>::new();
            for (idx, col) in selected_cols.iter().enumerate() {
                let v: rusqlite::types::Value = row.get(idx)?;
                out.insert(col.clone(), format!("{v:?}"));
            }
            Ok(out)
        })
        .unwrap_or_else(|_| panic!("content row missing for title '{title}'"))
    };

    for title in &track_titles {
        let row_a = load_row(&conn_a, title, &compare_cols);
        let row_b = load_row(&conn_b, title, &compare_cols);
        assert_eq!(row_a, row_b, "eDB content row diverged for title '{title}'");
    }

    // ── PDB track row comparison ────────────────────────────────────
    let pdb_a = parse_pdb(&vendor_db_dir(&usb_a).join("export.pdb")).expect("parse PDB a");
    let pdb_b = parse_pdb(&vendor_db_dir(&usb_b).join("export.pdb")).expect("parse PDB b");

    let by_title_a: HashMap<String, &backend::pdb_reader::PdbTrackRow> = pdb_a
        .tracks
        .iter()
        .map(|t| (t.title.to_lowercase(), t))
        .collect();
    let by_title_b: HashMap<String, &backend::pdb_reader::PdbTrackRow> = pdb_b
        .tracks
        .iter()
        .map(|t| (t.title.to_lowercase(), t))
        .collect();

    for title in &track_titles {
        let key = title.to_lowercase();
        let track_a = by_title_a
            .get(&key)
            .unwrap_or_else(|| panic!("PDB export a missing track '{title}'"));
        let track_b = by_title_b
            .get(&key)
            .unwrap_or_else(|| panic!("PDB export b missing track '{title}'"));

        assert_eq!(
            track_a.title, track_b.title,
            "PDB title mismatch for '{title}'"
        );
        assert_eq!(
            pdb_a.artists.get(&track_a.artist_id),
            pdb_b.artists.get(&track_b.artist_id),
            "PDB artist mismatch for '{title}'"
        );
        assert_eq!(
            pdb_a.albums.get(&track_a.album_id),
            pdb_b.albums.get(&track_b.album_id),
            "PDB album mismatch for '{title}'"
        );
        assert_eq!(
            pdb_a.keys.get(&track_a.key_id),
            pdb_b.keys.get(&track_b.key_id),
            "PDB key mismatch for '{title}'"
        );
        assert_eq!(
            track_a.duration_seconds, track_b.duration_seconds,
            "PDB duration mismatch for '{title}'"
        );
        assert_eq!(
            track_a.tempo_x100, track_b.tempo_x100,
            "PDB tempo mismatch for '{title}'"
        );
        assert_eq!(
            track_a.track_number, track_b.track_number,
            "PDB track_number mismatch for '{title}'"
        );
    }
}

/// Replaces a formerly-broken test of the same behavior that only ever ran
/// if a literal `./USB` directory happened to exist in the current working
/// directory (so it silently no-op'd in every real CI/dev run). This
/// version is deterministic: it scans a real local track, exports it to a
/// synthetic USB root, and browses the export back to prove
/// `materialize_usb_track_row` dedupes the USB-side row against the
/// genuine local track (by fingerprint + duration + file size) instead of
/// creating a second, disconnected row -- the root cause that let playback
/// silently fall back to the USB copy even when a local copy existed.
#[test]
fn fetch_usb_playlists_matches_existing_local_track_by_fingerprint_without_touching_it() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let track_path = media.join("Test Artist - Dedup Roundtrip.wav");
    write_test_wav(&track_path, 440.0, 500);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let local_track = backend
        .search_tracks(SearchTracksRequest {
            query: "Dedup Roundtrip".to_string(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .next()
        .expect("scanned local track");

    // Simulate an already-analyzed track (duration/waveform/bpm are
    // populated by analyze_new_tracks in real usage; setting them directly
    // keeps this test fast while still exercising the confidence-gated
    // match -- export requires a fully-analyzed track anyway).
    seed_track_analysis_fields(&data_dir, &local_track.id);
    let db_path = data_dir.join("backend.db");
    let sentinel_waveform_path: String = rusqlite::Connection::open(&db_path)
        .expect("open backend db")
        .query_row(
            "SELECT waveform_peaks_path FROM tracks WHERE id = ?1",
            rusqlite::params![local_track.id],
            |row| row.get(0),
        )
        .expect("seeded waveform path");

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Dedup Roundtrip Playlist".to_string(),
    });
    assert!(created.ok, "create playlist failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let add = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![local_track.id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(add.ok, "add to playlist failed: {add:?}");

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");

    let track_count_before: i64 = rusqlite::Connection::open(&db_path)
        .expect("open db")
        .query_row("SELECT COUNT(1) FROM tracks", [], |row| row.get(0))
        .expect("count tracks");
    assert_eq!(
        track_count_before, 1,
        "export itself must not create extra track rows"
    );

    let usb_playlists = backend.fetch_usb_playlists(FetchUsbPlaylistsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(
        usb_playlists.ok,
        "fetch usb playlists failed: {usb_playlists:?}"
    );
    let usb_track = usb_playlists
        .data
        .expect("usb playlist data")
        .items
        .into_iter()
        .flat_map(|p| p.tracks)
        .find(|t| t.title.contains("Dedup Roundtrip"))
        .expect("roundtrip usb track");

    assert_eq!(
        usb_track.local_track_id.as_deref(),
        Some(local_track.id.as_str()),
        "usb-browsed row should dedupe to the genuine local track, not spawn a placeholder"
    );

    let conn = rusqlite::Connection::open(&db_path).expect("reopen backend db");
    let track_count_after: i64 = conn
        .query_row("SELECT COUNT(1) FROM tracks", [], |row| row.get(0))
        .expect("count tracks after browse");
    assert_eq!(
        track_count_after, 1,
        "browsing the usb copy must not create a second, disconnected tracks row"
    );
    let waveform_after: Option<String> = conn
        .query_row(
            "SELECT waveform_peaks_path FROM tracks WHERE id = ?1",
            rusqlite::params![local_track.id],
            |row| row.get(0),
        )
        .expect("waveform after browse");
    assert_eq!(
        waveform_after.as_deref(),
        Some(sentinel_waveform_path.as_str()),
        "genuine local track's waveform_peaks_path must never be clobbered by a USB-sourced match"
    );

    let link_count: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM track_usb_links WHERE track_id = ?1",
            rusqlite::params![local_track.id],
            |row| row.get(0),
        )
        .expect("count track_usb_links");
    assert_eq!(
        link_count, 1,
        "expected a track_usb_links row recording this device's copy"
    );
}

#[test]
fn export_to_usb_records_usb_device_export_history() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let track_path = media.join("Test Artist - Export History.wav");
    write_test_wav(&track_path, 440.0, 500);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    let local_track = backend
        .search_tracks(SearchTracksRequest {
            query: "Export History".to_string(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .next()
        .expect("scanned local track");
    seed_track_analysis_fields(&data_dir, &local_track.id);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Export History Playlist".to_string(),
    });
    let playlist_id = created.data.expect("playlist data").playlist_id;
    backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![local_track.id.clone()],
        dedupe: DedupeMode::Skip,
    });
    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id: playlist_id.clone(),
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");

    let db_path = data_dir.join("backend.db");
    let conn = rusqlite::Connection::open(&db_path).expect("open db");
    let (found_playlist_id, playlist_name, track_count): (Option<String>, String, i64) = conn
        .query_row(
            "SELECT playlist_id, playlist_name, track_count FROM usb_device_exports",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("usb_device_exports row");
    assert_eq!(found_playlist_id, Some(playlist_id.clone()));
    assert_eq!(playlist_name, "Export History Playlist");
    assert_eq!(track_count, 1);

    // Now delete the playlist and confirm the export history row survives
    // with its snapshotted name, matching the on-drive log's behavior.
    backend.delete_playlist(backend::models::DeletePlaylistRequest {
        playlist_id: playlist_id.clone(),
    });
    let (found_playlist_id_after, playlist_name_after): (Option<String>, String) = conn
        .query_row(
            "SELECT playlist_id, playlist_name FROM usb_device_exports",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("usb_device_exports row after delete");
    assert_eq!(
        found_playlist_id_after, None,
        "playlist_id should be nulled out, not the row removed"
    );
    assert_eq!(playlist_name_after, "Export History Playlist");
}

/// The other half of the same fix: once the local copy is gone, browsing
/// the USB export again should fall back to creating a placeholder row
/// (still linked so playback/UI works), not silently do nothing.
#[test]
fn fetch_usb_playlists_creates_placeholder_when_no_local_match() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("create media dir");
    fs::create_dir_all(&usb).expect("create usb dir");

    let track_path = media.join("Test Artist - Placeholder Case.wav");
    write_test_wav(&track_path, 440.0, 500);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    let local_track = backend
        .search_tracks(SearchTracksRequest {
            query: "Placeholder Case".to_string(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .into_iter()
        .next()
        .expect("scanned local track");
    seed_track_analysis_fields(&data_dir, &local_track.id);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Placeholder Case Playlist".to_string(),
    });
    let playlist_id = created.data.expect("playlist data").playlist_id;
    backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![local_track.id.clone()],
        dedupe: DedupeMode::Skip,
    });
    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");

    // Remove the genuine local track entirely -- only the USB-side copy remains.
    let removed = backend.remove_tracks_by_source_roots(RemoveTracksBySourceRootsRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
    });
    assert!(removed.ok, "remove by source roots failed: {removed:?}");
    assert_eq!(removed.data.expect("removed data").removed, 1);

    let usb_playlists = backend.fetch_usb_playlists(FetchUsbPlaylistsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
    });
    assert!(
        usb_playlists.ok,
        "fetch usb playlists failed: {usb_playlists:?}"
    );
    let usb_track = usb_playlists
        .data
        .expect("usb playlist data")
        .items
        .into_iter()
        .flat_map(|p| p.tracks)
        .find(|t| t.title.contains("Placeholder Case"))
        .expect("roundtrip usb track");

    let placeholder_id = usb_track
        .local_track_id
        .clone()
        .expect("placeholder track id should still be materialized");
    assert_ne!(
        placeholder_id, local_track.id,
        "with no genuine local copy left, browsing must materialize a fresh placeholder row"
    );
}

#[test]
fn analyze_new_tracks_essentia_without_node_returns_error_not_panic() {
    // When engine is set to essentia but Node.js is not available, analyze_new_tracks'
    // bpm/key detection step should return a graceful error (not panic).
    let _guard = test_env_lock().lock().expect("env lock");
    let prev_enabled = std::env::var("DJTKIT_ENABLE_ESSENTIA_JS").ok();
    let prev_runner = std::env::var("DJTKIT_ESSENTIA_RUNNER").ok();
    // SAFETY: tests serialize env access through a global mutex.
    unsafe {
        std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", "1");
        // Point runner at a path that does not exist so essentia invocation will fail.
        std::env::set_var(
            "DJTKIT_ESSENTIA_RUNNER",
            "/tmp/__nonexistent_essentia_runner__",
        );
    }

    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media");
    let source = media.join("Artist - essentia_fail_test.wav");
    write_test_wav(&source, 440.0, 800);

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    // Set engine to essentia via frontend setting.
    let set_resp = backend.set_frontend_setting(SetFrontendSettingRequest {
        key: "ui_analysis_engine_v1".to_string(),
        value: Some("essentia".to_string()),
    });
    assert!(set_resp.ok, "set_frontend_setting failed: {set_resp:?}");

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let track = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
        })
        .data
        .expect("search data")
        .items
        .first()
        .expect("scanned track")
        .clone();

    // analyze_new_tracks for this track should return an error, not panic.
    let resp = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
        bpm_min: None,
        bpm_max: None,
        track_ids: vec![track.id.clone()],
        analysis_engine: None,
        ..Default::default()
    });
    // We expect either a graceful per-track failure (counted in `failed`, with a
    // warning) or success with no BPM/key (essentia returns None when the runner
    // fails). Either way, no panic, and the call itself still reports ok.
    assert!(resp.ok, "analyze_new_tracks failed: {resp:?}");
    let data = resp.data.expect("analyze data");
    assert_eq!(
        data.analyzed + data.failed,
        1,
        "expected the one track to be accounted for: {data:?}"
    );
    let combined = format!("{data:?}");
    assert!(
        !combined.contains("panic"),
        "should be a graceful error, not a panic"
    );

    // Restore env.
    match prev_enabled {
        Some(v) => unsafe { std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", v) },
        None => unsafe { std::env::remove_var("DJTKIT_ENABLE_ESSENTIA_JS") },
    }
    match prev_runner {
        Some(v) => unsafe { std::env::set_var("DJTKIT_ESSENTIA_RUNNER", v) },
        None => unsafe { std::env::remove_var("DJTKIT_ESSENTIA_RUNNER") },
    }
}

#[test]
fn frontend_setting_analysis_engine_persists_and_reads_back() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    // Default: no engine setting stored yet.
    let settings = backend.get_frontend_settings();
    assert!(settings.ok, "get_frontend_settings failed: {settings:?}");
    let values = &settings.data.expect("settings data").values;
    assert!(
        !values.contains_key("ui_analysis_engine_v1"),
        "engine setting should not be present by default"
    );

    // Set to essentia.
    let set_resp = backend.set_frontend_setting(SetFrontendSettingRequest {
        key: "ui_analysis_engine_v1".to_string(),
        value: Some("essentia".to_string()),
    });
    assert!(set_resp.ok, "set essentia failed: {set_resp:?}");

    // Read back.
    let settings2 = backend.get_frontend_settings();
    assert!(settings2.ok);
    let values2 = &settings2.data.expect("settings data").values;
    assert_eq!(
        values2.get("ui_analysis_engine_v1").map(String::as_str),
        Some("essentia"),
        "engine setting should persist as essentia"
    );

    // Switch to stratum.
    let set_resp2 = backend.set_frontend_setting(SetFrontendSettingRequest {
        key: "ui_analysis_engine_v1".to_string(),
        value: Some("stratum".to_string()),
    });
    assert!(set_resp2.ok, "set stratum failed: {set_resp2:?}");

    let settings3 = backend.get_frontend_settings();
    assert!(settings3.ok);
    let values3 = &settings3.data.expect("settings data").values;
    assert_eq!(
        values3.get("ui_analysis_engine_v1").map(String::as_str),
        Some("stratum"),
        "engine setting should persist as stratum"
    );

    // Clear (delete).
    let set_resp3 = backend.set_frontend_setting(SetFrontendSettingRequest {
        key: "ui_analysis_engine_v1".to_string(),
        value: None,
    });
    assert!(set_resp3.ok, "clear engine failed: {set_resp3:?}");

    let settings4 = backend.get_frontend_settings();
    assert!(settings4.ok);
    let values4 = &settings4.data.expect("settings data").values;
    assert!(
        !values4.contains_key("ui_analysis_engine_v1"),
        "engine setting should be cleared"
    );

    // Reject unknown key.
    let set_bad = backend.set_frontend_setting(SetFrontendSettingRequest {
        key: "ui_bogus_key_v999".to_string(),
        value: Some("test".to_string()),
    });
    assert!(!set_bad.ok, "unknown key should be rejected: {set_bad:?}");
}

// ── Essentia install helpers ──────────────────────────────────────────────────

#[test]
fn essentia_installed_false_when_missing() {
    use backend::service::check_essentia_installed;
    let root = tempdir().expect("temp root");
    assert!(!check_essentia_installed(root.path()));
}

#[test]
fn essentia_installed_true_when_package_json_present() {
    use backend::service::check_essentia_installed;
    let root = tempdir().expect("temp root");
    let pkg_dir = root.path().join("essentia/node_modules/essentia.js");
    let dist_dir = pkg_dir.join("dist");
    let dep_dir = root.path().join("essentia/node_modules/node-wav");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::create_dir_all(&dist_dir).unwrap();
    fs::create_dir_all(&dep_dir).unwrap();
    fs::write(pkg_dir.join("package.json"), b"{}").unwrap();
    fs::write(dist_dir.join("essentia-wasm.umd.js"), b"// wasm").unwrap();
    fs::write(dep_dir.join("package.json"), b"{}").unwrap();
    assert!(check_essentia_installed(root.path()));
}

#[test]
fn remove_essentia_deletes_dir_and_resets_engine() {
    use backend::models::SetFrontendSettingRequest;
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    // Seed essentia dir and set engine to essentia.
    let essentia_dir = data_dir.join("essentia/node_modules/essentia.js");
    fs::create_dir_all(&essentia_dir).unwrap();
    fs::write(essentia_dir.join("package.json"), b"{}").unwrap();
    backend.set_frontend_setting(SetFrontendSettingRequest {
        key: "ui_analysis_engine_v1".to_string(),
        value: Some("essentia".to_string()),
    });

    let resp = backend.remove_essentia();
    assert!(resp.ok, "remove_essentia failed: {resp:?}");
    assert!(
        !data_dir.join("essentia").exists(),
        "essentia dir should be deleted"
    );

    let settings = backend.get_frontend_settings();
    assert!(settings.ok);
    let values = settings.data.expect("settings data").values;
    let engine = values
        .get("ui_analysis_engine_v1")
        .map(String::as_str)
        .unwrap_or("stratum");
    assert_eq!(
        engine, "stratum",
        "engine should be reset to stratum after remove"
    );
}

#[test]
fn get_frontend_settings_essentia_installed_field() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let resp = backend.get_frontend_settings();
    assert!(resp.ok);
    assert!(
        !resp.data.as_ref().unwrap().essentia_installed,
        "should be false when not installed"
    );

    let pkg_dir = data_dir.join("essentia/node_modules/essentia.js");
    let dist_dir = pkg_dir.join("dist");
    let dep_dir = data_dir.join("essentia/node_modules/node-wav");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::create_dir_all(&dist_dir).unwrap();
    fs::create_dir_all(&dep_dir).unwrap();
    fs::write(pkg_dir.join("package.json"), b"{}").unwrap();
    fs::write(dist_dir.join("essentia-wasm.umd.js"), b"// wasm").unwrap();
    fs::write(dep_dir.join("package.json"), b"{}").unwrap();

    let resp2 = backend.get_frontend_settings();
    assert!(resp2.ok);
    assert!(
        resp2.data.unwrap().essentia_installed,
        "should be true when essentia and node-wav are present"
    );
}

#[test]
fn export_to_usb_fingerprint_fallback_reuses_foreign_scheme_track_instead_of_duplicating() {
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    fs::create_dir_all(&usb).expect("create usb dir");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize usb failed: {initialized:?}");

    // Seed a track already on the USB under a path this app's own
    // exported_media_target_path() would never generate — simulating a
    // Rekordbox-managed layout, or an older export run's naming scheme.
    let foreign_relative = "/Contents/Foreign Scheme/Unflinching-1.mp3";
    let foreign_abs = usb.join("Contents/Foreign Scheme/Unflinching-1.mp3");
    fs::create_dir_all(foreign_abs.parent().expect("foreign parent")).expect("create foreign dir");
    let audio_bytes = b"fake-mp3-bytes-for-fingerprint-fallback-regression-test";
    fs::write(&foreign_abs, audio_bytes).expect("write foreign audio");
    let file_size = audio_bytes.len() as i64;

    let playlist = ExportPlaylistData {
        id: "pl-foreign-seed".to_string(),
        name: "Foreign Seed".to_string(),
        tracks: Vec::new(),
    };
    let manifest = ExportManifest {
        version: 1,
        generated_at: "1970-01-01T00:00:00Z".to_string(),
        playlist_id: "pl-foreign-seed".to_string(),
        playlist_name: "Foreign Seed".to_string(),
        usb_root: usb.to_string_lossy().to_string(),
        options: ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        },
        exported_tracks: 1,
        skipped_tracks: 0,
        warnings: Vec::new(),
        tracks: vec![ExportManifestTrack {
            id: "t-foreign".to_string(),
            master_db_id: None,
            master_content_id: None,
            content_link: None,
            position: 1,
            track_number: Some(1),
            title: "Unflinching".to_string(),
            artist: "Kuro".to_string(),
            album: Some("KURO".to_string()),
            bpm: Some(86.0),
            key: Some("8A".to_string()),
            source_path: "/tmp/does-not-matter.mp3".to_string(),
            exported_path: foreign_relative.to_string(),
            file_modified_at: None,
            file_size_bytes: Some(file_size),
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
            duration_ms: Some(163_000),
        }],
    };
    write_pdb(&usb, &playlist, &manifest, true, None, None).expect("seed foreign-scheme pdb row");

    let before = parse_pdb(&vendor_db_dir(&usb).join("export.pdb")).expect("parse seeded pdb");
    assert_eq!(before.tracks.len(), 1);
    let seeded_track_id = before.tracks[0].id;

    // Materialize a LOCAL library track with matching size/title/artist but a
    // source file located OUTSIDE the USB, under a path this app's own naming
    // scheme would compute differently from the foreign scheme above.
    let source_dir = root.path().join("library-source");
    fs::create_dir_all(&source_dir).expect("create source dir");
    let source_path = source_dir.join("Unflinching.mp3");
    fs::write(&source_path, audio_bytes).expect("write local source audio");

    let materialized = backend.materialize_source_track(MaterializeSourceTrackRequest {
        file_path: source_path.to_string_lossy().to_string(),
        title: "Unflinching".to_string(),
        artist: "Kuro".to_string(),
        album: Some("KURO".to_string()),
        track_number: None,
        key: None,
        file_size_bytes: None,
        format_ext: Some("mp3".to_string()),
        sample_rate_hz: None,
        bit_depth: None,
        bitrate_kbps: None,
    });
    assert!(materialized.ok, "materialize failed: {materialized:?}");
    let track_id = materialized.data.expect("materialize data").track_id;

    // Analysis is required for export regardless of include_analysis, so seed a
    // minimal (dummy-content) DAT/EXT/2EX bundle and point the local track at it.
    let local_analysis_dir = root.path().join("local-analysis");
    fs::create_dir_all(&local_analysis_dir).expect("create local analysis dir");
    let waveform_dat = local_analysis_dir.join("ANLZ0000.DAT");
    fs::write(&waveform_dat, b"dat").expect("write DAT");
    fs::write(local_analysis_dir.join("ANLZ0000.EXT"), b"ext").expect("write EXT");
    fs::write(local_analysis_dir.join("ANLZ0000.2EX"), b"2ex").expect("write 2EX");

    let conn = rusqlite::Connection::open(data_dir.join("backend.db")).expect("open backend db");
    conn.execute(
        r#"
        UPDATE tracks
        SET bpm = 86.0,
            duration_ms = 163000,
            waveform_peaks_path = ?1
        WHERE id = ?2
        "#,
        rusqlite::params![waveform_dat.to_string_lossy().to_string(), track_id],
    )
    .expect("seed local track analysis fields");
    drop(conn);

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Fingerprint Fallback".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![track_id],
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add failed: {added:?}");

    let contents_before = WalkDir::new(usb.join("Contents"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();

    let exported = backend.export_to_usb(ExportToUsbRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        playlist_id,
        options: Some(ExportToUsbOptions {
            include_artwork: false,
            include_analysis: false,
            prune_stale: false,
            ..Default::default()
        }),
    });
    assert!(exported.ok, "export failed: {exported:?}");

    let contents_after = WalkDir::new(usb.join("Contents"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    assert_eq!(
        contents_after, contents_before,
        "re-exporting a track already on the USB under a foreign naming scheme must not copy a second audio file"
    );

    let after = parse_pdb(&vendor_db_dir(&usb).join("export.pdb")).expect("parse pdb after export");
    let unflinching_rows: Vec<_> = after
        .tracks
        .iter()
        .filter(|t| t.title == "Unflinching")
        .collect();
    assert_eq!(
        unflinching_rows.len(),
        1,
        "expected exactly one PDB row for the fingerprint-matched track, found: {:?}",
        after
            .tracks
            .iter()
            .map(|t| (t.id, t.track_file_path.as_str()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        unflinching_rows[0].id, seeded_track_id,
        "the existing seeded row's PDB track id must be reused, not replaced by a new one"
    );
    assert_eq!(unflinching_rows[0].track_file_path, foreign_relative);
}
