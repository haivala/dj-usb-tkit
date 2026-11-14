//! The one place analysis is run from. Whatever the user does, one coordinator
//! thread owns the database connection, the worker budget and what the user is
//! told. Workers only analyze: they send track pieces to the user and their
//! result back here, where it is saved and reported.
//!
//! All state is the coordinator's own, so nothing needs synchronizing. It lives
//! as long as there is work, blocked on its channel in between, and pause and
//! cancel die with it, so the next job always starts clean.

use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Instant;

use rusqlite::{Connection, params};
use uuid::Uuid;

use super::{
    AnalyzeTrackProgress, BpmDetectionParams, LibraryDurationContext, TRACK_ANALYSIS_UPDATE_SQL,
    analysis_capacity, analysis_debug_workers_enabled,
    analyze_track_with_usb_fallback_with_updates, build_done_progress_error,
    build_done_progress_success, build_partial_progress, build_started_progress,
    collect_tracks_for_analysis, elapsed_ms_since, has_memory_headroom_for_engine,
    incomplete_analysis_reason, resolve_analysis_bpm_range, resolve_analysis_engine,
    resolve_worker_cap_for_engine,
};
use crate::error::{BackendError, BackendResult};
use crate::i18n::{Text, tr};
use crate::logging::{self, Level};
use crate::models::{
    AnalysisSubmittedData, AnalyzeNewTracksData, AnalyzeNewTracksRequest, SetAnalysisPausedData,
    WarningEntry,
};
use crate::service::bpm_key::AnalysisEngine;
use crate::service::export_helpers::{LocalAnalysisResult, LocalTrackForAnalysis};
use crate::service::key_notation::{self, KeyNotation};
use crate::service::{BackendService, has_core_analysis_fields, now};

/// What a job tells whoever shows it to the user.
#[derive(Debug, Clone)]
pub enum AnalysisEvent {
    JobStarted {
        job_id: String,
        total: usize,
    },
    TracksWaiting {
        job_id: String,
        track_ids: Vec<String>,
        tooltip: String,
    },
    /// Cancelled before their turn came.
    TracksNotWaiting {
        job_id: String,
        track_ids: Vec<String>,
    },
    Track {
        job_id: String,
        progress: Box<AnalyzeTrackProgress>,
    },
    JobFinished(AnalysisJobSummary),
}

#[derive(Debug, Clone)]
pub struct AnalysisJobSummary {
    pub job_id: String,
    pub warnings: Vec<WarningEntry>,
    pub warning_count: usize,
    /// For the status line.
    pub message: String,
}

pub type AnalysisObserver = Arc<dyn Fn(&AnalysisEvent) + Send + Sync>;

/// The route to the coordinator, shared across `.clone()`s of the service.
#[derive(Default)]
pub(crate) struct AnalysisCoordinatorHandle {
    /// `Some` while a coordinator runs. Sending and ending both happen under
    /// this lock, so a message can't be left behind in a coordinator that quit.
    sender: Mutex<Option<mpsc::Sender<Message>>>,
    observer: Mutex<Option<AnalysisObserver>>,
}

impl fmt::Debug for AnalysisCoordinatorHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AnalysisCoordinatorHandle")
    }
}

impl AnalysisCoordinatorHandle {
    fn sender(&self) -> MutexGuard<'_, Option<mpsc::Sender<Message>>> {
        self.sender.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn observe(&self, event: &AnalysisEvent) {
        let observer = self
            .observer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(observer) = observer {
            observer(event);
        }
    }

    /// Does nothing when no coordinator runs: there is no job to affect.
    fn send(&self, message: Message) {
        if let Some(sender) = self.sender().as_ref() {
            let _ = sender.send(message);
        }
    }
}

enum Message {
    Submit {
        request: AnalyzeNewTracksRequest,
        listener: Option<mpsc::Sender<ListenerEvent>>,
        reply: mpsc::Sender<BackendResult<Submitted>>,
    },
    Pause(bool),
    Cancel,
    WorkerDone {
        item: QueuedTrack,
        result: BackendResult<LocalAnalysisResult>,
    },
}

/// For a caller waiting on its own request (`analyze_new_tracks_with_progress`).
/// The channel closes when the last of the request's tracks is done.
enum ListenerEvent {
    Progress(Box<AnalyzeTrackProgress>),
    Warning(WarningEntry),
    Cancelled,
}

struct Submitted {
    data: AnalysisSubmittedData,
    queued_ids: Vec<String>,
    skipped_ids: Vec<String>,
    warnings: Vec<WarningEntry>,
}

/// What a request's tracks share. Dropped with the last of them, which is what
/// closes the listener's channel.
struct RequestContext {
    engine: AnalysisEngine,
    bpm_min: u32,
    bpm_max: u32,
    notation: KeyNotation,
    listener: Option<mpsc::Sender<ListenerEvent>>,
}

impl RequestContext {
    fn tell_listener(&self, event: ListenerEvent) {
        if let Some(listener) = &self.listener {
            let _ = listener.send(event);
        }
    }

    fn emit(
        &self,
        handle: &AnalysisCoordinatorHandle,
        job_id: &str,
        mut progress: AnalyzeTrackProgress,
    ) {
        if progress.key.is_some() {
            (progress.key_display, progress.key_color) =
                key_notation::key_display_fields(progress.key.as_deref(), self.notation);
        }
        handle.observe(&AnalysisEvent::Track {
            job_id: job_id.to_string(),
            progress: Box::new(progress.clone()),
        });
        self.tell_listener(ListenerEvent::Progress(Box::new(progress)));
    }
}

struct QueuedTrack {
    track: LocalTrackForAnalysis,
    request: Arc<RequestContext>,
}

struct Coordinator {
    service: BackendService,
    /// Its own, outside the pool: analysis never waits on the rest of the app,
    /// nor the app on analysis.
    conn: Connection,
    tx: mpsc::Sender<Message>,
    waveform_dir: PathBuf,
    artwork_dir: PathBuf,
    job_id: String,
    started_at: Instant,
    job_started: bool,
    queue: VecDeque<QueuedTrack>,
    /// Waiting or in flight, by id: a track is analyzed by one thing at a time.
    active: HashSet<String>,
    in_flight: usize,
    /// Read once per engine: memory the running workers hold would otherwise
    /// count against it and wrongly shrink it.
    budgets: Vec<(AnalysisEngine, usize)>,
    paused: bool,
    total: usize,
    completed: usize,
    analyzed: usize,
    incomplete: usize,
    failed: usize,
    warnings: Vec<WarningEntry>,
    library: Option<LibraryDurationContext>,
    /// The total a cancel cut short, until new work comes in.
    cancelled_of: Option<usize>,
}

impl Coordinator {
    fn new(
        service: BackendService,
        conn: Connection,
        tx: mpsc::Sender<Message>,
    ) -> BackendResult<Self> {
        let analysis_dir = service.db.data_dir().join("analysis");
        let waveform_dir = analysis_dir.join("waveforms");
        let artwork_dir = analysis_dir.join("artwork");
        std::fs::create_dir_all(&waveform_dir)?;
        std::fs::create_dir_all(&artwork_dir)?;
        Ok(Self {
            service,
            conn,
            tx,
            waveform_dir,
            artwork_dir,
            job_id: format!("job-analysis-{}", Uuid::now_v7()),
            started_at: Instant::now(),
            job_started: false,
            queue: VecDeque::new(),
            active: HashSet::new(),
            in_flight: 0,
            budgets: Vec::new(),
            paused: false,
            total: 0,
            completed: 0,
            analyzed: 0,
            incomplete: 0,
            failed: 0,
            warnings: Vec::new(),
            library: None,
            cancelled_of: None,
        })
    }

    fn run(mut self, rx: mpsc::Receiver<Message>) {
        // Only ends through `next_or_finish`; it holds a sender for its workers.
        while let Ok(message) = rx.recv() {
            let mut next = Some(message);
            while let Some(message) = next {
                self.handle(message);
                self.dispatch();
                next = if self.is_drained() {
                    match self.next_or_finish(&rx) {
                        Some(message) => Some(message),
                        None => return,
                    }
                } else {
                    None
                };
            }
        }
    }

    fn is_drained(&self) -> bool {
        self.queue.is_empty() && self.in_flight == 0
    }

    /// With no work left: a message that came in meanwhile, or the end of the
    /// job. Under the lock, so a new request either got in here or starts a new
    /// coordinator after this job's end has been reported.
    fn next_or_finish(&mut self, rx: &mpsc::Receiver<Message>) -> Option<Message> {
        let handle = self.service.analysis_coordinator.clone();
        let mut sender = handle.sender();
        if let Ok(message) = rx.try_recv() {
            return Some(message);
        }
        *sender = None;
        self.finish_job();
        None
    }

    fn handle(&mut self, message: Message) {
        match message {
            Message::Submit {
                request,
                listener,
                reply,
            } => {
                let _ = reply.send(self.submit(request, listener));
            }
            Message::Pause(paused) => self.paused = paused,
            Message::Cancel => self.cancel(),
            Message::WorkerDone { item, result } => {
                self.in_flight -= 1;
                self.finish_track(item, result);
            }
        }
    }

    fn submit(
        &mut self,
        request: AnalyzeNewTracksRequest,
        listener: Option<mpsc::Sender<ListenerEvent>>,
    ) -> BackendResult<Submitted> {
        let engine = resolve_analysis_engine(&self.conn, request.analysis_engine.as_deref());
        let (bpm_min, bpm_max) = resolve_analysis_bpm_range(
            request.bpm_range.as_deref(),
            request.bpm_min,
            request.bpm_max,
        );
        let context = Arc::new(RequestContext {
            engine,
            bpm_min,
            bpm_max,
            notation: key_notation::key_notation_setting(&self.conn)?,
            listener,
        });

        let reanalyze = request.reanalyze && !request.track_ids.is_empty();
        let mut new_ids = HashSet::new();
        let mut new_tracks = Vec::new();
        let mut skipped_ids = Vec::new();
        for mut track in select_tracks(&self.service, &self.conn, &request)? {
            if self.active.contains(&track.id) {
                skipped_ids.push(track.id);
            } else if new_ids.insert(track.id.clone()) {
                if reanalyze {
                    track.force_reanalysis();
                }
                new_tracks.push(track);
            }
        }

        // A Reanalyze of a track still waiting takes effect when its turn comes.
        if reanalyze && !skipped_ids.is_empty() {
            let skipped: HashSet<&String> = skipped_ids.iter().collect();
            for item in self.queue.iter_mut() {
                if skipped.contains(&item.track.id) {
                    item.track.force_reanalysis();
                }
            }
        }

        let mut warnings = Vec::new();
        if new_tracks.is_empty() {
            // Tracks already being analyzed aren't a problem to report.
            if skipped_ids.is_empty() {
                warnings.push(logging::log_text(
                    Level::Info,
                    "analysis",
                    "analysis.no-eligible-tracks",
                    &tr::analysis_no_eligible_tracks(),
                ));
            }
        } else {
            let in_job: HashSet<&str> = self
                .active
                .iter()
                .chain(new_ids.iter())
                .map(String::as_str)
                .collect();
            self.library =
                self.service
                    .library_duration_context(&self.conn, &request.query, &in_job)?;
        }

        let message = submitted_message(new_tracks.len(), skipped_ids.len());
        let queued_ids: Vec<String> = new_tracks.iter().map(|t| t.id.clone()).collect();
        if !queued_ids.is_empty() {
            self.cancelled_of = None;
            self.total += queued_ids.len();
            if !self.job_started {
                self.job_started = true;
                self.started_at = Instant::now();
                self.emit(&AnalysisEvent::JobStarted {
                    job_id: self.job_id.clone(),
                    total: self.total,
                });
            }
            self.active.extend(new_ids);
            self.queue
                .extend(new_tracks.into_iter().map(|track| QueuedTrack {
                    track,
                    request: context.clone(),
                }));
            self.emit(&AnalysisEvent::TracksWaiting {
                job_id: self.job_id.clone(),
                track_ids: queued_ids.clone(),
                tooltip: tr::analysis_waiting_tooltip().render(),
            });
        }

        Ok(Submitted {
            data: AnalysisSubmittedData {
                job_id: self.job_id.clone(),
                queued: queued_ids.len(),
                skipped: skipped_ids.len(),
                message,
            },
            queued_ids,
            skipped_ids,
            warnings,
        })
    }

    fn budget(&mut self, engine: AnalysisEngine) -> usize {
        if let Some((_, budget)) = self.budgets.iter().find(|(e, _)| *e == engine) {
            return *budget;
        }
        let budget = analysis_capacity(engine);
        if analysis_debug_workers_enabled() {
            let cap = resolve_worker_cap_for_engine(engine);
            logging::debug(
                "analysis",
                format!(
                    "debug workers budget={budget} memory_cap={:?} env_cap={:?}",
                    cap.memory_cap, cap.env_cap
                ),
            );
        }
        self.budgets.push((engine, budget));
        budget
    }

    /// Starts what may start. Runs after every message: a freed slot, new work,
    /// a resume or a cancel is exactly when this can change.
    fn dispatch(&mut self) {
        while !self.paused {
            let Some(engine) = self.queue.front().map(|item| item.request.engine) else {
                return;
            };
            if self.in_flight >= self.budget(engine) {
                return;
            }
            let has_memory = has_memory_headroom_for_engine(engine);
            // Too little memory: wait for a running track to free some. With
            // none running nothing will, so the track fails instead.
            if !has_memory && self.in_flight > 0 {
                return;
            }
            let Some(item) = self.queue.pop_front() else {
                return;
            };
            item.request.emit(
                &self.service.analysis_coordinator,
                &self.job_id,
                build_started_progress(self.completed, self.total, &item.track),
            );
            if has_memory {
                self.in_flight += 1;
                self.spawn_worker(item);
            } else {
                self.finish_track(
                    item,
                    Err(BackendError::Internal(format!(
                        "Not enough available memory to start analysis right now (needs ~{} MB free). Wait for other analysis to finish and try again.",
                        super::analysis_bytes_per_worker(engine) / (1024 * 1024)
                    ))),
                );
            }
        }
    }

    fn spawn_worker(&self, item: QueuedTrack) {
        let tx = self.tx.clone();
        let handle = self.service.analysis_coordinator.clone();
        let job_id = self.job_id.clone();
        let (current, total) = (self.completed, self.total);
        let waveform_dir = self.waveform_dir.clone();
        let artwork_dir = self.artwork_dir.clone();
        std::thread::spawn(move || {
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                #[cfg(test)]
                assert!(
                    !item.track.title.contains("worker panics"),
                    "a worker panicked"
                );
                analyze_track_with_usb_fallback_with_updates(
                    &item.track,
                    &waveform_dir,
                    &artwork_dir,
                    BpmDetectionParams {
                        bpm_min: item.request.bpm_min,
                        bpm_max: item.request.bpm_max,
                        engine: item.request.engine,
                    },
                    |update| {
                        // Straight to the user: only the result goes through
                        // the coordinator, and it is sent after these.
                        item.request.emit(
                            &handle,
                            &job_id,
                            build_partial_progress(
                                current,
                                total,
                                item.track.id.clone(),
                                item.track.title.clone(),
                                item.track.file_path.clone(),
                                update,
                            ),
                        );
                    },
                )
            }))
            .unwrap_or_else(|payload| {
                Err(BackendError::Internal(format!(
                    "analysis worker panicked: {}",
                    panic_text(payload.as_ref())
                )))
            });
            let _ = tx.send(Message::WorkerDone { item, result });
        });
    }

    fn finish_track(&mut self, item: QueuedTrack, result: BackendResult<LocalAnalysisResult>) {
        let QueuedTrack { track, request } = item;
        self.active.remove(&track.id);
        self.completed += 1;
        let saved = result.and_then(|local| {
            save_result(&self.conn, &track.id, &local)?;
            Ok(local)
        });
        let mut warning = None;
        let mut progress = match saved {
            Ok(local) => {
                let ready = has_core_analysis_fields(
                    local.waveform_peaks_path.as_deref(),
                    local.bpm,
                    local.duration_ms,
                );
                if ready {
                    self.analyzed += 1;
                } else {
                    self.incomplete += 1;
                    warning = Some(logging::log_composed(
                        Level::Warn,
                        "analysis",
                        "analysis.track-incomplete",
                        |english| {
                            let r = crate::i18n::renderer(english);
                            r(&tr::analysis_track_incomplete(
                                &track.file_path,
                                incomplete_analysis_reason(&local, r),
                            ))
                        },
                    ));
                }
                if let Some(library) = self.library.as_mut()
                    && ready
                    && library.visible_track_ids.contains(&track.id)
                {
                    library.total_ms += local.duration_ms.unwrap_or(0);
                    library.known_count += 1;
                }
                build_done_progress_success(self.completed, self.total, &track, &local)
            }
            Err(err) => {
                self.failed += 1;
                let err_msg = err.render();
                warning = Some(logging::log_text(
                    Level::Error,
                    "analysis",
                    "analysis.track-failed",
                    &tr::analysis_track_failed(&track.file_path, &err),
                ));
                build_done_progress_error(self.completed, self.total, &track, err_msg)
            }
        };
        if let Some(warning) = warning {
            request.tell_listener(ListenerEvent::Warning(warning.clone()));
            self.warnings.push(warning);
        }
        progress.track =
            crate::service::library::track_rows(&self.conn, std::slice::from_ref(&track.id))
                .ok()
                .and_then(|mut rows| rows.pop());
        if let Some(library) = self.library.as_ref() {
            progress.library_total_duration_ms = Some(library.total_ms);
            progress.library_duration_unknown_count = Some(
                library
                    .visible_track_ids
                    .len()
                    .saturating_sub(library.known_count),
            );
        }
        request.emit(&self.service.analysis_coordinator, &self.job_id, progress);
    }

    /// Clears what hasn't started and resumes, so work asked for after this runs.
    fn cancel(&mut self) {
        self.paused = false;
        if self.queue.is_empty() {
            return;
        }
        self.cancelled_of.get_or_insert(self.total);
        let mut track_ids = Vec::with_capacity(self.queue.len());
        for item in self.queue.drain(..) {
            self.active.remove(&item.track.id);
            item.request.tell_listener(ListenerEvent::Cancelled);
            track_ids.push(item.track.id);
        }
        self.total = self.total.saturating_sub(track_ids.len());
        self.emit(&AnalysisEvent::TracksNotWaiting {
            job_id: self.job_id.clone(),
            track_ids,
        });
    }

    fn finish_job(&mut self) {
        if !self.job_started {
            return;
        }
        if let Some(total) = self.cancelled_of {
            self.warnings
                .push(super::analysis_cancelled_warning(self.completed, total));
        }
        let warning_count = logging::count_warnings(&self.warnings);
        let elapsed_ms = elapsed_ms_since(self.started_at);
        let (analyzed, failed) = (self.analyzed, self.failed);
        // The job's summary for the Event Log, logged here so the file gets it in English.
        if self.completed > 0 {
            logging::emit_text(
                Level::Info,
                "analysis",
                &log_text(
                    self.completed,
                    &crate::utils::duration_text(elapsed_ms),
                    analyzed,
                    failed,
                    self.incomplete,
                ),
            );
        }
        let summary = AnalysisEvent::JobFinished(AnalysisJobSummary {
            job_id: self.job_id.clone(),
            warnings: std::mem::take(&mut self.warnings),
            warning_count,
            message: finished_text(analyzed, failed, self.incomplete),
        });
        self.emit(&summary);
    }

    fn emit(&self, event: &AnalysisEvent) {
        self.service.analysis_coordinator.observe(event);
    }
}

fn select_tracks(
    service: &BackendService,
    conn: &Connection,
    request: &AnalyzeNewTracksRequest,
) -> BackendResult<Vec<LocalTrackForAnalysis>> {
    // In auto mode the caller can scope "tracks needing analysis" to some
    // playlists or to the current library filter instead of the whole DB. We
    // resolve the scoped id list here and feed it through the explicit-id path
    // of `collect_tracks_for_analysis`.
    let playlist_ids: Vec<&str> = request
        .playlist_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .collect();
    let scoped_ids = if !request.track_ids.is_empty() {
        None
    } else if !playlist_ids.is_empty() {
        Some(service.collect_playlists_unanalyzed_track_ids(conn, &playlist_ids, &request.query)?)
    } else if request.scope_to_library_filter {
        Some(service.library_filter_unanalyzed_track_ids(conn, &request.query)?)
    } else {
        None
    };
    match scoped_ids {
        // A scoped request with nothing to do must NOT fall through to
        // `collect_tracks_for_analysis(&[])`, which would treat an empty slice
        // as "auto-select the whole DB".
        Some(ids) if ids.is_empty() => Ok(Vec::new()),
        Some(ids) => collect_tracks_for_analysis(conn, &ids),
        None => collect_tracks_for_analysis(conn, &request.track_ids),
    }
}

fn save_result(
    conn: &Connection,
    track_id: &str,
    local: &LocalAnalysisResult,
) -> BackendResult<()> {
    conn.prepare_cached(TRACK_ANALYSIS_UPDATE_SQL)?
        .execute(params![
            local.bpm,
            local.key,
            local.duration_ms,
            local.artwork_path,
            local.waveform_peaks_path,
            now(),
            track_id,
            local.bpm_analyzer,
            // Only a first beat this analysis placed is written; a kept one
            // stays as it is, with its source.
            local
                .first_beat_ms
                .filter(|_| local.first_beat_estimated)
                .map(|v| v as i64),
        ])?;
    Ok(())
}

fn submitted_message(queued: usize, skipped: usize) -> String {
    match (queued, skipped) {
        (0, 0) => tr::analysis_submitted_none(),
        (0, skipped) => tr::analysis_submitted_already(skipped),
        (queued, 0) => tr::analysis_submitted(queued),
        (queued, skipped) => tr::analysis_submitted_some_already(queued, skipped),
    }
    .render()
}

/// The Event Log's line for a finished job.
fn log_text(
    tracks: usize,
    elapsed: &str,
    analyzed: usize,
    failed: usize,
    incomplete: usize,
) -> Text {
    if incomplete > 0 {
        tr::analysis_log_incomplete(tracks, elapsed, analyzed, failed, incomplete)
    } else {
        tr::analysis_log(tracks, elapsed, analyzed, failed)
    }
}

/// The status line; the job event's `warning_text` follows it as a link to the Event Log.
fn finished_text(analyzed: usize, failed: usize, incomplete: usize) -> String {
    if incomplete > 0 {
        tr::analysis_finished_incomplete(analyzed, failed, incomplete)
    } else {
        tr::analysis_finished(analyzed, failed)
    }
    .render()
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "unknown panic".to_string()
    }
}

impl BackendService {
    /// Who is told about analysis jobs. Replaces any earlier one.
    pub fn set_analysis_observer(&self, observer: AnalysisObserver) {
        *self
            .analysis_coordinator
            .observer
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(observer);
    }

    /// Adds a request's tracks to the analysis job, starting one if none runs,
    /// and returns at once. Everything after that comes through the observer.
    pub fn submit_analysis(
        &self,
        request: AnalyzeNewTracksRequest,
    ) -> BackendResult<AnalysisSubmittedData> {
        Ok(self.submit_request(request, None)?.data)
    }

    fn submit_request(
        &self,
        request: AnalyzeNewTracksRequest,
        listener: Option<mpsc::Sender<ListenerEvent>>,
    ) -> BackendResult<Submitted> {
        self.ensure_no_data_migration()?;
        let (reply, replied) = mpsc::channel();
        let mut message = Message::Submit {
            request,
            listener,
            reply,
        };
        {
            let mut sender = self.analysis_coordinator.sender();
            if let Some(running) = sender.as_ref() {
                match running.send(message) {
                    Ok(()) => return wait_for_reply(sender, replied),
                    // The coordinator is gone (it panicked): start another.
                    Err(mpsc::SendError(unsent)) => message = unsent,
                }
            }
            let conn = self.db.open_dedicated()?;
            let (tx, rx) = mpsc::channel();
            let coordinator = Coordinator::new(self.clone(), conn, tx.clone())?;
            tx.send(message)
                .map_err(|_| BackendError::Internal("analysis channel closed".to_string()))?;
            std::thread::Builder::new()
                .name("analysis-coordinator".to_string())
                .spawn(move || coordinator.run(rx))?;
            *sender = Some(tx);
            wait_for_reply(sender, replied)
        }
    }

    pub fn set_analysis_paused(&self, paused: bool) -> BackendResult<SetAnalysisPausedData> {
        self.analysis_coordinator.send(Message::Pause(paused));
        Ok(SetAnalysisPausedData { paused })
    }

    pub fn cancel_analysis(&self) -> BackendResult<()> {
        self.analysis_coordinator.send(Message::Cancel);
        Ok(())
    }

    pub fn analyze_new_tracks(
        &self,
        req: AnalyzeNewTracksRequest,
    ) -> BackendResult<AnalyzeNewTracksData> {
        self.analyze_new_tracks_with_progress(req, |_| {})
    }

    /// Submits a request and waits for its own tracks, reporting them to
    /// `on_progress` on this thread, for callers with no job to show.
    pub fn analyze_new_tracks_with_progress<F>(
        &self,
        req: AnalyzeNewTracksRequest,
        mut on_progress: F,
    ) -> BackendResult<AnalyzeNewTracksData>
    where
        F: FnMut(&AnalyzeTrackProgress),
    {
        let (listener, events) = mpsc::channel();
        let Submitted {
            data: _,
            queued_ids,
            skipped_ids,
            mut warnings,
        } = self.submit_request(req, Some(listener))?;

        let (mut analyzed, mut incomplete, mut failed) = (0usize, 0usize, 0usize);
        let mut cancelled = false;
        for event in events {
            match event {
                ListenerEvent::Progress(progress) => {
                    if progress.track_ready {
                        if progress.failed {
                            failed += 1;
                        } else if progress.analysis_ready {
                            analyzed += 1;
                        } else {
                            incomplete += 1;
                        }
                    }
                    on_progress(&progress);
                }
                ListenerEvent::Warning(warning) => warnings.push(warning),
                ListenerEvent::Cancelled => cancelled = true,
            }
        }
        let processed = analyzed + incomplete + failed;
        if cancelled {
            warnings.push(super::analysis_cancelled_warning(
                processed,
                queued_ids.len(),
            ));
        }
        Ok(AnalyzeNewTracksData {
            analyzed,
            incomplete,
            failed,
            processed,
            warnings,
            already_analyzing_track_ids: skipped_ids,
        })
    }
}

/// Lets go of the lock first: the coordinator may need it to finish.
fn wait_for_reply(
    sender: MutexGuard<'_, Option<mpsc::Sender<Message>>>,
    replied: mpsc::Receiver<BackendResult<Submitted>>,
) -> BackendResult<Submitted> {
    drop(sender);
    replied
        .recv()
        .map_err(|_| BackendError::Internal("analysis coordinator stopped".to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn a_worker_panic_fails_only_its_track_and_the_job_finishes() {
        let dir = tempdir().expect("tempdir");
        let service = BackendService::new(dir.path()).expect("service");
        service
            .db
            .connect()
            .expect("connect")
            .execute_batch(
                r#"
                INSERT INTO tracks (id, title, artist, file_path, created_at, updated_at) VALUES
                  ('panics', 'worker panics', 'A', '/missing/a.wav', '1', '1'),
                  ('fails', 'missing file', 'A', '/missing/b.wav', '1', '1');
                "#,
            )
            .expect("seed tracks");

        let data = service
            .analyze_new_tracks(AnalyzeNewTracksRequest {
                track_ids: vec!["panics".to_string(), "fails".to_string()],
                ..Default::default()
            })
            .expect("the request returns");

        assert_eq!(data.failed, 2);
        assert!(
            data.warnings
                .iter()
                .any(|w| w.message.contains("analysis worker panicked")),
            "{:?}",
            data.warnings
        );
    }

    #[test]
    fn submitted_texts() {
        assert_eq!(submitted_message(0, 0), "No tracks need analysis");
        assert_eq!(submitted_message(0, 1), "Already analyzing 1 track");
        assert_eq!(
            submitted_message(2, 1),
            "Analyzing 2 tracks, 1 already in progress"
        );
        assert_eq!(
            log_text(3, "0:12", 2, 0, 1).render(),
            "Analysis: 3 tracks in 0:12 (analyzed 2, failed 0, incomplete 1)"
        );
        assert_eq!(
            finished_text(0, 1, 0),
            "Analysis finished: analyzed 0, failed 1"
        );
        assert_eq!(
            finished_text(2, 0, 1),
            "Analysis finished: analyzed 2, failed 0, incomplete 1"
        );
    }
}
