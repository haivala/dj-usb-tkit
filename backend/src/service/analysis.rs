//! Track analysis: BPM detection, key detection, waveform generation, cover art.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::time::Instant;

use image::imageops::FilterType;
use lofty::file::TaggedFileExt;
use lofty::picture::PictureType;
use lofty::probe::read_from_path;
use rodio::Decoder;
use rodio::Source;
use rusqlite::params;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use uuid::Uuid;

use crate::error::{BackendError, BackendResult};
use crate::models::{
    AnalyzeNewTracksData, AnalyzeNewTracksRequest, AnalyzeTrackPieceData, AnalyzeTrackPieceRequest,
};

use super::anlz::{WaveformData, write_generated_anlz_bundle_with_first_beat};
use super::bpm_key::{AnalysisEngine, BpmKeyResult, detect_bpm_key_stratum};
use super::export_helpers::{LocalAnalysisResult, LocalTrackForAnalysis, stable_u32_hash};
use super::{BackendService, SETTING_UI_ANALYSIS_ENGINE, WAVEFORM_PREVIEW_BINS, now};

const ANALYSIS_DECODE_MAX_SAMPLES: usize = 24_000_000;
const ANLZ_DETAIL_DEFAULT_DURATION_MS: u64 = 180_000;
const ANLZ_DETAIL_BINS_PER_SECOND: f64 = 150.0;
const LIBRARY_ARTWORK_SIZE_PX: u32 = 80;
const DEFAULT_ANALYSIS_BPM_MIN: u32 = 70;
const DEFAULT_ANALYSIS_BPM_MAX: u32 = 180;
const TRACK_ANALYSIS_UPDATE_SQL: &str = r#"
                UPDATE tracks
                SET bpm = COALESCE(?1, bpm),
                    tonality = COALESCE(?2, tonality),
                    duration_ms = COALESCE(?3, duration_ms),
                    artwork_path = COALESCE(?4, artwork_path),
                    waveform_peaks_path = COALESCE(?5, waveform_peaks_path),
                    bpm_analyzer = COALESCE(?8, bpm_analyzer),
                    first_beat_ms = COALESCE(?9, first_beat_ms),
                    updated_at = ?6
                WHERE id = ?7
                "#;

const ANALYSIS_AUTO_SELECT_LIMIT: usize = 3000;

/// Read the configured analysis engine from app_settings. Defaults to Stratum.
fn resolve_analysis_engine(db: &crate::db::Db, requested: Option<&str>) -> AnalysisEngine {
    if let Some(raw) = requested {
        return AnalysisEngine::from_setting(raw);
    }
    let conn = match db.connect() {
        Ok(c) => c,
        Err(_) => return AnalysisEngine::Stratum,
    };
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![SETTING_UI_ANALYSIS_ENGINE],
            |row| row.get(0),
        )
        .ok();
    match raw {
        Some(s) => AnalysisEngine::from_setting(&s),
        None => AnalysisEngine::Stratum,
    }
}

/// Dispatch BPM/key detection to the selected engine.
fn detect_bpm_key(
    engine: AnalysisEngine,
    samples: &[f32],
    sample_rate: u32,
    bpm_min: u32,
    bpm_max: u32,
) -> BackendResult<BpmKeyResult> {
    match engine {
        AnalysisEngine::Stratum => detect_bpm_key_stratum(samples, sample_rate, bpm_min, bpm_max),
        AnalysisEngine::Essentia => {
            let result = detect_bpm_key_with_essentia_js(samples, sample_rate, bpm_min, bpm_max)?;
            let Some(result) = result else {
                return Err(BackendError::Internal(
                    "Essentia analysis returned no result; no fallback engine applied".to_string(),
                ));
            };
            if !essentia_result_has_detected_values(&result) {
                return Err(BackendError::Internal(
                    "Essentia analysis returned empty BPM/key result; no fallback engine applied"
                        .to_string(),
                ));
            }
            Ok(BpmKeyResult {
                bpm: result.bpm,
                key: result.key,
                first_beat_ms: result.first_beat_ms.map(|v| v.round() as u32),
            })
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct EssentiaResult {
    ok: bool,
    #[serde(default)]
    bpm: Option<f64>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    first_beat_ms: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct AnalyzeTrackProgress {
    pub current: usize,
    pub total: usize,
    pub track_id: String,
    pub track_title: String,
    pub file_path: String,
    pub bpm: Option<f64>,
    pub bpm_analyzer: Option<String>,
    pub key: Option<String>,
    pub artwork_path: Option<String>,
    pub waveform_peaks_path: Option<String>,
    pub waveform_preview: Option<Vec<u8>>,
    pub duration_ms: Option<u64>,
    pub track_ready: bool,
    pub failed: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct TrackPartialUpdate {
    bpm: Option<f64>,
    bpm_analyzer: Option<String>,
    key: Option<String>,
    artwork_path: Option<String>,
    waveform_peaks_path: Option<String>,
    waveform_preview: Option<Vec<u8>>,
    duration_ms: Option<u64>,
}

fn build_partial_progress(
    current: usize,
    total: usize,
    track_id: String,
    track_title: String,
    file_path: String,
    update: TrackPartialUpdate,
) -> AnalyzeTrackProgress {
    AnalyzeTrackProgress {
        current,
        total,
        track_id,
        track_title,
        file_path,
        bpm: update.bpm,
        bpm_analyzer: update.bpm_analyzer,
        key: update.key,
        artwork_path: update.artwork_path,
        waveform_peaks_path: update.waveform_peaks_path,
        waveform_preview: update.waveform_preview,
        duration_ms: update.duration_ms,
        track_ready: false,
        failed: false,
        error_message: None,
    }
}

fn build_done_progress_success(
    current: usize,
    total: usize,
    track: &LocalTrackForAnalysis,
    local: &LocalAnalysisResult,
) -> AnalyzeTrackProgress {
    AnalyzeTrackProgress {
        current,
        total,
        track_id: track.id.clone(),
        track_title: track.title.clone(),
        file_path: track.file_path.clone(),
        bpm: local.bpm,
        bpm_analyzer: local.bpm_analyzer.clone(),
        key: local.key.clone(),
        artwork_path: local.artwork_path.clone(),
        waveform_peaks_path: local.waveform_peaks_path.clone(),
        waveform_preview: local.waveform_preview.clone(),
        duration_ms: local.duration_ms,
        track_ready: true,
        failed: false,
        error_message: None,
    }
}

fn build_done_progress_error(
    current: usize,
    total: usize,
    track: &LocalTrackForAnalysis,
    error_message: String,
) -> AnalyzeTrackProgress {
    AnalyzeTrackProgress {
        current,
        total,
        track_id: track.id.clone(),
        track_title: track.title.clone(),
        file_path: track.file_path.clone(),
        bpm: None,
        bpm_analyzer: None,
        key: None,
        artwork_path: None,
        waveform_peaks_path: None,
        waveform_preview: None,
        duration_ms: None,
        track_ready: true,
        failed: true,
        error_message: Some(error_message),
    }
}

fn normalize_essentia_result(mut r: EssentiaResult) -> EssentiaResult {
    // Mirror AppBpmFinder display behavior: integer BPM.
    if let Some(v) = r.bpm {
        r.bpm = Some(v.round());
    }
    r
}

fn essentia_result_has_detected_values(result: &EssentiaResult) -> bool {
    if result.bpm.is_some() {
        return true;
    }
    result
        .key
        .as_ref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EssentiaWorkerConfig {
    node_bin: String,
    runner_path: PathBuf,
    pool_size: usize,
}

struct EssentiaNodeWorker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl EssentiaNodeWorker {
    fn spawn(config: &EssentiaWorkerConfig) -> BackendResult<Self> {
        let mut child = Command::new(&config.node_bin)
            .arg(config.runner_path.to_string_lossy().to_string())
            .arg("--worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|err| {
                BackendError::Internal(format!("Failed to launch BPM/key analysis worker: {err}"))
            })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            BackendError::Internal("BPM/key analysis worker missing stdin pipe".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            BackendError::Internal("BPM/key analysis worker missing stdout pipe".to_string())
        })?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    fn request(&mut self, args: &serde_json::Value) -> BackendResult<Vec<u8>> {
        self.stdin.write_all(args.to_string().as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;

        let mut line = String::new();
        let read = self.stdout.read_line(&mut line)?;
        if read == 0 {
            return Err(BackendError::Internal(
                "BPM/key analysis worker closed stdout unexpectedly".to_string(),
            ));
        }
        Ok(line.into_bytes())
    }

    fn terminate(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for EssentiaNodeWorker {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[derive(Default)]
struct EssentiaWorkerPoolState {
    config: Option<EssentiaWorkerConfig>,
    workers: Vec<Option<EssentiaNodeWorker>>,
    available: Vec<usize>,
}

#[derive(Default)]
struct EssentiaWorkerPool {
    state: Mutex<EssentiaWorkerPoolState>,
    ready: Condvar,
}

impl EssentiaWorkerPool {
    fn invoke(
        &self,
        config: &EssentiaWorkerConfig,
        args: &serde_json::Value,
    ) -> BackendResult<Vec<u8>> {
        let (idx, mut worker) = self.acquire(config)?;
        let result = worker.request(args);
        let replacement = match result {
            Ok(_) => Some(worker),
            Err(_) => EssentiaNodeWorker::spawn(config).ok(),
        };
        self.release(idx, replacement, config);
        result
    }

    fn acquire(&self, config: &EssentiaWorkerConfig) -> BackendResult<(usize, EssentiaNodeWorker)> {
        let mut state = self.state.lock().map_err(|_| {
            BackendError::Internal("analysis worker pool mutex poisoned".to_string())
        })?;
        self.ensure_config_locked(&mut state, config)?;

        loop {
            if let Some(idx) = state.available.pop()
                && let Some(worker) = state.workers.get_mut(idx).and_then(Option::take)
            {
                return Ok((idx, worker));
            }
            state = self.ready.wait(state).map_err(|_| {
                BackendError::Internal("analysis worker pool condvar poisoned".to_string())
            })?;
            self.ensure_config_locked(&mut state, config)?;
        }
    }

    fn release(
        &self,
        idx: usize,
        worker: Option<EssentiaNodeWorker>,
        config: &EssentiaWorkerConfig,
    ) {
        if let Ok(mut state) = self.state.lock()
            && state.config.as_ref() == Some(config)
        {
            if idx >= state.workers.len() {
                state.workers.resize_with(idx + 1, || None);
            }
            state.workers[idx] = worker;
            if state.workers[idx].is_some() {
                state.available.push(idx);
                self.ready.notify_one();
            } else {
                state.config = None;
                state.available.clear();
                state.workers.clear();
                self.ready.notify_all();
            }
        }
    }

    fn reset(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.config = None;
            state.available.clear();
            state.workers.clear();
            self.ready.notify_all();
        }
    }

    fn ensure_config_locked(
        &self,
        state: &mut EssentiaWorkerPoolState,
        config: &EssentiaWorkerConfig,
    ) -> BackendResult<()> {
        if state.config.as_ref() == Some(config) && state.available.len() == state.workers.len() {
            return Ok(());
        }
        if state.config.as_ref() == Some(config) {
            return Ok(());
        }

        state.available.clear();
        state.workers.clear();
        state.config = Some(config.clone());
        for idx in 0..config.pool_size {
            state.workers.push(Some(EssentiaNodeWorker::spawn(config)?));
            state.available.push(idx);
        }
        Ok(())
    }
}

fn essentia_worker_pool() -> &'static EssentiaWorkerPool {
    static POOL: OnceLock<EssentiaWorkerPool> = OnceLock::new();
    POOL.get_or_init(EssentiaWorkerPool::default)
}

fn resolve_analysis_bpm_range(bpm_min: Option<u32>, bpm_max: Option<u32>) -> (u32, u32) {
    let min = bpm_min.unwrap_or(DEFAULT_ANALYSIS_BPM_MIN);
    let max = bpm_max.unwrap_or(DEFAULT_ANALYSIS_BPM_MAX);
    if min == 0 || max == 0 || min >= max {
        return (DEFAULT_ANALYSIS_BPM_MIN, DEFAULT_ANALYSIS_BPM_MAX);
    }
    (min, max)
}

impl BackendService {
    pub fn analyze_new_tracks(
        &self,
        req: AnalyzeNewTracksRequest,
    ) -> BackendResult<AnalyzeNewTracksData> {
        self.analyze_new_tracks_with_progress(req, |_| {})
    }

    pub fn analyze_new_tracks_with_progress<F>(
        &self,
        req: AnalyzeNewTracksRequest,
        mut on_progress: F,
    ) -> BackendResult<AnalyzeNewTracksData>
    where
        F: FnMut(&AnalyzeTrackProgress),
    {
        let job_id = format!("job-analysis-{}", Uuid::now_v7());
        let conn = self.db.connect()?;
        let auto_mode = req.track_ids.is_empty();
        let auto_eligible_total = if auto_mode {
            Some(count_tracks_missing_core_fields(&conn)?)
        } else {
            None
        };
        let mut tracks = collect_tracks_for_analysis(&conn, &req.track_ids)?;
        if tracks.is_empty() {
            return Ok(AnalyzeNewTracksData {
                job_id,
                analyzed: 0,
                failed: 0,
                warnings: vec!["No eligible tracks found for analysis".to_string()],
            });
        }

        let analysis_dir = self.db.data_dir().join("analysis");
        let waveform_dir = analysis_dir.join("waveforms");
        let artwork_dir = analysis_dir.join("artwork");
        std::fs::create_dir_all(&waveform_dir)?;
        std::fs::create_dir_all(&artwork_dir)?;

        let mut analyzed = 0usize;
        let mut failed = 0usize;
        let mut warnings = Vec::<String>::new();
        if let Some(total) = auto_eligible_total
            && total > tracks.len()
        {
            warnings.push(format!(
                    "Auto analysis limit reached: selected {} of {} eligible tracks (limit {}). Run analysis again or select tracks explicitly to continue.",
                    tracks.len(),
                    total,
                    ANALYSIS_AUTO_SELECT_LIMIT
                ));
        }
        let total = tracks.len();
        let (bpm_min, bpm_max) = resolve_analysis_bpm_range(None, None);
        let engine = resolve_analysis_engine(&self.db, req.analysis_engine.as_deref());

        let mut completed_count = 0usize;

        if total > 1 {
            let tx_db = conn.unchecked_transaction()?;
            let mut persist_stmt = tx_db.prepare_cached(TRACK_ANALYSIS_UPDATE_SQL)?;
            let cpu_workers = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1);
            let worker_count = resolve_analysis_worker_count(total, cpu_workers);
            let debug_workers = std::env::var("DJTKIT_ANALYSIS_DEBUG_WORKERS")
                .ok()
                .map(|v| {
                    let s = v.trim().to_ascii_lowercase();
                    !(s.is_empty() || s == "0" || s == "false" || s == "off")
                })
                .unwrap_or(false);

            if debug_workers {
                crate::logging::emit(
                    crate::logging::Level::Info,
                    "analysis",
                    &format!(
                        "debug workers total_tracks={} cpu_workers={} worker_count={}",
                        total, cpu_workers, worker_count
                    ),
                );
            }

            let queue = Arc::new(Mutex::new(
                tracks
                    .drain(..)
                    .collect::<std::collections::VecDeque<LocalTrackForAnalysis>>(),
            ));

            enum WorkerEvent {
                Partial {
                    worker_idx: usize,
                    track_id: String,
                    track_title: String,
                    file_path: String,
                    update: TrackPartialUpdate,
                },
                Done {
                    worker_idx: usize,
                    elapsed_ms: u64,
                    track: LocalTrackForAnalysis,
                    result: BackendResult<LocalAnalysisResult>,
                },
            }
            #[derive(Default, Clone)]
            struct WorkerStats {
                processed: usize,
                ok: usize,
                failed: usize,
                elapsed_ms: u64,
            }
            let mut worker_stats = vec![WorkerStats::default(); worker_count];
            let (tx, rx) = mpsc::channel::<WorkerEvent>();
            let mut handles = Vec::with_capacity(worker_count);
            for worker_idx in 0..worker_count {
                let tx = tx.clone();
                let waveform_dir = waveform_dir.clone();
                let artwork_dir = artwork_dir.clone();
                let queue = queue.clone();
                let handle = std::thread::spawn(move || {
                    loop {
                        let track = {
                            let mut guard = match queue.lock() {
                                Ok(g) => g,
                                Err(_) => break,
                            };
                            guard.pop_front()
                        };
                        let Some(track) = track else {
                            break;
                        };

                        let started = Instant::now();
                        let track_id = track.id.clone();
                        let track_title = track.title.clone();
                        let file_path = track.file_path.clone();
                        let tx_partial = tx.clone();
                        let result = analyze_track_with_usb_fallback_with_updates(
                            &track,
                            &waveform_dir,
                            &artwork_dir,
                            bpm_min,
                            bpm_max,
                            engine,
                            |update| {
                                let _ = tx_partial.send(WorkerEvent::Partial {
                                    worker_idx,
                                    track_id: track_id.clone(),
                                    track_title: track_title.clone(),
                                    file_path: file_path.clone(),
                                    update,
                                });
                            },
                        );
                        if tx
                            .send(WorkerEvent::Done {
                                worker_idx,
                                elapsed_ms: started.elapsed().as_millis() as u64,
                                track,
                                result,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                handles.push(handle);
            }
            drop(tx);

            while completed_count < total {
                let evt = rx.recv().map_err(|_| {
                    BackendError::Internal(
                        "analysis worker channel closed unexpectedly".to_string(),
                    )
                })?;
                match evt {
                    WorkerEvent::Partial {
                        worker_idx,
                        track_id,
                        track_title,
                        file_path,
                        update,
                    } => {
                        if debug_workers {
                            crate::logging::emit(
                                crate::logging::Level::Info,
                                "analysis",
                                &format!(
                                    "debug worker={} partial track_id={}",
                                    worker_idx, track_id
                                ),
                            );
                        }
                        on_progress(&build_partial_progress(
                            completed_count,
                            total,
                            track_id,
                            track_title,
                            file_path,
                            update,
                        ));
                    }
                    WorkerEvent::Done {
                        worker_idx,
                        elapsed_ms,
                        track,
                        result,
                    } => {
                        let was_ok = result.is_ok();
                        if let Some(stats) = worker_stats.get_mut(worker_idx) {
                            stats.processed += 1;
                            stats.elapsed_ms = stats.elapsed_ms.saturating_add(elapsed_ms);
                            if was_ok {
                                stats.ok += 1;
                            } else {
                                stats.failed += 1;
                            }
                        }
                        persist_done_result(
                            &mut persist_stmt,
                            track,
                            result,
                            total,
                            &mut completed_count,
                            &mut analyzed,
                            &mut failed,
                            &mut warnings,
                            &mut on_progress,
                        )?;
                    }
                }
            }

            drop(persist_stmt);
            tx_db.commit()?;

            if debug_workers {
                for (idx, stats) in worker_stats.iter().enumerate() {
                    if stats.processed == 0 {
                        continue;
                    }
                    crate::logging::emit(
                        crate::logging::Level::Info,
                        "analysis",
                        &format!(
                            "debug worker={} processed={} ok={} failed={} elapsed_ms={}",
                            idx, stats.processed, stats.ok, stats.failed, stats.elapsed_ms
                        ),
                    );
                }
            }

            for handle in handles {
                handle.join().map_err(|panic_payload| {
                    let msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                        format!("analysis worker thread panicked: {s}")
                    } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                        format!("analysis worker thread panicked: {s}")
                    } else {
                        "analysis worker thread panicked".to_string()
                    };
                    BackendError::Internal(msg)
                })?;
            }
        } else {
            let tx_db = conn.unchecked_transaction()?;
            let mut persist_stmt = tx_db.prepare_cached(TRACK_ANALYSIS_UPDATE_SQL)?;
            for track in tracks.drain(..) {
                let result = analyze_track_with_usb_fallback_with_updates(
                    &track,
                    &waveform_dir,
                    &artwork_dir,
                    bpm_min,
                    bpm_max,
                    engine,
                    |update| {
                        on_progress(&build_partial_progress(
                            completed_count,
                            total,
                            track.id.clone(),
                            track.title.clone(),
                            track.file_path.clone(),
                            update,
                        ));
                    },
                );
                persist_done_result(
                    &mut persist_stmt,
                    track,
                    result,
                    total,
                    &mut completed_count,
                    &mut analyzed,
                    &mut failed,
                    &mut warnings,
                    &mut on_progress,
                )?;
            }
            drop(persist_stmt);
            tx_db.commit()?;
        }

        Ok(AnalyzeNewTracksData {
            job_id,
            analyzed,
            failed,
            warnings,
        })
    }

    pub fn analyze_track_piece(
        &self,
        req: AnalyzeTrackPieceRequest,
    ) -> BackendResult<AnalyzeTrackPieceData> {
        let track_id = req.track_id.trim().to_string();
        if track_id.is_empty() {
            return Err(BackendError::Validation(
                "track_id is required for analyze_track_piece".to_string(),
            ));
        }
        let piece = req.piece.trim().to_ascii_lowercase();
        if piece.is_empty() {
            return Err(BackendError::Validation(
                "piece is required for analyze_track_piece".to_string(),
            ));
        }

        let track = {
            let conn = self.db.connect()?;
            let mut track_rows =
                collect_tracks_for_analysis(&conn, std::slice::from_ref(&track_id))?;
            track_rows
                .drain(..)
                .next()
                .ok_or_else(|| BackendError::NotFound(format!("track not found: {track_id}")))?
        };
        let path = PathBuf::from(&track.file_path);
        if !path.exists() {
            return Err(BackendError::NotFound(format!(
                "audio file not found: {}",
                path.display()
            )));
        }

        let analysis_dir = self.db.data_dir().join("analysis");
        let waveform_dir = analysis_dir.join("waveforms");
        let artwork_dir = analysis_dir.join("artwork");
        std::fs::create_dir_all(&waveform_dir)?;
        std::fs::create_dir_all(&artwork_dir)?;
        let updated_at = now();
        let (bpm_min, bpm_max) = resolve_analysis_bpm_range(req.bpm_min, req.bpm_max);
        let engine = resolve_analysis_engine(&self.db, req.analysis_engine.as_deref());

        match piece.as_str() {
            "duration" => {
                let duration_ms = detect_track_duration_ms(&path);
                let conn = self.db.connect()?;
                conn.execute(
                    "UPDATE tracks SET duration_ms = COALESCE(?1, duration_ms), updated_at = ?2 WHERE id = ?3",
                    params![duration_ms, updated_at, track.id],
                )?;
                Ok(AnalyzeTrackPieceData {
                    track_id: track.id,
                    piece,
                    updated: duration_ms.is_some(),
                    bpm: None,
                    bpm_analyzer: None,
                    key: None,
                    duration_ms,
                    artwork_path: None,
                    waveform_peaks_path: None,
                    waveform_preview: None,
                })
            }
            "artwork" => {
                let artwork_path = resolve_persisted_artwork(&path, &artwork_dir, &track.id);
                let conn = self.db.connect()?;
                conn.execute(
                    "UPDATE tracks SET artwork_path = COALESCE(?1, artwork_path), updated_at = ?2 WHERE id = ?3",
                    params![artwork_path, updated_at, track.id],
                )?;
                Ok(AnalyzeTrackPieceData {
                    track_id: track.id,
                    piece,
                    updated: artwork_path.is_some(),
                    bpm: None,
                    bpm_analyzer: None,
                    key: None,
                    duration_ms: None,
                    artwork_path,
                    waveform_peaks_path: None,
                    waveform_preview: None,
                })
            }
            "waveform" => {
                // Fetch current BPM/duration/first_beat_ms from DB so ANLZ files include beat grid + correct entry counts.
                let conn = self.db.connect()?;
                let (db_bpm, db_duration, db_first_beat_ms): (
                    Option<f64>,
                    Option<u64>,
                    Option<u32>,
                ) = conn
                    .query_row(
                        "SELECT bpm, duration_ms, first_beat_ms FROM tracks WHERE id = ?1",
                        params![track.id],
                        |row| {
                            Ok((
                                row.get(0)?,
                                row.get(1)?,
                                row.get::<_, Option<i64>>(2)?.map(|v| v as u32),
                            ))
                        },
                    )
                    .unwrap_or((None, None, None));
                drop(conn);

                let anlz_duration = db_duration.or_else(|| detect_track_duration_ms(&path));
                let waveform = build_waveform_data_for_track(
                    &path,
                    waveform_detail_bins_for_duration(anlz_duration),
                )?;
                let (dat_path, ext_path, twoex_path) =
                    local_analysis_bundle_paths(&waveform_dir, &track.id, &track.file_path);
                let waveform_peaks_path = if waveform.peaks.is_empty() {
                    None
                } else {
                    write_generated_anlz_bundle_with_first_beat(
                        &waveform,
                        &dat_path,
                        &ext_path,
                        &twoex_path,
                        "",
                        db_bpm,
                        anlz_duration,
                        db_first_beat_ms,
                    )?;
                    Some(dat_path.to_string_lossy().to_string())
                };
                let waveform_preview =
                    waveform_preview_if_persisted(&waveform, &waveform_peaks_path);
                let conn = self.db.connect()?;
                conn.execute(
                    "UPDATE tracks SET waveform_peaks_path = COALESCE(?1, waveform_peaks_path), updated_at = ?2 WHERE id = ?3",
                    params![waveform_peaks_path, updated_at, track.id],
                )?;
                Ok(AnalyzeTrackPieceData {
                    track_id: track.id,
                    piece,
                    updated: waveform_peaks_path.is_some() || waveform_preview.is_some(),
                    bpm: None,
                    bpm_analyzer: None,
                    key: None,
                    duration_ms: None,
                    artwork_path: None,
                    waveform_peaks_path,
                    waveform_preview,
                })
            }
            "bpm_key" => {
                let decoded = decode_audio_mono_samples(&path, ANALYSIS_DECODE_MAX_SAMPLES).ok();
                let bpm_key_result = match decoded.as_ref() {
                    Some((samples, sample_rate)) => {
                        detect_bpm_key(engine, samples, *sample_rate, bpm_min, bpm_max)?
                    }
                    None => BpmKeyResult {
                        bpm: None,
                        key: None,
                        first_beat_ms: None,
                    },
                };
                let bpm = bpm_key_result.bpm;
                let key = bpm_key_result.key;
                let first_beat_ms = bpm_key_result.first_beat_ms.map(|v| v as i64);
                let analyzer_label: Option<&str> = if bpm.is_some() {
                    Some(engine.as_str())
                } else {
                    None
                };
                let conn = self.db.connect()?;
                conn.execute(
                    "UPDATE tracks SET bpm = COALESCE(?1, bpm), tonality = COALESCE(?2, tonality), bpm_analyzer = COALESCE(?5, bpm_analyzer), first_beat_ms = COALESCE(?6, first_beat_ms), updated_at = ?3 WHERE id = ?4",
                    params![bpm, key, updated_at, track.id, analyzer_label, first_beat_ms],
                )?;
                Ok(AnalyzeTrackPieceData {
                    track_id: track.id,
                    piece,
                    updated: bpm.is_some() || key.is_some(),
                    bpm,
                    bpm_analyzer: analyzer_label.map(|s| s.to_string()),
                    key,
                    duration_ms: None,
                    artwork_path: None,
                    waveform_peaks_path: None,
                    waveform_preview: None,
                })
            }
            _ => Err(BackendError::Validation(format!(
                "unsupported analyze piece '{}'; expected one of: duration, artwork, waveform, bpm_key",
                req.piece
            ))),
        }
    }
}

fn persist_done_result(
    persist_stmt: &mut rusqlite::CachedStatement<'_>,
    track: LocalTrackForAnalysis,
    result: BackendResult<LocalAnalysisResult>,
    total: usize,
    completed_count: &mut usize,
    analyzed: &mut usize,
    failed: &mut usize,
    warnings: &mut Vec<String>,
    on_progress: &mut dyn FnMut(&AnalyzeTrackProgress),
) -> BackendResult<()> {
    *completed_count += 1;
    match result {
        Ok(local) => {
            persist_stmt.execute(params![
                local.bpm,
                local.key,
                local.duration_ms,
                local.artwork_path,
                local.waveform_peaks_path,
                now(),
                track.id,
                local.bpm_analyzer,
                local.first_beat_ms.map(|v| v as i64),
            ])?;
            *analyzed += 1;
            on_progress(&build_done_progress_success(
                *completed_count,
                total,
                &track,
                &local,
            ));
        }
        Err(err) => {
            *failed += 1;
            let err_msg = err.to_string();
            warnings.push(format!("{}: {}", track.file_path, err_msg));
            on_progress(&build_done_progress_error(
                *completed_count,
                total,
                &track,
                err_msg,
            ));
        }
    }
    Ok(())
}

fn waveform_preview_if_persisted(
    waveform: &WaveformData,
    waveform_peaks_path: &Option<String>,
) -> Option<Vec<u8>> {
    waveform_peaks_path
        .as_ref()
        .map(|_| downsample_waveform_peaks(&waveform.peaks, WAVEFORM_PREVIEW_BINS))
}

pub(crate) fn waveform_detail_entries_for_duration(duration_ms: Option<u64>) -> usize {
    let duration_ms = duration_ms.unwrap_or(ANLZ_DETAIL_DEFAULT_DURATION_MS);
    (((duration_ms as f64 / 1000.0) * ANLZ_DETAIL_BINS_PER_SECOND).ceil() as usize)
        .saturating_add(4)
        .max(400)
}

pub(crate) fn waveform_detail_bins_for_duration(duration_ms: Option<u64>) -> usize {
    waveform_detail_entries_for_duration(duration_ms).max(WAVEFORM_PREVIEW_BINS)
}

fn downsample_waveform_peaks(peaks: &[u8], bins: usize) -> Vec<u8> {
    if bins == 0 || peaks.is_empty() {
        return Vec::new();
    }
    if peaks.len() == bins {
        return peaks.to_vec();
    }
    (0..bins)
        .map(|i| {
            let start = i * peaks.len() / bins;
            let mut end = ((i + 1) * peaks.len()) / bins;
            if end <= start {
                end = (start + 1).min(peaks.len());
            }
            peaks[start.min(peaks.len().saturating_sub(1))..end.min(peaks.len())]
                .iter()
                .copied()
                .max()
                .unwrap_or(0)
        })
        .collect()
}

fn resolve_persisted_artwork(path: &Path, artwork_dir: &Path, track_id: &str) -> Option<String> {
    extract_embedded_cover_art_bytes(path)
        .as_deref()
        .and_then(|bytes| {
            persist_library_artwork_thumbnail_from_bytes(bytes, artwork_dir, track_id)
        })
        .or_else(|| {
            discover_cover_art_path(path)
                .or_else(|| discover_cover_art_in_parent(path))
                .as_deref()
                .and_then(|found| persist_library_artwork_thumbnail(found, artwork_dir, track_id))
        })
}

fn build_waveform_data_for_track(path: &Path, bins: usize) -> BackendResult<WaveformData> {
    let decoded = decode_audio_mono_samples(path, ANALYSIS_DECODE_MAX_SAMPLES).ok();
    waveform_data_from_decoded_or_file(
        decoded
            .as_ref()
            .map(|(samples, sample_rate)| (samples.as_slice(), *sample_rate)),
        path,
        bins,
    )
}

fn waveform_data_from_decoded_or_file(
    decoded: Option<(&[f32], u32)>,
    path: &Path,
    bins: usize,
) -> BackendResult<WaveformData> {
    match decoded {
        Some((samples, sample_rate)) => Ok(build_waveform_data_from_samples_with_rate(
            samples,
            bins,
            sample_rate,
        )),
        None => Ok(WaveformData::from_peaks(
            build_waveform_preview_from_file_bytes(path, bins, 256 * 1024)?,
        )),
    }
}

pub(crate) fn collect_tracks_for_analysis(
    conn: &rusqlite::Connection,
    requested_track_ids: &[String],
) -> BackendResult<Vec<LocalTrackForAnalysis>> {
    if requested_track_ids.is_empty() {
        let sql = format!(
            r#"
            SELECT id, title, file_path
            FROM tracks
            WHERE bpm IS NULL
               OR tonality IS NULL
               OR duration_ms IS NULL
               OR artwork_path IS NULL
               OR waveform_peaks_path IS NULL
            ORDER BY updated_at ASC
            LIMIT {}
            "#,
            ANALYSIS_AUTO_SELECT_LIMIT
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(LocalTrackForAnalysis {
                id: row.get(0)?,
                title: row.get(1)?,
                file_path: row.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        return Ok(out);
    }

    let mut ordered_ids = Vec::<String>::new();
    let mut seen = std::collections::HashSet::<String>::new();
    for raw in requested_track_ids {
        let id = raw.trim();
        if id.is_empty() {
            continue;
        }
        if seen.insert(id.to_string()) {
            ordered_ids.push(id.to_string());
        }
    }
    if ordered_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = vec!["?"; ordered_ids.len()].join(", ");
    let sql = format!("SELECT id, title, file_path FROM tracks WHERE id IN ({placeholders})");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(ordered_ids.iter()), |row| {
        Ok(LocalTrackForAnalysis {
            id: row.get(0)?,
            title: row.get(1)?,
            file_path: row.get(2)?,
        })
    })?;

    let mut by_id = std::collections::HashMap::<String, LocalTrackForAnalysis>::new();
    for row in rows {
        let track = row?;
        by_id.insert(track.id.clone(), track);
    }

    let mut out = Vec::with_capacity(ordered_ids.len());
    for id in ordered_ids {
        if let Some(track) = by_id.remove(&id) {
            out.push(track);
        }
    }
    Ok(out)
}

fn count_tracks_missing_core_fields(conn: &rusqlite::Connection) -> BackendResult<usize> {
    let total: i64 = conn.query_row(
        r#"
        SELECT COUNT(*)
        FROM tracks
        WHERE bpm IS NULL
           OR tonality IS NULL
           OR duration_ms IS NULL
           OR artwork_path IS NULL
           OR waveform_peaks_path IS NULL
        "#,
        [],
        |row| row.get(0),
    )?;
    Ok(total.max(0) as usize)
}

fn analyze_track_with_usb_fallback_with_updates<F>(
    track: &LocalTrackForAnalysis,
    waveform_dir: &Path,
    artwork_dir: &Path,
    bpm_min: u32,
    bpm_max: u32,
    engine: AnalysisEngine,
    mut on_update: F,
) -> BackendResult<LocalAnalysisResult>
where
    F: FnMut(TrackPartialUpdate),
{
    analyze_local_track_with_updates(
        &track.file_path,
        &track.id,
        waveform_dir,
        artwork_dir,
        bpm_min,
        bpm_max,
        engine,
        &mut on_update,
    )
}

pub(crate) fn resolve_analysis_worker_count(total_tracks: usize, cpu_workers: usize) -> usize {
    resolve_analysis_worker_count_with_cap(
        total_tracks,
        cpu_workers,
        analysis_worker_cap_from_env(),
    )
}

pub(crate) fn resolve_analysis_worker_count_with_cap(
    total_tracks: usize,
    cpu_workers: usize,
    cap: Option<usize>,
) -> usize {
    let tracks = total_tracks.max(1);
    let cpus = match cap {
        Some(c) if c > 0 => cpu_workers.max(1).min(c),
        _ => cpu_workers.max(1),
    };
    tracks.min(cpus)
}

pub(crate) fn resolve_analysis_parallelism_budget(cpu_workers: usize) -> usize {
    resolve_analysis_parallelism_budget_with_cap(cpu_workers, analysis_worker_cap_from_env())
}

pub(crate) fn resolve_analysis_parallelism_budget_with_cap(
    cpu_workers: usize,
    cap: Option<usize>,
) -> usize {
    let budget = cpu_workers.saturating_sub(2).max(1);
    match cap {
        Some(c) if c > 0 => budget.min(c),
        _ => budget,
    }
}

fn analysis_worker_cap_from_env() -> Option<usize> {
    std::env::var("DJTKIT_ANALYSIS_MAX_WORKERS")
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
}

fn analyze_local_track_with_updates(
    file_path: &str,
    track_id: &str,
    waveform_dir: &Path,
    artwork_dir: &Path,
    bpm_min: u32,
    bpm_max: u32,
    engine: AnalysisEngine,
    on_update: &mut dyn FnMut(TrackPartialUpdate),
) -> BackendResult<LocalAnalysisResult> {
    let path = PathBuf::from(file_path);
    if !path.exists() {
        return Err(BackendError::NotFound(format!(
            "audio file not found: {}",
            path.display()
        )));
    }

    let decoded = decode_audio_mono_samples(&path, ANALYSIS_DECODE_MAX_SAMPLES).ok();
    let bpm_key_result = match decoded.as_ref() {
        Some((samples, sample_rate)) => {
            detect_bpm_key(engine, samples, *sample_rate, bpm_min, bpm_max)?
        }
        None => BpmKeyResult {
            bpm: None,
            key: None,
            first_beat_ms: None,
        },
    };
    let bpm = bpm_key_result.bpm;
    let key = bpm_key_result.key;
    let first_beat_ms = bpm_key_result.first_beat_ms;
    let duration_ms = detect_track_duration_ms(&path).or_else(|| {
        decoded.as_ref().and_then(|(samples, sample_rate)| {
            duration_ms_from_decoded(samples.len(), *sample_rate)
        })
    });
    if duration_ms.is_some() {
        on_update(TrackPartialUpdate {
            duration_ms,
            ..TrackPartialUpdate::default()
        });
    }
    let waveform = waveform_data_from_decoded_or_file(
        decoded
            .as_ref()
            .map(|(samples, sample_rate)| (samples.as_slice(), *sample_rate)),
        &path,
        waveform_detail_bins_for_duration(duration_ms),
    )?;
    let persisted_artwork = resolve_persisted_artwork(&path, artwork_dir, track_id);
    if persisted_artwork.is_some() {
        on_update(TrackPartialUpdate {
            artwork_path: persisted_artwork.clone(),
            ..TrackPartialUpdate::default()
        });
    }

    let (dat_path, ext_path, twoex_path) =
        local_analysis_bundle_paths(waveform_dir, track_id, file_path);
    let waveform_peaks_path = if waveform.peaks.is_empty() {
        None
    } else {
        write_generated_anlz_bundle_with_first_beat(
            &waveform,
            &dat_path,
            &ext_path,
            &twoex_path,
            "",
            bpm,
            duration_ms,
            first_beat_ms,
        )?;
        Some(dat_path.to_string_lossy().to_string())
    };

    let waveform_preview = waveform_preview_if_persisted(&waveform, &waveform_peaks_path);
    if waveform_peaks_path.is_some() || waveform_preview.is_some() {
        on_update(TrackPartialUpdate {
            waveform_peaks_path: waveform_peaks_path.clone(),
            waveform_preview: waveform_preview.clone(),
            ..TrackPartialUpdate::default()
        });
    }
    if bpm.is_some() || key.is_some() {
        on_update(TrackPartialUpdate {
            bpm,
            bpm_analyzer: if bpm.is_some() {
                Some(engine.as_str().to_string())
            } else {
                None
            },
            key: key.clone(),
            ..TrackPartialUpdate::default()
        });
    }

    Ok(LocalAnalysisResult {
        bpm,
        bpm_analyzer: if bpm.is_some() {
            Some(engine.as_str().to_string())
        } else {
            None
        },
        key,
        first_beat_ms,
        duration_ms,
        artwork_path: persisted_artwork,
        waveform_peaks_path,
        waveform_preview,
    })
}

fn duration_ms_from_decoded(sample_count: usize, sample_rate: u32) -> Option<u64> {
    if sample_count == 0 || sample_rate == 0 {
        return None;
    }
    Some((sample_count as u64).saturating_mul(1000) / u64::from(sample_rate))
}

fn detect_track_duration_ms(path: &Path) -> Option<u64> {
    // Deterministic single-source duration resolution: Symphonia metadata only.
    let file = File::open(path).ok()?;
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let format = probed.format;
    let track = format.default_track()?;
    let sr = u64::from(track.codec_params.sample_rate?);
    let frames = track.codec_params.n_frames?;
    if sr == 0 || frames == 0 {
        return None;
    }
    Some(frames.saturating_mul(1000) / sr)
}

fn persist_library_artwork_thumbnail(
    source: &Path,
    artwork_dir: &Path,
    track_id: &str,
) -> Option<String> {
    let img = image::open(source).ok()?;
    persist_library_artwork_thumbnail_from_image(img, artwork_dir, track_id)
}

fn persist_library_artwork_thumbnail_from_bytes(
    bytes: &[u8],
    artwork_dir: &Path,
    track_id: &str,
) -> Option<String> {
    let img = image::load_from_memory(bytes).ok()?;
    persist_library_artwork_thumbnail_from_image(img, artwork_dir, track_id)
}

fn persist_library_artwork_thumbnail_from_image(
    img: image::DynamicImage,
    artwork_dir: &Path,
    track_id: &str,
) -> Option<String> {
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return None;
    }
    let side = w.min(h);
    let x = (w - side) / 2;
    let y = (h - side) / 2;
    let cropped = img.crop_imm(x, y, side, side);
    let resized = cropped.resize_exact(
        LIBRARY_ARTWORK_SIZE_PX,
        LIBRARY_ARTWORK_SIZE_PX,
        FilterType::Lanczos3,
    );

    let target = artwork_dir.join(format!("{track_id}.jpg"));
    let mut buf = Vec::<u8>::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 82);
    encoder.encode_image(&resized).ok()?;
    atomic_write_bytes(&target, &buf).ok()?;
    Some(target.to_string_lossy().to_string())
}

fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> BackendResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| BackendError::Internal("missing parent directory".to_string()))?;
    std::fs::create_dir_all(parent)?;
    let tmp_name = format!(
        ".{}.tmp.{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("tmp"),
        Uuid::now_v7()
    );
    let tmp_path = parent.join(tmp_name);
    std::fs::write(&tmp_path, bytes)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

fn extract_embedded_cover_art_bytes(path: &Path) -> Option<Vec<u8>> {
    let tagged = read_from_path(path).ok()?;
    let mut fallback = None::<Vec<u8>>;
    for tag in tagged.tags() {
        for picture in tag.pictures() {
            let data = picture.data();
            if data.is_empty() {
                continue;
            }
            if picture.pic_type() == PictureType::CoverFront {
                return Some(data.to_vec());
            }
            if fallback.is_none() {
                fallback = Some(data.to_vec());
            }
        }
    }
    fallback
}

fn detect_bpm_key_with_essentia_js(
    samples: &[f32],
    sample_rate: u32,
    bpm_min: u32,
    bpm_max: u32,
) -> BackendResult<Option<EssentiaResult>> {
    if samples.is_empty() || sample_rate == 0 {
        return Ok(None);
    }
    let config = resolve_essentia_worker_config()?;

    let tmp_path = std::env::temp_dir().join(format!("djtkit-essentia-{}.f32", Uuid::now_v7()));
    let mut tmp = File::create(&tmp_path)?;
    for &s in samples {
        tmp.write_all(&s.to_le_bytes())?;
    }
    tmp.flush()?;

    let args = serde_json::json!({
        "pcmPath": tmp_path.to_string_lossy().to_string(),
        "sampleRate": sample_rate,
        "bpmMin": bpm_min,
        "bpmMax": bpm_max
    });

    let debug = std::env::var("DJTKIT_ESSENTIA_DEBUG")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let output = match essentia_worker_pool().invoke(&config, &args) {
        Ok(output) => output,
        Err(err) => {
            essentia_worker_pool().reset();
            run_essentia_request_once(&config, &args).map_err(|fallback_err| {
                BackendError::Internal(format!(
                    "Failed to launch BPM/key analysis runtime: worker={err}; fallback={fallback_err}"
                ))
            })?
        }
    };
    let _ = std::fs::remove_file(&tmp_path);
    if debug {
        crate::logging::emit(
            crate::logging::Level::Info,
            "essentia-js",
            &format!("stdout={}", String::from_utf8_lossy(&output)),
        );
    }
    let parsed: EssentiaResult = match serde_json::from_slice(&output) {
        Ok(parsed) => parsed,
        Err(err) => {
            let stdout_text = String::from_utf8_lossy(&output).trim().to_string();
            let message = format!("BPM/key analysis runtime returned invalid JSON: {err}");
            let details = format!(
                "node={} runner={} stdout={}",
                config.node_bin,
                config.runner_path.display(),
                stdout_text
            );
            return Err(BackendError::Internal(format!("{message} | {details}")));
        }
    };
    if debug {
        crate::logging::emit(
            crate::logging::Level::Info,
            "essentia-js",
            &format!(
                "parsed ok={} bpm={:?} key={:?}",
                parsed.ok, parsed.bpm, parsed.key
            ),
        );
    }
    if !parsed.ok {
        let stdout_text = String::from_utf8_lossy(&output).trim().to_string();
        let message = "BPM/key analysis runtime reported failure".to_string();
        let details = format!(
            "node={} runner={} stdout={}",
            config.node_bin,
            config.runner_path.display(),
            stdout_text
        );
        return Err(BackendError::Internal(format!("{message} | {details}")));
    }
    if !essentia_result_has_detected_values(&parsed) {
        let message = "BPM/key analysis runtime returned empty BPM/key result".to_string();
        let details = format!(
            "node={} runner={}",
            config.node_bin,
            config.runner_path.display()
        );
        return Err(BackendError::Internal(format!("{message} | {details}")));
    }
    Ok(Some(normalize_essentia_result(parsed)))
}

fn resolve_essentia_worker_config() -> BackendResult<EssentiaWorkerConfig> {
    let node_bin = std::env::var("DJTKIT_ESSENTIA_NODE").unwrap_or_else(|_| "node".to_string());
    let runner_script = std::env::var("DJTKIT_ESSENTIA_RUNNER")
        .unwrap_or_else(|_| "../desktop/scripts/essentia_runner.cjs".to_string());
    let runner_candidate = PathBuf::from(&runner_script);
    let runner_path = if runner_candidate.is_absolute() {
        runner_candidate
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(runner_candidate)
    };

    let node_is_explicit_path =
        node_bin.contains(std::path::MAIN_SEPARATOR) || node_bin.contains('/');
    if node_is_explicit_path {
        let node_path = PathBuf::from(&node_bin);
        if !node_path.exists() {
            let message = format!(
                "BPM/key analysis runtime missing: bundled node not found at {}",
                node_path.display()
            );
            let details = format!("runner={}", runner_path.display());
            return Err(BackendError::Internal(format!("{message} | {details}")));
        }
    }

    if !runner_path.exists() {
        let message = format!("BPM/key analysis runner missing: {}", runner_path.display());
        let details = format!("node={}", node_bin);
        return Err(BackendError::Internal(format!("{message} | {details}")));
    }

    let cpu_workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let pool_size = resolve_analysis_parallelism_budget(cpu_workers);
    Ok(EssentiaWorkerConfig {
        node_bin,
        runner_path,
        pool_size,
    })
}

fn run_essentia_request_once(
    config: &EssentiaWorkerConfig,
    args: &serde_json::Value,
) -> BackendResult<Vec<u8>> {
    let output = Command::new(&config.node_bin)
        .arg(config.runner_path.to_string_lossy().to_string())
        .arg(args.to_string())
        .output()
        .map_err(|err| {
            let details = format!(
                "node={} runner={}",
                config.node_bin,
                config.runner_path.display()
            );
            BackendError::Internal(format!(
                "Failed to launch BPM/key analysis runtime: {err} | {details}"
            ))
        })?;

    if !output.status.success() {
        let stderr_text = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr_text.is_empty() {
            format!(
                "BPM/key analysis runtime exited with non-zero status: {}",
                output.status
            )
        } else {
            format!(
                "BPM/key analysis runtime exited with non-zero status: {} ({})",
                output.status, stderr_text
            )
        };
        let details = format!(
            "node={} runner={}",
            config.node_bin,
            config.runner_path.display()
        );
        return Err(BackendError::Internal(format!("{message} | {details}")));
    }

    Ok(output.stdout)
}

pub(crate) fn local_analysis_bundle_paths(
    waveform_dir: &Path,
    track_id: &str,
    source_path: &str,
) -> (PathBuf, PathBuf, PathBuf) {
    let normalized = source_path.trim().replace('\\', "/").to_ascii_lowercase();
    let seed = if normalized.is_empty() {
        track_id.to_string()
    } else {
        format!("src:{normalized}")
    };
    let base = format!("{:08X}", stable_u32_hash(&seed));
    (
        waveform_dir.join(format!("{base}.DAT")),
        waveform_dir.join(format!("{base}.EXT")),
        waveform_dir.join(format!("{base}.2EX")),
    )
}

pub(crate) fn normalize_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect::<String>()
}

pub(crate) fn decode_audio_mono_samples(
    path: &Path,
    max_samples: usize,
) -> BackendResult<(Vec<f32>, u32)> {
    let primary_err = match decode_audio_mono_samples_rodio(path, max_samples) {
        Ok(result) => return Ok(result),
        Err(err) => err,
    };

    match decode_audio_mono_samples_symphonia(path, max_samples) {
        Ok(result) => Ok(result),
        Err(fallback_err) => Err(BackendError::Internal(format!(
            "decoder error (rodio: {primary_err}; symphonia: {fallback_err})"
        ))),
    }
}

fn decode_audio_mono_samples_rodio(
    path: &Path,
    max_samples: usize,
) -> Result<(Vec<f32>, u32), String> {
    let file = File::open(path).map_err(|err| err.to_string())?;
    let decoder = Decoder::new(BufReader::new(file)).map_err(|err| format!("{err}"))?;
    let sample_rate = decoder.sample_rate().max(1);
    let channels = decoder.channels().max(1) as usize;

    let mut mono = Vec::<f32>::with_capacity(max_samples);
    let mut acc = 0.0f32;
    let mut in_frame = 0usize;
    let inv_channels = 1.0f32 / channels as f32;
    // rodio Decoder yields i16 samples (-32768..32767); normalise to -1.0..1.0.
    let scale = 1.0 / i16::MAX as f32;
    for sample in decoder.take(max_samples.saturating_mul(channels)) {
        acc += sample as f32 * scale;
        in_frame += 1;
        if in_frame == channels {
            mono.push(acc * inv_channels);
            acc = 0.0;
            in_frame = 0;
        }
    }
    if in_frame > 0 {
        mono.push(acc / in_frame as f32);
    }
    if mono.is_empty() {
        return Err("decoder produced no samples".to_string());
    }
    Ok((mono, sample_rate))
}

fn decode_audio_mono_samples_symphonia(
    path: &Path,
    max_samples: usize,
) -> Result<(Vec<f32>, u32), String> {
    let file = File::open(path).map_err(|err| err.to_string())?;
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|err| err.to_string())?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| "no default audio track".to_string())?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|err| err.to_string())?;

    let mut mono = Vec::<f32>::with_capacity(max_samples);
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(44_100).max(1);
    let mut sample_buffer: Option<SampleBuffer<f32>> = None;
    let mut buffer_spec: Option<(u32, usize)> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                break;
            }
            Err(err) => return Err(err.to_string()),
        };
        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(err) => return Err(err.to_string()),
        };

        sample_rate = decoded.spec().rate.max(1);
        let channels = decoded.spec().channels.count().max(1);
        let spec_key = (sample_rate, channels);
        let needs_recreate = buffer_spec != Some(spec_key)
            || sample_buffer
                .as_ref()
                .map(|buf| buf.capacity() < decoded.capacity() as usize)
                .unwrap_or(true);
        if needs_recreate {
            sample_buffer = Some(SampleBuffer::<f32>::new(
                decoded.capacity() as u64,
                *decoded.spec(),
            ));
            buffer_spec = Some(spec_key);
        }
        let sample_buffer_ref = match sample_buffer.as_mut() {
            Some(buf) => buf,
            None => return Err("decoder buffer allocation failed".to_string()),
        };
        sample_buffer_ref.copy_interleaved_ref(decoded);
        let inv_channels = 1.0f32 / channels as f32;

        for frame in sample_buffer_ref.samples().chunks(channels) {
            let avg = frame.iter().copied().sum::<f32>() * inv_channels;
            mono.push(avg);
            if mono.len() >= max_samples {
                break;
            }
        }
        if mono.len() >= max_samples {
            break;
        }
    }

    if mono.is_empty() {
        return Err("decoder produced no samples".to_string());
    }

    Ok((mono, sample_rate))
}

pub(crate) fn discover_cover_art_path(audio_path: &Path) -> Option<PathBuf> {
    let parent = audio_path.parent()?;
    discover_cover_art_in_dir(parent)
}

pub(crate) fn discover_cover_art_in_parent(audio_path: &Path) -> Option<PathBuf> {
    let parent = audio_path.parent()?.parent()?;
    discover_cover_art_in_dir(parent)
}

fn discover_cover_art_in_dir(dir: &Path) -> Option<PathBuf> {
    let preferred = [
        "cover.jpg",
        "folder.jpg",
        "front.jpg",
        "cover.png",
        "folder.png",
    ];
    for name in preferred {
        let candidate = dir.join(name);
        if candidate.exists() && candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub(crate) fn build_waveform_preview_from_audio(
    path: &Path,
    bins: usize,
    max_samples: usize,
) -> BackendResult<WaveformData> {
    if bins == 0 {
        return Ok(WaveformData::empty());
    }
    let (samples, sample_rate) = match decode_audio_mono_samples(path, max_samples) {
        Ok(decoded) => decoded,
        Err(_) => {
            return Ok(WaveformData::from_peaks(
                build_waveform_preview_from_file_bytes(path, bins, 256 * 1024)?,
            ));
        }
    };
    if samples.is_empty() {
        return Ok(WaveformData::from_peaks(
            build_waveform_preview_from_file_bytes(path, bins, 256 * 1024)?,
        ));
    }
    Ok(build_waveform_data_from_samples_with_rate(
        &samples,
        bins,
        sample_rate,
    ))
}

#[cfg(test)]
pub(crate) fn build_waveform_preview_from_samples(samples: &[f32], bins: usize) -> Vec<u8> {
    if bins == 0 || samples.is_empty() {
        return Vec::new();
    }
    let n = samples.len();
    let mut levels = Vec::<f32>::with_capacity(bins);
    let mut max_val = 0.0f32;
    for i in 0..bins {
        let start = i * n / bins;
        let end = ((i + 1) * n / bins).max(start + 1).min(n);
        let mut local_peak = 0.0f32;
        let mut sum_sq = 0.0f32;
        let mut count = 0usize;
        for &v in &samples[start..end] {
            let a = v.abs();
            if a > local_peak {
                local_peak = a;
            }
            sum_sq += a * a;
            count += 1;
        }
        let rms = if count > 0 {
            (sum_sq / count as f32).sqrt()
        } else {
            0.0
        };
        let level = (local_peak * 0.35) + (rms * 0.65);
        if level > max_val {
            max_val = level;
        }
        levels.push(level);
    }

    if max_val <= f32::EPSILON {
        return vec![0; bins];
    }
    levels
        .into_iter()
        .map(|level| ((level / max_val) * 100.0).round().clamp(0.0, 100.0) as u8)
        .collect::<Vec<u8>>()
}

/// Build waveform data with both amplitude peaks (0-100) and frequency bands (0-5) per bin.
///
/// Frequency band is determined by zero-crossing rate (ZCR) within each bin:
///   0 = sub-bass, 1 = bass, 2 = low-mid, 3 = mid, 4 = high-mid, 5 = treble
///
/// This matches the External library PWAV encoding where high 3 bits encode the dominant
/// frequency band and low 5 bits encode amplitude.
#[cfg(test)]
pub(crate) fn build_waveform_data_from_samples(samples: &[f32], bins: usize) -> WaveformData {
    build_waveform_data_from_samples_with_rate(samples, bins, 44100)
}

pub(crate) fn build_waveform_data_from_samples_with_rate(
    samples: &[f32],
    bins: usize,
    sample_rate: u32,
) -> WaveformData {
    if bins == 0 || samples.is_empty() {
        return WaveformData::empty();
    }

    // Pre-filter the entire signal into 3 bands using 2nd-order Butterworth IIR filters.
    // Crossover frequencies: low < 350Hz, mid 350-1000Hz, high > 1000Hz.
    // Tuned to match reference-export energy distribution (see docs/WAVEFORMS.md).
    let sr = sample_rate as f64;
    let (low_filtered, mid_filtered, high_filtered) = three_band_split(samples, sr, 350.0, 1000.0);

    let n = samples.len();
    let mut raw_peak_levels = Vec::<f32>::with_capacity(bins);
    let mut zcr_values = Vec::<f32>::with_capacity(bins);
    let mut raw_low = Vec::<f32>::with_capacity(bins);
    let mut raw_mid = Vec::<f32>::with_capacity(bins);
    let mut raw_high = Vec::<f32>::with_capacity(bins);
    let mut max_sample_peak = 0.0f32;

    for i in 0..bins {
        let start = i * n / bins;
        let end = ((i + 1) * n / bins).max(start + 1).min(n);
        let slice = &samples[start..end];

        // Amplitude: same weighted peak+RMS as build_waveform_preview_from_samples
        let mut local_peak = 0.0f32;
        for &v in slice {
            let a = v.abs();
            if a > local_peak {
                local_peak = a;
            }
        }
        if local_peak > max_sample_peak {
            max_sample_peak = local_peak;
        }
        raw_peak_levels.push(local_peak);

        // 3-band RMS energy per bin from filtered audio.
        let low_rms = rms_of_slice(&low_filtered[start..end]);
        let mid_rms = rms_of_slice(&mid_filtered[start..end]);
        let high_rms = rms_of_slice(&high_filtered[start..end]);
        raw_low.push(low_rms);
        raw_mid.push(mid_rms);
        raw_high.push(high_rms);

        // Zero-crossing rate: count sign changes normalized to 0.0..1.0
        let mut crossings = 0u32;
        if slice.len() > 1 {
            for w in slice.windows(2) {
                if (w[0] >= 0.0) != (w[1] >= 0.0) {
                    crossings += 1;
                }
            }
        }
        let zcr = if slice.len() > 1 {
            crossings as f32 / (slice.len() - 1) as f32
        } else {
            0.0
        };
        zcr_values.push(zcr);
    }

    if max_sample_peak <= f32::EPSILON {
        return WaveformData {
            peaks: vec![0; bins],
            bands: vec![3; bins],
            low_energy: vec![0; bins],
            mid_energy: vec![0; bins],
            high_energy: vec![0; bins],
            low_energy_full: vec![0; bins],
            mid_energy_full: vec![0; bins],
            high_energy_full: vec![0; bins],
            peak_level: 0.0,
        };
    }

    let peaks: Vec<u8> = raw_peak_levels
        .iter()
        .map(|level| {
            let normalized = (level / max_sample_peak).clamp(0.0, 1.0);
            (normalized * 100.0).round().clamp(0.0, 100.0) as u8
        })
        .collect();

    // Scale 3-band energies to bytes using one shared per-track p95 reference.
    // All bands share the same reference so relative balance is preserved.
    // Used by PWV6/PWV7 (stacked 3-band rendering).
    let low_ref = percentile(&raw_low, 0.95).max(1e-6);
    let mid_ref = percentile(&raw_mid, 0.95).max(1e-6);
    let high_ref = percentile(&raw_high, 0.95).max(1e-6);
    let common_ref = low_ref.max(mid_ref).max(high_ref);
    let low_energy: Vec<u8> = raw_low
        .iter()
        .map(|&v| ((v / common_ref) * 127.0).round().clamp(0.0, 127.0) as u8)
        .collect();
    let mid_energy: Vec<u8> = raw_mid
        .iter()
        .map(|&v| ((v / common_ref) * 127.0).round().clamp(0.0, 127.0) as u8)
        .collect();
    let high_energy: Vec<u8> = raw_high
        .iter()
        .map(|&v| ((v / common_ref) * 127.0).round().clamp(0.0, 127.0) as u8)
        .collect();

    // Per-band scaled to full 0-127 range using max normalization.
    // Max-based scaling preserves the natural energy distribution shape
    // while ensuring each band's peaks reach 127.
    // Used by PWV4 where each lane needs full dynamic range.
    let low_max = raw_low.iter().copied().fold(0.0f32, f32::max).max(1e-6);
    let mid_max = raw_mid.iter().copied().fold(0.0f32, f32::max).max(1e-6);
    let high_max = raw_high.iter().copied().fold(0.0f32, f32::max).max(1e-6);
    let low_energy_full: Vec<u8> = raw_low
        .iter()
        .map(|&v| ((v / low_max) * 127.0).round().clamp(0.0, 127.0) as u8)
        .collect();
    let mid_energy_full: Vec<u8> = raw_mid
        .iter()
        .map(|&v| ((v / mid_max) * 127.0).round().clamp(0.0, 127.0) as u8)
        .collect();
    let high_energy_full: Vec<u8> = raw_high
        .iter()
        .map(|&v| ((v / high_max) * 127.0).round().clamp(0.0, 127.0) as u8)
        .collect();

    // Map ZCR to frequency band 0-5.
    let bands: Vec<u8> = zcr_values
        .iter()
        .map(|&zcr| {
            if zcr < 0.01 {
                0 // sub-bass
            } else if zcr < 0.03 {
                1 // bass
            } else if zcr < 0.06 {
                2 // low-mid
            } else if zcr < 0.10 {
                3 // mid
            } else if zcr < 0.18 {
                4 // high-mid
            } else {
                5 // treble
            }
        })
        .collect();

    WaveformData {
        peaks,
        bands,
        low_energy,
        mid_energy,
        high_energy,
        low_energy_full,
        mid_energy_full,
        high_energy_full,
        peak_level: max_sample_peak,
    }
}

fn rms_of_slice(slice: &[f32]) -> f32 {
    if slice.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = slice.iter().map(|&v| v * v).sum();
    (sum_sq / slice.len() as f32).sqrt()
}

fn percentile(values: &[f32], q: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let qq = q.clamp(0.0, 1.0);
    let idx = ((sorted.len().saturating_sub(1)) as f32 * qq).round() as usize;
    sorted[idx.min(sorted.len().saturating_sub(1))]
}

/// Split audio samples into 3 frequency bands using cascaded 2nd-order Butterworth filters.
///
/// Returns (low, mid, high) where:
///   low  = frequencies below `low_cutoff` Hz
///   mid  = frequencies between `low_cutoff` and `high_cutoff` Hz
///   high = frequencies above `high_cutoff` Hz
fn three_band_split(
    samples: &[f32],
    sample_rate: f64,
    low_cutoff: f64,
    high_cutoff: f64,
) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let n = samples.len();

    // Low-pass at low_cutoff → gives us the low band
    let low_band = biquad_lowpass_filter(samples, sample_rate, low_cutoff);

    // High-pass at high_cutoff → gives us the high band
    let high_band = biquad_highpass_filter(samples, sample_rate, high_cutoff);

    // Mid = original - low - high
    let mid_band: Vec<f32> = (0..n)
        .map(|i| samples[i] - low_band[i] - high_band[i])
        .collect();

    (low_band, mid_band, high_band)
}

/// 2nd-order Butterworth low-pass filter (biquad, direct form II transposed).
fn biquad_lowpass_filter(samples: &[f32], sample_rate: f64, cutoff: f64) -> Vec<f32> {
    let (b0, b1, b2, a1, a2) = biquad_lowpass_coeffs(sample_rate, cutoff);
    biquad_apply(samples, b0, b1, b2, a1, a2)
}

/// 2nd-order Butterworth high-pass filter (biquad, direct form II transposed).
fn biquad_highpass_filter(samples: &[f32], sample_rate: f64, cutoff: f64) -> Vec<f32> {
    let (b0, b1, b2, a1, a2) = biquad_highpass_coeffs(sample_rate, cutoff);
    biquad_apply(samples, b0, b1, b2, a1, a2)
}

/// Compute 2nd-order Butterworth low-pass biquad coefficients.
fn biquad_lowpass_coeffs(sample_rate: f64, cutoff: f64) -> (f64, f64, f64, f64, f64) {
    let omega = 2.0 * std::f64::consts::PI * cutoff / sample_rate;
    let cos_w = omega.cos();
    let sin_w = omega.sin();
    let alpha = sin_w / (2.0 * std::f64::consts::SQRT_2); // Q = sqrt(2)/2 for Butterworth

    let a0 = 1.0 + alpha;
    let b0 = ((1.0 - cos_w) / 2.0) / a0;
    let b1 = (1.0 - cos_w) / a0;
    let b2 = b0;
    let a1 = (-2.0 * cos_w) / a0;
    let a2 = (1.0 - alpha) / a0;

    (b0, b1, b2, a1, a2)
}

/// Compute 2nd-order Butterworth high-pass biquad coefficients.
fn biquad_highpass_coeffs(sample_rate: f64, cutoff: f64) -> (f64, f64, f64, f64, f64) {
    let omega = 2.0 * std::f64::consts::PI * cutoff / sample_rate;
    let cos_w = omega.cos();
    let sin_w = omega.sin();
    let alpha = sin_w / (2.0 * std::f64::consts::SQRT_2);

    let a0 = 1.0 + alpha;
    let b0 = ((1.0 + cos_w) / 2.0) / a0;
    let b1 = (-(1.0 + cos_w)) / a0;
    let b2 = b0;
    let a1 = (-2.0 * cos_w) / a0;
    let a2 = (1.0 - alpha) / a0;

    (b0, b1, b2, a1, a2)
}

/// Apply biquad filter (direct form II transposed).
fn biquad_apply(samples: &[f32], b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Vec<f32> {
    let n = samples.len();
    let mut out = Vec::with_capacity(n);
    let mut z1 = 0.0f64;
    let mut z2 = 0.0f64;

    for &s in samples {
        let x = s as f64;
        let y = b0 * x + z1;
        z1 = b1 * x - a1 * y + z2;
        z2 = b2 * x - a2 * y;
        out.push(y as f32);
    }

    out
}

pub(crate) fn build_waveform_preview_from_file_bytes(
    path: &Path,
    bins: usize,
    max_bytes: usize,
) -> BackendResult<Vec<u8>> {
    if bins == 0 {
        return Ok(Vec::new());
    }
    let mut file = File::open(path)?;
    let mut bytes = vec![0u8; max_bytes.max(1)];
    let read = file.read(&mut bytes)?;
    bytes.truncate(read);
    if bytes.is_empty() {
        return Ok(vec![0; bins]);
    }

    let n = bytes.len();
    let mut out = Vec::<u8>::with_capacity(bins);
    for i in 0..bins {
        let start = i * n / bins;
        let end = ((i + 1) * n / bins).max(start + 1).min(n);
        let slice = &bytes[start..end];
        let mut sum = 0f32;
        for b in slice {
            sum += (*b as f32 - 128.0).abs() / 128.0;
        }
        let mean = if slice.is_empty() {
            0.0
        } else {
            sum / slice.len() as f32
        };
        out.push((mean * 100.0).round().clamp(0.0, 100.0) as u8);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{
        EssentiaResult, build_waveform_data_from_samples, build_waveform_preview_from_samples,
        collect_tracks_for_analysis, essentia_result_has_detected_values,
        resolve_analysis_bpm_range, resolve_analysis_parallelism_budget,
        resolve_analysis_parallelism_budget_with_cap, resolve_analysis_worker_count,
        resolve_analysis_worker_count_with_cap, waveform_detail_bins_for_duration,
        waveform_detail_entries_for_duration, waveform_preview_if_persisted,
    };
    use crate::service::WAVEFORM_PREVIEW_BINS;
    use crate::service::anlz::WaveformData;
    use rusqlite::Connection;

    fn setup_tracks_table(conn: &Connection) {
        conn.execute_batch(
            r#"
            CREATE TABLE tracks (
              id TEXT PRIMARY KEY,
              title TEXT NOT NULL,
              file_path TEXT NOT NULL,
              bpm REAL,
              tonality TEXT,
              duration_ms INTEGER,
              artwork_path TEXT,
              waveform_peaks_path TEXT,
              updated_at TEXT NOT NULL
            );
            "#,
        )
        .expect("create tracks table");
    }

    fn insert_track(conn: &Connection, id: &str, title: &str) {
        conn.execute(
            "INSERT INTO tracks (id, title, file_path, updated_at) VALUES (?1, ?2, ?3, '2026-04-01T00:00:00Z')",
            rusqlite::params![id, title, format!("/music/{id}.mp3")],
        )
        .expect("insert track");
    }

    #[test]
    fn resolve_analysis_worker_count_uses_detected_parallelism() {
        let workers = resolve_analysis_worker_count(20, 24);
        assert_eq!(workers, 20);
    }

    #[test]
    fn resolve_analysis_worker_count_is_bounded_by_tracks_and_cpu() {
        let workers = resolve_analysis_worker_count(5, 3);
        assert_eq!(workers, 3);
    }

    #[test]
    fn resolve_analysis_worker_count_respects_explicit_cap() {
        let workers = resolve_analysis_worker_count_with_cap(20, 24, Some(6));
        assert_eq!(workers, 6);
    }

    #[test]
    fn resolve_analysis_parallelism_budget_reserves_two_cores_when_possible() {
        assert_eq!(resolve_analysis_parallelism_budget(24), 22);
        assert_eq!(resolve_analysis_parallelism_budget(8), 6);
    }

    #[test]
    fn resolve_analysis_parallelism_budget_never_drops_below_one() {
        assert_eq!(resolve_analysis_parallelism_budget(2), 1);
        assert_eq!(resolve_analysis_parallelism_budget(1), 1);
        assert_eq!(resolve_analysis_parallelism_budget(0), 1);
    }

    #[test]
    fn resolve_analysis_parallelism_budget_respects_explicit_cap() {
        assert_eq!(resolve_analysis_parallelism_budget_with_cap(24, Some(5)), 5);
        assert_eq!(resolve_analysis_parallelism_budget_with_cap(4, Some(5)), 2);
    }

    #[test]
    fn resolve_analysis_bpm_range_defaults_to_70_180() {
        assert_eq!(resolve_analysis_bpm_range(None, None), (70, 180));
    }

    #[test]
    fn resolve_analysis_bpm_range_rejects_invalid_values() {
        assert_eq!(resolve_analysis_bpm_range(Some(180), Some(70)), (70, 180));
        assert_eq!(resolve_analysis_bpm_range(Some(0), Some(180)), (70, 180));
    }

    #[test]
    fn essentia_result_requires_bpm_or_key() {
        assert!(!essentia_result_has_detected_values(&EssentiaResult {
            ok: true,
            bpm: None,
            key: None,
            first_beat_ms: None,
        }));
        assert!(!essentia_result_has_detected_values(&EssentiaResult {
            ok: true,
            bpm: None,
            key: Some("   ".to_string()),
            first_beat_ms: Some(0.0),
        }));
        assert!(essentia_result_has_detected_values(&EssentiaResult {
            ok: true,
            bpm: Some(128.0),
            key: None,
            first_beat_ms: None,
        }));
        assert!(essentia_result_has_detected_values(&EssentiaResult {
            ok: true,
            bpm: None,
            key: Some("Am".to_string()),
            first_beat_ms: None,
        }));
    }

    #[test]
    fn build_waveform_preview_reflects_dynamic_range() {
        // Simulate a track with quiet intro, loud middle, quiet outro
        let sample_rate = 44100;
        let duration_sec = 10;
        let n = sample_rate * duration_sec;
        let mut samples = vec![0.0f32; n];

        // Quiet intro (0-2s): amplitude 0.05
        for i in 0..(2 * sample_rate) {
            let t = i as f32 / sample_rate as f32;
            samples[i] = (t * 440.0 * std::f32::consts::TAU).sin() * 0.05;
        }
        // Loud middle (2-8s): amplitude 0.8
        for i in (2 * sample_rate)..(8 * sample_rate) {
            let t = i as f32 / sample_rate as f32;
            samples[i] = (t * 440.0 * std::f32::consts::TAU).sin() * 0.8;
        }
        // Quiet outro (8-10s): amplitude 0.1
        for i in (8 * sample_rate)..(10 * sample_rate) {
            let t = i as f32 / sample_rate as f32;
            samples[i] = (t * 440.0 * std::f32::consts::TAU).sin() * 0.1;
        }

        let peaks = build_waveform_preview_from_samples(&samples, 100);
        assert_eq!(peaks.len(), 100);

        // Bins 0-19 = intro (quiet), bins 20-79 = middle (loud), bins 80-99 = outro (quiet)
        let intro_avg: f32 = peaks[0..20].iter().map(|&v| v as f32).sum::<f32>() / 20.0;
        let middle_avg: f32 = peaks[20..80].iter().map(|&v| v as f32).sum::<f32>() / 60.0;
        let outro_avg: f32 = peaks[80..100].iter().map(|&v| v as f32).sum::<f32>() / 20.0;

        assert!(
            middle_avg > intro_avg * 3.0,
            "middle ({middle_avg}) should be much louder than intro ({intro_avg})"
        );
        assert!(
            middle_avg > outro_avg * 3.0,
            "middle ({middle_avg}) should be much louder than outro ({outro_avg})"
        );
        assert!(
            peaks.iter().any(|&v| v > 80),
            "loudest section should have peaks near 100, got max={}",
            peaks.iter().max().unwrap()
        );
        assert!(
            intro_avg < 20.0,
            "quiet intro should have low peaks, got avg={intro_avg}"
        );
    }

    #[test]
    fn waveform_detail_entries_follow_cdj_detail_rate() {
        assert_eq!(waveform_detail_entries_for_duration(Some(30_000)), 4_504);
        assert_eq!(
            waveform_detail_bins_for_duration(Some(8_000)),
            WAVEFORM_PREVIEW_BINS,
            "short tracks still keep enough local bins for UI preview"
        );
        assert_eq!(waveform_detail_bins_for_duration(Some(180_000)), 27_004);
    }

    #[test]
    fn persisted_waveform_preview_is_downsampled_from_detail_cache() {
        let detail_bins = waveform_detail_bins_for_duration(Some(180_000));
        let waveform =
            WaveformData::from_peaks((0..detail_bins).map(|i| ((i * 37) % 101) as u8).collect());

        let preview = waveform_preview_if_persisted(&waveform, &Some("/tmp/ANLZ0000.DAT".into()))
            .expect("preview");

        assert_eq!(preview.len(), WAVEFORM_PREVIEW_BINS);
        assert!(preview.iter().any(|&v| v > 0));
    }

    #[test]
    fn waveform_data_bass_section_gets_low_band_treble_gets_high_band() {
        let sample_rate = 44100;
        let bins = 20;
        // 2 seconds total: first half is 100Hz bass, second half is 8kHz treble
        let n = sample_rate * 2;
        let mut samples = vec![0.0f32; n];
        for i in 0..n / 2 {
            let t = i as f32 / sample_rate as f32;
            samples[i] = (t * 100.0 * std::f32::consts::TAU).sin() * 0.8;
        }
        for i in n / 2..n {
            let t = i as f32 / sample_rate as f32;
            samples[i] = (t * 8000.0 * std::f32::consts::TAU).sin() * 0.8;
        }

        let data = build_waveform_data_from_samples(&samples, bins);
        assert_eq!(data.peaks.len(), bins);
        assert_eq!(data.bands.len(), bins);

        // First half bins (0-9) should have low frequency bands (0-2)
        let bass_bands: Vec<u8> = data.bands[0..bins / 2].to_vec();
        let treble_bands: Vec<u8> = data.bands[bins / 2..bins].to_vec();

        let bass_avg = bass_bands.iter().map(|&b| b as f32).sum::<f32>() / bass_bands.len() as f32;
        let treble_avg =
            treble_bands.iter().map(|&b| b as f32).sum::<f32>() / treble_bands.len() as f32;

        assert!(
            bass_avg < 2.0,
            "bass section should have low band values (0-1), got avg={bass_avg}, bands={bass_bands:?}"
        );
        assert!(
            treble_avg > 3.0,
            "treble section should have high band values (4-5), got avg={treble_avg}, bands={treble_bands:?}"
        );
    }

    #[test]
    fn waveform_data_peaks_and_bands_have_same_length() {
        let samples: Vec<f32> = (0..44100)
            .map(|i| (i as f32 / 44100.0 * 440.0 * std::f32::consts::TAU).sin() * 0.5)
            .collect();
        let data = build_waveform_data_from_samples(&samples, 150);
        assert_eq!(data.peaks.len(), 150);
        assert_eq!(data.bands.len(), 150);
        // All bands must be in 0-5
        assert!(data.bands.iter().all(|&b| b <= 5));
    }

    #[test]
    fn waveform_data_empty_input_returns_empty() {
        let data = build_waveform_data_from_samples(&[], 100);
        assert!(data.peaks.is_empty());
        assert!(data.bands.is_empty());

        let data = build_waveform_data_from_samples(&[0.5, 0.3], 0);
        assert!(data.peaks.is_empty());
        assert!(data.bands.is_empty());
    }

    #[test]
    fn collect_tracks_for_analysis_preserves_requested_order_and_dedupes() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        setup_tracks_table(&conn);
        insert_track(&conn, "t1", "Track 1");
        insert_track(&conn, "t2", "Track 2");
        insert_track(&conn, "t3", "Track 3");

        let requested = vec![
            "t3".to_string(),
            "t2".to_string(),
            "t3".to_string(),
            "missing".to_string(),
            "t1".to_string(),
            " ".to_string(),
        ];
        let rows = collect_tracks_for_analysis(&conn, &requested).expect("collect tracks");
        let ids = rows.into_iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(ids, vec!["t3", "t2", "t1"]);
    }

    #[test]
    fn collect_tracks_for_analysis_returns_empty_for_blank_requests() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        setup_tracks_table(&conn);
        insert_track(&conn, "t1", "Track 1");

        let requested = vec!["".to_string(), "   ".to_string()];
        let rows = collect_tracks_for_analysis(&conn, &requested).expect("collect tracks");
        assert!(rows.is_empty());
    }
}
