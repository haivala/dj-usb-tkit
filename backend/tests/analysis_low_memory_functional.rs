//! In its own test binary, and so its own process: it reports almost no memory
//! through the environment, which would fail any analysis running beside it.

mod library_sources;

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use backend::models::{AnalyzeNewTracksRequest, BrowseSourceFilesRequest, ScanLibraryRequest};
use backend::service::{AnalysisEvent, BackendService};
use tempfile::tempdir;

const SAMPLE_RATE: u32 = 44_100;

// Two seconds of 120 BPM pulses, 16-bit mono. Different pitches make different
// files: the library keeps one of identical ones.
fn write_pulsed_wav(path: &Path, pitch_hz: f32) {
    let samples = SAMPLE_RATE * 2;
    let data_len = samples * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    let beat = SAMPLE_RATE / 2;
    for i in 0..samples {
        let in_pulse = i % beat < beat / 4;
        let phase = i as f32 / SAMPLE_RATE as f32 * pitch_hz * std::f32::consts::TAU;
        let sample = if in_pulse {
            phase.sin() * 20_000.0
        } else {
            0.0
        };
        out.extend_from_slice(&(sample as i16).to_le_bytes());
    }
    fs::write(path, out).expect("write wav");
}

#[test]
fn a_track_waits_for_memory_while_another_runs_and_fails_when_nothing_is_left_to_free_any() {
    // SAFETY: this binary's only test, before any thread starts.
    unsafe {
        std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", "0");
        std::env::remove_var("DJTKIT_ESSENTIA_RUNNER");
        std::env::set_var("DJTKIT_ANALYSIS_MAX_WORKERS", "2");
        std::env::remove_var("DJTKIT_ANALYSIS_AVAILABLE_MEMORY_BYTES");
    }
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media");
    write_pulsed_wav(&media.join("Artist - one.wav"), 220.0);
    write_pulsed_wav(&media.join("Artist - two.wav"), 330.0);
    let source = media.to_string_lossy().to_string();
    let service = BackendService::new(root.path().join("data")).expect("create service");
    service
        .scan_library(ScanLibraryRequest {
            source_roots: vec![source.clone()],
            incremental: true,
        })
        .expect("scan succeeds");
    library_sources::show_only(&service, vec![source], false, false);
    let track_ids: Vec<String> = service
        .browse_source_files(BrowseSourceFilesRequest {
            query: String::new(),
            limit: 10,
            cursor: None,
            ..Default::default()
        })
        .expect("browse succeeds")
        .items
        .into_iter()
        .map(|track| track.id)
        .collect();
    assert_eq!(track_ids.len(), 2);

    let (ended_tx, ended_rx) = std::sync::mpsc::channel();
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorded = events.clone();
    service.set_analysis_observer(Arc::new(move |event| {
        recorded.lock().expect("events lock").push(event.clone());
        if matches!(event, AnalysisEvent::JobFinished(_)) {
            let _ = ended_tx.send(());
        }
    }));

    let mut asked = false;
    service
        .analyze_new_tracks_with_progress(
            AnalyzeNewTracksRequest {
                track_ids: vec![track_ids[0].clone()],
                ..Default::default()
            },
            |progress| {
                if progress.track_started && !asked {
                    asked = true;
                    // SAFETY: the coordinator reads this between its own steps
                    // and nothing else in this process writes it.
                    unsafe { std::env::set_var("DJTKIT_ANALYSIS_AVAILABLE_MEMORY_BYTES", "1") };
                    service
                        .submit_analysis(AnalyzeNewTracksRequest {
                            track_ids: vec![track_ids[1].clone()],
                            ..Default::default()
                        })
                        .expect("submit succeeds");
                }
            },
        )
        .expect("first request succeeds");
    ended_rx
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("the job finishes");

    let events = events.lock().expect("events lock");
    let ready: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AnalysisEvent::Track { progress, .. } if progress.track_ready => Some(progress),
            _ => None,
        })
        .collect();
    assert_eq!(ready.len(), 2);
    assert_eq!(
        ready[0].track_id, track_ids[0],
        "the running track came first"
    );
    assert!(!ready[0].failed, "{:?}", ready[0].error_message);
    assert_eq!(ready[1].track_id, track_ids[1]);
    assert!(
        ready[1]
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("Not enough available memory")),
        "{:?}",
        ready[1].error_message
    );
}
