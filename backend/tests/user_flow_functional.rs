use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use backend::commands::BackendCommands;
use backend::models::{
    AddTracksToPlaylistRequest, AnalyzeNewTracksRequest, BrowseSourceFilesRequest,
    CreatePlaylistRequest, DedupeMode, ExportToUsbOptions, ExportToUsbRequest,
    FetchUsbHistoriesRequest, FetchUsbPlaylistsRequest, GetPlaylistTracksRequest,
    GetTracksByIdsRequest, InitializeUsbRequest, MaterializeSourceTrackRequest, ScanLibraryRequest,
    SearchTracksRequest,
};
use backend::pdb_reader::parse_pdb;
use tempfile::tempdir;

const USB_VENDOR_ROOT_DIR: &str = "PIONEER";
const USB_VENDOR_DB_DIR: &str = "rekordbox";

fn pdb_path(usb_root: &Path) -> PathBuf {
    usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb")
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn with_essentia_js_env<F>(f: F)
where
    F: FnOnce(bool),
{
    let _guard = env_lock().lock().expect("env lock");
    let prev_runner = std::env::var("DJTKIT_ESSENTIA_RUNNER").ok();

    let runner =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../desktop/scripts/essentia_runner.cjs");
    let available = runner.is_file() && essentia_runner_probe_succeeds(&runner);
    if available {
        // SAFETY: test scope serializes env mutation using a global mutex.
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
            // SAFETY: test scope serializes env mutation using a global mutex.
            unsafe { std::env::set_var("DJTKIT_ESSENTIA_RUNNER", v) }
        }
        None => {
            // SAFETY: test scope serializes env mutation using a global mutex.
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
fn user_like_flow_imports_sources_analyzes_and_adds_from_library_usb_and_history() {
    with_essentia_js_env(|available| {
        if !available {
            return;
        }
        let root = tempdir().expect("temp root");
        let source_a = root.path().join("source-a");
        let source_b = root.path().join("source-b");
        let usb = root.path().join("usb");
        fs::create_dir_all(&source_a).expect("create source-a");
        fs::create_dir_all(&source_b).expect("create source-b");
        fs::create_dir_all(&usb).expect("create usb root");

        write_test_pulsed_key_wav(&source_a.join("Artist - A120.wav"), 120.0, 20_000);
        write_test_pulsed_key_wav(&source_a.join("Artist - B128.wav"), 128.0, 20_000);
        write_test_pulsed_key_wav(&source_b.join("Artist - C132.wav"), 132.0, 20_000);

        let data_dir = root.path().join("data");
        let backend = BackendCommands::new(&data_dir).expect("create backend");

        let scan_all = backend.scan_library(ScanLibraryRequest {
            source_roots: vec![
                source_a.to_string_lossy().to_string(),
                source_b.to_string_lossy().to_string(),
            ],
            incremental: true,
        });
        assert!(scan_all.ok, "scan all failed: {scan_all:?}");
        let scan_all_data = scan_all.data.expect("scan all data");
        assert_eq!(scan_all_data.indexed, 3, "expected 3 indexed tracks");

        fs::remove_dir_all(&source_b).expect("remove source-b directory");
        let scan_remove_source_b = backend.scan_library(ScanLibraryRequest {
            // Simulate removing one media source from the environment.
            // The scanner prunes tracks when files disappear between incremental scans.
            source_roots: vec![
                source_a.to_string_lossy().to_string(),
                source_b.to_string_lossy().to_string(),
            ],
            incremental: true,
        });
        assert!(
            scan_remove_source_b.ok,
            "scan remove source-b failed: {scan_remove_source_b:?}"
        );
        let remove_data = scan_remove_source_b.data.expect("scan remove data");
        assert!(
            remove_data.removed >= 1,
            "expected at least one removed track after dropping source-b"
        );

        let remaining_tracks = backend
            .search_tracks(SearchTracksRequest {
                query: String::new(),
                limit: 50,
                cursor: None,
            })
            .data
            .expect("remaining tracks data")
            .items;
        assert_eq!(remaining_tracks.len(), 2, "expected two remaining tracks");
        let remaining_ids = remaining_tracks
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>();

        let analyzed = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
            track_ids: remaining_ids.clone(),
            analysis_engine: None,
        });
        assert!(analyzed.ok, "analyze failed: {analyzed:?}");
        let analyzed_data = analyzed.data.expect("analyze data");
        assert_eq!(analyzed_data.analyzed, 2, "expected 2 analyzed tracks");
        assert_eq!(analyzed_data.failed, 0, "analysis should not fail");

        let analyzed_tracks = backend
            .search_tracks(SearchTracksRequest {
                query: String::new(),
                limit: 50,
                cursor: None,
            })
            .data
            .expect("tracks after analysis")
            .items;
        assert!(
            analyzed_tracks.iter().all(|t| {
                t.waveform_peaks_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .is_some()
            }),
            "expected waveform paths for all analyzed tracks"
        );
        assert!(
            analyzed_tracks
                .iter()
                .all(|t| t.bpm.is_some() && t.key.is_some()),
            "expected bpm and key for all analyzed tracks"
        );

        let init_usb = backend.initialize_usb(InitializeUsbRequest {
            usb_root: usb.to_string_lossy().to_string(),
        });
        assert!(init_usb.ok, "initialize usb failed: {init_usb:?}");

        let source_playlist = backend.create_playlist(CreatePlaylistRequest {
            name: "Flow Source Playlist".to_string(),
        });
        assert!(source_playlist.ok, "create source playlist failed");
        let source_playlist_id = source_playlist
            .data
            .expect("source playlist data")
            .playlist_id;

        let add_source_tracks = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: source_playlist_id.clone(),
            track_ids: remaining_ids.clone(),
            dedupe: DedupeMode::Skip,
        });
        assert!(add_source_tracks.ok, "add source tracks failed");
        assert_eq!(add_source_tracks.data.expect("add source data").added, 2);

        let export = backend.export_to_usb(ExportToUsbRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
            playlist_id: source_playlist_id,
            options: Some(ExportToUsbOptions {
                include_artwork: false,
                include_analysis: true,
                prune_stale: false,
                ..Default::default()
            }),
        });
        assert!(export.ok, "export failed: {export:?}");
        assert_eq!(export.data.expect("export data").exported_tracks, 2);

        let parsed = parse_pdb(&pdb_path(&usb)).expect("parse exported pdb");
        let history_track_ids = parsed
            .tracks
            .iter()
            .take(2)
            .map(|t| t.id)
            .collect::<Vec<_>>();
        assert_eq!(history_track_ids.len(), 2, "need two exported track ids");
        append_history_to_pdb(&pdb_path(&usb), 1, &history_track_ids);

        let usb_playlists = backend.fetch_usb_playlists(FetchUsbPlaylistsRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
        });
        assert!(
            usb_playlists.ok,
            "fetch usb playlists failed: {usb_playlists:?}"
        );
        let usb_items = usb_playlists.data.expect("usb playlist data").items;
        let usb_playlist = usb_items
            .iter()
            .find(|p| p.name == "Flow Source Playlist" && !p.tracks.is_empty())
            .unwrap_or_else(|| panic!("expected exported USB playlist in {:?}", usb_items));
        let usb_local_track_id = usb_playlist
            .tracks
            .iter()
            .find_map(|t| t.local_track_id.clone())
            .expect("expected local_track_id from usb playlist materialization");

        let usb_histories = backend.fetch_usb_histories(FetchUsbHistoriesRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
        });
        assert!(
            usb_histories.ok,
            "fetch usb histories failed: {usb_histories:?}"
        );
        let history_items = usb_histories.data.expect("usb history data").items;
        assert!(
            !history_items.is_empty(),
            "expected at least one injected history playlist"
        );
        let history_local_track_id = history_items
            .iter()
            .flat_map(|h| h.tracks.iter())
            .filter_map(|t| t.local_track_id.clone())
            .find(|id| id != &usb_local_track_id)
            .unwrap_or_else(|| {
                panic!(
                    "expected distinct history local track id: {:?}",
                    history_items
                )
            });

        let target_playlist = backend.create_playlist(CreatePlaylistRequest {
            name: "Flow Target Playlist".to_string(),
        });
        assert!(target_playlist.ok, "create target playlist failed");
        let target_playlist_id = target_playlist
            .data
            .expect("target playlist data")
            .playlist_id;

        let add_from_library = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: target_playlist_id.clone(),
            track_ids: vec![remaining_ids[0].clone()],
            dedupe: DedupeMode::Skip,
        });
        assert!(add_from_library.ok, "add from library failed");
        assert_eq!(add_from_library.data.expect("add library data").added, 1);

        let add_from_usb = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: target_playlist_id.clone(),
            track_ids: vec![usb_local_track_id.clone()],
            dedupe: DedupeMode::Skip,
        });
        assert!(add_from_usb.ok, "add from usb failed");
        assert_eq!(add_from_usb.data.expect("add usb data").added, 1);

        let add_from_history = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
            playlist_id: target_playlist_id.clone(),
            track_ids: vec![history_local_track_id.clone()],
            dedupe: DedupeMode::Skip,
        });
        assert!(add_from_history.ok, "add from history failed");
        assert_eq!(add_from_history.data.expect("add history data").added, 1);

        let target_tracks = backend.get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: target_playlist_id,
        });
        assert!(target_tracks.ok, "get target playlist tracks failed");
        let target_items = target_tracks.data.expect("target tracks data").items;
        assert_eq!(
            target_items.len(),
            3,
            "expected 3 tracks in target playlist"
        );
        let id_set = target_items.into_iter().map(|t| t.id).collect::<Vec<_>>();
        assert!(id_set.iter().any(|id| id == &remaining_ids[0]));
        assert!(id_set.iter().any(|id| id == &usb_local_track_id));
        assert!(id_set.iter().any(|id| id == &history_local_track_id));

        let reloaded_tracks = backend
            .get_tracks_by_ids_with_previews(GetTracksByIdsRequest {
                track_ids: vec![usb_local_track_id.clone(), history_local_track_id.clone()],
            })
            .data
            .expect("reloaded imported tracks")
            .items;
        for track in reloaded_tracks {
            assert!(
                track.duration_ms.unwrap_or(0) > 0,
                "imported USB/history track should retain duration: {:?}",
                track
            );
            assert!(
                track
                    .waveform_peaks_path
                    .as_deref()
                    .map(|p| !p.trim().is_empty())
                    .unwrap_or(false),
                "imported USB/history track should retain waveform path: {:?}",
                track
            );
        }
    });
}

fn append_history_to_pdb(pdb_path: &Path, history_id: u32, track_ids: &[u32]) {
    let mut bytes = fs::read(pdb_path).expect("read PDB");
    let len_page = read_u32_le(&bytes, 4).expect("len_page") as usize;
    let num_tables = read_u32_le(&bytes, 8).expect("num_tables") as usize;
    let max_page = parse_max_last_page(&bytes, num_tables) as u32;

    let history_playlist_page = max_page + 1;
    let history_entries_page = max_page + 2;

    let playlist_row = build_history_playlist_row(history_id, "HISTORY 1");
    let entry_rows = track_ids
        .iter()
        .enumerate()
        .map(|(idx, track_id)| build_history_entry_row(*track_id, history_id, (idx + 1) as u32))
        .collect::<Vec<_>>();

    let playlist_page = build_pdb_page(
        11,
        history_playlist_page,
        history_playlist_page,
        &[playlist_row],
        len_page,
    );
    let entries_page = build_pdb_page(
        12,
        history_entries_page,
        history_entries_page,
        &entry_rows,
        len_page,
    );

    bytes.extend_from_slice(&playlist_page);
    bytes.extend_from_slice(&entries_page);

    let p1 = 28 + num_tables * 16;
    let p2 = p1 + 16;
    bytes[p1..p1 + 4].copy_from_slice(&11u32.to_le_bytes());
    bytes[p1 + 8..p1 + 12].copy_from_slice(&history_playlist_page.to_le_bytes());
    bytes[p1 + 12..p1 + 16].copy_from_slice(&history_playlist_page.to_le_bytes());

    bytes[p2..p2 + 4].copy_from_slice(&12u32.to_le_bytes());
    bytes[p2 + 8..p2 + 12].copy_from_slice(&history_entries_page.to_le_bytes());
    bytes[p2 + 12..p2 + 16].copy_from_slice(&history_entries_page.to_le_bytes());

    bytes[8..12].copy_from_slice(&((num_tables as u32) + 2).to_le_bytes());
    fs::write(pdb_path, bytes).expect("write PDB with history tables");
}

fn build_history_playlist_row(id: u32, name: &str) -> Vec<u8> {
    let mut row = Vec::<u8>::new();
    row.extend_from_slice(&id.to_le_bytes());
    row.extend_from_slice(&encode_pdb_ascii_string(name));
    row
}

fn build_history_entry_row(track_id: u32, playlist_id: u32, entry_index: u32) -> Vec<u8> {
    let mut row = vec![0u8; 12];
    row[0..4].copy_from_slice(&track_id.to_le_bytes());
    row[4..8].copy_from_slice(&playlist_id.to_le_bytes());
    row[8..12].copy_from_slice(&entry_index.to_le_bytes());
    row
}

fn encode_pdb_ascii_string(value: &str) -> Vec<u8> {
    let bytes = value.as_bytes();
    let mut out = Vec::<u8>::with_capacity(4 + bytes.len());
    out.push(0u8);
    out.extend_from_slice(&(4u16 + bytes.len() as u16).to_le_bytes());
    out.push(0u8);
    out.extend_from_slice(bytes);
    out
}

fn build_pdb_page(
    table_type: u32,
    page_index: u32,
    seq: u32,
    rows: &[Vec<u8>],
    len_page: usize,
) -> Vec<u8> {
    let mut page = vec![0u8; len_page];
    page[4..8].copy_from_slice(&page_index.to_le_bytes());
    page[8..12].copy_from_slice(&table_type.to_le_bytes());
    page[12..16].copy_from_slice(&0u32.to_le_bytes());
    page[16..20].copy_from_slice(&seq.to_le_bytes());

    let mut payload_offset = 0usize;
    let mut row_offsets = Vec::<u16>::new();
    for row in rows {
        row_offsets.push(payload_offset as u16);
        let start = 40 + payload_offset;
        let end = start + row.len();
        page[start..end].copy_from_slice(row);
        payload_offset += row.len();
    }

    page[24] = (rows.len() % 256) as u8;
    page[30..32].copy_from_slice(&(payload_offset as u16).to_le_bytes());
    page[34..36].copy_from_slice(&((rows.len().saturating_sub(1)) as u16).to_le_bytes());

    let mut cursor = len_page;
    for group_start in (0..rows.len()).step_by(16) {
        cursor -= 4;
        let group_len = (rows.len() - group_start).min(16);
        let bits = ((1u32 << group_len) - 1) as u16;
        page[cursor..cursor + 2].copy_from_slice(&bits.to_le_bytes());
        for j in 0..group_len {
            cursor -= 2;
            page[cursor..cursor + 2].copy_from_slice(&row_offsets[group_start + j].to_le_bytes());
        }
    }

    page
}

fn parse_max_last_page(bytes: &[u8], num_tables: usize) -> usize {
    let mut cursor = 28usize;
    let mut max_page = 0usize;
    for _ in 0..num_tables {
        let Some(last_page) = read_u32_le(bytes, cursor + 12) else {
            break;
        };
        max_page = max_page.max(last_page as usize);
        cursor += 16;
    }
    max_page
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let a = *bytes.get(offset)?;
    let b = *bytes.get(offset + 1)?;
    let c = *bytes.get(offset + 2)?;
    let d = *bytes.get(offset + 3)?;
    Some(u32::from_le_bytes([a, b, c, d]))
}

fn write_test_pulsed_key_wav(path: &Path, bpm: f32, duration_ms: u32) {
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
        out.extend_from_slice(&v.to_le_bytes());
    }

    fs::write(path, out).expect("write pulsed wav");
}

#[test]
fn analyze_local_track_waveform_preview_comes_from_anlz_roundtrip() {
    with_essentia_js_env(|available| {
        if !available {
            return;
        }
        let root = tempdir().expect("temp root");
        let data_dir = root.path().join("data");
        let source = root.path().join("source");
        fs::create_dir_all(&source).expect("create source dir");

        let wav = source.join("roundtrip_test.wav");
        write_test_pulsed_key_wav(&wav, 128.0, 8000);

        let backend = BackendCommands::new(&data_dir).expect("create backend");
        let scan = backend.scan_library(ScanLibraryRequest {
            source_roots: vec![source.to_string_lossy().to_string()],
            incremental: true,
        });
        assert!(scan.ok, "scan failed: {scan:?}");

        let tracks = backend
            .search_tracks(SearchTracksRequest {
                query: String::new(),
                limit: 50,
                cursor: None,
            })
            .data
            .expect("search tracks")
            .items;
        assert_eq!(tracks.len(), 1, "expected exactly 1 scanned track");
        let track_id = tracks[0].id.clone();

        let analyze = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
            track_ids: vec![track_id.clone()],
            analysis_engine: None,
        });
        assert!(analyze.ok, "analyze failed: {analyze:?}");
        let analyze_data = analyze.data.expect("analyze data");
        assert_eq!(analyze_data.analyzed, 1);
        assert_eq!(analyze_data.failed, 0);

        // Fetch with previews — this reads waveform from ANLZ file on disk
        let hydrated = backend
            .get_tracks_by_ids_with_previews(GetTracksByIdsRequest {
                track_ids: vec![track_id.clone()],
            })
            .data
            .expect("get tracks with previews")
            .items;
        assert_eq!(hydrated.len(), 1);
        let track = &hydrated[0];

        let waveform_path = track
            .waveform_peaks_path
            .as_deref()
            .expect("waveform_peaks_path must be set after analysis");
        assert!(
            Path::new(waveform_path).exists(),
            "ANLZ .DAT file must exist on disk: {waveform_path}"
        );

        let preview = track
            .waveform_preview
            .as_ref()
            .expect("waveform_preview must be Some after analysis");
        assert!(!preview.is_empty(), "waveform preview must have data");
        assert!(
            preview.iter().any(|&v| v > 0),
            "waveform preview must have at least one non-zero value"
        );
    });
}

#[test]
fn startup_hydration_reopen_keeps_waveform_bpm_key_and_length_without_reanalysis_side_effects() {
    with_essentia_js_env(|available| {
        if !available {
            return;
        }
        let root = tempdir().expect("temp root");
        let data_dir = root.path().join("data");
        let source = root.path().join("source");
        fs::create_dir_all(&source).expect("create source dir");

        let wav = source.join("startup_hydration.wav");
        write_test_pulsed_key_wav(&wav, 126.0, 9000);

        let backend = BackendCommands::new(&data_dir).expect("create backend");
        let scan = backend.scan_library(ScanLibraryRequest {
            source_roots: vec![source.to_string_lossy().to_string()],
            incremental: true,
        });
        assert!(scan.ok, "scan failed: {scan:?}");

        let scanned = backend
            .search_tracks(SearchTracksRequest {
                query: String::new(),
                limit: 50,
                cursor: None,
            })
            .data
            .expect("search tracks")
            .items;
        assert_eq!(scanned.len(), 1, "expected exactly 1 scanned track");
        let track_id = scanned[0].id.clone();

        let analyze = backend.analyze_new_tracks(AnalyzeNewTracksRequest {
            track_ids: vec![track_id.clone()],
            analysis_engine: None,
        });
        assert!(analyze.ok, "analyze failed: {analyze:?}");
        let analyze_data = analyze.data.expect("analyze data");
        assert_eq!(analyze_data.analyzed, 1);
        assert_eq!(analyze_data.failed, 0);

        let before = backend
            .search_tracks(SearchTracksRequest {
                query: String::new(),
                limit: 50,
                cursor: None,
            })
            .data
            .expect("search tracks after analyze")
            .items
            .into_iter()
            .find(|t| t.id == track_id)
            .expect("track exists after analyze");
        assert!(before.bpm.is_some(), "bpm should exist after analyze");
        assert!(
            before
                .key
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .is_some(),
            "key should exist after analyze"
        );
        assert!(
            before.duration_ms.unwrap_or(0) > 0,
            "duration should exist after analyze"
        );
        assert!(
            before
                .waveform_peaks_path
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .is_some(),
            "waveform path should exist after analyze"
        );

        let before_updated_at = before.updated_at.clone();
        let before_waveform_path = before.waveform_peaks_path.clone();
        let before_bpm = before.bpm;
        let before_key = before.key.clone();
        let before_duration = before.duration_ms;

        drop(backend);

        let reopened = BackendCommands::new(&data_dir).expect("reopen backend");
        let hydrated = reopened
            .get_tracks_by_ids_with_previews(GetTracksByIdsRequest {
                track_ids: vec![track_id.clone()],
            })
            .data
            .expect("hydrate tracks by ids")
            .items;
        assert_eq!(hydrated.len(), 1, "expected one hydrated track");
        let hydrated_track = &hydrated[0];
        let preview = hydrated_track
            .waveform_preview
            .as_ref()
            .expect("waveform preview must hydrate from analyzed DAT");
        assert!(
            !preview.is_empty(),
            "hydrated waveform preview should not be empty"
        );
        assert!(
            preview.iter().any(|&v| v > 0),
            "hydrated waveform preview should contain non-zero bins"
        );

        let after = reopened
            .search_tracks(SearchTracksRequest {
                query: String::new(),
                limit: 50,
                cursor: None,
            })
            .data
            .expect("search tracks after reopen")
            .items
            .into_iter()
            .find(|t| t.id == track_id)
            .expect("track exists after reopen");

        assert_eq!(
            after.waveform_peaks_path, before_waveform_path,
            "startup hydration must not rewrite waveform path"
        );
        assert_eq!(
            after.bpm, before_bpm,
            "startup hydration must not rewrite bpm"
        );
        assert_eq!(
            after.key, before_key,
            "startup hydration must not rewrite key"
        );
        assert_eq!(
            after.duration_ms, before_duration,
            "startup hydration must not rewrite duration"
        );
        assert_eq!(
            after.updated_at, before_updated_at,
            "startup hydration must not mutate updated_at (no reanalysis side effects)"
        );
    });
}

#[test]
fn browse_only_track_can_be_materialized_and_added_without_scan_or_analysis() {
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let source = root.path().join("source");
    fs::create_dir_all(&source).expect("create source dir");

    let mp3 = source.join("Artist One - Track One - 01 Track One.mp3");
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio/noart/track_no_art.mp3"),
        &mp3,
    )
    .expect("copy fixture track");

    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let browsed = backend
        .browse_source_files(BrowseSourceFilesRequest {
            source_roots: vec![source.to_string_lossy().to_string()],
            include_master_db: false,
            query: String::new(),
            limit: 50,
            cursor: None,
        })
        .data
        .expect("browse data")
        .items;
    assert_eq!(browsed.len(), 1, "expected one browsed track");
    let browsed_track = &browsed[0];
    assert_eq!(
        browsed_track.id,
        mp3.to_string_lossy().to_string(),
        "browse-only row should use file path before materialization"
    );

    let materialized = backend
        .materialize_source_track(MaterializeSourceTrackRequest {
            file_path: browsed_track.file_path.clone(),
            title: browsed_track.title.clone(),
            artist: browsed_track.artist.clone(),
            album: browsed_track.album.clone(),
            track_number: browsed_track.track_number,
            key: browsed_track.key.clone(),
            file_size_bytes: browsed_track.file_size_bytes,
            format_ext: browsed_track.format_ext.clone(),
            sample_rate_hz: browsed_track.sample_rate_hz,
            bit_depth: browsed_track.bit_depth,
            bitrate_kbps: browsed_track.bitrate_kbps,
        })
        .data
        .expect("materialize data");

    let created = backend.create_playlist(CreatePlaylistRequest {
        name: "Browse Only".to_string(),
    });
    assert!(created.ok, "create failed: {created:?}");
    let playlist_id = created.data.expect("playlist data").playlist_id;

    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist_id.clone(),
        track_ids: vec![materialized.track_id.clone()],
        dedupe: DedupeMode::Skip,
    });
    assert!(added.ok, "add failed: {added:?}");
    assert_eq!(added.data.expect("add data").added, 1);

    let playlist_tracks = backend
        .get_playlist_tracks(GetPlaylistTracksRequest { playlist_id })
        .data
        .expect("playlist tracks")
        .items;
    assert_eq!(playlist_tracks.len(), 1, "expected added browse-only track");
    assert_eq!(playlist_tracks[0].id, materialized.track_id);
    assert_eq!(
        playlist_tracks[0].file_path,
        mp3.to_string_lossy().to_string()
    );
}
