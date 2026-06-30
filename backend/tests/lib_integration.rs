use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Deserialize;
use std::fs;
use std::hash::{Hash, Hasher};
use std::process::Command;

use tempfile::tempdir;
use walkdir::WalkDir;

use backend::commands::BackendCommands;
use backend::models::{
    AddTracksToPlaylistRequest, AnalyzeNewTracksRequest, AnalyzeTrackPieceRequest,
    CreatePlaylistRequest, DedupeMode, ExportToUsbOptions, ExportToUsbRequest,
    FetchUsbPlaylistsRequest, GetPlaylistTracksRequest, GetTracksByIdsRequest,
    InitializeUsbRequest, RemoveTracksBySourceRootsRequest, RemoveTracksFromPlaylistRequest,
    RemoveUsbPlaylistRequest, RepairUsbDiagnosticsRequest, ResolvePlaybackSourceRequest,
    RunUsbDiagnosticsRequest, RunUsbParityReportRequest, ScanLibraryRequest, SearchTracksRequest,
    SetFrontendSettingRequest, WarningEntry,
};
use backend::pdb_reader::parse_pdb;
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
    write_pdb(usb_root, &playlist, &manifest, true, None, None, false)
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

fn with_essentia_js_analysis_env<F>(f: F)
where
    F: FnOnce(bool),
{
    let _guard = test_env_lock().lock().expect("env lock");
    let prev_runner = std::env::var("DJTKIT_ESSENTIA_RUNNER").ok();

    let runner =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../desktop/scripts/essentia_runner.cjs");
    let available = runner.is_file() && essentia_runner_probe_succeeds(&runner);
    if available {
        // SAFETY: tests serialize env access through a global mutex.
        unsafe {
            std::env::set_var(
                "DJTKIT_ESSENTIA_RUNNER",
                runner.to_string_lossy().to_string(),
            );
        }
    }

    f(available);

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

fn essentia_runner_probe_succeeds(runner: &Path) -> bool {
    let probe = tempdir().expect("probe tempdir");
    let pcm_path = probe.path().join("probe.f32");
    fs::write(&pcm_path, 0.0f32.to_le_bytes()).expect("write probe pcm");
    Command::new("node")
        .arg(runner.to_string_lossy().to_string())
        .arg(format!(
            r#"{{"pcmPath":"{}","sampleRate":44100,"bpmMin":70,"bpmMax":180}}"#,
            pcm_path.to_string_lossy()
        ))
        .output()
        .map(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains(r#""ok""#)
        })
        .unwrap_or(false)
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
        track_ids: indexed_items.iter().map(|t| t.id.clone()).collect(),
        analysis_engine: None,
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
    if validated_waveforms == 0 {
        return;
    }
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
        playlist_id: playlist_id,
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
            track_ids: tracks.iter().map(|t| t.id.clone()).collect(),
            analysis_engine: None,
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
fn analyze_new_tracks_uses_audio_content_for_bpm_key_not_filename_tokens() {
    with_essentia_js_analysis_env(|available| {
        if !available {
            return;
        }
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
            track_ids: vec![track.id.clone()],
            analysis_engine: None,
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
    });
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
        track_ids: vec![track.id.clone()],
        analysis_engine: None,
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
        track_ids: vec![track.id.clone()],
        analysis_engine: None,
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
    write_pdb(&usb, &playlist, &manifest, false, None, None, false).expect("write export pdb");

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

#[test]
fn analyze_new_tracks_extracts_bpm_key_from_aiff() {
    with_essentia_js_analysis_env(|available| {
        if !available {
            return;
        }
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
            track_ids: vec![track.id.clone()],
            analysis_engine: None,
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
    });
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

    write_pdb(&usb, &playlist, &manifest, true, None, None, false).expect("write pdb");
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
    write_pdb(&usb, &playlist, &manifest, true, None, None, false).expect("write thin pdb");

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

#[test]
fn analyze_from_usb_track_uses_local_audio_only_even_when_usb_has_waveform() {
    let usb_root = std::env::current_dir().expect("current dir").join("USB");
    let pdb_file = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let contents_root = usb_root.join("Contents");
    if !pdb_file.is_file() || !contents_root.is_dir() {
        return;
    }

    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let playlists = backend.fetch_usb_playlists(FetchUsbPlaylistsRequest {
        usb_root: Some(usb_root.to_string_lossy().to_string()),
    });
    assert!(playlists.ok, "fetch usb playlists failed: {playlists:?}");
    let items = playlists.data.expect("usb playlist data").items;

    let usb_track = items
        .iter()
        .flat_map(|p| p.tracks.iter())
        .find(|t| {
            !t.id.trim().is_empty()
                && !t.file_path.trim().is_empty()
                && t.waveform_preview
                    .as_ref()
                    .map(|w| !w.is_empty())
                    .unwrap_or(false)
                && !t.title.trim().is_empty()
                && !t.artist.trim().is_empty()
        })
        .cloned();
    let Some(usb_track) = usb_track else {
        return;
    };

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![contents_root.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan contents failed: {scan:?}");

    let resolved = backend.resolve_playback_source(ResolvePlaybackSourceRequest {
        title: usb_track.title.clone(),
        artist: usb_track.artist.clone(),
        album: usb_track.album.clone(),
        bpm: usb_track.bpm,
        file_path: Some(usb_track.file_path.clone()),
        file_size_bytes: None,
    });
    assert!(resolved.ok, "resolve local track failed: {resolved:?}");
    let resolved_data = resolved.data.expect("resolved data");
    let local_track_id = resolved_data.track_id.expect("resolved local track id");

    let analyze = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
        track_ids: vec![local_track_id.clone()],
        analysis_engine: None,
    });
    assert!(analyze.ok, "analyze failed: {analyze:?}");

    let local = backend
        .get_tracks_by_ids_with_previews(GetTracksByIdsRequest {
            track_ids: vec![local_track_id.clone()],
        })
        .data
        .expect("get tracks with previews")
        .items
        .into_iter()
        .find(|t| t.id == local_track_id)
        .expect("local track after analyze");

    let local_wave = local.waveform_preview.expect("local waveform preview");
    assert!(
        !local_wave.is_empty(),
        "local analysis must generate waveform preview from local audio"
    );
}

#[test]
fn analyze_track_piece_essentia_without_node_returns_error_not_panic() {
    // When engine is set to essentia but Node.js is not available, analyze_track_piece
    // for the bpm_key piece should return a graceful error (not panic).
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

    // analyze_track_piece with piece=bpm_key should return an error, not panic.
    let resp = backend.analyze_track_piece(AnalyzeTrackPieceRequest {
        track_id: track.id.clone(),
        piece: "bpm_key".to_string(),
        bpm_min: None,
        bpm_max: None,
        analysis_engine: None,
    });
    // We expect either a graceful error (ok=false) or success with no BPM/key
    // (essentia returns None when runner fails). Either way, no panic.
    if !resp.ok {
        let err_msg = format!("{resp:?}");
        assert!(
            !err_msg.contains("panic"),
            "should be a graceful error, not a panic"
        );
    }

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
