use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rodio::buffer::SamplesBuffer;
use rodio::cpal::default_host;
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{Decoder, OutputStream, Sink, Source};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::error::{BackendError, BackendResult};
use crate::models::{PlaybackPreflightData, PlaybackStatusData};

const PLAYBACK_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct PlaybackController {
    tx: mpsc::Sender<PlaybackCommand>,
}

impl Default for PlaybackController {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaybackController {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<PlaybackCommand>();
        thread::spawn(move || playback_worker(rx));
        Self { tx }
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

fn playback_worker(rx: mpsc::Receiver<PlaybackCommand>) {
    let mut state = WorkerState::default();

    while let Ok(command) = rx.recv() {
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

    let file = File::open(&normalized)?;
    let reader = BufReader::new(file);
    match Decoder::new(reader) {
        Ok(decoder) => {
            let duration_ms = decoder
                .total_duration()
                .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64);
            let offset_ms = compute_target_offset_ms(start_offset_ms, start_ratio, duration_ms);
            if offset_ms > 0 {
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
        Err(rodio_err) => {
            let decoded = match decode_audio_pcm_symphonia(&normalized) {
                Ok(v) => v,
                Err(sym_err) => {
                    return Err(BackendError::Internal(format!(
                        "decoder error (rodio: {rodio_err}; symphonia: {sym_err})"
                    )));
                }
            };
            let duration_ms = Some(
                (((decoded.samples.len() as u128) * 1000u128)
                    / ((decoded.sample_rate.max(1) as u128) * (decoded.channels.max(1) as u128)))
                    .min(u128::from(u64::MAX)) as u64,
            );
            let offset_ms = compute_target_offset_ms(start_offset_ms, start_ratio, duration_ms);
            let src = SamplesBuffer::new(
                decoded.channels.max(1),
                decoded.sample_rate.max(1),
                decoded.samples,
            );
            if offset_ms > 0 {
                sink.append(src.skip_duration(Duration::from_millis(offset_ms)));
            } else {
                sink.append(src);
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
    }
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

struct DecodedPcm {
    samples: Vec<f32>,
    sample_rate: u32,
    channels: u16,
}

fn decode_audio_pcm_symphonia(path: &str) -> Result<DecodedPcm, String> {
    let file = File::open(path).map_err(|err| err.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = Path::new(path).extension().and_then(|v| v.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions {
                enable_gapless: false,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|err| err.to_string())?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| "missing default audio track".to_string())?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|err| err.to_string())?;

    let mut samples = Vec::<f32>::new();
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(44_100).max(1);
    let mut channels_out: u16 = 1;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => continue,
            Err(err) => return Err(err.to_string()),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::ResetRequired) => continue,
            Err(err) => return Err(err.to_string()),
        };

        sample_rate = decoded.spec().rate.max(1);
        let channels = decoded.spec().channels.count().max(1);
        channels_out = u16::try_from(channels).unwrap_or(1).max(1);
        let mut sample_buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        sample_buf.copy_interleaved_ref(decoded);
        let interleaved = sample_buf.samples();
        samples.extend_from_slice(interleaved);
    }

    if samples.is_empty() {
        return Err("decoder produced no samples".to_string());
    }

    Ok(DecodedPcm {
        samples,
        sample_rate,
        channels: channels_out,
    })
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
        let file = File::open(&normalized)?;
        Decoder::new(BufReader::new(file)).is_ok()
            || decode_audio_pcm_symphonia(&normalized).is_ok()
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

    #[test]
    fn symphonia_fallback_decodes_flac_fixture() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio/formats/track_format_flac.flac");
        let decoded = decode_audio_pcm_symphonia(path.to_string_lossy().as_ref())
            .expect("symphonia should decode flac fixture");
        assert!(
            !decoded.samples.is_empty(),
            "decoded sample buffer should not be empty"
        );
        assert!(decoded.sample_rate > 0, "sample rate should be positive");
        assert!(decoded.channels > 0, "channel count should be positive");
    }
}
