//! Transcodes: extra files of a library track in another format (AIFF, WAV,
//! MP3 320), made by the user from the library.
//!
//! A transcode is not a track. It has no cues, grid, tags or playlist entries
//! of its own; it is only another file of the same track, kept in the
//! transcode folder (never a source folder, so it is never scanned, indexed
//! or seen by the duplicate checks). `track_transcodes` records it, and every
//! track read carries the list (`Track.transcodes`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use rusqlite::{OptionalExtension, params, params_from_iter};

use super::anlz::AnlzBundlePaths;
use super::{
    BackendService, LibrarySources, browse_path_matches_root, now, stored_file_path, track_rows,
};
use crate::audio_encode::{TranscodeError, TranscodeFormat, TranscodeTags, transcode_file};
use crate::audio_format::AudioFormat;
use crate::error::{BackendError, BackendResult};
use crate::models::{
    MaterializeSourceTrackRequest, RemoveTrackTranscodesData, RemoveTrackTranscodesRequest,
    SetTranscodeFolderRequest, Track, TrackTranscode, TranscodeFolderData, TranscodeTracksData,
    TranscodeTracksRequest,
};

pub(crate) const SETTING_TRANSCODE_DIR: &str = "transcode_dir_v1";
const DEFAULT_DIR_NAME: &str = "transcodes";
/// Tracks outside every source folder go here, named after the file plus a
/// piece of the track id.
const OTHER_DIR_NAME: &str = "other";
const MAX_WORKERS: usize = 4;

/// Progress of one transcode job, reported once per finished track.
#[derive(Debug, Clone)]
pub struct TranscodeProgress {
    pub current: usize,
    pub total: usize,
    pub track_id: String,
    pub file_path: String,
    pub failed: bool,
}

/// One planned transcode.
struct Job {
    track: Track,
    format: TranscodeFormat,
    source: PathBuf,
    output: PathBuf,
    tags: TranscodeTags,
    source_size: Option<i64>,
    source_modified_at: Option<i64>,
    /// Where the transcode's own waveform bundle goes (`None` without a data
    /// folder, in tests on an in-memory DB).
    bundle_dir: Option<PathBuf>,
}

type JobResult = Result<crate::audio_encode::TranscodeOutput, TranscodeError>;

/// Where a transcode's analysis bundle lives: in the app data, named after
/// the transcode file's path, so it follows the file whichever track owns it.
const BUNDLE_DIR_NAME: &str = "transcode_analysis";
/// The bundle drawn from the transcode file: waveforms and seek index only.
const WAVEFORM_BUNDLE_STEM: &str = "TRANSCODE";
/// That bundle merged with the track's own analysis for export (see
/// `anlz::transcode_bundle_file`).
const EXPORT_BUNDLE_STEM: &str = "ANLZ0000";

/// The app data folder of `conn`'s database.
fn data_dir_of(conn: &rusqlite::Connection) -> Option<PathBuf> {
    conn.path()
        .filter(|path| !path.is_empty())
        .and_then(|path| Path::new(path).parent().map(Path::to_path_buf))
}

pub(crate) fn transcode_bundle_dir(data_dir: &Path, transcode_path: &Path) -> PathBuf {
    // FNV-1a: stable across runs and platforms.
    let hash = transcode_path
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    data_dir.join(BUNDLE_DIR_NAME).join(format!("{hash:016x}"))
}

fn bundle_paths(dir: &Path, stem: &str) -> AnlzBundlePaths {
    AnlzBundlePaths {
        dat_path: dir.join(format!("{stem}.DAT")),
        ext_path: dir.join(format!("{stem}.EXT")),
        twoex_path: dir.join(format!("{stem}.2EX")),
    }
}

pub(crate) fn transcode_waveform_bundle(dir: &Path) -> AnlzBundlePaths {
    bundle_paths(dir, WAVEFORM_BUNDLE_STEM)
}

pub(crate) fn transcode_export_bundle(dir: &Path) -> AnlzBundlePaths {
    bundle_paths(dir, EXPORT_BUNDLE_STEM)
}

/// Draws the transcode's waveform bundle from its file. A failure is logged
/// and leaves no bundle, so export draws it again when it needs it instead
/// of using one drawn from an earlier file at this path.
pub(crate) fn write_transcode_waveform_bundle(dir: &Path, transcode: &Path) -> bool {
    let paths = transcode_waveform_bundle(dir);
    match super::analysis::write_waveform_bundle_for_audio(transcode, &paths) {
        Ok(()) => true,
        Err(err) => {
            crate::logging::emit(
                crate::logging::Level::Warn,
                "transcode",
                &format!("no waveform for transcode {}: {err}", transcode.display()),
            );
            for path in [&paths.dat_path, &paths.ext_path, &paths.twoex_path] {
                remove_file_if_present(path);
            }
            false
        }
    }
}

pub(crate) fn file_facts(path: &Path) -> (Option<i64>, Option<i64>) {
    match std::fs::metadata(path) {
        Ok(meta) => (
            i64::try_from(meta.len()).ok(),
            crate::scanner::file_time_secs(meta.modified()),
        ),
        Err(_) => (None, None),
    }
}

/// Whether `path` holds lossless audio a transcode can be made from.
fn is_lossless_source(path: &Path) -> bool {
    match AudioFormat::of_path(path) {
        Some(AudioFormat::Flac | AudioFormat::Wav | AudioFormat::Aiff | AudioFormat::Alac) => true,
        // `.m4a` holds AAC or ALAC; only the codec tells.
        Some(AudioFormat::M4a) => crate::audio_format::is_alac_file(path),
        _ => false,
    }
}

/// Whether the source already is `format` (no transcode needed).
fn source_is_format(path: &Path, format: TranscodeFormat) -> bool {
    let target = match format {
        TranscodeFormat::Aiff => AudioFormat::Aiff,
        TranscodeFormat::Wav => AudioFormat::Wav,
        TranscodeFormat::Mp3 => AudioFormat::Mp3,
    };
    AudioFormat::of_path(path) == Some(target)
}

impl BackendService {
    /// Where new transcodes go: the chosen folder, or `transcodes` inside
    /// the app data.
    fn transcode_dir(&self, conn: &rusqlite::Connection) -> BackendResult<(PathBuf, bool)> {
        let chosen: Option<String> = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                params![SETTING_TRANSCODE_DIR],
                |row| row.get(0),
            )
            .optional()?;
        Ok(
            match chosen
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
            {
                Some(path) => (PathBuf::from(path), false),
                None => (self.db.data_dir().join(DEFAULT_DIR_NAME), true),
            },
        )
    }

    pub fn transcode_folder(&self) -> BackendResult<TranscodeFolderData> {
        let conn = self.db.connect()?;
        let (path, is_default) = self.transcode_dir(&conn)?;
        Ok(TranscodeFolderData {
            path: path.to_string_lossy().to_string(),
            is_default,
        })
    }

    /// Changing the folder only affects new transcodes: existing ones keep
    /// their stored paths.
    pub fn set_transcode_folder(
        &self,
        req: SetTranscodeFolderRequest,
    ) -> BackendResult<TranscodeFolderData> {
        let conn = self.db.connect()?;
        match req.path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            Some(path) => {
                if !Path::new(path).is_absolute() {
                    return Err(BackendError::Validation(
                        "the transcode folder must be an absolute path".to_string(),
                    ));
                }
                let sources = LibrarySources::load(&conn)?;
                if sources
                    .roots
                    .iter()
                    .any(|root| browse_path_matches_root(path, root))
                {
                    return Err(BackendError::Validation(
                        "the transcode folder can't be inside a source folder: the scan would \
                         list the transcodes as tracks"
                            .to_string(),
                    ));
                }
                conn.execute(
                    "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                       updated_at = excluded.updated_at",
                    params![SETTING_TRANSCODE_DIR, path, now()],
                )?;
            }
            None => {
                conn.execute(
                    "DELETE FROM app_settings WHERE key = ?1",
                    params![SETTING_TRANSCODE_DIR],
                )?;
            }
        }
        drop(conn);
        self.transcode_folder()
    }

    pub fn cancel_transcode(&self) -> BackendResult<()> {
        self.transcode_cancelled.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Makes a `format` transcode of each of `track_ids` that needs one. The
    /// backend decides which do: lossless sources only, never into their own
    /// format, and not when an up-to-date one exists. Finished files are kept
    /// when the job is cancelled.
    pub fn transcode_tracks_with_progress<F>(
        &self,
        req: TranscodeTracksRequest,
        mut on_progress: F,
    ) -> BackendResult<TranscodeTracksData>
    where
        F: FnMut(&TranscodeProgress),
    {
        let format = TranscodeFormat::parse(&req.format).ok_or_else(|| {
            BackendError::Validation(format!("unknown transcode format: {}", req.format))
        })?;
        self.transcode_cancelled.store(false, Ordering::SeqCst);
        let conn = self.db.connect()?;
        let track_ids = self.resolve_transcode_track_ids(&conn, &req.track_ids)?;
        let tracks = track_rows(&conn, &track_ids)?;
        let (out_dir, _) = self.transcode_dir(&conn)?;
        let roots = LibrarySources::load(&conn)?.roots;

        let mut data = TranscodeTracksData::default();
        let mut reserved = HashSet::<PathBuf>::new();
        let mut jobs = Vec::<Job>::new();
        for track in tracks {
            let source = stored_file_path(&track.file_path).to_path_buf();
            let name = source
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| track.file_path.clone());
            let skip = if !source.is_file() {
                Some("file not found")
            } else if source_is_format(&source, format) {
                Some("already this format")
            } else if !is_lossless_source(&source) {
                Some("not a lossless file")
            } else if track
                .transcodes
                .iter()
                .any(|t| t.format == format.as_str() && !t.outdated)
            {
                Some("already has an up-to-date transcode")
            } else {
                None
            };
            if let Some(reason) = skip {
                data.skipped += 1;
                data.messages.push(format!("{name}: {reason}"));
                continue;
            }
            jobs.push(plan_job(
                &conn,
                &out_dir,
                &roots,
                track,
                source,
                format,
                &mut reserved,
            )?);
        }
        drop(conn);

        let total = jobs.len();
        if total == 0 {
            data.tracks = track_rows(&*self.db.connect()?, &track_ids)?;
            return Ok(data);
        }
        let cancel = Arc::clone(&self.transcode_cancelled);
        let mut current = 0usize;
        self.run_transcode_jobs(jobs, &cancel, |job, result| {
            let name = job
                .source
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let failed = match result {
                Ok(()) => {
                    data.transcoded += 1;
                    false
                }
                Err(TranscodeError::Cancelled) => {
                    data.cancelled = true;
                    return;
                }
                Err(TranscodeError::Failed(message)) => {
                    data.failed += 1;
                    data.messages.push(format!("{name}: {message}"));
                    true
                }
            };
            current += 1;
            on_progress(&TranscodeProgress {
                current,
                total,
                track_id: job.track.id.clone(),
                file_path: job.track.file_path.clone(),
                failed,
            });
        })?;
        data.cancelled |= self.transcode_cancelled.load(Ordering::SeqCst);
        data.tracks = track_rows(&*self.db.connect()?, &track_ids)?;
        Ok(data)
    }

    /// Runs `jobs` on a few worker threads and records each finished one;
    /// `on_done` hears of every job, recorded or not, in finishing order.
    fn run_transcode_jobs(
        &self,
        jobs: Vec<Job>,
        cancel: &AtomicBool,
        mut on_done: impl FnMut(&Job, Result<(), TranscodeError>),
    ) -> BackendResult<()> {
        let total = jobs.len();
        if total == 0 {
            return Ok(());
        }
        let queue = Mutex::new(jobs.into_iter().collect::<VecDeque<_>>());
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .clamp(1, MAX_WORKERS)
            .min(total);
        let (tx, rx) = mpsc::channel::<(Job, JobResult)>();
        std::thread::scope(|scope| -> BackendResult<()> {
            for _ in 0..workers {
                let tx = tx.clone();
                let queue = &queue;
                scope.spawn(move || {
                    loop {
                        if cancel.load(Ordering::Relaxed) {
                            return;
                        }
                        let Some(job) = queue.lock().ok().and_then(|mut q| q.pop_front()) else {
                            return;
                        };
                        let result = run_job(&job, cancel);
                        if tx.send((job, result)).is_err() {
                            return;
                        }
                    }
                });
            }
            drop(tx);
            let conn = self.db.connect()?;
            for (job, result) in rx {
                let result = match result {
                    Ok(output) => {
                        record_transcode(&conn, &job, &output)?;
                        Ok(())
                    }
                    Err(err) => Err(err),
                };
                on_done(&job, result);
            }
            Ok(())
        })
    }

    /// Makes the transcodes an export needs: `wanted` is (track id, format).
    /// Not cancellable on its own (export isn't). Reports each finished one
    /// as (done, total, file name) and returns the failures as (track id,
    /// message).
    pub(crate) fn make_transcodes_for_export(
        &self,
        wanted: &[(String, TranscodeFormat)],
        mut on_progress: impl FnMut(usize, usize, &str),
    ) -> BackendResult<Vec<(String, String)>> {
        let conn = self.db.connect()?;
        let ids: Vec<String> = wanted.iter().map(|(id, _)| id.clone()).collect();
        let mut tracks: HashMap<String, Track> = track_rows(&conn, &ids)?
            .into_iter()
            .map(|track| (track.id.clone(), track))
            .collect();
        let (out_dir, _) = self.transcode_dir(&conn)?;
        let roots = LibrarySources::load(&conn)?.roots;
        let mut reserved = HashSet::<PathBuf>::new();
        let mut jobs = Vec::new();
        let mut failures = Vec::new();
        for (id, format) in wanted {
            let Some(track) = tracks.remove(id) else {
                failures.push((id.clone(), "track not found".to_string()));
                continue;
            };
            let source = stored_file_path(&track.file_path).to_path_buf();
            jobs.push(plan_job(
                &conn,
                &out_dir,
                &roots,
                track,
                source,
                *format,
                &mut reserved,
            )?);
        }
        drop(conn);
        let total = jobs.len();
        let mut done = 0usize;
        let never_cancelled = AtomicBool::new(false);
        self.run_transcode_jobs(jobs, &never_cancelled, |job, result| {
            done += 1;
            if let Err(err) = result {
                failures.push((job.track.id.clone(), err.to_string()));
            }
            let name = job
                .source
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            on_progress(done, total, &name);
        })?;
        Ok(failures)
    }

    /// The selected ids as `tracks` rows. A browse-only row (a file the
    /// scan lists but hasn't indexed, `id == file_path`) is indexed first,
    /// as adding it to a playlist does.
    fn resolve_transcode_track_ids(
        &self,
        conn: &rusqlite::Connection,
        ids: &[String],
    ) -> BackendResult<Vec<String>> {
        let mut out = Vec::with_capacity(ids.len());
        let mut seen = HashSet::new();
        for id in ids.iter().map(|id| id.trim()).filter(|id| !id.is_empty()) {
            if !seen.insert(id.to_string()) {
                continue;
            }
            if super::playlists::track_id_exists(conn, id)? {
                out.push(id.to_string());
                continue;
            }
            let path = stored_file_path(id);
            if !path.is_file() {
                continue;
            }
            let scanned = crate::scanner::scan_audio_file(path)?;
            let materialized = self.materialize_source_track(MaterializeSourceTrackRequest {
                file_path: id.to_string(),
                title: scanned.title,
                artist: scanned.artist,
                album: scanned.album,
                track_number: scanned.track_number,
                key: scanned.tonality,
                file_size_bytes: scanned.file_size_bytes,
                format_ext: scanned.format_ext,
                sample_rate_hz: scanned.sample_rate_hz,
                bit_depth: scanned.bit_depth,
                bitrate_kbps: scanned.bitrate_kbps,
            })?;
            out.push(materialized.track_id);
        }
        Ok(out)
    }

    pub fn remove_track_transcodes(
        &self,
        req: RemoveTrackTranscodesRequest,
    ) -> BackendResult<RemoveTrackTranscodesData> {
        let conn = self.db.connect()?;
        let mut removed = 0usize;
        for id in &req.track_ids {
            let paths = transcode_paths_where(&conn, "track_id = ?1", params![id])?;
            for path in &paths {
                remove_transcode_files(&conn, path);
            }
            removed += conn.execute(
                "DELETE FROM track_transcodes WHERE track_id = ?1",
                params![id],
            )?;
        }
        Ok(RemoveTrackTranscodesData {
            removed,
            tracks: track_rows(&conn, &req.track_ids)?,
        })
    }
}

fn run_job(job: &Job, cancel: &AtomicBool) -> JobResult {
    let format = job.format;
    let parent = job
        .output
        .parent()
        .ok_or_else(|| TranscodeError::Failed("bad output path".to_string()))?;
    std::fs::create_dir_all(parent).map_err(|err| TranscodeError::Failed(err.to_string()))?;
    // `<stem>.part.<ext>`: the real extension stays last so the tag writer
    // can tell the file type.
    let stem = job
        .output
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let part = parent.join(format!("{stem}.part.{}", format.extension()));
    let result = transcode_file(&job.source, &part, format, &job.tags, cancel).and_then(|out| {
        std::fs::rename(&part, &job.output)
            .map(|()| out)
            .map_err(|err| TranscodeError::Failed(err.to_string()))
    });
    if result.is_err() {
        remove_file_if_present(&part);
    } else if let Some(dir) = &job.bundle_dir {
        write_transcode_waveform_bundle(dir, &job.output);
    }
    result
}

/// Deletes a transcode file and its analysis bundle.
fn remove_transcode_files(conn: &rusqlite::Connection, path: &Path) {
    remove_file_if_present(path);
    if let Some(data_dir) = data_dir_of(conn) {
        let dir = transcode_bundle_dir(&data_dir, path);
        if let Err(err) = std::fs::remove_dir_all(&dir)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            crate::logging::emit(
                crate::logging::Level::Warn,
                "transcode",
                &format!("could not remove {}: {err}", dir.display()),
            );
        }
    }
}

/// A transcode of `track` into `format`, at its own free output path.
fn plan_job(
    conn: &rusqlite::Connection,
    out_dir: &Path,
    roots: &[String],
    track: Track,
    source: PathBuf,
    format: TranscodeFormat,
    reserved: &mut HashSet<PathBuf>,
) -> BackendResult<Job> {
    let existing = existing_transcode_path(conn, &track.id, format)?;
    let output = output_path(
        out_dir,
        roots,
        &track,
        &source,
        format,
        existing.as_deref(),
        reserved,
    );
    reserved.insert(output.clone());
    let (source_size, source_modified_at) = file_facts(&source);
    let tags = tags_for(&track, &source);
    let bundle_dir = data_dir_of(conn).map(|dir| transcode_bundle_dir(&dir, &output));
    Ok(Job {
        track,
        format,
        source,
        output,
        tags,
        source_size,
        source_modified_at,
        bundle_dir,
    })
}

fn remove_file_if_present(path: &Path) {
    if let Err(err) = std::fs::remove_file(path)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        crate::logging::emit(
            crate::logging::Level::Warn,
            "transcode",
            &format!("could not remove transcode {}: {err}", path.display()),
        );
    }
}

fn existing_transcode_path(
    conn: &rusqlite::Connection,
    track_id: &str,
    format: TranscodeFormat,
) -> BackendResult<Option<PathBuf>> {
    Ok(conn
        .query_row(
            "SELECT file_path FROM track_transcodes WHERE track_id = ?1 AND format = ?2",
            params![track_id, format.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(PathBuf::from))
}

/// Where a new transcode goes: the source's path relative to its source
/// folder, mirrored under `out_dir` with the new extension. Tracks outside
/// every source folder (imported from rekordbox, Mixxx or a USB) go to
/// `other/<name>-<id>.<ext>`. A name taken by another file gets `-2`, `-3`,
/// ...; the track's own earlier transcode of this format is replaced.
fn output_path(
    out_dir: &Path,
    roots: &[String],
    track: &Track,
    source: &Path,
    format: TranscodeFormat,
    own_existing: Option<&Path>,
    reserved: &HashSet<PathBuf>,
) -> PathBuf {
    let ext = format.extension();
    let relative = roots
        .iter()
        .filter(|root| browse_path_matches_root(&track.file_path, root))
        .max_by_key(|root| root.len())
        .and_then(|root| source.strip_prefix(stored_file_path(root)).ok())
        .map(Path::to_path_buf);
    let base = match relative {
        Some(relative) => out_dir.join(relative).with_extension(ext),
        None => {
            let stem = source
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "track".to_string());
            let short_id: String = track
                .id
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .take(8)
                .collect();
            out_dir
                .join(OTHER_DIR_NAME)
                .join(format!("{stem}-{short_id}.{ext}"))
        }
    };
    let free = |candidate: &Path| {
        !reserved.contains(candidate) && (own_existing == Some(candidate) || !candidate.exists())
    };
    if free(&base) {
        return base;
    }
    let stem = base
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir = base.parent().map(Path::to_path_buf).unwrap_or_default();
    (2..)
        .map(|n| dir.join(format!("{stem}-{n}.{ext}")))
        .find(|candidate| free(candidate))
        .unwrap_or(base)
}

/// Tag values for the transcode: the app's own track data (so edits made in
/// the app are in the file), and the track's cover.
fn tags_for(track: &Track, source: &Path) -> TranscodeTags {
    let artwork = track
        .artwork_path
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .and_then(|p| std::fs::read(p).ok())
        .or_else(|| super::analysis::extract_embedded_cover_art_bytes(source));
    TranscodeTags {
        title: Some(track.title.clone()),
        artist: Some(track.artist.clone()),
        album: track.album.clone(),
        genre: track.genre.clone(),
        year: track.year,
        track_number: track.track_number,
        bpm: track.bpm,
        key: track.key.clone(),
        artwork,
    }
}

fn record_transcode(
    conn: &rusqlite::Connection,
    job: &Job,
    output: &crate::audio_encode::TranscodeOutput,
) -> BackendResult<()> {
    let format = job.format;
    let previous = existing_transcode_path(conn, &job.track.id, format)?;
    let size = std::fs::metadata(&job.output)
        .ok()
        .and_then(|meta| i64::try_from(meta.len()).ok())
        .unwrap_or(0);
    conn.execute(
        "INSERT INTO track_transcodes (track_id, format, file_path, file_size_bytes,
           sample_rate_hz, bit_depth, bitrate_kbps, duration_ms, offset_ms,
           source_size_bytes, source_modified_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(track_id, format) DO UPDATE SET
           file_path = excluded.file_path, file_size_bytes = excluded.file_size_bytes,
           sample_rate_hz = excluded.sample_rate_hz, bit_depth = excluded.bit_depth,
           bitrate_kbps = excluded.bitrate_kbps, duration_ms = excluded.duration_ms,
           offset_ms = excluded.offset_ms, source_size_bytes = excluded.source_size_bytes,
           source_modified_at = excluded.source_modified_at, created_at = excluded.created_at",
        params![
            job.track.id,
            format.as_str(),
            job.output.to_string_lossy(),
            size,
            output.sample_rate,
            output.bit_depth,
            output.bitrate_kbps,
            i64::try_from(output.duration_ms).unwrap_or(i64::MAX),
            output.offset_ms,
            job.source_size,
            job.source_modified_at,
            now(),
        ],
    )?;
    if let Some(previous) = previous
        && previous != job.output
    {
        remove_transcode_files(conn, &previous);
    }
    Ok(())
}

fn transcode_paths_where(
    conn: &rusqlite::Connection,
    condition: &str,
    params: impl rusqlite::Params,
) -> BackendResult<Vec<PathBuf>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT file_path FROM track_transcodes WHERE {condition}"
    ))?;
    let rows = stmt.query_map(params, |row| row.get::<_, String>(0))?;
    Ok(rows
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(PathBuf::from)
        .collect())
}

/// Deletes the transcodes (files and rows) of tracks that no longer exist.
/// Called wherever track rows are deleted.
pub(crate) fn sweep_orphan_transcodes(conn: &rusqlite::Connection) -> BackendResult<usize> {
    const ORPHAN: &str = "track_id NOT IN (SELECT id FROM tracks)";
    let paths = transcode_paths_where(conn, ORPHAN, [])?;
    if paths.is_empty() {
        return Ok(0);
    }
    for path in &paths {
        remove_transcode_files(conn, path);
    }
    Ok(conn.execute(&format!("DELETE FROM track_transcodes WHERE {ORPHAN}"), [])?)
}

/// A merge of `from_id` into `into_id` keeps `from_id`'s transcodes for the
/// formats `into_id` has none of; the rest are swept with `from_id`.
pub(crate) fn move_transcodes(
    tx: &rusqlite::Transaction<'_>,
    from_id: &str,
    into_id: &str,
) -> BackendResult<()> {
    tx.execute(
        "UPDATE OR IGNORE track_transcodes SET track_id = ?1 WHERE track_id = ?2",
        params![into_id, from_id],
    )?;
    Ok(())
}

/// Fills `Track.transcodes` for a batch of frontend-bound tracks. A
/// transcode whose file is gone isn't listed; one whose source file changed
/// since is `outdated`. A source that can't be read (a stick that isn't
/// plugged in) doesn't make it outdated.
pub(crate) fn fill_track_transcodes(
    conn: &rusqlite::Connection,
    tracks: &mut [Track],
) -> BackendResult<()> {
    if tracks.is_empty() {
        return Ok(());
    }
    let any: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM track_transcodes)", [], |row| {
        row.get(0)
    })?;
    if !any {
        return Ok(());
    }
    let ids: Vec<&str> = tracks.iter().map(|t| t.id.as_str()).collect();
    let mut by_track =
        HashMap::<String, Vec<(String, String, i64, Option<i64>, Option<i64>)>>::new();
    for chunk in ids.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let mut stmt = conn.prepare(&format!(
            "SELECT track_id, format, file_path, file_size_bytes, source_size_bytes,
                    source_modified_at
             FROM track_transcodes WHERE track_id IN ({placeholders})
             ORDER BY format"
        ))?;
        let rows = stmt.query_map(params_from_iter(chunk.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ),
            ))
        })?;
        for row in rows {
            let (track_id, transcode) = row?;
            by_track.entry(track_id).or_default().push(transcode);
        }
    }
    for track in tracks.iter_mut() {
        let Some(rows) = by_track.remove(&track.id) else {
            continue;
        };
        let (source_size, source_modified_at) = file_facts(stored_file_path(&track.file_path));
        track.transcodes = rows
            .into_iter()
            .filter(|(_, path, ..)| Path::new(path).is_file())
            .filter_map(|(format, path, size, stored_size, stored_modified)| {
                let parsed = TranscodeFormat::parse(&format)?;
                let outdated = source_size.is_some()
                    && (source_size != stored_size || source_modified_at != stored_modified);
                Some(TrackTranscode {
                    format: parsed.as_str().to_string(),
                    label: parsed.label().to_string(),
                    file_path: path,
                    size_bytes: size,
                    outdated,
                })
            })
            .collect();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AddSourceFoldersRequest, ScanLibraryRequest};

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio")
            .join(name)
    }

    #[test]
    fn a_waveform_bundle_that_cant_be_drawn_leaves_none_behind() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = transcode_waveform_bundle(dir.path());
        // Drawn from the file that was at this path before.
        for path in [&bundle.dat_path, &bundle.ext_path, &bundle.twoex_path] {
            std::fs::write(path, b"old").unwrap();
        }
        let not_audio = dir.path().join("broken.mp3");
        std::fs::write(&not_audio, b"not audio").unwrap();

        assert!(!write_transcode_waveform_bundle(dir.path(), &not_audio));
        assert!(!bundle.dat_path.exists());
        assert!(!bundle.ext_path.exists());
        assert!(!bundle.twoex_path.exists());
    }

    struct Setup {
        _dirs: tempfile::TempDir,
        service: BackendService,
        music: PathBuf,
        out: PathBuf,
    }

    /// A library with one source folder holding `files` (fixture name →
    /// path inside the folder), scanned, and a transcode folder outside it.
    fn setup(files: &[(&str, &str)]) -> Setup {
        let dirs = tempfile::tempdir().expect("tempdir");
        let music = dirs.path().join("music");
        let out = dirs.path().join("transcodes");
        for (fixture_name, inside) in files {
            let dst = music.join(inside);
            std::fs::create_dir_all(dst.parent().expect("parent")).expect("mkdir");
            std::fs::copy(fixture(fixture_name), &dst).expect("copy fixture");
        }
        std::fs::create_dir_all(&music).expect("mkdir music");
        let service = BackendService::new(dirs.path().join("data")).expect("service");
        service
            .add_source_folders(AddSourceFoldersRequest {
                paths: vec![music.to_string_lossy().to_string()],
            })
            .expect("add source folder");
        scan(&service);
        service
            .set_transcode_folder(SetTranscodeFolderRequest {
                path: Some(out.to_string_lossy().to_string()),
            })
            .expect("set transcode folder");
        Setup {
            _dirs: dirs,
            service,
            music,
            out,
        }
    }

    fn scan(service: &BackendService) {
        service
            .scan_library(ScanLibraryRequest {
                source_roots: Vec::new(),
                incremental: true,
            })
            .expect("scan");
    }

    fn track_id_for(service: &BackendService, path: &Path) -> String {
        let conn = service.db.connect().expect("connect");
        conn.query_row(
            "SELECT id FROM tracks WHERE file_path = ?1",
            params![path.to_string_lossy()],
            |row| row.get(0),
        )
        .expect("track row")
    }

    fn track(service: &BackendService, id: &str) -> Track {
        let conn = service.db.connect().expect("connect");
        track_rows(&conn, &[id.to_string()])
            .expect("track rows")
            .pop()
            .expect("track")
    }

    fn convert(service: &BackendService, ids: &[String], format: &str) -> TranscodeTracksData {
        service
            .transcode_tracks_with_progress(
                TranscodeTracksRequest {
                    track_ids: ids.to_vec(),
                    format: format.to_string(),
                },
                |_| {},
            )
            .expect("transcode")
    }

    #[test]
    fn transcode_mirrors_the_source_path_and_shows_on_the_track() {
        let s = setup(&[("formats/track_format_flac.flac", "Crate A/set.flac")]);
        let id = track_id_for(&s.service, &s.music.join("Crate A/set.flac"));

        let data = convert(&s.service, std::slice::from_ref(&id), "mp3");
        assert_eq!((data.transcoded, data.skipped, data.failed), (1, 0, 0));
        let expected = s.out.join("Crate A/set.mp3");
        assert!(expected.is_file(), "no file at {}", expected.display());

        let transcodes = track(&s.service, &id).transcodes;
        assert_eq!(transcodes.len(), 1);
        assert_eq!(transcodes[0].format, "mp3");
        assert_eq!(transcodes[0].label, "MP3 320");
        assert_eq!(transcodes[0].file_path, expected.to_string_lossy());
        assert!(!transcodes[0].outdated);

        // Not a library track: a scan doesn't index it.
        scan(&s.service);
        let conn = s.service.db.connect().expect("connect");
        let tracks: i64 = conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
            .expect("count");
        assert_eq!(tracks, 1);

        // Up to date: nothing to do.
        let again = convert(&s.service, std::slice::from_ref(&id), "mp3");
        assert_eq!((again.transcoded, again.skipped), (0, 1));
    }

    #[test]
    fn a_changed_source_makes_the_transcode_outdated_until_redone() {
        let s = setup(&[("formats/track_format_flac.flac", "set.flac")]);
        let source = s.music.join("set.flac");
        let id = track_id_for(&s.service, &source);
        convert(&s.service, std::slice::from_ref(&id), "aiff");

        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&source)
            .expect("open source");
        file.set_modified(
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000),
        )
        .expect("touch source");
        drop(file);
        assert!(track(&s.service, &id).transcodes[0].outdated);

        let redo = convert(&s.service, std::slice::from_ref(&id), "aiff");
        assert_eq!(redo.transcoded, 1);
        let transcodes = track(&s.service, &id).transcodes;
        assert_eq!(transcodes.len(), 1);
        assert!(!transcodes[0].outdated);
        assert_eq!(
            transcodes[0].file_path,
            s.out.join("set.aiff").to_string_lossy()
        );
    }

    #[test]
    fn only_lossless_sources_in_another_format_are_converted() {
        let s = setup(&[
            ("formats/track_format_wav.wav", "a.wav"),
            ("embedded/track_embedded.mp3", "b.mp3"),
            ("formats/track_format_alac.m4a", "c.m4a"),
        ]);
        let ids =
            ["a.wav", "b.mp3", "c.m4a"].map(|name| track_id_for(&s.service, &s.music.join(name)));
        let data = convert(&s.service, &ids, "wav");
        assert_eq!(
            (data.transcoded, data.skipped),
            (1, 2),
            "{:?}",
            data.messages
        );
        assert!(
            data.messages
                .iter()
                .any(|m| m.starts_with("a.wav: already this format"))
        );
        assert!(
            data.messages
                .iter()
                .any(|m| m.starts_with("b.mp3: not a lossless file"))
        );
        assert!(s.out.join("c.wav").is_file());
    }

    #[test]
    fn transcodes_go_with_their_track() {
        let s = setup(&[("formats/track_format_flac.flac", "set.flac")]);
        let source = s.music.join("set.flac");
        let id = track_id_for(&s.service, &source);
        convert(&s.service, std::slice::from_ref(&id), "wav");
        let wav = s.out.join("set.wav");
        assert!(wav.is_file());

        // The file goes away, the scan drops the track, and its transcode.
        std::fs::remove_file(&source).expect("remove source");
        scan(&s.service);
        assert!(!wav.exists());
        let conn = s.service.db.connect().expect("connect");
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM track_transcodes", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(rows, 0);
    }

    #[test]
    fn remove_deletes_the_files() {
        let s = setup(&[("formats/track_format_flac.flac", "set.flac")]);
        let id = track_id_for(&s.service, &s.music.join("set.flac"));
        convert(&s.service, std::slice::from_ref(&id), "wav");
        convert(&s.service, std::slice::from_ref(&id), "mp3");
        assert_eq!(track(&s.service, &id).transcodes.len(), 2);

        let removed = s
            .service
            .remove_track_transcodes(RemoveTrackTranscodesRequest {
                track_ids: vec![id.clone()],
            })
            .expect("remove");
        assert_eq!(removed.removed, 2);
        assert!(!s.out.join("set.wav").exists());
        assert!(!s.out.join("set.mp3").exists());
        assert!(track(&s.service, &id).transcodes.is_empty());
    }

    #[test]
    fn a_track_outside_every_source_folder_goes_to_other() {
        let s = setup(&[]);
        let elsewhere = s.music.parent().expect("parent").join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("mkdir");
        let source = elsewhere.join("loose.flac");
        std::fs::copy(fixture("formats/track_format_flac.flac"), &source).expect("copy");
        let id = s
            .service
            .materialize_source_track(MaterializeSourceTrackRequest {
                file_path: source.to_string_lossy().to_string(),
                title: "Loose".into(),
                artist: "Nobody".into(),
                album: None,
                track_number: None,
                key: None,
                file_size_bytes: None,
                format_ext: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
            })
            .expect("materialize")
            .track_id;
        convert(&s.service, std::slice::from_ref(&id), "aiff");
        let path = PathBuf::from(&track(&s.service, &id).transcodes[0].file_path);
        assert_eq!(path.parent(), Some(s.out.join(OTHER_DIR_NAME).as_path()));
        assert!(path.is_file());
    }

    #[test]
    fn the_transcode_folder_cannot_be_inside_a_source_folder() {
        let s = setup(&[]);
        let err = s
            .service
            .set_transcode_folder(SetTranscodeFolderRequest {
                path: Some(s.music.join("converted").to_string_lossy().to_string()),
            })
            .expect_err("inside a source folder");
        assert!(matches!(err, BackendError::Validation(_)));
        let reset = s
            .service
            .set_transcode_folder(SetTranscodeFolderRequest { path: None })
            .expect("reset");
        assert!(reset.is_default);
    }
}
