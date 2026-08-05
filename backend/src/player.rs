use std::fs::File;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rodio::cpal::default_host;
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{OutputStream, Sink, Source};

use crate::error::{BackendError, BackendResult};
use crate::models::{PlaybackPreflightData, PlaybackStatusData};

const PLAYBACK_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const NATURAL_STOP_CHECK_INTERVAL: Duration = Duration::from_millis(250);

/// A natural end-of-track notification: the loaded sink emptied on its own, without an
/// explicit `Stop` command. Detected by the same serialized worker thread that processes
/// explicit Play/Stop commands (see `check_natural_stop`), so it can never race a stop/play
/// the way a separate polling thread could.
#[derive(Debug, Clone)]
pub struct PlaybackTransition {
    pub path: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct PlaybackController {
    tx: mpsc::Sender<PlaybackCommand>,
}

impl PlaybackController {
    pub fn new() -> (Self, mpsc::Receiver<PlaybackTransition>) {
        let (tx, rx) = mpsc::channel::<PlaybackCommand>();
        let (transition_tx, transition_rx) = mpsc::channel::<PlaybackTransition>();
        thread::spawn(move || playback_worker(rx, transition_tx));
        (Self { tx }, transition_rx)
    }

    pub fn play_path(
        &self,
        path: &str,
        start_offset_ms: Option<u64>,
        start_ratio: Option<f64>,
    ) -> BackendResult<PlaybackStatusData> {
        self.send_command(
            |reply_tx| PlaybackCommand::Play {
                path: path.to_string(),
                start_offset_ms,
                start_ratio,
                reply_tx,
            },
            "starting playback",
        )
    }

    pub fn stop(&self) -> BackendResult<PlaybackStatusData> {
        self.send_command(
            |reply_tx| PlaybackCommand::Stop { reply_tx },
            "stopping playback",
        )
    }

    pub fn status(&self) -> BackendResult<PlaybackStatusData> {
        self.send_command(
            |reply_tx| PlaybackCommand::Status { reply_tx },
            "reading playback status",
        )
    }

    fn send_command(
        &self,
        build: impl FnOnce(mpsc::Sender<BackendResult<PlaybackStatusData>>) -> PlaybackCommand,
        action: &str,
    ) -> BackendResult<PlaybackStatusData> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(build(reply_tx))
            .map_err(|err| BackendError::Internal(format!("playback worker unavailable: {err}")))?;
        reply_rx
            .recv_timeout(PLAYBACK_COMMAND_TIMEOUT)
            .map_err(|err| {
                BackendError::Internal(format!(
                    "playback worker timed out after {}s while {action}: {err}",
                    PLAYBACK_COMMAND_TIMEOUT.as_secs()
                ))
            })?
    }
}

#[derive(Debug)]
enum PlaybackCommand {
    Play {
        path: String,
        start_offset_ms: Option<u64>,
        start_ratio: Option<f64>,
        reply_tx: mpsc::Sender<BackendResult<PlaybackStatusData>>,
    },
    Stop {
        reply_tx: mpsc::Sender<BackendResult<PlaybackStatusData>>,
    },
    Status {
        reply_tx: mpsc::Sender<BackendResult<PlaybackStatusData>>,
    },
}

#[derive(Default)]
struct WorkerState {
    stream: Option<OutputStream>,
    stream_handle: Option<rodio::OutputStreamHandle>,
    sink: Option<Sink>,
    path: Option<String>,
    started_at: Option<Instant>,
    start_offset_ms: u64,
    duration_ms: Option<u64>,
}

fn playback_worker(rx: mpsc::Receiver<PlaybackCommand>, transitions: mpsc::Sender<PlaybackTransition>) {
    let mut state = WorkerState::default();

    loop {
        let command = if state.sink.is_some() {
            match rx.recv_timeout(NATURAL_STOP_CHECK_INTERVAL) {
                Ok(command) => command,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    check_natural_stop(&mut state, &transitions);
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(command) => command,
                Err(_) => break,
            }
        };

        match command {
            PlaybackCommand::Play {
                path,
                start_offset_ms,
                start_ratio,
                reply_tx,
            } => {
                let result = play_in_worker(&mut state, &path, start_offset_ms, start_ratio);
                let _ = reply_tx.send(result);
            }
            PlaybackCommand::Stop { reply_tx } => {
                stop_in_worker(&mut state);
                let _ = reply_tx.send(Ok(snapshot(&mut state)));
            }
            PlaybackCommand::Status { reply_tx } => {
                let _ = reply_tx.send(Ok(snapshot(&mut state)));
            }
        }
    }
}

/// Checks whether the loaded sink emptied on its own (natural end of track) and, if so,
/// tears it down (mirroring an explicit stop) and notifies the transition channel once.
fn check_natural_stop(state: &mut WorkerState, transitions: &mpsc::Sender<PlaybackTransition>) {
    let sink_emptied = state.sink.as_ref().is_some_and(|sink| sink.empty());
    if !sink_emptied {
        return;
    }
    let path = state.path.clone();
    let duration_ms = state.duration_ms;
    stop_in_worker(state);
    let _ = transitions.send(PlaybackTransition { path, duration_ms });
}

fn play_in_worker(
    state: &mut WorkerState,
    path: &str,
    start_offset_ms: Option<u64>,
    start_ratio: Option<f64>,
) -> BackendResult<PlaybackStatusData> {
    let normalized = normalize_and_validate_path(path)?;
    let same_track_loaded = state.path.as_deref() == Some(normalized.as_str());
    if same_track_loaded
        && let Some(sink) = state.sink.as_ref()
        && !sink.empty()
    {
        let offset_ms = compute_target_offset_ms(start_offset_ms, start_ratio, state.duration_ms);
        if sink.try_seek(Duration::from_millis(offset_ms)).is_ok() {
            sink.play();
            state.started_at = Some(Instant::now());
            state.start_offset_ms = offset_ms;
            return Ok(snapshot(state));
        }
    }

    if state.stream.is_none() || state.stream_handle.is_none() {
        let (stream, stream_handle) = open_output_stream()?;
        state.stream = Some(stream);
        state.stream_handle = Some(stream_handle);
    }
    let Some(stream_handle) = state.stream_handle.as_ref() else {
        return Err(BackendError::Internal(
            "audio output handle unavailable after initialization".to_string(),
        ));
    };
    let sink = Sink::try_new(stream_handle)
        .map_err(|err| BackendError::Internal(format!("failed to create audio sink: {err}")))?;

    let mut decoder = crate::symphonia_decoder::SeekableSymphoniaSource::open(Path::new(&normalized))
        .map_err(|err| BackendError::Internal(format!("decoder error: {err}")))?;

    let duration_ms = decoder
        .total_duration()
        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64);
    let offset_ms = compute_target_offset_ms(start_offset_ms, start_ratio, duration_ms);
    if offset_ms > 0 && decoder.try_seek(Duration::from_millis(offset_ms)).is_err() {
        // Falls back only for a source whose format genuinely has no seek table.
        sink.append(decoder.skip_duration(Duration::from_millis(offset_ms)));
    } else {
        sink.append(decoder);
    }
    sink.play();
    Ok(load_playback_state(
        state,
        sink,
        normalized,
        offset_ms,
        duration_ms,
    ))
}

fn load_playback_state(
    state: &mut WorkerState,
    sink: Sink,
    normalized_path: String,
    offset_ms: u64,
    duration_ms: Option<u64>,
) -> PlaybackStatusData {
    stop_in_worker(state);
    state.sink = Some(sink);
    state.path = Some(normalized_path);
    state.started_at = Some(Instant::now());
    state.start_offset_ms = offset_ms;
    state.duration_ms = duration_ms;
    snapshot(state)
}

fn compute_target_offset_ms(
    start_offset_ms: Option<u64>,
    start_ratio: Option<f64>,
    duration_ms: Option<u64>,
) -> u64 {
    let ratio = start_ratio.unwrap_or(0.0).clamp(0.0, 1.0);
    let mut offset_ms = start_offset_ms.unwrap_or(0);
    if offset_ms == 0
        && let Some(total_ms) = duration_ms
    {
        offset_ms = ((total_ms as f64) * ratio).round() as u64;
    }
    if let Some(total_ms) = duration_ms {
        offset_ms = offset_ms.min(total_ms);
    }
    offset_ms
}

pub fn run_playback_preflight(path: &str) -> BackendResult<PlaybackPreflightData> {
    let normalized = normalize_and_validate_path(path)?;
    let file_exists = Path::new(&normalized).exists();
    let file_readable = File::open(&normalized).is_ok();
    let file_decodable = if file_readable {
        crate::symphonia_decoder::SeekableSymphoniaSource::open(Path::new(&normalized)).is_ok()
    } else {
        false
    };

    let safe_output_devices = list_safe_output_device_names()?;
    let ready = file_exists && file_readable && file_decodable && !safe_output_devices.is_empty();

    let message = if !file_exists {
        "Audio file does not exist".to_string()
    } else if !file_readable {
        "Audio file is not readable".to_string()
    } else if !file_decodable {
        "Audio file is not decodable by playback engine".to_string()
    } else if safe_output_devices.is_empty() {
        "No usable output devices found. Ensure system audio output is available.".to_string()
    } else {
        format!(
            "Ready. Using {} safe output device candidate(s).",
            safe_output_devices.len()
        )
    };

    Ok(PlaybackPreflightData {
        path: normalized,
        file_exists,
        file_readable,
        safe_output_devices,
        ready,
        message,
    })
}

fn open_output_stream() -> BackendResult<(OutputStream, rodio::OutputStreamHandle)> {
    let _alsa_error_silencer = AlsaErrorSilencer::new();
    let _stderr_probe_silencer = StderrProbeSilencer::new();
    OutputStream::try_default().map_err(|err| {
        BackendError::Internal(format!(
            "audio output unavailable via system default device: {err}"
        ))
    })
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
))]
struct StderrProbeSilencer {
    old_stderr_fd: std::os::raw::c_int,
    _devnull: Option<File>,
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
))]
impl StderrProbeSilencer {
    fn new() -> Self {
        use std::os::fd::AsRawFd;

        let Ok(devnull) = File::options().read(true).write(true).open("/dev/null") else {
            return Self {
                old_stderr_fd: -1,
                _devnull: None,
            };
        };

        // SAFETY: libc fd operations are used with valid fds; failures are handled.
        let old_stderr_fd = unsafe { libc::dup(libc::STDERR_FILENO) };
        if old_stderr_fd >= 0 {
            // SAFETY: dup2 redirects STDERR to /dev/null for probe lifetime.
            let _ = unsafe { libc::dup2(devnull.as_raw_fd(), libc::STDERR_FILENO) };
        }

        Self {
            old_stderr_fd,
            _devnull: Some(devnull),
        }
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
))]
impl Drop for StderrProbeSilencer {
    fn drop(&mut self) {
        if self.old_stderr_fd >= 0 {
            // SAFETY: restore previously dup'd stderr fd, then close duplicate.
            unsafe {
                let _ = libc::dup2(self.old_stderr_fd, libc::STDERR_FILENO);
                let _ = libc::close(self.old_stderr_fd);
            }
        }
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
)))]
struct StderrProbeSilencer;

#[cfg(not(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
)))]
impl StderrProbeSilencer {
    fn new() -> Self {
        Self
    }
}

fn list_safe_output_device_names() -> BackendResult<Vec<String>> {
    let host = default_host();
    let devices_iter = host.output_devices().map_err(|err| {
        BackendError::Internal(format!("failed to enumerate output devices: {err}"))
    })?;

    let mut safe_devices = Vec::<String>::new();
    for device in devices_iter {
        let name = device
            .name()
            .unwrap_or_else(|_| "unknown-output-device".to_string());
        if is_blocked_device_name(&name) {
            continue;
        }
        safe_devices.push(name);
    }
    Ok(safe_devices)
}

fn is_blocked_device_name(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    lowered.contains("jack")
        || lowered.contains("oss")
        || lowered.contains("dmix")
        || lowered.contains("default")
        || lowered.contains("sysdefault")
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
))]
struct AlsaErrorSilencer {
    previous: alsa_sys::snd_local_error_handler_t,
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
))]
impl AlsaErrorSilencer {
    fn new() -> Self {
        // Silence ALSA plugin probe noise (jack/oss/dmix fallbacks) for this thread.
        let previous = unsafe { alsa_sys::snd_lib_error_set_local(Some(alsa_noop_error_handler)) };
        Self { previous }
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
))]
impl Drop for AlsaErrorSilencer {
    fn drop(&mut self) {
        unsafe {
            alsa_sys::snd_lib_error_set_local(self.previous);
        }
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
))]
unsafe extern "C" fn alsa_noop_error_handler(
    _file: *const std::os::raw::c_char,
    _line: std::os::raw::c_int,
    _func: *const std::os::raw::c_char,
    _err: std::os::raw::c_int,
    _fmt: *const std::os::raw::c_char,
    _arg: *mut alsa_sys::__va_list_tag,
) {
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
)))]
struct AlsaErrorSilencer;

#[cfg(not(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
)))]
impl AlsaErrorSilencer {
    fn new() -> Self {
        Self
    }
}

fn stop_in_worker(state: &mut WorkerState) {
    if let Some(sink) = state.sink.take() {
        sink.stop();
    }
    state.started_at = None;
    state.start_offset_ms = 0;
    state.duration_ms = None;
}

fn snapshot(state: &mut WorkerState) -> PlaybackStatusData {
    let playing = state.sink.as_ref().is_some_and(|sink| !sink.empty());
    if !playing {
        state.started_at = None;
    }

    let elapsed_ms = state
        .started_at
        .map(|s| s.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0);
    let mut position_ms = state.start_offset_ms.saturating_add(elapsed_ms);
    if let Some(total) = state.duration_ms {
        position_ms = position_ms.min(total);
    }

    PlaybackStatusData {
        path: state.path.clone(),
        playing,
        position_ms,
        duration_ms: state.duration_ms,
    }
}

fn normalize_and_validate_path(path: &str) -> BackendResult<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(BackendError::Validation(
            "path must be a non-empty filesystem path".to_string(),
        ));
    }

    let as_path = Path::new(trimmed);
    if !as_path.exists() {
        return Err(BackendError::NotFound(format!(
            "audio file not found: {trimmed}"
        )));
    }
    if !as_path.is_file() {
        return Err(BackendError::Validation(format!(
            "audio path is not a file: {trimmed}"
        )));
    }

    Ok(as_path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flac_fixture_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio/formats/track_format_flac.flac")
    }

    fn mp3_fixture_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio/noart/track_no_art.mp3")
    }

    #[test]
    fn seekable_symphonia_source_decodes_flac_fixture_and_supports_real_seek() {
        // This is the actual point of bypassing rodio::Decoder: its private ReadSeekSource
        // always reports byte_len() == None, and FLAC's demuxer unconditionally needs a real
        // byte length to seek at all (regardless of SeekMode) — so through rodio's own
        // decoder, FLAC seeking always fails with SeekError::Unseekable. Our own
        // FileMediaSource reports a real length, so this now genuinely works.
        let mut decoder = crate::symphonia_decoder::SeekableSymphoniaSource::open(
            &flac_fixture_path(),
        )
        .expect("should decode flac fixture");
        let duration = decoder
            .total_duration()
            .expect("flac fixture should report a duration");
        assert!(duration.as_millis() > 0, "duration should be positive");

        decoder
            .try_seek(duration / 2)
            .expect("flac fixture should support real seeking with a known byte length");
    }

    #[test]
    fn seekable_symphonia_source_decodes_mp3_fixture_and_supports_real_seek() {
        let mut decoder = crate::symphonia_decoder::SeekableSymphoniaSource::open(
            &mp3_fixture_path(),
        )
        .expect("should decode mp3 fixture");
        let duration = decoder
            .total_duration()
            .expect("mp3 fixture should report a duration");
        assert!(duration.as_millis() > 1000, "fixture should be more than a second long");

        decoder
            .try_seek(duration / 2)
            .expect("mp3 fixture should support seeking");
    }

    #[test]
    fn run_playback_preflight_reports_flac_fixture_as_decodable() {
        let preflight = run_playback_preflight(flac_fixture_path().to_str().unwrap())
            .expect("preflight should succeed for a readable fixture");
        assert!(preflight.file_exists);
        assert!(preflight.file_readable);
        // Device availability varies by test environment; decodability specifically is what
        // this test guards (the field this branch used to gate on `decode_audio_pcm_symphonia`,
        // now removed, still needs to report the fixture as decodable via rodio alone).
        assert_ne!(
            preflight.message, "Audio file is not decodable by playback engine",
            "flac fixture should be reported as decodable"
        );
    }

    #[test]
    fn natural_stop_detected_when_sink_has_no_queued_audio() {
        let (sink, _unused_output) = Sink::new_idle();
        // sink.empty() == true immediately — nothing was ever appended.
        let mut state = WorkerState {
            sink: Some(sink),
            path: Some("fake/track.mp3".to_string()),
            duration_ms: Some(1234),
            started_at: Some(Instant::now()),
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();

        check_natural_stop(&mut state, &tx);

        let transition = rx.try_recv().expect("expected a natural-stop transition");
        assert_eq!(transition.path.as_deref(), Some("fake/track.mp3"));
        assert_eq!(transition.duration_ms, Some(1234));
        assert!(
            state.sink.is_none(),
            "sink should be torn down, mirroring an explicit stop"
        );
        assert!(state.started_at.is_none());
    }

    #[test]
    fn natural_stop_not_detected_while_sink_still_has_queued_audio() {
        let (sink, _unused_output) = Sink::new_idle();
        // Bumps len() > 0; the infinite Zero source is never polled/drained in this test.
        sink.append(rodio::source::Zero::<f32>::new(1, 44_100));
        let mut state = WorkerState {
            sink: Some(sink),
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();

        check_natural_stop(&mut state, &tx);

        assert!(
            rx.try_recv().is_err(),
            "no transition should fire while audio is still queued"
        );
        assert!(state.sink.is_some(), "sink should be left untouched");
    }

    #[test]
    fn compute_target_offset_ms_prefers_explicit_offset_over_ratio() {
        assert_eq!(compute_target_offset_ms(Some(5_000), Some(0.5), Some(10_000)), 5_000);
    }

    #[test]
    fn compute_target_offset_ms_falls_back_to_ratio_when_offset_is_zero() {
        assert_eq!(compute_target_offset_ms(None, Some(0.25), Some(8_000)), 2_000);
    }

    #[test]
    fn compute_target_offset_ms_clamps_ratio_outside_unit_range() {
        assert_eq!(compute_target_offset_ms(None, Some(1.5), Some(4_000)), 4_000);
    }

    #[test]
    fn compute_target_offset_ms_clamps_explicit_offset_to_duration() {
        assert_eq!(compute_target_offset_ms(Some(9_000), None, Some(4_000)), 4_000);
    }

    #[test]
    fn compute_target_offset_ms_defaults_to_zero_without_duration_or_offset() {
        assert_eq!(compute_target_offset_ms(None, None, None), 0);
    }

    #[test]
    fn normalize_and_validate_path_rejects_empty_path() {
        let err = normalize_and_validate_path("   ").unwrap_err();
        assert!(matches!(err, BackendError::Validation(_)));
    }

    #[test]
    fn normalize_and_validate_path_rejects_missing_file() {
        let err = normalize_and_validate_path("/nonexistent/path/to/track.mp3").unwrap_err();
        assert!(matches!(err, BackendError::NotFound(_)));
    }

    #[test]
    fn normalize_and_validate_path_rejects_directory() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio");
        let err = normalize_and_validate_path(dir.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, BackendError::Validation(_)));
    }

    #[test]
    fn normalize_and_validate_path_accepts_existing_file() {
        let normalized = normalize_and_validate_path(mp3_fixture_path().to_str().unwrap())
            .expect("existing file should normalize successfully");
        assert!(normalized.ends_with("track_no_art.mp3"));
    }

    #[test]
    fn snapshot_reports_idle_state_when_no_sink_loaded() {
        let mut state = WorkerState::default();
        let status = snapshot(&mut state);
        assert!(!status.playing);
        assert_eq!(status.position_ms, 0);
        assert_eq!(status.path, None);
        assert_eq!(status.duration_ms, None);
    }

    #[test]
    fn snapshot_clamps_position_to_duration_and_reports_playing() {
        let (sink, _unused_output) = Sink::new_idle();
        // Bumps len() > 0 so `snapshot` sees the sink as playing without actually
        // decoding audio.
        sink.append(rodio::source::Zero::<f32>::new(1, 44_100));
        let mut state = WorkerState {
            sink: Some(sink),
            path: Some("fake/track.mp3".to_string()),
            duration_ms: Some(10),
            // Comfortably larger than any scheduling jitter, so elapsed time will
            // exceed duration_ms without risking Instant subtraction underflow.
            started_at: Some(Instant::now() - Duration::from_millis(500)),
            ..Default::default()
        };

        let status = snapshot(&mut state);

        assert!(status.playing);
        assert_eq!(status.position_ms, 10, "position should clamp to duration_ms");
    }

    #[test]
    fn snapshot_clears_started_at_when_not_playing() {
        let mut state = WorkerState {
            started_at: Some(Instant::now()),
            ..Default::default()
        };
        snapshot(&mut state);
        assert!(state.started_at.is_none());
    }

    #[test]
    fn is_blocked_device_name_matches_known_problematic_names_case_insensitively() {
        for name in [
            "JACK Audio Connection Kit",
            "surround51:CARD=default",
            "OSS Loopback",
            "dmix:CARD=PCH",
        ] {
            assert!(is_blocked_device_name(name), "{name} should be blocked");
        }
        assert!(!is_blocked_device_name("USB Audio Device"));
    }

    #[test]
    fn run_playback_preflight_reports_non_audio_file_as_not_decodable() {
        let manifest_toml = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let preflight = run_playback_preflight(manifest_toml.to_str().unwrap())
            .expect("preflight should succeed even for a non-audio file");
        assert!(preflight.file_exists);
        assert!(preflight.file_readable);
        assert!(!preflight.ready);
        assert_eq!(
            preflight.message,
            "Audio file is not decodable by playback engine"
        );
    }

    #[test]
    fn open_output_stream_does_not_panic_regardless_of_hardware_availability() {
        // No audio device is guaranteed in CI/sandboxed environments; this only
        // asserts that the ALSA/stderr noise-silencing wrappers unwind cleanly
        // either way (Ok or Err).
        let _ = open_output_stream();
    }

    #[test]
    fn playback_controller_status_reports_idle_before_any_play() {
        let (controller, _transitions) = PlaybackController::new();
        let status = controller.status().expect("status should succeed");
        assert!(!status.playing);
        assert_eq!(status.path, None);
        assert_eq!(status.position_ms, 0);
    }

    #[test]
    fn playback_controller_stop_is_a_no_op_when_nothing_is_loaded() {
        let (controller, _transitions) = PlaybackController::new();
        let status = controller
            .stop()
            .expect("stop should succeed even when idle");
        assert!(!status.playing);
    }

    #[test]
    fn playback_controller_play_path_rejects_missing_file() {
        let (controller, _transitions) = PlaybackController::new();
        let err = controller
            .play_path("/nonexistent/path/to/track.mp3", None, None)
            .expect_err("missing file should be rejected before touching audio hardware");
        assert!(matches!(err, BackendError::NotFound(_)));
    }
}
